//! The handle members of a module use to reach the space of their host.

use super::SpaceNode;
use crate::{service, Address, AnyObject, Error, Object, Result};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::OnceCell;
use tracing::{info, warn};

/// A handle to the space of the process that loaded a module, the counterpart of the
/// `qi::SessionPtr` that `libqi` gives to the members of C++ modules.
///
/// The handle is cheap to clone. The node behind it is connected to the space on first use, at
/// the URL the host gave when it loaded the module (`qilaunch` and `qi-launch` give theirs),
/// else at the `QI_URL` environment variable, else at the local NAOqi
/// ([`DEFAULT_SPACE_URL`](super::DEFAULT_SPACE_URL)).
#[derive(Clone, Default)]
pub struct Session(Arc<Inner>);

#[derive(Default)]
struct Inner {
    url: Mutex<Option<String>>,
    listen_url: Mutex<Option<String>>,
    node: OnceCell<SpaceNode>,
}

/// The URL the node of a session listens on, for the services the module registers: loopback
/// only, as `libqi` does.
const DEFAULT_LISTEN_URL: &str = "tcp://127.0.0.1:0";

impl Session {
    /// A handle connecting to the space at the given URL on first use.
    pub fn new(url: impl Into<String>) -> Self {
        let session = Self::default();
        session.set_url(url.into());
        session
    }

    /// Sets the URL of the space, unless the node is already connected; returns whether the URL
    /// is the one in use.
    pub(super) fn set_url(&self, url: String) -> bool {
        if self.0.node.initialized() {
            return self.url() == url;
        }
        *self.0.url.lock().expect("lock is not poisoned") = Some(url);
        true
    }

    /// Sets the URL the node listens on, unless it is already connected.
    pub(super) fn set_listen_url(&self, url: String) {
        *self.0.listen_url.lock().expect("lock is not poisoned") = Some(url);
    }

    /// The URL the node listens on to serve the services the module registers: the
    /// `QI_MODULE_LISTEN_URL` environment variable, else `tcp://0.0.0.0:0` for a module loaded
    /// by NAOqi (its services are reachable from other machines, like the ones of NAOqi), else
    /// `tcp://127.0.0.1:0`.
    pub fn listen_url(&self) -> String {
        std::env::var("QI_MODULE_LISTEN_URL")
            .ok()
            .filter(|url| !url.is_empty())
            .or_else(|| {
                self.0
                    .listen_url
                    .lock()
                    .expect("lock is not poisoned")
                    .clone()
            })
            .unwrap_or_else(|| DEFAULT_LISTEN_URL.to_owned())
    }

    /// The URL of the space.
    pub fn url(&self) -> String {
        self.0
            .url
            .lock()
            .expect("lock is not poisoned")
            .clone()
            .unwrap_or_else(super::default_space_url)
    }

    /// The node connected to the space, connecting it on first use.
    pub async fn node(&self) -> Result<&SpaceNode> {
        self.0
            .node
            .get_or_try_init(|| async {
                let url = self.url();
                let addresses = Address::resolve_all(&url).map_err(|err| {
                    Error::Other(format!("invalid space URL {url:?}: {err}").into())
                })?;
                let listen_url = self.listen_url();
                let mut node = crate::node::init();
                for address in Address::resolve_all(&listen_url).map_err(|err| {
                    Error::Other(format!("invalid listen URL {listen_url:?}: {err}").into())
                })? {
                    node.bind(address);
                }
                let node = node.connect_to_space_at(addresses, None).start().await?;
                info!(%url, "module connected to the space");
                Ok(node)
            })
            .await
    }

    /// Connects to the space, retrying until it answers. Used when the module is loaded at the
    /// start of its host, which may not accept connections yet (NAOqi loading its modules).
    pub(super) async fn node_with_retry(&self, delay: Duration) -> &SpaceNode {
        loop {
            match self.node().await {
                Ok(node) => return node,
                Err(err) => {
                    warn!(
                        url = %self.url(),
                        error = &err as &dyn std::error::Error,
                        "could not connect to the space, retrying"
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }

    /// Gets a service of the space by name.
    pub async fn service(&self, name: &str) -> Result<AnyObject> {
        self.node().await?.service(name).await
    }

    /// Registers a service to the space. It stays registered as long as the module is loaded,
    /// or until it is unregistered through [`Session::node`].
    pub async fn register_service<O>(
        &self,
        name: impl Into<String>,
        object: O,
    ) -> Result<service::Id>
    where
        O: Object + 'static,
    {
        self.node().await?.register_service(name, object).await
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("url", &self.url())
            .field("connected", &self.0.node.initialized())
            .finish()
    }
}
