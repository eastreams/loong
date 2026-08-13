# Examples for loac

These examples are small, executable guides to `loac`.
Start with [`sync_handler`](sync_handler.rs).

Run an example from the workspace root:

```console
cargo run -p loac --example sync_handler
```

Replace `sync_handler` with any target listed below.
Most examples print nothing.
Their assertions check the demonstrated behavior.
`streaming` prints receive times to show items arriving during production.

## Message handling

| Example | Focus |
| --- | --- |
| [`sync_handler`](sync_handler.rs) | Immediate replies, one-way messages, and root shutdown. |

Prefer `SyncHandler<M>` for an immediate reply.
It is equivalent to returning `.ready()` from `Handler<M>`.

## Streaming

A provider actor produces a stream for each subscriber.

| Example | Focus |
| --- | --- |
| [`streaming`](streaming.rs) | A one-way subscribe whose reply task produces the items. |

The subscriber owns the receiver and passes the sender as message data.
The subscribe handler returns a future: the runtime tracks it as an owned task,
and it produces items into the caller's channel.
One-way `send` admits the subscription without waiting for production, so the
caller reads while the provider still produces.
The stream ends when the production task finishes or a send fails.
Stop and Drain wait for a running production task; Kill cancels it.

## Actor configuration

Each capability selects its limit profile independently.

| Example | Focus |
| --- | --- |
| [`actor_configuration`](configuration/actor_configuration.rs) | Mixed limit profiles, one spawn override, and dispatch budget. |
| [`const_generic_configuration`](configuration/const_generic_configuration.rs) | Reuse a const generic as a fixed limit and dynamic default. |

`SpawnOptions` changes one actor spawn.
Dynamic profiles expose per-spawn overrides.
Unbounded removes only the selected finite limit.
Applications remain responsible for resource growth.
`mailbox_budget` belongs to the actor type.

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

Topology examples select `children = unbounded`.
This enables `spawn_child` without a finite limit.
Its error is `Infallible`.
The examples destructure `Ok` without panicking.

An address cycle does not create lifecycle ownership.
Cyclic calls can still wait forever.
