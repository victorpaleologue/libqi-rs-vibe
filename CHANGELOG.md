# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the crates follow
[Semantic Versioning](https://semver.org/): until 1.0, minor versions may break the API.

The two crates, `libqi-vibe` and `libqi-macros-vibe`, share one version number and are
released together.

## [Unreleased]

### Added

- Compatibility with NAOqi 2.1 robots (the `libqi` of 2014): the protocol variant of a peer is
  detected when a session is established (`qi::Protocol`, readable on `ObjectClient` and on the
  service directory client). A legacy server (error reply to the authentication call after its
  `Capabilities` message) gets the local capabilities with a `Capabilities` message; a legacy
  client (first request without authentication) is accepted when no authenticator is set.
  Service infos are read with or without their `objectUid` field and written without it to
  peers without the `ObjectPtrUID` capability; members with optional or variadic signatures are
  hidden from legacy peers.
- Emulation of a NAOqi 2.1 server: `InitializingNode::with_server_protocol(Protocol::Legacy)`,
  `naoqi_sim::Config::with_protocol`, and `naoqi-sim --protocol legacy` (the default for a
  NAOqi version below 2.3).
- `interop/cpp21/`: the harness ported to the `libqi` of NAOqi 2.1, built from source in an
  Ubuntu 14.04 container, with its byte fixtures (`interop/vectors/libqi-2.1-values.jsonl`), a
  NAOqi probe for `naoqi-sim`, and `qi/tests/interop_cpp21.rs`.

### Fixed

- A reply to a call that requests a return signature the value does not convert to is sent
  with the declared type, like `libqi` does, instead of as a dynamic value (which lost the
  structure annotations the caller needs to convert it).

## [0.1.0] - 2026-09-29

First release of libqi-rs-vibe, a fork of [libqi-rs](https://github.com/nyibbang/libqi-rs)
completed into a stack that interoperates with `libqi` 4.0.5 processes and NAOqi robots.

The crates require Rust 1.90 or later and are licensed under the BSD 3-Clause license.

### Added

- `libqi-vibe` (used as `qi`): nodes, sessions, services, objects, signals, properties, object
  passing in both directions, the special bound-object actions, cooperative cancellation,
  capability negotiation, user/token authentication, a hostable service directory with its
  signals, and TLS transports (`tcps://`, the endpoint of NAOqi 2.9 robots).
- `#[qi::object]` (in `libqi-macros-vibe`): a trait becomes a meta object, an `Object` adapter and a typed client.
- `qi::value`, `qi::format`: the type system and binary format, byte-identical with `libqi`
  4.0.5 on 66 reference fixtures, with runtime `Value` conversion to callee signatures.
- `qi::messaging`: message framing, TCP and TLS channels, client and server loops.
- Feature `cli`: the `qi-cli` command (`info`, `call`, `post`, `watch`, `get`, `set`).
- Feature `naoqi-sim`: `qi::naoqi_sim` and the `naoqi-sim` command, a simulated NAO or Pepper with 16 NAOqi services, validated against the
  ROS 2 `naoqi_driver2`.
- Interoperability harness against C++ `libqi` (`interop/`), architecture and validation
  documentation, contributed patches for the ros-naoqi build.

### Known limitations

See `docs/issues/`: no mutual TLS (`tcpsm://`), no `Manageable` statistics members, no
gateway mode, posts are not flushed at shutdown, and two pinned byte-level deviations in the
runtime `Value` model (invalid dynamics, structure annotations inside dynamics).

[Unreleased]: https://github.com/victorpaleologue/libqi-rs-vibe/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/victorpaleologue/libqi-rs-vibe/releases/tag/v0.1.0
