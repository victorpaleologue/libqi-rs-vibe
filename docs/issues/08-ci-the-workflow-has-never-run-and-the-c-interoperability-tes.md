---
title: CI: the workflow has never run, and the C++ interoperability tests are skipped there
labels: enhancement
---

## State

`.github/workflows/ci.yml` runs fmt, clippy, `cargo test --workspace`, rustdoc with warnings denied, and `cargo check` on the declared minimum Rust version (1.90). It was written in a container without access to GitHub Actions and has not run once: expect small fixes on the first run (cache keys, the MSRV pin, the toolchain action syntax).

Two parts of the validation are not covered:

1. **Interoperability with libqi.** `qi/tests/interop_cpp.rs` spawns the programs of `interop/cpp` and skips itself when `interop/cpp/build` does not exist. Building libqi 4.0.5 takes about 15 minutes and needs Boost, OpenSSL and CMake (`interop/cpp/README.md`).
2. **naoqi_driver2 validation.** `naoqi-sim/scripts/validate-naoqi-driver2.sh` needs a ROS 2 Jazzy workspace with `naoqi_driver` built (patches in `contrib/ros-naoqi`), which does not fit a standard runner.

## Work

- Run the workflow once and fix what breaks.
- Add a job that builds libqi from source (cached by tag with `actions/cache`), runs `cargo test -p qi --test interop_cpp`, and checks that `interop/cpp/build/qi-dump-values` still produces `interop/vectors/libqi-4.0.5-values.jsonl`.
- Optionally a nightly job with a RoboStack (micromamba) environment for the ROS 2 driver run, kept out of the pull-request path.
