---
title: naoqi-sim: no physics, constant sensors, missing services
labels: enhancement
---

`naoqi-sim` covers what `naoqi_driver2` and the arora HAL call (16 services, listed in `qi/src/naoqi_sim/README.md`). Known limits:

- **No physics.** Joints move linearly toward their targets at a fraction of their max velocity; the robot never falls; feet, wheels and force sensors are not modeled. The IMU, sonar, laser and FSR keys in `ALMemory` hold constant values unless a script inserts others.
- **No real audio or video.** `ALVideoDevice` images are synthetic moving patterns of the right size and colorspace; `ALAudioDevice` pushes sine-wave PCM buffers; `ALSpeechRecognition` and `ALDialog` inputs are triggered explicitly (`_recognize`, `forceInput`, script `raise`).
- **Missing services.** `ALNavigation`, `ALTabletService`, `ALBasicAwareness`, `ALAnimatedSpeech`, `ALPeoplePerception`, `ALBehaviorManager`, `PackageManager`, Choregraphe boxes.
- **`ALMotion` Cartesian API** computes positions for `Torso`, `Head` and the cameras only (`getPosition`); `getTransform`, `setPositions` and `positionInterpolations` are not implemented.
- **`ALMemory` history**: `getEventHistory`, `getTimestamp` and the legacy `DataChanged` callback path are stubs.

Each item is independent. The modules under `qi/src/naoqi_sim/services/` are one file per service and follow the same pattern (`#[qi::object]` trait, shared body state, `ALMemory` keys), so adding one is mechanical.
