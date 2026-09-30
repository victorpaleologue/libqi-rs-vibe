//! The simulator: a node hosting a space with the services of a simulated robot.

use crate::naoqi_sim::{
    alvalue::AlValue,
    body::Body,
    log::LogHub,
    memory::Memory,
    robot::{initial_memory, RobotModel},
    script::{Script, ScriptError},
    services::{Context, Services},
};
use qi::{
    auth::UserTokenAuthenticator, node::Node, service, service_directory::LocalServiceDirectory,
    Address,
};
use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::task::AbortHandle;

/// The user of the authentication, like on real robots.
pub const AUTH_USER: &str = "nao";

/// The default port of NAOqi service directories.
pub const DEFAULT_PORT: u16 = 9559;

/// The configuration of a simulator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// The model of the robot.
    pub robot: RobotModel,
    /// The NAOqi version the robot reports, the default of the model if `None`.
    pub version: Option<String>,
    /// The name of the robot.
    pub name: String,
    /// The addresses the node listens on.
    pub listen: Vec<Address>,
    /// The password of the `nao` user; without it, every connection is accepted.
    pub password: Option<String>,
    /// The period of the simulation of the body.
    pub tick_period: Duration,
    /// The period of the heartbeat log messages.
    pub heartbeat_period: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            robot: RobotModel::Nao,
            version: None,
            name: "naoqi-sim".to_owned(),
            listen: vec![Address::Tcp {
                address: SocketAddr::from((Ipv4Addr::UNSPECIFIED, DEFAULT_PORT)),
                ssl: None,
            }],
            password: None,
            tick_period: Duration::from_millis(20),
            heartbeat_period: Duration::from_secs(5),
        }
    }
}

impl Config {
    /// The default configuration of a robot model.
    pub fn new(robot: RobotModel) -> Self {
        Self {
            robot,
            ..Self::default()
        }
    }

    /// Listens on the loopback interface, on a port chosen by the system: for tests.
    pub fn on_loopback(mut self) -> Self {
        self.listen = vec![Address::Tcp {
            address: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            ssl: None,
        }];
        self
    }

    /// Listens on the given addresses instead of the default one.
    pub fn listen_on(mut self, addresses: Vec<Address>) -> Self {
        self.listen = addresses;
        self
    }

    /// Requires the given password from connecting nodes.
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Reports the given NAOqi version.
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Names the robot.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// The NAOqi version the robot reports.
    pub fn resolved_version(&self) -> String {
        self.version
            .clone()
            .unwrap_or_else(|| self.robot.default_version().to_owned())
    }
}

/// A running simulated robot.
///
/// The simulator hosts a space (it runs the service directory) on the configured addresses, and
/// registers the services of the robot on it. Dropping the simulator stops it.
pub struct Simulator {
    config: Config,
    node: Arc<Node<LocalServiceDirectory>>,
    context: Context,
    services: Services,
    service_ids: Vec<(&'static str, service::Id)>,
    tasks: Vec<AbortHandle>,
    started: Instant,
}

impl Simulator {
    /// Starts a simulator.
    pub async fn start(config: Config) -> qi::Result<Self> {
        let version = config.resolved_version();
        let description = config.robot.description();
        let memory = Memory::new();
        memory.insert_all_silent(initial_memory(description, &version, &config.name));
        let body = Body::new(description, memory.clone());
        let logs = LogHub::new();

        let mut init = qi::node::init();
        for address in &config.listen {
            init.bind(*address);
        }
        if let Some(password) = &config.password {
            init.with_authenticator(Arc::new(UserTokenAuthenticator::new(
                AUTH_USER.to_owned(),
                password.clone(),
            )));
        }
        let node = Arc::new(init.host_space().start().await?);

        let context = Context {
            robot: config.robot,
            version: version.clone(),
            name: config.name.clone(),
            memory,
            body: Arc::clone(&body),
            logs: logs.clone(),
            node: Arc::downgrade(&node),
        };
        let services = Services::new(&context);
        let mut service_ids = Vec::new();
        for (name, object) in services.objects() {
            let id = node.register_service_object(name, object.clone()).await?;
            service_ids.push((name, id));
        }

        let started = Instant::now();
        let tasks = vec![
            body.spawn_ticker(config.tick_period),
            spawn_heartbeat(&context, config.heartbeat_period, started),
        ];
        logs.info(
            "naoqi-sim",
            format!(
                "{} robot \"{}\" running NAOqi {version} with {} services",
                config.robot.body_type(),
                config.name,
                service_ids.len()
            ),
        );
        Ok(Self {
            config,
            node,
            context,
            services,
            service_ids,
            tasks,
            started,
        })
    }

