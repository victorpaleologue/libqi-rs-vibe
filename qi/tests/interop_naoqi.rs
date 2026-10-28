//! The example module loaded the way NAOqi loads the libraries of its `autoload.ini`, by the
//! programs of `interop/naoqi-autoload` (built by its `build.sh`):
//!
//! - `fake-naoqi` calls `_createModule` with a `boost::shared_ptr` broker, as libalcommon declares
//!   it, checks that the broker is untouched and destroyed by its owner, then `_closeModule`;
//! - `fake-dcm` builds a broker of the stub of the NAOqi C++ SDK (`interop/naoqi-sdk-stub`) with a
//!   `DCM` module and ALMemory, and runs the loop of the DCM: the example module is built with its
//!   DCM bridge (feature `module-naoqi`) against that stub, and its plugin `RustEyes` reads a
//!   sensor and commands the LEDs of the eyes through an alias.
//!
//! The stub mimics the API the DCM plugins written in C++ use; it does not prove compatibility
//! with the SDK of a given NAOqi version. Skipped when the programs are not built;
//! `QI_INTEROP_REQUIRE=1` fails instead.

mod module_common;

use module_common::{
    build_example, host_space, repository, require, wait_for_no_service, wait_for_service,
};
use qi::ObjectExt;
use std::process::Stdio;

const BUILD: &str = "interop/naoqi-autoload/build";

#[tokio::test(flavor = "multi_thread")]
async fn naoqi_creates_and_closes_a_rust_module() {
    let Some(fake_naoqi) = require(
        repository().join(BUILD).join("fake-naoqi"),
        "fake-naoqi",
        "run interop/naoqi-autoload/build.sh",
    ) else {
        return;
    };
    let library = build_example("plain", &[], &[]);
    let (host, url) = host_space().await;
    let mut naoqi = tokio::process::Command::new(fake_naoqi)
        .arg(&library)
        .arg("3")
        .env("QI_URL", &url)
        .stdout(Stdio::piped())
        .stderr(module_common::verbose_stderr())
        .spawn()
        .unwrap();
    let greeter = wait_for_service(&host, "RustGreeter").await;
    let greeting: String = greeter.call("greet", "NAOqi".to_owned()).await.unwrap();
    assert_eq!(greeting, "Hello, NAOqi, from Rust!");
    wait_for_service(&host, "RustDcm").await;
    let status = naoqi.wait().await.unwrap();
    assert!(status.success(), "fake-naoqi: {status}");
    wait_for_no_service(&host, "RustGreeter").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dcm_runs_a_rust_plugin() {
    let Some(fake_dcm) = require(
        repository().join(BUILD).join("fake-dcm"),
        "fake-dcm",
        "run interop/naoqi-autoload/build.sh",
    ) else {
        return;
    };
    let sdk = repository().join(BUILD).join("sdk");
    let library = build_example(
        "naoqi-stub",
        &["naoqi"],
        &[
            ("QI_MODULE_NAOQI_SDK", sdk.display().to_string()),
            ("QI_MODULE_NAOQI_LIBS", "naoqistub".to_owned()),
        ],
    );
    let (host, url) = host_space().await;
    let naoqi = tokio::process::Command::new(&fake_dcm)
        .args([
            library.to_str().unwrap(),
            "150",
            "Device/SubDeviceList/HeadYaw/Position/Sensor/Value",
            "RustEyes",
        ])
        .env("QI_URL", &url)
        .stdout(Stdio::piped())
        .stderr(module_common::verbose_stderr())
        .spawn()
        .unwrap();
    // The service reports what the plugin sees, while the loop runs.
    let dcm = wait_for_service(&host, "RustDcm").await;
    let mut cycles = 0;
    for _ in 0..100 {
        cycles = dcm.call::<i64, _, _>("cycles", ()).await.unwrap();
        if cycles > 10 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(cycles > 10, "{cycles} cycles");
    let period: i32 = dcm.call("period", ()).await.unwrap();
    assert_eq!(period, 10);
    let yaw: f32 = dcm.call("headYaw", ()).await.unwrap();
    assert!(yaw > 0.0, "{yaw}");

    let output = naoqi.wait_with_output().await.unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "fake-dcm: {}\n{stdout}",
        output.status
    );
    assert!(stdout.contains("modules: DCM RustEyes"), "{stdout}");
    assert!(
        stdout.contains(
            r#"alias ["RustEyes", ["Device/SubDeviceList/Face/Led/Blue/Left/0Deg/Actuator/Value""#
        ),
        "{stdout}"
    );
    // One command every 10 cycles, with one value per LED, applied at the time of the DCM.
    assert!(stdout.contains("commands 15"), "{stdout}");
    assert!(stdout.contains(r#"last ["RustEyes", "ClearAll", "time-separate", 0, [2490], [[0], [0], [0], [0], [0], [0], [0], [1], "#), "{stdout}");
}
