//! Interoperability tests against the reference C++ implementation of the qi framework.
//!
//! The tests spawn the programs of the `interop/cpp` harness (see `interop/cpp/README.md`) and
//! exercise every combination of C++ and Rust service directories, services and clients over TCP
//! loopback:
//!
//! - a C++ client running its whole scenario list against a Rust node hosting the service
//!   directory and the test service,
//! - a Rust client running the same scenarios against a C++ service directory and service,
//! - a Rust service registered on a C++ service directory, used by a C++ client,
//! - a C++ service registered on a Rust service directory, used by a Rust client.
//!
//! They are skipped when the harness is not built. Set `QI_INTEROP_CPP_BUILD_DIR` to the directory
//! of its binaries (defaults to `interop/cpp/build` in the repository), `QI_INTEROP_REQUIRE=1` to
//! fail instead of skipping, and `QI_INTEROP_VERBOSE=1` to forward the logs of the C++ processes
//! to the standard error output.

use futures::StreamExt;
use qi::{
    call,
    dynamic::ObjectBuilder,
    node::{self, Node},
    service_directory::{LocalServiceDirectory, ServiceDirectory},
    value::{os::MachineId, AsRaw, Dynamic, Value},
    Address, AnyObject, Error, ObjectExt, Property, Signal,
};
use serde::Deserialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicI32, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader, Lines},
    process::{Child, ChildStdout, Command},
    time::timeout,
};

const TIMEOUT: Duration = Duration::from_secs(30);
const SERVICE_NAME: &str = "TestService";
const LOOPBACK_ANY_PORT: &str = "tcp://127.0.0.1:0";

/// The C++ harness: the directory of its binaries.
struct Harness {
    dir: PathBuf,
}

impl Harness {
    /// Locates the harness binaries, or returns `None` (after printing why) if they are not built.
    fn locate() -> Option<Self> {
        let dir = match std::env::var_os("QI_INTEROP_CPP_BUILD_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../interop/cpp/build"),
        };
        if dir.join("qi-cpp-client").is_file() {
            return Some(Self { dir });
        }
        assert!(
            std::env::var_os("QI_INTEROP_REQUIRE").is_none(),
            "the C++ interop harness is required but is not built in {}",
            dir.display()
        );
        eprintln!(
            "skipping: the C++ interop harness is not built in {} (see interop/cpp/README.md)",
            dir.display()
        );
        None
    }

    fn command(&self, program: &str) -> Command {
        let mut command = Command::new(self.dir.join(program));
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(if std::env::var_os("QI_INTEROP_VERBOSE").is_some() {
                Stdio::inherit()
            } else {
                Stdio::null()
            })
            .kill_on_drop(true);
        command
    }

    /// Starts a C++ standalone service directory, and returns its process and its address.
    async fn service_directory(&self) -> (Process, Address) {
        let mut process = Process::spawn(
            "qi-cpp-sd",
            self.command("qi-cpp-sd")
                .args(["--qi-listen-url", LOOPBACK_ANY_PORT]),
        );
        let url = process.expect_line("LISTENING ").await;
        let address = url.trim().parse().expect("service directory address");
        (process, address)
    }

    /// Starts a C++ test service registered on the service directory at the address.
    async fn service(&self, service_directory: &Address) -> Process {
        let mut process = Process::spawn(
            "qi-cpp-service",
            self.command("qi-cpp-service").args([
                "--qi-url",
                &service_directory.to_string(),
                "--qi-listen-url",
                LOOPBACK_ANY_PORT,
                "--name",
                SERVICE_NAME,
            ]),
        );
        process.expect_line("READY").await;
        process
    }

    /// Runs every scenario of the C++ client against the test service of the space whose service
    /// directory is at the address, and asserts that they all pass.
    async fn run_client_scenarios(&self, service_directory: &Address) {
        let output = timeout(
            TIMEOUT,
            self.command("qi-cpp-client")
                .args([
                    "--qi-url",
                    &service_directory.to_string(),
                    "--service",
                    SERVICE_NAME,
                    "--scenarios",
                    "all",
                ])
                .stderr(Stdio::piped())
                .output(),
        )
        .await
        .expect("the C++ client completes in time")
        .expect("the C++ client runs");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let steps: Vec<Step> = stdout
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line)
                    .unwrap_or_else(|err| panic!("invalid C++ client output {line:?}: {err}"))
            })
            .collect();
        let failed: Vec<String> = steps
            .iter()
            .filter(|step| !step.ok)
            .map(|step| {
                format!(
                    "  {}: {}",
                    step.step,
                    step.result
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default()
                )
            })
            .collect();
        assert!(
            failed.is_empty() && output.status.success(),
            "C++ client failures (exit status {:?}):\n{}\nstderr:\n{}",
            output.status,
            failed.join("\n"),
            String::from_utf8_lossy(&output.stderr)
        );
        let summary = steps
            .iter()
            .find(|step| step.step == "summary")
            .expect("the C++ client prints a summary");
        assert!(
            summary.passed.unwrap_or(0) >= 30,
            "unexpectedly few scenarios ran: {summary:?}"
        );
    }
}

