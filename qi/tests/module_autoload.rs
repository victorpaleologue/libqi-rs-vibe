//! The library of a module loaded the way NAOqi loads the libraries of its `autoload.ini`:
//! `_createModule` registers the objects of the `autoload` factories as services of the space at
//! `QI_URL`, and `_closeModule` unregisters them. In a test process of its own, as the library is
//! loaded in it.

mod module_common;

use module_common::{build_example, host_space, wait_for_no_service, wait_for_service};
use qi::ObjectExt;
use std::ffi::{c_int, c_void};

#[tokio::test(flavor = "multi_thread")]
async fn create_module_registers_the_autoload_services() {
    let library = build_example("plain", &[], &[]);
    // A copy, for this process to load a library of its own.
    let copy = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("librustexample-autoload.so");
    std::fs::copy(&library, &copy).unwrap();
    let (host, url) = host_space().await;
    // The module reads `QI_URL` when NAOqi creates it; nothing else of this process does.
    std::env::set_var("QI_URL", &url);

    // SAFETY: loading the example module, whose initializers do nothing.
    let library = unsafe { libloading::Library::new(&copy) }.unwrap();
    // SAFETY: the entry points of NAOqi, as `#[qi::module]` declares them. NAOqi passes a
    // `boost::shared_ptr<AL::ALBroker>` by value: a pointer to it under the C++ ABI, which the
    // module only reads for its DCM plugins (none here: the library is built without the bridge).
    let (create, close) = unsafe {
        (
            *library
                .get::<unsafe extern "C" fn(*mut c_void) -> c_int>(b"_createModule\0")
                .unwrap(),
            *library
                .get::<unsafe extern "C" fn() -> c_int>(b"_closeModule\0")
                .unwrap(),
        )
    };
    let mut broker = [0usize; 2];
    // SAFETY: the broker is a stand-in, unused.
    let status = tokio::task::spawn_blocking(move || unsafe { create(broker.as_mut_ptr().cast()) })
        .await
        .unwrap();
    assert_eq!(status, 0);

    let greeter = wait_for_service(&host, "RustGreeter").await;
    let greeting: String = greeter.call("greet", "NAOqi".to_owned()).await.unwrap();
    assert_eq!(greeting, "Hello, NAOqi, from Rust!");
    let dcm = wait_for_service(&host, "RustDcm").await;
    let () = dcm.call("setLedAnimation", false).await.unwrap();
    // Not an autoload factory.
    assert!(host.service("OtherGreeter").await.is_err());

    // SAFETY: as above.
    let status = tokio::task::spawn_blocking(move || unsafe { close() })
        .await
        .unwrap();
    assert_eq!(status, 0);
    wait_for_no_service(&host, "RustGreeter").await;
    wait_for_no_service(&host, "RustDcm").await;
    // Modules are never unloaded: their threads keep running.
    std::mem::forget(library);
}
