# Examples for loong-actor

These examples are small, executable guides to `loong-actor`.
Start with [`sync_handler`](sync_handler.rs).

Run an example from the workspace root:

```console
cargo run -p loong-actor --example sync_handler
```

Replace `sync_handler` with any target listed below.
Most examples print nothing.
Their assertions check the demonstrated behavior.

## Message handling

| Example | Focus |
| --- | --- |
| [`sync_handler`](sync_handler.rs) | Immediate replies, one-way messages, and root shutdown. |

Prefer `SyncHandler<M>` for an immediate reply.
It is equivalent to returning `.ready()` from `Handler<M>`.

## Reply scheduling

A reply strategy controls actor progress after handler dispatch.

| Example | Focus |
| --- | --- |
| [`explicit_replies`](replies/explicit_replies.rs) | Select ready or owned work at runtime. |
| [`interleaved_reply`](replies/interleaved_reply.rs) | Let mailbox work progress between actor-aware polls. |
| [`exclusive_reply`](replies/exclusive_reply.rs) | Pause mailbox work until actor-aware work completes. |

Use `SyncHandler` when the reply is already available.
Return a bare `Future` for independent async work.
Use `interleaved` for cooperative actor-aware work.
Use `exclusive` when that work requires actor isolation.

The bare `interleaved` option uses a fixed limit of 32.
Dynamic options allow `with_max_in_flight` per spawn.
Omitting the option provides no capability or reply queue.
Unbounded interleaving can retain arbitrarily many active replies.
Exclusive replies need no interleaving capability.

## Actor topology

Lifecycle ownership and message addresses form different graphs.

| Example | Focus |
| --- | --- |
| [`top_level_actors`](topology/top_level_actors.rs) | Own independent roots and drop one owner. |
| [`child_actors`](topology/child_actors.rs) | Own child actors and gather their replies. |
| [`address_cycle`](topology/address_cycle.rs) | Build an address cycle during actor initialization. |

An address cycle does not create lifecycle ownership.
Cyclic calls can still wait forever.
