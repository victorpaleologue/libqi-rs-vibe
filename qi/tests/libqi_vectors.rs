//! Byte-identity tests against the serialization vectors of the reference C++ implementation of
//! the `qi` framework (libqi 4.0.5), `interop/vectors/libqi-4.0.5-values.jsonl`, for the rows
//! that map to types of the `qi` crate: service infos, capability maps and messages.
//!
//! The other value rows are checked in `qi/tests/format_libqi_vectors.rs`, which also verifies
//! that the two files together cover every row of the fixture exactly once. The list of value rows
//! checked here, [`VALUE_ROWS_CHECKED_HERE`], must be kept in sync with the list of delegated rows
//! of that file.

use bytes::BytesMut;
use qi::{
    format::{from_slice, to_bytes},
    messaging::{
        codec::{Decoder, Encoder},
        message::{self, Address, Id},
        Flags, Message,
    },
    service,
    value::{
        object, Dynamic, FormatInto, FromValue, IntoFormat, IntoValue, KeyDynValueMap, Reflect,
        String as QiString, Value,
    },
    Capabilities,
};
use std::path::{Path, PathBuf};
use tokio_util::codec::{Decoder as _, Encoder as _};

// Fixture
// ============================================================================

/// A line of the fixture. Value rows have a `signature`; message rows have `kind: "message"`
/// and the header fields.
#[derive(serde::Deserialize, Debug)]
struct Row {
    name: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    signature: String,
    hex: String,
    #[serde(default)]
    id: Option<u32>,
    #[serde(default)]
    type_name: Option<String>,
    #[serde(default)]
    flags: Option<u8>,
    #[serde(default)]
    service: Option<u32>,
    #[serde(default)]
    object: Option<u32>,
    #[serde(default)]
    action: Option<u32>,
    #[serde(default)]
    payload_size: Option<usize>,
    #[serde(default)]
    payload_hex: Option<String>,
}

impl Row {
    fn is_message(&self) -> bool {
        self.kind.as_deref() == Some("message")
    }

    fn bytes(&self) -> Vec<u8> {
        decode_hex(&self.hex)
    }

    fn header_field<T: Copy>(&self, field: &str, value: Option<T>) -> T {
        value.unwrap_or_else(|| panic!("{}: message row without field {field:?}", self.name))
    }
}

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../interop/vectors/libqi-4.0.5-values.jsonl")
}

fn load_rows() -> Vec<Row> {
    let path = fixture_path();
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("cannot read the fixture {}: {err}", path.display()));
    let rows: Vec<Row> = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|err| panic!("invalid fixture line {line:?}: {err}"))
        })
        .collect();
    assert!(!rows.is_empty(), "the fixture {} is empty", path.display());
    rows
}

fn decode_hex(hex: &str) -> Vec<u8> {
    assert!(hex.len().is_multiple_of(2), "odd hex string {hex:?}");
    (0..hex.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&hex[index..index + 2], 16)
                .unwrap_or_else(|err| panic!("invalid hex string {hex:?}: {err}"))
        })
        .collect()
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

// Service infos
// ============================================================================

const MACHINE_ID: &str = "9a65b56e-c3d3-4485-8924-661b036202b3";
const PROCESS_ID: u32 = 3_420_486;
const CALCULATOR_SESSION_ID: &str = "361ecec4-00f7-4c94-a6e2-d91e28c5a06c";
const CALCULATOR_ENDPOINT: &str = "tcp://127.0.0.1:41681";
const SERVICE_DIRECTORY_ENDPOINT: &str = "tcp://127.0.0.1:9559";

/// The object UID of `dump_values.cpp`: bytes 0x01..0x14.
fn calculator_object_uid() -> object::Uid {
    object::Uid::from_bytes(std::array::from_fn(|index| index as u8 + 1))
}

