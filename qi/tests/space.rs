//! End-to-end tests of nodes exchanging over TCP loopback: calls, signals, properties, object
//! passing in both directions, cancellation and authentication.

use futures::StreamExt;
use qi::{
    call,
    dynamic::ObjectBuilder,
    node::{self, Node},
    object::ActionNameOrId,
    service_directory::{LocalServiceDirectory, ServiceDirectory},
    Address, AnyObject, Error, ObjectExt, Property, Signal,
};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicI32, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::time::timeout;

const TIMEOUT: Duration = Duration::from_secs(10);

/// The test service: a calculator with a signal, properties and object-passing methods.
struct Calculator {
    fired: Signal<i32>,
    value: Property<i32>,
    text: Property<String>,
    sleeping: Arc<AtomicBool>,
    canceled: Arc<AtomicBool>,
}

impl Calculator {
    fn new() -> (Self, AnyObject) {
        let calc = Self {
            fired: Signal::new(),
            value: Property::new(0),
            text: Property::new("initial".to_owned()),
            sleeping: Arc::default(),
            canceled: Arc::default(),
        };
        let object = calc.object();
        (calc, object)
    }

    fn object(&self) -> AnyObject {
        let mut builder = ObjectBuilder::new();
        builder.set_description("A test calculator");
        builder.add_method("add", |(a, b): (i32, i32)| async move { Ok(a + b) });
        builder.add_method("concat", |(a, b): (String, String)| async move {
            Ok(format!("{a}{b}"))
        });
        builder.add_method("echo_list", |list: Vec<i32>| async move { Ok(list) });
        builder.add_method("fail", |(): ()| async move {
            Err::<(), _>(Error::Other("expected failure".into()))
        });
        let fired = self.fired.clone();
        builder.add_method("fire", move |n: i32| {
            let fired = fired.clone();
            async move {
                fired.emit(n);
                Ok(())
            }
        });
        builder.add_signal("fired", self.fired.clone());
        builder.add_property("value", self.value.clone());
        builder.add_property("text", self.text.clone());
        builder.add_method("make_counter", |(): ()| async move { Ok(counter_object()) });
        builder.add_method(
            "use_callback",
            |(callback, n): (AnyObject, i32)| async move {
                let result: i32 = callback.call("compute", n).await?;
                Ok(result + 1)
            },
        );
        let sleeping = self.sleeping.clone();
        let canceled = self.canceled.clone();
        builder.add_method("sleep_ms", move |ms: i32| {
            let sleeping = sleeping.clone();
            let canceled = canceled.clone();
            async move {
                sleeping.store(true, Ordering::SeqCst);
                let result = tokio::select! {
                    () = tokio::time::sleep(Duration::from_millis(ms as u64)) => Ok(ms),
                    () = call::cancelled() => {
                        canceled.store(true, Ordering::SeqCst);
                        Err(Error::CallCanceled)
                    }
                };
                sleeping.store(false, Ordering::SeqCst);
                result
            }
        });
        AnyObject::new(builder.build())
    }
}

/// A counter object returned by value by the calculator.
fn counter_object() -> AnyObject {
    let count = Arc::new(AtomicI32::new(0));
    let changed = Signal::<i32>::new();
    let mut builder = ObjectBuilder::new();
    let (count2, changed2) = (count.clone(), changed.clone());
    builder.add_method("increment", move |(): ()| {
        let count = count2.clone();
        let changed = changed2.clone();
        async move {
            let value = count.fetch_add(1, Ordering::SeqCst) + 1;
            changed.emit(value);
            Ok(value)
        }
    });
    builder.add_method("current", move |(): ()| {
        let count = count.clone();
        async move { Ok(count.load(Ordering::SeqCst)) }
    });
    builder.add_signal("changed", changed);
    AnyObject::new(builder.build())
}

struct Space {
    host: Node<LocalServiceDirectory>,
    address: Address,
}

async fn host_space(services: Vec<(&str, AnyObject)>) -> Space {
    let mut init = qi::node::init();
    for (name, object) in services {
        init.add_service_object(name, object);
    }
    init.bind("tcp://127.0.0.1:0".parse().unwrap());
    let host = init.host_space().start().await.expect("host space");
    let address = host_address(&host);
    Space { host, address }
}

fn host_address(host: &Node<LocalServiceDirectory>) -> Address {
    host.endpoints()
        .iter()
        .find_map(|target| target.to_string().parse::<Address>().ok())
        .expect("host has a TCP endpoint")
}

async fn connect(address: Address) -> Node<qi::service_directory::Client> {
    node::init()
        .connect_to_space(address, None)
        .start()
        .await
        .expect("connect to space")
}

