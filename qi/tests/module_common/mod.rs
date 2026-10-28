//! Support for the tests of modules: building the library of the example module
//! (`examples/rust-module`, the module `rustexample`), staging it in an SDK prefix the way
//! `libqi` finds modules, and running the programs that load it.

#![allow(dead_code)]

use qi::{node, service_directory::LocalServiceDirectory, AnyObject, Node};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::Duration,
};

/// The name of the example module.
pub const MODULE: &str = "rustexample";

/// The root of the repository.
pub fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crate is in the repository")
        .to_owned()
}

/// Builds the library of the example module with the given features and environment, once per
/// configuration and test process, in a target directory of its own (the one of the tests is
/// locked by the running Cargo). Returns the path of `librustexample.so`.
pub fn build_example(config: &str, features: &[&str], env: &[(&str, String)]) -> PathBuf {
    static BUILT: Mutex<Option<HashMap<String, PathBuf>>> = Mutex::new(None);
    let mut built = BUILT.lock().unwrap();
    if let Some(path) = built.get_or_insert_with(HashMap::new).get(config) {
        return path.clone();
    }
    let target_dir = repository().join("target/module-tests").join(config);
    let mut command = Command::new(env!("CARGO"));
    command
        .current_dir(repository())
        .args([
            "build",
            "--quiet",
            "-p",
            "qi-module-example",
            "--target-dir",
        ])
        .arg(&target_dir);
    if !features.is_empty() {
        command.arg("--features").arg(features.join(","));
    }
    for (key, value) in env {
        command.env(key, value);
    }
    let status = command.status().expect("cargo runs");
    assert!(status.success(), "the example module builds ({config})");
    let path = target_dir.join("debug/librustexample.so");
    assert!(path.is_file(), "{} exists", path.display());
    built
        .get_or_insert_with(HashMap::new)
        .insert(config.to_owned(), path.clone());
    path
}

/// An SDK prefix declaring the example module: `lib/librustexample.so` and
/// `share/qi/module/rustexample.mod`.
pub fn stage_prefix(name: &str, library: &Path) -> PathBuf {
    let prefix = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("module-prefixes")
        .join(name);
    std::fs::create_dir_all(prefix.join("lib")).unwrap();
    std::fs::create_dir_all(prefix.join("share/qi/module")).unwrap();
    std::fs::copy(library, prefix.join("lib/librustexample.so")).unwrap();
    std::fs::write(prefix.join("share/qi/module/rustexample.mod"), "cpp\n").unwrap();
    prefix
}

/// A space hosted by the test, on the loopback interface.
pub async fn host_space() -> (Node<LocalServiceDirectory>, String) {
    let mut init = node::init();
    init.bind("tcp://127.0.0.1:0".parse().unwrap());
    let host = init.host_space().start().await.unwrap();
    let url = host
        .endpoints()
        .iter()
        .map(ToString::to_string)
        .find(|endpoint| endpoint.starts_with("tcp://"))
        .expect("the space has a TCP endpoint");
    (host, url)
}

/// A child process, killed when dropped.
pub struct Process(pub Child);

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Process {
    pub fn spawn(command: &mut Command) -> Self {
        Self(
            command
                .stdout(Stdio::null())
                .stderr(verbose_stderr())
                .spawn()
                .expect("the program runs"),
        )
    }
}

/// The standard error of the programs: forwarded with `QI_INTEROP_VERBOSE=1`.
pub fn verbose_stderr() -> Stdio {
    if std::env::var_os("QI_INTEROP_VERBOSE").is_some() {
        Stdio::inherit()
    } else {
        Stdio::null()
    }
}

/// Waits for a service of the space, for up to 20 seconds.
pub async fn wait_for_service(host: &Node<LocalServiceDirectory>, name: &str) -> AnyObject {
    for _ in 0..200 {
        if let Ok(object) = host.service(name).await {
            return object;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the service {name} did not appear");
}

/// Waits until a service is gone from the space, for up to 20 seconds.
pub async fn wait_for_no_service(host: &Node<LocalServiceDirectory>, name: &str) {
    for _ in 0..200 {
        if host.service(name).await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the service {name} did not go away");
}

/// Returns the path if it exists, or skips the test (fails with `QI_INTEROP_REQUIRE=1`),
/// explaining how to build what is missing.
pub fn require(path: PathBuf, what: &str, how: &str) -> Option<PathBuf> {
    if path.exists() {
        return Some(path);
    }
    assert!(
        std::env::var_os("QI_INTEROP_REQUIRE").is_none(),
        "{what} is missing ({}): {how}",
        path.display()
    );
    eprintln!("skipped: {what} is missing ({}): {how}", path.display());
    None
}