/// Builds a `qi::service::Info` from the value libqi's `ServiceInfo` converts to, the fields of
/// the Rust type not being constructible from outside the crate.
fn service_info(
    name: &str,
    id: u32,
    endpoint: &str,
    session_id: &str,
    object_uid: Option<object::Uid>,
) -> service::Info {
    let object_uid = match object_uid {
        // Object UIDs are transmitted as strings of raw bytes.
        Some(uid) => QiString::from_maybe_utf8_owned(uid.bytes().to_vec()),
        None => QiString::Owned(String::new()),
    };
    let value = Value::Tuple(vec![
        name.to_owned().into_value(),
        id.into_value(),
        MACHINE_ID.to_owned().into_value(),
        PROCESS_ID.into_value(),
        vec![endpoint.to_owned()].into_value(),
        session_id.to_owned().into_value(),
        Value::String(object_uid),
    ]);
    service::Info::from_value(value).expect("service info from value")
}

fn calculator_info() -> service::Info {
    service_info(
        "Calculator",
        2,
        CALCULATOR_ENDPOINT,
        CALCULATOR_SESSION_ID,
        Some(calculator_object_uid()),
    )
}

fn service_directory_info() -> service::Info {
    service_info("ServiceDirectory", 1, SERVICE_DIRECTORY_ENDPOINT, "0", None)
}

// Capability maps
// ============================================================================

const AUTH_STATE_KEY: &str = "__qi_auth_state";
const AUTH_STATE_DONE: u32 = 3;

/// `{__qi_auth_state: 3u}`, the reply of a server to a successful authentication.
fn auth_state_done_map() -> KeyDynValueMap {
    KeyDynValueMap::from_iter([(AUTH_STATE_KEY.to_owned(), Value::UInt32(AUTH_STATE_DONE))])
}

/// The local capabilities merged with the authentication state, as `Server::sendAuthReply` does.
fn local_capabilities_with_auth_state_done() -> KeyDynValueMap {
    let mut map = Capabilities::LOCAL.to_map();
    map.set(AUTH_STATE_KEY, AUTH_STATE_DONE);
    map
}

// Statically typed checks
// ============================================================================

/// Checks that `value`, the Rust equivalent of the C++ value of the row, converts to exactly the
/// reference bytes through the conversions the `qi` crate uses on the wire, that the reference
/// bytes convert back into it, and that its static signature is the reference signature.
fn check<T>(row: &Row, value: T)
where
    T: Reflect + IntoValue<'static> + for<'a> FromValue<'a> + Clone + PartialEq + std::fmt::Debug,
{
    let name = &row.name;
    assert_eq!(
        T::signature().to_string(),
        row.signature,
        "{name}: the static signature differs from libqi"
    );
    let encoded = value
        .clone()
        .into_format()
        .unwrap_or_else(|err| panic!("{name}: serialization failed: {err}"));
    assert_eq!(
        encode_hex(&encoded),
        row.hex,
        "{name}: serialization differs from libqi"
    );
    let bytes = row.bytes();
    let decoded: T = bytes
        .to_reflect_value()
        .unwrap_or_else(|err| panic!("{name}: deserialization failed: {err}"));
    assert_eq!(
        decoded, value,
        "{name}: the deserialized value differs from the expected one"
    );
}

/// Capability maps are also (de)serialized directly with `serde`, as the payload of capability
/// messages: both paths must agree with libqi.
fn check_capability_map(row: &Row, map: KeyDynValueMap) {
    let name = &row.name;
    check(row, map.clone());
    let encoded =
        to_bytes(&map).unwrap_or_else(|err| panic!("{name}: serialization failed: {err}"));
    assert_eq!(
        encode_hex(&encoded),
        row.hex,
        "{name}: serde serialization differs from libqi"
    );
    let decoded: KeyDynValueMap = from_slice(&row.bytes())
        .unwrap_or_else(|err| panic!("{name}: serde deserialization failed: {err}"));
    assert_eq!(decoded, map, "{name}: the serde deserialized map differs");
}

