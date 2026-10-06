# Architecture

`libqi-rs` is a Rust implementation of the `qi` framework: the type system, binary
format, messaging protocol and object model of Aldebaran's `libqi` (qimessaging), the
middleware of the NAO and Pepper robots. It aims at being a drop-in peer of the C++
implementation: byte-identical serialization, the same protocol semantics, and
interoperability in every role (client, service, service directory) with `libqi` 4.x
processes and the NAOqi services they expose.

This document describes the layering of the workspace and the design decisions that
are not obvious from the code. The reference material extracted from `libqi` (binary
format, messaging protocol, objects, signals and properties, service directory) lives
in `interop/cpp/README.md` and in the byte fixtures under `interop/vectors/`.

## Crates and modules

The implementation is one crate, `libqi-vibe` (library `qi`), plus the procedural macros
that Rust requires in a crate of their own, `libqi-macros-vibe`. The crate is layered in
modules, each depending only on the ones above it:

| Module | Role |
|---|---|
| `qi::value` | The type system: `Type` and `Signature`, the dynamic `Value`, conversions between Rust types and values (`Reflect`, `ToValue`, `IntoValue`, `FromValue`), object references and meta objects, key/dynamic-value maps, machine identifiers. |
| `qi::format` | The binary format of `libqi` as a `serde` data format (`to_bytes`, `from_slice`), schema-driven: values are encoded and decoded according to their type, without tags. |
| `qi::messaging` | The messaging protocol: message framing (`Message`, `codec`), TCP and TLS channels, the client (`Client`: call, post, event, cancellation) and server (`Server`: dispatch of calls, posts and events to handlers, cooperative cancellation) loops, and the `Endpoint` that runs both over one channel. |
| `qi` (the rest) | The framework: sessions and capabilities, authentication, objects (`Object`, `AnyObject`, `ObjectClient`, `ObjectBuilder`), signals and properties, services and the service directory, nodes. |
| `qi::naoqi_sim` (feature `naoqi-sim`) | A simulated NAOqi robot exposing the services used by `naoqi_driver2` and by robot HALs; the `naoqi-sim` command runs it. |

`libqi-macros-vibe` holds the `Reflect`/`ToValue`/`IntoValue`/`FromValue`/`Valuable`
derives and the `#[qi::object]` attribute; `qi` re-exports them. The feature `cli` adds the
`qi-cli` command (inspect services, call methods, watch signals, get and set properties).
`tests-macros` holds compile tests of the macros, and `interop/`, outside the Cargo
workspace, the C++ harness built against `libqi` 4.0.5 and the byte fixtures it generates.

`qi::value` and `qi::format` know nothing about networking; `qi::messaging` knows nothing
about the type system beyond raw payloads; the rest of `qi` binds everything together. The
macros generate `::qi::...` paths, which resolve inside the crate too through
`extern crate self as qi`.

## The type system and the binary format (`qi::value`, `qi::format`)

Values of the `qi` type system are described by `Type` (unit, booleans, integers of
every width, floats, strings, raw buffers, optionals, lists, maps, tuples with
optional structure annotations, objects, and the *dynamic* type) and written as
`Signature` strings (`(is)<Point,x,y>`, `{sm}`, `[o]`...).

The binary format is *schema-driven*: a value is encoded as its raw fields in order,
with lengths prefixing strings, raw buffers, lists and maps, a boolean prefixing
optionals, and nothing else. Decoding therefore requires knowing the type in
advance: the format is a `serde` data format whose deserializer follows the shape
requested by the `Deserialize` implementation, and `qi_value::value::de::ValueType`
is a `DeserializeSeed` that decodes a dynamic `Value` given a `Type`.

Dynamic values (`m`) are the exception: they are self-describing on the wire, as the
signature string of their content followed by the content. An empty signature is
what `libqi` writes for an invalid `AnyValue`, and decodes as a unit.

Rust types map to the type system through four traits:

- `Reflect` gives the static `Type` of a Rust type (`None` for the dynamic type);
- `RuntimeReflect` gives the `Type` of a value (needed for `Value` itself);
- `ToValue`, `IntoValue` and `FromValue` convert to and from `Value`.

Structures derive them with `#[derive(qi::Valuable)]`, which also supports renaming
(`name`, `case`) and `transparent` wrappers. `Dynamic<T>` marks a dynamically typed
member, `AsRaw<T>` a raw buffer.

