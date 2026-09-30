//! Byte-identity tests against the serialization vectors of the reference C++ implementation of
//! the `qi` framework (libqi 4.0.5), `interop/vectors/libqi-4.0.5-values.jsonl`.
//!
//! The vectors are produced by `interop/cpp/dump_values.cpp` and documented in
//! `interop/cpp/README.md`. Every *value* row of the fixture is checked in three ways:
//!
//! 1. the statically typed Rust equivalent of the C++ value serializes to exactly the reference
//!    bytes, and its static (`Reflect`) signature is the reference signature;
//! 2. the reference bytes deserialize back into that Rust value;
//! 3. the reference bytes deserialize into a dynamic [`Value`], driven by the type parsed from the
//!    reference signature, and that value serializes back to the same bytes.
//!
//! Service infos, capability maps and messages need the `qi` crate and are checked in
//! `qi/tests/libqi_vectors.rs`. A coverage test makes sure that every row of the fixture is
//! handled by exactly one of the two files, so that new rows cannot be silently ignored.

use bytes::Bytes;
use qi::format::{from_slice, to_bytes, SliceDeserializer};
use qi::value::{
    de::ValueType,
    object::{ActionId, MetaMethod, MetaObject, MetaProperty, MetaSignal},
    Dynamic, IntoValue, Map, Reflect, Signature, Type, Value,
};
use serde::de::DeserializeSeed;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

// Fixture
// ============================================================================

/// A line of the fixture. Message rows carry more fields, which are read by the `qi` crate tests.
#[derive(serde::Deserialize, Debug)]
struct Row {
    name: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    signature: String,
    hex: String,
}

impl Row {
    fn is_message(&self) -> bool {
        self.kind.as_deref() == Some("message")
    }

    fn bytes(&self) -> Vec<u8> {
        decode_hex(&self.hex)
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

fn sig(signature: &str) -> Signature {
    signature
        .parse()
        .unwrap_or_else(|err| panic!("invalid signature {signature:?}: {err}"))
}

// Registered structures of `interop/cpp/interop_types.hpp`
// ============================================================================

#[derive(
    Debug,
    PartialEq,
    serde::Serialize,
    serde::Deserialize,
    qi::Reflect,
    qi::ToValue,
    qi::IntoValue,
    qi::FromValue,
)]
#[qi(value(crate = "qi::value"))]
struct Point2D {
    x: i32,
    y: i32,
}

#[derive(
    Debug,
    PartialEq,
    serde::Serialize,
    serde::Deserialize,
    qi::Reflect,
    qi::ToValue,
    qi::IntoValue,
    qi::FromValue,
)]
#[qi(value(crate = "qi::value"))]
struct TimeStamp {
    i: i32,
    j: i32,
}

#[derive(
    Debug,
    PartialEq,
    serde::Serialize,
    serde::Deserialize,
    qi::Reflect,
    qi::ToValue,
    qi::IntoValue,
    qi::FromValue,
)]
#[qi(value(crate = "qi::value"))]
struct TimeStampedPoint2D {
    p: Point2D,
    t: TimeStamp,
}

/// `qi::os::timeval`, registered by libqi as `(ll)<timeval,tv_sec,tv_usec>`.
#[derive(
    Debug,
    PartialEq,
    serde::Serialize,
    serde::Deserialize,
    qi::Reflect,
    qi::ToValue,
    qi::IntoValue,
    qi::FromValue,
)]
#[qi(value(crate = "qi::value", name = "timeval"))]
struct Timeval {
    tv_sec: i64,
    tv_usec: i64,
}

// Meta objects
// ============================================================================

const ADD_DESCRIPTION: &str = "Adds one to its argument";
const OBJECT_DESCRIPTION: &str = "Interop test object";

/// Return signature of `Manageable::stats`.
const STATS_SIGNATURE: &str = "{I(I(fff)<MinMaxSum,minValue,maxValue,cumulatedValue>\
                               (fff)<MinMaxSum,minValue,maxValue,cumulatedValue>\
                               (fff)<MinMaxSum,minValue,maxValue,cumulatedValue>)\
                               <MethodStatistics,count,wall,user,system>}";

/// Signature of the `Manageable::traceObject` signal.
const TRACE_OBJECT_SIGNATURE: &str = "((IiIm(ll)<timeval,tv_sec,tv_usec>llII)\
                                      <EventTrace,id,kind,slotId,arguments,timestamp,\
                                      userUsTime,systemUsTime,callerContext,calleeContext>)";

