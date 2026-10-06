use crate::{
    auth::Authenticator,
    messaging::Address,
    service::SharedServices,
    session::{self, Protocol},
};
use std::{collections::HashMap, sync::Arc};
use tokio::{sync::watch, task};

/// A set of servers with their aggregated endpoints.
///
/// Drops all server tasks when the set is dropped.
#[derive(Default, Debug)]
pub(super) struct ServerSet {
    _servers: Vec<session::Server>,
    _update_endpoints_tasks: task::JoinSet<()>,
}

pub(super) type EndpointsWatcher = watch::Receiver<HashMap<Address, Vec<Address>>>;

/// Instantiates a set of servers for a list of addresses and aggregates their endpoints.
///
/// For each server created, a task is spawned that will track changes to its endpoints.
///
/// If any server fails to bind to its address, then the future terminates with an error and all
/// created servers are stopped.
pub(super) async fn start_servers(
    services: SharedServices,
    authenticator: Option<Arc<dyn Authenticator + Send + Sync>>,
    protocol: Protocol,
    addresses: impl IntoIterator<Item = Address>,
) -> Result<(ServerSet, EndpointsWatcher), std::io::Error> {
    let (endpoints_sender, endpoints_receiver) = watch::channel(Default::default());
    let mut servers = Vec::new();
    let mut update_endpoints_tasks = task::JoinSet::new();
    for address in addresses {
        let (server, mut server_endpoints) =
            session::server(address, authenticator.clone(), protocol, services.clone()).await?;
        servers.push(server);
        // Publish the initial endpoints synchronously, so that they are known when the servers
        // are started.
        {
            let server_endpoints = server_endpoints.borrow_and_update();
            endpoints_sender.send_modify(|endpoints: &mut HashMap<_, _>| {
                endpoints.insert(server_endpoints.0, server_endpoints.1.clone());
            });
        }
        let endpoints_sender = endpoints_sender.clone();
        update_endpoints_tasks.spawn(async move {
            while let Ok(()) = server_endpoints.changed().await {
                endpoints_sender.send_modify(|endpoints: &mut HashMap<_, _>| {
                    let server_endpoints = server_endpoints.borrow_and_update();
                    endpoints.insert(server_endpoints.0, server_endpoints.1.clone());
                });
            }
        });
    }
    Ok((
        ServerSet {
            _servers: servers,
            _update_endpoints_tasks: update_endpoints_tasks,
        },
        endpoints_receiver,
    ))
}
