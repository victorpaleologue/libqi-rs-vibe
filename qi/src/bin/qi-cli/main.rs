//! `qi-cli`: a command-line tool to explore the services of a `qi` space and interact with them.
//!
//! See the README of the crate for the usage.

#![deny(unreachable_pub, unsafe_code)]
#![warn(
    clippy::all,
    clippy::clone_on_ref_ptr,
    clippy::dbg_macro,
    clippy::decimal_literal_representation,
    clippy::empty_drop,
    clippy::empty_structs_with_brackets,
    clippy::exit,
    clippy::float_cmp_const,
    clippy::format_push_string,
    clippy::get_unwrap,
    clippy::if_then_some_else_none,
    clippy::implicit_clone,
    clippy::integer_division,
    clippy::large_include_file,
    clippy::let_underscore_must_use,
    clippy::lossy_float_literal,
    clippy::map_err_ignore,
    clippy::mem_forget,
    clippy::mixed_read_write_in_expression,
    clippy::mod_module_files,
    clippy::multiple_inherent_impl,
    clippy::mutex_atomic,
    clippy::panic,
    clippy::rc_buffer,
    clippy::rc_mutex,
    clippy::rest_pat_in_fully_bound_structs,
    clippy::same_name_method,
    clippy::str_to_string,
    clippy::string_slice,
    clippy::todo,
    clippy::try_err,
    clippy::unimplemented,
    clippy::unnecessary_self_imports,
    clippy::unneeded_field_pattern,
    clippy::use_debug
)]

mod commands;
mod json;
mod meta;
mod output;

use anyhow::{anyhow, bail, Context as _, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use commands::{Context, InfoOptions};
use meta::MemberFilter;
use qi::value::KeyDynValueMap;
use std::{process::ExitCode, time::Duration};
use tracing_subscriber::{filter::LevelFilter, EnvFilter};

const LONG_ABOUT: &str = "\
Explores the services of a qi space and interacts with them: lists services and their members, \
calls methods, watches signals, and reads or writes properties.

Members are designated as SERVICE.MEMBER. Arguments and values are JSON documents, converted to \
the types advertised by the meta object of the service (a JSON number becomes an int32 or a float \
according to the signature of the method). For string parameters, text that is not a JSON string \
literal is taken as the string itself. Structures are given as arrays, or as objects when their \
fields are named; raw data as base64 strings.

Results are printed as JSON, except that in the default human-readable mode strings are printed \
as they are, raw data as a hexadecimal dump and nothing for void. Use --json for machine-readable \
output.";

/// Explores the services of a qi space and interacts with them.
#[derive(Debug, Parser)]
#[command(name = "qi-cli", version, about, long_about = LONG_ABOUT)]
struct Cli {
    /// The address of the service directory of the space.
    #[arg(
        long,
        visible_alias = "qi-url",
        global = true,
        default_value = "tcp://localhost:9559",
        value_name = "URL"
    )]
    url: qi::Address,

    /// The user to authenticate as, together with --token.
    #[arg(long, global = true, requires = "token", value_name = "USER")]
    user: Option<String>,

    /// The token of the user to authenticate as.
    #[arg(long, global = true, requires = "user", value_name = "TOKEN")]
    token: Option<String>,

    /// Prints JSON instead of human-readable renderings.
    #[arg(long, global = true)]
    json: bool,

    /// The level of the logs printed on the standard error: error, warn, info, debug or trace.
    ///
    /// Defaults to the RUST_LOG environment variable, or to "warn". Both accept tracing filter
    /// directives, such as "qi=debug".
    #[arg(long, global = true, value_name = "LEVEL")]
    log_level: Option<String>,

    /// The time given to connections to the space and to its services, in seconds.
    #[arg(long, global = true, default_value_t = 10.0, value_name = "SECONDS")]
    timeout: f64,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Lists the services of the space, or describes the given services.
    ///
    /// A service is described by its identifier, its name, the machine and process that host it,
    /// its endpoints, and the members of its object: methods, signals and properties.
    Info {
        /// The names of the services to describe. All the services when omitted.
        #[arg(value_name = "SERVICE")]
        services: Vec<String>,

        /// Shows the hidden members, whose names start with an underscore.
        #[arg(short = 'z', long)]
        hidden: bool,

        /// Shows the special members of the messaging protocol (identifiers below 100), the raw
        /// signatures and the descriptions of the members.
        #[arg(short, long)]
        details: bool,

        /// Only lists the identifiers and names of the services.
        #[arg(short, long, conflicts_with_all = ["hidden", "details"])]
        list: bool,
    },

    /// Calls a method and prints its result.
    Call {
        #[arg(value_name = "SERVICE.METHOD")]
        target: String,

        /// The arguments of the call, as JSON documents.
        #[arg(value_name = "JSON_ARG", allow_negative_numbers = true)]
        args: Vec<String>,
    },

    /// Calls a method, or emits a signal, without waiting for a result.
    Post {
        #[arg(value_name = "SERVICE.MEMBER")]
        target: String,

        /// The arguments of the call or the parameters of the signal, as JSON documents.
        #[arg(value_name = "JSON_ARG", allow_negative_numbers = true)]
        args: Vec<String>,
    },

    /// Subscribes to a signal, or to the changes of a property, and prints every value received
    /// on a line, until interrupted.
    Watch {
        #[arg(value_name = "SERVICE.SIGNAL")]
        target: String,

        /// Prefixes each value with the time it was received.
        #[arg(short, long)]
        time: bool,
    },

    /// Prints the value of a property.
    Get {
        #[arg(value_name = "SERVICE.PROPERTY")]
        target: String,
    },

    /// Sets the value of a property.
    Set {
        #[arg(value_name = "SERVICE.PROPERTY")]
        target: String,

        /// The new value, as a JSON document.
        #[arg(value_name = "JSON_VALUE", allow_negative_numbers = true)]
        value: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{} {error:#}", "error:".red().bold());
            ExitCode::FAILURE
        }
    }
}

