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

use anyhow::{anyhow, bail, Context as _, Result};
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
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

const AFTER_HELP: &str = "\
Modules are found like libqi finds them: the module NAME is declared by the file \
share/qi/module/NAME.mod (containing `cpp`) of an SDK prefix (--qi-sdk-prefix or QI_SDK_PREFIX, \
the prefix of the program, QI_ADDITIONAL_SDK_PREFIXES), and its library is lib/libNAME.so (a \
module a.b is lib/a/libb.so). qi-launch loads the modules written in Rust with #[qi::module]; \
C++ modules need qilaunch.

LIBRARIES (positional arguments) are loaded the way the qi-launch of NAOqi 2.1 loads them: every \
factory of the library is registered as a service.";

/// Loads qi modules: registers the objects of their factories as services and calls their
/// functions, like qilaunch.
#[derive(Debug, Parser)]
#[command(name = "qi-launch", version, about, after_help = AFTER_HELP)]
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

    /// Listen on all interfaces (the -a of the qi-launch of NAOqi 2.1): tcp://0.0.0.0:0.
    #[arg(short = 'a', long)]
    all: bool,

    /// The URL of the service directory to connect to (a host alone is tcp://HOST:9559).
    /// Defaults to the QI_URL environment variable, else tcp://127.0.0.1:9559.
    #[arg(long = "qi-url", value_name = "URL")]
    qi_url: Option<String>,

    /// The URLs to listen on, separated by `;` (can be set multiple times). The services of the
    /// objects are served there. Defaults to QI_LISTEN_URL, else tcp://127.0.0.1:0 (only local
    /// processes reach the services, as with libqi), and tcp://127.0.0.1:9559 with
    /// --qi-standalone.
    #[arg(long = "qi-listen-url", value_name = "URL", value_delimiter = ';')]
    qi_listen_urls: Vec<String>,

    /// Host a standalone space instead of connecting to one.
    #[arg(long = "qi-standalone")]
    qi_standalone: bool,

    /// The SDK prefix where modules are searched. Defaults to QI_SDK_PREFIX.
    #[arg(long = "qi-sdk-prefix", value_name = "PREFIX")]
    qi_sdk_prefix: Option<PathBuf>,

    /// The level of the logs: 0 to 6, or silent, fatal, error, warning, info, verbose or debug
    /// (or filter directives of tracing, such as "qi=debug"). Defaults to QI_LOG_LEVEL, else
    /// info.
    #[arg(long = "qi-log-level", value_name = "LEVEL")]
    log_level: Option<String>,

    /// Libraries to load like the qi-launch of NAOqi 2.1 does: a path, or a name found in the
    /// lib directories of the prefixes.
    #[arg(value_name = "LIBRARIES")]
    libraries: Vec<String>,
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
        if module.name() != name {
            bail!(
                "module name do not match: '{}' vs '{name}' (the library {} is another module)",
                module.name(),
                info.path.display()
            );
        }
        self.loaded.insert(name.to_owned(), Arc::clone(&module));
        Ok(module)
    }

    /// Loads a library by path or name, as `Session::loadService` of libqi 2.1 does.
    async fn load_library(&mut self, library: &str) -> Result<Arc<Module>> {
        let path = self
            .finder
            .find_library(library)
            .unwrap_or_else(|| PathBuf::from(library));
        let module = Arc::new(Module::load(&path, Some(&self.space_url)).await?);
        self.loaded
            .entry(module.name().to_owned())
            .or_insert_with(|| Arc::clone(&module));
        Ok(module)
    }
}

/// Splits `module.member` at its last dot, as `libqi` does.
fn split_member(full: &str) -> (&str, &str) {
    full.rsplit_once('.').unwrap_or((full, full))
}

/// Completes a URL like libqi does: the scheme, host and port missing are the default ones.
fn complete_url(url: &str, default_host: &str, default_port: u16) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("tcp", url));
    let rest = if rest.is_empty() { default_host } else { rest };
    let has_port = rest
        .rsplit_once(':')
        .is_some_and(|(host, port)| !host.ends_with(':') && port.parse::<u16>().is_ok());
    if has_port {
        format!("{scheme}://{rest}")
    } else {
        format!("{scheme}://{rest}:{default_port}")
    }
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
    // The signals are handled from the start, as libqi 2.5 to 2.9 do: the first one cancels the
    // function calls and stops the launcher, the second one forces the exit.
    let cancel = CancellationToken::new();
    let stop = Arc::new(Notify::new());
    tokio::spawn({
        let (cancel, stop) = (cancel.clone(), Arc::clone(&stop));
        async move {
            signal().await;
            info!("Signal received, exiting...");
            cancel.cancel();
            stop.notify_one();
            signal().await;
            info!("Signal received again, forcing exit...");
            #[allow(clippy::exit)]
            std::process::exit(1);
        }
    });

    if cli.name.is_empty() {
        error!("No launcher name, set one with --name or -n");
        return 1;
    }
    if !cli.modules.is_empty() {
        warn!("--module is deprecated, use --object");
    }
    let mut objects = std::mem::take(&mut cli.objects);
    objects.append(&mut cli.modules);
    if objects.is_empty() && cli.functions.is_empty() && cli.libraries.is_empty() {
        error!("No object to load and no function to call, add one with --object or --function");
        return 1;
    }
    if cli.qi_standalone && cli.qi_url.is_some() {
        error!("You cannot specify both --qi-standalone and --qi-url to connect.");
        return 1;
    }
    if cli.qt_eventloop {
        warn!("--qt-eventloop has no effect: qi-launch has no Qt event loop");
    }

    let exit_status = Arc::new(AtomicU8::new(0));
    let mut calls = JoinSet::new();
    let context = Context {
        cancel: cancel.clone(),
        stop: Arc::clone(&stop),
        exit_status: Arc::clone(&exit_status),
    };
    // The space and the modules live until the launcher exits.
    let _launched = match launch(&cli, &objects, &mut calls, &context).await {
        Ok(launched) => launched,
        Err(err) => {
            error!("{err:#}");
            return 1;
        }
    };

    if !objects.is_empty() || !cli.libraries.is_empty() || cli.keep_running {
        stop.notified().await;
        cancel.cancel();
    }
    // Waits for the functions, canceled or not.
    while calls.join_next().await.is_some() {}
    exit_status.load(Ordering::SeqCst)
}

