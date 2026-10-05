//! The service directory: the registry of the services of a space.
//!
//! The service directory is itself a service, with the fixed identifier 1 and the name
//! `ServiceDirectory`. A node hosting a space runs a [`LocalServiceDirectory`], other nodes access
//! it through a [`Client`]. Both implement the [`ServiceDirectory`] trait.

use crate::{
    call,
    object::{self, generic, ActionId, ActionNameOrId, MetaMethod, MetaObject, MetaSignal, Object},
    service, session,
    signal::{Signal, ValueStream},
    value::{os, FromValue, IntoValue, Reflect, Value},
    Error, ObjectClient, Result,
};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use tokio::task;
use tracing::info;

/// The name of the service directory service.
pub const SD_SERVICE_NAME: &str = "ServiceDirectory";

/// The identifier of the service directory service.
pub const SD_SERVICE_ID: service::Id = service::Id(1);

/// The identifier of the node of a service directory, as published in its service information.
const SD_NODE_UID: &str = "0";

/// The interface of service directories.
#[async_trait]
pub trait ServiceDirectory: Send + Sync {
    /// All the services of the space that are ready.
    async fn services(&self) -> Result<Vec<service::Info>>;

    /// The service of the space with the given name.
    async fn service(&self, name: &str) -> Result<service::Info>;

    /// Registers a service to the space, returning its identifier. The service is not visible
    /// until it is marked ready.
    async fn register(&self, info: &service::Info) -> Result<service::Id>;

    /// Unregisters a service from the space.
    async fn unregister(&self, id: service::Id) -> Result<()>;

    /// Marks a registered service as ready.
    async fn set_ready(&self, id: service::Id) -> Result<()>;

    /// Updates the information of a service.
    async fn update(&self, info: &service::Info) -> Result<()>;

    /// The identifier of the machine hosting the service directory.
    async fn machine_id(&self) -> Result<os::MachineId>;

    /// The signal of services added to the space, with their identifier and name.
    fn service_added(&self) -> &Signal<(service::Id, String)>;

    /// The signal of services removed from the space, with their identifier and name.
    fn service_removed(&self) -> &Signal<(service::Id, String)>;
}

/// A service directory hosted by this node.
#[derive(Clone, Debug)]
pub struct LocalServiceDirectory(Arc<Inner>);

#[derive(Debug)]
struct Inner {
    map: RwLock<ServiceInfoMap>,
    service_added: Signal<(service::Id, String)>,
    service_removed: Signal<(service::Id, String)>,
}

#[derive(Debug, Default)]
struct ServiceInfoMap {
    pending_services: HashMap<service::Id, service::Info>,
    services: HashMap<service::Id, service::Info>,
    next_id: u32,
}

impl ServiceInfoMap {
    fn has_service(&self, name: &str) -> bool {
        self.pending_services
            .values()
            .chain(self.services.values())
            .any(|info| info.name == name)
    }

    fn find(&self, name: &str) -> Option<&service::Info> {
        self.services.values().find(|info| info.name == name)
    }

    fn name_of(&self, id: service::Id) -> Option<String> {
        self.services
            .get(&id)
            .or_else(|| self.pending_services.get(&id))
            .map(|info| info.name.clone())
    }
}

impl LocalServiceDirectory {
    pub(crate) fn new() -> Self {
        Self(Arc::new(Inner {
            map: RwLock::new(ServiceInfoMap {
                next_id: SD_SERVICE_ID.0,
                ..Default::default()
            }),
            service_added: Signal::new(),
            service_removed: Signal::new(),
        }))
    }