#[tokio::main]
async fn run(cli: Cli) -> Result<()> {
    init_tracing(cli.log_level.as_deref())?;
    if !(cli.timeout.is_finite() && cli.timeout >= 0.0) {
        bail!("the timeout must be a positive number of seconds");
    }
    let timeout = Duration::from_secs_f64(cli.timeout);

    let credentials = cli.user.zip(cli.token).map(|(user, token)| {
        let mut credentials = KeyDynValueMap::new();
        credentials.set("auth_user", user);
        credentials.set("auth_token", token);
        credentials
    });
    let node = tokio::time::timeout(
        timeout,
        qi::node::init()
            .connect_to_space(cli.url, credentials)
            .start(),
    )
    .await
    .map_err(|_elapsed| anyhow!("timed out after {}", commands::seconds(timeout)))
    .and_then(|result| result.map_err(anyhow::Error::from))
    .with_context(|| format!("cannot connect to the space at {}", cli.url))?;
    let ctx = Context {
        node,
        json: cli.json,
        timeout,
    };

    match cli.command {
        Command::Info {
            services,
            hidden,
            details,
            list,
        } => {
            commands::info(
                &ctx,
                &services,
                InfoOptions {
                    list,
                    filter: MemberFilter { hidden, details },
                },
            )
            .await
        }
        Command::Call { target, args } => commands::call(&ctx, &target, &args).await,
        Command::Post { target, args } => commands::post(&ctx, &target, &args).await,
        Command::Watch { target, time } => commands::watch(&ctx, &target, time).await,
        Command::Get { target } => commands::get(&ctx, &target).await,
        Command::Set { target, value } => commands::set(&ctx, &target, &value).await,
    }
}

/// Prints the logs on the standard error, filtered by the `--log-level` option or the `RUST_LOG`
/// environment variable.
fn init_tracing(level: Option<&str>) -> Result<()> {
    let filter = match level {
        Some(level) => {
            EnvFilter::try_new(level).with_context(|| format!("invalid log level \"{level}\""))?
        }
        None => EnvFilter::builder()
            .with_default_directive(LevelFilter::WARN.into())
            .from_env()
            .context("invalid RUST_LOG environment variable")?,
    };
    tracing_subscriber::fmt()
        .compact()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init()
        .map_err(|error| anyhow!("cannot initialize the logs: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_parses() {
        let cli = Cli::try_parse_from([
            "qi-cli",
            "--url",
            "tcp://127.0.0.1:9559",
            "--json",
            "call",
            "Calculator.add",
            "-1",
            "[2]",
        ])
        .unwrap();
        assert!(cli.json);
        assert_eq!(cli.url.to_string(), "tcp://127.0.0.1:9559");
        assert!(matches!(
            cli.command,
            Command::Call { target, args } if target == "Calculator.add" && args == ["-1", "[2]"]
        ));

        // Global options may follow the subcommand.
        let cli =
            Cli::try_parse_from(["qi-cli", "info", "--qi-url", "tcp://[::1]:1", "-zd"]).unwrap();
        assert_eq!(cli.url.to_string(), "tcp://[::1]:1");
        assert!(matches!(
            cli.command,
            Command::Info { services, hidden: true, details: true, list: false } if services.is_empty()
        ));

        assert!(Cli::try_parse_from(["qi-cli", "--user", "nao", "info"]).is_err());
        assert!(Cli::try_parse_from(["qi-cli", "--url", "http://x", "info"]).is_err());
        assert!(Cli::try_parse_from(["qi-cli", "info", "--list", "--hidden"]).is_err());
        assert!(Cli::try_parse_from(["qi-cli", "set", "Calculator.value"]).is_err());
    }
}
