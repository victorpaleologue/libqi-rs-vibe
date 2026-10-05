---
title: Value model loses structure annotations inside dynamic values
labels: bug
---

## Problem

`qi_value::Value::Tuple` carries no structure name or field names. A structure that travels *inside a dynamic value* through the runtime representation is therefore written back with the anonymous signature: a libqi `Point2D` in a dynamic comes in as `(ii)<Point2D,x,y>` and goes out as `(ii)`.

Statically typed dynamics (`Dynamic<Point2D>`) keep their annotations, and every other fixture is byte-identical. The deviation is pinned in `qi/tests/format_libqi_vectors.rs` (`KNOWN_VALUE_ROUNDTRIP_DEVIATIONS`, row `dynamic_point2d`) so that it is noticed when it changes.

## Impact

libqi clients converting such a value to their struct type still succeed because libqi's conversion is structural, but tools that display signatures, and meta objects relayed through `Value`, show anonymous tuples: `qi-cli info` on a relayed object shows `(ii)` where `qicli` shows `Point2D`.

## Fix

Give `Value::Tuple` an optional annotation (`Option<TupleAnnotation { name, fields }>`) mirroring `Type::Tuple(Tuple::Struct { .. })`, propagate it in the serializer and deserializer of dynamics in `qi::format`, and in `Value::convert_to`. Remove the pinned deviation once the round trip is identical.

Where noted: `docs/architecture.md`, "Limitations".
