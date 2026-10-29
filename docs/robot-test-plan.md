# Test plan: Rust modules on a NAO (NAOqi 2.1)

The steps go from the least to the most intrusive: nothing is installed on the robot before step
3, and NAOqi only loads Rust code from step 5. Each step says what to check and what to send back
when it fails. Commands marked *(to confirm)* are assumptions about the NAOqi 2.1 system that this
plan could not check.

Conventions: `NAO=<address of the robot>` on the computer; the package is the one built by
`examples/rust-module/build-robot.sh` (`dist/rustexample-robot`), unpacked on the computer.

## 0. Before starting: what the robot is

On the robot (`ssh nao@$NAO`):

```sh
uname -m                       # i686 expected on a NAO V4: use the i686/ folder of the package
uname -r                       # the kernel; Rust supports 3.2 and later
/lib/libc.so.6 | head -1       # the glibc; the package needs 2.12 or later
cat /etc/naoqi-version 2>/dev/null; ls /opt/aldebaran 2>/dev/null   # (to confirm) NAOqi version
ls -l /home/nao/naoqi/preferences/autoload.ini
which qi-launch qicli
```

Send back the output of all of these: it settles the open points of `docs/issues/17`.

Stop here if the glibc is older than 2.12 (the package must be rebuilt with `GLIBC=2.11` or
lower: tell me the version).

## 1. From the computer, nothing on the robot

The module runs on the computer, with the Rust `qi-launch`, in the space of the robot.

```sh
cargo build --release -p libqi-vibe --features launch,cli --bin qi-launch --bin qi-cli
P=dist/rustexample-robot/x86_64          # the architecture of the computer
target/release/qi-launch --qi-url tcp://$NAO:9559 --qi-sdk-prefix $P -n demo -a -o rustexample.RustGreeter
```

In another terminal:

- [ ] `target/release/qi-cli --url tcp://$NAO:9559 call RustGreeter.greet '"Nao"'` prints
      `Hello, Nao, from Rust!`
- [ ] `qi-cli ... call RustGreeter.services` lists the services of the robot (`ALMemory`,
      `ALTextToSpeech`...): the session of the module reached NAOqi 2.1 (legacy protocol).
- [ ] `qicli --qi-url tcp://$NAO:9559 call RustGreeter.greet Nao` from the robot, or with the
      `qicli` of the robot: a C++ 2.1 client reaches a service registered from the computer
      (needs `-a`, done above).
- [ ] `target/release/qi-launch --qi-url tcp://$NAO:9559 --qi-sdk-prefix $P -n demo -f 'rustexample.say("Hello from Rust")'`:
      the robot speaks, the command exits with status 0.
- [ ] Ctrl-C on the first `qi-launch`: it exits, and `RustGreeter` is gone from
      `qicli --qi-url tcp://$NAO:9559 info`.

On failure: the output of `qi-launch` with `--qi-log-level debug`.

## 2. Build what runs on the robot

On the computer, for step 3 (a `qi-launch` running on the robot):

```sh
cargo zigbuild --release -p libqi-vibe --features launch --bin qi-launch \
  --target i686-unknown-linux-gnu.2.13
objdump -T target/i686-unknown-linux-gnu/release/qi-launch | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1
```

- [ ] The newest glibc version required is not newer than the one of step 0.

## 3. On the robot, outside NAOqi

The library is loaded on the robot, by a process of its own: its loading, the glibc and the kernel
are tested without risk for NAOqi.

```sh
scp -r dist/rustexample-robot/i686 nao@$NAO:/home/nao/rust
scp target/i686-unknown-linux-gnu/release/qi-launch nao@$NAO:/home/nao/rust/
ssh nao@$NAO
cd /home/nao/rust
./qi-launch --qi-sdk-prefix /home/nao/rust -n demo -a -o rustexample.RustGreeter
```

- [ ] It starts without `dlopen` error (`GLIBC_... not found`, `wrong ELF class`) and without
      crash (a crash at start would point to the kernel).
- [ ] From the computer: `qicli --qi-url tcp://$NAO:9559 call RustGreeter.greet Nao` and
      `call RustGreeter.services` answer.
- [ ] `./qi-launch --qi-sdk-prefix /home/nao/rust -n demo -f 'rustexample.say("Rust on the robot")'`:
      the robot speaks.
- [ ] `top` while it runs: note the CPU and memory of `qi-launch` (idle: expect around 0 % and
      a few MB).

On failure: the full output, and `uname -r`.

## 4. The entry point of NAOqi, outside NAOqi

Before handing the library to NAOqi, its `_createModule` is called by a small loader on the robot,
which is what NAOqi does. Build it on the computer (`pip install ziglang` provides the compiler):

```sh
python3 -m ziglang cc -target x86-linux-gnu.2.13 -O1 -o loader32 -x c - -ldl <<'EOF'
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
int main(int argc, char** argv) {
  void* broker[2] = {0, 0};
  void* lib = dlopen(argv[1], RTLD_NOW | RTLD_GLOBAL);
  if (!lib) { fprintf(stderr, "%s\n", dlerror()); return 1; }
  int (*create)(void*) = (int (*)(void*))dlsym(lib, "_createModule");
  int (*close_module)(void) = (int (*)(void))dlsym(lib, "_closeModule");
  printf("_createModule: %d\n", create(broker)); fflush(stdout);
  sleep(atoi(argv[2]));
  printf("_closeModule: %d\n", close_module());
  return 0;
}
EOF
scp loader32 nao@$NAO:/home/nao/rust/
```

