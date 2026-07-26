use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use loong_actor::{Actor, ActorScope, Handler, IntoActorFuture, Message, Shutdown, reply, spawn};

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
    ) -> impl loong_actor::IntoReply<Self, Ready> + use<> {
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
    ) -> impl loong_actor::IntoReply<Self, Owned> + use<> {
        async { 1 }
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
    ) -> impl loong_actor::IntoReply<Self, Interleaved> + use<> {
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
    ) -> impl loong_actor::IntoReply<Self, Exclusive> + use<> {
        reply::exclusive(std::future::ready(1).into_actor())
    }
}

fn message_round_trip(criterion: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("the benchmark runtime builds");
    let owner = runtime.block_on(async { spawn(ReplyActor) });
    let actor = owner.actor_ref();

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
    let reason = runtime.block_on(owner.shutdown(Shutdown::Kill));
    assert_eq!(reason, loong_actor::ExitReason::Killed);
}

criterion_group!(benches, message_round_trip);
criterion_main!(benches);