#[tokio::test]
async fn call_methods_and_errors() {
    let (_calc, object) = Calculator::new();
    let space = host_space(vec![("Calculator", object)]).await;
    let client = connect(space.address).await;
    let calculator = client.service("Calculator").await.unwrap();

    let sum: i32 = calculator.call("add", (40, 2)).await.unwrap();
    assert_eq!(sum, 42);
    let text: String = calculator
        .call("concat", ("foo".to_owned(), "bar".to_owned()))
        .await
        .unwrap();
    assert_eq!(text, "foobar");
    let list: Vec<i32> = calculator.call("echo_list", vec![1, 2, 3]).await.unwrap();
    assert_eq!(list, [1, 2, 3]);

    let err = calculator.call::<(), _, _>("fail", ()).await.unwrap_err();
    assert!(err.to_string().contains("expected failure"), "{err}");

    let err = calculator.call::<(), _, _>("nope", ()).await.unwrap_err();
    assert!(matches!(err, Error::MethodNotFound(ActionNameOrId::Name(name)) if name == "nope"));

    assert_eq!(calculator.meta().description, "A test calculator");
    let _ = &space.host;
}

#[tokio::test]
async fn service_directory_lists_services() {
    let (_calc, object) = Calculator::new();
    let space = host_space(vec![("Calculator", object)]).await;
    let client = connect(space.address).await;

    let services = client.service_directory().services().await.unwrap();
    let names: Vec<_> = services.iter().map(|info| info.name().to_owned()).collect();
    assert!(names.contains(&"ServiceDirectory".to_owned()), "{names:?}");
    assert!(names.contains(&"Calculator".to_owned()), "{names:?}");

    let info = client
        .service_directory()
        .service("Calculator")
        .await
        .unwrap();
    assert_eq!(info.name(), "Calculator");
    assert!(info.object_uid().is_some());
    // Relative endpoints are computed for peers supporting them.
    assert!(
        info.endpoints()
            .iter()
            .any(|e| e.to_string() == "qi:ServiceDirectory"),
        "{:?}",
        info.endpoints()
    );

    let machine_id = client.service_directory().machine_id().await.unwrap();
    assert_eq!(machine_id, qi::value::os::MachineId::local());

    // Unknown services are errors.
    assert!(client.service("Unknown").await.is_err());
}

#[tokio::test]
async fn services_added_by_other_nodes_are_announced() {
    let space = host_space(vec![]).await;
    let watcher = connect(space.address).await;
    let mut added = watcher
        .service_directory()
        .service_added()
        .subscribe()
        .await
        .unwrap();

    let (_calc, object) = Calculator::new();
    let mut init = node::init();
    init.add_service_object("Calculator", object);
    init.bind("tcp://127.0.0.1:0".parse().unwrap());
    let provider = init
        .connect_to_space(space.address, None)
        .start()
        .await
        .unwrap();

    let (id, name) = timeout(TIMEOUT, added.next()).await.unwrap().unwrap();
    assert_eq!(name, "Calculator");
    assert!(id.0 >= 2);

    // The watcher can now reach the service hosted by the provider node.
    let calculator = watcher.service("Calculator").await.unwrap();
    let sum: i32 = calculator.call("add", (1, 2)).await.unwrap();
    assert_eq!(sum, 3);

    // Dropping the provider unregisters its services.
    let mut removed = watcher
        .service_directory()
        .service_removed()
        .subscribe()
        .await
        .unwrap();
    drop(calculator);
    drop(provider);
    let (_id, name) = timeout(TIMEOUT, removed.next()).await.unwrap().unwrap();
    assert_eq!(name, "Calculator");
}

#[tokio::test]
async fn signals_are_received_remotely() {
    let (_calc, object) = Calculator::new();
    let space = host_space(vec![("Calculator", object)]).await;
    let client = connect(space.address).await;
    let calculator = client.service("Calculator").await.unwrap();

    let mut fired = calculator.subscribe::<_, i32>("fired").await.unwrap();
    let () = calculator.call("fire", 1).await.unwrap();
    let () = calculator.call("fire", 2).await.unwrap();
    assert_eq!(timeout(TIMEOUT, fired.next()).await.unwrap(), Some(1));
    assert_eq!(timeout(TIMEOUT, fired.next()).await.unwrap(), Some(2));

    // Emitting a remote signal posts to the remote object, which bounces the event.
    let signal = calculator
        .as_client()
        .unwrap()
        .signal::<i32, _>("fired")
        .unwrap();
    signal.emit(3);
    assert_eq!(timeout(TIMEOUT, fired.next()).await.unwrap(), Some(3));
}

