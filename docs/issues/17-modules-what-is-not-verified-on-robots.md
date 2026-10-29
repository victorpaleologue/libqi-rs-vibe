---
title: Modules: what is not verified on robots yet
labels: documentation
---

The modules of `docs/modules.md` are tested against libqi 4.0.5 and 2.9 (the real `qilaunch`),
libqi 2.1 (its `qi-launch`), and programs loading the libraries the way NAOqi does, the 32-bit library
included on a glibc 2.13 system. Not verified:

1. **NAOqi itself.** That NAOqi 2.1 and 2.9 load the libraries of `autoload.ini` through
   `_createModule(boost::shared_ptr<AL::ALBroker>)` comes from the C++ DCM plugins (mc_naoqi_dcm
   for 2.1.4 and 2.5, rUNSWift's libagent up to 2.8). NAOqi 2.9 may differ.
2. **The DCM bridge against the SDK.** `qi/cpp/naoqi_shim.cpp` is built against a stub of the
   NAOqi C++ SDK (`interop/naoqi-sdk-stub`) mimicking the API those plugins use; it has not been
   compiled against a real SDK (the cross toolchains are only available from mirrors such as the
   one mc_naoqi_dcm documents). The types it does not name (`ProcessSignalConnection`, the return
   type of `ALProxy::getModule`) are deduced with `BOOST_TYPEOF`.
3. **The libqi shim against the libqi of a robot**: it is tested against libqi 2.1, 4.0.5 and the
   `release-2.9` branch of libqi (NAOqi 2.9) built from source, with GCC or Clang; the libqi of a
   robot is built by its toolchain, whose compiler and Boost the shim must be built with.
4. **The kernel of NAOqi 2.1 robots** (`uname -r`), which may be older than the 3.2 the Rust
   standard library supports: untested.
5. **Real-time behavior** of DCM plugins: the callbacks are synchronous Rust code on the thread of
   the DCM; their duration is the responsibility of the plugin.
