//! Nodes: participants of a space.
//!
//! A *space* is a set of nodes that share a service directory. A node either *hosts* a space
//! (it runs the service directory) or *connects* to an existing one. Nodes publish services, and
//! access the services of the space.
//!
//! ```no_run
//! # async fn example() -> qi::Result<()> {
//! use qi::ObjectExt;
//! let address: qi::Address = "tcp://localhost:9559".parse().expect("valid address");
//! let node = qi::node::init()
//!     .connect_to_space(address, None)
//!     .start()
//!     .await?;
//! let tts = node.service("ALTextToSpeech").await?;
//! let () = tts.call("say", "Hello").await?;
//! # Ok(()) }
//! # let _ = example;
//! ```

mod server;

use crate::value::KeyDynValueMap;
use crate::{
    auth::Authenticator,
    object::{self, AnyObject, ObjectClient},
    service::{self, Info},
    service_directory::{self, ServiceDirectory},
    session::{self, Protocol},
    value::os::MachineId,
    Address, Error, Object, Result,
};
use async_trait::async_trait;
use futures::{stream, StreamExt, TryStreamExt};
use serde_with::serde_as;
use std::{collections::HashMap, sync::Arc};
use tokio::task;
use tracing::warn;

/// Starts the initialization of a node.
pub fn init() -> InitializingNode<NotSet> {
    InitializingNode::default()
}

/// A node being configured. See [`init`].
#[derive(Default)]
pub struct InitializingNode<Method> {
    uid: Uid,
    authenticator: Option<Arc<dyn Authenticator + Send + Sync>>,
    server_protocol: Protocol,
    bind_addresses: Vec<Address>,
    pending_services: Vec<(String, AnyObject)>,
    services: service::SharedServices,
    method: Method,
}

impl<Method> InitializingNode<Method> {
    /// Sets the authenticator that verifies the credentials of the nodes connecting to this one.
    ///
    /// Without authenticator, every connection is accepted.
    pub fn with_authenticator(
        &mut self,
        authenticator: Arc<dyn Authenticator + Send + Sync>,
    ) -> &mut Self {
        self.authenticator = Some(authenticator);
        self
    }

    /// Sets the protocol variant the servers of the node speak to the nodes connecting to them.
    ///
    /// Servers speak the [standard](Protocol::Standard) protocol by default and accept clients
    /// of both variants. With [`Protocol::Legacy`], they emulate the servers of NAOqi 2.1: the
    /// authentication call is rejected like `libqi` 2.1 does and the capabilities are advertised
    /// with a message on connection, so that clients can be tested against that handshake. No
    /// authenticator can be enforced then.
    pub fn with_server_protocol(&mut self, protocol: Protocol) -> &mut Self {
        self.server_protocol = protocol;
        self
    }

    /// Adds a service to the node, registered to the space when the node starts.
    pub fn add_service<Name, O>(&mut self, name: Name, object: O) -> &mut Self
    where
        Name: Into<String>,
        O: Object + 'static,
    {
        self.pending_services
            .push((name.into(), AnyObject::new(object)));
        self
    }

    /// Adds a shared service object to the node, registered to the space when the node starts.
    pub fn add_service_object<Name>(&mut self, name: Name, object: AnyObject) -> &mut Self
    where
        Name: Into<String>,
    {
        self.pending_services.push((name.into(), object));
        self
    }

    /// Binds the node to an address so that it may accept incoming connections on an endpoint at
    /// that address.
    pub fn bind(&mut self, address: Address) -> &mut Self {
        self.bind_addresses.push(address);
        self
    }

    /// Attaches the node to the space hosted at the given address.
    pub fn connect_to_space(
        self,
        address: Address,
        credentials: Option<KeyDynValueMap>,
    ) -> InitializingNode<ConnectToSpace> {
        InitializingNode {
            uid: self.uid,
            authenticator: self.authenticator,
            server_protocol: self.server_protocol,
            services: self.services,
            bind_addresses: self.bind_addresses,
            pending_services: self.pending_services,
            method: ConnectToSpace {
                address,
                credentials,
            },
        }
    }

    /// Hosts a new space on this node: the node runs the service directory of the space.
    pub fn host_space(self) -> InitializingNode<HostSpace> {
        InitializingNode {
            uid: self.uid,
            authenticator: self.authenticator,
            server_protocol: self.server_protocol,
            services: self.services,
            bind_addresses: self.bind_addresses,
            pending_services: self.pending_services,
            method: HostSpace,
        }
    }
}

