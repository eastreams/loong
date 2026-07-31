//! Full-mailbox admission and drain benchmarks.
//!
//! `saturated_try_call_full` keeps the queue full while measuring synchronous
//! rejected admissions. `saturated_ready_drain` fills the queue before timing
//! and measures dispatching and completing that fixed ready backlog. Neither
//! metric includes actor startup, queue setup, or shutdown.

use std::{
    hint::black_box,
    num::NonZeroUsize,
    time::{Duration, Instant},
};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use loong_actor::{
    Actor, ActorOwner, ActorRef, ActorScope, ExitReason, Handler, Message, ReplyExt, Response,
    Shutdown, SpawnOptions, TryCallErrorKind, spawn_with,
};

const MAILBOX_CAPACITIES: [usize; 3] = [1, 32, 256];

struct MailboxActor;

impl Actor for MailboxActor {}

struct ReadyTraffic;

impl Message for ReadyTraffic {
    type Reply = ();
}

impl Handler<ReadyTraffic> for MailboxActor {
    fn handle(
        &mut self,
        _message: ReadyTraffic,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, ReadyTraffic> + use<> {
        ().ready()
    }
}

/// Ready replies leave no interleaved work active.
/// The fixed limit leaves mailbox capacity as the only variable.
fn spawn_benchmark_actor(capacity: usize) -> ActorOwner<MailboxActor> {
    let capacity = NonZeroUsize::new(capacity).expect("mailbox capacities are non-zero");
    spawn_with(
        MailboxActor,
        SpawnOptions::default()
            .with_mailbox_capacity(capacity)
            .with_max_in_flight(NonZeroUsize::MIN),
    )
}

/// Fills the queue without yielding, which is exact on the current-thread runtime.
fn fill_mailbox(actor: &ActorRef<MailboxActor>, capacity: usize) -> Vec<Response<()>> {
    (0..capacity)
        .map(|_| {
            actor
                .try_call(ReadyTraffic)
                .expect("each configured mailbox slot accepts one request")
        })
        .collect()
}

async fn warm_up(actor: &ActorRef<MailboxActor>) {
    actor
        .call(ReadyTraffic)
        .await
        .expect("the benchmark actor starts and remains alive");
}

async fn drain(responses: Vec<Response<()>>) {
    for response in responses {
        response
            .await
            .expect("the benchmark actor completes the queued request");
    }
}

async fn measure_saturated_try_call_full(iters: u64, capacity: usize) -> Duration {
    let owner = spawn_benchmark_actor(capacity);
    let actor = owner.actor_ref();
    warm_up(&actor).await;
    let queued = fill_mailbox(&actor, capacity);

    let probe = actor
        .try_call(ReadyTraffic)
        .expect_err("a request beyond configured capacity is rejected");
    assert_eq!(probe.kind(), TryCallErrorKind::Full);

    let started = Instant::now();
    for _ in 0..iters {
        let error = actor
            .try_call(ReadyTraffic)
            .expect_err("the queue remains full throughout the measured batch");
        assert_eq!(error.kind(), TryCallErrorKind::Full);
        black_box(error.into_message());
    }
    let measured = started.elapsed();

    drain(queued).await;
    assert_eq!(
        owner.shutdown(Shutdown::Kill).await.reason(),
        ExitReason::Killed
    );
    measured
}

async fn measure_saturated_ready_drain(iters: u64, capacity: usize) -> Duration {
    let owner = spawn_benchmark_actor(capacity);
    let actor = owner.actor_ref();
    warm_up(&actor).await;
    let mut measured = Duration::ZERO;

    for _ in 0..iters {
        let queued = fill_mailbox(&actor, capacity);
        let started = Instant::now();
        drain(queued).await;
        measured += started.elapsed();
    }

    assert_eq!(
        owner.shutdown(Shutdown::Kill).await.reason(),
        ExitReason::Killed
    );
    measured
}

fn mailbox_saturation(criterion: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("the benchmark runtime builds");
    let mut group = criterion.benchmark_group("mailbox");

    for capacity in MAILBOX_CAPACITIES {
        group.throughput(Throughput::Elements(1));
        group.bench_with_input(
            BenchmarkId::new("saturated_try_call_full", capacity),
            &capacity,
            |bencher, &capacity| {
                bencher
                    .to_async(&runtime)
                    .iter_custom(move |iters| measure_saturated_try_call_full(iters, capacity));
            },
        );

        group.throughput(Throughput::Elements(capacity as u64));
        group.bench_with_input(
            BenchmarkId::new("saturated_ready_drain", capacity),
            &capacity,
            |bencher, &capacity| {
                bencher
                    .to_async(&runtime)
                    .iter_custom(move |iters| measure_saturated_ready_drain(iters, capacity));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, mailbox_saturation);
criterion_main!(benches);