`Value::convert_to(&Type)` implements the conversions `libqi` applies when binding
arguments to a signature: wrapping and unwrapping dynamics, numeric conversions when
the value is representable, elementwise conversion of containers. Every place where a
value of one static type meets a member of another type (a proxy calling a remote
method, a subscription receiving events, a builder-made method receiving arguments)
applies it, which is what lets statically typed Rust code drive dynamically typed
NAOqi APIs.

Meta objects (`MetaObject`, `MetaMethod`, `MetaSignal`, `MetaProperty`) are values
too, with the exact field order and sorted member maps of `libqi`.

## Messaging (`qi::messaging`)

Messages carry a 28-byte little-endian header (magic, id, payload size, version,
type, flags, service, object and action identifiers) and a payload. The types are
call, reply, error, post, event, capabilities, cancel and canceled; the flags mark a
dynamic payload (the payload is a dynamic value carrying its own signature) and a
requested return type.

The `Client` sends calls and matches replies, errors and canceled messages to
pending requests by id. Dropping the future of a call sends a *cancel* message for
it, so cancellation follows Rust's ownership of futures. The `Server` dispatches
incoming calls to a `CallHandler`, which receives a `CancellationToken` per call: a
cancel message cancels the token, and the loop answers *canceled* when the handler
reports cancellation. Posts and events go to `PostHandler` and `EventHandler`.

Errors of handlers are either replied to the caller (the normal case: a method
failing must not disturb the link) or fatal to the loop, which then closes the
channel; only protocol-level failures are fatal.

## Sessions and capabilities (`qi`)

A `Session` is a messaging endpoint bound to a peer, in both directions: it calls the
peer's objects and serves the local objects the peer may reach. On connection, the
client authenticates (service 0, object 0, action 8) with its capability map and its
credentials (`auth_user`, `auth_token`); the server replies with its capabilities and
the authentication state. The capabilities shared by both sides (`ClientServerSocket`,
`MessageFlags`, `RemoteCancelableCalls`, `ObjectPtrUID`, `RelativeEndpointURI`;
`MetaObjectCache` is never enabled) drive the protocol variants, notably whether
object references carry the 20-byte object UID.

### Protocol variants

Two generations of the protocol exist, with the same binary format and messages
(`session::Protocol`). The *standard* one (`libqi` 2.3 and later, NAOqi 2.3 to 2.9) is the
handshake above. The *legacy* one (`libqi` up to 2.2, NAOqi 2.1 robots) has no
authentication: both ends advertise their capabilities with a `Capabilities` message right after
the connection, a server answers the authentication call with an error, and a client sends its
requests directly. Sessions detect the variant of their peer like `libqi` 2.3+ does:

- the connecting side sends the authentication call; an error reply preceded by a
  `Capabilities` message of the peer means a legacy server (an error without it is a refused
  authentication), and the local capabilities are then advertised with a `Capabilities`
  message. Credentials cannot be verified by a legacy server, so giving some is an error;
- the accepting side, when no authenticator is set, takes a first request that is not the
  authentication call as coming from a legacy client, authorizes it and advertises its
  capabilities.

The capabilities shared with a legacy peer are those it advertises (`ClientServerSocket`,
`MessageFlags`; the `MetaObjectCache` is never shared), so object references carry no UID and
calls cannot be canceled. Two more adaptations follow from what the old type system lacks:
service infos are advertised and written without their `objectUid` field to peers without the
`ObjectPtrUID` capability (and read in both forms), and the members whose signatures a legacy
peer cannot parse (optionals `+`, variadic parameters `#`) are left out of the meta objects sent
to it. A node can also *emulate* a legacy server (`with_server_protocol(Protocol::Legacy)`,
`naoqi-sim --protocol legacy` or a NAOqi version below 2.3), to test clients against the
handshake of NAOqi 2.1. The protocol of a peer is readable on its proxies
(`ObjectClient::protocol`) and on the service directory client.

### Special bound-object actions

Every object reachable through a session answers the special actions below 100 like
`libqi`'s `BoundObject`: `registerEvent`/`unregisterEvent` (0, 1) for signal
subscriptions, `metaObject` (2), `terminate` (3) to release objects, `property`,
`setProperty` and `properties` (5, 6, 7). The meta object sent to peers is the merge
of the object's own meta object with these special members.

### Object passing and hosting

