---
title: Support mutual TLS authentication (tcpsm://)
labels: enhancement
---

## Gap

`qi::messaging` serves and connects to `tcps://` endpoints with libqi's semantics (no certificate verification, self-signed server certificate unless `QI_TLS_CERTIFICATE` / `QI_TLS_PRIVATE_KEY` name PEM files). The `tcpsm://` scheme, where both peers present a certificate and verify the other's against a trusted CA, is not implemented: parsing an address with that scheme fails.

## Reference behavior

In libqi 4.x, `tcpsm` is handled by `qi::ssl` in `src/messaging/tcpmessagesocket.hpp` and `src/messaging/ssl/`: the server requires a client certificate, both sides verify against the CA configured through `qi::ssl::ClientConfig` / `ServerConfig`, and the connection fails otherwise.

## Work

- Add a `Tcpsm` scheme to `Address` and to the connector in `qi/src/messaging/channel/tls.rs`.
- Expose a TLS configuration on `node::init()` (CA bundle, client certificate and key) instead of environment variables only.
- Verify the peer certificate with `rustls` when the scheme requires it.
- Test with the C++ harness in `interop/cpp` once `qi-cpp-client` accepts a `tcpsm` URL and certificates.

Where noted: `docs/architecture.md`, "Limitations".
