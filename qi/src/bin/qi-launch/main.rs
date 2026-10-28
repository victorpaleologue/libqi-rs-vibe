//! `qi-launch`: the counterpart of `qilaunch`, the launcher of `qi` modules of NAOqi.
//!
//! It connects to a space, loads modules, registers the objects their factories create as
//! services (`--object module.Factory`) and calls their functions (`--function
//! 'module.function(json arguments)'`), with the options and the behavior of `qilaunch`. It loads
//! modules written in Rust with `#[qi::module]`; C++ modules need `qilaunch` itself.

#![deny(unreachable_pub, unsafe_code)]
#![warn(
    clippy::all,
    clippy::clone_on_ref_ptr,
    clippy::dbg_macro,
    clippy::exit,
    clippy::get_unwrap,
    clippy::implicit_clone,
    clippy::map_err_ignore,
    clippy::mem_forget,
    clippy::mod_module_files,
    clippy::panic,
    clippy::str_to_string,
    clippy::string_slice,
    clippy::todo,
    clippy::unimplemented,
    clippy::use_debug
)]

#[path = "../qi-cli/json.rs"]
#[allow(dead_code)]
mod json;

use anyhow::{anyhow, Context as _, Result};
use clap::Parser;
use qi::{
    module::host::{Finder, Module},
    value::{Type, Value},
    AnyObject, Object, ObjectExt,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    process::ExitCode,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
};
use tokio::{sync::Notify, task::JoinSet};
use tracing::{debug, error, info, warn};
use tracing_subscriber::{filter::LevelFilter, EnvFilter};

/// Loads qi modules: registers the objects of their factories as services and calls their
/// functions.
#[derive(Debug, Parser)]
#[command(name = "qi-launch", version, about)]
struct Cli {
    /// Load given module (can be set multiple times) (deprecated, use --object).
    #[arg(short = 'm', long = "module", value_name = "MODULE")]
    modules: Vec<String>,

    /// Load given object (syntax: yourmodule.YourObject) (can be set multiple times).
    #[arg(short = 'o', long = "object", value_name = "OBJECT")]
    objects: Vec<String>,

    /// Call given function (syntax: yourmodule.yourFunction) (can be set multiple times).
    ///
    /// Arguments are given as JSON values between parentheses:
    /// 'yourmodule.yourFunction(1, "text", [true])'.
    #[arg(short = 'f', long = "function", value_name = "FUNCTION")]
    functions: Vec<String>,

    /// Keep running after the function(s) has(have) returned (implied by -o).
    #[arg(short = 'k', long)]
    keep_running: bool,

    /// Disable breakpad (accepted for compatibility with qilaunch; no effect).
    #[arg(short = 'b', long)]
    no_breakpad: bool,

    /// Disable remote logging (accepted for compatibility with qilaunch; no effect).
    #[arg(short = 'l', long)]
    no_logging: bool,

    /// Enable a Qt Eventloop (accepted for compatibility with qilaunch; no effect).
    #[arg(short = 'q', long)]
    qt_eventloop: bool,

    /// Name of the launcher used to prefix logs and breakpad dump files.
    #[arg(short = 'n', long, value_name = "NAME", default_value = "")]
    name: String,

    /// The URL of the service directory to connect to. Defaults to the QI_URL environment
    /// variable, else tcp://127.0.0.1:9559.
    #[arg(long = "qi-url", value_name = "URL")]
    qi_url: Option<String>,

    /// The URLs to listen on, separated by `;` (can be set multiple times). The services of the
    /// objects are served there. Defaults to tcp://127.0.0.1:0 (only local processes reach the
    /// services, as with libqi), and to tcp://127.0.0.1:9559 with --qi-standalone.
    #[arg(long = "qi-listen-url", value_name = "URL", value_delimiter = ';')]
    qi_listen_urls: Vec<String>,

    /// Host a standalone space instead of connecting to one.
    #[arg(long = "qi-standalone")]
    qi_standalone: bool,

    /// The SDK prefix where modules are searched (share/qi/module/<name>.mod, lib/lib<name>.so).
    /// Defaults to QI_SDK_PREFIX, else the parent of the directory of the program; the prefixes
    /// of QI_ADDITIONAL_SDK_PREFIXES are searched too.
    #[arg(long = "qi-sdk-prefix", value_name = "PREFIX")]
    qi_sdk_prefix: Option<PathBuf>,

    /// The level of the logs printed on the standard error: error, warn, info, debug or trace.
    /// Defaults to RUST_LOG, else "info".
    #[arg(long = "qi-log-level", value_name = "LEVEL")]
    log_level: Option<String>,
}

/// The space the launcher belongs to: a node connected to it, or hosting it.
enum Space {
    Connected(qi::Node<qi::service_directory::Client>),
    Hosted(qi::Node<qi::service_directory::LocalServiceDirectory>),
}

