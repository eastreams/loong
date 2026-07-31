use std::{future::Future, hint::black_box, num::NonZeroUsize};

use actix::Actor as _;
use criterion::{
    Criterion, Throughput, async_executor::AsyncExecutor, criterion_group, criterion_main,
};
use loong_actor::{ExitReason, Shutdown, SpawnOptions, prelude::*, spawn_with};

const MAILBOX_CAPACITY: usize = 64;
const ONE_WAY_BATCH: usize = 32;

struct Ready;

impl Message for Ready {
    type Reply = u64;
}

impl actix::Message for Ready {
    type Result = u64;
}

struct Notify;

impl Message for Notify {
    type Reply = ();
}

impl actix::Message for Notify {
    type Result = ();
}

struct Barrier;

impl Message for Barrier {
    type Reply = u64;
}

impl actix::Message for Barrier {
    type Result = u64;
}

struct LoongActor {
    handled: u64,
}

impl Actor for LoongActor {}

impl SyncHandler<Ready> for LoongActor {
    fn handle(&mut self, _message: Ready, _scope: &mut ActorScope<Self>) -> u64 {
        1
    }
}

impl SyncHandler<Notify> for LoongActor {
    fn handle(&mut self, _message: Notify, _scope: &mut ActorScope<Self>) {
        self.handled += 1;
    }
}

impl SyncHandler<Barrier> for LoongActor {
    fn handle(&mut self, _message: Barrier, _scope: &mut ActorScope<Self>) -> u64 {
        self.handled
    }
}

struct ActixActor {
    handled: u64,
}

impl actix::Actor for ActixActor {
    type Context = actix::Context<Self>;

    fn started(&mut self, context: &mut Self::Context) {
        context.set_mailbox_capacity(MAILBOX_CAPACITY);
    }
}

impl actix::Handler<Ready> for ActixActor {
    type Result = u64;

    fn handle(&mut self, _message: Ready, _context: &mut Self::Context) -> Self::Result {
        1
    }
}

impl actix::Handler<Notify> for ActixActor {
    type Result = ();

    fn handle(&mut self, _message: Notify, _context: &mut Self::Context) -> Self::Result {
        self.handled += 1;
    }
}

impl actix::Handler<Barrier> for ActixActor {
    type Result = u64;

    fn handle(&mut self, _message: Barrier, _context: &mut Self::Context) -> Self::Result {
        self.handled
    }
}

// Criterion supports Tokio directly.
// Actix needs its LocalSet-aware System runner.
// This adapter preserves one System across iterations.
// Runtime construction stays outside measurements.
struct ActixExecutor<'a>(&'a actix::SystemRunner);

impl AsyncExecutor for ActixExecutor<'_> {
    fn block_on<T>(&self, future: impl Future<Output = T>) -> T {
        self.0.block_on(future)
    }
}

fn actor_runtime_comparison(criterion: &mut Criterion) {
    let loong_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("the Loong benchmark runtime builds");
    let mailbox_capacity =
        NonZeroUsize::new(MAILBOX_CAPACITY).expect("the benchmark capacity is non-zero");
    let loong_owner = loong_runtime.block_on(async {
        spawn_with(
            LoongActor { handled: 0 },
            SpawnOptions::default().with_mailbox_capacity(mailbox_capacity),
        )
    });
    let loong_actor = loong_owner.actor_ref();

    let actix_system = actix::System::with_tokio_rt(|| {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("the Actix benchmark runtime builds")
    });
    let actix_actor = actix_system.block_on(async { ActixActor { handled: 0 }.start() });

    // Force both actors through startup before Criterion begins sampling.
    assert_eq!(
        loong_runtime
            .block_on(loong_actor.call(Ready))
            .expect("the Loong actor starts"),
        1
    );
    assert_eq!(
        actix_system
            .block_on(actix_actor.send(Ready))
            .expect("the Actix actor starts"),
        1
    );

    // Both cases include bounded admission.
    // Both dispatch a synchronous handler.
    // Both deliver one ready value.
    // Actor and runtime construction stay outside measurements.
    // Lifecycle guarantees differ and are not compared.
    let mut ready = criterion.benchmark_group("actor_runtime/ready_request_reply");
    ready.throughput(Throughput::Elements(1));
    ready.bench_function("loong", |bencher| {
        bencher.to_async(&loong_runtime).iter(|| async {
            let reply = loong_actor
                .call(Ready)
                .await
                .expect("the Loong actor stays alive");
            black_box(reply)
        });
    });
    ready.bench_function("actix", |bencher| {
        bencher
            .to_async(ActixExecutor(&actix_system))
            .iter(|| async {
                let reply = actix_actor
                    .send(Ready)
                    .await
                    .expect("the Actix actor stays alive");
                black_box(reply)
            });
    });
    ready.finish();

    // `try_send` is both runtimes' bounded, non-waiting API.
    // A FIFO Barrier follows each batch.
    // Its counter proves all prior Notify messages ran.
    // No per-message observation channel is needed.
    // Barrier cost is amortized across the batch.
    // Each barrier empties the mailbox before another batch.
    // This is not a saturation or backpressure benchmark.
    let mut one_way = criterion.benchmark_group("actor_runtime/one_way_handler_drain");
    one_way.throughput(Throughput::Elements(ONE_WAY_BATCH as u64));
    one_way.bench_function("loong", |bencher| {
        // Criterion may restart sampling while actors remain alive.
        // Seed expected state outside measured iterations.
        let mut expected = loong_runtime
            .block_on(loong_actor.call(Barrier))
            .expect("the Loong baseline completes");
        bencher.to_async(&loong_runtime).iter(|| {
            expected += ONE_WAY_BATCH as u64;
            let expected = expected;
            let actor = &loong_actor;
            async move {
                for _ in 0..ONE_WAY_BATCH {
                    actor
                        .try_send(Notify)
                        .expect("the Loong mailbox accepts one batch");
                }
                let handled = actor
                    .call(Barrier)
                    .await
                    .expect("the Loong barrier completes");
                assert_eq!(handled, expected);
                black_box(handled)
            }
        });
    });
    one_way.bench_function("actix", |bencher| {
        let mut expected = actix_system
            .block_on(actix_actor.send(Barrier))
            .expect("the Actix baseline completes");
        bencher.to_async(ActixExecutor(&actix_system)).iter(|| {
            expected += ONE_WAY_BATCH as u64;
            let expected = expected;
            let actor = &actix_actor;
            async move {
                for _ in 0..ONE_WAY_BATCH {
                    actor
                        .try_send(Notify)
                        .expect("the Actix mailbox accepts one batch");
                }
                let handled = actor
                    .send(Barrier)
                    .await
                    .expect("the Actix barrier completes");
                assert_eq!(handled, expected);
                black_box(handled)
            }
        });
    });
    one_way.finish();

    let status = loong_runtime.block_on(loong_owner.shutdown(Shutdown::Kill));
    assert_eq!(status.reason(), ExitReason::Killed);

    drop(actix_actor);
    actix::System::current().stop();
    actix_system
        .run()
        .expect("the Actix benchmark system stops cleanly");
}

criterion_group!(benches, actor_runtime_comparison);
criterion_main!(benches);
