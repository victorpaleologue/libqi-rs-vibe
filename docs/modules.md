# Modules written in Rust

A `qi` *module* is a shared library exporting functions and object factories under a name, which
the processes of a space load: `qilaunch -o module.Factory` registers the object a factory creates
as a service, `qilaunch -f module.function` calls a function, and NAOqi loads the libraries listed
in its `autoload.ini`, DCM plugins included. With the feature `module` of the crate, a Rust crate
built as a `cdylib` is such a module.

The example module of the repository, [`examples/rust-module`](../examples/rust-module/), is the
module `rustexample`: functions (`hello`, `add`, `fail`, `say`, `services`), factories
(`RustGreeter`, `RustDcm`, `OtherGreeter`) and a DCM plugin (`RustEyes`).

## Which host loads what

| Host | Entry point of the library | What it does | Needs |
|---|---|---|---|
| NAOqi 2.1 to 2.9, `autoload.ini` (`[user]` section) | `_createModule(boost::shared_ptr<AL::ALBroker>)`, `_closeModule()` | registers the objects of the `autoload` factories as services of NAOqi; creates the DCM plugins | nothing for the services; the C++ SDK of NAOqi for DCM plugins (feature `module-naoqi`) |
| `qilaunch` (NAOqi 2.3 to 2.9, libqi 2.3 and later), Python's `qi.module` | `qi_module_init(qi::ModuleBuilder*)` | `-o module.Factory`, `-f 'module.function(args)'` | the libqi SDK of the robot (feature `module-cpp`) |
| `qi-launch` of NAOqi 2.1 (`Session::loadService(library)`) | factories registered when the library is loaded (`QI_REGISTER_OBJECT_FACTORY`) | registers the objects of every factory as services | the libqi 2.1 SDK (feature `module-cpp`) |
| `qi-launch` of this crate (feature `launch`) | `qi_rs_module_v1` | the options of `qilaunch`, plus the positional libraries and `-a` of the 2.1 `qi-launch` | nothing |

The entry points are all in the same library: a library built with the features it needs serves
every host above.

NAOqi 2.1 (2014) predates the module system of libqi (`qi::import`, `QI_REGISTER_MODULE`) and
`qilaunch`, which appeared in libqi 2.3 and libqicore in 2015. Its launcher is the `qi-launch` of
libqimessaging, which loads libraries whose factories register at load time. The libraries NAOqi
loads from its `autoload.ini` are `ALModule`s of libalcommon, created by `_createModule` (the C++
DCM plugins, such as the `fastgetsetdcm` example of the SDK, mc_naoqi_dcm or the libagent of
rUNSWift, are such libraries) on NAOqi 2.1 to 2.8.

## Writing a module

```toml
[lib]
name = "greetings"              # the library is libgreetings.so: the name of the module
crate-type = ["cdylib"]

[dependencies]
qi = { package = "libqi-vibe", version = "1", features = ["module"] }
```

```rust
#[qi::module]
pub mod greetings {
    use qi::module::Session;

    /// Returns a greeting: `qilaunch -n demo -f 'greetings.hello("Nao")'`.
    pub fn hello(name: String) -> String {
        format!("Hello, {name}!")
    }

    /// Says a text, with the `ALTextToSpeech` of the space of the host.
    pub async fn say(session: Session, text: String) -> qi::Result<()> {
        use qi::ObjectExt;
        session.service("ALTextToSpeech").await?.call("say", text).await
    }

    /// A service: `qilaunch -n demo -o greetings.Greeter` registers it as `Greeter`, and so does
    /// NAOqi when it loads the library (`autoload`).
    #[qi::factory(autoload)]
    pub fn greeter(session: Session) -> qi::Result<super::GreeterObject<super::MyGreeter>> {
        Ok(super::GreeterObject::new(super::MyGreeter::new(session)))
    }
}
```

- Every `pub fn` of the module is a function, named after its identifier (`case = "camelCase"` on
  `#[qi::module]` converts the names; `#[qi::function(name = "...")]` sets one).
- `#[qi::factory]` marks a function returning an object: anything implementing `qi::Object`, such
  as the adapters `#[qi::object]` generates for typed interfaces. Its name is the `PascalCase` of
  its identifier unless `name = "..."` is given. `autoload` registers it when NAOqi loads the
  library.
- Members are synchronous or `async`, return a `Result<T>` (whose error converts into
  `qi::Error`) or a plain value; their parameters and results are values of the type system.
