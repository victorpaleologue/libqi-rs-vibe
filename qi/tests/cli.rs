//! End-to-end tests of the `qi-cli` binary against a space hosted in the test process.

use futures::StreamExt;
use qi::{
    dynamic::ObjectBuilder, node::Node, service_directory::LocalServiceDirectory, Address,
    AnyObject, Property, Signal,
};
use std::{
    io::{BufRead, BufReader},
    process::{Command, Output, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
use tokio::time::timeout;

const TIMEOUT: Duration = Duration::from_secs(30);

/// A space hosting a `Calculator` service.
struct Space {
    _host: Node<LocalServiceDirectory>,
    url: String,
    fired: Signal<i32>,
    value: Property<i32>,
}

async fn host_space() -> Space {
    let fired = Signal::<i32>::new();
    let value = Property::new(0i32);
    let text = Property::new("initial".to_owned());
    let mut builder = ObjectBuilder::new();
    builder.set_description("A test calculator");
    builder.add_method("add", |(a, b): (i32, i32)| async move { Ok(a + b) });
    builder.add_method("scale", |(x, factor): (f32, f64)| async move {
        Ok(f64::from(x) * factor)
    });
    builder.add_method("concat", |(a, b): (String, String)| async move {
        Ok(format!("{a}{b}"))
    });
    builder.add_method("echo_list", |list: Vec<i32>| async move { Ok(list) });
    builder.add_method("pair", |(): ()| async move { Ok((1i32, "one".to_owned())) });
    builder.add_method("raw_len", |data: qi::value::AsRaw<Vec<u8>>| async move {
        Ok(u32::try_from(data.0.len()).unwrap_or(u32::MAX))
    });
    builder.add_method("fail", |(): ()| async move {
        Err::<(), _>(qi::Error::Other("expected failure".into()))
    });
    builder.add_method("_hidden", |(): ()| async move { Ok(()) });
    let fired2 = fired.clone();
    builder.add_method("fire", move |n: i32| {
        let fired = fired2.clone();
        async move {
            fired.emit(n);
            Ok(())
        }
    });
    builder.add_signal("fired", fired.clone());
    builder.add_property("value", value.clone());
    builder.add_property("text", text);

    let mut init = qi::node::init();
    init.add_service_object("Calculator", AnyObject::new(builder.build()));
    init.bind("tcp://127.0.0.1:0".parse().expect("valid address"));
    let host = init.host_space().start().await.expect("host space");
    let address = host
        .endpoints()
        .iter()
        .find_map(|target| target.to_string().parse::<Address>().ok())
        .expect("host has a TCP endpoint");
    Space {
        _host: host,
        url: address.to_string(),
        fired,
        value,
    }
}

fn command(url: &str, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_qi-cli"));
    command
        .arg("--url")
        .arg(url)
        .args(args)
        .env("NO_COLOR", "1")
        .env_remove("RUST_LOG");
    command
}

/// Runs the tool to completion, off the asynchronous runtime that hosts the space.
async fn qi_cli(url: &str, args: &[&str]) -> Output {
    let url = url.to_owned();
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    tokio::task::spawn_blocking(move || {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        command(&url, &args).output().expect("run qi-cli")
    })
    .await
    .expect("qi-cli task")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn success(output: &Output) -> String {
    assert!(
        output.status.success(),
        "qi-cli failed: {}\n{}",
        stderr(output),
        stdout(output)
    );
    stdout(output)
}

fn failure(output: &Output) -> String {
    assert!(
        !output.status.success(),
        "qi-cli should have failed: {}",
        stdout(output)
    );
    assert_eq!(output.status.code(), Some(1), "{}", stderr(output));
    stderr(output)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn info_lists_services_and_describes_members() {
    let space = host_space().await;

    let listing = success(&qi_cli(&space.url, &["info", "--list"]).await);
    assert!(listing.contains("001 [ServiceDirectory]"), "{listing}");
    assert!(listing.contains("002 [Calculator]"), "{listing}");
    assert!(!listing.contains("Methods"), "{listing}");

    let report = success(&qi_cli(&space.url, &["info", "Calculator"]).await);
    assert!(
        report.starts_with("002 [Calculator]\n  * Info:\n"),
        "{report}"
    );
    assert!(report.contains("    machine   "), "{report}");
    assert!(report.contains("    process   "), "{report}");
    assert!(report.contains("    endpoints "), "{report}");
    assert!(
        report
            .lines()
            .any(|line| line.trim_start().starts_with("tcp://127.0.0.1:")),
        "{report}"
    );
    assert!(
        report.contains("  * Description: A test calculator\n"),
        "{report}"
    );
    assert!(report.contains(" add::(Int32,Int32)->Int32\n"), "{report}");
    assert!(
        report.contains(" scale::(Float,Double)->Double\n"),
        "{report}"
    );
    assert!(
        report.contains(" echo_list::(List<Int32>)->List<Int32>\n"),
        "{report}"
    );
    assert!(report.contains(" pair::()->(Int32,String)\n"), "{report}");
    assert!(report.contains("  * Signals:\n"), "{report}");
    assert!(report.contains(" fired::(Int32)\n"), "{report}");
    assert!(report.contains("  * Properties:\n"), "{report}");
    assert!(report.contains(" value::Int32\n"), "{report}");
    assert!(report.contains(" text::String\n"), "{report}");
    assert!(!report.contains("_hidden"), "{report}");
    assert!(!report.contains("metaObject"), "{report}");
    assert!(!report.contains("ServiceDirectory]"), "{report}");

    let report =
        success(&qi_cli(&space.url, &["info", "Calculator", "--hidden", "--details"]).await);
    assert!(report.contains(" _hidden::()->Void\n"), "{report}");
    assert!(
        report.contains(" metaObject::(UInt32)->MetaObject\n"),
        "{report}"
    );
    assert!(report.contains("signature   (ii)->i\n"), "{report}");

    // Every service, with members.
    let report = success(&qi_cli(&space.url, &["info"]).await);
    assert!(report.contains("001 [ServiceDirectory]"), "{report}");
    assert!(
        report.contains(" services::()->List<ServiceInfo>\n"),
        "{report}"
    );
    assert!(report.contains("002 [Calculator]"), "{report}");
    assert!(report.contains(" add::(Int32,Int32)->Int32\n"), "{report}");

    // JSON report.
    let report = success(&qi_cli(&space.url, &["--json", "info", "Calculator"]).await);
    let json: serde_json::Value = serde_json::from_str(&report).expect("JSON report");
    let service = &json[0];
    assert_eq!(service["serviceId"], 2);
    assert_eq!(service["name"], "Calculator");
    assert_eq!(service["description"], "A test calculator");
    assert!(service["endpoints"]
        .as_array()
        .is_some_and(|e| !e.is_empty()));
    let methods = service["methods"].as_array().expect("methods");
    let add = methods
        .iter()
        .find(|method| method["name"] == "add")
        .expect("add method");
    assert_eq!(add["signature"], "add::(Int32,Int32)->Int32");
    assert_eq!(add["parametersSignature"], "(ii)");
    assert_eq!(add["returnSignature"], "i");
    assert!(!methods.iter().any(|method| method["name"] == "_hidden"));
    assert_eq!(
        service["properties"].as_array().expect("properties").len(),
        2
    );

    // Unknown services are errors, but known ones are still described.
    let output = qi_cli(&space.url, &["info", "Nope", "Calculator"]).await;
    let error = failure(&output);
    assert!(error.contains("no service named \"Nope\""), "{error}");
    assert!(
        stdout(&output).contains("002 [Calculator]"),
        "{}",
        stdout(&output)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn call_converts_arguments_and_prints_results() {
    let space = host_space().await;

    let out = success(&qi_cli(&space.url, &["call", "Calculator.add", "40", "2"]).await);
    assert_eq!(out, "42\n");
    let out = success(&qi_cli(&space.url, &["call", "Calculator.add", "-40", "2"]).await);
    assert_eq!(out, "-38\n");
    // The JSON number 1 is a float32 for this method.
    let out = success(&qi_cli(&space.url, &["call", "Calculator.scale", "1", "2.5"]).await);
    assert_eq!(out, "2.5\n");
    // Strings need no quotes, but accept them.
    let out = success(&qi_cli(&space.url, &["call", "Calculator.concat", "foo", "\"bar\""]).await);
    assert_eq!(out, "foobar\n");
    let out = success(
        &qi_cli(
            &space.url,
            &["--json", "call", "Calculator.concat", "1", "2"],
        )
        .await,
    );
    assert_eq!(out, "\"12\"\n");
    let out = success(&qi_cli(&space.url, &["call", "Calculator.echo_list", "[1, 2, 3]"]).await);
    assert_eq!(out, "[\n  1,\n  2,\n  3\n]\n");
    let out = success(&qi_cli(&space.url, &["call", "Calculator.pair"]).await);
    assert_eq!(out, "[\n  1,\n  \"one\"\n]\n");
    let out = success(&qi_cli(&space.url, &["call", "Calculator.raw_len", "\"AAEC\""]).await);
    assert_eq!(out, "3\n");
    // Nothing is printed for void results, unless JSON is requested.
    let out = success(&qi_cli(&space.url, &["call", "Calculator.fire", "1"]).await);
    assert_eq!(out, "");
    let out = success(&qi_cli(&space.url, &["--json", "call", "Calculator.fire", "1"]).await);
    assert_eq!(out, "null\n");

    // Errors.
    let error = failure(&qi_cli(&space.url, &["call", "Calculator.fail"]).await);
    assert!(error.contains("expected failure"), "{error}");
    let error = failure(&qi_cli(&space.url, &["call", "Calculator.nope"]).await);
    assert!(error.contains("no method \"nope\""), "{error}");
    let error = failure(&qi_cli(&space.url, &["call", "Nope.add", "1", "2"]).await);
    assert!(error.contains("cannot reach service \"Nope\""), "{error}");
    let error = failure(&qi_cli(&space.url, &["call", "Calculator.add", "1", "x"]).await);
    assert!(error.contains("argument 2"), "{error}");
    assert!(error.contains("invalid JSON"), "{error}");
    let error = failure(&qi_cli(&space.url, &["call", "Calculator.add", "1", "2.5"]).await);
    assert!(error.contains("not representable as int32"), "{error}");
    let error = failure(&qi_cli(&space.url, &["call", "Calculator.add", "1"]).await);
    assert!(error.contains("1 argument(s) given"), "{error}");
    let error = failure(&qi_cli(&space.url, &["call", "Calculator"]).await);
    assert!(error.contains("SERVICE.MEMBER"), "{error}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn properties_are_read_and_written() {
    let space = host_space().await;

    let out = success(&qi_cli(&space.url, &["get", "Calculator.value"]).await);
    assert_eq!(out, "0\n");
    success(&qi_cli(&space.url, &["set", "Calculator.value", "7"]).await);
    assert_eq!(space.value.get().await.unwrap(), 7);
    let out = success(&qi_cli(&space.url, &["get", "Calculator.value"]).await);
    assert_eq!(out, "7\n");
    success(&qi_cli(&space.url, &["set", "Calculator.value", "-1"]).await);
    assert_eq!(space.value.get().await.unwrap(), -1);

    let out = success(&qi_cli(&space.url, &["get", "Calculator.text"]).await);
    assert_eq!(out, "initial\n");
    success(&qi_cli(&space.url, &["set", "Calculator.text", "hello world"]).await);
    let out = success(&qi_cli(&space.url, &["--json", "get", "Calculator.text"]).await);
    assert_eq!(out, "\"hello world\"\n");

    let error = failure(&qi_cli(&space.url, &["set", "Calculator.value", "x"]).await);
    assert!(
        error.contains("invalid value for the property Calculator.value"),
        "{error}"
    );
    let error = failure(&qi_cli(&space.url, &["get", "Calculator.nope"]).await);
    assert!(error.contains("no property \"nope\""), "{error}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn posts_reach_the_service() {
    let space = host_space().await;
    let mut fired = space.fired.subscribe().await.unwrap();

    let out = success(&qi_cli(&space.url, &["post", "Calculator.fire", "3"]).await);
    assert_eq!(out, "");
    assert_eq!(timeout(TIMEOUT, fired.next()).await.unwrap(), Some(3));

    // Posting to a signal emits it.
    success(&qi_cli(&space.url, &["post", "Calculator.fired", "4"]).await);
    assert_eq!(timeout(TIMEOUT, fired.next()).await.unwrap(), Some(4));

    let error = failure(&qi_cli(&space.url, &["post", "Calculator.nope"]).await);
    assert!(error.contains("no method or signal \"nope\""), "{error}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn watch_prints_the_values_of_signals() {
    let space = host_space().await;
    let mut child = command(&space.url, &["watch", "Calculator.fired", "--time"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn qi-cli");
    let child_stdout = child.stdout.take().expect("piped stdout");
    let (lines, received) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(child_stdout).lines().map_while(Result::ok) {
            if lines.send(line).is_err() {
                break;
            }
        }
    });

    // The watcher needs some time to connect and subscribe: emit until it reports a value.
    let deadline = Instant::now() + TIMEOUT;
    let mut emitted = 0;
    let first = loop {
        emitted += 1;
        space.fired.emit(emitted);
        match received.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => break line,
            Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
            Err(error) => {
                let _ = child.kill();
                let output = child.wait_with_output().expect("wait qi-cli");
                panic!("no value watched: {error}\n{}", stderr(&output));
            }
        }
    };
    let value_of = |line: &str| -> i32 {
        let (time, value) = line.rsplit_once(' ').expect("time and value");
        assert!(time.starts_with('[') && time.ends_with("Z]"), "{line}");
        value.parse().expect("integer value")
    };
    let first = value_of(&first);
    assert!(first >= 1 && first <= emitted, "{first}");

    // Later values are all received, in order.
    space.fired.emit(4242);
    space.fired.emit(4243);
    let mut seen = Vec::new();
    while seen.last() != Some(&4243) {
        let line = received.recv_timeout(TIMEOUT).expect("watched value");
        seen.push(value_of(&line));
    }
    let position = seen
        .iter()
        .position(|v| *v == 4242)
        .expect("4242 is watched");
    assert_eq!(seen[position..], [4242, 4243], "{seen:?}");

    let _ = child.kill();
    let _ = child.wait();

    // Properties are watched as signals of their changes.
    let error = failure(&qi_cli(&space.url, &["watch", "Calculator.nope"]).await);
    assert!(error.contains("no signal or property \"nope\""), "{error}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connection_failures_are_reported() {
    // Nothing listens on port 1.
    let output = qi_cli("tcp://127.0.0.1:1", &["--timeout", "5", "info"]).await;
    let error = failure(&output);
    assert!(
        error.contains("cannot connect to the space at tcp://127.0.0.1:1"),
        "{error}"
    );
    assert!(stdout(&output).is_empty());

    // Usage errors exit with the status 2.
    let output = qi_cli("tcp://127.0.0.1:1", &["nope"]).await;
    assert_eq!(output.status.code(), Some(2));
}