impl<M> InitializingNode<M>
where
    M: Method,
    M::ServiceDirectory: Clone + Send + Sync + 'static,
{
    /// Starts the node: binds its servers, creates or connects to the service directory and
    /// registers the services.
    pub async fn start(self) -> Result<Node<M::ServiceDirectory>> {
        let services = self.services;
        let (server_set, mut endpoints_watcher) = server::start_servers(
            services.clone(),
            self.authenticator,
            self.server_protocol,
            self.bind_addresses,
        )
        .await?;
        let session_store = session::Store::new(services.clone());
        let server_endpoints = endpoints_to_client_targets(&endpoints_watcher.borrow_and_update());
        let service_directory = self
            .method
            .create_service_directory(SpaceContext {
                store: &session_store,
                services: &services,
                endpoints: server_endpoints.clone(),
            })
            .await?;

        // Register each service to the directory, and mark them as ready.
        stream::iter(self.pending_services)
            .map(Ok::<_, Error>)
            .try_for_each_concurrent(None, |(service_name, service_object)| async {
                register_service(
                    self.uid.clone(),
                    &services,
                    &service_directory,
                    service_name,
                    service_object,
                    server_endpoints.clone(),
                )
                .await?;
                Ok(())
            })
            .await?;

        // Update services info to the service directory whenever the server endpoints change.
        task::spawn({
            let service_directory = service_directory.clone();
            let services = services.clone();
            let mut endpoints_watcher = endpoints_watcher.clone();
            async move {
                while let Ok(()) = endpoints_watcher.changed().await {
                    let server_endpoints =
                        endpoints_to_client_targets(&endpoints_watcher.borrow_and_update());
                    let infos = services.set_endpoints(&server_endpoints);
                    stream::iter(infos)
                        .for_each_concurrent(None, |service_info| {
                            let service_directory = &service_directory;
                            async move {
                                if let Err(err) = service_directory.update(&service_info).await {
                                    warn!(
                                        error = &err as &dyn std::error::Error,
                                        "could not update service info to service directory"
                                    )
                                }
                            }
                        })
                        .await;
                }
            }
        });

        Ok(Node {
            uid: self.uid,
            services,
            session_store,
            service_directory,
            server_set,
            endpoints: endpoints_watcher,
        })
    }
}

/// Registers a service to the directory and indexes it in the node services.
async fn register_service<SD>(
    uid: Uid,
    services: &service::SharedServices,
    service_directory: &SD,
    name: String,
    object: AnyObject,
    endpoints: Vec<session::Target>,
) -> Result<service::Id>
where
    SD: ServiceDirectory,
{
    let mut info = service::Info::unregistered(name, endpoints, uid, object.uid());
    // Registering the service to the directory gets us a service ID, that we can use to
    // update the local service info. With it, we can also index the service to the
    // messaging handler so that it can start treating requests for that service.
    // Consequently, we can notify the service directory of the readiness of the service.
    let service_id = service_directory.register(&info).await?;
    info.id = service_id;
    services.add(info, object);
    if let Err(err) = service_directory.set_ready(service_id).await {
        services.remove(service_id);
        return Err(err);
    }
    Ok(service_id)
}

impl<Method> std::fmt::Debug for InitializingNode<Method>
where
    Method: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InitializingNode")
            .field("uid", &self.uid)
            .field("bind_addresses", &self.bind_addresses)
            .field(
                "pending_services",
                &self
                    .pending_services
                    .iter()
                    .map(|(name, _)| name)
                    .collect::<Vec<_>>(),
            )
            .field("method", &self.method)
            .finish_non_exhaustive()
    }
}

/// A started node. See [`init`].
#[derive(Debug)]
pub struct Node<SD> {
    uid: Uid,
    services: service::SharedServices,
    session_store: session::Store,
    service_directory: SD,
    #[allow(dead_code)]
    server_set: server::ServerSet,
    endpoints: server::EndpointsWatcher,
}

impl<SD> Node<SD>
where
    SD: ServiceDirectory,
{
    /// Gets a service of the space by name.
    ///
    /// Services registered by this node are returned directly, without going through the network.
    /// Other services are resolved through the service directory, and a session to their node is
    /// established (or reused) to return a proxy to their main object.
    pub async fn service(&self, name: &str) -> Result<AnyObject> {
        if let Some((_info, object)) = self.services.find(name) {
            return Ok(object);
        }
        let service = self.service_directory.service(name).await?;
        let session = self
            .session_store
            .get_or_create(
                name,
                sort_service_endpoints(&service),
                // Connecting to service nodes of a space should not require credentials.
                Default::default(),
            )
            .await?;
        let object = ObjectClient::connect(
            service.id(),
            object::MAIN_OBJECT_ID,
            service.object_uid().unwrap_or_default(),
            session,
        )
        .await?;
        Ok(AnyObject::new(object))
    }

    /// Registers a service on the node, making it reachable from the space under the name.
    ///
    /// Services may be registered at any time once the node is started, which is required for
    /// services that need the node itself, for instance to reach other services of the space.
    /// Returns the identifier of the service in the space.
    pub async fn register_service<Name, O>(&self, name: Name, object: O) -> Result<service::Id>
    where
        Name: Into<String>,
        O: Object + 'static,
    {
        self.register_service_object(name, AnyObject::new(object))
            .await
    }

    /// Registers a shared service object on the node. See [`Node::register_service`].
    pub async fn register_service_object<Name>(
        &self,
        name: Name,
        object: AnyObject,
    ) -> Result<service::Id>
    where
        Name: Into<String>,
    {
        register_service(
            self.uid.clone(),
            &self.services,
            &self.service_directory,
            name.into(),
            object,
            self.endpoints(),
        )
        .await
    }

    /// Unregisters a service of the node from the space.
    ///
    /// Sessions that hold proxies to the service object may keep using them until they are
    /// released.
    pub async fn unregister_service(&self, id: service::Id) -> Result<()> {
        self.service_directory.unregister(id).await?;
        self.services.remove(id);
        Ok(())
    }

    /// The service directory of the space.
    pub fn service_directory(&self) -> &SD {
        &self.service_directory
    }

    /// The unique identifier of the node.
    pub fn uid(&self) -> &Uid {
        &self.uid
    }

    /// The endpoints of the node servers, that other nodes may connect to.
    pub fn endpoints(&self) -> Vec<session::Target> {
        endpoints_to_client_targets(&self.endpoints.borrow())
    }
}