On the robot: `QI_MODULE_LOG=info /home/nao/rust/loader32 /home/nao/rust/lib/librustexample.so 30`.

- [ ] `_createModule: 0`, then the logs `service registered ... RustGreeter` and `RustDcm`.
- [ ] Within the 30 seconds, from the computer: `qicli --qi-url tcp://$NAO:9559 call RustGreeter.greet Nao`
      answers (the services listen on every interface, as NAOqi's).
- [ ] `_closeModule: 0`, the services are gone, and the loader exits promptly.
- [ ] The log `could not create the DCM plugin ... module-naoqi` is expected: the package has no
      DCM bridge.

## 5. NAOqi loads the library (`autoload.ini`)

```sh
ssh nao@$NAO
cp /home/nao/naoqi/preferences/autoload.ini /home/nao/naoqi/preferences/autoload.ini.before-rust
printf '[user]\n/home/nao/rust/lib/librustexample.so\n' >> /home/nao/naoqi/preferences/autoload.ini
nao restart
```

If `autoload.ini` already has a `[user]` section, add the path to it instead. To undo:
`cp autoload.ini.before-rust autoload.ini && nao restart`.

- [ ] NAOqi starts as usual (the robot says its name / its eyes turn on as usual).
- [ ] `qicli --qi-url tcp://$NAO:9559 info RustGreeter RustDcm` lists both services, with the
      process of NAOqi.
- [ ] `qicli ... call RustGreeter.greet Nao`, `call RustGreeter.services`, `call RustDcm.cycles`
      (0: no DCM bridge in this build).
- [ ] `qicli ... call RustGreeter.greet` with a 2.1 Python or Choregraphe client, if at hand.
- [ ] `nao restart` again: NAOqi stops in its usual time (`_closeModule` does not delay it), and
      the services come back.
- [ ] `top`: the CPU of NAOqi at rest is the same as without the module (± 1 %).
- [ ] The logs of NAOqi at start (where it logs on this robot, *to confirm*: the output of
      `nao restart`, `/var/log/naoqi*`, `~/.naoqi`): the lines `module loaded by NAOqi` and
      `service registered` of `qi::module`, no error.

On failure: the logs of NAOqi at start, and whether NAOqi came up at all.

## 6. The DCM plugin

Needs the library built with the `naoqi` feature against the cross toolchain of the robot
(`ctc-linux64-atom-2.1.4.13`; see "With the C++ bridges" in `docs/modules.md` for the variables).
On the computer:

```sh
cargo build --release -p qi-module-example --target i686-unknown-linux-gnu --features naoqi
objdump -p target/i686-unknown-linux-gnu/release/librustexample.so | grep NEEDED   # libalcommon...
objdump -T target/i686-unknown-linux-gnu/release/librustexample.so | grep -o 'GLIBC_[0-9.]*\|GLIBCXX_[0-9.]*' | sort -uV | tail -3
```

- [ ] It builds. If it does not, send the compiler errors: the bridge was only built against a
      stub of the SDK so far, and this is where a difference shows.
- [ ] The `GLIBCXX` versions required exist on the robot
      (`strings /usr/lib/libstdc++.so.6 | grep GLIBCXX` *(to confirm the path)*).

Deploy it in place of the library of step 5 and `nao restart`. The plugin only commands the blue
LEDs of the eyes: no motor is moved.

- [ ] `qicli ... info RustEyes`: the ALModule of the plugin exists.
- [ ] The blue LEDs of the eyes turn one segment every 100 ms.
- [ ] `qicli ... call RustDcm.cycles` twice, 10 s apart: about 1000 more (one per cycle of 10 ms).
- [ ] `qicli ... call RustDcm.period`: 10.
- [ ] Head stiffness off (`qicli call ALMotion.setStiffnesses Head 0.0`), turn the head by hand:
      `qicli ... call RustDcm.headYaw` follows (radians).
- [ ] `qicli ... call RustDcm.setLedAnimation false`: the eyes stop turning; `true` restarts them.
- [ ] `top` and the logs of NAOqi: no new warning about the DCM being late *(to confirm the
      wording)*, CPU of NAOqi as in step 5 plus a little.
- [ ] `nao restart` twice: clean stops (the plugin is destroyed with its ALModule).

## 7. The `qi-launch` of NAOqi 2.1 (optional)

Needs the library built with the `cpp` feature against the libqi of the toolchain.

```sh
QI_URL=tcp://127.0.0.1:9559 qi-launch /home/nao/rust/lib/librustexample.so
```

- [ ] `RustGreeter`, `RustDcm` and `OtherGreeter` appear, and answer from the computer.

## What to send back

For each step: passed, or the failing check with its output. With step 0, that is enough to fix
what differs from the emulations, and to update `docs/issues/17`.
