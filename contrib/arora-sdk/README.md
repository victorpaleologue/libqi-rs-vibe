# NAOqi HAL for arora-sdk

The HAL is proposed upstream as
[semio-ai/arora-sdk#258](https://github.com/semio-ai/arora-sdk/pull/258).
`0001-Add-arora-hal-naoqi.patch` is that pull request squashed into one commit on arora-sdk
commit `770a4a7f`, kept here so the validation in `docs/validation.md` stays reproducible.

It adds the crate `crates/arora-hal-naoqi`: NAO and Pepper robots as Arora devices through
their NAOqi middleware, using `libqi-vibe` (this repository, used as `qi`) instead of the
C++ SDK. It replaces the role of the `modules/nao` stub (a cross-compiled C++ module that
only said hello), following the recommendation of the port study: the runtime expects
hardware behind the `Hal` trait, not behind a module.

```sh
cd arora-sdk
git am path/to/0001-Add-arora-hal-naoqi.patch
cargo test -p arora-hal-naoqi
cargo run -p arora-hal-naoqi --features runner --bin arora-naoqi -- tcp://nao.local:9559
```

The patch depends on `libqi-vibe` through a git dependency pinned to a commit of this
repository; once `libqi-vibe` is on crates.io it becomes
`qi = { package = "libqi-vibe", version = "0.1" }`.

## What the HAL does

- Sensed keys: `<joint_id>.position|velocity|stiffness|temperature|current`,
  `battery.charge|current|charging`, `imu.angle.{x,y}`, `imu.gyroscope.{x,y,z}`,
  `imu.accelerometer.{x,y,z}`, `sonar.{left,right}.distance` (sampled from `ALMemory` in one
  `getListData` call per period, 50 ms by default), `touch.head.*`, `touch.hand.*`,
  `bumper.*`, `chest_button` (driven by `ALMemory` events through `subscriber().signal`).
- Commanded keys: `<joint_id>.target_position` (`ALMotion.setAngles`),
  `<joint_id>.target_stiffness` (`setStiffnesses`), `text` (`ALTextToSpeech.say`, unset once
  spoken), `led.<group>.color|intensity` (`ALLeds`), `velocity.{x,y}` and `rotation.z`
  (`ALMotion.moveToward`).
- Joint ids are the NAOqi names in snake case (`LShoulderPitch` -> `l_shoulder_pitch`),
  overridable in the configuration.
- Tested against an in-process fake NAOqi (in the crate's tests) and against `naoqi-sim`.

## Questions for arbitration

1. Crate name and placement: `arora-hal-naoqi` under `crates/` (transport naming like
   `-ros2` and `-restful`; the plan document reserved `arora-hal-nao`).
2. Fate of `modules/nao`: the patch leaves it in place; it can be deleted along with the
   Homebrew i686 toolchain configuration once the HAL is adopted.
3. Key vocabulary beyond joints and speech (battery, IMU, sonar, touch, LEDs, base velocity):
   the names above follow the `entity.component` grammar and the proposal of the port study;
   nothing in the workspace defined them before.
4. Joint ids: snake case of the NAOqi names by default. The ROS 2 HAL derives ids from the
   GLB model of the robot; reusing that mapping would need the GLB parser shared between the
   two HALs (and the model download in a build script). `HalAssets::model_glb` is not
   implemented for the same reason.
5. Speech semantics: `text` is a fire-once command. The HAL unsets the key once the sentence
   is spoken, so that the same sentence can be said again despite the store's echo damping.
   The alternative is a host-module `say` function returning `Status::Running`.
6. Opening inputs: the HAL cannot set `KeyMeta`; `*.target_position`, `text`, `led.*` and the
   velocity keys must be declared editable by the runner or the store, as for the other HALs.
7. `describe()`: filled from the robot (`ALRobotModel.getRobotType`, `ALSystem.systemVersion`,
   `RobotConfig/Body/BaseVersion`) unless the configuration overrides it.
8. Sensor cadence: polling `ALMemory` at a configurable period, one `StateChange` per sample
   with only the values that changed; velocities derived from consecutive samples (NAOqi's
   `Motion/Velocity/Sensor/*` keys are not available on every robot).
9. Authentication: NAOqi 2.5+ user/token credentials are supported (`user`, `password` in the
   configuration); TLS endpoints (`tcps://`, NAOqi 2.9) are supported by `qi`.
10. Logging: the HAL uses the `log` facade like the other HALs; `qi` emits `tracing` events,
    which the runner can bridge with `tracing-subscriber` if wanted.
11. Publishing: like the other concrete HALs, the crate is not in the release list.
