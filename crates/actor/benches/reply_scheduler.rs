//! Reply-scheduler scaling benchmarks.
//!
//! `complete_all` starts with every reply pending and measures releasing and
//! completing the whole active set. `single_wake_to_target_poll` keeps that set
//! pending across iterations and measures one external notification until its
//! target future is polled. The latter is deliberately a steady-state latency:
//! scheduler continuation work may already be runnable, and work after the
//! target records the duration is outside the interval.
//!
//! `mailbox_turn_to_target_poll_under_backlog` starts inside the first mailbox
//! handler after an exclusive staging barrier, with more ready messages still
//! queued, and ends inside the selected owned or interleaved probe's next poll.
//! The all-class fairness contract remains a deterministic runtime test; this
//! benchmark reports only the notification-to-poll handoff under mailbox load.

use std::{
    future::Future,
    num::NonZeroUsize,
    pin::Pin,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use loong_actor::{
    Actor, ActorOwner, ActorRef, ActorScope, ExitReason, Handler, IntoActorFuture, Message,
    Response, Shutdown, SpawnOptions, TryCallErrorKind, reply, spawn_with,
};
use tokio::sync::{mpsc, oneshot};

const ACTIVE_COUNTS: [usize; 3] = [1, 32, 256];
const MAILBOX_BACKLOG: usize = 32;

struct ReplyActor;

impl Actor for ReplyActor {}

struct OwnedReply {
    started: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl Message for OwnedReply {
    type Reply = ();
}

impl Handler<OwnedReply> for ReplyActor {
    fn handle(
        &mut self,
        message: OwnedReply,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, OwnedReply> + use<> {
        reply::owned(async move {
            let _ = message.started.send(());
            let _ = message.release.await;
        })
    }
}

struct InterleavedReply {
    started: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl Message for InterleavedReply {
    type Reply = ();
}

impl Handler<InterleavedReply> for ReplyActor {
    fn handle(
        &mut self,
        message: InterleavedReply,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, InterleavedReply> + use<> {
        reply::interleaved(
            async move {
                let _ = message.started.send(());
                let _ = message.release.await;
            }
            .into_actor(),
        )
    }
}

struct WakeCommand {
    started: Instant,
    completed: oneshot::Sender<Duration>,
}

struct WakeProbe {
    started: Option<oneshot::Sender<()>>,
    commands: mpsc::Receiver<WakeCommand>,
}

impl Future for WakeProbe {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, task: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(started) = self.started.take() {
            let _ = started.send(());
        }
        loop {
            match self.commands.poll_recv(task) {
                Poll::Ready(Some(WakeCommand { started, completed })) => {
                    let _ = completed.send(started.elapsed());
                }
                Poll::Ready(None) => panic!("a measured wake probe lost its controller"),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

struct OwnedWakeProbe(WakeProbe);

impl Message for OwnedWakeProbe {
    type Reply = ();
}

impl Handler<OwnedWakeProbe> for ReplyActor {
    fn handle(
        &mut self,
        message: OwnedWakeProbe,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, OwnedWakeProbe> + use<> {
        reply::owned(message.0)
    }
}

struct InterleavedWakeProbe(WakeProbe);

impl Message for InterleavedWakeProbe {
    type Reply = ();
}

impl Handler<InterleavedWakeProbe> for ReplyActor {
    fn handle(
        &mut self,
        message: InterleavedWakeProbe,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, InterleavedWakeProbe> + use<> {
        reply::interleaved(message.0.into_actor())
    }
}

struct MailboxBacklog;

impl Message for MailboxBacklog {
    type Reply = ();
}

impl Handler<MailboxBacklog> for ReplyActor {
    fn handle(
        &mut self,
        _message: MailboxBacklog,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, MailboxBacklog> + use<> {
        reply::ready(())
    }
}

struct MailboxTurnTrigger {
    commands: mpsc::Sender<WakeCommand>,
    completed: oneshot::Sender<Duration>,
}

impl Message for MailboxTurnTrigger {
    type Reply = ();
}

impl Handler<MailboxTurnTrigger> for ReplyActor {
    fn handle(
        &mut self,
        message: MailboxTurnTrigger,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, MailboxTurnTrigger> + use<> {
        if message
            .commands
            .try_send(WakeCommand {
                started: Instant::now(),
                completed: message.completed,
            })
            .is_err()
        {
            panic!("the selected wake probe has one empty command slot");
        }
        reply::ready(())
    }
}

struct StageMailboxBacklog {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl Message for StageMailboxBacklog {
    type Reply = ();
}

impl Handler<StageMailboxBacklog> for ReplyActor {
    fn handle(
        &mut self,
        message: StageMailboxBacklog,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, StageMailboxBacklog> + use<> {
        reply::exclusive(
            async move {
                let _ = message.entered.send(());
                message
                    .release
                    .await
                    .expect("the benchmark releases the exclusive staging barrier");
            }
            .into_actor(),
        )
    }
}

type EnqueueReply =
    fn(&ActorRef<ReplyActor>, oneshot::Sender<()>, oneshot::Receiver<()>) -> Response<()>;
type EnqueueWakeProbe =
    fn(&ActorRef<ReplyActor>, oneshot::Sender<()>, mpsc::Receiver<WakeCommand>) -> Response<()>;

fn enqueue_owned(
    actor: &ActorRef<ReplyActor>,
    started: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
) -> Response<()> {
    actor
        .try_call(OwnedReply { started, release })
        .expect("the benchmark mailbox has capacity")
}

fn enqueue_interleaved(
    actor: &ActorRef<ReplyActor>,
    started: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
) -> Response<()> {
    actor
        .try_call(InterleavedReply { started, release })
        .expect("the benchmark mailbox has capacity")
}

fn enqueue_owned_wake_probe(
    actor: &ActorRef<ReplyActor>,
    started: oneshot::Sender<()>,
    commands: mpsc::Receiver<WakeCommand>,
) -> Response<()> {
    actor
        .try_call(OwnedWakeProbe(WakeProbe {
            started: Some(started),
            commands,
        }))
        .expect("the benchmark mailbox has capacity")
}

fn enqueue_interleaved_wake_probe(
    actor: &ActorRef<ReplyActor>,
    started: oneshot::Sender<()>,
    commands: mpsc::Receiver<WakeCommand>,
) -> Response<()> {
    actor
        .try_call(InterleavedWakeProbe(WakeProbe {
            started: Some(started),
            commands,
        }))
        .expect("the benchmark mailbox has capacity")
}

fn spawn_benchmark_actor(active: usize) -> ActorOwner<ReplyActor> {
    let capacity = NonZeroUsize::new(active).expect("active reply counts are non-zero");
    spawn_with(
        ReplyActor,
        SpawnOptions::default()
            .with_mailbox_capacity(capacity)
            .with_max_in_flight(capacity),
    )
}

async fn install_pending(
    actor: &ActorRef<ReplyActor>,
    count: usize,
    enqueue: EnqueueReply,
) -> (Vec<oneshot::Sender<()>>, Vec<Response<()>>) {
    let mut started = Vec::with_capacity(count);
    let mut releases = Vec::with_capacity(count);
    let mut responses = Vec::with_capacity(count);

    for _ in 0..count {
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        responses.push(enqueue(actor, started_tx, release_rx));
        started.push(started_rx);
        releases.push(release_tx);
    }
    for started in started {
        started
            .await
            .expect("each benchmark reply reaches its first poll");
    }

    (releases, responses)
}

async fn install_wake_probes(
    actor: &ActorRef<ReplyActor>,
    count: usize,
    enqueue: EnqueueWakeProbe,
) -> (Vec<mpsc::Sender<WakeCommand>>, Vec<Response<()>>) {
    let mut started = Vec::with_capacity(count);
    let mut commands = Vec::with_capacity(count);
    let mut responses = Vec::with_capacity(count);

    for _ in 0..count {
        let (started_tx, started_rx) = oneshot::channel();
        let (commands_tx, commands_rx) = mpsc::channel(1);
        responses.push(enqueue(actor, started_tx, commands_rx));
        started.push(started_rx);
        commands.push(commands_tx);
    }
    for started in started {
        started
            .await
            .expect("each wake probe reaches its first poll");
    }

    (commands, responses)
}

async fn measure_complete_all(iters: u64, active: usize, enqueue: EnqueueReply) -> Duration {
    let owner = spawn_benchmark_actor(active);
    let actor = owner.actor_ref();
    let mut measured = Duration::ZERO;

    for _ in 0..iters {
        // Dispatch and first-poll setup stay outside the accumulated duration.
        let (releases, responses) = install_pending(&actor, active, enqueue).await;
        let started = Instant::now();
        for release in releases {
            release
                .send(())
                .expect("the active reply remains scheduled");
        }
        for response in responses {
            response.await.expect("the benchmark actor stays alive");
        }
        measured += started.elapsed();
    }

    assert_eq!(owner.shutdown(Shutdown::Kill).await, ExitReason::Killed);
    measured
}

async fn measure_single_wake_to_target_poll(
    iters: u64,
    active: usize,
    enqueue: EnqueueWakeProbe,
) -> Duration {
    let owner = spawn_benchmark_actor(active);
    let actor = owner.actor_ref();
    let (commands, responses) = install_wake_probes(&actor, active, enqueue).await;
    let mut measured = Duration::ZERO;
    let mut index = 0;

    for _ in 0..iters {
        let (completed_tx, completed_rx) = oneshot::channel();
        // The target records its own poll latency, so confirmation-sweep and
        // benchmark-task work after that point stay outside this interval.
        commands[index]
            .try_send(WakeCommand {
                started: Instant::now(),
                completed: completed_tx,
            })
            .expect("the selected wake probe remains scheduled");
        measured += completed_rx
            .await
            .expect("the selected wake probe records its poll latency");
        index = (index + 1) % active;
    }

    assert_eq!(owner.shutdown(Shutdown::Kill).await, ExitReason::Killed);
    drop(commands);
    drop(responses);
    measured
}

async fn measure_mailbox_turn_to_target_poll_under_backlog(
    iters: u64,
    backlog: usize,
    enqueue: EnqueueWakeProbe,
) -> Duration {
    let queued_after_trigger = backlog
        .checked_sub(1)
        .expect("mailbox backlog counts include a trigger message");
    let mailbox_capacity = NonZeroUsize::new(backlog).expect("mailbox backlog counts are non-zero");
    let owner = spawn_with(
        ReplyActor,
        SpawnOptions::default()
            .with_mailbox_capacity(mailbox_capacity)
            .with_max_in_flight(NonZeroUsize::new(2).expect("two active slots are non-zero")),
    );
    let actor = owner.actor_ref();
    let (mut commands, probe_responses) = install_wake_probes(&actor, 1, enqueue).await;
    let commands = commands.pop().expect("one wake probe was installed");
    let mut measured = Duration::ZERO;

    for _ in 0..iters {
        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let exclusive_response = actor
            .try_call(StageMailboxBacklog {
                entered: entered_tx,
                release: release_rx,
            })
            .expect("the exclusive probe is admitted");
        entered_rx
            .await
            .expect("the exclusive probe reaches its first poll");

        let (completed_tx, completed_rx) = oneshot::channel();
        let trigger_response = actor
            .try_call(MailboxTurnTrigger {
                commands: commands.clone(),
                completed: completed_tx,
            })
            .expect("the mailbox trigger is admitted");
        let mailbox_responses: Vec<_> = (0..queued_after_trigger)
            .map(|_| {
                actor
                    .try_call(MailboxBacklog)
                    .expect("the configured mailbox backlog is admitted")
            })
            .collect();
        let full = actor
            .try_call(MailboxBacklog)
            .expect_err("the mailbox is full before contention timing begins");
        assert_eq!(full.kind(), TryCallErrorKind::Full);

        release_tx
            .send(())
            .expect("the exclusive staging barrier remains scheduled");
        measured += completed_rx
            .await
            .expect("the target probe records its poll latency");

        exclusive_response
            .await
            .expect("the exclusive probe completes before cleanup");
        trigger_response
            .await
            .expect("the mailbox trigger completes before cleanup");
        for response in mailbox_responses {
            response
                .await
                .expect("the ready mailbox backlog drains after measurement");
        }
    }

    assert_eq!(owner.shutdown(Shutdown::Kill).await, ExitReason::Killed);
    drop(commands);
    drop(probe_responses);
    measured
}

fn reply_scheduler(criterion: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("the benchmark runtime builds");

    for (reply, enqueue, enqueue_wake_probe) in [
        (
            "owned",
            enqueue_owned as EnqueueReply,
            enqueue_owned_wake_probe as EnqueueWakeProbe,
        ),
        (
            "interleaved",
            enqueue_interleaved as EnqueueReply,
            enqueue_interleaved_wake_probe as EnqueueWakeProbe,
        ),
    ] {
        let mut group = criterion.benchmark_group(format!("reply_scheduler/{reply}"));
        for active in ACTIVE_COUNTS {
            group.throughput(Throughput::Elements(active as u64));
            group.bench_with_input(
                BenchmarkId::new("complete_all", active),
                &active,
                |bencher, &active| {
                    bencher
                        .to_async(&runtime)
                        .iter_custom(move |iters| measure_complete_all(iters, active, enqueue));
                },
            );

            group.throughput(Throughput::Elements(1));
            group.bench_with_input(
                BenchmarkId::new("single_wake_to_target_poll", active),
                &active,
                |bencher, &active| {
                    bencher.to_async(&runtime).iter_custom(move |iters| {
                        measure_single_wake_to_target_poll(iters, active, enqueue_wake_probe)
                    });
                },
            );
        }

        group.throughput(Throughput::Elements(1));
        group.bench_with_input(
            BenchmarkId::new("mailbox_turn_to_target_poll_under_backlog", MAILBOX_BACKLOG),
            &MAILBOX_BACKLOG,
            |bencher, &backlog| {
                bencher.to_async(&runtime).iter_custom(move |iters| {
                    measure_mailbox_turn_to_target_poll_under_backlog(
                        iters,
                        backlog,
                        enqueue_wake_probe,
                    )
                });
            },
        );
        group.finish();
    }
}

criterion_group!(benches, reply_scheduler);
criterion_main!(benches);
