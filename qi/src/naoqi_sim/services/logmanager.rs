//! `LogManager`: the `qicore` log manager, whose listeners receive the log messages of the
//! robot.

use super::Context;
use crate::naoqi_sim::log::{level, LogHub, LogMessage};
use futures::StreamExt;
use qi::{dynamic::ObjectBuilder, AnyObject, Property, Signal};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicI32, Ordering},
        Arc, Mutex,
    },
};

/// Builds the `LogManager` service object.
pub fn object(context: &Context) -> AnyObject {
    let logs = context.logs.clone();
    let providers: Arc<Mutex<HashMap<i32, AnyObject>>> = Arc::default();
    let next_provider = Arc::new(AtomicI32::new(1));
    let mut builder = ObjectBuilder::new();
    builder.set_description(
        "LogManager gathers the log messages of the robot and dispatches them to listeners.",
    );
    method!(builder, "getListener", [logs], |(): ()| {
        Ok(listener_object(&logs))
    });
    method!(builder, "createListener", [logs], |(): ()| {
        Ok(listener_object(&logs))
    });
    method!(builder, "log", [logs], |messages: Vec<LogMessage>| {
        for message in messages {
            logs.log(message.level, &message.category, message.message);
        }
        Ok(())
    });
    method!(
        builder,
        "addProvider",
        [providers, next_provider],
        |provider: AnyObject| {
            let id = next_provider.fetch_add(1, Ordering::Relaxed);
            crate::naoqi_sim::lock(&providers).insert(id, provider);
            Ok(id)
        }
    );
    method!(builder, "removeProvider", [providers], |id: i32| {
        crate::naoqi_sim::lock(&providers).remove(&id);
        Ok(())
    });
    AnyObject::new(builder.build())
}

/// Builds a `LogListener` object: it forwards the messages of the hub whose level is at most
/// its `logLevel` (and matches its filters) to its `onLogMessage` signal.
pub fn listener_object(logs: &LogHub) -> AnyObject {
    let on_message: Signal<LogMessage> = Signal::with_capacity(256);
    let on_messages: Signal<Vec<LogMessage>> = Signal::with_capacity(256);
    let on_messages_with_backlog: Signal<Vec<LogMessage>> = Signal::with_capacity(256);
    let log_level: Property<i32> = Property::new(level::INFO);
    let filters: Arc<Mutex<HashMap<String, i32>>> = Arc::default();

    // The forwarding task ends when the listener object is dropped: it holds weak references
    // to the signals through the subscription of the hub only.
    let hub = logs.signal().clone();
    let stop = tokio_util_stop();
    {
        let on_message = on_message.clone();
        let on_messages = on_messages.clone();
        let on_messages_with_backlog = on_messages_with_backlog.clone();
        let log_level = log_level.clone();
        let filters = Arc::clone(&filters);
        let mut stopped = stop.1;
        tokio::spawn(async move {
            let Ok(mut messages) = hub.subscribe().await else {
                return;
            };
            loop {
                let message = tokio::select! {
                    message = messages.next() => match message {
                        Some(message) => message,
                        None => break,
                    },
                    _ = stopped.changed() => break,
                };
                let max_level = log_level.get().await.unwrap_or(level::INFO);
                let allowed = {
                    let filters = crate::naoqi_sim::lock(&filters);
                    match filters
                        .iter()
                        .filter(|(category, _)| category_matches(category, &message.category))
                        .map(|(_, level)| *level)
                        .max()
                    {
                        Some(filter_level) => message.level <= filter_level,
                        None => message.level <= max_level,
                    }
                };
                if allowed {
                    on_message.emit(message.clone());
                    on_messages.emit(vec![message.clone()]);
                    on_messages_with_backlog.emit(vec![message]);
                }
            }
        });
    }

    let mut builder = ObjectBuilder::new();
    builder.set_description("A listener of the log messages of the robot.");
    builder.add_signal("onLogMessage", on_message);
    builder.add_signal("onLogMessages", on_messages);
    builder.add_signal("onLogMessagesWithBacklog", on_messages_with_backlog);
    builder.add_property("logLevel", log_level.clone());
    method!(builder, "setLevel", [log_level], |new_level: i32| {
        log_level
            .set(new_level.clamp(level::SILENT, level::DEBUG))
            .await
    });
    method!(builder, "addFilter", [filters], |(
        category,
        filter_level,
    ): (String, i32)| {
        crate::naoqi_sim::lock(&filters).insert(category, filter_level);
        Ok(())
    });
    method!(builder, "clearFilters", [filters], |(): ()| {
        crate::naoqi_sim::lock(&filters).clear();
        Ok(())
    });
    method!(builder, "setCategory", [filters], |(
        category,
        filter_level,
    ): (String, i32)| {
        crate::naoqi_sim::lock(&filters).insert(category, filter_level);
        Ok(())
    });
    method!(builder, "clearAndSet", [filters], |categories: HashMap<
        String,
        i32,
    >| {
        *crate::naoqi_sim::lock(&filters) = categories;
        Ok(())
    });
    let object = AnyObject::new(builder.build());
    // Stop the forwarding task when the listener is released by its last user.
    tokio::spawn({
        let weak = std::sync::Arc::downgrade(object.as_arc());
        let stop = stop.0;
        async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            loop {
                interval.tick().await;
                if weak.upgrade().is_none() {
                    stop.send(true).ok();
                    break;
                }
            }
        }
    });
    object
}

/// Creates the channel stopping the forwarding task of a listener.
fn tokio_util_stop() -> (
    tokio::sync::watch::Sender<bool>,
    tokio::sync::watch::Receiver<bool>,
) {
    tokio::sync::watch::channel(false)
}

/// Returns true if a filter category (with `*` wildcards) matches a message category.
fn category_matches(filter: &str, category: &str) -> bool {
    if filter == "*" {
        return true;
    }
    match filter.strip_suffix('*') {
        Some(prefix) => category.starts_with(prefix),
        None => filter == category,
    }
}
