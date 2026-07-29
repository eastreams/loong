# loong-actor

`loong-actor` is a small runtime for typed, local actors. It combines
Actix-style message/reply typing with bounded admission, explicit ownership,
and a Ractor-style supervision tree.

The crate is an early MVP. Its current contract is deliberately narrow:

- `SyncHandler` returns immediate reply values;
- bare `Future` values use owned scheduling;
- `.interleaved()` and `.exclusive()` select actor-aware scheduling;
- dispatched work is bounded independently from mailbox capacity;
- `ActorRef` values communicate but do not own actor lifetimes;
- the unique `ActorOwner` controls root lifetime;
- child actors are owned by their parent's runtime scope;
- Stop, Drain, and Kill use a control plane separate from the mailbox;
- mailbox FIFO determines dispatch order, not reply completion order;
- Kill can interrupt cooperative async work between polls, but cannot interrupt
  a running synchronous handler, a poll call that never returns, or user `Drop`.

```rust
use loong_actor::{ExitReason, Shutdown, prelude::*, spawn};

struct Counter(u64);

impl Actor for Counter {}

struct Add(u64);

impl Message for Add {
    type Reply = u64;
}

impl SyncHandler<Add> for Counter {
    fn handle(
        &mut self,
        message: Add,
        _scope: &mut ActorScope<Self>,
    ) -> u64 {
        self.0 += message.0;
        self.0
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owner = spawn(Counter(0));
    let counter = owner.actor_ref();

    assert_eq!(counter.call(Add(2)).await?, 2);
    assert_eq!(counter.call(Add(3)).await?, 5);
    assert_eq!(owner.shutdown(Shutdown::Drain).await, ExitReason::Drained);
    Ok(())
}
```

Streaming does not require a runtime-specific message kind: a message reply may
be a bounded channel receiver or another application-defined stream handle.

Owned and interleaved replies permit mailbox re-entry only while another
`max_in_flight` slot is free. With a limit of one, an outer reply waiting for its
own queued call deadlocks until externally interrupted. Awaiting a self-call
from an exclusive reply, `on_start`, or a running actor's `on_child_exit` has the
same limitation. Admission is already closed in `on_stop`, so a new self-call
returns `Closed` instead. Prefer `ActorFutureExt::map` or `then` for consecutive
work on the same actor. Address cycles can likewise deadlock when every
participant waits.

Licensed under the MIT License.