/// A method advertised without parameter documentation, as `DynamicObjectBuilder` does.
fn meta_method(
    uid: u32,
    name: &str,
    parameters_signature: &str,
    return_signature: &str,
    description: &str,
) -> (ActionId, MetaMethod) {
    (
        ActionId(uid),
        MetaMethod {
            uid: ActionId(uid),
            return_signature: sig(return_signature),
            name: name.to_owned(),
            parameters_signature: sig(parameters_signature),
            description: description.to_owned(),
            parameters: Vec::new(),
            return_description: String::new(),
        },
    )
}

fn meta_signal(uid: u32, name: &str, signature: &str) -> (ActionId, MetaSignal) {
    (
        ActionId(uid),
        MetaSignal {
            uid: ActionId(uid),
            name: name.to_owned(),
            signature: sig(signature),
        },
    )
}

fn meta_property(uid: u32, name: &str, signature: &str) -> (ActionId, MetaProperty) {
    (
        ActionId(uid),
        MetaProperty {
            uid: ActionId(uid),
            name: name.to_owned(),
            signature: sig(signature),
        },
    )
}

/// The meta object of the `DynamicObjectBuilder` of `dump_values.cpp`, before `object()`: user
/// members start at 100 and the description is the one set on the builder.
fn metaobject_builder_raw() -> MetaObject {
    MetaObject {
        methods: BTreeMap::from([meta_method(100, "add", "(i)", "i", ADD_DESCRIPTION)]),
        signals: BTreeMap::from([
            meta_signal(101, "fire", "(i)"),
            meta_signal(102, "val", "(i)"),
        ]),
        properties: BTreeMap::from([meta_property(102, "val", "i")]),
        description: OBJECT_DESCRIPTION.to_owned(),
    }
}

/// The same object after `object()`: merged with the `Manageable` members 80..86, whose (empty)
/// description replaces the one of the builder.
fn metaobject_full_object() -> MetaObject {
    MetaObject {
        methods: BTreeMap::from([
            meta_method(80, "isStatsEnabled", "()", "b", ""),
            meta_method(81, "enableStats", "(b)", "v", ""),
            meta_method(82, "stats", "()", STATS_SIGNATURE, ""),
            meta_method(83, "clearStats", "()", "v", ""),
            meta_method(84, "isTraceEnabled", "()", "b", ""),
            meta_method(85, "enableTrace", "(b)", "v", ""),
            meta_method(100, "add", "(i)", "i", ADD_DESCRIPTION),
        ]),
        signals: BTreeMap::from([
            meta_signal(86, "traceObject", TRACE_OBJECT_SIGNATURE),
            meta_signal(101, "fire", "(i)"),
            meta_signal(102, "val", "(i)"),
        ]),
        properties: BTreeMap::from([meta_property(102, "val", "i")]),
        description: String::new(),
    }
}

// Statically typed checks
// ============================================================================

/// Checks that `value`, the statically typed Rust equivalent of the C++ value of the row,
/// serializes to the reference bytes, that the reference bytes deserialize back into it, and that
/// its static signature is the reference signature.
fn check<'b, T>(row: &Row, bytes: &'b [u8], value: T)
where
    T: serde::Serialize + serde::Deserialize<'b> + Reflect + PartialEq + std::fmt::Debug,
{
    let name = &row.name;
    assert_eq!(
        T::signature().to_string(),
        row.signature,
        "{name}: the static signature differs from libqi"
    );
    let encoded =
        to_bytes(&value).unwrap_or_else(|err| panic!("{name}: serialization failed: {err}"));
    assert_eq!(
        encode_hex(&encoded),
        row.hex,
        "{name}: serialization differs from libqi"
    );
    let decoded: T =
        from_slice(bytes).unwrap_or_else(|err| panic!("{name}: deserialization failed: {err}"));
    assert_eq!(
        decoded, value,
        "{name}: the deserialized value differs from the expected one"
    );
}

/// libqi serializes an invalid (empty) `AnyValue` as an empty signature followed by no value at
/// all. The Rust type system has no invalid value: the empty dynamic is decoded as a unit. No Rust
/// value serializes to the empty form (a unit dynamic serializes as `"v"`, see `dynamic_void`), so
/// only the decoding direction is checked here.
fn check_dynamic_empty(row: &Row, bytes: &[u8]) {
    assert_eq!(row.signature, "m");
    assert_eq!(row.hex, "00000000");
    let decoded: Dynamic<()> =
        from_slice(bytes).unwrap_or_else(|err| panic!("{}: decoding failed: {err}", row.name));
    assert_eq!(decoded, Dynamic(()));
    let decoded: Dynamic<Value<'_>> =
        from_slice(bytes).unwrap_or_else(|err| panic!("{}: decoding failed: {err}", row.name));
    assert_eq!(decoded, Dynamic(Value::Unit));
}

