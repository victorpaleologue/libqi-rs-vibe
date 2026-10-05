//! A node connecting to a space and using its `Calculator` service.

#[path = "common/config.rs"]
mod config;

use anyhow::{Context, Result};
use clap::Parser;
use config::Args;
use futures::StreamExt;
use qi::ObjectExt;
use tracing::info;
use tracing_subscriber::fmt;

#[tokio::main(flavor = "current_thread")]
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
    if let Some(config::UserAndToken { user, token }) = &args.user_and_token {
        let mut credentials = qi::value::KeyDynValueMap::new();
        credentials.set("auth_user", user.clone());
        credentials.set("auth_token", token.clone());
        node = qi::node::init();
        let node = node
            .connect_to_space(args.address, Some(credentials))
            .start()
            .await
            .with_context(|| format!("Failed to connect to space at address {}", args.address))?;
        return run(node).await;
    }
    let node = node
        .connect_to_space(args.address, None)
        .start()
        .await
        .with_context(|| format!("Failed to connect to space at address {}", args.address))?;
    run(node).await
}

async fn run(node: qi::Node<qi::service_directory::Client>) -> Result<()> {
    // You can access remote services and call methods on them.
    info!("getting \"Calculator\" service");
    let calculator = node.service("Calculator").await?;
    let mut results = calculator.subscribe::<_, i32>("result").await?;
    let () = calculator.call("reset", 3).await?; // => 3
    let () = calculator.call("add", 9).await?; // => 12
    let () = calculator.call("mul", 4).await?; // => 48
    let () = calculator.call("add", 80).await?; // => 128
    let () = calculator.call("div", 2).await?; // => 64
    let result: i32 = calculator.call("ans", ()).await?;
    info!(%result, "calculation is done"); // result = 64
    for _ in 0..5 {
        if let Some(intermediate) = results.next().await {
            info!(intermediate, "result signal");
        }
    }
    let precision: i32 = calculator.property("precision").await?;
    info!(precision, "precision property");
    Ok(())
}
