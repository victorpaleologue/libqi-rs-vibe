use crate::value::KeyDynValueMap;
use crate::{
    messaging::{self, Address},
    service,
    session::{self, target::Kind, Session, WeakSession},
    Error,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

/// A session store that handles session targets and keeps track of existing
/// sessions to a space services.
///
/// It creates new sessions that are associated with services and register them
/// for further retrieval, enabling usage of service session targets.
///
/// A session is also kept by the address it was opened to, and reused for every
/// service reached at that address, as libqi's socket cache does: a node that
/// hosts many services (`naoqi` hosts hundreds) is reached through one
/// connection, the one to the service directory included when it hosts that too.
#[derive(Debug)]
pub(crate) struct Store {
    services: service::SharedServices,

    /// The list of existing sessions with the associated service name.
    sessions: Arc<Mutex<HashMap<String, WeakSession>>>,

    /// The existing sessions by the address of their channel.
    by_address: Arc<Mutex<HashMap<Address, WeakSession>>>,

    /// One lock per address, held while a session to it is looked up or
    /// opened: concurrent requests for services at one address share a single
    /// connection instead of each opening its own.
    connecting: Arc<Mutex<HashMap<Address, Arc<Mutex<()>>>>>,
}

impl Store {
    pub(crate) fn new(services: service::SharedServices) -> Self {
        Self {
            services,
            sessions: Default::default(),
            by_address: Default::default(),
            connecting: Default::default(),
        }
    }

    async fn get_by_address(&self, address: &Address) -> Option<Session> {
        let mut sessions = self.by_address.lock().await;
        let session = sessions.get(address)?.upgrade();
        if session.is_none() {
            sessions.remove(address);
        }
        session
    }

    /// A session to the address: the existing one if any, else a new one.
    /// Either way it is registered for the service name.
    async fn get_or_connect(
        &self,
        service_name: &str,
        address: Address,
        credentials: KeyDynValueMap,
    ) -> Result<Session, Error> {
        let lock = Arc::clone(self.connecting.lock().await.entry(address).or_default());
        let _connecting = lock.lock().await;
        let session = match self.get_by_address(&address).await {
            Some(session) => session,
            None => {
                let session = self.create(address, credentials).await?;
                self.by_address
                    .lock()
                    .await
                    .insert(address, session.downgrade());
                session
            }
        };
        self.sessions
            .lock()
            .await
            .insert(service_name.to_owned(), session.downgrade());
        Ok(session)
    }

    async fn get(&self, name: &str) -> Option<Session> {
        let mut sessions = self.sessions.lock().await;
        match sessions.get(name) {
            Some(weak) => {
                let session = weak.upgrade();
                if session.is_none() {
                    sessions.remove(name);
                }
                session
            }
            None => None,
        }
    }

    /// Gets a session to the given targets, using the service name to store
    /// any created session for further retrieval.
    pub(crate) async fn get_or_create<Targets>(
        &self,
        service_name: &str,
        targets: Targets,
        credentials: KeyDynValueMap,
    ) -> Result<Session, Error>
    where
        for<'t> &'t Targets: IntoIterator<Item = &'t session::Target>,
    {
        let targets = targets.into_iter();
        let mut connection_errors = Vec::with_capacity({
            let (lower_bound, upper_bound) = targets.size_hint();
            upper_bound.unwrap_or(lower_bound)
        });
        for target in targets {
            match target.kind() {
                Kind::Service(service) => {
                    if let Some(session) = self.get(service).await {
                        return Ok(session);
                    }
                }
                Kind::Endpoint(address) => {
                    match self
                        .get_or_connect(service_name, *address, credentials.clone())
                        .await
                    {
                        Err(err) => connection_errors.push(ConnectionError {
                            address: *address,
                            source: err.into(),
                        }),
                        Ok(session) => return Ok(session),
                    }
                }
            }
        }
        Err(UnreachableServiceError(service_name.to_owned(), connection_errors).into())
    }

    /// Opens a new channel to the address and connects a session client over
    /// it.
    async fn create(
        &self,
        address: messaging::Address,
        credentials: KeyDynValueMap,
    ) -> Result<Session, Error> {
        let (messages_in, messages_out) = messaging::channel::connect(address).await?;
        Session::connect(
            messages_in,
            messages_out,
            credentials,
            self.services.clone(),
        )
        .await
    }
}

#[derive(Debug)]
struct UnreachableServiceError(String, Vec<ConnectionError>);

impl std::fmt::Display for UnreachableServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "could not reach service \"{}\", ", self.0)?;
        if self.1.is_empty() {
            f.write_str("no connection was tried")
        } else {
            f.write_str("tried the following connections: [")?;
            for (index, error) in self.1.iter().enumerate() {
                if index > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{} => {}", error.address, error.source)?;
            }
            f.write_str("]")
        }
    }
}

impl std::error::Error for UnreachableServiceError {}

impl From<UnreachableServiceError> for Error {
    fn from(err: UnreachableServiceError) -> Self {
        Self::Other(err.into())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("could not connect to address \"{address}\"")]
pub(super) struct ConnectionError {
    address: Address,
    source: Box<dyn std::error::Error + Send + Sync>,
}