Objects are values: a method may take or return objects, and signals may carry them.
An `AnyObject` (a shared `dyn Object`) converts to a `Value::Object` holding an opaque
handle to it. When a value crosses a session, the session *binds* outgoing local
objects into its object host (giving them an object id: servers allocate ids from 2,
clients from 2^31, each side in its own range, as `libqi` does) and turns incoming
references into `ObjectClient` proxies bound to the session. Proxies fetch the meta
object lazily and send `terminate` when dropped, so remote objects live as long as
someone holds a proxy.

A service calling back a client-hosted object (the "object passing" style of NAOqi
2.x) and a service reaching a service registered by a client node (the "call me by
name" style of NAOqi 1.x, like `ALAudioDevice.subscribe`) are both supported, and
covered by tests.

### Signals and properties

`Signal<T>` and `Property<T>` are either *local* (a broadcast channel and, for
properties, a shared value) or *proxies* to a member of an object. Both are used
through the same API, so an interface declared with `#[qi::object]` can expose
`&Signal<T>` and `&Property<T>` accessors implemented by objects and by their
clients alike. In the protocol, a property is also a signal of its changes with the
same identifier.

Remote subscriptions choose a link identifier on the subscriber side and register it
with `registerEvent`; the session forwards each event to the subscription streams
and unregisters the link when the last subscription is dropped. Subscriptions are
lossy by design: a slow subscriber misses the oldest values, and values that cannot
be converted to its type are dropped with a warning.

### Cancellation

Calls are cancelled by dropping their future, on the client side. On the server side,
cancellation is cooperative: the handler of a call runs in a task-local
`call::Context` exposing the cancellation token of the call (`call::cancelled()`)
and the token of the caller's link (`call::link_closed()`, which services use to
clean up what a peer registered when it disconnects). A method that ignores its
token runs to completion, and its result is discarded. When the link to a caller
drops, the tasks of its pending calls are aborted.

## Services, service directory and nodes

A *space* is a set of processes sharing a service directory. A `Node` either *hosts*
a space (it runs the service directory as service 1, object 1, with the
`serviceAdded`/`serviceRemoved` signals and the relative `qi:ServiceName` endpoints
of `libqi`) or *connects* to one. Nodes bind servers on the addresses they are given
(`tcp://host:port` or `tcps://host:port`, port 0 for an ephemeral port), publish their services (at start
or later with `register_service`), and resolve the services of the space through the
directory, reusing one session per peer node. Local services are returned directly,
without a network round trip.

Services registered by a peer are unregistered when its link closes, as the
reference implementation does.

## Interoperability testing

`interop/cpp` is a CMake project built against `libqi` 4.0.5 that provides a value
dumper, a standalone service directory, a reference test service and a scenario
client. `interop/vectors/libqi-4.0.5-values.jsonl` holds the bytes `libqi` produces
for 66 values and messages; `qi/tests/format_libqi_vectors.rs` and
`qi/tests/libqi_vectors.rs` check that the Rust crates produce and accept exactly
those bytes. `qi/tests/interop_cpp.rs` spawns the C++ processes and runs every
combination of C++ and Rust directories, services and clients (skipped when the
harness is not built).

## Known limitations

- Mutual TLS authentication (`tcpsm://`) is not implemented. Plain TLS (`tcps://`, the
  transport of NAOqi 2.9 robots on port 9503) follows `libqi`: peers do not verify each
  other's certificates, and servers use a self-signed certificate unless the
  `QI_TLS_CERTIFICATE` and `QI_TLS_PRIVATE_KEY` environment variables name PEM files.
- `Value` has no structure annotations: a structure passed *inside a dynamic value*
  through the dynamic representation loses its `<Name,fields>` annotation (statically
  typed structures keep it). `libqi` accepts both.
- The `Manageable` members `libqi` adds to every object (statistics and tracing,
  identifiers 80 to 86) are not exposed.
- `MetaObjectCache` is never negotiated (like `libqi` by default; `libqi` 2.1 advertises it,
  and two 2.1 processes transmit object references through it, in a form this implementation
  does not decode: it only ever receives the plain form, as it never advertises the cache).
- Machine identifiers follow `libqi`'s file-based scheme (`~/.config/qimessaging/machine_id`),
  then the systemd machine id hashed with `libqi`'s salt; a `libqi` built with
  systemd support uses the latter first.
