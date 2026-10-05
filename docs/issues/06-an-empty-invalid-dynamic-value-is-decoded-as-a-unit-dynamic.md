---
title: An empty (invalid) dynamic value is decoded as a unit dynamic
labels: bug
---

## Problem

libqi serializes an invalid `AnyValue` (default constructed, no type) as a dynamic with an empty signature: `00000000`. `qi::format` decodes it as `Value::Dynamic(Value::Unit)` and re-encodes it as the unit dynamic `0100000076` (`"v"`). The bytes differ and the distinction between "no value" and "unit" is lost.

Pinned as a known deviation in `qi/tests/format_libqi_vectors.rs` (`KNOWN_VALUE_ROUNDTRIP_DEVIATIONS`, row `dynamic_empty`).

## Impact

Low. libqi treats both as "nothing" in most conversions. It matters for byte-identical relaying (gateways, recording tools) and for code that checks `AnyValue::isValid()` on the receiving side.

## Fix

Add a `Value::Invalid` variant (or make `Value::Dynamic` hold an `Option`), serialize it as an empty signature with no payload, and decide what `FromValue` implementations do with it (an error, as libqi's `to<T>()` throws). Remove the pinned deviation.