- A first parameter of type `qi::module::Session` is a handle to the space of the host: a node
  connected on first use to the URL the host gives (`qilaunch` and `qi-launch` give theirs; libqi
  passes its session to the members that accept one), else `QI_URL`, else the local NAOqi
  (`tcp://127.0.0.1:9559`). Its services listen on `tcp://127.0.0.1:0`, or `tcp://0.0.0.0:0` when
  NAOqi loaded the library (like the services of NAOqi, they are reachable from other machines);
  `QI_MODULE_LISTEN_URL` overrides both.
- The name of the module is the name of the `mod` unless `#[qi::module(name = "...")]` sets it. It
  must be the name of the library (`lib<name>.so`, `lib/a/libb.so` for a module `a.b`) and of its
  declaration (`share/qi/module/<name>.mod`). One module per library.
- The logs of the library go to its standard error, filtered by `QI_MODULE_LOG` (the syntax of
  `RUST_LOG`; default: warnings, and the information of the module system).

## DCM plugins

A DCM plugin is code running in the loop of the DCM of NAOqi (every 10 ms on NAO, 12 ms on
Pepper): right after the DCM updated the values of the sensors in ALMemory (`post_process`), and
just before it computes the orders it sends to the boards (`pre_process`), where commands are
applied with the shortest delay. It reads sensor values in place in the memory of ALMemory and
sends commands with DCM aliases, without network nor serialization.

```rust
use qi::module::naoqi::{Alias, Cycle, DcmPlugin, MemoryF32, Naoqi, Update};

struct Follow { yaw: MemoryF32, eyes: Alias }

impl DcmPlugin for Follow {
    fn pre_process(&mut self, cycle: &mut Cycle<'_>) {
        let intensity = (self.yaw.get().abs() / 2.0).min(1.0);
        if let Ok(time) = cycle.time(0) {
            let _ = cycle.set_alias(&self.eyes, Update::ClearAll, time, &[intensity; 16]);
        }
    }
}

#[qi::module]
pub mod follow {
    /// Creates the ALModule `Follow` when NAOqi loads the library.
    #[qi::dcm(name = "Follow")]
    pub fn follow(naoqi: &qi::module::naoqi::Naoqi<'_>) -> qi::Result<super::Follow> {
        Ok(super::Follow {
            yaw: naoqi.memory_f32("Device/SubDeviceList/HeadYaw/Position/Sensor/Value")?,
            eyes: naoqi.create_alias("FollowEyes", &EYE_LED_KEYS)?,
        })
    }
}
```

The plugin is an `ALModule` of NAOqi, created in `_createModule` with the broker NAOqi gives, by a
C++ bridge (`qi/cpp/naoqi_shim.cpp`) built with the feature `module-naoqi` against the C++ SDK of
NAOqi. The callbacks run on the thread of the DCM and delay its loop: they must return within a
fraction of the period, without blocking (no network call, no lock held by slow code). Share
state with the services of the module through atomics or briefly held locks, as the `RustDcm`
service of the example does with its `RustEyes` plugin (`examples/rust-module/src/dcm.rs`). A
panicking plugin is disabled, not unwound into NAOqi.

Without the feature, NAOqi still loads the library and registers its services, and the module logs
that its DCM plugins are not available.

## Building

```sh
cargo build --release -p qi-module-example          # target/release/librustexample.so
```

### For a robot

