//! The example module, built with its libqi entry points (feature `module-cpp`) against libqi
//! 4.0.5, loaded by the real `qilaunch` (built by `interop/qilaunch/build.sh`): `-o` registers the
//! objects of factories through `qi_module_init` and `Session::loadService`, `-f` calls functions
//! through `Session::callModule` with JSON arguments.
//!
//! Skipped when `interop/qilaunch/build/qilaunch` or the libqi install (`QI_INTEROP_LIBQI_PREFIX`,
//! default `/home/user/aldebaran/libqi/install`) is missing; `QI_INTEROP_REQUIRE=1` fails instead.

mod module_common;

use futures::StreamExt;
use module_common::{
    build_example, host_space, repository, require, stage_prefix, verbose_stderr, wait_for_service,
    Process,
};
use qi::ObjectExt;
use std::{
    path::PathBuf,
    process::{Command, Stdio},
};

struct Setup {
    qilaunch: PathBuf,
    prefix: PathBuf,
    libqi: PathBuf,
}

fn setup(name: &str) -> Option<Setup> {
    let qilaunch = require(
        repository().join("interop/qilaunch/build/qilaunch"),
        "qilaunch",
        "run interop/qilaunch/build.sh",
    )?;
    let libqi = require(
        std::env::var_os("QI_INTEROP_LIBQI_PREFIX")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/home/user/aldebaran/libqi/install")),
        "the libqi 4.0.5 install",
        "build libqi 4.0.5 (interop/cpp/README.md) or set QI_INTEROP_LIBQI_PREFIX",
    )?;
    let library = build_example(
        "cpp-libqi4",
        &["cpp"],
        &[("QI_MODULE_CPP_SDK", libqi.display().to_string())],
    );
    let prefix = stage_prefix(name, &library);
    Some(Setup {
        qilaunch,
        prefix,
        libqi,
    })
}

fn qilaunch(setup: &Setup, url: &str) -> Command {
    let mut command = Command::new(&setup.qilaunch);
    command
        .arg("--qi-url")
        .arg(url)
        .arg("--qi-sdk-prefix")
        .arg(&setup.prefix)
        .env("LD_LIBRARY_PATH", setup.libqi.join("lib"))
        .env_remove("QI_ADDITIONAL_SDK_PREFIXES");
    command
}

#[tokio::test(flavor = "multi_thread")]
async fn qilaunch_registers_the_objects_of_a_rust_module() {
    let Some(setup) = setup("qilaunch-objects") else {
        return;
    };
    let (host, url) = host_space().await;
    let _launcher = Process::spawn(qilaunch(&setup, &url).args([
        "-n",
        "test",
        "-o",
        "rustexample.RustGreeter",
    ]));

    let greeter = wait_for_service(&host, "RustGreeter").await;
    // The meta object of the service, merged by libqi with its own members, has every member.
    let meta = greeter.meta();
    for member in ["greet", "services", "version"] {
        assert!(
            meta.methods.values().any(|m| m.name == member),
            "{member} in {:?}",
            meta.methods.values().map(|m| &m.name).collect::<Vec<_>>()
        );
    }
    let mut greetings = greeter.subscribe::<_, String>("greeted").await.unwrap();
    let greeting: String = greeter.call("greet", "qilaunch".to_owned()).await.unwrap();
    assert_eq!(greeting, "Hello, qilaunch, from Rust!");
    assert_eq!(
        greetings.next().await.as_deref(),
        Some("Hello, qilaunch, from Rust!")
    );
    assert_eq!(greeter.property::<_, i32>("greetings").await.unwrap(), 1);
    // libqi gave its session to the factory: the session of the module is in this space.
    let services: Vec<String> = greeter.call("services", ()).await.unwrap();
    assert!(services.contains(&"RustGreeter".to_owned()), "{services:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn qilaunch_calls_the_functions_of_a_rust_module() {
    let Some(setup) = setup("qilaunch-functions") else {
        return;
    };
    let (host, url) = host_space().await;
    let run = |args: &[&str]| {
        qilaunch(&setup, &url)
            .args(args)
            .stdout(Stdio::null())
            .stderr(verbose_stderr())
            .status()
            .unwrap()
            .code()
    };
    // JSON arguments, decoded by libqi as dynamic values (int64...), converted by the module.
    assert_eq!(run(&["-n", "test", "-f", "rustexample.add(1, 2)"]), Some(0));
    assert_eq!(
        run(&["-n", "test", "-f", r#"rustexample.hello("qilaunch")"#]),
        Some(0)
    );
    assert_eq!(
        run(&["-n", "test", "-f", r#"rustexample.fail("boom")"#]),
        Some(1)
    );

    // A function taking the session: libqi passes its own, which is in this space.
    let tts = text_to_speech(&host).await;
    assert_eq!(
        run(&["-n", "test", "-f", r#"rustexample.say("from qilaunch")"#]),
        Some(0)
    );
    assert_eq!(tts.lock().unwrap().as_slice(), ["from qilaunch"]);
}

/// A stand-in for `ALTextToSpeech`, recording what it is asked to say.
async fn text_to_speech(
    host: &qi::Node<qi::service_directory::LocalServiceDirectory>,
) -> std::sync::Arc<std::sync::Mutex<Vec<String>>> {
    let said = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut builder = qi::dynamic::ObjectBuilder::new();
    let recorder = std::sync::Arc::clone(&said);
    builder.add_method("say", move |text: String| {
        let recorder = std::sync::Arc::clone(&recorder);
        async move {
            recorder.lock().unwrap().push(text);
            Ok(())
        }
    });
    host.register_service_object("ALTextToSpeech", qi::AnyObject::new(builder.build()))
        .await
        .unwrap();
    said
}
