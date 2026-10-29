# rustexample: a qi module written in Rust, for NAO and Pepper

Built from `examples/rust-module` of libqi-rs-vibe by `examples/rust-module/build-robot.sh`:
`i686/` for 32-bit systems (NAO V4 and V5), `x86_64/` for 64-bit ones. Check with `uname -m` on
the robot. Both need glibc 2.12 or later and nothing else (no libqi, no libstdc++).

These libraries have no C++ bridge: NAOqi loads them from its `autoload.ini`, but the DCM plugin
`RustEyes` and `qilaunch` need the bridges built with the cross toolchain of the robot (see
`docs/modules.md`).

## On the robot (NAOqi 2.1 to 2.9)

```sh
scp -r i686 nao@<robot>:/home/nao/rust     # or x86_64: creates /home/nao/rust
ssh nao@<robot>
cat >> /home/nao/naoqi/preferences/autoload.ini <<'END'
[user]
/home/nao/rust/lib/librustexample.so
END
nao restart          # or restart NAOqi from the web page of the robot
```

If `autoload.ini` already has a `[user]` section, add the path to it instead.

From a computer:

```sh
qicli --qi-url tcp://<robot>:9559 info RustGreeter
qicli --qi-url tcp://<robot>:9559 call RustGreeter.greet Nao       # "Hello, Nao, from Rust!"
qicli --qi-url tcp://<robot>:9559 call RustGreeter.services        # the services of NAOqi
qicli --qi-url tcp://<robot>:9559 call RustDcm.cycles              # 0: no DCM plugin in this build
```

(or `qi-cli --url tcp://<robot>:9559 call RustGreeter.greet '"Nao"'` with the Rust client).

The logs of the module are in the logs of NAOqi (its standard error), with more details with
`QI_MODULE_LOG=debug` in the environment of NAOqi. A library that does not load shows in the logs
of NAOqi when it starts (`dlopen` errors: a missing `GLIBC_...` version means the robot needs a
build for an older glibc, `wrong ELF class` the other architecture).

## With qi-launch, from anywhere

The Rust counterpart of qilaunch loads these libraries too, in a process of its own:

```sh
qi-launch --qi-url tcp://<robot>:9559 --qi-sdk-prefix x86_64 -n demo -o rustexample.RustGreeter
qi-launch --qi-url tcp://<robot>:9559 --qi-sdk-prefix x86_64 -n demo -f 'rustexample.say("Hello from Rust")'
```

(on a computer: the prefix of its architecture; on the robot: `/home/nao/rust`).
