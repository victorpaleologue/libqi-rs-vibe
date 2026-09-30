# C++ interoperability harness (reference libqi 4.0.5)

This directory is a standalone CMake project built against the reference C++
implementation of the qi framework (`libqi`, tag `qi-framework-v4.0.5`). It is
used to check that the Rust crate (`libqi-vibe`, used as `qi`) produces byte-identical
serialization and interoperate with real libqi
processes. Nothing here is part of the Rust workspace.

## Building

Prerequisites: a libqi 4.0.5 install (headers, `libqi.so`, CMake package
config) and its source tree (one private header, `src/messaging/message.hpp`,
is needed to build `qi::Message` objects). Boost 1.83, OpenSSL 3 and a C++17
compiler are required by libqi itself.

```sh
cmake -S interop/cpp -B interop/cpp/build \
      -DCMAKE_PREFIX_PATH=/home/user/aldebaran/libqi/install \
      -DLIBQI_SOURCE_DIR=/home/user/aldebaran/libqi      # default, optional
cmake --build interop/cpp/build
```

The binaries end up in `interop/cpp/build/`:

| Binary | Purpose |
|---|---|
| `qi-dump-values` | serialize a fixed catalogue of values/messages, one JSON object per line |
| `qi-cpp-sd` | standalone service directory process |
| `qi-cpp-service` | reference implementation of the `TestService` interop service |
| `qi-cpp-client` | scenario runner exercising a `TestService` (foreign or C++) |
| `qi-cpp-echo-client` | minimal smoke client: calls `add(1, 2)` once |

All programs use **stdout for machine-readable output only**; libqi logs are
redirected to stderr (`common/stderr_log.hpp`), because libqi's default console
log handler writes to stdout.

`run_cpp_smoke.sh` runs the whole C++-vs-C++ chain (SD, service, echo client,
client with all scenarios) and exits 0 only if everything passes.

## `qi-dump-values`

```sh
interop/cpp/build/qi-dump-values > values.jsonl
```

No arguments. Output is deterministic (message ids are set explicitly; the
capability map comes from `qi::StreamContext::defaultCapabilities()`, so do
not set `QI_TRANSPORT_CAPABILITIES` when regenerating). The committed copy of
the output is `interop/vectors/libqi-4.0.5-values.jsonl`.

Value lines:

```json
{"name": "<id>", "signature": "<qi signature>", "hex": "<lowercase hex>", "qi_json": "<optional>"}
```

