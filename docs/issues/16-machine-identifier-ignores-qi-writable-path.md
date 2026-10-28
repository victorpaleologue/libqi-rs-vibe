---
title: The machine identifier ignores QI_WRITABLE_PATH, which libqi honors
labels: bug
---

`MachineId::local` (`qi/src/value/os.rs`) reads `<config dir>/qimessaging/machine_id`, with the
configuration directory of the `dirs` crate (`$XDG_CONFIG_HOME`, else `~/.config`). `libqi` reads
`qi::path::userWritableConfPath("qimessaging", "machine_id")`: `$QI_WRITABLE_PATH/config/...` when
the variable is set (2.1: `libqi-2.1.3/src/os.cpp`, 4.0.5: `src/os.cpp:275`), and ignores
`XDG_CONFIG_HOME`.

A Rust process and a C++ one of the same machine then disagree on the identifier whenever one of
these variables is set, and each drops the loopback endpoints of the services of the other, which
it takes for a remote machine (`sort_service_endpoints` in `qi/src/node.rs`; `libqi` filters the
same way). The module tests met it: `qilaunch` run with `QI_WRITABLE_PATH` registered services
the Rust client of the test could not reach.

Modules are not affected (the C++ shim gives the identifier of libqi to the module,
`MachineId::set_local`). Fix: follow `libqi`'s lookup (`QI_WRITABLE_PATH` first, `XDG_CONFIG_HOME`
ignored), with a test.
