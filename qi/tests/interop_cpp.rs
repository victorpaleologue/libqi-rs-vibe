//! Interoperability tests against the reference C++ implementation of the qi framework
//! (`libqi` 4.0.5).
//!
//! The tests spawn the programs of the `interop/cpp` harness (see `interop/cpp/README.md`) and
//! exercise every combination of C++ and Rust service directories, services and clients over TCP
//! loopback:
//!
//! - a C++ client running its whole scenario list against a Rust node hosting the service
//!   directory and the test service,
//! - a Rust client running the same scenarios against a C++ service directory and service,
//! - a Rust service registered on a C++ service directory, used by a C++ client,
//! - a C++ service registered on a Rust service directory, used by a Rust client,
//! - a C++ client against a Rust node that emulates the legacy protocol of NAOqi 2.1, which
//!   `libqi` 4.0.5 supports with its own compatibility code.
//!
//! They are skipped when the harness is not built. Set `QI_INTEROP_CPP_BUILD_DIR` to the directory
//! of its binaries (defaults to `interop/cpp/build` in the repository), `QI_INTEROP_REQUIRE=1` to
//! fail instead of skipping, and `QI_INTEROP_VERBOSE=1` to forward the logs of the C++ processes
//! to the standard error output.

mod interop_common;

use interop_common::{
    connect_node, host_address, run_rust_client_scenarios, Libqi, TestService, LOOPBACK_ANY_PORT,
    SERVICE_NAME,
};
use qi::{node, Protocol};

#[tokio::test]
async fn cpp_client_against_rust_node() {
    let harness = harness_or_skip!(Libqi::V405);
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
    let harness = harness_or_skip!(Libqi::V405);
    let (_sd, sd_address) = harness.service_directory().await;
    let mut cpp_service = harness.service(&sd_address).await;
    let client = connect_node(sd_address).await;
    assert_eq!(
        client.service_directory().protocol(),
        Some(Protocol::Standard)
    );
    run_rust_client_scenarios(&client, &mut cpp_service, Libqi::V405).await;
}

#[tokio::test]
async fn rust_service_on_cpp_service_directory_serves_cpp_client() {
    let harness = harness_or_skip!(Libqi::V405);
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
    let harness = harness_or_skip!(Libqi::V405);
    let mut init = node::init();
    init.bind(LOOPBACK_ANY_PORT.parse().unwrap());
    let host = init.host_space().start().await.unwrap();
    let sd_address = host_address(&host);
    let mut cpp_service = harness.service(&sd_address).await;
    let client = connect_node(sd_address).await;
    run_rust_client_scenarios(&client, &mut cpp_service, Libqi::V405).await;
}

#[tokio::test]
async fn cpp_client_over_tls_against_rust_node() {
    let harness = harness_or_skip!(Libqi::V405);
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

/// `libqi` 4.0.5 clients still support servers of the legacy protocol (they advertise their
/// capabilities after the failed authentication): a Rust node emulating NAOqi 2.1 must look like
/// one to them. Cancellation does not exist in that protocol, and optionals are hidden from
/// legacy peers, so those scenarios are left out.
#[tokio::test]
async fn cpp_client_against_legacy_rust_node() {
    let harness = harness_or_skip!(Libqi::V405);
    let service = TestService::new();
    let mut init = node::init();
    init.with_server_protocol(Protocol::Legacy);
    init.add_service_object(SERVICE_NAME, service.object());
    init.bind(LOOPBACK_ANY_PORT.parse().unwrap());
    let host = init.host_space().start().await.unwrap();

    let scenarios = [
        "sd_services",
        "sd_machineId",
        "add",
        "concat",
        "echoDynamic_int",
        "echoDynamic_string",
        "echoDynamic_list",
        "echoDynamic_struct",
        "echoDynamic_empty",
        "echoList",
        "echoList_empty",
        "echoMap",
        "echoStruct",
        "echoBuffer",
        "fail",
        "sleepMs_short",
        "signal_fired",
        "signal_void",
        "property_value",
        "property_text",
        "property_generic",
        "makeCounter",
        "useCallback",
        "bigString",
    ];
    harness
        .run_selected_client_scenarios(&host_address(&host), &scenarios.join(","), 27)
        .await;
    assert_eq!(service.wait_ticks().await, [7]);
}