* `signature` is `qi::typeOf<T>()->signature().toString()` (for runtime tuples,
  the reference's own type signature). Struct annotations are included.
* `hex` is exactly what libqi puts on the wire for that value: the result of
  `qi::encodeBinary`, with `qi::Buffer` sub-buffers inlined after their
  `uint32` size prefix the same way `src/messaging/sock/send.hpp` does.
* `qi_json` is `qi::encodeJSON(value)` embedded as a string, informative only
  (omitted for raw buffers, which libqi cannot encode to JSON).

Message lines (`"kind": "message"`):

```json
{"name": "...", "kind": "message", "id": 1, "type": 1, "type_name": "Call", "flags": 0,
 "service": 0, "object": 0, "action": 8, "payload_size": 161,
 "payload_hex": "...", "hex": "<28-byte header + payload>", "note": "..."}
```

`hex` includes the 28-byte header (`magic 0x42adde42`, id, size, version 0,
type, flags, service, object, action; all little-endian). `payload_hex` is
the payload alone.

Catalogue (names as they appear in the file):

* primitives: `bool_true`, `bool_false`, `int8_neg5`, `uint8_200`, `int16_neg1234`,
  `uint16_60000`, `int32_neg100000`, `uint32_3000000000`, `int64_neg5e12`,
  `uint64_1e19`, `float_1_5`, `double_neg2_25`
* strings: `string_empty`, `string_abc`, `string_hello_utf8` ("héllo")
* containers: `vec_int32_1_2_3`, `vec_string_a_bc`, `vec_int32_empty`,
  `map_string_int32`, `map_int32_string`, `tuple_int32_string` (runtime
  `AnyValue::makeTuple`, `std::tuple` is not a libqi type), `pair_int32_string`
* structs: `struct_point2d` (`(ii)<Point2D,x,y>`, x=4 y=2),
  `struct_nested_timestamped_point2d`
* dynamics (`m`): `dynamic_int32_5`, `dynamic_string_abc`, `dynamic_vec_int32_1_2`,
  `dynamic_point2d`, `dynamic_empty` (empty `AnyValue`), `dynamic_void`,
  `dynamic_bool_true`, `dynamic_double_1_5`, `dynamic_map_string_dynamic`,
  `vec_dynamic_int32_string` (`[m]`)
* optionals: `optional_int32_none`, `optional_int32_some_7`, `optional_string_some_abc`
* raw: `buffer_4_bytes` (`2a 00 00 00`), `buffer_empty`, `tuple_int32_buffer_string`
* misc: `signature_value_is`, `os_timeval`, `url_tcp_localhost`, `objectuid_raw_20_bytes`
* `metaobject_builder_raw` (DynamicObjectBuilder before `object()`: `add::(i)->i`
  with description, signal `fire::(i)`, property `val` of type `i`, description
  "Interop test object"), `metaobject_full_object` (same after `object()`,
  i.e. merged with the Manageable members 80..86), `metaobject_empty`
* `serviceinfo_calculator` (name Calculator, id 2, machineId
  `9a65b56e-c3d3-4485-8924-661b036202b3`, pid 3420486, endpoint
  `tcp://127.0.0.1:41681`, sessionId `361ecec4-00f7-4c94-a6e2-d91e28c5a06c`,
  objectUid = bytes 0x01..0x14 through `serializeObjectUid<std::string>`),
  `serviceinfo_servicedirectory_no_uid`, `vec_serviceinfo_one`
* `capabilitymap_default` (the six default capabilities), `capabilitymap_auth_state_done`
* messages: `msg_call_authenticate` (Call, service 0, object 0, action 8, id 1,
  payload = default capability map), `msg_reply_authenticate_state_only`
  (`{__qi_auth_state: 3u}`), `msg_reply_authenticate_with_caps`, `msg_error_boom`
  (Error id 5), `msg_cancel_call_7` (Cancel, payload uint32 7),
  `msg_canceled_call_7` (Canceled, id 7, empty payload), `msg_event_int32_42`
  (Event service 2 object 1 action 100), `msg_call_add_1_2`, `msg_reply_add_3`,
  `msg_call_metaobject`, `msg_call_registerevent`, `msg_post_string_abc`,
  `msg_call_dynamic_payload` (flag `TypeFlag_DynamicPayload`),
  `msg_capability_default`

## `qi-cpp-sd`

```sh
qi-cpp-sd [--qi-listen-url tcp://127.0.0.1:0]
```

Runs `qi::Session::listenStandalone(url)` and prints `LISTENING <url>` with the
actual endpoint (resolved port) on stdout, then waits for SIGINT/SIGTERM and
exits 0. (libqi's own `tests/messaging/simplesd` is built in
`/home/user/aldebaran/libqi/build/sdk/bin/simplesd`, but it uses a TLS
server configuration and prints nothing, hence this dedicated program.)

## `qi-cpp-service`

```sh
qi-cpp-service [--qi-url tcp://sd:port] [--qi-listen-url URL] [--qi-standalone] [--name TestService]
```

A `qi::ApplicationSession` program; the standard qi options apply (`--qi-url`
to reach a service directory, `--qi-listen-url` for the endpoint,
`--qi-standalone` to host the SD in-process). Prints `READY` on stdout once
registered, `TICK <n>` whenever a callback object emits `tick`, and exits
cleanly on SIGINT/SIGTERM. The object is built with `qi::DynamicObjectBuilder`
(multi-threaded threading model) and exposes:

| Member | Signature | Behaviour |
|---|---|---|
| `add` | `(ii) -> i` | a + b |
| `concat` | `(ss) -> s` | a + b |
| `echoDynamic` | `(m) -> m` | returns its argument |
| `echoList` | `([i]) -> [i]` | returns its argument |
| `echoMap` | `({si}) -> {si}` | returns its argument |
| `echoStruct` | `((ii)<Point2D,x,y>) -> (ii)<Point2D,x,y>` | returns its argument |
| `echoOptional` | `(+i) -> +i` | returns its argument |
| `echoBuffer` | `(r) -> r` | returns its argument |
| `fail` | `() -> v` | throws `std::runtime_error("expected failure")`, delivered as an Error reply with text `expected failure` |
| `sleepMs` | `(i) -> i` | returns ms after ms milliseconds; cancellable (Promise `setOnCancel`), a cancelled call is answered with `Type_Canceled` |
| `fire` | `(i) -> v` | emits `fired(int)` |
| `makeCounter` | `() -> o` | new dynamic object with `increment() -> i`, `current() -> i`, signal `changed(i)` |
| `useCallback` | `(oi) -> i` | calls `cb.compute(n)`, subscribes to `cb.tick` if present (keeping `cb` alive), returns the result |
| `emitVoid` | `() -> v` | emits `voidSignal()` |
| `bigString` | `(i) -> s` | string of `size` `'x'` characters |
| signal `fired` | `(i)` | |
| signal `voidSignal` | `()` | |
| property `value` | `i` | |
| property `text` | `s` | |

## `qi-cpp-client`

```sh
qi-cpp-client --qi-url tcp://sd:port [--service TestService] [--scenarios all|list|a,b,c] [--timeout-ms 5000]
```

Uses `qi::Session` directly. Every step prints one line:

```json
{"step": "<name>", "ok": true, "result": <json>}
```

followed by `{"step":"summary","ok":<bool>,"passed":N,"failed":M}`. On failure
`result` is `{"error": "<message>"}`. Exit code 0 only if every step passed.
`--scenarios list` prints the step names; a comma-separated subset always adds
`connect`, `get_service` and `close`.

Steps and expectations:

| Step | What it checks |
|---|---|
| `connect` | `Session::connect(url)` |
| `sd_services` | `Session::services()` lists the service; result = every ServiceInfo (name, serviceId, machineId, processId, sessionId, objectUid length, endpoints) |
| `sd_machineId` | `ServiceDirectory.machineId()` is non-empty |
| `get_service` | `Session::service(name)`; result = user members (ids >= 100) of the MetaObject |
| `add` | `add(1, 2) == 3` |
| `concat` | `concat("foo", "bar") == "foobar"` |
| `echoDynamic_int` / `_string` / `_list` / `_struct` | the dynamic comes back with the same content (`42`, `"héllo"`, `[1,2,3]`, `Point2D{4,2}`); result includes the returned inner signature |
| `echoDynamic_empty` | an empty dynamic comes back empty (or void) |
| `echoList`, `echoList_empty`, `echoMap`, `echoStruct` | round trips of `[1,-2,300000]`, `[]`, `{"a":1,"b":2,"héllo":-3}`, `Point2D{4,2}` |
| `echoOptional_none` / `echoOptional_some` | `none` and `some(7)` round trips |
| `echoBuffer` | 5-byte raw buffer round trip |
| `fail` | the call fails and the error text contains `expected failure` |
| `sleepMs_short` | `sleepMs(50) == 50` |
| `sleepMs_cancel` | `sleepMs(5000)` cancelled after 100 ms ends in the canceled state |
| `signal_fired` | subscribe to `fired`, call `fire(1)`, `fire(2)`, receive `[1, 2]`, unsubscribe |
| `signal_void` | `emitVoid()` delivers one `voidSignal` event |
| `property_value` | `setProperty("value", 42)`, `property("value") == 42`, and a change event carrying 42 |
| `property_text` | `text` set/get with a UTF-8 string |
| `property_generic` | the low-level `property(id)` / `setProperty(id, AnyValue)` path |
| `makeCounter` | returned object: `increment()` twice gives 1, 2; `current() == 2`; `changed` events `[1, 2]`; then the proxy is dropped |
| `useCallback` | passes a local dynamic object (`compute(int) -> int` returning `n*2`, signal `tick(int)`); expects 42 and exactly one `compute(21)` call, then emits `tick(7)` once |
| `bigString` | `bigString(200000)` returns 200000 characters |
| `close` | `Session::close()` |

## `qi-cpp-echo-client`

```sh
qi-cpp-echo-client --qi-url tcp://sd:port [--service TestService]
```

Fetches the service, calls `add(1, 2)`, prints the result (`3`) and exits 0 on
success.

## Protocol observations recorded while building the harness

* The authenticate call sent by a default libqi client is exactly
  `msg_call_authenticate` in the fixture: 28-byte header
  `42dead42 01000000 a1000000 0000 01 00 00000000 00000000 08000000` followed by
  a 161-byte `{sm}` map of the six default capabilities (`ClientServerSocket=true`,
  `MessageFlags=true`, `MetaObjectCache=false`, `ObjectPtrUID=true`,
  `RelativeEndpointURI=true`, `RemoteCancelableCalls=true`; each value a
  dynamic `"b"` + 1 byte).
* Struct annotations **are** transmitted inside dynamics: `dynamic_point2d` is
  the string `(ii)<Point2D,x,y>` followed by the two int32s, and a
  `echoDynamic(Point2D)` round trip through libqi returns the annotated
  signature. They are never on the wire for statically typed fields.
* A `qi::AnyValue` passed straight to `qi::encodeBinary` is serialized as its
  *content* (`AutoAnyReference` slices it); to serialize it as a dynamic you
  need an `AnyReference::from(anyValue)` (done by `dumpDynamic`).
* `std::tuple` is not registered with the libqi type system (only `std::pair`
  and runtime tuples via `AnyValue::makeTuple`).
* `DynamicObjectBuilder::setDescription` is lost by `object()`: the merged
  MetaObject takes the (empty) Manageable description
  (`metaobject_builder_raw` ends with `"Interop test object"`,
  `metaobject_full_object` with an empty string). User member ids start at
  100 and are shared between methods, signals and properties (`add`=100,
  `fire`=101, `val`=102 both as signal `val::(i)` and property `val::i`).
* The full object MetaObject carries the six Manageable methods 80..85
  (`isStatsEnabled::()->b`, `enableStats::(b)`, `stats::()->{I(...)<MethodStatistics,...>}`,
  `clearStats::()`, `isTraceEnabled::()->b`, `enableTrace::(b)`) and the
  signal 86 `traceObject::((IiIm(ll)<timeval,tv_sec,tv_usec>llII)<EventTrace,...>)`,
  all with empty descriptions.
* A cancelled `sleepMs` call is answered with `Type_Canceled` almost
  immediately (about 1 ms in the smoke run).
* A service that connects to a client-side object's signal (`useCallback`)
  must keep the proxy alive, otherwise the proxy destructor sends `terminate`
  and unregisters the event before the client emits it.
* The service directory lists itself with a relative endpoint `qi:ServiceDirectory`
  in addition to its TCP endpoint (`RelativeEndpointURI` capability);
  `qi::Url::str()` renders that URI as `qi://`, so the client reports
  `uriEndpoints()`.
* `qi::os::timeval` is a registered struct `(ll)<timeval,tv_sec,tv_usec>`;
  `qi::Signature`, `qi::Url` and `qi::Uri` are string-equivalent (`s`).