fn check_calculator_info(row: &Row) {
    check(row, calculator_info());
    let info: service::Info = row.bytes().to_reflect_value().expect("service info");
    assert_eq!(info.name(), "Calculator");
    assert_eq!(info.id(), service::Id(2));
    assert_eq!(info.machine_id().to_string(), MACHINE_ID);
    assert_eq!(info.process_id(), PROCESS_ID);
    let endpoints: Vec<String> = info.endpoints().iter().map(ToString::to_string).collect();
    assert_eq!(endpoints, [CALCULATOR_ENDPOINT]);
    assert_eq!(info.node_uid().to_string(), CALCULATOR_SESSION_ID);
    assert_eq!(info.object_uid(), Some(calculator_object_uid()));
}

fn check_service_directory_info(row: &Row) {
    check(row, service_directory_info());
    let info: service::Info = row.bytes().to_reflect_value().expect("service info");
    assert_eq!(info.name(), "ServiceDirectory");
    assert_eq!(info.id(), service::Id(1));
    let endpoints: Vec<String> = info.endpoints().iter().map(ToString::to_string).collect();
    assert_eq!(endpoints, [SERVICE_DIRECTORY_ENDPOINT]);
    assert_eq!(info.node_uid().to_string(), "0");
    assert_eq!(info.object_uid(), None);
}

/// The six default capabilities of `qi::StreamContext::defaultCapabilities()` are the local
/// capabilities of this implementation, in the same (sorted) order.
fn check_default_capability_map(row: &Row) {
    let map = Capabilities::LOCAL.to_map();
    check_capability_map(row, map.clone());
    assert_eq!(Capabilities::from_map(&map), Capabilities::LOCAL);
    let keys: Vec<&str> = map.as_map().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "ClientServerSocket",
            "MessageFlags",
            "MetaObjectCache",
            "ObjectPtrUID",
            "RelativeEndpointURI",
            "RemoteCancelableCalls",
        ]
    );
    assert_eq!(map.get_as("MetaObjectCache"), Some(false));
}

type ValueCheck = fn(&Row);

/// The check of a value row, if this file has one for it.
fn value_case(name: &str) -> Option<ValueCheck> {
    let check: ValueCheck = match name {
        "serviceinfo_calculator" => check_calculator_info,
        "serviceinfo_servicedirectory_no_uid" => check_service_directory_info,
        "vec_serviceinfo_one" => |row| check(row, vec![calculator_info()]),
        "capabilitymap_default" => check_default_capability_map,
        "capabilitymap_auth_state_done" => |row| check_capability_map(row, auth_state_done_map()),
        _ => return None,
    };
    Some(check)
}

/// The value rows this file checks. Keep in sync with `DELEGATED_TO_QI_CRATE` in
/// `qi/tests/format_libqi_vectors.rs`.
const VALUE_ROWS_CHECKED_HERE: &[&str] = &[
    "serviceinfo_calculator",
    "serviceinfo_servicedirectory_no_uid",
    "vec_serviceinfo_one",
    "capabilitymap_default",
    "capabilitymap_auth_state_done",
];

// Messages
// ============================================================================

const HEADER_SIZE: usize = 28;

fn address(service: u32, object: u32, action: u32) -> Address {
    Address(
        service::Id(service),
        object::Id(object),
        object::ActionId(action),
    )
}

fn payload<T: serde::Serialize>(value: &T) -> bytes::Bytes {
    to_bytes(value).expect("payload serialization")
}