/// A line of output of the C++ client.
#[derive(Debug, Deserialize)]
struct Step {
    step: String,
    ok: bool,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    passed: Option<u32>,
}

/// A C++ process of the harness, killed when dropped.
struct Process {
    name: &'static str,
    #[allow(dead_code)]
    child: Child,
    lines: Lines<BufReader<ChildStdout>>,
}

impl Process {
    fn spawn(name: &'static str, command: &mut Command) -> Self {
        let mut child = command
            .spawn()
            .unwrap_or_else(|err| panic!("cannot spawn {name}: {err}"));
        let stdout = child.stdout.take().expect("piped standard output");
        Self {
            name,
            child,
            lines: BufReader::new(stdout).lines(),
        }
    }

    /// Waits for the next line of the standard output of the process.
    async fn next_line(&mut self) -> String {
        timeout(TIMEOUT, self.lines.next_line())
            .await
            .unwrap_or_else(|_| panic!("{}: timed out waiting for output", self.name))
            .unwrap_or_else(|err| panic!("{}: cannot read output: {err}", self.name))
            .unwrap_or_else(|| panic!("{}: exited without the expected output", self.name))
    }

    /// Waits for a line starting with the prefix and returns the rest of the line.
    async fn expect_line(&mut self, prefix: &str) -> String {
        loop {
            let line = self.next_line().await;
            if let Some(rest) = line.strip_prefix(prefix) {
                return rest.to_owned();
            }
        }
    }
}

macro_rules! harness_or_skip {
    () => {
        match Harness::locate() {
            Some(harness) => harness,
            None => return,
        }
    };
}

/// A structure exchanged with the C++ side, registered there as `(ii)<Point2D,x,y>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, qi::Valuable)]
#[qi(value(crate = "qi::value"))]
struct Point2D {
    x: i32,
    y: i32,
}

/// The Rust implementation of the interop test service, mirroring `interop/cpp/service.cpp`.
struct TestService {
    fired: Signal<i32>,
    void_signal: Signal<()>,
    value: Property<i32>,
    text: Property<String>,
    /// The last callback object received by `useCallback`, kept alive so that the subscription to
    /// its `tick` signal stays registered.
    callback: Arc<Mutex<Option<AnyObject>>>,
    /// The values received from the `tick` signals of callback objects.
    ticks: Arc<Mutex<Vec<i32>>>,
}

impl TestService {
    fn new() -> Self {
        Self {
            fired: Signal::new(),
            void_signal: Signal::new(),
            value: Property::new(0),
            text: Property::new(String::new()),
            callback: Arc::default(),
            ticks: Arc::default(),
        }
    }

