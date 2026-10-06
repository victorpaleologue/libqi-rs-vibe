# Validation

How the implementation was checked against the reference implementation and against
real users of NAOqi. Everything below ran on the same machine, over TCP loopback, with
`libqi` 4.0.5 built from source (`interop/cpp/README.md` explains how to rebuild the harness).

## Byte identity with libqi 4.0.5

`interop/cpp/build/qi-dump-values` serializes 66 values and messages with `libqi` (primitives,
strings, containers, structures with annotations, dynamics, optionals, raw buffers, meta
objects, service infos, capability maps, and every message type: authentication, calls,
replies, errors, cancel, canceled, events, posts, capabilities). The output is committed as
`interop/vectors/libqi-4.0.5-values.jsonl`, and `qi/tests/format_libqi_vectors.rs` and
`qi/tests/libqi_vectors.rs` check that the Rust crates produce exactly those bytes and decode
them back, through static Rust types and through the dynamic `Value` representation.

Two deviations are known and pinned in the tests: an empty (invalid) dynamic value decodes
as a unit and re-encodes as `v`, and a structure inside a dynamic value loses its
`<Name,fields>` annotation when it goes through `Value` (statically typed structures keep it).

## Interoperability with libqi processes

`qi/tests/interop_cpp.rs` spawns the C++ programs of `interop/cpp` and runs, over TCP and
over TLS:

| Service directory | Service | Client | Result |
|---|---|---|---|
| Rust | Rust | C++ (`qi-cpp-client`, 30 scenarios) | all pass, TCP and `tcps://` |
| C++ | C++ | Rust | all scenarios pass |
| C++ | Rust | C++ | all pass |
| Rust | C++ | Rust | all scenarios pass |

The scenarios cover calls with every kind of value, dynamics, errors, cancellation (the
C++ client observes `Type_Canceled` from the Rust service), signals, properties, objects
returned by methods, objects passed as arguments with the service calling back and
subscribing to a signal of the client's object, and large payloads.

## naoqi_driver2 (ROS 2 Jazzy) against naoqi-sim

The ROS 2 driver of the robots (`ros-naoqi/naoqi_driver2`, with its `naoqi_libqi` 3.0 and
`naoqi_libqicore` dependencies) was built from source in a RoboStack Jazzy environment
(Boost 1.90 required patches to `naoqi_libqi`, kept outside this repository) and run against
`naoqi-sim`:

```sh
cargo build -p libqi-vibe --features cli,naoqi-sim
naoqi-sim/scripts/validate-naoqi-driver2.sh    # needs a sourced ROS 2 workspace with naoqi_driver
```

The driver connects, authenticates, discovers the robot (`ALMemory`, `ALSystem`, `ALMotion`,
`ALRobotModel`), subscribes to the cameras, registers its `ROS-Driver-Audio` service and
subscribes to `ALAudioDevice`, connects to the touch events and starts its loop. Result of
the checks (`summary: 13 passed, 1 failed`):

| Check | Result |
|---|---|
| `/joint_states`, `/imu/torso`, `/sonar/*`, `/diagnostics`, `/odom`, `/tf` | received |
| `/camera/front/image_raw`, `/camera/bottom/image_raw` | received (synthesized images) |
| `/audio` | received: the simulated `ALAudioDevice` calls `processRemote` on the driver's service |
| `/bumper` after `ALMemory.raiseEvent LeftBumperPressed` | received |
| `/speech` → `ALTextToSpeech.say` | the simulator said the sentence |
| `/joint_angles` → `ALMotion.setAngles` | `getAngles` reads the commanded angle back |
| `/cmd_vel` → `ALMotion.move` | `getRobotVelocity` reports the commanded velocity |
| `/info` | not received: the driver publishes it once at startup on a non-latched publisher (a ROS 1 assumption in the driver) |

`naoqi-sim/interop/driver_smoke.cpp` additionally replays the driver's exact call forms with
the real `libqi`/`libqicore` (typed `LogManager` proxies, `getImageRemote` structure checks,
audio callbacks, touch subscribers, dialog flow): 22 steps pass.

With a password, the driver switches to `tcps://<robot>:9503`: `naoqi-sim --password`
listens there too, over TLS.

## arora-sdk

The NAOqi HAL for Arora is semio-ai/arora-sdk#258. At its
commit `b5160244`, its 23 tests run against an in-crate fake NAOqi, one of them the whole
device (HAL, behavior leaves, default tree) on the Arora runtime; its runner cross-builds
for the NAO (`i686-unknown-linux-musl`). Its probe example was run against `naoqi-sim`:
description, joint states, battery, inertial unit and sonar keys flow in; joint targets,
speech and LEDs flow out.

## NAOqi 2.1 (the libqi of 2014)

NAOqi 2.1 robots run the `libqi` of 2014, whose protocol has no authentication, fewer
capabilities and six-field service infos. That stack (core `v2.1.3`, `libqitype` and
`libqimessaging` at the commits of June 2014 that fed the 2.1 release) was built in an Ubuntu
14.04 container, and the harness was ported to its API: [`interop/cpp21/`](../interop/cpp21/README.md)
describes the build, the port and every protocol difference found. In short, every shared value
is byte-identical with 4.0.5 (`interop/vectors/libqi-2.1-values.jsonl`, compared with the 4.0.5
fixture by `qi/tests/libqi21_vectors.rs`); what differs is the handshake, the capability set,
`ServiceInfo` (no `objectUid`), and what the type system lacks (optionals, cancellation, object
UIDs).

The Rust side detects the protocol of each peer when the session is established
(`qi::Protocol`): a `Capabilities` message followed by an error reply to the authentication
call means a legacy server; a first request that is not the authentication call means a legacy
client. `qi/tests/interop_cpp21.rs` then runs, over TCP loopback:

| Service directory | Service | Client | Result |
|---|---|---|---|
| Rust | Rust | 2.1 (`qi-cpp-client`, 28 scenarios) | all pass |
| Rust emulating 2.1 (`Protocol::Legacy`) | Rust | 2.1 | all pass |
| 2.1 | 2.1 | Rust | all scenarios pass (minus optionals and cancellation, plus the ALMemory subscriber pattern) |
| 2.1 | Rust | 2.1 | all pass |
| Rust | 2.1 | Rust | all scenarios pass |
| `naoqi-sim --version 2.1.4.13` | | 2.1 (`qi-cpp-naoqi-probe`) | `systemVersion`, `getData`, `subscriber` + `raiseEvent` events, `say`, `getAngles` pass |

and `interop_cpp.rs` runs the 4.0.5 client against the Rust emulation of a 2.1 server (its own
compatibility code for old servers): all its scenarios but cancellation and optionals pass.

Three findings drove the implementation: a 2.1 process cannot convert a seven-field
`ServiceInfo` (the service directory advertises and writes the six-field form to peers without
the `ObjectPtrUID` capability, and reads both); a 2.1 process fails to fetch a meta object with
an optional or variadic signature (`Invalid signature`), so those members are hidden from legacy
peers; and when a client forces a return signature the value cannot convert to, the reply is
sent with the declared type like `libqi` does, rather than as a dynamic that would lose the
structure annotations the client needs.

Not reproduced here: a real NAOqi 2.1 (`naoqi-bin` and its C++ modules such as `ALMemory`),
whose SDK is no longer downloadable; the 2.1 `ALMemory.subscriber` pattern is exercised with an
equivalent service of the harness.
