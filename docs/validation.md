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

`contrib/arora-sdk/` holds the NAOqi HAL for Arora. Its 17 tests run against an in-crate fake
NAOqi, and its probe example was run against `naoqi-sim`: description, joint states, battery,
inertial unit and sonar keys flow in; joint targets, speech and LEDs flow out.
