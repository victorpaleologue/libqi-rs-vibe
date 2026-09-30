//! The simulated `ALMemory`: a key/value store whose keys are also events.
//!
//! Every key may be read, written and subscribed to. Writing a key (`insertData` or `raiseEvent`
//! in NAOqi terms) notifies the subscribers of the key with the new value, through a signal that
//! the `ALMemory.subscriber(key)` objects expose.

use crate::naoqi_sim::alvalue::AlValue;
use qi::{value::IntoValue, Signal, Subscription};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

/// The key/value store of the simulated robot. Cheap to clone: clones share the same store.
#[derive(Clone, Default)]
pub struct Memory(Arc<Inner>);

#[derive(Default)]
struct Inner {
    entries: RwLock<HashMap<String, Entry>>,
}

#[derive(Default)]
struct Entry {
    /// The value of the key, `None` for a declared event that was never raised.
    value: Option<AlValue>,
    /// The signal notifying the changes of the key, created by the first subscriber.
    signal: Option<Signal<AlValue>>,
}

impl Memory {
    /// Creates an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the value of a key, creating it if needed, and notifies its subscribers.
    ///
    /// This is what both `ALMemory.insertData` and `ALMemory.raiseEvent` do.
    pub fn insert<K, V>(&self, key: K, value: V)
    where
        K: Into<String>,
        V: Into<AlValue>,
    {
        let value = value.into();
        let signal = {
            let mut entries = write(&self.0.entries);
            let entry = entries.entry(key.into()).or_default();
            entry.value = Some(value.clone());
            entry.signal.clone()
        };
        if let Some(signal) = signal {
            signal.emit(value);
        }
    }

    /// Sets the value of a key without notifying its subscribers.
    ///
    /// Used by the periodic sensor updates of the simulation, which are not events in NAOqi.
    pub fn insert_silent<K, V>(&self, key: K, value: V)
    where
        K: Into<String>,
        V: Into<AlValue>,
    {
        let mut entries = write(&self.0.entries);
        entries.entry(key.into()).or_default().value = Some(value.into());
    }

    /// Sets the value of several keys without notifying subscribers.
    pub fn insert_all_silent<I, K, V>(&self, items: I)
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<AlValue>,
    {
        let mut entries = write(&self.0.entries);
        for (key, value) in items {
            entries.entry(key.into()).or_default().value = Some(value.into());
        }
    }

    /// Raises an event: sets the value of the key and notifies its subscribers.
    pub fn raise<K, V>(&self, key: K, value: V)
    where
        K: Into<String>,
        V: Into<AlValue>,
    {
        self.insert(key, value);
    }

    /// Declares a key without value, so that it may be subscribed to and listed.
    pub fn declare<K: Into<String>>(&self, key: K) {
        write(&self.0.entries).entry(key.into()).or_default();
    }

    /// Removes a key. Its subscribers are not notified anymore.
    pub fn remove(&self, key: &str) -> bool {
        write(&self.0.entries).remove(key).is_some()
    }

    /// Returns true if the key exists, even without value.
    pub fn contains(&self, key: &str) -> bool {
        read(&self.0.entries).contains_key(key)
    }

    /// The value of a key, `None` if the key does not exist or has no value.
    pub fn get(&self, key: &str) -> Option<AlValue> {
        read(&self.0.entries)
            .get(key)
            .and_then(|entry| entry.value.clone())
    }

    /// The values of several keys, in the order of the keys.
    pub fn get_list<I, K>(&self, keys: I) -> Vec<Option<AlValue>>
    where
        I: IntoIterator<Item = K>,
        K: AsRef<str>,
    {
        let entries = read(&self.0.entries);
        keys.into_iter()
            .map(|key| {
                entries
                    .get(key.as_ref())
                    .and_then(|entry| entry.value.clone())
            })
            .collect()
    }

    /// The keys of the store, sorted.
    pub fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = read(&self.0.entries).keys().cloned().collect();
        keys.sort();
        keys
    }

    /// The keys containing the given text, sorted. This is `ALMemory.getDataList(filter)`.
    pub fn keys_containing(&self, filter: &str) -> Vec<String> {
        let mut keys: Vec<String> = read(&self.0.entries)
            .keys()
            .filter(|key| key.contains(filter))
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    /// The number of keys.
    pub fn len(&self) -> usize {
        read(&self.0.entries).len()
    }

    /// Returns true if the store has no key.
    pub fn is_empty(&self) -> bool {
        read(&self.0.entries).is_empty()
    }

    /// The signal of the changes of a key, created (with the key) if needed.
    ///
    /// Clones of the signal share their subscribers: this is the signal that the
    /// `ALMemory.subscriber(key)` objects expose as `signal`.
    pub fn signal(&self, key: &str) -> Signal<AlValue> {
        let mut entries = write(&self.0.entries);
        let entry = entries.entry(key.to_owned()).or_default();
        entry.signal.get_or_insert_with(Signal::new).clone()
    }

    /// Subscribes to the changes of a key.
    pub async fn subscribe(&self, key: &str) -> qi::Result<Subscription<AlValue>> {
        self.signal(key).subscribe().await
    }

    /// Reads a key as a 32-bit float, if it is a number.
    pub fn get_f32(&self, key: &str) -> Option<f32> {
        self.get(key).and_then(|value| value.as_f32())
    }

    /// Reads a key as a 32-bit integer, if it is an integral number.
    pub fn get_i32(&self, key: &str) -> Option<i32> {
        self.get(key).and_then(|value| value.as_i32())
    }

    /// Reads a key as a string, if it is one.
    pub fn get_string(&self, key: &str) -> Option<String> {
        self.get(key)
            .and_then(|value| value.as_str().map(str::to_owned))
    }

    /// Reads a key as a boolean, if it is a boolean or a number.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(|value| value.as_bool())
    }

    /// Sets a key from any value of the type system, notifying subscribers.
    pub fn set<K, V>(&self, key: K, value: V)
    where
        K: Into<String>,
        V: IntoValue<'static>,
    {
        self.insert(key, AlValue::new(value));
    }
}

impl std::fmt::Debug for Memory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Memory").field("keys", &self.len()).finish()
    }
}

fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|err| {
        lock.clear_poison();
        err.into_inner()
    })
}

fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(|err| {
        lock.clear_poison();
        err.into_inner()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn insert_notifies_subscribers() {
        let memory = Memory::new();
        let mut changes = memory.subscribe("FrontTactilTouched").await.unwrap();
        memory.insert("FrontTactilTouched", 1.0);
        assert_eq!(changes.next().await, Some(AlValue::from(1.0)));
        assert_eq!(memory.get_f32("FrontTactilTouched"), Some(1.0));
        memory.insert_silent("FrontTactilTouched", 0.0);
        assert_eq!(memory.get_f32("FrontTactilTouched"), Some(0.0));
        memory.insert("FrontTactilTouched", 1.0);
        assert_eq!(changes.next().await, Some(AlValue::from(1.0)));
    }

    #[test]
    fn keys_are_filtered_and_sorted() {
        let memory = Memory::new();
        memory.insert("b/x", 1);
        memory.insert("a/x", 2);
        memory.insert("c", 3);
        assert_eq!(memory.keys_containing("/x"), ["a/x", "b/x"]);
        assert_eq!(memory.keys().len(), 3);
        assert_eq!(
            memory.get_list(["c", "nope"]),
            [Some(AlValue::from(3)), None]
        );
    }
}
