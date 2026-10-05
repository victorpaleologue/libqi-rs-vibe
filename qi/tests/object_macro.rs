//! Tests of the `#[qi::object]` attribute macro: the derived meta object, the dispatch of
//! interactions to implementations, and typed clients over local and remote objects.

use futures::StreamExt;
use qi::{
    node,
    object::{ActionId, ActionNameOrId},
    value::{Dynamic, FromValue, IntoValue, Signature, Value},
    AnyObject, Error, ObjectExt, Property, Signal,
};
use std::{
    sync::atomic::{AtomicI32, Ordering},
    time::Duration,
};
use tokio::time::timeout;

const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, qi::Valuable)]
#[qi(value(crate = "qi::value"))]
struct Point {
    x: i32,
    y: i32,
}

/// A calculator.
#[qi::object(case = "camelCase")]
trait Calculator {
    /// Adds two numbers.
    async fn add(&self, a: i32, b: i32) -> qi::Result<i32>;
    async fn concat(&self, a: String, b: String) -> qi::Result<String>;
    async fn echo_point(&self, point: Point) -> qi::Result<Point>;
    async fn echo_dynamic(
        &self,
        value: Dynamic<Value<'static>>,
    ) -> qi::Result<Dynamic<Value<'static>>>;
    async fn reset(&self) -> qi::Result<()>;
    async fn fail(&self) -> qi::Result<()>;
    #[qi::method(name = "_secret")]
    async fn secret(&self) -> qi::Result<i32>;
    async fn make_counter(&self) -> qi::Result<AnyObject>;
    /// Fired with the result of each addition.
    #[qi::signal]
    fn added(&self) -> &Signal<i32>;
    #[qi::property]
    fn total(&self) -> &Property<i32>;
    #[qi::property(name = "Label")]
    fn label(&self) -> &Property<String>;
}

#[qi::object]
trait Counter {
    async fn increment(&self) -> qi::Result<i32>;
    #[qi::signal]
    fn changed(&self) -> &Signal<i32>;
}

struct Calc {
    added: Signal<i32>,
    total: Property<i32>,
    label: Property<String>,
    secret_calls: AtomicI32,
}

impl Calc {
    fn new() -> Self {
        Self {
            added: Signal::new(),
            total: Property::new(0),
            label: Property::new("initial".to_owned()),
            secret_calls: AtomicI32::new(0),
        }
    }
}

#[qi::async_trait]
impl Calculator for Calc {
    async fn add(&self, a: i32, b: i32) -> qi::Result<i32> {
        let sum = a + b;
        self.total.set(self.total.get().await? + sum).await?;
        self.added.emit(sum);
        Ok(sum)
    }

    async fn concat(&self, a: String, b: String) -> qi::Result<String> {
        Ok(a + &b)
    }

    async fn echo_point(&self, point: Point) -> qi::Result<Point> {
        Ok(point)
    }

    async fn echo_dynamic(
        &self,
        value: Dynamic<Value<'static>>,
    ) -> qi::Result<Dynamic<Value<'static>>> {
        Ok(value)
    }

    async fn reset(&self) -> qi::Result<()> {
        self.total.set(0).await
    }

    async fn fail(&self) -> qi::Result<()> {
        Err(Error::Other("expected failure".into()))
    }

    async fn secret(&self) -> qi::Result<i32> {
        Ok(self.secret_calls.fetch_add(1, Ordering::SeqCst) + 1)
    }

    async fn make_counter(&self) -> qi::Result<AnyObject> {
        Ok(CounterObject::new(Count::default()).into_any())
    }

    fn added(&self) -> &Signal<i32> {
        &self.added
    }

    fn total(&self) -> &Property<i32> {
        &self.total
    }

    fn label(&self) -> &Property<String> {
        &self.label
    }
}

#[derive(Default)]
struct Count {
    value: AtomicI32,
    changed: Signal<i32>,
}

#[qi::async_trait]
impl Counter for Count {
    async fn increment(&self) -> qi::Result<i32> {
        let value = self.value.fetch_add(1, Ordering::SeqCst) + 1;
        self.changed.emit(value);
        Ok(value)
    }

    fn changed(&self) -> &Signal<i32> {
        &self.changed
    }
}

fn signature_of(ty: &Signature) -> String {
    ty.to_string()
}

