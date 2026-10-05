//! The log messages of the simulated robot, served by the `LogManager` service.
//!
//! NAOqi robots run the `qicore` `LogManager`, whose listeners receive [`LogMessage`]s. The
//! [`LogHub`] collects the messages the simulator emits (heartbeats and notable actions) and
//! mirrors them to `tracing`.

use qi::{
    value::{FromValue, FromValueError, IntoValue, Reflect, RuntimeReflect, Type, Value},
    Signal, Subscription,
};
use std::{
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

/// The log levels of `qi::LogLevel`.
pub mod level {
    /// No message.
    pub const SILENT: i32 = 0;
    /// A fatal error.
    pub const FATAL: i32 = 1;
    /// An error.
    pub const ERROR: i32 = 2;
    /// A warning.
    pub const WARNING: i32 = 3;
    /// An information.
    pub const INFO: i32 = 4;
    /// A verbose information.
    pub const VERBOSE: i32 = 5;
    /// A debugging message.
    pub const DEBUG: i32 = 6;
}

/// A log message, as `qicore` defines it: the structure
/// `(sisssIll)<LogMessage,source,level,category,location,message,id,date,systemDate>`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LogMessage {
    /// The origin of the message as `file:function:line`.
    pub source: String,
    /// The level of the message, see [`level`].
    pub level: i32,
    /// The category of the message.
    pub category: String,
    /// The emitter of the message as `machineId:processId`.
    pub location: String,
    /// The message itself.
    pub message: String,
    /// The unique identifier of the message.
    pub id: u32,
    /// The steady clock timestamp of the message, in nanoseconds.
    pub date: i64,
    /// The wall clock timestamp of the message, in nanoseconds since the epoch.
    pub system_date: i64,
}

impl LogMessage {
    /// The name of the structure in the `qi` type system.
    pub const TYPE_NAME: &'static str = "LogMessage";

    /// The names of the fields of the structure in the `qi` type system.
    pub const FIELD_NAMES: [&'static str; 8] = [
        "source",
        "level",
        "category",
        "location",
        "message",
        "id",
        "date",
        "systemDate",
    ];

    /// The type of the structure.
    pub fn struct_type() -> Type {
        let types = [
            Type::String,
            Type::Int32,
            Type::String,
            Type::String,
            Type::String,
            Type::UInt32,
            Type::Int64,
            Type::Int64,
        ];
        Type::struct_of(Self::TYPE_NAME, Self::FIELD_NAMES.into_iter().zip(types))
    }
}

impl Reflect for LogMessage {
    fn ty() -> Option<Type> {
        Some(Self::struct_type())
    }
}

impl RuntimeReflect for LogMessage {
    fn ty(&self) -> Type {
        Self::struct_type()
    }
}

impl IntoValue<'static> for LogMessage {
    fn into_value(self) -> Value<'static> {
        Value::Tuple(vec![
            self.source.into_value(),
            self.level.into_value(),
            self.category.into_value(),
            self.location.into_value(),
            self.message.into_value(),
            self.id.into_value(),
            self.date.into_value(),
            self.system_date.into_value(),
        ])
    }
}

impl FromValue<'static> for LogMessage {
    fn from_value(value: Value<'static>) -> Result<Self, FromValueError> {
        let value = value.convert_to(Some(&Self::struct_type()))?;
        let (source, level, category, location, message, id, date, system_date) =
            value.cast_into()?;
        Ok(Self {
            source,
            level,
            category,
            location,
            message,
            id,
            date,
            system_date,
        })
    }
}

/// The collector of the log messages of the simulator. Cheap to clone: clones share the hub.
#[derive(Clone)]
pub struct LogHub(Arc<Inner>);

struct Inner {
    signal: Signal<LogMessage>,
    next_id: AtomicU32,
    location: String,
    start: Instant,
}