/// The message of a message row, if this file has one for it.
fn message_case(name: &str) -> Option<Message> {
    let message = match name {
        "msg_call_authenticate" => Message::Call {
            id: Id(1),
            address: address(0, 0, 8),
            payload: payload(&Capabilities::LOCAL.to_map()),
            flags: Flags::NONE,
        },
        "msg_reply_authenticate_state_only" => Message::Reply {
            id: Id(1),
            address: address(0, 0, 8),
            payload: payload(&auth_state_done_map()),
            flags: Flags::NONE,
        },
        "msg_reply_authenticate_with_caps" => Message::Reply {
            id: Id(1),
            address: address(0, 0, 8),
            payload: payload(&local_capabilities_with_auth_state_done()),
            flags: Flags::NONE,
        },
        "msg_error_boom" => Message::Error {
            id: Id(5),
            address: address(2, 1, 100),
            error: "boom".to_owned(),
        },
        "msg_cancel_call_7" => Message::Cancel {
            id: Id(9),
            address: address(2, 1, 0),
            call_id: Id(7),
        },
        "msg_canceled_call_7" => Message::Canceled {
            id: Id(7),
            address: address(2, 1, 100),
        },
        "msg_event_int32_42" => Message::Event {
            id: Id(11),
            address: address(2, 1, 100),
            payload: payload(&(42i32,)),
            flags: Flags::NONE,
        },
        "msg_call_add_1_2" => Message::Call {
            id: Id(3),
            address: address(2, 1, 100),
            payload: payload(&(1i32, 2i32)),
            flags: Flags::NONE,
        },
        "msg_reply_add_3" => Message::Reply {
            id: Id(3),
            address: address(2, 1, 100),
            payload: payload(&3i32),
            flags: Flags::NONE,
        },
        "msg_call_metaobject" => Message::Call {
            id: Id(2),
            address: address(2, 1, 2),
            payload: payload(&(0u32,)),
            flags: Flags::NONE,
        },
        "msg_call_registerevent" => Message::Call {
            id: Id(4),
            address: address(2, 1, 0),
            // registerEvent::(IIL): service, event, link = (event << 32) | counter
            payload: payload(&(2u32, 100u32, (100u64 << 32) | 2u64)),
            flags: Flags::NONE,
        },
        "msg_post_string_abc" => Message::Post {
            id: Id(6),
            address: address(2, 1, 101),
            payload: payload(&("abc",)),
            flags: Flags::NONE,
        },
        "msg_call_dynamic_payload" => Message::Call {
            id: Id(8),
            address: address(2, 1, 100),
            // The dynamic payload wraps the arguments tuple.
            payload: payload(&Dynamic((1i32, "a"))),
            flags: Flags::DYNAMIC_PAYLOAD,
        },
        "msg_capability_default" => Message::Capabilities {
            id: Id(10),
            address: address(0, 0, 0),
            capabilities: Capabilities::LOCAL.to_map(),
        },
        _ => return None,
    };
    Some(message)
}

fn message_type(type_name: &str) -> message::Type {
    match type_name {
        "Call" => message::Type::Call,
        "Reply" => message::Type::Reply,
        "Error" => message::Type::Error,
        "Post" => message::Type::Post,
        "Event" => message::Type::Event,
        "Capability" => message::Type::Capabilities,
        "Cancel" => message::Type::Cancel,
        "Canceled" => message::Type::Canceled,
        other => panic!("unknown message type name {other:?}"),
    }
}

/// The comparable content of a message.
#[derive(Debug, PartialEq)]
enum Payload {
    Bytes(bytes::Bytes, Flags),
    Error(String),
    Capabilities(KeyDynValueMap),
    Cancel(Id),
    None,
}

fn payload_of(message: &Message) -> Payload {
    match message {
        Message::Call { payload, flags, .. }
        | Message::Reply { payload, flags, .. }
        | Message::Post { payload, flags, .. }
        | Message::Event { payload, flags, .. } => Payload::Bytes(payload.clone(), *flags),
        Message::Error { error, .. } => Payload::Error(error.clone()),
        Message::Capabilities { capabilities, .. } => Payload::Capabilities(capabilities.clone()),
        Message::Cancel { call_id, .. } => Payload::Cancel(*call_id),
        Message::Canceled { .. } => Payload::None,
    }
}

fn flags_of(message: &Message) -> Flags {
    match payload_of(message) {
        Payload::Bytes(_, flags) => flags,
        _ => Flags::NONE,
    }
}

fn encode(message: Message) -> BytesMut {
    let mut buffer = BytesMut::new();
    Encoder
        .encode(message, &mut buffer)
        .expect("message encoding");
    buffer
}

fn decode(bytes: &[u8]) -> Message {
    let mut buffer = BytesMut::from(bytes);
    let message = Decoder::default()
        .decode(&mut buffer)
        .expect("message decoding")
        .expect("complete message");
    assert!(buffer.is_empty(), "trailing bytes after the message");
    message
}

