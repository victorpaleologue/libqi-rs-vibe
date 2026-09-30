---
title: Implement the Manageable statistics and tracing bound-object actions
labels: enhancement
---

## Gap

Every libqi bound object exposes the `Manageable` members (action ids 80 to 99): `isStatsEnabled`, `enableStats`, `stats`, `clearStats`, `isTraceEnabled`, `enableTrace` and the `traceObject` signal. The Rust `Object` adapter answers the special actions every client uses (`registerEvent`, `unregisterEvent`, `metaObject`, `terminate`, `property`, `setProperty`, `properties`, `registerEventWithSignature`) but not the `Manageable` ones: calling them returns a method-not-found error.

## Impact

Tools such as `qicli trace` and Choregraphe's profiler call these members and fail against Rust services. Regular clients do not use them.

## Work

- Add the seven members to the meta object built by `qi/src/object/generic.rs` for every hosted object, with libqi's ids and signatures (`libqi/qi/type/detail/manageable.hpp`, `src/type/manageable.cpp`).
- Keep per-action call counts and durations when stats are enabled; emit `traceObject` events (`(IiIm(ll)i)<EventTrace,...>`) when tracing is enabled.
- Extend `interop/vectors` with a meta-object fixture including these members so byte identity keeps holding.

Where noted: `docs/architecture.md`, "Limitations"; `README.md`.