#[test]
fn meta_object_is_derived_from_the_trait() {
    let meta = CalculatorObject::<Calc>::meta_object();
    assert_eq!(meta.description, "A calculator.");

    // Members are identified from 100 in declaration order, names are converted to camel case
    // or overridden, hidden names keep their underscore.
    let names: Vec<(u32, &str)> = meta
        .methods
        .values()
        .map(|m| (m.uid.0, m.name.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            (100, "add"),
            (101, "concat"),
            (102, "echoPoint"),
            (103, "echoDynamic"),
            (104, "reset"),
            (105, "fail"),
            (106, "_secret"),
            (107, "makeCounter"),
        ]
    );
    let add = meta.method(&ActionNameOrId::from("add")).unwrap();
    assert_eq!(signature_of(&add.parameters_signature), "(ii)");
    assert_eq!(signature_of(&add.return_signature), "i");
    assert_eq!(add.description, "Adds two numbers.");
    assert_eq!(
        add.parameters
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    let echo_point = meta.method(&ActionNameOrId::from("echoPoint")).unwrap();
    assert_eq!(
        signature_of(&echo_point.parameters_signature),
        "((ii)<Point,x,y>)"
    );
    assert_eq!(
        signature_of(&echo_point.return_signature),
        "(ii)<Point,x,y>"
    );
    let echo_dynamic = meta.method(&ActionNameOrId::from("echoDynamic")).unwrap();
    assert_eq!(signature_of(&echo_dynamic.parameters_signature), "(m)");
    assert_eq!(signature_of(&echo_dynamic.return_signature), "m");
    let reset = meta.method(&ActionNameOrId::from("reset")).unwrap();
    assert_eq!(signature_of(&reset.parameters_signature), "()");
    assert_eq!(signature_of(&reset.return_signature), "v");
    let make_counter = meta.method(&ActionNameOrId::from("makeCounter")).unwrap();
    assert_eq!(signature_of(&make_counter.return_signature), "o");

    // Signals and properties; a property is also a signal of its changes.
    let added = meta.signal(&ActionNameOrId::from("added")).unwrap();
    assert_eq!(
        (added.uid, signature_of(&added.signature).as_str()),
        (ActionId(108), "(i)")
    );
    let total = meta.property(&ActionNameOrId::from("total")).unwrap();
    assert_eq!(
        (total.uid, signature_of(&total.signature).as_str()),
        (ActionId(109), "i")
    );
    let total_signal = meta.signal(&ActionNameOrId::from("total")).unwrap();
    assert_eq!(signature_of(&total_signal.signature), "(i)");
    let label = meta.property(&ActionNameOrId::from("Label")).unwrap();
    assert_eq!(
        (label.uid, signature_of(&label.signature).as_str()),
        (ActionId(110), "s")
    );
    assert!(meta.property(&ActionNameOrId::from("label")).is_none());

    // The client shares the same meta object.
    assert_eq!(CalculatorClient::meta_object(), meta);
}

#[tokio::test]
async fn local_objects_dispatch_interactions() {
    let object = CalculatorObject::new(Calc::new()).into_any();

    let sum: i32 = object.call("add", (1, 2)).await.unwrap();
    assert_eq!(sum, 3);
    // Static arguments for dynamic parameters are wrapped, and results of dynamic methods are
    // unwrapped and converted to the type the caller expects.
    let echoed: Dynamic<Value<'static>> = object.call("echoDynamic", 5i64).await.unwrap();
    assert_eq!(echoed, Dynamic(Value::Int64(5)));
    let echoed: i32 = object.call("echoDynamic", 5i64).await.unwrap();
    assert_eq!(echoed, 5);
    let point: Point = object
        .call("echoPoint", Point { x: 1, y: 2 })
        .await
        .unwrap();
    assert_eq!(point, Point { x: 1, y: 2 });
    let err = object.call::<(), _, _>("fail", ()).await.unwrap_err();
    assert!(err.to_string().contains("expected failure"), "{err}");
    let err = object.call::<(), _, _>("missing", ()).await.unwrap_err();
    assert!(matches!(err, Error::MethodNotFound(_)), "{err}");
    let err = object
        .call::<i32, _, _>("add", "not numbers")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("arguments"), "{err}");

    // Signals and properties through the untyped API.
    let mut added = object.subscribe::<_, i32>("added").await.unwrap();
    let mut totals = object.subscribe::<_, i32>("total").await.unwrap();
    let _: i32 = object.call("add", (10, 5)).await.unwrap();
    assert_eq!(timeout(TIMEOUT, added.next()).await.unwrap(), Some(15));
    assert_eq!(timeout(TIMEOUT, totals.next()).await.unwrap(), Some(18));
    let total: i32 = object.property("total").await.unwrap();
    assert_eq!(total, 18);
    object
        .set_property("Label", "changed".to_owned())
        .await
        .unwrap();
    let label: String = object.property("Label").await.unwrap();
    assert_eq!(label, "changed");
    let mut properties = object.properties();
    properties.sort();
    assert_eq!(properties, ["Label", "total"]);
    object.emit("added", 99).await.unwrap();
    assert_eq!(timeout(TIMEOUT, added.next()).await.unwrap(), Some(99));

    // A typed client works over a local object too.
    let client = CalculatorClient::new(object.clone()).unwrap();
    assert_eq!(client.concat("a".into(), "b".into()).await.unwrap(), "ab");
    assert_eq!(client.total().get().await.unwrap(), 18);
    client.total().set(1).await.unwrap();
    assert_eq!(client.total().get().await.unwrap(), 1);
    let mut changes = client.added().subscribe().await.unwrap();
    assert_eq!(client.add(2, 2).await.unwrap(), 4);
    assert_eq!(timeout(TIMEOUT, changes.next()).await.unwrap(), Some(4));
    client.added().emit(7);
    assert_eq!(timeout(TIMEOUT, changes.next()).await.unwrap(), Some(7));

    // Objects lacking members of the interface are rejected.
    let counter = CounterObject::new(Count::default()).into_any();
    let err = CalculatorClient::new(counter).unwrap_err();
    assert!(matches!(err, Error::MethodNotFound(_)), "{err}");
}

