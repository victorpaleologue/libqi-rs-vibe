# naoqi-sim

A simulated NAOqi robot, implemented in Rust on top of the [`qi`](../qi) crate of this
workspace. It hosts a `qi` space (service directory) and serves the NAOqi services that clients
such as the ROS 2 driver [`naoqi_driver2`](https://github.com/ros-naoqi/naoqi_driver2) and NAO
hardware abstraction layers use, with the exact member names and signatures of a real robot:
`ALMemory`, `ALMotion`, `ALVideoDevice`, `ALAudioDevice`, `ALTextToSpeech`, `LogManager`, …

Both a binary (`naoqi-sim`) and a library (`qi::naoqi_sim::Simulator`, to start a robot in-process
for tests) are provided.

## Running

```text
naoqi-sim [--robot nao|pepper] [--version M.m.p.b] [--name NAME]
          [--listen URL]... [--password PW] [--script FILE] [-v]...
```

| Option | Default | Meaning |
|---|---|---|
| `--robot` | `nao` | The robot model: `nao` (V6) or `pepper` (1.8). |
| `--version` | `2.8.7.4` (NAO), `2.9.5.1` (Pepper) | The NAOqi version `ALSystem.systemVersion` reports. `naoqi_driver2` selects code paths on `< 2.8` and `< 2.9`. |
| `--name` | `naoqi-sim` | The robot name (`ALSystem.robotName`). |
| `--listen` | `tcp://0.0.0.0:9559` (+ `tcps://0.0.0.0:9503` with a password) | An address to listen on: `tcp://` or `tcps://` (TLS). Repeatable. |
| `--password` | none | Enables authentication of the `nao` user with this password (`auth_user` / `auth_token` capabilities). Without it every connection is accepted. |
| `--script` | none | A scenario script run once started (`-` reads the standard input), see below. |
| `-v` | | More logs (`-vv` for the `qi` traces). |

For example, to run the ROS 2 driver against the simulator on the same machine:

```sh
cargo run -p libqi-vibe --features naoqi-sim --bin naoqi-sim -- --robot pepper
ros2 launch naoqi_driver naoqi_driver.launch.py nao_ip:=127.0.0.1 qi_listen_url:=tcp://0.0.0.0:0
```

With a password, `naoqi_driver2` switches to `tcps://<ip>:9503` (TLS) and authenticates as
`nao`; the simulator then listens on that port too, with a self-signed certificate (see the
`QI_TLS_CERTIFICATE` and `QI_TLS_PRIVATE_KEY` variables of the `qi` crate to provide one):

```sh
cargo run -p libqi-vibe --features naoqi-sim --bin naoqi-sim -- --password secret
ros2 launch naoqi_driver naoqi_driver.launch.py nao_ip:=127.0.0.1 password:=secret
```

### Scenario scripts

Scripts trigger events on a running simulator, one command per line:

```text
# Press then release the front head sensor, drain the battery, speak.
sleep 500
raise FrontTactilTouched 1.0
sleep 200
raise FrontTactilTouched 0.0
insert BatteryChargeChanged 42
say "Hello from the script"
log something happened
```

`sleep <ms>`, `raise <key> <json>` (also spelled `insert`), `say <text>`, `log <text>`; `#`
starts a comment. JSON numbers become 32-bit integers when integral and 32-bit floats otherwise.

## In-process use

```rust,no_run
use qi::naoqi_sim::{Config, RobotModel, Simulator};
use qi::ObjectExt;

# async fn example() -> qi::Result<()> {
let simulator = Simulator::start(Config::new(RobotModel::Nao).on_loopback()).await?;
let client = qi::node::init()
    .connect_to_space(simulator.address().expect("an endpoint"), None)
    .start()
    .await?;
let tts = client.service("ALTextToSpeech").await?;
let () = tts.call("say", "Hello".to_owned()).await?;
assert_eq!(simulator.spoken(), ["Hello"]);
// Trigger a touch event that `ALMemory.subscriber("FrontTactilTouched")` clients receive.
simulator.touch("FrontTactilTouched", true);
# Ok(()) }
# let _ = example;
```

The `Simulator` exposes its `Memory` (read, write and subscribe to keys), its `Body` (joints and
odometry), its `LogHub` and the states of its services (`services().tts`, `.audio`, `.video`,
`.leds`, …).

## What is simulated

| Service | Members | Behavior |
|---|---|---|
| `ALMemory` | `getData(s)->m`, `getListData(m)->[m]`, `insertData(s,m)`, `insertListData(m)`, `raiseEvent(s,m)`, `raiseMicroEvent(s,m)`, `declareEvent(s)`, `removeData(s)`, `getDataList(s)->[s]`, `getEventList()`, `getType(s)->s`, `subscriber(s)->o` (object with signal `signal(m)`), `subscribeToEvent(sss)` / `unsubscribeToEvent(ss)` (legacy module callbacks), `version()`. | Pre-populated with every key `naoqi_driver2` reads (identification, IMU, joints, battery, sonars, Pepper laser, touch events) plus speech, recognition and life keys. Writing a key notifies its subscribers. Missing keys are errors (`ALMemory::getData ... key not found`). |
| `ALMotion` | `getRobotConfig()->[[m]]`, `getSensorNames()`, `getBodyNames(s)`, `getJointNames(s)`, `getLimits(s)->[[m]]`, `getAngles(m,b)->[f]`, `setAngles(m,m,f)`, `changeAngles(m,m,f)`, `angleInterpolation(m,m,m,b)`, `angleInterpolationWithSpeed(m,m,f)`, `getStiffnesses(m)`, `setStiffnesses(m,m)`, `stiffnessInterpolation(m,m,m)`, `getPosition(s,i,b)->[f]`, `getRobotPosition(b)`, `getRobotVelocity()`, `move(fff)`, `moveToward(fff)`, `moveTo(fff)`, `stopMove()`, `moveInit()`, `moveIsActive()`, `waitUntilMoveIsFinished()`, `wakeUp()`, `rest()`, `robotIsWakeUp()`, `getSummary()`, `killAll()`, protection/breathing/idle/fall-manager flags. | Full NAO (26 joints, `JointActuators` = 25) and Pepper (17 joints + 3 wheels) tables with limits. A 50 Hz tick moves the joints toward their targets at `fractionMaxSpeed × maxVelocity`, publishes `Device/SubDeviceList/<J>/{Position/Sensor,Position/Actuator,ElectricCurrent,Temperature,Hardness}` and `Motion/{Velocity,Torque}/Sensor/<J>`, and integrates the odometry (`move` sets a velocity, `moveTo` moves for a computed duration and lands exactly on the target). |
| `ALVideoDevice` | `subscribeCamera(siiii)->s`, `subscribe(siii)->s`, `unsubscribe(s)->b`, `unsubscribeAllInstances(s)`, `getImageRemote(s)->[m]`, `getDirectRawImageRemote`, `getImageLocal`, `releaseImage(s)->b`, `getSubscribers()`, `getActiveCamera()`, `setActiveCamera(i)`, `getCameraIndexes()`, `getCameraName(i)`, `isCameraOpen(i)`, `hasDepthCamera()`, `getCameraModel(i)`, `set/getResolution`, `set/getColorSpace`, `set/getFrameRate`, `setParameter(iii)`, `getParameter(ii)`, `setCameraParameter`, `getCameraParameter`. | Cameras 0 (top) and 1 (bottom); 2 (depth) and 3 (stereo) on Pepper. Handles are `<name>_<n>`. Images are synthesized (a gradient with a moving square) at the size of the resolution (kQQVGA … k720px2) and colorspace (1, 2, 3, 4 or 12 bytes per pixel; depth and infrared are 16-bit). `getImageRemote` returns the 12-element `ALValue` of NAOqi with fresh timestamps and the camera field of view. |
| `ALAudioDevice` | `setClientPreferences(siii)`, `subscribe(s)`, `unsubscribe(s)`, `getSubscribers()`, `setOutputVolume(i)`, `getOutputVolume()`, `muteAudioOut(b)`, `isAudioOutMuted()`, energy computation and mic energies, `setParameter(si)`, `sendRemoteBufferToOutput(ir)`, `flushAudioOutputs()`, microphones recording. | On `subscribe(name)`, a task looks the service `name` up in the space and calls its `processRemote(nbOfChannels, nbOfSamplesByChannel, timestamp, buffer)` every buffer with 16-bit PCM sines: 4 channels × 8192 samples at 48 kHz by default (16 kHz and single channels honored, interleaved or not). It gives up with a warning after 30 consecutive failures. |
| `ALTextToSpeech` | `say(s)`, `sayToFile(ss)`, `stopAll()`, `setLanguage(s)`, `getLanguage()`, `getAvailableLanguages()`, `setVolume(f)`, `getVolume()`, `setParameter(sf)`, `getParameter(s)`, `setVoice(s)`, `getVoice()`, `_history()`. | `say` records the utterance, logs it, sets `ALTextToSpeech/CurrentSentence` (kept after the speech) and raises `ALTextToSpeech/TextStarted`, `TextDone` and `Status`, taking a duration proportional to the text. |
| `ALDialog` | `setLanguage(s)`, `getLanguage()`, `loadTopicContent(s)->s`, `loadTopic(s)`, `unloadTopic(s)`, `activateTopic(s)`, `deactivateTopic(s)`, `subscribe(s)`, `unsubscribe(s)`, `setFocus(s)`, `getActivatedTopics()`, `getAllLoadedTopics()`, `getLoadedTopics(s)`, `forceInput(s)`, `_recognize(s)`. | Parses the `topic:`, `language:` and `u:(...) $var=$1` lines of QiChat topics; `forceInput` raises the variables of the matching rules with the input (the `listen` action of the driver works end to end). |
| `ALSpeechRecognition` | `setVocabulary([s]b)`, `subscribe(s)`, `unsubscribe(s)`, `pause(b)`, `setLanguage(s)`, `getLanguage()`, `getAvailableLanguages()`, `setAudioExpression(b)`, `setVisualExpression(b)`, `isRunning()`, `_enableFreeSpeechToText()`, `_disableFreeSpeechToText()`, `_recognize(sf)`. | `_recognize` raises `SpeechDetected`, `WordRecognized` and `ALSpeechRecognition/Status` like a recognition would. |
| `LogManager` | `getListener()->o`, `createListener()->o`, `log([LogMessage])`, `addProvider(o)`, `removeProvider(i)`. Listeners: signals `onLogMessage`, `onLogMessages`, `onLogMessagesWithBacklog`, property `logLevel`, methods `setLevel(i)`, `addFilter(si)`, `clearFilters()`, `setCategory(si)`, `clearAndSet({si})`. | Messages carry the `qicore` structure `(sisssIll)<LogMessage,source,level,category,location,message,id,date,systemDate>`. The simulator logs a heartbeat every 5 s and every notable action (say, subscriptions, moves, …). |
| `ALSystem` | `systemVersion()`, `systemInfo()`, `robotName()`, `setRobotName(s)`, `timezone()`, `setTimezone(s)`, `freeMemory()`, `totalMemory()`, `diskFree(b)`, `shutdown()`, `reboot()`. | Static values; shutdown and reboot only log. |
| `ALRobotModel` | `getRobotType()`, `hasLegs()`, `hasWheels()`, `hasArms()`, `hasTablet()`, `hasLaser()`, `hasDepthCamera()`, `getConfig()->s` (XML), `_getConfigMap()->{ss}`, `_getMicrophoneConfig()->i`, `getRobotVersion()`. | The NAOqi 2.9 fallback of the driver. |
| `ALBodyTemperature` | `setEnableNotifications(b)`, `areNotificationsEnabled()`, `getTemperatureDiagnosis()->m`. | Joints heat up slightly while moving and cool down at rest. |
| `ALSonar` | `subscribe(s)`, `unsubscribe(s)`, `updatePeriod(si)`, `updatePrecision(sf)`, `getSubscribersInfo()`, `getCurrentPeriod()`, `getCurrentPrecision()`, `getMyPeriod(s)`, `getMyPrecision(s)`, `getOutputNames()`, `getEventList()`, `isRunning()`. | Bookkeeping only; the sonar keys hold constant distances until a script or a client changes them. |
| `ALLeds` | `fadeRGB(sif)`, `fadeListRGB(s[i][f])`, `fade(sff)`, `setIntensity(sf)`, `getIntensity(s)`, `on(s)`, `off(s)`, `reset(s)`, `listGroups()`, `listLEDs()`, `listGroup(s)`, `createGroup(s[s])`, `randomEyes(f)`, `rasta(f)`, `rotateEyes(iff)`. | Group states published as `ALLeds/<group>/Color` (`0x00RRGGBB`) and `ALLeds/<group>/Intensity`. Fades wait for their duration (10 s at most). |
| `ALBattery` | `getBatteryCharge()->i`, `_setBatteryCharge(i)`. | Reads and writes `BatteryChargeChanged`. |
| `ALAutonomousLife` | `getState()`, `setState(s)`, `getAutonomousAbilityEnabled(s)`, `setAutonomousAbilityEnabled(sb)`, `focusedActivity()`, `stopFocus()`, `stopAll()`. | State kept in `AutonomousLife/State`, `disabled` by default. |
| `ALRobotPosture` | `goToPosture(sf)->b`, `applyPosture(sf)->b`, `stopMove()`, `getPostureList()`, `getPosture()`, `getPostureFamily()`, `getPostureFamilyList()`, `setMaxTryNumber(i)`. | Postures (`Stand`, `StandInit`, `StandZero`, `Crouch`, `Sit`, …) move the joints; `goToPosture` waits until they arrive. |

Touch sensors and bumpers are memory events (`FrontTactilTouched`, `MiddleTactilTouched`,
`RearTactilTouched`, `HandLeftBackTouched`, `HandLeftLeftTouched`, `HandLeftRightTouched`,
`HandRightBackTouched`, `HandRightLeftTouched`, `HandRightRightTouched`, `LeftBumperPressed`,
`RightBumperPressed`, `ChestButtonPressed`, and `BackBumperPressed` on Pepper) raised with `1.0`
(pressed) or `0.0`: from a script, from `Simulator::touch`, or through `ALMemory.raiseEvent`.

## What is not simulated

- No physics: joints move linearly toward their targets, the robot never falls, feet and
  wheels are not modeled; the IMU, sonar, laser and force sensors hold constant values.
- No real audio or video: images are synthetic patterns, audio buffers are sine waves, and
  nothing is heard: speech recognition and dialog inputs are triggered explicitly.
- No `ALNavigation`, `ALTabletService`, `ALBasicAwareness`,
  `ALAnimatedSpeech`, `ALPeoplePerception`, `PackageManager`, behaviors or Choregraphe boxes.
- `ALMotion` computes Cartesian positions for `Torso`, `Head` and the cameras only.

## Testing

```sh
cargo test -p libqi-vibe --features naoqi-sim --test naoqi_sim
```

`interop/driver_smoke.cpp` is a C++ client built on the real `libqi` and `libqicore` (the
`ros-naoqi` forks) that replays the calls of `naoqi_driver2` with its exact call forms and
value accessors (`asListValuePtr().content()`, `as<std::string>()`, the typed
`qi::LogListenerPtr` proxy, the `processRemote` audio callback…). Build it with
`interop/build.sh` (see the variables at its top) and run it with the simulator's URL.

The integration tests connect a `qi` client node to an in-process simulator and replay the
startup contract of `naoqi_driver2` in order (for NAO and Pepper), check the `getImageRemote`
structure, receive audio callbacks on a client-registered service, subscribe to memory events,
move joints and the base, speak, authenticate and run scripts.