#[tokio::test]
async fn properties_are_accessed_remotely() {
    let (calc, object) = Calculator::new();
    let space = host_space(vec![("Calculator", object)]).await;
    let client = connect(space.address).await;
    let calculator = client.service("Calculator").await.unwrap();

    let value: i32 = calculator.property("value").await.unwrap();
    assert_eq!(value, 0);
    let mut changes = calculator.subscribe::<_, i32>("value").await.unwrap();
    calculator.set_property("value", 7).await.unwrap();
    assert_eq!(calc.value.get().await.unwrap(), 7);
    assert_eq!(timeout(TIMEOUT, changes.next()).await.unwrap(), Some(7));
    let value: i32 = calculator.property("value").await.unwrap();
    assert_eq!(value, 7);

    let text: String = calculator.property("text").await.unwrap();
    assert_eq!(text, "initial");
    let mut properties = calculator.properties();
    properties.sort();
    assert_eq!(properties, ["text", "value"]);

    // Through a property proxy.
    let proxy = calculator
        .as_client()
        .unwrap()
        .property_proxy::<i32, _>("value")
        .unwrap();
    proxy.set(9).await.unwrap();
    assert_eq!(proxy.get().await.unwrap(), 9);
    assert_eq!(calc.value.get().await.unwrap(), 9);
}