#[tokio::test]
async fn typed_clients_work_over_the_network() {
    let calc = Calc::new();
    let mut init = node::init();
    init.add_service("Calculator", CalculatorObject::new(calc));
    init.bind("tcp://127.0.0.1:0".parse().unwrap());
    let host = init.host_space().start().await.unwrap();
    let address = host
        .endpoints()
        .iter()
        .find_map(|target| target.to_string().parse::<qi::Address>().ok())
        .unwrap();
    let client_node = node::init()
        .connect_to_space(address, None)
        .start()
        .await
        .unwrap();

    let calculator =
        CalculatorClient::new(client_node.service("Calculator").await.unwrap()).unwrap();
    assert!(calculator.as_object().as_client().is_some());
    assert_eq!(calculator.add(40, 2).await.unwrap(), 42);
    assert_eq!(
        calculator.concat("foo".into(), "bar".into()).await.unwrap(),
        "foobar"
    );
    assert_eq!(
        calculator.echo_point(Point { x: 4, y: 2 }).await.unwrap(),
        Point { x: 4, y: 2 }
    );
    assert_eq!(
        calculator
            .echo_dynamic(Dynamic(Value::String("héllo".into())))
            .await
            .unwrap(),
        Dynamic(Value::String("héllo".into()))
    );
    assert_eq!(calculator.secret().await.unwrap(), 1);
    let err = calculator.fail().await.unwrap_err();
    assert!(err.to_string().contains("expected failure"), "{err}");

    // Signals and properties.
    let mut added = calculator.added().subscribe().await.unwrap();
    let mut totals = calculator.total().subscribe().await.unwrap();
    assert_eq!(calculator.add(1, 1).await.unwrap(), 2);
    assert_eq!(timeout(TIMEOUT, added.next()).await.unwrap(), Some(2));
    assert_eq!(timeout(TIMEOUT, totals.next()).await.unwrap(), Some(44));
    assert_eq!(calculator.total().get().await.unwrap(), 44);
    calculator.reset().await.unwrap();
    assert_eq!(timeout(TIMEOUT, totals.next()).await.unwrap(), Some(0));
    calculator.label().set("remote".to_owned()).await.unwrap();
    assert_eq!(calculator.label().get().await.unwrap(), "remote");
    // Emitting a remote signal bounces through the object.
    calculator.added().emit(5);
    assert_eq!(timeout(TIMEOUT, added.next()).await.unwrap(), Some(5));

    // Objects returned by methods are wrapped in typed clients.
    let counter = CounterClient::new(calculator.make_counter().await.unwrap()).unwrap();
    let mut changed = counter.changed().subscribe().await.unwrap();
    assert_eq!(counter.increment().await.unwrap(), 1);
    assert_eq!(counter.increment().await.unwrap(), 2);
    assert_eq!(timeout(TIMEOUT, changed.next()).await.unwrap(), Some(1));
    assert_eq!(timeout(TIMEOUT, changed.next()).await.unwrap(), Some(2));

    // Clients are values: they convert to and from objects.
    let as_value = counter.clone().into_value();
    let again = CounterClient::from_value(as_value).unwrap();
    assert_eq!(again.increment().await.unwrap(), 3);
    let any: AnyObject = again.into();
    let current: i32 = any.call("increment", ()).await.unwrap();
    assert_eq!(current, 4);
}
