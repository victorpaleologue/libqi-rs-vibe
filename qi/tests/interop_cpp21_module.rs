//! The example module, built with its libqi entry points against the libqi of NAOqi 2.1
//! (`interop/cpp21/build-module.sh`), loaded by the `qi-launch` of NAOqi 2.1 in a space of the
//! service directory of NAOqi 2.1 (`qi-master`): its factories are registered when the library is
//! loaded (`QI_REGISTER_OBJECT_FACTORY`), and `Session::loadService(library)` registers their
//! objects as services.
//!
//! Skipped when `interop/cpp21/build/modules/librustexample.so` or the install of libqi 2.1
//! (`QI_INTEROP_CPP21_INSTALL_DIR`, default `/home/user/aldebaran/install21`) is missing;
//! `QI_INTEROP_REQUIRE=1` fails instead.

mod module_common;

use module_common::{repository, require, Process};
use qi::{ObjectExt, Protocol};
use std::{path::PathBuf, process::Command, time::Duration};

#[tokio::test(flavor = "multi_thread")]
async fn cpp21_qi_launch_registers_the_factories_of_a_rust_module() {
    let Some(library) = require(
        repository().join("interop/cpp21/build/modules/librustexample.so"),
        "the example module built against libqi 2.1",
        "run interop/cpp21/build-module.sh",
    ) else {
        return;
    };
    let Some(install) = require(
        std::env::var_os("QI_INTEROP_CPP21_INSTALL_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/home/user/aldebaran/install21")),
        "libqi 2.1",
        "run interop/cpp21/libqi-2.1/build-libqi-2.1.sh",
    ) else {
        return;
    };
    let library_path = format!(
        "{}:{}",
        install.join("lib").display(),
        install.join("lib-deps").display()
    );
    let port = free_port();
    let url = format!("tcp://127.0.0.1:{port}");
    let _master = Process::spawn(
        Command::new(install.join("bin/qi-master"))
            .arg(&url)
            .env("LD_LIBRARY_PATH", &library_path),
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    let _launcher = Process::spawn(
        Command::new(install.join("bin/qi-launch"))
            .arg("--qi-url")
            .arg(&url)
            .arg(&library)
            .env("LD_LIBRARY_PATH", &library_path)
            // libqi 2.1 gives no session to factories: the session of the module is at QI_URL.
            .env("QI_URL", &url),
    );

    let client = qi::node::init()
        .connect_to_space(url.parse().unwrap(), None)
        .start()
        .await
        .unwrap();
    assert_eq!(
        client.service_directory().protocol(),
        Some(Protocol::Legacy)
    );
    let mut greeter = None;
    for _ in 0..200 {
        if let Ok(object) = client.service("RustGreeter").await {
            greeter = Some(object);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let greeter = greeter.expect("RustGreeter is registered");
    let greeting: String = greeter.call("greet", "NAOqi 2.1".to_owned()).await.unwrap();
    assert_eq!(greeting, "Hello, NAOqi 2.1, from Rust!");
    assert_eq!(greeter.property::<_, i32>("greetings").await.unwrap(), 1);
    let services: Vec<String> = greeter.call("services", ()).await.unwrap();
    for service in ["RustGreeter", "RustDcm", "OtherGreeter"] {
        assert!(services.contains(&service.to_owned()), "{services:?}");
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
