---
title: Posts are fire-and-forget: nothing flushes them before a process exits
labels: bug
---

## Problem

`Object::post` (`qi/src/object.rs`) and `Session::post` (`qi/src/session.rs`) hand the message to the endpoint's outgoing queue and return without waiting for the bytes to reach the socket. A program that posts and then drops its node (or returns from `main`) can lose the post: the connection closes while the message still sits in the queue.

libqi has the same fire-and-forget semantics for `post`, but `Session::close` and the destructor drain the socket's write queue before shutting down, so the usual "post then exit" pattern works in C++.

## Reproduce

```rust
let tts = node.service("ALTextToSpeech").await?;
tts.post("say", ("hello",)).await?;
// program ends here: the robot sometimes says nothing
```

## Fix

- Make `Node::stop` and the drop of the last handle flush pending outgoing messages on every session (bounded by a timeout) before closing channels.
- Optionally return a future from `post` that resolves once the message is written, for callers that want to wait explicitly.
- Add a test in `qi/tests/space.rs`: post then drop the client node immediately; the service must receive the post.
