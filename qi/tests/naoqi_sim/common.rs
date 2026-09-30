//! Helpers shared by the integration tests: a simulator on the loopback and a client node.

#![allow(dead_code)]

use qi::naoqi_sim::{Config, RobotModel, Simulator};
use qi::{node::Node, service_directory::Client, AnyObject, ObjectExt};
use std::time::Duration;

/// The longest time a test waits for an asynchronous event.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// A simulator with a client node connected to it.
pub struct Fixture {
    pub simulator: Simulator,
    pub client: Node<Client>,
}

impl Fixture {
    /// Starts a simulator of the given model on the loopback, and connects a client node to it.
    pub async fn start(robot: RobotModel) -> Self {
        Self::with_config(Config::new(robot).on_loopback()).await
    }

    /// Starts a simulator with the given configuration and connects a client node to it.
    pub async fn with_config(config: Config) -> Self {
        let simulator = Simulator::start(config).await.expect("start the simulator");
        let address = simulator.address().expect("the simulator has an endpoint");
        let client = qi::node::init()
            .connect_to_space(address, None)
            .start()
            .await
            .expect("connect to the simulator");
        Self { simulator, client }
    }

    /// Gets a service of the simulator through the client node.
    pub async fn service(&self, name: &str) -> AnyObject {
        self.client
            .service(name)
            .await
            .unwrap_or_else(|err| panic!("service {name}: {err}"))
    }

    /// Reads a memory key through `ALMemory.getData`.
    pub async fn get_data<R>(&self, key: &str) -> R
    where
        R: qi::value::FromValue<'static> + qi::value::Reflect,
    {
        self.service("ALMemory")
            .await
            .call("getData", key.to_owned())
            .await
            .unwrap_or_else(|err| panic!("getData({key}): {err}"))
    }
}

/// Waits until a condition holds, polling it, or panics after the timeout.
pub async fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(TIMEOUT, async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}
