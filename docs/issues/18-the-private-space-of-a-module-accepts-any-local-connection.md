---
title: The private space of a module accepts any local connection
labels: enhancement
---

A module serves its members to its host from a space on `tcp://127.0.0.1` (ephemeral port), see
`docs/modules.md`. Any process of the machine that finds the port can call them, as any process
can reach the services of NAOqi on its loopback endpoint. Hosts of libqi 2.3 and later and
`qi-launch` could authenticate with a random token given through `ModuleV1::set_option`
(`UserTokenAuthenticator` on the module side, a client authenticator in the C++ shim); libqi 2.1
has no authentication.