    fn object(&self) -> AnyObject {
        let mut builder = ObjectBuilder::new();
        builder.set_description("Interop test service");
        builder.add_method("add", |(a, b): (i32, i32)| async move { Ok(a + b) });
        builder.add_method(
            "concat",
            |(a, b): (String, String)| async move { Ok(a + &b) },
        );
        builder.add_method("echoDynamic", |value: Dynamic<Value<'static>>| async move {
            Ok(value)
        });
        builder.add_method("echoList", |list: Vec<i32>| async move { Ok(list) });
        builder.add_method(
            "echoMap",
            |map: HashMap<String, i32>| async move { Ok(map) },
        );
        builder.add_method("echoStruct", |point: Point2D| async move { Ok(point) });
        builder.add_method(
            "echoOptional",
            |value: Option<i32>| async move { Ok(value) },
        );
        builder.add_method(
            "echoBuffer",
            |buffer: AsRaw<Vec<u8>>| async move { Ok(buffer) },
        );
        builder.add_method("fail", |(): ()| async move {
            Err::<(), _>(Error::Other("expected failure".into()))
        });
        builder.add_method("sleepMs", |ms: i32| async move {
            let duration = Duration::from_millis(u64::try_from(ms).unwrap_or(0));
            tokio::select! {
                () = tokio::time::sleep(duration) => Ok(ms),
                () = call::cancelled() => Err(Error::CallCanceled),
            }
        });
        let fired = self.fired.clone();
        builder.add_method("fire", move |n: i32| {
            let fired = fired.clone();
            async move {
                fired.emit(n);
                Ok(())
            }
        });
        builder.add_method("makeCounter", |(): ()| async move { Ok(counter_object()) });
        let callback = self.callback.clone();
        let ticks = self.ticks.clone();
        builder.add_method("useCallback", move |(cb, n): (AnyObject, i32)| {
            let callback = callback.clone();
            let ticks = ticks.clone();
            async move {
                let result: i32 = cb.call("compute", n).await?;
                if cb
                    .meta()
                    .signals
                    .values()
                    .any(|signal| signal.name == "tick")
                {
                    let mut tick = cb.subscribe::<_, i32>("tick").await?;
                    tokio::spawn(async move {
                        while let Some(n) = tick.next().await {
                            ticks.lock().unwrap().push(n);
                        }
                    });
                }
                *callback.lock().unwrap() = Some(cb);
                Ok(result)
            }
        });
        let void_signal = self.void_signal.clone();
        builder.add_method("emitVoid", move |(): ()| {
            let void_signal = void_signal.clone();
            async move {
                void_signal.emit(());
                Ok(())
            }
        });
        builder.add_method("bigString", |size: i32| async move {
            Ok("x".repeat(usize::try_from(size).unwrap_or(0)))
        });
        builder.add_signal("fired", self.fired.clone());
        builder.add_signal("voidSignal", self.void_signal.clone());
        builder.add_property("value", self.value.clone());
        builder.add_property("text", self.text.clone());
        AnyObject::new(builder.build())
    }

    /// Waits until a `tick` value has been received from a callback object, and returns all the
    /// received values.
    async fn wait_ticks(&self) -> Vec<i32> {
        timeout(TIMEOUT, async {
            loop {
                let ticks = self.ticks.lock().unwrap().clone();
                if !ticks.is_empty() {
                    return ticks;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("a tick is received from the callback object")
    }
}

/// A counter object returned by `makeCounter`.
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

/// A callback object passed to `useCallback`: `compute(n)` returns `2 * n` and counts its calls,
/// and `tick` is a signal the service is expected to subscribe to.
struct Callback {
    object: AnyObject,
    tick: Signal<i32>,
    compute_calls: Arc<AtomicI32>,
}

impl Callback {
    fn new() -> Self {
        let tick = Signal::<i32>::new();
        let compute_calls = Arc::new(AtomicI32::new(0));
        let mut builder = ObjectBuilder::new();
        let calls = compute_calls.clone();
        builder.add_method("compute", move |n: i32| {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(n * 2)
            }
        });
        builder.add_signal("tick", tick.clone());
        Self {
            object: AnyObject::new(builder.build()),
            tick,
            compute_calls,
        }
    }
}

fn host_address(host: &Node<LocalServiceDirectory>) -> Address {
    host.endpoints()
        .iter()
        .find_map(|target| target.to_string().parse::<Address>().ok())
        .expect("the host has a TCP endpoint")
}

async fn connect_node(address: Address) -> Node<qi::service_directory::Client> {
    node::init()
        .connect_to_space(address, None)
        .start()
        .await
        .expect("connect to the space")
}

/// Runs the scenarios of the C++ client (`interop/cpp/client.cpp`) from a Rust node against the
/// C++ test service, whose process is given to observe its output.
async fn run_rust_client_scenarios<SD>(client: &Node<SD>, cpp_service: &mut Process)
where
    SD: ServiceDirectory,
{
    // Service directory.
    let services = client.service_directory().services().await.unwrap();
    let names: Vec<&str> = services.iter().map(|info| info.name()).collect();
    assert!(names.contains(&SERVICE_NAME), "{names:?}");
    assert!(names.contains(&"ServiceDirectory"), "{names:?}");
    let info = services
        .iter()
        .find(|info| info.name() == SERVICE_NAME)
        .unwrap();
    assert!(info.object_uid().is_some(), "{info:?}");
    assert!(info.id().0 >= 2, "{info:?}");
    let machine_id = client.service_directory().machine_id().await.unwrap();
    assert_eq!(machine_id, MachineId::local());

    // The service and its meta object.
    let service = client.service(SERVICE_NAME).await.unwrap();
    for name in [
        "add",
        "concat",
        "echoDynamic",
        "echoList",
        "echoMap",
        "echoStruct",
        "echoOptional",
        "echoBuffer",
        "fail",
        "sleepMs",
        "fire",
        "makeCounter",
        "useCallback",
        "emitVoid",
        "bigString",
    ] {
        assert!(
            service.meta().methods.values().any(|m| m.name == name),
            "missing method {name}"
        );
    }
    let mut properties = service.properties();
    properties.sort();
    assert_eq!(properties, ["text", "value"]);

    // Calls with static types.
    let sum: i32 = service.call("add", (1, 2)).await.unwrap();
    assert_eq!(sum, 3);
    let text: String = service
        .call("concat", ("foo".to_owned(), "bar".to_owned()))
        .await
        .unwrap();
    assert_eq!(text, "foobar");
    let list: Vec<i32> = service
        .call("echoList", vec![1, -2, 300_000])
        .await
        .unwrap();
    assert_eq!(list, [1, -2, 300_000]);
    let list: Vec<i32> = service.call("echoList", Vec::<i32>::new()).await.unwrap();
    assert!(list.is_empty());
    let map: HashMap<String, i32> = [("a", 1), ("b", 2), ("héllo", -3)]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
    let echoed: HashMap<String, i32> = service.call("echoMap", map.clone()).await.unwrap();
    assert_eq!(echoed, map);
    let point: Point2D = service
        .call("echoStruct", Point2D { x: 4, y: 2 })
        .await
        .unwrap();
    assert_eq!(point, Point2D { x: 4, y: 2 });
    let none: Option<i32> = service.call("echoOptional", None::<i32>).await.unwrap();
    assert_eq!(none, None);
    let some: Option<i32> = service.call("echoOptional", Some(7)).await.unwrap();
    assert_eq!(some, Some(7));
    let AsRaw(buffer): AsRaw<Vec<u8>> = service
        .call("echoBuffer", AsRaw(vec![1u8, 2, 3, 4, 5]))
        .await
        .unwrap();
    assert_eq!(buffer, [1, 2, 3, 4, 5]);
    let big: String = service.call("bigString", 200_000).await.unwrap();
    assert_eq!(big.len(), 200_000);
    assert!(big.bytes().all(|b| b == b'x'));

    // Dynamic values keep their content through the C++ side.
    for value in [
        Value::Int32(42),
        Value::String("héllo".into()),
        Value::List(vec![Value::Int32(1), Value::Int32(2), Value::Int32(3)]),
        Value::Tuple(vec![Value::Int32(4), Value::Int32(2)]),
    ] {
        let Dynamic(echoed): Dynamic<Value<'static>> = service
            .call("echoDynamic", Dynamic(value.clone()))
            .await
            .unwrap();
        assert_eq!(echoed, value);
    }
    let Dynamic(point): Dynamic<Point2D> = service
        .call("echoDynamic", Dynamic(Point2D { x: 4, y: 2 }))
        .await
        .unwrap();
    assert_eq!(point, Point2D { x: 4, y: 2 });

    // Errors.
    let err = service.call::<(), _, _>("fail", ()).await.unwrap_err();
    assert!(err.to_string().contains("expected failure"), "{err}");

    // Cancellation: a short call completes, a long one is canceled by dropping it.
    let ms: i32 = service.call("sleepMs", 50).await.unwrap();
    assert_eq!(ms, 50);
    let long_call = tokio::spawn({
        let service = service.clone();
        async move { service.call::<i32, _, _>("sleepMs", 5000).await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    long_call.abort();
    assert!(long_call.await.unwrap_err().is_cancelled());
    // The service is still responsive.
    let ms: i32 = service.call("sleepMs", 1).await.unwrap();
    assert_eq!(ms, 1);

    // Signals.
    let mut fired = service.subscribe::<_, i32>("fired").await.unwrap();
    let () = service.call("fire", 1).await.unwrap();
    let () = service.call("fire", 2).await.unwrap();
    assert_eq!(timeout(TIMEOUT, fired.next()).await.unwrap(), Some(1));
    assert_eq!(timeout(TIMEOUT, fired.next()).await.unwrap(), Some(2));
    drop(fired);
    let mut void_signal = service.subscribe::<_, ()>("voidSignal").await.unwrap();
    let () = service.call("emitVoid", ()).await.unwrap();
    assert_eq!(
        timeout(TIMEOUT, void_signal.next()).await.unwrap(),
        Some(())
    );
    drop(void_signal);

    // Properties.
    let mut changes = service.subscribe::<_, i32>("value").await.unwrap();
    service.set_property("value", 42).await.unwrap();
    let value: i32 = service.property("value").await.unwrap();
    assert_eq!(value, 42);
    assert_eq!(timeout(TIMEOUT, changes.next()).await.unwrap(), Some(42));
    drop(changes);
    service
        .set_property("text", "héllo wörld".to_owned())
        .await
        .unwrap();
    let text: String = service.property("text").await.unwrap();
    assert_eq!(text, "héllo wörld");

    // Object returned by the C++ service.
    let counter: AnyObject = service.call("makeCounter", ()).await.unwrap();
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

    // Object passed to the C++ service, that calls it back and subscribes to its signal.
    let callback = Callback::new();
    let result: i32 = service
        .call("useCallback", (callback.object.clone(), 21))
        .await
        .unwrap();
    assert_eq!(result, 42);
    assert_eq!(callback.compute_calls.load(Ordering::SeqCst), 1);
    callback.tick.emit(7);
    let tick = cpp_service.expect_line("TICK ").await;
    assert_eq!(tick.trim(), "7");
}

#[tokio::test]
async fn cpp_client_against_rust_node() {
    let harness = harness_or_skip!();
    let service = TestService::new();
    let mut init = node::init();
    init.add_service_object(SERVICE_NAME, service.object());
    init.bind(LOOPBACK_ANY_PORT.parse().unwrap());
    let host = init.host_space().start().await.unwrap();

    harness.run_client_scenarios(&host_address(&host)).await;
    // The `useCallback` scenario of the C++ client emits `tick(7)` once after the call.
    assert_eq!(service.wait_ticks().await, [7]);
}

#[tokio::test]
async fn rust_client_against_cpp_node() {
    let harness = harness_or_skip!();
    let (_sd, sd_address) = harness.service_directory().await;
    let mut cpp_service = harness.service(&sd_address).await;
    let client = connect_node(sd_address).await;
    run_rust_client_scenarios(&client, &mut cpp_service).await;
}

#[tokio::test]
async fn rust_service_on_cpp_service_directory_serves_cpp_client() {
    let harness = harness_or_skip!();
    let (_sd, sd_address) = harness.service_directory().await;
    let service = TestService::new();
    let mut init = node::init();
    init.add_service_object(SERVICE_NAME, service.object());
    init.bind(LOOPBACK_ANY_PORT.parse().unwrap());
    let _provider = init
        .connect_to_space(sd_address, None)
        .start()
        .await
        .unwrap();

    harness.run_client_scenarios(&sd_address).await;
    assert_eq!(service.wait_ticks().await, [7]);
}

#[tokio::test]
async fn cpp_service_on_rust_service_directory_serves_rust_client() {
    let harness = harness_or_skip!();
    let mut init = node::init();
    init.bind(LOOPBACK_ANY_PORT.parse().unwrap());
    let host = init.host_space().start().await.unwrap();
    let sd_address = host_address(&host);
    let mut cpp_service = harness.service(&sd_address).await;
    let client = connect_node(sd_address).await;
    run_rust_client_scenarios(&client, &mut cpp_service).await;
}

#[tokio::test]
async fn cpp_client_over_tls_against_rust_node() {
    let harness = harness_or_skip!();
    let service = TestService::new();
    let mut init = node::init();
    init.add_service_object(SERVICE_NAME, service.object());
    init.bind("tcps://127.0.0.1:0".parse().unwrap());
    let host = init.host_space().start().await.unwrap();
    let address = host_address(&host);
    assert!(address.to_string().starts_with("tcps://"), "{address}");

    harness.run_client_scenarios(&address).await;
    assert_eq!(service.wait_ticks().await, [7]);
}
