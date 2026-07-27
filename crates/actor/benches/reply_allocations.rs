//! Steady-state end-to-end allocation counts for reply strategies.
//!
//! The measurement includes mailbox admission, envelope and response transport,
//! handler dispatch, and any reply-scheduler ownership. Runtime creation, actor
//! spawn, per-mode warmup, reporting, and shutdown stay outside each region.
//! Counts are raw diagnostics rather than compatibility thresholds; the ready
//! row is the common transport baseline and is not subtracted from other rows.
//! The allocator counter is thread-local, so the current-thread runtime is part
//! of the measurement contract: it keeps both caller and actor-task allocations
//! on the thread enclosed by `measure`.

use std::hint::black_box;

use allocation_counter::{AllocationInfo, measure};
use loong_actor::{ActorRef, ExitReason, Shutdown, prelude::*, spawn};

const WARMUP_CALLS: usize = 64;
const MEASURED_CALLS: usize = 10_000;

struct ReplyActor;

impl Actor for ReplyActor {}

struct Ready;

impl Message for Ready {
    type Reply = u64;
}

impl Handler<Ready> for ReplyActor {
    fn handle(
        &mut self,
        _message: Ready,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, Ready> + use<> {
        reply::ready(1)
    }
}

struct Owned;

impl Message for Owned {
    type Reply = u64;
}

impl Handler<Owned> for ReplyActor {
    fn handle(
        &mut self,
        _message: Owned,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, Owned> + use<> {
        // Match the actor-aware modes' concrete base future so byte counts
        // reflect reply wrappers and scheduler ownership, not payload layout.
        reply::owned(std::future::ready(1))
    }
}

struct Interleaved;

impl Message for Interleaved {
    type Reply = u64;
}

impl Handler<Interleaved> for ReplyActor {
    fn handle(
        &mut self,
        _message: Interleaved,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, Interleaved> + use<> {
        reply::interleaved(std::future::ready(1).into_actor())
    }
}

struct Exclusive;

impl Message for Exclusive {
    type Reply = u64;
}

impl Handler<Exclusive> for ReplyActor {
    fn handle(
        &mut self,
        _message: Exclusive,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, Exclusive> + use<> {
        reply::exclusive(std::future::ready(1).into_actor())
    }
}

fn run_calls<M>(
    runtime: &tokio::runtime::Runtime,
    actor: &ActorRef<ReplyActor>,
    calls: usize,
    mut message: impl FnMut() -> M,
) where
    M: Message,
    ReplyActor: Handler<M>,
{
    runtime.block_on(async {
        for _ in 0..calls {
            let reply = actor
                .call(message())
                .await
                .expect("the benchmark actor stays alive");
            black_box(reply);
        }
    });
}

fn measure_calls<M>(
    runtime: &tokio::runtime::Runtime,
    actor: &ActorRef<ReplyActor>,
    message: impl FnMut() -> M,
) -> AllocationInfo
where
    M: Message,
    ReplyActor: Handler<M>,
{
    measure(|| run_calls(runtime, actor, MEASURED_CALLS, message))
}

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("the benchmark runtime builds");
    let owner = runtime.block_on(async { spawn(ReplyActor) });
    let actor = owner.actor_ref();

    run_calls(&runtime, &actor, WARMUP_CALLS, || Ready);
    run_calls(&runtime, &actor, WARMUP_CALLS, || Owned);
    run_calls(&runtime, &actor, WARMUP_CALLS, || Interleaved);
    run_calls(&runtime, &actor, WARMUP_CALLS, || Exclusive);

    let samples = [
        ("ready", measure_calls(&runtime, &actor, || Ready)),
        ("owned", measure_calls(&runtime, &actor, || Owned)),
        (
            "interleaved",
            measure_calls(&runtime, &actor, || Interleaved),
        ),
        ("exclusive", measure_calls(&runtime, &actor, || Exclusive)),
    ];

    println!("reply allocations ({MEASURED_CALLS} calls per mode)");
    println!(
        "{:<12} {:>14} {:>12} {:>16} {:>12}",
        "mode", "allocations", "alloc/call", "bytes", "bytes/call"
    );
    for (mode, sample) in samples {
        println!(
            "{mode:<12} {:>14} {:>12.3} {:>16} {:>12.3}",
            sample.count_total,
            sample.count_total as f64 / MEASURED_CALLS as f64,
            sample.bytes_total,
            sample.bytes_total as f64 / MEASURED_CALLS as f64,
        );
    }

    assert_eq!(
        runtime.block_on(owner.shutdown(Shutdown::Kill)),
        ExitReason::Killed
    );
}