fn raw(bytes: &'static [u8]) -> Bytes {
    Bytes::from_static(bytes)
}

type Check = fn(&Row, &[u8]);

/// The statically typed check of a value row, if this file has one for it.
fn value_case(name: &str) -> Option<Check> {
    let check: Check = match name {
        // primitives
        "bool_true" => |row, bytes| check(row, bytes, true),
        "bool_false" => |row, bytes| check(row, bytes, false),
        "int8_neg5" => |row, bytes| check(row, bytes, -5i8),
        "uint8_200" => |row, bytes| check(row, bytes, 200u8),
        "int16_neg1234" => |row, bytes| check(row, bytes, -1234i16),
        "uint16_60000" => |row, bytes| check(row, bytes, 60000u16),
        "int32_neg100000" => |row, bytes| check(row, bytes, -100_000i32),
        "uint32_3000000000" => |row, bytes| check(row, bytes, 3_000_000_000u32),
        "int64_neg5e12" => |row, bytes| check(row, bytes, -5_000_000_000_000i64),
        "uint64_1e19" => |row, bytes| check(row, bytes, 10_000_000_000_000_000_000u64),
        "float_1_5" => |row, bytes| check(row, bytes, 1.5f32),
        "double_neg2_25" => |row, bytes| check(row, bytes, -2.25f64),
        // strings
        "string_empty" => |row, bytes| check(row, bytes, String::new()),
        "string_abc" => |row, bytes| check(row, bytes, "abc".to_owned()),
        "string_hello_utf8" => |row, bytes| check(row, bytes, "héllo".to_owned()),
        // containers
        "vec_int32_1_2_3" => |row, bytes| check(row, bytes, vec![1i32, 2, 3]),
        "vec_string_a_bc" => |row, bytes| check(row, bytes, vec!["a".to_owned(), "bc".to_owned()]),
        "vec_int32_empty" => |row, bytes| check(row, bytes, Vec::<i32>::new()),
        "map_string_int32" => |row, bytes| {
            check(
                row,
                bytes,
                BTreeMap::from([("a".to_owned(), 1i32), ("b".to_owned(), 2i32)]),
            )
        },
        "map_int32_string" => {
            |row, bytes| check(row, bytes, BTreeMap::from([(1i32, "a".to_owned())]))
        }
        "tuple_int32_string" | "pair_int32_string" => {
            |row, bytes| check(row, bytes, (1i32, "a".to_owned()))
        }
        // registered structures
        "struct_point2d" => |row, bytes| check(row, bytes, Point2D { x: 4, y: 2 }),
        "struct_nested_timestamped_point2d" => |row, bytes| {
            check(
                row,
                bytes,
                TimeStampedPoint2D {
                    p: Point2D { x: 4, y: 2 },
                    t: TimeStamp { i: 3, j: 1 },
                },
            )
        },
        // dynamics
        "dynamic_int32_5" => |row, bytes| check(row, bytes, Dynamic(5i32)),
        "dynamic_string_abc" => |row, bytes| check(row, bytes, Dynamic("abc".to_owned())),
        "dynamic_vec_int32_1_2" => |row, bytes| check(row, bytes, Dynamic(vec![1i32, 2])),
        "dynamic_point2d" => |row, bytes| check(row, bytes, Dynamic(Point2D { x: 4, y: 2 })),
        "dynamic_empty" => check_dynamic_empty,
        "dynamic_void" => |row, bytes| check(row, bytes, Dynamic(())),
        "dynamic_bool_true" => |row, bytes| check(row, bytes, Dynamic(true)),
        "dynamic_double_1_5" => |row, bytes| check(row, bytes, Dynamic(1.5f64)),
        "dynamic_map_string_dynamic" => |row, bytes| {
            check(
                row,
                bytes,
                Dynamic(Value::Map(Map::from_iter([
                    ("i".into_value(), Value::Int32(1).into_dynamic()),
                    ("s".into_value(), "x".into_value().into_dynamic()),
                ]))),
            )
        },
        "vec_dynamic_int32_string" => |row, bytes| {
            check(
                row,
                bytes,
                vec![Dynamic(Value::Int32(1)), Dynamic("a".into_value())],
            )
        },
        // optionals
        "optional_int32_none" => |row, bytes| check(row, bytes, None::<i32>),
        "optional_int32_some_7" => |row, bytes| check(row, bytes, Some(7i32)),
        "optional_string_some_abc" => |row, bytes| check(row, bytes, Some("abc".to_owned())),
        // raw buffers
        "buffer_4_bytes" => |row, bytes| check(row, bytes, raw(&[0x2a, 0, 0, 0])),
        "buffer_empty" => |row, bytes| check(row, bytes, Bytes::new()),
        "tuple_int32_buffer_string" => {
            |row, bytes| check(row, bytes, (7i32, raw(&[0x2a, 0, 0, 0]), "z".to_owned()))
        }
        // misc registered types
        "signature_value_is" => |row, bytes| check(row, bytes, sig("(is)")),
        "os_timeval" => |row, bytes| {
            check(
                row,
                bytes,
                Timeval {
                    tv_sec: 1_700_000_000,
                    tv_usec: 123_456,
                },
            )
        },
        "url_tcp_localhost" => |row, bytes| check(row, bytes, "tcp://127.0.0.1:9559".to_owned()),
        // meta objects
        "metaobject_builder_raw" => |row, bytes| check(row, bytes, metaobject_builder_raw()),
        "metaobject_full_object" => |row, bytes| check(row, bytes, metaobject_full_object()),
        "metaobject_empty" => |row, bytes| check(row, bytes, MetaObject::default()),
        // object UID as a raw buffer
        "objectuid_raw_20_bytes" => |row, bytes| check(row, bytes, Bytes::from_iter(1u8..=20)),
        _ => return None,
    };
    Some(check)
}

