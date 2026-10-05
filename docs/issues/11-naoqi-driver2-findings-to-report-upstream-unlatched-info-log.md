---
title: naoqi_driver2 findings to report upstream: unlatched /info, LogMessage timestamp
labels: documentation
---

Two driver-side issues surfaced while validating `naoqi_driver2` against `naoqi-sim` (`docs/validation.md`). Nothing to fix here; they should be reported to `ros-naoqi/naoqi_driver2` and this issue closed with the links.

1. **`/info` is published once, unlatched.** The `info` converter is registered with frequency 0 in `src/naoqi_driver.cpp` and published a single time at startup on a default publisher (`src/publishers/basic.hpp`). The comment assumes ROS 1 latching; in ROS 2 the publisher needs `rclcpp::QoS(1).transient_local()` for a late subscriber to receive it. This is the one failing check of the validation (`summary: 13 passed, 1 failed`).

2. **`/rosout` stamps from the robot are always 0.** The log converter reads `msg.timestamp.tv_sec` on `qi::LogMessage`, but `timestamp` is not part of the struct registered with `QI_TYPE_STRUCT` in `naoqi_libqicore` (fields end at `date, systemDate`), so it is never deserialized from the wire. The driver should use `date` (microseconds since the epoch) instead.

Reproduction for both: build the driver in a RoboStack Jazzy environment with the patches of `contrib/ros-naoqi`, then run `naoqi-sim/scripts/validate-naoqi-driver2.sh`.
