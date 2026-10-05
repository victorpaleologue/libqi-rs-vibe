---
title: Validation gaps: no real robot, no second machine, driver password mode not exercised end to end
labels: documentation
---

Everything in `docs/validation.md` ran on one machine over loopback, against libqi 4.0.5 built from source and against `naoqi-sim`. Not checked:

- **A real NAO or Pepper.** No robot was reachable. NAOqi 2.1, 2.5, 2.8 and 2.9 differ in capability negotiation (`MetaObjectCache`, `RemoteCancelableCalls`, `ObjectPtrUID`), in authentication (2.5+ requires user/token, 2.9 runs the gateway on `tcps://:9503`) and in message-size limits. The code follows libqi 4.0.5 and the ROS driver's observed behavior, not a robot.
- **Two machines.** Endpoint selection when a service advertises several addresses (loopback, LAN, docker bridge) is tested with loopback only; libqi's preference order for reachable endpoints is untested against a remote host.
- **naoqi_driver2 password mode.** `naoqi-sim --password` opens `tcps://:9503` and the driver switches to it when given a password, but `naoqi-sim/scripts/validate-naoqi-driver2.sh` runs the driver without one. TLS itself was validated with the libqi 4.0.5 C++ client (`cpp_client_over_tls_against_rust_node`), not with the driver's libqi 3.0 fork.
- **libqi 3.0 (the ros-naoqi fork) as a service.** `naoqi-sim/interop/driver_smoke.cpp` links the fork as a client of a Rust service; a Rust client against a libqi 3.0 service was not run.
- **Pepper.** Smoke-tested with the driver's call forms only; the full ROS 2 validation ran with `ROBOT=nao`.

## Work

- Run `qi-cli info` and the arora probe against a real robot of each available NAOqi generation; file whatever differs.
- Run the driver validation with `ROBOT=pepper` and with a password.
- A two-host run (or two network namespaces) of the `qi/tests/space.rs` scenarios.
