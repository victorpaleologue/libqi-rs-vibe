//! A typed object interface declared with `#[qi::object]`, served and used in one process.
//!
//! Run with `cargo run --example typed_calculator`.

use anyhow::Result;
use futures::StreamExt;
use qi::{Property, Signal};

/// A calculator with an accumulator.
#[qi::object(case = "camelCase")]
trait Calculator {
    /// Adds two numbers, accumulates the result and notifies it.
    async fn add(&self, a: i32, b: i32) -> qi::Result<i32>;
    /// Notified with each result.
    #[qi::signal]
    fn result(&self) -> &Signal<i32>;
    /// The sum of every result so far.
    #[qi::property]
    fn total(&self) -> &Property<i32>;
}

struct Accumulator {
    result: Signal<i32>,
    total: Property<i32>,
}

#[qi::async_trait]
impl Calculator for Accumulator {
    async fn add(&self, a: i32, b: i32) -> qi::Result<i32> {
        let sum = a + b;
        self.total.set(self.total.get().await? + sum).await?;
        self.result.emit(sum);
        Ok(sum)
    }

    fn result(&self) -> &Signal<i32> {
        &self.result
    }

    fn total(&self) -> &Property<i32> {
        &self.total
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // A node hosting a space with the service.
    let accumulator = Accumulator {
        result: Signal::new(),
        total: Property::new(0),
    };
    let mut host = qi::node::init();
    host.add_service("Calculator", CalculatorObject::new(accumulator));
    host.bind("tcp://127.0.0.1:0".parse()?);
    let host = host.host_space().start().await?;
    let address = host
        .endpoints()
        .iter()
        .find_map(|endpoint| endpoint.to_string().parse::<qi::Address>().ok())
        .expect("the host has a TCP endpoint");
    println!("space hosted at {address}");

    // A node connecting to the space, using the service through the typed client.
    let client = qi::node::init()
        .connect_to_space(address, None)
        .start()
        .await?;
    let calculator = CalculatorClient::new(client.service("Calculator").await?)?;
    let mut results = calculator.result().subscribe().await?;
    for (a, b) in [(1, 2), (3, 4)] {
        let sum = calculator.add(a, b).await?;
        println!("{a} + {b} = {sum}");
    }
    println!(
        "notified: {:?}, {:?}",
        results.next().await,
        results.next().await
    );
    println!("total: {}", calculator.total().get().await?);
    Ok(())
}