The library must be built for the processor of the robot (`uname -m` on the robot: `i686` for the
32-bit systems of NAO V4 and V5, `x86_64` for 64-bit ones such as NAO6's), and for its C library: a library linked on a recent
system needs a recent glibc (2.34 on Ubuntu 24.04), and fails to load on the robot (`version
GLIBC_2.34 not found`). Without C++ bridge, `cargo-zigbuild` links against the glibc version given:

```sh
pip install ziglang && cargo install cargo-zigbuild
rustup target add i686-unknown-linux-gnu
cargo zigbuild --release -p qi-module-example --target i686-unknown-linux-gnu.2.13
cargo zigbuild --release -p qi-module-example --target x86_64-unknown-linux-gnu.2.17
```

`examples/rust-module/build-robot.sh` does it for both architectures, in `dist/rustexample-robot`
(with the module declaration, an `autoload.ini` and instructions). The 32-bit library was loaded,
`_createModule` included, on the glibc 2.13 of Debian 7.

glibc 2.13 is older than the robots of NAOqi 2.1 are thought to have (check with
`/lib/libc.so.6` on the robot). Rust's standard library references `getauxval`, which glibc
added in 2.16: modules link a hidden fallback (`qi/cpp/glibc_compat.c`) calling the one of the C
library when it has one, so that they load on older ones. `objdump -T librustexample.so | grep
-o 'GLIBC_[0-9.]*' | sort -uV | tail -1` gives the newest version the library needs.

### With the C++ bridges

The bridges are C++ code using the C++ API of the robot (libqi for `module-cpp`, libalcommon for
`module-naoqi`): they must be compiled against the SDK of the NAOqi version of the robot, with its
compiler (the C++ ABI, `std::string` included, and the Boost of the robot must match). Use the
cross toolchain of the robot (`ctc-linux64-atom-<version>`):

```sh
CTC=/path/to/ctc-linux64-atom-2.1.4.13
export CXX_i686_unknown_linux_gnu=$CTC/cross/bin/i686-aldebaran-linux-gnu-g++     # the C++ compiler
export CARGO_TARGET_I686_UNKNOWN_LINUX_GNU_LINKER=$CTC/cross/bin/i686-aldebaran-linux-gnu-gcc
export QI_MODULE_NAOQI_SDK=$CTC/libnaoqi                # include/alcommon/..., lib/libalcommon.so
export QI_MODULE_NAOQI_INCLUDES=$CTC/boost/include      # more include directories, `:`-separated
export QI_MODULE_CPP_SDK=$CTC/libnaoqi                  # include/qi/..., lib/libqi.so (feature cpp)
cargo build --release -p qi-module-example --target i686-unknown-linux-gnu --features naoqi,cpp
```

The paths above are indicative: they depend on the layout of the toolchain. The build script
(`qi/build.rs`) documents the variables: `QI_MODULE_CPP_SDK`/`QI_MODULE_NAOQI_SDK` (a prefix
with `include` and `lib`), `..._INCLUDES`, `..._FLAGS` (such as `-D_GLIBCXX_USE_CXX11_ABI=0`) and
`..._LIBS` (the libraries linked: `qi`; `qimessaging:qitype:qi` for libqi 2.1;
`alcommon:alproxies:alvalue:alerror` for NAOqi). The API of the libqi shim is chosen from the
SDK: `qi/anymodule.hpp` (libqi 2.3 and later) or `qitype/objectfactory.hpp` (libqi 2.1).

## Deploying

The library and its declaration go in an SDK prefix, for instance `/home/nao/rust`:

```
/home/nao/rust/lib/librustexample.so
/home/nao/rust/share/qi/module/rustexample.mod      # contains: cpp
```

- **NAOqi autoload** (2.1 to 2.9): add the library to `/home/nao/naoqi/preferences/autoload.ini`
  and restart NAOqi; its `autoload` factories become services, its DCM plugins `ALModule`s.

  ```ini
  [user]
  /home/nao/rust/lib/librustexample.so
  ```

- **qilaunch** (NAOqi 2.3 to 2.9; library built with `module-cpp` against the libqi of the robot):

  ```sh
  qilaunch --qi-sdk-prefix /home/nao/rust -n rustexample -o rustexample.RustGreeter
  qilaunch --qi-sdk-prefix /home/nao/rust -n rustexample -f 'rustexample.say("Hello from Rust")'
  ```

- **qi-launch of NAOqi 2.1** (library built with `module-cpp` against libqi 2.1): the path of the
  library; every factory becomes a service. libqi 2.1 gives no session to factories: the session
  of the module is at `QI_URL`, else the local NAOqi.

  ```sh
  qi-launch /home/nao/rust/lib/librustexample.so
  ```

- **qi-launch of this crate**, on the robot or anywhere with a `qi` space:

  ```sh
  qi-launch --qi-url tcp://nao.local:9559 --qi-sdk-prefix ./prefix -n demo -o rustexample.RustGreeter
  ```

Then, from any machine: `qicli call RustGreeter.greet Nao`, `qicli call RustDcm.cycles`.

## `qi-launch`

`qi-launch` (feature `launch`) reproduces `qilaunch` (its last public version, libqicore
`d1ba0a4^:bin/qilaunch/qilaunch.cpp`, with the signal handling of libqi 2.5 to 2.9):

- `-n/--name` is mandatory, and an object (`-o/--object module.Factory`, `-m/--module`
  deprecated) or a function (`-f/--function 'module.function(json args)'`) too: else exit 1;
- `--qi-url` (default `QI_URL`, else `tcp://127.0.0.1:9559`, completed like libqi: a host alone is
  `tcp://HOST:9559`), `--qi-listen-url` (`;`-separated, default `QI_LISTEN_URL`, else
  `tcp://127.0.0.1:0` when serving objects), `--qi-standalone` (exclusive with `--qi-url`),
  `--qi-sdk-prefix`, `--qi-log-level` (0 to 6 or the names of libqi; default `QI_LOG_LEVEL`);
  `-k`, and `-b`, `-l`, `-q` accepted without effect;
- objects are loaded first, in order, then the functions are called concurrently; the arguments
  are JSON, converted to the types of the parameters (a parse failure is logged and the function
  is called without arguments, as `qilaunch` does);
- with objects or `-k`, it runs until a signal or a failing function; else it waits for every
  function. It exits 1 when a function fails or is canceled, or when loading fails;
- the first `SIGINT`/`SIGTERM` cancels the function calls and stops; the second exits at once;
- modules are found like libqi finds them: `qi/module/<name>.mod` in the writable data directory
  (`$QI_WRITABLE_PATH/data`, else `~/.local/share`), then in `share` of the SDK prefixes
  (`--qi-sdk-prefix` or `QI_SDK_PREFIX`, the parent of the directory of the program,
  `QI_ADDITIONAL_SDK_PREFIXES`, the entries of `share/qi/path.conf`); the library with the `lib`
  prefix and `.so` suffix or without;
- like the `qi-launch` of NAOqi 2.1, positional arguments are libraries whose factories are all
  registered, and `-a` listens on all interfaces.

It loads the modules written in Rust only: a C++ module needs `qilaunch`.

## How it works

The library carries its own copy of the `qi` crate and runs it on its own threads (a Tokio
runtime of two workers, started on first use). It serves the module object, whose methods are the
functions and factories of the module, from a *private space* on `tcp://127.0.0.1` (ephemeral
port), where the module is the service named after it. Hosts reach the members through that
space, with the messaging protocol, so that no Rust or C++ type crosses the boundary of the
library:

- `qi-launch` reads the descriptor of `qi_rs_module_v1` (version, name, factories, two functions:
  `set_option` and `start`, which returns the endpoint of the private space), connects to the
  private space and registers the objects of factories, proxies to the Rust objects, in its space;
- the libqi shim (`qi/cpp/module_shim.cpp`) does the same in C++: it connects a `qi::Session` to
  the private space, and fills the `ModuleBuilder` with one dynamic method per member forwarding to
  the module object, returning futures (the network thread of libqi never blocks). It passes the
  machine identifier of libqi to the module: libqi keeps the loopback endpoints of the services of
  its own machine only;
- `_createModule` registers the `autoload` objects through the session of the module, and hands
  the broker to the DCM bridge.

The library pins itself in memory (`RTLD_NODELETE`: its threads run until the process exits) and
stops its runtime at the exit of the process, before the static destructors of the host run.

## Testing

`qi/tests/module.rs` (feature `launch`) runs `qi-launch` and the host API against spaces hosted by
the tests; `qi/tests/module_autoload.rs` (feature `module`) calls `_createModule` and
`_closeModule` in process. The interoperability tests, skipped when their programs are missing
(`QI_INTEROP_REQUIRE=1` makes it an error):

| Test | Program | Built by |
|---|---|---|
| `interop_qilaunch.rs` | the real `qilaunch`, on libqi 4.0.5 | `interop/qilaunch/build.sh` |
| `interop_cpp21_module.rs` | the `qi-launch` and `qi-master` of libqi 2.1 | `interop/cpp21/libqi-2.1/build-libqi-2.1.sh`, `interop/cpp21/build-module.sh` |
| (manual) `interop/libqi-2.9/run.sh` | the real `qilaunch`, on the libqi of NAOqi 2.9 | `interop/libqi-2.9/build.sh` (Ubuntu 18.04 container) |
| `interop_naoqi.rs` | `fake-naoqi` (`_createModule` with a `boost::shared_ptr` broker) and `fake-dcm` (the loop of a DCM, against the stub of the NAOqi SDK of `interop/naoqi-sdk-stub`) | `interop/naoqi-autoload/build.sh` |

## Limitations and what is not verified

- **Not tested on a robot.** The autoload entry points follow the C++ DCM plugins of NAOqi 2.1 to
  2.8; whether NAOqi 2.9 still loads `ALModule` libraries from `autoload.ini` is not verified.
- **The DCM bridge is built against a stub** of the NAOqi C++ SDK, which mimics the API the C++
  DCM plugins use (`ALModule::createModule`, `getParentBroker()->getDcmProxy()`,
  `getProxy("DCM")->getModule()->atPreProcess(...)`, `ALMemoryProxy::getDataPtr`,
  `DCMProxy::createAlias/setAlias/getTime`, `ALValue`), not against the SDK itself.
- The libqi shim is tested against libqi 4.0.5 and the libqi of NAOqi 2.9 (the `release-2.9`
  branch of libqi) with the real `qilaunch`, and libqi 2.1 with its `qi-launch`, all built from
  source: not against the libraries of a robot, built by its toolchain.
- The private space of a module accepts any local connection, without authentication.
- After a `fork` without `exec`, the threads of the module do not exist in the child.
- The kernel of NAOqi 2.1 robots may be older than the 3.2 the Rust standard
  library supports (`uname -r` tells); not tested.

The open points are tracked in `docs/issues/` (16 to 18).