fn sort_service_endpoints(service: &Info) -> Vec<session::Target> {
    let service_is_local = service.machine_id() == MachineId::local();
    let mut endpoints = service.endpoints().to_vec();
    // Relative endpoints first, then machine local ones when the service is local.
    endpoints.sort_by_cached_key(|endpoint| {
        (
            !endpoint.is_service_relative(),
            !(service_is_local && endpoint.is_machine_local()),
        )
    });
    // A remote service cannot be reached through loopback endpoints.
    if !service_is_local {
        endpoints.retain(|endpoint| !endpoint.is_machine_local());
    }
    endpoints
}

fn endpoints_to_client_targets(endpoints: &HashMap<Address, Vec<Address>>) -> Vec<session::Target> {
    let mut targets: Vec<_> = endpoints
        .values()
        .flatten()
        .copied()
        .map(session::Target::from)
        .collect();
    targets.sort();
    targets.dedup();
    targets
}

/// The unique identifier of a node, that identifies it in the service directory.
#[derive(
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    qi_macros::Valuable,
    serde_with::SerializeDisplay,
    serde_with::DeserializeFromStr,
)]
#[serde_as]
#[qi(value(crate = "crate::value", transparent))]
pub struct Uid(String);

impl Uid {
    /// Creates a new random identifier.
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    pub fn from_string(id: String) -> Self {
        Self(id)
    }
}

impl Default for Uid {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for Uid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for Uid {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self::from_string(s.to_owned()))
    }
}

/// The context given to a [`Method`] to create the service directory of a node.
pub struct SpaceContext<'a> {
    store: &'a session::Store,
    services: &'a service::SharedServices,
    endpoints: Vec<session::Target>,
}

impl std::fmt::Debug for SpaceContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpaceContext")
            .field("endpoints", &self.endpoints)
            .finish_non_exhaustive()
    }
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::ConnectToSpace {}
    impl Sealed for super::HostSpace {}
}

/// The method a node uses to join a space.
///
/// This trait is sealed: it is implemented by [`ConnectToSpace`] and [`HostSpace`] only.
#[async_trait]
pub trait Method: sealed::Sealed {
    type ServiceDirectory: ServiceDirectory;

    async fn create_service_directory(
        self,
        context: SpaceContext<'_>,
    ) -> Result<Self::ServiceDirectory>;
}

/// The method is not set yet.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NotSet;

/// Connect to the space hosted by another node.
#[derive(Debug, PartialEq, Eq)]
pub struct ConnectToSpace {
    address: Address,
    credentials: Option<KeyDynValueMap>,
}

#[async_trait]
impl Method for ConnectToSpace {
    type ServiceDirectory = service_directory::Client;

    async fn create_service_directory(
        self,
        context: SpaceContext<'_>,
    ) -> Result<Self::ServiceDirectory> {
        let session = context
            .store
            .get_or_create(
                service_directory::SD_SERVICE_NAME,
                [self.address.into()],
                self.credentials.unwrap_or_default(),
            )
            .await?;
        service_directory::Client::connect(session).await
    }
}

/// Host the space on this node.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostSpace;

#[async_trait]
impl Method for HostSpace {
    type ServiceDirectory = service_directory::LocalServiceDirectory;

    async fn create_service_directory(
        self,
        context: SpaceContext<'_>,
    ) -> Result<Self::ServiceDirectory> {
        let directory = service_directory::LocalServiceDirectory::new();
        let info = directory.register_self(context.endpoints).await?;
        context
            .services
            .add(info, AnyObject::new(directory.clone()));
        Ok(directory)
    }
}

impl From<std::convert::Infallible> for Error {
    fn from(err: std::convert::Infallible) -> Self {
        match err {}
    }
}
