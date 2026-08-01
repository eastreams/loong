# loong-actor

`loong-actor` is a small runtime for typed, local actors. It combines
Actix-style message/reply typing with bounded admission, explicit ownership,
and a Ractor-style supervision tree.

The crate is an early MVP. Its current contract is deliberately narrow:

- `SyncHandler` returns immediate reply values;
- bare `Future` values use owned scheduling;
- `.interleaved()` and `.exclusive()` select actor-aware scheduling;
- interleaved work has a separate bound; owned tasks are unbounded;
- `ActorRef` values communicate and may request shutdown;
- the unique `ActorOwner` owns root lifetime;
- child actors are owned by their parent runtime;
- exit status separates local reason from subtree confirmation;
- Stop, Drain, and Kill use a control plane separate from the mailbox;
- mailbox FIFO determines dispatch order, not reply completion order;
- Kill can interrupt cooperative async work between polls, but cannot interrupt
  a running synchronous handler, a poll call that never returns, or user `Drop`.

```rust
use loong_actor::{ExitReason, Shutdown, SubtreeStatus, prelude::*, spawn};

struct Counter(u64);

impl Actor for Counter {
    type SpawnArgs = u64;

    async fn init(value: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self(value)
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct Add(u64);

impl SyncHandler<Add> for Counter {
    fn handle(
        &mut self,
        message: Add,
        _scope: &mut ActorScope<'_, Self>,
    ) -> u64 {
        self.0 += message.0;
        self.0
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owner = spawn::<Counter>(0);
    let counter = owner.actor_ref();

    assert_eq!(counter.call(Add(2)).await?, 2);
    assert_eq!(counter.call(Add(3)).await?, 5);
    let status = owner.shutdown(Shutdown::Drain).await;
    assert_eq!(status.reason(), ExitReason::Drained);
    assert_eq!(status.subtree(), SubtreeStatus::Terminated);
    Ok(())
}
```

`ExitStatus::reason` describes only that actor. `ExitStatus::subtree` reports
whether the runtime confirmed all owned descendants terminated. An unconfirmed
child does not automatically stop its parent. The missing guarantee remains
sticky. `Unconfirmed` means proof is unavailable. It does not prove liveness.

Streaming does not require a runtime-specific message kind: a message reply may
be a bounded channel receiver or another application-defined stream handle.

Owned replies consume no `max_in_flight` slot. Their self-calls can progress
while an interleaved slot remains available. Interleaved replies need another
slot for mailbox re-entry. An exclusive reply blocks its queued self-call.
`init` completes before dispatch starts. `on_child_exit` blocks dispatch while
running. `spawn` returns before `init`; sends may admit while calls await
dispatch. Admission is closed in `on_stop`, so a new self-call returns
`Closed`. Prefer `ActorFutureExt::map` or `then` for consecutive actor work.
Address cycles can still deadlock when every participant waits.

Licensed under the MIT License.
