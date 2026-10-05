---
title: Interop harness: libqi must be built from source by hand, and its absence is silent
labels: enhancement
---

The C++ programs under `interop/cpp` (`qi-dump-values`, `qi-cpp-client`, `qi-cpp-service`, `qi-cpp-sd`) prove byte identity and cross-process interoperability. Building them requires a libqi 4.0.5 install produced by hand following `interop/cpp/README.md` (CMake, Boost, OpenSSL, about 15 minutes). `qi/tests/interop_cpp.rs` silently skips when `interop/cpp/build` is missing, so a fresh clone passes `cargo test` without exercising any of this.

Improvements, in order of value:

1. A script (`interop/cpp/build.sh`) that clones libqi at the pinned tag, builds and installs it under `interop/cpp/libqi-install`, then builds the harness, so the whole thing is one command.
2. Make the skip loud: warn with the script's name when the harness is missing, and add `QI_INTEROP_REQUIRED=1` to turn the skip into a failure for CI.
3. Cache the libqi build in CI (see the CI issue).
4. Pin the libqi version in one place (currently the tag appears in the README, in `interop/vectors/libqi-4.0.5-values.jsonl` and in test names).
