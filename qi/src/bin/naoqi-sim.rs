//! `naoqi-sim`: a simulated NAOqi robot served over the qi protocol.

#![deny(unreachable_pub, unsafe_code)]
#![warn(clippy::all, clippy::print_stderr)]

use anyhow::Context as _;
use clap::Parser;
use qi::naoqi_sim::{Config, RobotModel, Script, Simulator};
use qi::Address;
use std::path::PathBuf;
use tracing_subscriber::{
    filter::LevelFilter, filter::Targets, layer::SubscriberExt, util::SubscriberInitExt,
};

/// A simulated NAOqi robot (NAO or Pepper) that libqi clients connect to like a real one.
#[derive(Parser, Debug)]
#[command(name = "naoqi-sim", about, long_about = None, disable_version_flag = true)]
struct Args {
    /// The model of the robot: nao or pepper.
    #[arg(long, default_value = "nao")]
    robot: RobotModel,

    /// The NAOqi version the robot reports (default: 2.8.7.4 for NAO, 2.9.5.1 for Pepper).
    #[arg(long)]
    version: Option<String>,

    /// The name of the robot.
    #[arg(long, default_value = "naoqi-sim")]
    name: String,

    /// An address to listen on; may be repeated. Defaults to tcp://0.0.0.0:9559, plus
    /// tcps://0.0.0.0:9503 (TLS, the port NAOqi 2.9 clients use with a password) when a
    /// password is set.
    #[arg(long = "listen")]
    listen: Vec<Address>,

    /// The password of the "nao" user. Without it, every connection is accepted.
    #[arg(long)]
    password: Option<String>,

    /// A scenario script to run once the simulator is started ("-" reads the standard input).
    #[arg(long)]
    script: Option<PathBuf>,

    /// Increases the verbosity of the logs; may be repeated.
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn init_tracing(verbose: u8) {
    let (sim, qi, others) = match verbose {
        0 => (LevelFilter::INFO, LevelFilter::WARN, LevelFilter::WARN),
        1 => (LevelFilter::DEBUG, LevelFilter::INFO, LevelFilter::WARN),
        2 => (LevelFilter::TRACE, LevelFilter::DEBUG, LevelFilter::INFO),
        _ => (LevelFilter::TRACE, LevelFilter::TRACE, LevelFilter::TRACE),
    };
    let targets = Targets::new()
        .with_target("naoqi_sim", sim)
        .with_target("qi", qi)
        .with_default(others);
    tracing_subscriber::registry()
        .with(targets)
        .with(
            tracing_subscriber::fmt::layer()
                .compact()
                .with_target(verbose > 0),
        )
        .init();
}

async fn read_script(path: &PathBuf) -> anyhow::Result<Script> {
    let text = if path.as_os_str() == "-" {
        let mut text = String::new();
        tokio::io::AsyncReadExt::read_to_string(&mut tokio::io::stdin(), &mut text)
            .await
            .context("cannot read the script from the standard input")?;
        text
    } else {
        tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("cannot read the script {}", path.display()))?
    };
    text.parse().context("invalid script")
}

/// The default addresses: the NAOqi ports, TLS only when a password protects the robot.
fn default_listen(with_password: bool) -> anyhow::Result<Vec<Address>> {
    let mut addresses = vec!["tcp://0.0.0.0:9559".parse::<Address>()?];
    if with_password {
        addresses.push("tcps://0.0.0.0:9503".parse::<Address>()?);
    }
    Ok(addresses)
}

#[allow(clippy::print_stdout)]
fn announce(simulator: &Simulator) {
    for endpoint in simulator.endpoints() {
        println!(
            "naoqi-sim: {} robot \"{}\" (NAOqi {}) listening on {endpoint}",
            simulator.robot().body_type(),
            simulator.config().name,
            simulator.version()
        );
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    init_tracing(args.verbose);
    let listen = if args.listen.is_empty() {
        default_listen(args.password.is_some())?
    } else {
        args.listen
    };
    let script = match &args.script {
        Some(path) => Some(read_script(path).await?),
        None => None,
    };
    let mut config = Config::new(args.robot)
        .with_name(args.name)
        .listen_on(listen);
    if let Some(version) = args.version {
        config = config.with_version(version);
    }
    if let Some(password) = args.password {
        config = config.with_password(password);
    }
    let simulator = Simulator::start(config)
        .await
        .context("cannot start the simulator")?;
    announce(&simulator);

    let script_task = async {
        if let Some(script) = &script {
            tracing::info!(steps = script.steps.len(), "running the script");
            if let Err(err) = simulator.run_script(script).await {
                tracing::error!(error = %err, "the script failed");
            } else {
                tracing::info!("the script completed");
            }
        }
        std::future::pending::<()>().await;
    };
    tokio::select! {
        () = script_task => {}
        result = tokio::signal::ctrl_c() => {
            result.context("cannot listen to the interrupt signal")?;
            tracing::info!("interrupted, shutting down");
        }
    }
    simulator.shutdown().await;
    Ok(())
}
