//! Interoperability tests against the `libqi` of NAOqi 2.1 (2014), which speaks the legacy
//! protocol: no authentication, capabilities advertised with a message on connection, no
//! cancellation, no object UIDs, six-field service infos.
//!
//! The tests spawn the programs of the `interop/cpp21` harness (see `interop/cpp21/README.md`),
//! built against that `libqi` in an Ubuntu 14.04 container, and exercise every combination of
//! C++ and Rust service directories, services and clients over TCP loopback, like
//! `interop_cpp.rs` does with `libqi` 4.0.5. The Rust side detects the protocol at connection.
//!
//! They are skipped when the harness is not built. Set `QI_INTEROP_CPP21_BUILD_DIR` to the
//! directory of its binaries (defaults to `interop/cpp21/build` in the repository),
//! `QI_INTEROP_REQUIRE=1` to fail instead of skipping, and `QI_INTEROP_VERBOSE=1` to forward the
//! logs of the C++ processes to the standard error output.

mod interop_common;

use interop_common::{
    connect_node, host_address, run_rust_client_scenarios, Libqi, TestService, LOOPBACK_ANY_PORT,
    SERVICE_NAME,
};
use qi::{node, Protocol};

#[tokio::test]
async fn cpp21_client_against_rust_node() {
    let harness = harness_or_skip!(Libqi::V21);
    let service = TestService::new();
    let mut init = node::init();
    init.add_service_object(SERVICE_NAME, service.object());
    init.bind(LOOPBACK_ANY_PORT.parse().unwrap());
    let host = init.host_space().start().await.unwrap();

    harness.run_client_scenarios(&host_address(&host)).await;
    assert_eq!(service.wait_ticks().await, [7]);
}

/// The Rust node emulates a NAOqi 2.1 server: the 2.1 client sees the handshake it expects.
#[tokio::test]
async fn cpp21_client_against_legacy_rust_node() {
    let harness = harness_or_skip!(Libqi::V21);
    let service = TestService::new();
    let mut init = node::init();
    init.with_server_protocol(Protocol::Legacy);
    init.add_service_object(SERVICE_NAME, service.object());
    init.bind(LOOPBACK_ANY_PORT.parse().unwrap());
    let host = init.host_space().start().await.unwrap();

    harness.run_client_scenarios(&host_address(&host)).await;
    assert_eq!(service.wait_ticks().await, [7]);
}

#[tokio::test]
async fn rust_client_against_cpp21_node() {
    let harness = harness_or_skip!(Libqi::V21);
    let (_sd, sd_address) = harness.service_directory().await;
    let mut cpp_service = harness.service(&sd_address).await;
    let client = connect_node(sd_address).await;
    assert_eq!(
        client.service_directory().protocol(),
        Some(Protocol::Legacy)
    );
    run_rust_client_scenarios(&client, &mut cpp_service, Libqi::V21).await;
}

#[tokio::test]
async fn rust_service_on_cpp21_service_directory_serves_cpp21_client() {
    let harness = harness_or_skip!(Libqi::V21);
    let (_sd, sd_address) = harness.service_directory().await;
    let service = TestService::new();
    let mut init = node::init();
    init.add_service_object(SERVICE_NAME, service.object());
    init.bind(LOOPBACK_ANY_PORT.parse().unwrap());
    let provider = init
        .connect_to_space(sd_address, None)
        .start()
        .await
        .unwrap();
    assert_eq!(
        provider.service_directory().protocol(),
        Some(Protocol::Legacy)
    );

    harness.run_client_scenarios(&sd_address).await;
    assert_eq!(service.wait_ticks().await, [7]);
}

#[tokio::test]
async fn cpp21_service_on_rust_service_directory_serves_rust_client() {
    let harness = harness_or_skip!(Libqi::V21);
    let mut init = node::init();
    init.bind(LOOPBACK_ANY_PORT.parse().unwrap());
    let host = init.host_space().start().await.unwrap();
    let sd_address = host_address(&host);
    let mut cpp_service = harness.service(&sd_address).await;
    let client = connect_node(sd_address).await;
    // The service directory is Rust: the client speaks the standard protocol with it, and the
    // legacy protocol with the 2.1 service.
    assert_eq!(
        client.service_directory().protocol(),
        Some(Protocol::Standard)
    );
    run_rust_client_scenarios(&client, &mut cpp_service, Libqi::V21).await;
}

/// Credentials cannot be verified by a NAOqi 2.1 server: connecting with some is an error that
/// says so, rather than a silent fallback.
#[tokio::test]
async fn credentials_are_refused_by_a_cpp21_node() {
    let harness = harness_or_skip!(Libqi::V21);
    let (_sd, sd_address) = harness.service_directory().await;
    let mut credentials = qi::value::KeyDynValueMap::new();
    credentials.set("auth_user", "nao");
    credentials.set("auth_token", "secret");
    let error = node::init()
        .connect_to_space(sd_address, Some(credentials))
        .start()
        .await
        .expect_err("the connection fails");
    assert!(
        error.to_string().contains("legacy protocol"),
        "unexpected error: {error}"
    );
}

/// The probe of the harness does what a NAOqi 2.1 program does with a robot (`ALSystem`,
/// `ALMemory` with its subscriber objects, `ALTextToSpeech`, `ALMotion`), against the simulated
/// robot in its NAOqi 2.1 configuration.
#[cfg(feature = "naoqi-sim")]
#[tokio::test]
async fn cpp21_probe_against_naoqi_sim() {
    use qi::naoqi_sim::{Config, RobotModel, Simulator};
    use std::process::Stdio;
    use tokio::time::timeout;
    let harness = harness_or_skip!(Libqi::V21);
    let config = Config::new(RobotModel::Nao)
        .with_version("2.1.4.13")
        .on_loopback();
    assert_eq!(config.resolved_protocol(), Protocol::Legacy);
    let simulator = Simulator::start(config).await.unwrap();
    let address = simulator.address().expect("a loopback endpoint");

    let output = timeout(
        interop_common::TIMEOUT,
        harness
            .command("qi-cpp-naoqi-probe")
            .args(["--qi-url", &address.to_string()])
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .expect("the probe completes in time")
    .expect("the probe runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let steps: Vec<interop_common::Step> = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let failed: Vec<&interop_common::Step> = steps.iter().filter(|step| !step.ok).collect();
    assert!(
        failed.is_empty() && output.status.success(),
        "probe failures: {failed:?}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let version = steps
        .iter()
        .find(|step| step.step == "systemVersion")
        .and_then(|step| step.result.as_ref())
        .and_then(|result| result.as_str().map(ToOwned::to_owned));
    assert_eq!(version.as_deref(), Some("2.1.4.13"));
    simulator.shutdown().await;
}
