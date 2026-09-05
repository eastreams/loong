//! Message round-trip and interleaving profile benchmarks.
//!
//! `interleaving_profile` keeps at most one reply active.
//! It isolates fixed and unbounded profile overhead.
//! It does not measure saturated admission or queue scaling.

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use loac::{
    Actor, ActorRef, ActorScope, DispatchHandler, InterleavedFutureExt, IntoActorFuture, Message,
    ReplyExt, Shutdown,
};

struct ReplyActor;

#[loac::actor(mailbox = 1, interleaved = 1)]
impl Actor for ReplyActor {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct UnboundedReplyActor;

#[loac::actor(mailbox = 1, interleaved = unbounded)]
impl Actor for UnboundedReplyActor {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct Ready;

impl DispatchHandler<Ready> for ReplyActor {
    fn handle(
        &mut self,
        _message: Ready,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Ready> + use<> {
        1.ready()
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct Owned;

impl DispatchHandler<Owned> for ReplyActor {
    fn handle(
        &mut self,
        _message: Owned,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Owned> + use<> {
        std::future::ready(1)
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct Interleaved;

impl DispatchHandler<Interleaved> for ReplyActor {
    fn handle(
        &mut self,
        _message: Interleaved,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Interleaved> + use<> {
        std::future::ready(1).into_actor().interleaved()
    }
}

impl DispatchHandler<Interleaved> for UnboundedReplyActor {
    fn handle(
        &mut self,
        _message: Interleaved,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Interleaved> + use<> {
        std::future::ready(1).into_actor().interleaved()
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct Exclusive;

impl DispatchHandler<Exclusive> for ReplyActor {
    fn handle(
        &mut self,
        _message: Exclusive,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Exclusive> + use<> {
        std::future::ready(1).into_actor().exclusive()
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum MeasuredProfile {
    Fixed,
    Unbounded,
}

async fn measure_round_trip<A>(actor: &ActorRef<A>, measured: bool) -> Duration
where
    A: DispatchHandler<Interleaved>,
{
    let started = Instant::now();
    let reply = actor
        .call(Interleaved)
        .await
        .expect("the benchmark actor stays alive");
    let elapsed = started.elapsed();
    black_box(reply);
    if measured { elapsed } else { Duration::ZERO }
}

// Every sample drives both paths. Their order alternates each iteration.
async fn measure_interleaving_profile(
    iterations: u64,
    measured: MeasuredProfile,
    fixed: ActorRef<ReplyActor>,
    unbounded: ActorRef<UnboundedReplyActor>,
) -> Duration {
    let mut elapsed = Duration::ZERO;
    for iteration in 0..iterations {
        let fixed_measured = measured == MeasuredProfile::Fixed;
        if iteration % 2 == 0 {
            elapsed += measure_round_trip(&fixed, fixed_measured).await;
            elapsed += measure_round_trip(&unbounded, !fixed_measured).await;
        } else {
            elapsed += measure_round_trip(&unbounded, !fixed_measured).await;
            elapsed += measure_round_trip(&fixed, fixed_measured).await;
        }
    }
    elapsed
}

fn message_round_trip(criterion: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("the benchmark runtime builds");
    let owner = runtime.block_on(async { loac::spawn::<ReplyActor>(()) });
    let actor = owner.actor_ref();
    let unbounded_owner = runtime.block_on(async { loac::spawn::<UnboundedReplyActor>(()) });
    let unbounded_actor = unbounded_owner.actor_ref();

    let mut group = criterion.benchmark_group("message_round_trip");
    group.throughput(Throughput::Elements(1));

    group.bench_function("ready", |bencher| {
        bencher.to_async(&runtime).iter(|| async {
            let reply = actor.call(Ready).await.expect("the actor stays alive");
            black_box(reply)
        });
    });

    group.bench_function("owned", |bencher| {
        bencher.to_async(&runtime).iter(|| async {
            let reply = actor.call(Owned).await.expect("the actor stays alive");
            black_box(reply)
        });
    });

    group.bench_function("interleaved", |bencher| {
        bencher.to_async(&runtime).iter(|| async {
            let reply = actor
                .call(Interleaved)
                .await
                .expect("the actor stays alive");
            black_box(reply)
        });
    });

    group.bench_function("exclusive", |bencher| {
        bencher.to_async(&runtime).iter(|| async {
            let reply = actor.call(Exclusive).await.expect("the actor stays alive");
            black_box(reply)
        });
    });

    group.finish();

    // Each result accumulates only its selected round trip.
    let mut group = criterion.benchmark_group("interleaving_profile");
    group.throughput(Throughput::Elements(1));
    group.bench_function("fixed", |bencher| {
        bencher.to_async(&runtime).iter_custom(|iterations| {
            measure_interleaving_profile(
                iterations,
                MeasuredProfile::Fixed,
                actor.clone(),
                unbounded_actor.clone(),
            )
        });
    });
    group.bench_function("unbounded", |bencher| {
        bencher.to_async(&runtime).iter_custom(|iterations| {
            measure_interleaving_profile(
                iterations,
                MeasuredProfile::Unbounded,
                actor.clone(),
                unbounded_actor.clone(),
            )
        });
    });
    group.finish();

    let reason = runtime.block_on(owner.shutdown(Shutdown::Kill));
    assert_eq!(reason.reason(), loac::ExitReason::Killed);
    let reason = runtime.block_on(unbounded_owner.shutdown(Shutdown::Kill));
    assert_eq!(reason.reason(), loac::ExitReason::Killed);
}

criterion_group!(benches, message_round_trip);
criterion_main!(benches);