    /// Registers the service directory itself as the first service of the space.
    pub(crate) async fn register_self(
        &self,
        endpoints: Vec<session::Target>,
    ) -> Result<service::Info> {
        let info = service::Info::process_local(
            SD_SERVICE_NAME.to_owned(),
            service::Id(0),
            endpoints,
            crate::node::Uid::from_string(SD_NODE_UID.to_owned()),
            None,
        );
        let id = self.register(&info).await?;
        debug_assert_eq!(id, SD_SERVICE_ID);
        self.set_ready(id).await?;
        Ok(service::Info { id, ..info })
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, ServiceInfoMap> {
        self.0.map.read().unwrap_or_else(|err| err.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, ServiceInfoMap> {
        self.0.map.write().unwrap_or_else(|err| err.into_inner())
    }

    /// Finalizes the information of a service for a caller: relative endpoints are computed
    /// according to the capabilities of the caller session.
    fn finalize(&self, map: &ServiceInfoMap, mut info: service::Info) -> service::Info {
        let relative_enabled = call::context()
            .and_then(|ctx| ctx.session().and_then(|session| session.upgrade()))
            .map(|session| session.capabilities().relative_endpoint_uri)
            .unwrap_or(false);
        info.endpoints
            .retain(|endpoint| !endpoint.is_service_relative());
        info.endpoints.sort();
        info.endpoints.dedup();
        if relative_enabled {
            // A service is reachable through the session of another service when all the
            // endpoints of that other service are also endpoints of this service.
            let mut relative = Vec::new();
            for candidate in map.services.values() {
                let mut candidate_endpoints: Vec<_> = candidate
                    .endpoints
                    .iter()
                    .filter(|endpoint| !endpoint.is_service_relative())
                    .cloned()
                    .collect();
                candidate_endpoints.sort();
                candidate_endpoints.dedup();
                if !candidate_endpoints.is_empty()
                    && candidate_endpoints
                        .iter()
                        .all(|endpoint| info.endpoints.contains(endpoint))
                {
                    relative.push(session::Target::service(&candidate.name));
                }
            }
            info.endpoints.extend(relative);
        }
        info.endpoints
            .sort_by_cached_key(session::Target::preference_key);
        info
    }

    fn remove(&self, id: service::Id) -> Result<String> {
        let mut map = self.write();
        let name = map.name_of(id).ok_or_else(|| {
            Error::Other(format!("Unregister Service: Can't find service #{id}").into())
        })?;
        map.services.remove(&id);
        map.pending_services.remove(&id);
        Ok(name)
    }
}

#[async_trait]
impl ServiceDirectory for LocalServiceDirectory {
    async fn services(&self) -> Result<Vec<service::Info>> {
        let map = self.read();
        let mut services: Vec<_> = map
            .services
            .values()
            .cloned()
            .map(|info| self.finalize(&map, info))
            .collect();
        services.sort_by_key(|info| info.id);
        Ok(services)
    }

    async fn service(&self, name: &str) -> Result<service::Info> {
        let map = self.read();
        map.find(name)
            .cloned()
            .map(|info| self.finalize(&map, info))
            .ok_or_else(|| Error::Other(format!("Cannot find service '{name}' in index").into()))
    }

    async fn register(&self, info: &service::Info) -> Result<service::Id> {
        let id = {
            let mut map = self.write();
            if map.has_service(&info.name) {
                return Err(Error::Other(
                    format!(
                        "Service \"{}\" is already registered. Rejecting conflicting registration attempt.",
                        info.name
                    )
                    .into(),
                ));
            }
            let id = service::Id(map.next_id);
            map.next_id = map.next_id.checked_add(1).ok_or_else(|| {
                Error::Other(
                    "maximum service id has been reached, cannot register any more service".into(),
                )
            })?;
            map.pending_services
                .insert(id, service::Info { id, ..info.clone() });
            id
        };
        // Services registered by a remote node are unregistered when its link closes.
        if let Some(link_closed) = call::link_closed() {
            let directory = self.clone();
            task::spawn(async move {
                link_closed.cancelled().await;
                if directory.read().name_of(id).is_some() {
                    info!(%id, "service disconnected, unregistering");
                    let _res = directory.unregister(id).await;
                }
            });
        }
        Ok(id)
    }

    async fn unregister(&self, id: service::Id) -> Result<()> {
        let name = self.remove(id)?;
        self.0.service_removed.emit((id, name));
        Ok(())
    }

    async fn set_ready(&self, id: service::Id) -> Result<()> {
        let (id, name) = {
            let mut map = self.write();
            let service = map
                .pending_services
                .remove(&id)
                .ok_or_else(|| Error::Other(format!("Can't find pending service #{id}").into()))?;
            let name = service.name.clone();
            map.services.insert(id, service);
            (id, name)
        };
        self.0.service_added.emit((id, name));
        Ok(())
    }

    async fn update(&self, info: &service::Info) -> Result<()> {
        let mut map = self.write();
        // Every service of the same node shares the endpoints of the updated service.
        for service in map.services.values_mut() {
            if service.node_uid == info.node_uid && service.id != info.id {
                service.endpoints = info.endpoints.clone();
            }
        }
        if let Some(service) = map.services.get_mut(&info.id) {
            *service = info.clone();
        } else if let Some(service) = map.pending_services.get_mut(&info.id) {
            *service = info.clone();
        } else {
            return Err(Error::Other(
                format!("updateServiceInfo: Can't find service #{}", info.id).into(),
            ));
        }
        Ok(())
    }

    async fn machine_id(&self) -> Result<os::MachineId> {
        Ok(os::MachineId::local())
    }

    fn service_added(&self) -> &Signal<(service::Id, String)> {
        &self.0.service_added
    }

    fn service_removed(&self) -> &Signal<(service::Id, String)> {
        &self.0.service_removed
    }
}

#[async_trait]
impl Object for LocalServiceDirectory {
    fn meta(&self) -> &MetaObject {
        &Meta::get().object
    }

    async fn meta_call(&self, ident: ActionNameOrId, args: Value<'_>) -> Result<Value<'static>> {
        let meta = Meta::get();
        let method = meta
            .object
            .method(&ident)
            .ok_or_else(|| Error::MethodNotFound(ident.clone()))?;
        let id = method.uid;
        let value = if id == meta.service {
            let (name,): (String,) = args.cast_into()?;
            self.service(&name).await?.into_value()
        } else if id == meta.services {
            let (): () = args.cast_into()?;
            self.services().await?.into_value()
        } else if id == meta.register_service {
            let (info,): (service::Info,) = args.cast_into()?;
            self.register(&info).await?.into_value()
        } else if id == meta.unregister_service {
            let (id,): (service::Id,) = args.cast_into()?;
            self.unregister(id).await?;
            Value::Unit
        } else if id == meta.service_ready {
            let (id,): (service::Id,) = args.cast_into()?;
            self.set_ready(id).await?;
            Value::Unit
        } else if id == meta.update_service_info {
            let (info,): (service::Info,) = args.cast_into()?;
            self.update(&info).await?;
            Value::Unit
        } else if id == meta.machine_id {
            let (): () = args.cast_into()?;
            self.machine_id().await?.into_value()
        } else {
            return Err(Error::MethodNotFound(ident));
        };
        Ok(value)
    }

    async fn meta_subscribe(&self, ident: ActionNameOrId) -> Result<ValueStream> {
        let meta = Meta::get();
        let signal = meta
            .object
            .signal(&ident)
            .ok_or_else(|| Error::SignalNotFound(ident.clone()))?;
        if signal.uid == meta.service_added {
            self.0.service_added.subscribe_erased().await
        } else if signal.uid == meta.service_removed {
            self.0.service_removed.subscribe_erased().await
        } else {
            Err(Error::SignalNotFound(ident))
        }
    }
}

/// A client of the service directory of a space hosted by another node.
#[derive(Clone, Debug)]
pub struct Client {
    object: ObjectClient,
    service_added: Signal<(service::Id, String)>,
    service_removed: Signal<(service::Id, String)>,
}

impl Client {
    /// Connects to the service directory through a session to its node.
    pub(crate) async fn connect(session: session::Session) -> Result<Self> {
        let object = ObjectClient::connect(
            SD_SERVICE_ID,
            object::MAIN_OBJECT_ID,
            object::Uid::default(),
            session,
        )
        .await?;
        let service_added = object.signal("serviceAdded")?;
        let service_removed = object.signal("serviceRemoved")?;
        Ok(Self {
            object,
            service_added,
            service_removed,
        })
    }

    /// The proxy to the service directory object.
    pub fn object(&self) -> &ObjectClient {
        &self.object
    }

    async fn call<R, T>(&self, name: &str, args: T) -> Result<R>
    where
        T: for<'a> IntoValue<'a> + Reflect + Send,
        R: FromValue<'static> + Reflect,
    {
        crate::ObjectExt::call(&self.object, name, args).await
    }
}

#[async_trait]
impl ServiceDirectory for Client {
    async fn services(&self) -> Result<Vec<service::Info>> {
        self.call("services", ()).await
    }

    async fn service(&self, name: &str) -> Result<service::Info> {
        self.call("service", name.to_owned()).await
    }

    async fn register(&self, info: &service::Info) -> Result<service::Id> {
        self.call("registerService", info.clone()).await
    }

    async fn unregister(&self, id: service::Id) -> Result<()> {
        self.call("unregisterService", id).await
    }

    async fn set_ready(&self, id: service::Id) -> Result<()> {
        self.call("serviceReady", id).await
    }

    async fn update(&self, info: &service::Info) -> Result<()> {
        self.call("updateServiceInfo", info.clone()).await
    }

    async fn machine_id(&self) -> Result<os::MachineId> {
        self.call("machineId", ()).await
    }

    fn service_added(&self) -> &Signal<(service::Id, String)> {
        &self.service_added
    }

    fn service_removed(&self) -> &Signal<(service::Id, String)> {
        &self.service_removed
    }
}

/// The meta object of service directories, with the fixed identifiers of the protocol.
#[derive(Debug)]
struct Meta {
    object: MetaObject,
    service: ActionId,
    services: ActionId,
    register_service: ActionId,
    unregister_service: ActionId,
    service_ready: ActionId,
    update_service_info: ActionId,
    service_added: ActionId,
    service_removed: ActionId,
    machine_id: ActionId,
}

impl Meta {
    fn get() -> &'static Self {
        static META: Lazy<Meta> = Lazy::new(|| {
            let mut action_id = object::ACTION_START_ID;
            let mut builder = MetaObject::builder();
            let service = action_id.next().unwrap();
            builder.add_method({
                let mut m = MetaMethod::builder(service);
                m.set_name("service");
                m.parameter(0).set_type(<&str>::ty());
                m.return_value().set_type(service::Info::ty());
                m.build()
            });
            let services = action_id.next().unwrap();
            builder.add_method({
                let mut m = MetaMethod::builder(services);
                m.set_name("services");
                m.return_value().set_type(Vec::<service::Info>::ty());
                m.build()
            });
            let register_service = action_id.next().unwrap();
            builder.add_method({
                let mut m = MetaMethod::builder(register_service);
                m.set_name("registerService");
                m.parameter(0).set_type(service::Info::ty());
                m.return_value().set_type(service::Id::ty());
                m.build()
            });
            let unregister_service = action_id.next().unwrap();
            builder.add_method({
                let mut m = MetaMethod::builder(unregister_service);
                m.set_name("unregisterService");
                m.parameter(0).set_type(service::Id::ty());
                m.build()
            });
            let service_ready = action_id.next().unwrap();
            builder.add_method({
                let mut m = MetaMethod::builder(service_ready);
                m.set_name("serviceReady");
                m.parameter(0).set_type(service::Id::ty());
                m.build()
            });
            let update_service_info = action_id.next().unwrap();
            builder.add_method({
                let mut m = MetaMethod::builder(update_service_info);
                m.set_name("updateServiceInfo");
                m.parameter(0).set_type(service::Info::ty());
                m.build()
            });
            let service_added = action_id.next().unwrap();
            builder.add_signal(MetaSignal {
                uid: service_added,
                name: "serviceAdded".to_owned(),
                signature: Signal::<(service::Id, String)>::params_type().into(),
            });
            let service_removed = action_id.next().unwrap();
            builder.add_signal(MetaSignal {
                uid: service_removed,
                name: "serviceRemoved".to_owned(),
                signature: Signal::<(service::Id, String)>::params_type().into(),
            });
            let machine_id = action_id.next().unwrap();
            builder.add_method({
                let mut m = MetaMethod::builder(machine_id);
                m.set_name("machineId");
                m.return_value().set_type(os::MachineId::ty());
                m.build()
            });
            let mut object = builder.build();
            object.description = String::new();
            Meta {
                object,
                service,
                services,
                register_service,
                unregister_service,
                service_ready,
                update_service_info,
                service_added,
                service_removed,
                machine_id,
            }
        });
        &META
    }
}

#[allow(dead_code)]
fn assert_generic_ids_do_not_collide() {
    debug_assert!(generic::is_special(ActionId(8)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_directory_meta_object_ids() {
        let meta = Meta::get();
        assert_eq!(meta.service, ActionId(100));
        assert_eq!(meta.services, ActionId(101));
        assert_eq!(meta.register_service, ActionId(102));
        assert_eq!(meta.unregister_service, ActionId(103));
        assert_eq!(meta.service_ready, ActionId(104));
        assert_eq!(meta.update_service_info, ActionId(105));
        assert_eq!(meta.service_added, ActionId(106));
        assert_eq!(meta.service_removed, ActionId(107));
        assert_eq!(meta.machine_id, ActionId(108));
        let sig = |id| meta.object.methods[&id].parameters_signature.to_string();
        assert_eq!(sig(meta.service), "(s)");
        assert_eq!(sig(meta.unregister_service), "(I)");
        assert_eq!(
            meta.object.signals[&meta.service_added]
                .signature
                .to_string(),
            "(Is)"
        );
        assert!(sig(meta.register_service).starts_with("((sIsI[s]ss)<ServiceInfo,name,serviceId,machineId,processId,endpoints,sessionId,objectUid>)"), "{}", sig(meta.register_service));
    }
}
