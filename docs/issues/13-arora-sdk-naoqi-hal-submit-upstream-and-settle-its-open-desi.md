---
title: arora-sdk NAOqi HAL: submit upstream and settle its open design questions
labels: documentation
---

`contrib/arora-sdk/0001-Add-arora-hal-naoqi.patch` adds `crates/arora-hal-naoqi` to `semio-ai/arora-sdk` (on top of its commit `f2700c7`): NAO and Pepper as Arora devices through NAOqi, using `qi` instead of the C++ SDK and replacing the role of the `modules/nao` stub. It has 17 tests against an in-crate fake NAOqi and a probe example run against `naoqi-sim`. It lives here as a patch only because it belongs to another repository.

To do:

- Open the pull request on arora-sdk once `qi` is at a revision that can be depended on (the patch uses a git dependency plus a `[patch]` section pointing to a sibling checkout; that section must go before merging).
- Settle the eleven questions in `contrib/arora-sdk/README.md`. Those that change the code: the key vocabulary for sensors beyond joints and speech (battery, IMU, sonar, touch, LEDs, base velocity); joint ids (snake case of the NAOqi names vs. ids derived from the GLB model like the ROS 2 HAL); the fire-once semantics of `text` (the HAL unsets the key after speaking so the same sentence can be said twice despite the store's echo damping; the alternative is a host-module `say` function returning `Status::Running`).
- Decide the fate of `modules/nao` and the Homebrew i686 toolchain configuration once the HAL is in.
- Drop the HAL's local `ALValue` wrapper once `qi::value` provides a first-class dynamic type.
