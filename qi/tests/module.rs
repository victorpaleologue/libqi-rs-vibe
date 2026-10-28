//! Tests of modules written in Rust, loaded by `qi-launch` and by the host API of the crate: the
//! example module (`examples/rust-module`) is built, staged in an SDK prefix, and loaded in a
//! space hosted by the test.

mod module_common;

use futures::StreamExt;
use module_common::{
    build_example, host_space, stage_prefix, verbose_stderr, wait_for_no_service, wait_for_service,
    Process, MODULE,
};
use qi::{module::host::Module, AnyObject, ObjectExt};
use std::process::{Command, Stdio};

fn qi_launch(url: &str, prefix: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_qi-launch"));
    command
        .arg("--qi-url")
        .arg(url)
        .arg("--qi-sdk-prefix")
        .arg(prefix)
        .env_remove("QI_ADDITIONAL_SDK_PREFIXES");
    command
}

#[tokio::test(flavor = "multi_thread")]
async fn qi_launch_registers_the_objects_of_factories() {
    let library = build_example("plain", &[], &[]);
    let prefix = stage_prefix("objects", &library);
    let (host, url) = host_space().await;
    let _launcher = Process::spawn(qi_launch(&url, &prefix).args([
        "-n",
        "test",
        "-o",
        "rustexample.RustGreeter",
        "--object",
        "rustexample.OtherGreeter",
    ]));

    let greeter = wait_for_service(&host, "RustGreeter").await;
    let mut greetings = greeter.subscribe::<_, String>("greeted").await.unwrap();
    let greeting: String = greeter.call("greet", "Nao".to_owned()).await.unwrap();
    assert_eq!(greeting, "Hello, Nao, from Rust!");
    assert_eq!(
        greetings.next().await.as_deref(),
        Some("Hello, Nao, from Rust!")
    );
    assert_eq!(greeter.property::<_, i32>("greetings").await.unwrap(), 1);
    // The session of the module is connected to the space of the launcher.
    let services: Vec<String> = greeter.call("services", ()).await.unwrap();
    assert!(services.contains(&"RustGreeter".to_owned()), "{services:?}");
    assert!(
        services.contains(&"OtherGreeter".to_owned()),
        "{services:?}"
    );

    let other = wait_for_service(&host, "OtherGreeter").await;
    let version: String = other.call("version", ()).await.unwrap();
    assert_eq!(version, env!("CARGO_PKG_VERSION"));
}

#[tokio::test(flavor = "multi_thread")]
async fn qi_launch_calls_functions() {
    let library = build_example("plain", &[], &[]);
    let prefix = stage_prefix("functions", &library);
    let (_host, url) = host_space().await;
    let run = |args: &[&str]| {
        qi_launch(&url, &prefix)
            .args(args)
            .stdout(Stdio::null())
            .stderr(verbose_stderr())
            .status()
            .unwrap()
            .code()
    };

    assert_eq!(run(&["-n", "test", "-f", "rustexample.add(1, 2)"]), Some(0));
    assert_eq!(
        run(&[
            "-n",
            "test",
            "-f",
            r#"rustexample.hello("Pepper")"#,
            "-f",
            "rustexample.services()"
        ]),
        Some(0)
    );
    // A function that fails, a missing method, a missing module.
    assert_eq!(
        run(&["-n", "test", "-f", r#"rustexample.fail("boom")"#]),
        Some(1)
    );
    assert_eq!(run(&["-n", "test", "-f", "rustexample.nosuch()"]), Some(1));
    assert_eq!(run(&["-n", "test", "-f", "nosuch.function()"]), Some(1));
    // The mandatory name and the mandatory object or function.
    assert_eq!(run(&["-f", "rustexample.add(1, 2)"]), Some(1));
    assert_eq!(run(&["-n", "test"]), Some(1));
}

#[tokio::test(flavor = "multi_thread")]
async fn qi_launch_loads_libraries_like_naoqi_2_1() {
    let library = build_example("plain", &[], &[]);
    let prefix = stage_prefix("libraries", &library);
    let (host, url) = host_space().await;
    // Every factory of the library is registered, autoload or not.
    let _launcher = Process::spawn(
        qi_launch(&url, &prefix)
            .args(["-n", "test"])
            .arg(prefix.join("lib/librustexample.so")),
    );
    for service in ["RustGreeter", "RustDcm", "OtherGreeter"] {
        wait_for_service(&host, service).await;
    }
    let dcm = wait_for_service(&host, "RustDcm").await;
    // No DCM in this space: the plugin never ran.
    let cycles: i64 = dcm.call("cycles", ()).await.unwrap();
    assert_eq!(cycles, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn qi_launch_unregisters_services_when_stopped() {
    let library = build_example("plain", &[], &[]);
    let prefix = stage_prefix("stop", &library);
    let (host, url) = host_space().await;
    let launcher = Process::spawn(qi_launch(&url, &prefix).args([
        "-n",
        "test",
        "-o",
        "rustexample.RustGreeter",
    ]));
    wait_for_service(&host, "RustGreeter").await;
    drop(launcher);
    wait_for_no_service(&host, "RustGreeter").await;
}

/// The host API: what `qi-launch` does, in the test process.
#[tokio::test(flavor = "multi_thread")]
async fn host_loads_a_module() {
    let library = build_example("plain", &[], &[]);
    let prefix = stage_prefix("host", &library);
    let (host, url) = host_space().await;
    let finder = qi::module::host::Finder::new([prefix.clone()]);
    let info = finder.find(MODULE).unwrap();
    assert_eq!(info.path, prefix.join("lib/librustexample.so"));
    assert_eq!(info.kind, "cpp");
    assert!(finder.find("nosuch").is_err());
    assert!(finder.find("bad/name").is_err());

    let module = Module::load(&info.path, Some(&url)).await.unwrap();
    assert_eq!(module.name(), MODULE);
    assert_eq!(
        module.factories(),
        ["RustGreeter", "RustDcm", "OtherGreeter"]
    );
    let object = module.object();
    let sum: i32 = object.call("add", (2, 3)).await.unwrap();
    assert_eq!(sum, 5);
    assert!(object
        .call::<(), _, _>("fail", "boom".to_owned())
        .await
        .is_err());

    // A factory object, registered by the host in its space.
    let greeter: AnyObject = object.call("RustGreeter", ()).await.unwrap();
    host.register_service_object("HostedGreeter", greeter)
        .await
        .unwrap();
    let client = qi::node::init()
        .connect_to_space(url.parse().unwrap(), None)
        .start()
        .await
        .unwrap();
    let hosted = client.service("HostedGreeter").await.unwrap();
    let greeting: String = hosted.call("greet", "client".to_owned()).await.unwrap();
    assert_eq!(greeting, "Hello, client, from Rust!");
    // The module object exposes its documentation.
    let meta = object.meta();
    assert_eq!(meta.description, "An example module written in Rust.");
    let hello = meta.methods.values().find(|m| m.name == "hello").unwrap();
    assert_eq!(hello.description, "Returns a greeting.");
}