#[tokio::test]
async fn objects_are_passed_in_both_directions() {
    let (_calc, object) = Calculator::new();
    let space = host_space(vec![("Calculator", object)]).await;
    let client = connect(space.address).await;
    let calculator = client.service("Calculator").await.unwrap();

    // Object returned by the service.
    let counter: AnyObject = calculator.call("make_counter", ()).await.unwrap();
    assert!(counter.as_client().is_some());
    let mut changed = counter.subscribe::<_, i32>("changed").await.unwrap();
    let one: i32 = counter.call("increment", ()).await.unwrap();
    let two: i32 = counter.call("increment", ()).await.unwrap();
    assert_eq!((one, two), (1, 2));
    let current: i32 = counter.call("current", ()).await.unwrap();
    assert_eq!(current, 2);
    assert_eq!(timeout(TIMEOUT, changed.next()).await.unwrap(), Some(1));
    assert_eq!(timeout(TIMEOUT, changed.next()).await.unwrap(), Some(2));
    drop(changed);
    drop(counter);

    // Object passed by the client: the service calls back into the client.
    let mut builder = ObjectBuilder::new();
    let calls = Arc::new(AtomicI32::new(0));
    let calls2 = calls.clone();
    builder.add_method("compute", move |n: i32| {
        let calls = calls2.clone();
        async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(n * 2)
        }
    });
    let callback = AnyObject::new(builder.build());
    let result: i32 = calculator
        .call("use_callback", (callback.clone(), 21))
        .await
        .unwrap();
    assert_eq!(result, 43);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn calls_are_cancelable() {
    let (calc, object) = Calculator::new();
    let space = host_space(vec![("Calculator", object)]).await;
    let client = connect(space.address).await;
    let calculator = client.service("Calculator").await.unwrap();

    let call = tokio::spawn({
        let calculator = calculator.clone();
        async move { calculator.call::<i32, _, _>("sleep_ms", 10_000).await }
    });
    // Wait for the service to be sleeping, then cancel by dropping the call.
    timeout(TIMEOUT, async {
        while !calc.sleeping.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    call.abort();
    timeout(TIMEOUT, async {
        while !calc.canceled.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the service observed the cancellation");
    assert!(!calc.sleeping.load(Ordering::SeqCst));

    // A short sleep completes normally.
    let ms: i32 = calculator.call("sleep_ms", 10).await.unwrap();
    assert_eq!(ms, 10);
}

#[tokio::test]
async fn authentication_is_enforced() {
    let (_calc, object) = Calculator::new();
    let mut init = qi::node::init();
    init.add_service_object("Calculator", object);
    init.bind("tcp://127.0.0.1:0".parse().unwrap());
    init.with_authenticator(Arc::new(qi::auth::UserTokenAuthenticator::new(
        "nao".to_owned(),
        "secret".to_owned(),
    )));
    let host = init.host_space().start().await.unwrap();
    let address = host_address(&host);

    // Without credentials, the connection is refused.
    let result = node::init().connect_to_space(address, None).start().await;
    assert!(result.is_err());

    // With bad credentials, the connection is refused.
    let mut credentials = qi::value::KeyDynValueMap::new();
    credentials.set("auth_user", "nao");
    credentials.set("auth_token", "wrong");
    let result = node::init()
        .connect_to_space(address, Some(credentials))
        .start()
        .await;
    assert!(result.is_err());

    // With good credentials, the connection succeeds.
    let mut credentials = qi::value::KeyDynValueMap::new();
    credentials.set("auth_user", "nao");
    credentials.set("auth_token", "secret");
    let client = node::init()
        .connect_to_space(address, Some(credentials))
        .start()
        .await
        .unwrap();
    let calculator = client.service("Calculator").await.unwrap();
    let sum: i32 = calculator.call("add", (1, 1)).await.unwrap();
    assert_eq!(sum, 2);
}

#[tokio::test]
async fn services_are_registered_and_unregistered_after_start() {
    let space = host_space(vec![]).await;
    let client = connect(space.address).await;
    let (_calc, object) = Calculator::new();
    let mut init = node::init();
    init.bind("tcp://127.0.0.1:0".parse().unwrap());
    let provider = init
        .connect_to_space(space.address, None)
        .start()
        .await
        .unwrap();

    let id = provider
        .register_service_object("Calculator", object)
        .await
        .unwrap();
    assert!(id.0 >= 2);
    let calculator = client.service("Calculator").await.unwrap();
    let sum: i32 = calculator.call("add", (2, 3)).await.unwrap();
    assert_eq!(sum, 5);

    provider.unregister_service(id).await.unwrap();
    assert!(client
        .service_directory()
        .service("Calculator")
        .await
        .is_err());
    // The service object is not reachable through the provider anymore either.
    let (_calc2, object2) = Calculator::new();
    let new_id = provider
        .register_service_object("Calculator", object2)
        .await
        .unwrap();
    assert_ne!(new_id, id);
}

/// The pattern of the NAOqi APIs predating object passing: a client registers a service on its
/// own node, then asks a service of the space to call it back by name. The called service looks
/// the callback service up in the directory and calls it.
#[tokio::test]
async fn services_call_back_services_registered_by_clients() {
    let space = host_space(vec![]).await;
    let host = Arc::new(space.host);

    // The "audio device" of the host calls `processRemote` on the service whose name is given to
    // `subscribe`, like ALAudioDevice does.
    let mut builder = ObjectBuilder::new();
    let node = Arc::downgrade(&host);
    builder.add_method("subscribe", move |name: String| {
        let node = node.clone();
        async move {
            let node = node.upgrade().ok_or(Error::Other("node is gone".into()))?;
            let subscriber = node.service(&name).await?;
            let () = subscriber
                .call(
                    "processRemote",
                    (2, 3, 1234, qi::value::AsRaw(vec![0u8; 12])),
                )
                .await?;
            Ok(())
        }
    });
    host.register_service_object("AudioDevice", AnyObject::new(builder.build()))
        .await
        .unwrap();

    // The client registers its callback service on its own node, then subscribes.
    let received = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut builder = ObjectBuilder::new();
    let received2 = received.clone();
    builder.add_method(
        "processRemote",
        move |(channels, samples, timestamp, buffer): (
            i32,
            i32,
            qi::value::Dynamic<i32>,
            qi::value::Dynamic<qi::value::AsRaw<Vec<u8>>>,
        )| {
            let received = received2.clone();
            async move {
                received
                    .lock()
                    .unwrap()
                    .push((channels, samples, timestamp.0, buffer.0 .0.len()));
                Ok(())
            }
        },
    );
    let mut init = node::init();
    init.bind("tcp://127.0.0.1:0".parse().unwrap());
    let client = init
        .connect_to_space(space.address, None)
        .start()
        .await
        .unwrap();
    client
        .register_service_object("Audio-Subscriber", AnyObject::new(builder.build()))
        .await
        .unwrap();
    let audio = client.service("AudioDevice").await.unwrap();
    let () = audio
        .call("subscribe", "Audio-Subscriber".to_owned())
        .await
        .unwrap();
    assert_eq!(received.lock().unwrap().as_slice(), [(2, 3, 1234, 12)]);
}

#[tokio::test]
async fn nodes_communicate_over_tls() {
    let (_calc, object) = Calculator::new();
    let mut init = qi::node::init();
    init.add_service_object("Calculator", object);
    init.bind("tcps://127.0.0.1:0".parse().unwrap());
    let host = init.host_space().start().await.expect("host a TLS space");
    let address = host_address(&host);
    assert!(address.to_string().starts_with("tcps://"), "{address}");

    // The service directory advertises TLS endpoints, that the client node connects to.
    let client = connect(address).await;
    let calculator = client.service("Calculator").await.unwrap();
    let sum: i32 = calculator.call("add", (20, 22)).await.unwrap();
    assert_eq!(sum, 42);
    let mut fired = calculator.subscribe::<_, i32>("fired").await.unwrap();
    let () = calculator.call("fire", 7).await.unwrap();
    assert_eq!(timeout(TIMEOUT, fired.next()).await.unwrap(), Some(7));

    // A plain TCP client cannot talk to a TLS endpoint.
    let Address::Tcp { address, .. } = address;
    let plain = Address::Tcp { address, ssl: None };
    assert!(node::init()
        .connect_to_space(plain, None)
        .start()
        .await
        .is_err());
}
