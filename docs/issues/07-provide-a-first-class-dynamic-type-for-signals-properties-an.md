---
title: Provide a first-class dynamic type for signals, properties and typed clients
labels: enhancement
---

## Problem

No type in `qi::value` can be used wherever a *dynamic* (`m`) parameter is expected by the typed APIs:

- `Dynamic<T>` implements `Reflect` (so it works in `#[qi::object]` method signatures and `ObjectBuilder::add_method`) but not `RuntimeReflect`, so it cannot be the element type of a signal or a property (`ObjectBuilder::add_signal`, `Property<T>`, `Object::subscribe::<_, T>` bounds).
- `Value<'static>` implements `RuntimeReflect` but not `Reflect`, since its type is only known at runtime, so it fits neither the macro nor the builder bounds.

NAOqi is full of `m` signals and properties (`ALMemory.subscriber(...).signal`, every `ALValue` property). `naoqi-sim` works around this with its own `AlValue` newtype (`qi/src/naoqi_sim/alvalue.rs`): `Reflect` returns `None` (dynamic), `RuntimeReflect` is implemented, `IntoValue` wraps in `Value::Dynamic` and `FromValue` strips it. The arora HAL re-implements the same thing.

## Proposal

Move that type into `qi::value` as `qi::value::AnyValue` (or make `Dynamic<Value<'static>>` implement `RuntimeReflect`). Then `#[qi::object]` traits can declare `Signal<AnyValue>` and `Property<AnyValue>`, `Object::subscribe::<_, AnyValue>` works on any `m` signal, and the local copies go away.

Related: `qi/src/session/handler.rs::register_event` now converts emitted signal parameters to the declared signature before sending, so emitters no longer need to pre-wrap values in `Value::Dynamic`; the missing piece is the type on the declaration side.