/// Value rows checked by `qi/tests/libqi_vectors.rs`, which has access to the `qi` types they
/// map to (`qi::service::Info`, `qi::Capabilities`). Keep in sync with that file.
const DELEGATED_TO_QI_CRATE: &[&str] = &[
    "serviceinfo_calculator",
    "serviceinfo_servicedirectory_no_uid",
    "vec_serviceinfo_one",
    "capabilitymap_default",
    "capabilitymap_auth_state_done",
];

#[test]
fn every_fixture_row_is_covered_exactly_once() {
    let rows = load_rows();
    let mut problems = Vec::new();
    for row in &rows {
        let name = row.name.as_str();
        let has_case = value_case(name).is_some();
        let delegated = DELEGATED_TO_QI_CRATE.contains(&name);
        if row.is_message() {
            // Messages are all checked by the `qi` crate tests.
            if has_case || delegated {
                problems.push(format!("{name}: message row referenced by the value tests"));
            }
        } else {
            match (has_case, delegated) {
                (true, false) | (false, true) => {}
                (true, true) => problems.push(format!(
                    "{name}: checked here and delegated to the qi crate"
                )),
                (false, false) => problems.push(format!("{name}: no test case")),
            }
        }
    }
    for name in DELEGATED_TO_QI_CRATE {
        if !rows.iter().any(|row| row.name == *name) {
            problems.push(format!("{name}: delegated row missing from the fixture"));
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
            check(row, &row.bytes());
        }
    }
}

// Dynamic (`Value`) round trips
// ============================================================================

/// Rows whose re-serialization from a [`Value`] is known not to be byte-identical to the reference
/// bytes, with the bytes it produces instead. These are limitations of the `Value` model, pinned
/// here so that they are noticed if they change.
const KNOWN_VALUE_ROUNDTRIP_DEVIATIONS: &[(&str, &str)] = &[
    // libqi's empty (invalid) dynamic is decoded as a unit dynamic, which serializes as "v".
    ("dynamic_empty", "0100000076"),
    // `Value::Tuple` does not carry struct annotations: the dynamic Point2D is written back with
    // the anonymous signature "(ii)" instead of "(ii)<Point2D,x,y>". Statically typed dynamics
    // (`Dynamic<Point2D>`) keep the annotations, see `value_rows_match_libqi`.
    ("dynamic_point2d", "04000000286969290400000002000000"),
];

/// Deserializes the reference bytes into a [`Value`] of the type parsed from the reference
/// signature, and checks that the value serializes back to the same bytes.
fn check_value_roundtrip(row: &Row) {
    let name = &row.name;
    let bytes = row.bytes();
    let ty = sig(&row.signature).into_type();
    let value = ValueType::new(ty.as_ref())
        .deserialize(SliceDeserializer::new(&bytes))
        .unwrap_or_else(|err| panic!("{name}: deserialization as a Value failed: {err}"));
    let reencoded = to_bytes(&value)
        .unwrap_or_else(|err| panic!("{name}: serialization of the Value failed: {err}"));
    let reencoded = encode_hex(&reencoded);
    match KNOWN_VALUE_ROUNDTRIP_DEVIATIONS
        .iter()
        .find(|(deviating, _)| deviating == name)
    {
        Some((_, deviation)) => {
            assert_ne!(
                *deviation, row.hex,
                "{name}: the known deviation is no longer one, remove it"
            );
            assert_eq!(
                reencoded, *deviation,
                "{name}: the Value re-serialization changed"
            );
        }
        None => assert_eq!(
            reencoded, row.hex,
            "{name}: the Value round trip is not byte-identical (value: {value})"
        ),
    }
}

