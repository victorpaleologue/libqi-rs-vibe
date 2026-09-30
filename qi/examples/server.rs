//! A node hosting a space with a `Calculator` service.
//!
//! Run with `cargo run --example server -- --address tcp://127.0.0.1:9559`, then run the client
//! example against it (or any `libqi` client, such as `qicli`).

#[path = "common/config.rs"]
mod config;

use self::config::{Args, UserAndToken};
use anyhow::{Context, Result};
use clap::Parser;
use qi::{auth::UserTokenAuthenticator, dynamic::ObjectBuilder, AnyObject, Property, Signal};
use std::sync::{Arc, Mutex};
use tracing::info;
use tracing_subscriber::fmt;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Activate traces to the console.
    tracing_subscriber::fmt()
        .compact()
        .with_max_level(match args.verbose {
            0 => Some(tracing::Level::WARN),
            1 => Some(tracing::Level::INFO),
            2 => Some(tracing::Level::DEBUG),
            3.. => Some(tracing::Level::TRACE),
        })
        .with_target(false)
        .with_span_events(fmt::format::FmtSpan::NEW | fmt::format::FmtSpan::CLOSE)
        .with_thread_ids(true)
        .with_thread_names(true)
        .init();

    info!("creating node");
    let mut node = qi::node::init();
    // You can add services to the node and make them accessible to other nodes of joined spaces.
    node.add_service_object("Calculator", calculator());
    // Host the space on this node
    node.bind(args.address);
    let mut node = node.host_space();

    if let Some(UserAndToken { user, token }) = args.user_and_token {
        node.with_authenticator(Arc::new(UserTokenAuthenticator::new(user, token)));
    }

    let node = node
        .start()
        .await
        .with_context(|| format!("Failed to host space for node at address {}", args.address))?;
    info!(endpoints = ?node.endpoints(), "node is started");

    tokio::signal::ctrl_c()
        .await
        .context("Unable to listen for shutdown signal")?;
    Ok(())
}

/// A calculator with an accumulator, a signal notifying results and a property.
fn calculator() -> AnyObject {
    let accumulator = Arc::new(Mutex::new(0i32));
    let result = Signal::<i32>::new();
    let precision = Property::new(2i32);
    let mut builder = ObjectBuilder::new();
    builder.set_description("A calculator with an accumulator");
    let operation = |name: &str, f: fn(i32, i32) -> i32| {
        let accumulator = Arc::clone(&accumulator);
        let result = result.clone();
        let name = name.to_owned();
        move |value: i32| {
            let accumulator = Arc::clone(&accumulator);
            let result = result.clone();
            let name = name.clone();
            async move {
                let mut acc = accumulator.lock().unwrap();
                *acc = f(*acc, value);
                info!(operation = %name, value, result = *acc, "operation");
                result.emit(*acc);
                Ok(())
            }
        }
    };
    builder.add_method("reset", operation("reset", |_, v| v));
    builder.add_method("add", operation("add", |a, v| a + v));
    builder.add_method("sub", operation("sub", |a, v| a - v));
    builder.add_method("mul", operation("mul", |a, v| a * v));
    builder.add_method("div", operation("div", |a, v| a / v));
    let acc = Arc::clone(&accumulator);
    builder.add_method("ans", move |(): ()| {
        let acc = Arc::clone(&acc);
        async move { Ok(*acc.lock().unwrap()) }
    });
    builder.add_signal("result", result);
    builder.add_property("precision", precision);
    AnyObject::new(builder.build())
}
