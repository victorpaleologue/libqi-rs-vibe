# C++ interoperability harness (libqi of NAOqi 2.1)

This directory is the counterpart of [`interop/cpp`](../cpp/README.md) for the `libqi` that
NAOqi 2.1 robots run (2014): the same programs, written against the API of that time and built
against that library, to check that the Rust crate interoperates with NAOqi 2.1 processes as
client, service and service directory, and to characterize how the protocol differs from
`libqi` 4.0.5. Nothing here is part of the Rust workspace.

## The libqi of NAOqi 2.1

NAOqi 2.1 (`v2.1.3`, November 2014) predates the merge of the three repositories `libqi`,
`libqitype` and `libqimessaging` into one. The public `libqi` repository holds the history of
all three: tag `v2.1.3` is the core library only, and the type system and messaging of that
release are the last commits of their branches before the merge of June 2014:

| Component | Commit | Date |
|---|---|---|
| libqi (core) | `v2.1.3` (`e9ec1e5a`, hotfix/release-2.1-348) | 2014-11-13 |
| libqitype | `f0b299a1` (team/platform/dev-50) | 2014-06-12 |
| libqimessaging | `91217eb4` (last commit before the merge) | 2014-06-12 |
| qibuild (CMake framework) | `v3.5.3` | 2014-08-06 |

That code builds with GCC 4.8 and Boost 1.54: [`libqi-2.1/build-libqi-2.1.sh`](libqi-2.1/build-libqi-2.1.sh)
builds it in an Ubuntu 14.04 Docker container (the LTTng probes of the core are stubbed out),
installs it in `$LIBQI21_ROOT/install21` (default `/home/user/aldebaran`) and copies the Boost
and ICU libraries of the container next to it, so that the binaries run on the host.

```sh
interop/cpp21/libqi-2.1/build-libqi-2.1.sh   # clones and builds the 2.1 stack (about 5 minutes)
interop/cpp21/build.sh                       # builds this harness in the same container
interop/cpp21/run_cpp_smoke.sh               # 2.1 service directory + service + clients: 28 steps
```

The binaries end up in `interop/cpp21/build/`, with the library paths embedded (RPATH):

| Binary | Purpose |
|---|---|
| `qi-dump-values` | serialize the catalogue of values and messages of `interop/cpp` with the 2.1 codec |
| `qi-cpp-sd` | standalone service directory process (`qi::Session::listenStandalone`) |
| `qi-cpp-service` | the interop `TestService`, plus ALMemory-like `subscriber`, `raiseEvent`, `getData` |
| `qi-cpp-client` | scenario runner exercising a `TestService` (28 scenarios) |
| `qi-cpp-echo-client` | minimal smoke client: calls `add(1, 2)` once |
| `qi-cpp-naoqi-probe` | what a NAOqi 2.1 program does with a robot (`ALSystem`, `ALMemory` subscribers, `ALTextToSpeech`, `ALMotion`), for `naoqi-sim` |

Compared to the 4.0.5 programs: the 2.1 API has no optionals, no cancellation of remote calls,
no object UIDs, no `Session::services().uriEndpoints()`; its `DynamicObjectBuilder` cannot
bind methods returning futures (`sleepMs` is synchronous); a 2.1 client cannot subscribe to the
change signal of a property ("No such signal", on the client and on the server side), so
`property_value` only sets and gets. `qi::Message` is not exported by `libqimessaging` 2.1, so
`qi-dump-values` models the messages itself (same header, same payload rules as
`Message::setValue`/`setValues`/`setError`).

## What differs from libqi 4.0.5

`qi-dump-values` produces [`../vectors/libqi-2.1-values.jsonl`](../vectors/libqi-2.1-values.jsonl);
compared line by line with `libqi-4.0.5-values.jsonl`:

- **Every shared value is byte-identical**: primitives, strings, containers, annotated
  structures, dynamics (including the empty one), raw buffers, meta objects (same
  `MetaMethod`/`MetaSignal`/`MetaProperty`/`MetaObject` fields, same Manageable members
  80..86), messages (same 28-byte header, same magic `0x42adde42`, same version 0, same types
  and flags, same error payloads). Optionals (`+`), variadic parameters (`#`), object UIDs and
  the cancel/canceled messages do not exist in 2.1.
- **`ServiceInfo` has six fields** (`(sIsI[s]s)<ServiceInfo,name,serviceId,machineId,processId,endpoints,sessionId>`):
  the `objectUid` field was added by `libqi` 2.9. A 2.1 process cannot convert the seven-field
  structure to its own (`Unable to convert ... to ServiceInfo`), while 4.0.5 accepts the
  six-field one (`QI_TYPE_STRUCT_EXTENSION_ADDED_FIELDS`). Endpoints are absolute URLs (no
  `qi:ServiceName` relative endpoints).
- **Capabilities**: 2.1 advertises `ClientServerSocket`, `MetaObjectCache` and `MessageFlags`,
  all `true`, with a `Capability` message (type 6, service/object/action 0) that **both ends
  send right after the connection** (`TcpTransportSocket::startReading`). 4.0.5 adds
  `RemoteCancelableCalls`, `ObjectPtrUID` and `RelativeEndpointURI`, advertises
  `MetaObjectCache` as `false`, and exchanges them in the authentication handshake.
- **No authentication**: a 2.1 server has no server object (service 0). The authentication
  call of a newer client (`{0.0.8}`) is answered with a `Type_Error` reply, same id and
  address, text `can't find service, address: {0.0.8, id:1}` (`msg_error_cant_find_service`),
  and the link stays open. A 2.1 client sends its first call (the `metaObject` of the service
  directory) before its own `Capability` message.
- **Object references** are `metaObject, serviceId, objectId` without UID when the
  `MetaObjectCache` capability is not shared (`objectref_plain`). When it is shared, which
  happens between two 2.1 processes, they are `bool transmitMetaObject, [metaObject], uint32
  cacheId, serviceId, objectId` (`objectref_cached_*`); the 2.1 send cache never hits (the meta
  object is retransmitted every time). The Rust crate never advertises the cache, so it only
  ever sees the plain form.
- **Bound objects** answer the same special actions (`registerEvent` 0, `unregisterEvent` 1,
  `metaObject` 2, `terminate` 3, `property` 5, `setProperty` 6, `properties` 7,
  `registerEventWithSignature` 8), but `registerEvent` only knows signals: subscribing to the
  change signal of a property fails.
- A 2.1 process logs and ignores messages of unknown types (`cancel`, `canceled`).

## Tests

`qi/tests/interop_cpp21.rs` spawns these programs (skipped when `interop/cpp21/build` is
missing; `QI_INTEROP_CPP21_BUILD_DIR` overrides the location, `QI_INTEROP_REQUIRE=1` makes the
skip an error) and runs, over TCP loopback:

| Service directory | Service | Client | Scenarios |
|---|---|---|---|
| Rust | Rust | 2.1 (`qi-cpp-client`) | all 28 |
| Rust, emulating 2.1 (`Protocol::Legacy`) | Rust | 2.1 | all 28 |
| 2.1 | 2.1 | Rust | the client scenarios of `interop_cpp.rs` minus optionals and cancellation, plus the ALMemory subscriber pattern |
| 2.1 | Rust | 2.1 | all 28 |
| Rust | 2.1 | Rust | as above |
| `naoqi-sim --version 2.1.4.13` | | 2.1 (`qi-cpp-naoqi-probe`) | `ALSystem`, `ALMemory` (`getData`, `subscriber`, `raiseEvent`), `ALTextToSpeech`, `ALMotion` |

`interop_cpp.rs` additionally runs the 4.0.5 client against a Rust node emulating NAOqi 2.1,
which exercises the compatibility code of `libqi` 4.0.5 itself against the emulation.