impl Space {
    async fn register_service(&self, name: &str, object: AnyObject) -> qi::Result<()> {
        match self {
            Self::Connected(node) => node.register_service_object(name, object).await,
            Self::Hosted(node) => node.register_service_object(name, object).await,
        }
        .map(|_| ())
    }
}

/// The modules loaded by the launcher, by name: a module is loaded once (`qi::import`).
struct Modules {
    finder: Finder,
    space_url: String,
    loaded: HashMap<String, Arc<Module>>,
}

impl Modules {
    async fn import(&mut self, name: &str) -> Result<Arc<Module>> {
        if let Some(module) = self.loaded.get(name) {
            debug!("Module {name} already loaded.");
            return Ok(Arc::clone(module));
        }
        let info = self.finder.find(name)?;
        let module = Arc::new(Module::load(&info.path, Some(&self.space_url)).await?);
        self.loaded.insert(name.to_owned(), Arc::clone(&module));
        Ok(module)
    }
}

/// Splits `module.member` at its last dot, as `libqi` does.
fn split_member(full: &str) -> (&str, &str) {
    full.rsplit_once('.').unwrap_or((full, full))
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Err(err) = init_logging(cli.log_level.as_deref()) {
        eprintln!("qi-launch: {err:#}");
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            error!("cannot start the runtime: {err}");
            return ExitCode::FAILURE;
        }
    };
    let status = runtime.block_on(run(cli));
    // Modules run threads of their own, that do not stop with this runtime.
    runtime.shutdown_background();
    ExitCode::from(status)
}

async fn run(mut cli: Cli) -> u8 {
    if cli.name.is_empty() {
        error!("No launcher name, set one with --name or -n");
        return 1;
    }
    if !cli.modules.is_empty() {
        warn!("--module is deprecated, use --object");
    }
    let mut objects = std::mem::take(&mut cli.objects);
    objects.append(&mut cli.modules);
    if objects.is_empty() && cli.functions.is_empty() {
        error!("No object to load and no function to call, add one with --object or --function");
        return 1;
    }
    if cli.qt_eventloop {
        warn!("--qt-eventloop has no effect: qi-launch has no Qt event loop");
    }

    let stop = Arc::new(Notify::new());
    let exit_status = Arc::new(AtomicU8::new(0));
    let mut calls = JoinSet::new();
    // The space and the modules live until the launcher exits.
    let _launched = match launch(&cli, objects.clone(), &mut calls, &stop, &exit_status).await {
        Ok(launched) => launched,
        Err(err) => {
            error!("{err:#}");
            return 1;
        }
    };

    let keep_running = !objects.is_empty() || cli.keep_running;
    tokio::select! {
        () = wait_for_stop(&stop), if keep_running => {}
        () = async { while calls.join_next().await.is_some() {} }, if !keep_running => {}
        () = signal() => {
            info!("Signal received, exiting...");
            calls.abort_all();
            // A second signal forces the exit.
            tokio::spawn(async {
                signal().await;
                info!("Signal received again, forcing exit...");
                #[allow(clippy::exit)]
                std::process::exit(1);
            });
        }
    }
    calls.abort_all();
    exit_status.load(Ordering::SeqCst)
}

async fn wait_for_stop(stop: &Notify) {
    stop.notified().await
}

