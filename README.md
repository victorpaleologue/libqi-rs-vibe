# libqi-rs-vibe

[![CI](https://github.com/victorpaleologue/libqi-rs-vibe/actions/workflows/ci.yml/badge.svg)](https://github.com/victorpaleologue/libqi-rs-vibe/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/libqi.svg)](https://crates.io/crates/libqi)
[![docs.rs](https://img.shields.io/docsrs/libqi)](https://docs.rs/libqi)
[![License: BSD-3-Clause](https://img.shields.io/badge/license-BSD--3--Clause-blue.svg)](LICENSE)

A Rust implementation of the `qi` framework, the middleware of Aldebaran's NAO and Pepper
robots (`libqi`, also known as qimessaging), byte-compatible with the C++ implementation.

With it, Rust programs talk to NAOqi robots and to any `libqi` process: they call
services, subscribe to signals, read and write properties, expose their own services and
objects, or even host a service directory. The wire format is verified byte for byte
against `libqi` 4.0.5, and the implementation is exercised against C++ `libqi` processes
as client, service and service directory.

This is a fork of [libqi-rs](https://github.com/nyibbang/libqi-rs) by Vincent Palancher,
completed into a full stack.

## Installing

```toml
[dependencies]
qi = { package = "libqi-vibe", version = "0.1" }   # used as `qi` in code
```

The package is `libqi-vibe` on crates.io (the name `qi` is taken); the library itself is
named `qi`, so `use qi::...` works as in the examples. Rust 1.90 or later is required.

The command-line tools are features of the same crate:
`cargo install libqi-vibe --features cli --bin qi-cli` and
`cargo install libqi-vibe --features naoqi-sim --bin naoqi-sim`; prebuilt binaries for Linux, macOS and Windows are attached to
the [releases](https://github.com/victorpaleologue/libqi-rs-vibe/releases).

## Quick start

```rust
use qi::ObjectExt;

#[tokio::main]
async fn main() -> qi::Result<()> {
    let node = qi::node::init()
        .connect_to_space("tcp://nao.local:9559".parse().expect("valid address"), None)
        .start()
        .await?;
    let tts = node.service("ALTextToSpeech").await?;
    let () = tts.call("say", "Hello from Rust".to_owned()).await?;
    Ok(())
}
```

A typed interface is a trait:

```rust
#[qi::object(case = "camelCase")]
trait TextToSpeech {
    async fn say(&self, text: String) -> qi::Result<()>;
    async fn get_volume(&self) -> qi::Result<f32>;   // `getVolume` on the wire
}
```

The macro produces a typed client for remote objects and an adapter that exposes an
implementation of the trait as a service. Typed interfaces, services, signals, properties
and object passing are shown in the documentation of the `qi` crate and in
[`qi/examples/`](qi/examples/).

## Crates

Two crates, released together:

| Crate | Description |
|---|---|
| [`libqi-vibe`](qi/) (used as `qi`) | The framework. Modules: `qi::value` (type system, signatures, dynamic values), `qi::format` (binary format, as a `serde` data format), `qi::messaging` (messages, channels, client and server loops), nodes, sessions, objects, signals, properties, services and the service directory. Feature `cli`: the `qi-cli` command. Feature `naoqi-sim`: the `qi::naoqi_sim` module and the `naoqi-sim` command, a simulated NAOqi robot. |
| [`libqi-macros-vibe`](qi-macros/) | Procedural macros (`#[qi::object]`, the value derives), re-exported by `libqi-vibe`. |

The design is described in [`docs/architecture.md`](docs/architecture.md), and how the
implementation was validated against `libqi`, the ROS 2 driver of the robots and the Arora
runtime in [`docs/validation.md`](docs/validation.md).

## Status

Implemented and tested against `libqi` 4.0.5:

- binary format and messaging protocol, byte-identical (66 reference fixtures);
- sessions with capability negotiation and user/token authentication;
- TLS transports (`tcps://`, as used by NAOqi 2.9 robots) with `libqi`'s semantics;
- calls with cancellation, posts, signals (`registerEvent`/`unregisterEvent`), properties;
- objects passed in both directions, with the special bound-object actions;
- services and a standalone service directory with its signals and relative endpoints;
- callbacks in both NAOqi styles: object passing, and services calling back services
  registered by their clients.

Not implemented yet: mutual TLS authentication (`tcpsm://`), the `Manageable` statistics
and tracing members, and gateways. Known gaps and follow-ups are listed in
[`docs/issues/`](docs/issues/).

## Tools

`qi-cli` inspects and drives any `qi` space, a robot included:

```sh
qi-cli --url tcp://nao.local:9559 info ALTextToSpeech
qi-cli --url tcp://nao.local:9559 call ALTextToSpeech.say "Hello"
qi-cli --url tcp://nao.local:9559 call ALMemory.getData Device/SubDeviceList/Battery/Charge/Sensor/Value
```

`naoqi-sim` runs a simulated NAO or Pepper that `libqi` clients (the ROS 2 `naoqi_driver2`
included) connect to as to a real robot:

```sh
naoqi-sim --robot nao --listen tcp://0.0.0.0:9559
```

From a checkout, prefix the commands with
`cargo run -p libqi-vibe --features cli --bin qi-cli --` and
`cargo run -p libqi-vibe --features naoqi-sim --bin naoqi-sim --`.

## Building and testing

```sh
cargo build --workspace
cargo test --workspace
```

The interoperability tests against C++ `libqi` (`qi/tests/interop_cpp.rs`) run when the
harness in [`interop/cpp/`](interop/cpp/) is built (see its README); they are skipped
otherwise, and `QI_INTEROP_REQUIRE=1` makes skipping an error.

Releases are described in [`CHANGELOG.md`](CHANGELOG.md) and made as explained in
[`docs/releasing.md`](docs/releasing.md).

## Authors

- Victor Paleologue, this fork and its completion.
- Vincent Palancher, the original [libqi-rs](https://github.com/nyibbang/libqi-rs).

## License

BSD 3-Clause, see [`LICENSE`](LICENSE). The original work is copyright Aldebaran United
Robotics Group; the fork is copyright Victor Paleologue.