    /// The configuration of the simulator.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The model of the robot.
    pub fn robot(&self) -> RobotModel {
        self.config.robot
    }

    /// The NAOqi version the robot reports.
    pub fn version(&self) -> &str {
        &self.context.version
    }

    /// The time elapsed since the start of the simulator.
    pub fn uptime(&self) -> Duration {
        self.started.elapsed()
    }

    /// The addresses clients may connect to.
    pub fn endpoints(&self) -> Vec<Address> {
        self.node
            .endpoints()
            .iter()
            .filter_map(|target| target.to_string().parse().ok())
            .collect()
    }

    /// An address clients on this machine may connect to, preferring the loopback interface.
    pub fn address(&self) -> Option<Address> {
        let endpoints = self.endpoints();
        endpoints
            .iter()
            .find(|address| address.is_machine_local())
            .or_else(|| endpoints.first())
            .copied()
    }

    /// The node of the simulator, which hosts the space.
    pub fn node(&self) -> &Arc<Node<LocalServiceDirectory>> {
        &self.node
    }

    /// The memory of the robot: read and write keys, raise events, subscribe to them.
    pub fn memory(&self) -> &Memory {
        &self.context.memory
    }

    /// The body of the robot: joints and odometry.
    pub fn body(&self) -> &Arc<Body> {
        &self.context.body
    }

    /// The log hub of the robot.
    pub fn logs(&self) -> &LogHub {
        &self.context.logs
    }

    /// The services of the robot and their states.
    pub fn services(&self) -> &Services {
        &self.services
    }

    /// The identifiers of the services in the space.
    pub fn service_ids(&self) -> &[(&'static str, service::Id)] {
        &self.service_ids
    }

    /// Raises a memory event, like `ALMemory.raiseEvent` does.
    pub fn raise<K, V>(&self, key: K, value: V)
    where
        K: Into<String>,
        V: Into<AlValue>,
    {
        self.context.memory.raise(key, value);
    }

    /// Presses or releases a touch sensor or bumper: raises its event with `1.0` or `0.0`.
    pub fn touch(&self, event: &str, pressed: bool) {
        self.raise(event, if pressed { 1.0f32 } else { 0.0 });
    }

    /// Everything the robot said so far.
    pub fn spoken(&self) -> Vec<String> {
        self.services.tts.spoken()
    }

    /// Runs a scenario script against the simulator.
    pub async fn run_script(&self, script: &Script) -> Result<(), ScriptError> {
        script.run(self).await
    }

    /// Stops the simulator: unregisters the services and stops the simulation tasks.
    pub async fn shutdown(self) {
        for (name, id) in &self.service_ids {
            if let Err(err) = self.node.unregister_service(*id).await {
                tracing::debug!(service = name, error = %err, "could not unregister service");
            }
        }
        self.context.logs.info("naoqi-sim", "shutting down");
    }
}

impl Drop for Simulator {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl std::fmt::Debug for Simulator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Simulator")
            .field("robot", &self.config.robot)
            .field("version", &self.context.version)
            .field("endpoints", &self.endpoints())
            .finish_non_exhaustive()
    }
}

/// Spawns the task emitting a log message periodically.
fn spawn_heartbeat(context: &Context, period: Duration, started: Instant) -> AbortHandle {
    let context = context.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(period.max(Duration::from_millis(100)));
        interval.tick().await;
        loop {
            interval.tick().await;
            let pose = context.body.pose();
            context.logs.info(
                "naoqi-sim",
                format!(
                    "heartbeat: up {}s, battery {}%, pose x={:.2} y={:.2} theta={:.2}",
                    started.elapsed().as_secs(),
                    context
                        .memory
                        .get_i32("BatteryChargeChanged")
                        .unwrap_or_default(),
                    pose.x,
                    pose.y,
                    pose.theta
                ),
            );
        }
    })
    .abort_handle()
}