/// What the function calls share with the launcher.
struct Context {
    cancel: CancellationToken,
    stop: Arc<Notify>,
    exit_status: Arc<AtomicU8>,
}

/// Connects to the space, loads the objects and starts the function calls.
async fn launch(
    cli: &Cli,
    objects: &[String],
    calls: &mut JoinSet<()>,
    context: &Context,
) -> Result<(Space, Modules)> {
    let default_listen_url = if cli.qi_standalone {
        "tcp://127.0.0.1:9559"
    } else if cli.all {
        "tcp://0.0.0.0:0"
    } else {
        "tcp://127.0.0.1:0"
    };
    let listen_urls: Vec<String> = if !cli.qi_listen_urls.is_empty() {
        cli.qi_listen_urls.clone()
    } else if let Some(env) = std::env::var("QI_LISTEN_URL")
        .ok()
        .filter(|url| !url.is_empty())
    {
        env.split(';').map(str::to_owned).collect()
    } else {
        vec![default_listen_url.to_owned()]
    };
    let listen_urls: Vec<String> = listen_urls
        .iter()
        .map(|url| complete_url(url, "127.0.0.1", if cli.qi_standalone { 9559 } else { 0 }))
        .collect();
    let mut node = qi::node::init();
    // Like libqi, the launcher listens only when it has services to serve.
    if cli.qi_standalone || !objects.is_empty() || !cli.libraries.is_empty() {
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
        let url = complete_url(
            &cli.qi_url
                .clone()
                .unwrap_or_else(qi::module::default_space_url),
            "127.0.0.1",
            9559,
        );
        info!("Connection to service directory at {url}");
        let addresses = qi::Address::resolve_all(&url).context("invalid --qi-url")?;
        let node = node
            .connect_to_space_at(addresses, None)
            .start()
            .await
            .with_context(|| format!("Connect error: cannot connect to {url}"))?;
        (Space::Connected(node), url)
    };

    let mut modules = Modules {
        finder: Finder::from_environment(cli.qi_sdk_prefix.clone()),
        space_url,
        loaded: HashMap::new(),
    };

    for library in &cli.libraries {
        info!("loading {library}");
        let module = modules.load_library(library).await?;
        for factory in module.factories().to_vec() {
            let service: AnyObject = module
                .object()
                .call(factory.as_str(), ())
                .await
                .with_context(|| format!("cannot create the object {factory}"))?;
            space
                .register_service(&factory, service)
                .await
                .with_context(|| format!("cannot register the service {factory}"))?;
        }
    }

    for object in objects {
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
        let (cancel, stop, exit_status) = (
            context.cancel.clone(),
            Arc::clone(&context.stop),
            Arc::clone(&context.exit_status),
        );
        calls.spawn(async move {
            let call = module.object().meta_call(member.into(), Value::Tuple(args));
            let failure = tokio::select! {
                result = call => result.err().map(|err| format!("{function} has finished with an error: {err}")),
                () = cancel.cancelled() => Some(format!("{function} has been canceled")),
            };
            if let Some(failure) = failure {
                error!("{failure}");
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

/// The filter of the logs, from a libqi log level (`--qi-log-level`, `QI_LOG_LEVEL`) or
/// directives of tracing.
fn log_filter(level: &str) -> Result<EnvFilter> {
    let directive = match level.trim() {
        "0" | "silent" => "off",
        "1" | "fatal" | "2" | "error" => "error",
        "3" | "warning" => "warn",
        "4" | "info" => "info",
        "5" | "verbose" => "debug",
        "6" | "debug" => "trace",
        other => other,
    };
    Ok(EnvFilter::try_new(directive)?)
}

fn init_logging(level: Option<&str>) -> Result<()> {
    let level = level
        .map(str::to_owned)
        .or_else(|| std::env::var("QI_LOG_LEVEL").ok())
        .unwrap_or_else(|| "info".to_owned());
    tracing_subscriber::fmt()
        .with_env_filter(log_filter(&level)?)
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

    #[test]
    fn completes_urls_like_libqi() {
        assert_eq!(
            complete_url("10.0.0.2", "127.0.0.1", 9559),
            "tcp://10.0.0.2:9559"
        );
        assert_eq!(
            complete_url("tcps://nao.local", "127.0.0.1", 9559),
            "tcps://nao.local:9559"
        );
        assert_eq!(
            complete_url("tcp://127.0.0.1:0", "127.0.0.1", 9559),
            "tcp://127.0.0.1:0"
        );
        assert_eq!(
            complete_url("nao:9503", "127.0.0.1", 9559),
            "tcp://nao:9503"
        );
    }

    #[test]
    fn parses_libqi_log_levels() {
        for level in [
            "0", "silent", "fatal", "warning", "verbose", "6", "qi=debug",
        ] {
            assert!(log_filter(level).is_ok(), "{level}");
        }
    }
}
