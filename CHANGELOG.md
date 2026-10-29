# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the crates follow
[Semantic Versioning](https://semver.org/): a change that breaks the API is a major version.

The two crates, `libqi-vibe` and `libqi-macros-vibe`, share one version number and are
released together.

## [Unreleased]

### Added

- Modules written in Rust (feature `module`): `#[qi::module]` on a `mod` of a `cdylib` crate
  makes its public functions, its `#[qi::factory]` functions (objects; `autoload` ones are
  registered when NAOqi loads the library) and its `#[qi::dcm]` functions (DCM plugins) a `qi`
  module. Members taking a `qi::module::Session` reach the space of their host. The library
  serves its members from a private loopback space and exports `qi_rs_module_v1` (hosts written
  with this crate, `qi::module::host`), `_createModule`/`_closeModule` (NAOqi's `autoload.ini`) and,
  with the feature `module-cpp` built against a libqi SDK, `qi_module_init` (`qilaunch`,
  `qi::import` of libqi 2.3 and later) or the factory registration of libqi 2.1 (its `qi-launch`).
  See `docs/modules.md`.
- DCM plugins (`qi::module::naoqi`, feature `module-naoqi`, built against the C++ SDK of NAOqi):
  an `ALModule` calling the plugin in the pre-process and post-process callbacks of the DCM, with
  ALMemory values read in place and DCM aliases.
- `qi-launch` (feature `launch`): the counterpart of `qilaunch`, with its options and behavior,
  plus the positional libraries and `-a` of the `qi-launch` of NAOqi 2.1.
- `examples/rust-module`: the example module `rustexample` (functions, factories, the DCM plugin
  `RustEyes` and the `RustDcm` service reporting on it).
- `MachineId::set_local`, for a module to advertise the machine identifier of its C++ host.
- `ObjectClient::disconnected`: waits until the session to the remote object is closed.
- Tests against the real `qilaunch` (`interop/qilaunch/build.sh`, libqi 4.0.5;
  `interop/libqi-2.9`, the libqi of NAOqi 2.9), the `qi-launch` of libqi 2.1
  (`interop/cpp21/build-module.sh`), and programs loading libraries the way NAOqi does,
  with a DCM loop over a stub of the NAOqi C++ SDK (`interop/naoqi-autoload`).

### Fixed

- `interop/cpp21/libqi-2.1/Dockerfile` installs `git`, which `build-in-container.sh` uses.

## [1.0.0] - 2026-10-12

NAOqi 2.1 robots, characterized against a NAO V4 running NAOqi 2.1.4.13.

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
- `Address::resolve_all` and `InitializingNode::connect_to_space_at`: a host name resolves to
  every address it has, tried in order until one answers, as `libqi` does. A NAO announcing
  itself as `<robot_name>.local` resolves to an IPv6 address first, which its NAOqi does not
  listen on.
- `qi-cli info` takes patterns (`'AL*'`, `'*'`) and, with `--behaviors` (`-b`), shows the
  services NAOqi registers for the boxes of running Choregraphe behaviors.
- `qi-cli --token-stdin` reads the authentication token from standard input.

### Changed

- `qi-cli info` without service names lists the services, as `qicli` does; hidden services
  (an underscore first) are left out unless `--hidden` is given, and the Choregraphe box
  services unless `--behaviors` is.
- `qi-cli --user` asks for its token on the terminal, without echo: `--token` is removed, since
  the shell history and the process list kept the token.

### Fixed

- A NAOqi 2.1 robot is recognized whatever the order in which it sends its `Capabilities`
  message and its error reply to the authentication call: it sends them from two threads, and
  the advertisement may come last. The client waits for it briefly after an error.
- Sessions are kept by the address they connect to and reused for every service reached there,
  as `libqi`'s socket cache does: a node hosting hundreds of services (NAOqi 2.1 registers
  every running Choregraphe box as one) is reached through one connection, not one per
  service. `qi-cli info` opened 1,700 connections to a NAO at once, which brought its NAOqi
  down; it now describes 8 services at a time over one connection.
- The unknown type of `libqi` (`X` in signatures), which NAOqi 2.1 advertises for members of
  `ALMemory`, `ALMotion`, `ALVideoDevice` and `ALModularity`, reads as dynamic: their meta
  objects failed to parse.

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

[Unreleased]: https://github.com/victorpaleologue/libqi-rs-vibe/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/victorpaleologue/libqi-rs-vibe/compare/v0.1.0...v1.0.0
[0.1.0]: https://github.com/victorpaleologue/libqi-rs-vibe/releases/tag/v0.1.0
