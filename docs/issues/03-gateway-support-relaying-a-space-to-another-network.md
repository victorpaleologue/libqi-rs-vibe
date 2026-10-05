---
title: Gateway support (relaying a space to another network)
labels: enhancement
---

## Gap

libqi's `Gateway` (`src/messaging/gateway.cpp`) relays a service directory and its services to clients on another network, rewriting service ids and endpoints and authenticating the outer clients. NAOqi 2.9 robots run one on the public interface: `tcps://<robot>:9503` is served by the gateway while the services listen on localhost.

`qi` connects to a gateway fine as a client (that is how a Rust client reaches a 2.9 robot) but cannot act as one: there is no way to expose a space through another node with id remapping.

## Work

- A gateway node mode: connect to an upstream space, mirror its `ServiceDirectory` signals (`serviceAdded`, `serviceRemoved`) into a locally hosted directory, relay calls, posts and events per client session with service-id and object-id translation.
- Authentication of outer clients independently of the upstream space.
- Test with the C++ harness: a C++ client reaching a C++ service through a Rust gateway.

Where noted: `docs/architecture.md`, "Limitations"; `README.md`.
