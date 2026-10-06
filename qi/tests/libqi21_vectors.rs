//! Tests against the serialization vectors of the `libqi` of NAOqi 2.1 (2014),
//! `interop/vectors/libqi-2.1-values.jsonl`, produced by `interop/cpp21/qi-dump-values`.
//!
//! The rows shared with the 4.0.5 fixture must be byte-identical, except the documented
//! differences (six-field service infos, the capability set); the rows specific to 2.1 (service
//! infos, the error a 2.1 server answers to the authentication call, the capabilities message,
//! object references without UID) must decode as this crate expects them from a legacy peer.

use bytes::BytesMut;
use qi::{
    format::{from_slice, to_bytes, SliceDeserializer},
    messaging::{codec::Decoder, message::Address, Message},
    object::MetaObject,
    service,
    value::{object::ObjectSeed, value::de::ValueType, FromValue, Signature},
    Capabilities,
};
use serde::de::DeserializeSeed;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
use tokio_util::codec::Decoder as _;

#[derive(serde::Deserialize, Debug)]
struct Row {
    name: String,
    #[serde(default)]
    signature: String,
    hex: String,
}

fn decode_hex(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex"))
        .collect()
}

fn load(name: &str) -> HashMap<String, Row> {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../interop/vectors")
        .join(name);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()));
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let row: Row = serde_json::from_str(line)
                .unwrap_or_else(|err| panic!("invalid fixture line {line:?}: {err}"));
            (row.name.clone(), row)
        })
        .collect()
}

/// The rows whose bytes differ between the two generations, for documented reasons.
const DIFFERENT_ROWS: &[&str] = &[
    "serviceinfo_calculator",
    "serviceinfo_servicedirectory_no_uid",
    "vec_serviceinfo_one",
    "capabilitymap_default",
    "msg_capability_default",
];

#[test]
fn shared_rows_are_byte_identical_with_libqi_405() {
    let old = load("libqi-4.0.5-values.jsonl");
    let new = load("libqi-2.1-values.jsonl");
    let mut shared = 0;
    for (name, row) in &new {
        let Some(reference) = old.get(name) else {
            continue;
        };
        shared += 1;
        if DIFFERENT_ROWS.contains(&name.as_str()) {
            assert_ne!(row.hex, reference.hex, "{name}: expected to differ");
        } else {
            assert_eq!(row.signature, reference.signature, "{name}: signature");
            assert_eq!(row.hex, reference.hex, "{name}: bytes");
        }
    }
    assert!(shared >= 50, "only {shared} shared rows");
}

/// Service infos of 2.1 have six fields and convert to `service::Info` without object UID; they
/// re-encode identically.
#[test]
fn service_infos_without_object_uid_decode() {
    let rows = load("libqi-2.1-values.jsonl");
    let row = &rows["serviceinfo_calculator"];
    assert_eq!(
        row.signature,
        "(sIsI[s]s)<ServiceInfo,name,serviceId,machineId,processId,endpoints,sessionId>"
    );
    let ty = row.signature.parse::<Signature>().unwrap().into_type();
    let bytes = decode_hex(&row.hex);
    let value = ValueType::new(ty.as_ref())
        .deserialize(SliceDeserializer::new(&bytes))
        .unwrap();
    assert_eq!(to_bytes(&value).unwrap(), bytes);
    let info = service::Info::from_value(value).unwrap();
    assert_eq!(info.name(), "Calculator");
    assert_eq!(info.id(), service::Id(2));
    assert_eq!(info.process_id(), 3420486);
    assert_eq!(info.endpoints().len(), 1);
    assert_eq!(info.object_uid(), None);

    let row = &rows["vec_serviceinfo_one"];
    let ty = row.signature.parse::<Signature>().unwrap().into_type();
    let bytes = decode_hex(&row.hex);
    let value = ValueType::new(ty.as_ref())
        .deserialize(SliceDeserializer::new(&bytes))
        .unwrap();
    let infos: Vec<service::Info> = value.cast_into().unwrap();
    assert_eq!(infos[0].name(), "Calculator");
}

/// The capabilities a 2.1 process advertises, and what is shared with it.
#[test]
fn legacy_capabilities_decode() {
    let rows = load("libqi-2.1-values.jsonl");
    let map: qi::value::KeyDynValueMap =
        from_slice(&decode_hex(&rows["capabilitymap_default"].hex)).unwrap();
    let advertised = Capabilities::from_map(&map);
    assert!(advertised.client_server_socket);
    assert!(advertised.message_flags);
    assert!(advertised.meta_object_cache);
    assert!(!advertised.remote_cancelable_calls);
    assert!(!advertised.object_ptr_uid);
    assert!(!advertised.relative_endpoint_uri);
    let shared = Capabilities::shared_with_remote_map(&map);
    assert_eq!(shared, Capabilities::LEGACY_LOCAL);
}

/// The messages specific to the legacy handshake decode: the capabilities message both ends
/// send on connection, and the error a 2.1 server answers to the authentication call.
#[test]
fn legacy_messages_decode() {
    let rows = load("libqi-2.1-values.jsonl");
    let mut decoder = Decoder::default();
    let mut buffer = BytesMut::from(&decode_hex(&rows["msg_capability_default"].hex)[..]);
    let message = decoder.decode(&mut buffer).unwrap().unwrap();
    match message {
        Message::Capabilities {
            address,
            capabilities,
            ..
        } => {
            assert_eq!(address, Address::default());
            assert_eq!(capabilities.as_map().len(), 3);
        }
        other => panic!("unexpected message {other:?}"),
    }
    let mut buffer = BytesMut::from(&decode_hex(&rows["msg_error_cant_find_service"].hex)[..]);
    let message = decoder.decode(&mut buffer).unwrap().unwrap();
    match message {
        Message::Error { id, address, error } => {
            assert_eq!(u32::from(id), 1);
            assert_eq!(
                address,
                Address(service::Id(0), qi::object::Id(0), qi::object::ActionId(8))
            );
            assert_eq!(error, "can't find service, address: {0.0.8, id:1}");
        }
        other => panic!("unexpected message {other:?}"),
    }
}

/// Object references of 2.1 (without the meta object cache) are the plain form this crate
/// decodes from peers without the `ObjectPtrUID` capability.
#[test]
fn plain_object_references_decode() {
    let rows = load("libqi-2.1-values.jsonl");
    let bytes = decode_hex(&rows["objectref_plain"].hex);
    let reference = ObjectSeed { uid_on_wire: false }
        .deserialize(SliceDeserializer::new(&bytes))
        .unwrap();
    assert_eq!(reference.service_id, service::Id(2));
    assert_eq!(reference.object_id, qi::object::Id(3));
    let meta: MetaObject = from_slice(&decode_hex(&rows["metaobject_full_object"].hex)).unwrap();
    assert_eq!(reference.meta_object, meta);
    assert!(reference
        .meta_object
        .methods
        .values()
        .any(|m| m.name == "add"));
}