/// Connects to the space, loads the objects and starts the function calls.
async fn launch(
    cli: &Cli,
    objects: Vec<String>,
    calls: &mut JoinSet<()>,
    stop: &Arc<Notify>,
    exit_status: &Arc<AtomicU8>,
) -> Result<(Space, Modules)> {
    let default_listen_url = if cli.qi_standalone {
        "tcp://127.0.0.1:9559"
    } else {
        "tcp://127.0.0.1:0"
    };
    let listen_urls = if cli.qi_listen_urls.is_empty() {
        vec![default_listen_url.to_owned()]
    } else {
        cli.qi_listen_urls.clone()
    };
    let mut node = qi::node::init();
    // Like libqi, the launcher listens only when it has services to serve.
    if cli.qi_standalone || !objects.is_empty() {
        for url in &listen_urls {
            for address in qi::Address::resolve_all(url)
                .with_context(|| format!("invalid --qi-listen-url {url}"))?
            {
                node.bind(address);
            }
        }
    }
    let (space, space_url) = if cli.qi_standalone {
        info!("Starting a standalone space at {}", listen_urls.join(";"));
        let node = node.host_space().start().await?;
        let url = node
            .endpoints()
            .iter()
            .map(ToString::to_string)
            .find(|endpoint| endpoint.starts_with("tcp"))
            .unwrap_or_else(|| default_listen_url.to_owned());
        (Space::Hosted(node), url)
    } else {
        let url = cli
            .qi_url
            .clone()
            .unwrap_or_else(qi::module::default_space_url);
        info!("Connection to service directory at {url}");
        let addresses = qi::Address::resolve_all(&url).context("invalid --qi-url")?;
        let node = node
            .connect_to_space_at(addresses, None)
            .start()
            .await
            .with_context(|| format!("cannot connect to the service directory at {url}"))?;
        (Space::Connected(node), url)
    };

    let mut modules = Modules {
        finder: Finder::from_environment(cli.qi_sdk_prefix.clone()),
        space_url,
        loaded: HashMap::new(),
    };

    for object in &objects {
        info!("Loading object {object}");
        let (module_name, factory) = split_member(object);
        let module = modules.import(module_name).await?;
        let service: AnyObject = module
            .object()
            .call(factory, ())
            .await
            .with_context(|| format!("cannot create the object {object}"))?;
        debug!(object, "object created");
        space
            .register_service(factory, service)
            .await
            .with_context(|| format!("cannot register the service {factory}"))?;
    }

    for function in &cli.functions {
        let (name, args_text) = parse_function(function);
        let (module_name, member) = split_member(name);
        let module = modules.import(module_name).await?;
        let args = match parse_arguments(module.object(), member, args_text) {
            Ok(args) => args,
            Err(err) => {
                error!(
                    "Failed to parse arguments provided to the function call from JSON: [{args_text}]: {err:#}"
                );
                Vec::new()
            }
        };
        info!("Calling function {name}({args_text})");
        let module = Arc::clone(&module);
        let (member, function) = (member.to_owned(), function.clone());
        let (stop, exit_status) = (Arc::clone(stop), Arc::clone(exit_status));
        calls.spawn(async move {
            let result = module
                .object()
                .meta_call(member.into(), Value::Tuple(args))
                .await;
            if let Err(err) = result {
                error!("{function} has finished with an error: {err}");
                exit_status.store(1, Ordering::SeqCst);
                stop.notify_one();
            }
        });
    }
    Ok((space, modules))
}

/// Splits `name(arguments)` into the name and the text of the arguments, as `qilaunch` does
/// (the first `(` and the last `)`).
fn parse_function(function: &str) -> (&str, &str) {
    match function.split_once('(') {
        Some((name, rest)) => (name, rest.rsplit_once(')').map_or("", |(args, _)| args)),
        None => (function, ""),
    }
}

/// Converts the JSON arguments of a function to the types of its parameters.
fn parse_arguments(object: &dyn Object, member: &str, text: &str) -> Result<Vec<Value<'static>>> {
    let values: Vec<serde_json::Value> = serde_json::from_str(&format!("[{text}]"))?;
    let parameter_types =
        |method: &qi::object::MetaMethod| match method.parameters_signature.as_type() {
            Some(Type::Tuple(tuple)) => tuple.element_types(),
            other => vec![other.cloned()],
        };
    let methods: Vec<_> = object
        .meta()
        .methods
        .values()
        .filter(|method| method.name == member)
        .collect();
    let types = methods
        .iter()
        .find(|method| parameter_types(method).len() == values.len())
        .or_else(|| methods.first())
        .map(|method| parameter_types(method))
        .unwrap_or_default();
    values
        .iter()
        .enumerate()
        .map(|(index, json)| {
            json::to_value(json, types.get(index).and_then(Option::as_ref))
                .map_err(|err| anyhow!("argument {}: {err}", index + 1))
        })
        .collect()
}

async fn signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match (
            signal(SignalKind::interrupt()),
            signal(SignalKind::terminate()),
        ) {
            (Ok(mut interrupt), Ok(mut terminate)) => {
                tokio::select! {
                    _ = interrupt.recv() => {}
                    _ = terminate.recv() => {}
                }
            }
            _ => std::future::pending().await,
        }
    }
    #[cfg(not(unix))]
    {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

fn init_logging(level: Option<&str>) -> Result<()> {
    let filter = match level {
        Some(level) => EnvFilter::try_new(level)?,
        None => EnvFilter::builder()
            .with_default_directive(LevelFilter::INFO.into())
            .from_env_lossy(),
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init()
        .map_err(|err| anyhow!("cannot initialize the logs: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_functions_like_qilaunch() {
        assert_eq!(parse_function("mod.fn"), ("mod.fn", ""));
        assert_eq!(parse_function("mod.fn()"), ("mod.fn", ""));
        assert_eq!(
            parse_function(r#"mod.fn(1, "a(b)")"#),
            ("mod.fn", r#"1, "a(b)""#)
        );
        assert_eq!(split_member("a.b.c"), ("a.b", "c"));
    }
}
