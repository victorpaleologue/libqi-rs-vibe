---
title: NAOqi 2.1 compatibility: what is not covered yet
labels: documentation
---

The legacy protocol of NAOqi 2.1 is detected and supported (see `docs/architecture.md`, "Protocol
variants", and `interop/cpp21/README.md`), with these known gaps:

1. **No real robot.** The NAOqi 2.1 SDK is no longer downloadable, so the validation uses the
   `libqi` of 2014 built from source and an ALMemory-like service of the harness. The
   `ALMemory.subscriber` pattern of a real `naoqi-bin` 2.1 (C++ modules bridged to qi objects)
   has not been exercised; if the serialization of its objects turns out to differ from the plain
   form `metaObject, serviceId, objectId`, it will show up as a decoding error of the reply of
   `subscriber`.
2. **The `MetaObjectCache` form of object references is not decoded.** Two 2.1 processes share
   that capability and transmit `bool transmit, [metaObject], uint32 cacheId, serviceId,
   objectId`; this implementation never advertises the cache, so a conforming peer never sends
   that form. A peer that ignored the negotiation would not be understood.
3. **The six-field `ServiceInfo` is selected by the `ObjectPtrUID` capability.** The `objectUid`
   field appeared in `libqi` 2.9 (2020), two years after the capability, so peers of `libqi`
   2.3 to 2.8 (NAOqi 2.3 to 2.8), which have neither, get the form they expect; a 2.9+ peer that
   disabled `ObjectPtrUID` with `QI_TRANSPORT_CAPABILITIES` gets six fields too, which it accepts.
   Only `libqi` 4.0.5 and 2.1 were tested.
4. **Subscriptions to property changes do not work with 2.1 peers**: `registerEvent` of `libqi`
   2.1 only knows signals ("No such signal"), on its server and its client side alike.
5. **Members with optional (`+`) or variadic (`#`) signatures are hidden from legacy peers**
   rather than adapted, as 2.1 cannot parse them at all.