/// Checks that `built`, the Rust equivalent of the C++ message of the row, encodes to exactly the
/// reference frame (header and payload), and that the reference frame decodes to the same message.
fn check_message(row: &Row, built: &Message) {
    let name = &row.name;
    let frame = row.bytes();
    let payload_hex = row.payload_hex.as_deref().unwrap_or_default();
    let payload_size = row.header_field("payload_size", row.payload_size);
    assert_eq!(
        frame.len(),
        HEADER_SIZE + payload_size,
        "{name}: inconsistent row"
    );
    assert!(row.hex.ends_with(payload_hex), "{name}: inconsistent row");

    assert_eq!(
        encode_hex(&encode(built.clone())),
        row.hex,
        "{name}: encoding differs from libqi"
    );

    let decoded = decode(&frame);
    assert_eq!(
        decoded.id(),
        Id(row.header_field("id", row.id)),
        "{name}: decoded id"
    );
    let type_name = row.type_name.as_deref().expect("type_name");
    assert_eq!(
        decoded.ty(),
        message_type(type_name),
        "{name}: decoded type"
    );
    assert_eq!(
        flags_of(&decoded).bits(),
        row.header_field("flags", row.flags),
        "{name}: decoded flags"
    );
    assert_eq!(
        decoded.address(),
        address(
            row.header_field("service", row.service),
            row.header_field("object", row.object),
            row.header_field("action", row.action),
        ),
        "{name}: decoded address"
    );
    assert_eq!(
        payload_of(&decoded),
        payload_of(built),
        "{name}: decoded payload"
    );
    assert_eq!(
        encode_hex(&encode(decoded)),
        row.hex,
        "{name}: re-encoding the decoded message differs from libqi"
    );
}

// Tests
// ============================================================================

#[test]
fn every_message_row_and_delegated_value_row_is_covered() {
    let rows = load_rows();
    let mut problems = Vec::new();
    for row in &rows {
        let name = row.name.as_str();
        let has_value_case = value_case(name).is_some();
        let has_message_case = message_case(name).is_some();
        if row.is_message() {
            if !has_message_case {
                problems.push(format!("{name}: message row without a test case"));
            }
            if has_value_case {
                problems.push(format!("{name}: message row with a value test case"));
            }
        } else {
            if has_message_case {
                problems.push(format!("{name}: value row with a message test case"));
            }
            if has_value_case != VALUE_ROWS_CHECKED_HERE.contains(&name) {
                problems.push(format!(
                    "{name}: value case presence does not match VALUE_ROWS_CHECKED_HERE"
                ));
            }
        }
    }
    for name in VALUE_ROWS_CHECKED_HERE {
        if !rows.iter().any(|row| row.name == *name) {
            problems.push(format!("{name}: row missing from the fixture"));
        }
    }
    assert!(
        problems.is_empty(),
        "fixture coverage problems: {problems:#?}"
    );
}

#[test]
fn value_rows_match_libqi() {
    for row in load_rows().iter().filter(|row| !row.is_message()) {
        if let Some(check) = value_case(&row.name) {
            check(row);
        }
    }
}

#[test]
fn message_rows_match_libqi() {
    let rows = load_rows();
    let message_rows: Vec<_> = rows.iter().filter(|row| row.is_message()).collect();
    assert!(!message_rows.is_empty());
    for row in message_rows {
        if let Some(message) = message_case(&row.name) {
            check_message(row, &message);
        }
    }
}

/// The first message a libqi client sends carries exactly the local capabilities map.
#[test]
fn local_capabilities_are_the_libqi_authenticate_payload() {
    let rows = load_rows();
    let row = rows
        .iter()
        .find(|row| row.name == "msg_call_authenticate")
        .expect("msg_call_authenticate row");
    let payload_hex = row.payload_hex.as_deref().expect("payload_hex");
    assert_eq!(
        encode_hex(&payload(&Capabilities::LOCAL.to_map())),
        payload_hex
    );
    let decoded: KeyDynValueMap = from_slice(&decode_hex(payload_hex)).expect("capability map");
    assert_eq!(Capabilities::from_map(&decoded), Capabilities::LOCAL);
}
