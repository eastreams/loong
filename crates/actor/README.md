# loac

`loac` is a small runtime for typed, local actors. It combines Actix-style message/reply typing with bounded admission, explicit ownership, and a Ractor-style supervision tree.

The name joins `Loong` and `Actor` (`lo` + `ac`).

The crate is an early MVP with a deliberately narrow contract.

## Quick Start

```rust
use loac::{ExitReason, Shutdown, SubtreeStatus, prelude::*};

struct Counter(u64);

#[actor(mailbox, interleaved = unbounded)]
impl Actor for Counter {
    type SpawnArgs = u64;

    async fn init(value: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self(value)
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct Add(u64);

impl Handler<Add> for Counter {
    async fn handle(message: Add, mut cx: Cx<'_, Self>) -> u64 {
        cx.with(|actor, _| {
            actor.0 += message.0;
            actor.0
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owner = loac::spawn::<Counter>(0);

    assert_eq!(owner.call(Add(2)).await?, 2);
    assert_eq!(owner.call(Add(3)).await?, 5);
    let status = owner.shutdown(Shutdown::Drain).await;
    assert_eq!(status.reason(), ExitReason::Drained);
    assert_eq!(status.subtree(), SubtreeStatus::Terminated);
    Ok(())
}
```

See the [examples index](examples/README.md) for runnable guides.

## Core Model

