//! `ALMemory`: the key/value store and event bus of the robot.

use super::Context;
use crate::naoqi_sim::{alvalue::AlValue, error, lock, memory::Memory};
use futures::StreamExt;
use qi::{dynamic::ObjectBuilder, value::Value, AnyObject, ObjectExt};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// The version `ALMemory.version` reports.
const VERSION: &str = "2.8.7.4";

/// Builds the `ALMemory` service object.
pub fn object(context: &Context) -> AnyObject {
    let memory = context.memory.clone();
    let logs = context.logs.clone();
    let context = context.clone();
    let callbacks: Arc<Mutex<HashMap<(String, String), tokio::task::AbortHandle>>> = Arc::default();
    let mut builder = ObjectBuilder::new();
    builder.set_description("ALMemory provides a centralized memory used to store all key information related to the hardware configuration of your robot.");

    method!(builder, "getData", [memory], |key: String| {
        memory
            .get(&key)
            .ok_or_else(|| error(format!("ALMemory::getData\n\tkey not found: {key}")))
    });
    method!(builder, "getListData", [memory], |keys: AlValue| {
        let keys = keys
            .names()
            .ok_or_else(|| error("ALMemory::getListData\n\texpected a list of keys"))?;
        Ok(memory
            .get_list(&keys)
            .into_iter()
            .map(|value| value.unwrap_or(AlValue(Value::Unit)))
            .collect::<Vec<AlValue>>())
    });
    method!(builder, "insertData", [memory], |(key, value): (
        String,
        AlValue
    )| {
        memory.insert(key, value);
        Ok(())
    });
    method!(builder, "insertListData", [memory], |data: AlValue| {
        let pairs = data
            .as_list()
            .ok_or_else(|| error("ALMemory::insertListData\n\texpected a list of [key, value]"))?;
        for pair in pairs {
            let items = pair.as_list().unwrap_or_default();
            match items.as_slice() {
                [key, value] if key.as_str().is_some() => {
                    memory.insert(key.as_str().unwrap_or_default(), value.clone());
                }
                _ => {
                    return Err(error(
                        "ALMemory::insertListData\n\texpected [key, value] pairs",
                    ))
                }
            }
        }
        Ok(())
    });
    method!(builder, "raiseEvent", [memory], |(key, value): (
        String,
        AlValue
    )| {
        memory.raise(key, value);
        Ok(())
    });
    method!(builder, "raiseMicroEvent", [memory], |(key, value): (
        String,
        AlValue
    )| {
        memory.raise(key, value);
        Ok(())
    });
    method!(builder, "declareEvent", [memory], |key: String| {
        memory.declare(key);
        Ok(())
    });
    method!(builder, "removeData", [memory], |key: String| {
        memory.remove(&key);
        Ok(())
    });
    method!(builder, "removeMicroEvent", [memory], |key: String| {
        memory.remove(&key);
        Ok(())
    });
    method!(builder, "getDataList", [memory], |filter: String| {
        Ok(memory.keys_containing(&filter))
    });
    method!(builder, "getDataListName", [memory], |(): ()| {
        Ok(memory.keys())
    });
    method!(builder, "getEventList", [memory], |(): ()| {
        Ok(memory.keys())
    });
    method!(builder, "getMicroEventList", [memory], |(): ()| {
        Ok(memory.keys())
    });
    method!(builder, "getType", [memory], |key: String| {
        let value = memory
            .get(&key)
            .ok_or_else(|| error(format!("ALMemory::getType\n\tkey not found: {key}")))?;
        Ok(type_name(&value).to_owned())
    });
    method!(builder, "subscriber", [memory, logs], |key: String| {
        logs.verbose("ALMemory", format!("subscriber created for key {key}"));
        Ok(subscriber_object(&memory, &key))
    });
    method!(
        builder,
        "subscribeToEvent",
        [memory, context, callbacks],
        |(key, module, callback): (String, String, String)| {
            let node = context.node()?;
            let mut changes = memory.subscribe(&key).await?;
            let (module_name, callback_name, event) = (module.clone(), callback, key.clone());
            let logs = context.logs.clone();
            let task = tokio::spawn(async move {
                while let Some(value) = changes.next().await {
                    let service = match node.service(&module_name).await {
                        Ok(service) => service,
                        Err(err) => {
                            logs.warn(
                                "ALMemory",
                                format!(
                                    "cannot reach module {module_name} for event {event}: {err}"
                                ),
                            );
                            continue;
                        }
                    };
                    service
                        .post(
                            callback_name.as_str(),
                            (event.clone(), value, module_name.clone()),
                        )
                        .await;
                }
            });
            if let Some(previous) = lock(&callbacks).insert((key, module), task.abort_handle()) {
                previous.abort();
            }
            Ok(())
        }
    );
    method!(builder, "unsubscribeToEvent", [callbacks], |(
        key,
        module,
    ): (
        String,
        String
    )| {
        match lock(&callbacks).remove(&(key.clone(), module.clone())) {
            Some(task) => {
                task.abort();
                Ok(())
            }
            None => Err(error(format!(
                "ALMemory::unsubscribeToEvent\n\tmodule {module} is not subscribed to {key}"
            ))),
        }
    });
    method!(builder, "version", [], |(): ()| { Ok(VERSION.to_owned()) });
    method!(builder, "ping", [], |(): ()| { Ok(true) });
    AnyObject::new(builder.build())
}

/// Builds the object `ALMemory.subscriber(key)` returns: it exposes the changes of the key as
/// its `signal` signal.
pub fn subscriber_object(memory: &Memory, key: &str) -> AnyObject {
    let mut builder = ObjectBuilder::new();
    builder.set_description(format!("Subscriber to the ALMemory key {key}"));
    builder.add_signal("signal", memory.signal(key));
    let key = key.to_owned();
    builder.add_method("key", move |(): ()| {
        let key = key.clone();
        async move { Ok(key) }
    });
    AnyObject::new(builder.build())
}

/// The NAOqi name of the type of a value.
fn type_name(value: &AlValue) -> &'static str {
    match value.value() {
        Value::Bool(_) => "Bool",
        Value::Int8(_)
        | Value::UInt8(_)
        | Value::Int16(_)
        | Value::UInt16(_)
        | Value::Int32(_)
        | Value::UInt32(_)
        | Value::Int64(_)
        | Value::UInt64(_) => "Int",
        Value::Float32(_) | Value::Float64(_) => "Float",
        Value::String(_) => "String",
        Value::Raw(_) => "Binary",
        Value::List(_) | Value::Tuple(_) => "Array",
        Value::Map(_) => "Map",
        Value::Object(_) => "Object",
        Value::Unit | Value::Option(_) | Value::Dynamic(_) => "Invalid",
    }
}