impl LogHub {
    /// Creates a hub. The location of the messages is the local machine and process.
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            signal: Signal::with_capacity(256),
            next_id: AtomicU32::new(1),
            location: format!(
                "{}:{}",
                qi::value::os::MachineId::local(),
                std::process::id()
            ),
            start: Instant::now(),
        }))
    }

    /// Emits a log message from the given source and category.
    pub fn log(&self, level: i32, category: &str, message: impl Into<String>) {
        let message = message.into();
        match level {
            level::FATAL | level::ERROR => {
                tracing::error!(target: "naoqi_sim::robot", category, "{message}")
            }
            level::WARNING => tracing::warn!(target: "naoqi_sim::robot", category, "{message}"),
            level::INFO => tracing::info!(target: "naoqi_sim::robot", category, "{message}"),
            level::VERBOSE => tracing::debug!(target: "naoqi_sim::robot", category, "{message}"),
            _ => tracing::trace!(target: "naoqi_sim::robot", category, "{message}"),
        }
        let now = Instant::now();
        let date = i64::try_from(now.duration_since(self.0.start).as_nanos()).unwrap_or(i64::MAX);
        let system_date = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|elapsed| i64::try_from(elapsed.as_nanos()).ok())
            .unwrap_or_default();
        self.0.signal.emit(LogMessage {
            source: format!("naoqi-sim/{category}.rs:log:0"),
            level,
            category: category.to_owned(),
            location: self.0.location.clone(),
            message,
            id: self.0.next_id.fetch_add(1, Ordering::Relaxed),
            date,
            system_date,
        });
    }

    /// Emits an informational message.
    pub fn info(&self, category: &str, message: impl Into<String>) {
        self.log(level::INFO, category, message);
    }

    /// Emits a warning.
    pub fn warn(&self, category: &str, message: impl Into<String>) {
        self.log(level::WARNING, category, message);
    }

    /// Emits a verbose message.
    pub fn verbose(&self, category: &str, message: impl Into<String>) {
        self.log(level::VERBOSE, category, message);
    }

    /// The signal of all messages. Listeners filter it by level.
    pub fn signal(&self) -> &Signal<LogMessage> {
        &self.0.signal
    }

    /// Subscribes to all messages.
    pub async fn subscribe(&self) -> qi::Result<Subscription<LogMessage>> {
        self.0.signal.subscribe().await
    }
}

impl Default for LogHub {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for LogHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogHub")
            .field("location", &self.0.location)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[test]
    fn log_message_signature_matches_qicore() {
        let signature = qi::value::Signature::from(LogMessage::struct_type()).to_string();
        assert_eq!(
            signature,
            "(sisssIll)<LogMessage,source,level,category,location,message,id,date,systemDate>"
        );
    }

    #[test]
    fn log_message_value_round_trip() {
        let message = LogMessage {
            source: "a.cpp:f:1".to_owned(),
            level: level::INFO,
            category: "cat".to_owned(),
            location: "m:1".to_owned(),
            message: "hello".to_owned(),
            id: 7,
            date: 1,
            system_date: 2,
        };
        let value = message.clone().into_value();
        assert_eq!(LogMessage::from_value(value).unwrap(), message);
        // Old style tuples of the same shape convert too.
        let value = Value::Tuple(vec![
            "a.cpp:f:1".to_owned().into_value(),
            Value::Int64(4),
            "cat".to_owned().into_value(),
            "m:1".to_owned().into_value(),
            "hello".to_owned().into_value(),
            Value::Int32(7),
            Value::Int32(1),
            Value::Int32(2),
        ]);
        assert_eq!(LogMessage::from_value(value).unwrap(), message);
    }

    #[tokio::test]
    async fn hub_delivers_messages() {
        let hub = LogHub::new();
        let mut messages = hub.subscribe().await.unwrap();
        hub.info("test", "hello");
        let message = messages.next().await.unwrap();
        assert_eq!(message.message, "hello");
        assert_eq!(message.level, level::INFO);
        assert_eq!(message.source.matches(':').count(), 2);
    }
}