| Type | Role |
| --- | --- |
| [`Actor`](https://docs.rs/loac/latest/loac/trait.Actor.html) | Owns state. Lifecycle hooks run serially. |
| [`ActorRef`](https://docs.rs/loac/latest/loac/struct.ActorRef.html) | Cloneable, typed, non-owning handle. |
| [`Recipient`](https://docs.rs/loac/latest/loac/trait.Recipient.html) | Erases the actor type for one message type. |
| [`ActorOwner`](https://docs.rs/loac/latest/loac/struct.ActorOwner.html) | Uniquely owns one root actor. |
| [`ActorScope`](https://docs.rs/loac/latest/loac/struct.ActorScope.html) | Exposes temporary capabilities during actor work. |

## Choose Capabilities

`#[actor(...)]` generates the runtime configuration.
Messaging, interleaved replies, and child ownership are opt-in.
Omitting an option removes that capability.
`mailbox`, `interleaved`, and `children` share the five forms below.
An explicit finite limit must be a nonzero `usize` constant.

| Form | Selected profile |
| --- | --- |
| bare `option` | Fixed limit of 32 |
| `option = N` | Fixed limit of N |
| `option = dynamic` | Per-spawn limit defaulting to 32 |
| `option = dynamic(N)` | Per-spawn limit defaulting to N |
| `option = unbounded` | No finite limit |

### Mailbox

Here `option` is `mailbox`.

Mailbox capacity bounds messages awaiting dispatch.
It does not bound active replies. [`call`](https://docs.rs/loac/latest/loac/struct.ActorRef.html#method.call) and [`send`](https://docs.rs/loac/latest/loac/struct.ActorRef.html#method.send) wait when full.
[`try_call`](https://docs.rs/loac/latest/loac/struct.ActorRef.html#method.try_call) and [`try_send`](https://docs.rs/loac/latest/loac/struct.ActorRef.html#method.try_send) return immediately.
Dynamic options expose [`with_mailbox_capacity`](https://docs.rs/loac/latest/loac/trait.DynamicMailboxOptions.html#tymethod.with_mailbox_capacity).

### Interleaved Replies

Here `option` is `interleaved`. It requires `mailbox`.

The limit counts active replies. A full limit pauses dispatch before another
handler starts. Exclusive replies remain available without `interleaved`.
Dynamic options expose [`with_max_in_flight`](https://docs.rs/loac/latest/loac/trait.DynamicInterleavingOptions.html#tymethod.with_max_in_flight).

### Child-Spawning

Here `option` is `children`.

The limit counts retained child registrations.
Finite profiles return the original spawn inputs on [`Full`](https://docs.rs/loac/latest/loac/supervision/struct.Full.html).
Unbounded profiles use `Infallible` as their error.
Dynamic options expose [`with_max_children`](https://docs.rs/loac/latest/loac/trait.DynamicChildrenOptions.html#tymethod.with_max_children).

See the [attribute reference](https://docs.rs/loac/latest/loac/attr.actor.html) for syntax and constraints.

## Message Shapes

`#[derive(Message)]` supports two message shapes:

| Attribute | Handler trait | Caller receives |
| --- | --- | --- |
| `#[message(reply = Type)]` | [`Handler`](https://docs.rs/loac/latest/loac/trait.Handler.html) or [`SyncHandler`](https://docs.rs/loac/latest/loac/trait.SyncHandler.html) | `Type` |
| `#[message(stream = Item, reply = Final)]` | [`StreamHandler`](https://docs.rs/loac/latest/loac/trait.StreamHandler.html) | `StreamReply<Item, Final>` |

**Use `#[message(reply = Type)]` and `Handler<M>` for ordinary asynchronous
request handling.** Declare the eventual result as `Type`, including
`Result<Value, Error>` when appropriate. The handler returns a future, and
`loac` schedules it on the interleaved lane.

Use `#[message(reply = Type)]`, `SyncHandler<M>`, and attach
`#[loac::sync_handler]` to the impl when the reply value is already complete
by the time the handler returns. `SyncHandler` receives `&mut self` directly,
needs no interleaving lane, and is the short way to write a ready reply.

Implement [`DispatchHandler`](https://docs.rs/loac/latest/loac/trait.DispatchHandler.html)
directly only when a handler must choose an explicit reply strategy — for
example, runtime branching between ready and an independently scheduled owned
future, exclusive execution, or another runtime choice. Explicit dispatch
changes handler implementation and scheduling, not the declared result type.

Omitting the `#[message(...)]` attribute entirely produces a send-only
message with unit output. Selecting either `reply` or `stream` implements
`HasReply` and makes the message callable with `ActorRef::call`; the caller
still receives `Result<M::Reply, CallError>`. Reply and final types default
to `()` when omitted. `SyncHandler` uses the `reply` shape and dispatches
through a concrete impl emitted by the `#[loac::sync_handler]` attribute
macro.

## Reply Modes

| Strategy | Selected by | Actor progress while the reply runs |
| --- | --- | --- |
| ready | [`value.ready()`](https://docs.rs/loac/latest/loac/trait.ReplyExt.html#method.ready) from a [`DispatchHandler`](https://docs.rs/loac/latest/loac/trait.DispatchHandler.html) or [`SyncHandler`](https://docs.rs/loac/latest/loac/trait.SyncHandler.html) | The reply is already complete during dispatch. |
| owned | A bare `Future` from a [`DispatchHandler`](https://docs.rs/loac/latest/loac/trait.DispatchHandler.html) | A Tokio task runs it beside all actor work. |
| interleaved | [`Handler`](https://docs.rs/loac/latest/loac/trait.Handler.html) async fn, [`StreamHandler`](https://docs.rs/loac/latest/loac/trait.StreamHandler.html) async fn, `future.interleaved()`, or [`ActorScope::cx_reply`](https://docs.rs/loac/latest/loac/struct.ActorScope.html#method.cx_reply) / [`ActorScope::cx_stream`](https://docs.rs/loac/latest/loac/struct.ActorScope.html#method.cx_stream) from a [`DispatchHandler`](https://docs.rs/loac/latest/loac/trait.DispatchHandler.html) | The actor task polls it fairly with mailbox, lifecycle, and other interleaved work. |
| exclusive | `future.exclusive()`, [`ActorScope::cx_exclusive`](https://docs.rs/loac/latest/loac/struct.ActorScope.html#method.cx_exclusive), or [`ActorScope::cx_stream_exclusive`](https://docs.rs/loac/latest/loac/struct.ActorScope.html#method.cx_stream_exclusive) from a [`DispatchHandler`](https://docs.rs/loac/latest/loac/trait.DispatchHandler.html) | Mailbox and actor-aware work pause until it finishes; owned tasks continue. |

`Handler` and `StreamHandler` always select interleaved scheduling, so they
require `interleaved`. A `DispatchHandler` may select any strategy. `ready`
and `exclusive` need no interleaving capability.

The `cx` constructors on [`ActorScope`](https://docs.rs/loac/latest/loac/struct.ActorScope.html)
pair `Cx` access with an explicit scheduling lane.
Call [`cx_reply`](https://docs.rs/loac/latest/loac/struct.ActorScope.html#method.cx_reply) / [`cx_stream`](https://docs.rs/loac/latest/loac/struct.ActorScope.html#method.cx_stream) inside an explicit `DispatchHandler` for an interleaved cx future, and [`cx_exclusive`](https://docs.rs/loac/latest/loac/struct.ActorScope.html#method.cx_exclusive) / [`cx_stream_exclusive`](https://docs.rs/loac/latest/loac/struct.ActorScope.html#method.cx_stream_exclusive) for an exclusive cx future. Inside the returned future, call `Cx::with` for temporary actor and scope access.

Stream messages use `#[message(stream = Item, reply = Final)]`. The runtime
creates a bounded item channel and returns the receiver to the caller as a
`StreamReply`. `StreamHandler` produces items from an async `cx` future polled
on the interleaved lane. An explicit `DispatchHandler<M, StreamKind>` selects
a stream-final strategy and assembles the channel plumbing with
`StreamDispatch::new`. The item stream ends when the handler drops its writer.

## Lifecycle and Shutdown

Graceful shutdown runs post-order through the owned tree.

| Mode | Behavior |
| --- | --- |
| `Stop` | Finishes dispatched replies and discards queued messages. |
| `Drain` | Dispatches eligible queued messages and finishes their replies. |
| `Kill` | Cancels cooperative work and skips `on_stop`. |

Shutdown closes admission when it commits.
A later [`call`](https://docs.rs/loac/latest/loac/struct.ActorRef.html#method.call) returns [`CallError::Closed`](https://docs.rs/loac/latest/loac/enum.CallError.html#variant.Closed).
[`ExitStatus::reason`](https://docs.rs/loac/latest/loac/struct.ExitStatus.html) describes only that actor.
`subtree` reports whether the runtime confirmed every owned descendant terminated.
`Unconfirmed` means proof is unavailable. It stays sticky through ancestors and does not stop a running parent.
Kill takes effect between polls. It cannot interrupt a synchronous handler, a poll that never returns, or user `Drop`.

## Progress Boundaries

- [`spawn`](https://docs.rs/loac/latest/loac/fn.spawn.html) schedules `init` and returns immediately. Admission opens before `init` finishes.
- `init` and lifecycle hooks run serially and block dispatch.
- Mailbox FIFO decides dispatch order. Async replies may complete in a different order.
- A self-call needs fresh dispatch capacity. It cannot complete during `init` or exclusive work.
- Prefer [`ActorFutureExt::map`](https://docs.rs/loac/latest/loac/trait.ActorFutureExt.html#method.map) or [`then`](https://docs.rs/loac/latest/loac/trait.ActorFutureExt.html#method.then) for consecutive actor work.
- Address cycles can deadlock when every participant waits.

Licensed under the MIT License.
