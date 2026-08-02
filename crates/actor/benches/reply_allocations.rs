//! Steady-state end-to-end allocation counts for reply strategies.
//!
//! Measurement includes mailbox admission, envelopes, response transport, and dispatch.
//! It also includes Tokio-task or actor-scheduler ownership.
//! Runtime creation and actor spawn stay outside each region.
//! Warmup, reporting, and shutdown also stay outside.
//! Counts are diagnostics, not compatibility thresholds.
//! The ready row is the common transport baseline.
//! Other rows do not subtract it.
//! The allocator counter is thread-local.
//! A current-thread runtime keeps measured allocations on one thread.

use std::hint::black_box;

use allocation_counter::{AllocationInfo, measure};
use loong_actor::{ActorRef, ExitReason, Shutdown, prelude::*, spawn};

const WARMUP_CALLS: usize = 64;
const MEASURED_CALLS: usize = 10_000;

struct ReplyActor;

#[loong_actor::actor(mailbox = 1, interleaved = 1)]
impl Actor for ReplyActor {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct Ready;

impl Handler<Ready> for ReplyActor {
    fn handle(
        &mut self,
        _message: Ready,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, Ready> + use<> {
        1.ready()
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct Owned;

impl Handler<Owned> for ReplyActor {
    fn handle(
        &mut self,
        _message: Owned,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, Owned> + use<> {
        // All asynchronous modes share this concrete base future.
        // Counts therefore isolate wrappers and execution ownership.
        std::future::ready(1)
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct Interleaved;

impl Handler<Interleaved> for ReplyActor {
    fn handle(
        &mut self,
        _message: Interleaved,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, Interleaved> + use<> {
        std::future::ready(1).into_actor().interleaved()
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct Exclusive;

impl Handler<Exclusive> for ReplyActor {
    fn handle(
        &mut self,
        _message: Exclusive,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, Exclusive> + use<> {
        std::future::ready(1).into_actor().exclusive()
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
    let owner = runtime.block_on(async { spawn::<ReplyActor>(()) });
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
        runtime.block_on(owner.shutdown(Shutdown::Kill)).reason(),
        ExitReason::Killed
    );
}