#[test]
fn value_rows_round_trip_through_values() {
    let rows = load_rows();
    for row in rows.iter().filter(|row| !row.is_message()) {
        check_value_roundtrip(row);
    }
    for (name, _) in KNOWN_VALUE_ROUNDTRIP_DEVIATIONS {
        assert!(
            rows.iter().any(|row| row.name == *name),
            "{name}: known deviation missing from the fixture"
        );
    }
}

/// For a dynamic row, reads the signature string and the value it types separately, and writes
/// the pair back. Unlike the [`Value`] round trip, this keeps the exact signature, annotations
/// included, and must be byte-identical for every dynamic row that carries a value.
fn check_split_dynamic_roundtrip(row: &Row) {
    let name = &row.name;
    let bytes = row.bytes();
    let mut deserializer = SliceDeserializer::new(&bytes);
    let signature: String = serde::Deserialize::deserialize(&mut deserializer)
        .unwrap_or_else(|err| panic!("{name}: reading the signature failed: {err}"));
    if signature.is_empty() {
        // The empty dynamic carries no value.
        assert_eq!(row.hex, "00000000", "{name}: empty signature with a value");
        return;
    }
    let parsed = sig(&signature);
    assert_eq!(
        parsed.to_string(),
        signature,
        "{name}: the signature does not survive a parse/print round trip"
    );
    let value = ValueType::new(parsed.as_type())
        .deserialize(&mut deserializer)
        .unwrap_or_else(|err| panic!("{name}: deserialization of the value failed: {err}"));
    let reencoded = to_bytes(&(&parsed, &value))
        .unwrap_or_else(|err| panic!("{name}: serialization failed: {err}"));
    assert_eq!(
        encode_hex(&reencoded),
        row.hex,
        "{name}: the (signature, value) round trip is not byte-identical"
    );
}

#[test]
fn dynamic_rows_keep_their_signature() {
    let rows = load_rows();
    let dynamic_rows: Vec<_> = rows.iter().filter(|row| row.signature == "m").collect();
    assert!(!dynamic_rows.is_empty());
    for row in dynamic_rows {
        check_split_dynamic_roundtrip(row);
    }
}

// Builders
// ============================================================================

/// The meta object builders of `qi::value` reproduce what `DynamicObjectBuilder` produces for the
/// same members (`metaobject_builder_raw`).
#[test]
fn meta_object_builders_reproduce_libqi_builder_output() {
    let rows = load_rows();
    let row = rows
        .iter()
        .find(|row| row.name == "metaobject_builder_raw")
        .expect("metaobject_builder_raw row");

    let mut builder = MetaObject::builder();
    builder.add_method({
        let mut method = MetaMethod::builder(ActionId(100));
        method.set_name("add");
        method.set_description(ADD_DESCRIPTION);
        method.parameter(0).set_type(Type::Int32);
        method.return_value().set_type(Type::Int32);
        method.build()
    });
    builder.add_signal(MetaSignal {
        uid: ActionId(101),
        name: "fire".to_owned(),
        signature: sig("(i)"),
    });
    builder.add_property(MetaProperty {
        uid: ActionId(102),
        name: "val".to_owned(),
        signature: sig("i"),
    });
    let mut meta_object = builder.build();
    meta_object.description = OBJECT_DESCRIPTION.to_owned();

    assert_eq!(meta_object, metaobject_builder_raw());
    assert_eq!(encode_hex(&to_bytes(&meta_object).unwrap()), row.hex);
}

/// Members are serialized sorted by identifier whatever the insertion order, like the `std::map`
/// of the reference implementation.
#[test]
fn meta_object_members_are_serialized_sorted_by_id() {
    let rows = load_rows();
    let row = rows
        .iter()
        .find(|row| row.name == "metaobject_full_object")
        .expect("metaobject_full_object row");

    let sorted = metaobject_full_object();
    let mut builder = MetaObject::builder();
    for method in sorted.methods.values().rev() {
        builder.add_method(method.clone());
    }
    for signal in sorted.signals.values().rev() {
        builder.add_signal(signal.clone());
    }
    for property in sorted.properties.values().rev() {
        builder.add_property(property.clone());
    }
    let reversed = builder.build();

    assert_eq!(reversed, sorted);
    assert_eq!(encode_hex(&to_bytes(&reversed).unwrap()), row.hex);
}
