#![warn(missing_docs)]
#![doc = include_str!("naoqi_sim/README.md")]

pub mod alvalue;
pub mod body;
pub mod log;
pub mod memory;
pub mod robot;
pub mod script;
pub mod services;
pub mod simulator;

pub use self::{
    alvalue::AlValue,
    body::Body,
    log::{LogHub, LogMessage},
    memory::Memory,
    robot::RobotModel,
    script::Script,
    simulator::{Config, Simulator},
};

/// Builds a `qi` error carrying a message, the way NAOqi reports errors to callers.
pub(crate) fn error<M: std::fmt::Display>(message: M) -> qi::Error {
    qi::Error::Other(message.to_string().into())
}

/// Locks a mutex, recovering the guard if the lock was poisoned by a panicking task.
pub(crate) fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| {
        mutex.clear_poison();
        err.into_inner()
    })
}

/// The current wall clock time as NAOqi timestamps: seconds and microseconds since the epoch.
pub(crate) fn now_secs_usecs() -> (i32, i32) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    (
        i32::try_from(now.as_secs()).unwrap_or(i32::MAX),
        i32::try_from(now.subsec_micros()).unwrap_or(0),
    )
}
