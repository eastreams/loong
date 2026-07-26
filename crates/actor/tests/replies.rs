mod support;

use std::{
    future::Future,
    num::NonZeroUsize,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::Poll,
};

use loong_actor::{
    Actor, ActorFutureExt, ActorRef, ActorScope, CallError, ChildExit, ExitReason, Handler,
    IntoActorFuture, Message, Response, Shutdown, SpawnOptions, reply, spawn, spawn_with,
};
use tokio::sync::{mpsc, oneshot};

use support::{lock, watchdog};

async fn poll_once<F: Future>(mut future: Pin<&mut F>) -> Poll<F::Output> {
    std::future::poll_fn(|task| Poll::Ready(future.as_mut().poll(task))).await
}

struct Counter(u8);

impl Actor for Counter {}

struct Increment;

impl Message for Increment {
    type Reply = u8;
}

impl Handler<Increment> for Counter {
    fn handle(
        &mut self,
        _message: Increment,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, Increment> + use<> {
        self.0 += 1;
        reply::ready(self.0)
    }
}

#[tokio::test]
async fn ready_mutates_actor_and_replies_immediately() {
    let owner = spawn(Counter(0));
    let actor = owner.actor_ref();

    assert_eq!(watchdog(actor.call(Increment)).await, Ok(1));
    assert_eq!(watchdog(actor.call(Increment)).await, Ok(2));
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await,
        ExitReason::Stopped
    );
}

struct ChooseReply(bool);

impl Message for ChooseReply {
    type Reply = u8;
}

impl Handler<ChooseReply> for Counter {
    fn handle(
        &mut self,
        message: ChooseReply,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, ChooseReply> + use<> {
        if message.0 {
            reply::Either::Left(reply::ready(1))
        } else {
            reply::Either::Right(reply::owned(async { 2 }))
        }
    }
}

#[tokio::test]
async fn either_selects_between_reply_strategies_without_boxing() {
    let owner = spawn(Counter(0));
    let actor = owner.actor_ref();

    assert_eq!(watchdog(actor.call(ChooseReply(true))).await, Ok(1));
    assert_eq!(watchdog(actor.call(ChooseReply(false))).await, Ok(2));
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await,
        ExitReason::Stopped
    );
}

struct ProgressActor {
    log: Arc<Mutex<Vec<&'static str>>>,
}

impl Actor for ProgressActor {}

struct PendingOwned {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl Message for PendingOwned {
    type Reply = ();
}

impl Handler<PendingOwned> for ProgressActor {
    fn handle(
        &mut self,
        message: PendingOwned,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, PendingOwned> + use<> {
        reply::owned(async move {
            let _ = message.entered.send(());
            let _ = message.release.await;
        })
    }
}

struct Record(&'static str);

impl Message for Record {
    type Reply = ();
}

impl Handler<Record> for ProgressActor {
    fn handle(
        &mut self,
        message: Record,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, Record> + use<> {
        lock(&self.log).push(message.0);
        reply::ready(())
    }
}

struct InterleavedSequence {
    started: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

struct ThenSequence;

impl Message for ThenSequence {
    type Reply = u8;
}

impl Handler<ThenSequence> for ProgressActor {
    fn handle(
        &mut self,
        _message: ThenSequence,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, ThenSequence> + use<> {
        reply::interleaved(
            async { 1_u8 }
                .into_actor()
                .then(|value, actor: &mut Self, _scope| {
                    lock(&actor.log).push("then");
                    async move { value + 1 }.into_actor()
                }),
        )
    }
}

impl Message for InterleavedSequence {
    type Reply = ();
}

impl Handler<InterleavedSequence> for ProgressActor {
    fn handle(
        &mut self,
        message: InterleavedSequence,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, InterleavedSequence> + use<> {
        lock(&self.log).push("interleaved-start");
        let _ = message.started.send(());
        reply::interleaved(
            async move {
                let _ = message.release.await;
            }
            .into_actor()
            .map(|(), actor: &mut Self, _scope| {
                lock(&actor.log).push("interleaved-finish");
            }),
        )
    }
}

#[tokio::test]
async fn owned_does_not_block_mailbox_and_interleaved_reborrows_actor() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let owner = spawn(ProgressActor { log: log.clone() });
    let actor = owner.actor_ref();

    let (owned_entered_tx, owned_entered_rx) = oneshot::channel();
    let (owned_release_tx, owned_release_rx) = oneshot::channel();
    let owned = actor
        .try_call(PendingOwned {
            entered: owned_entered_tx,
            release: owned_release_rx,
        })
        .unwrap();
    watchdog(owned_entered_rx).await.unwrap();
    assert_eq!(watchdog(actor.call(Record("during-owned"))).await, Ok(()));

    let (interleaved_started_tx, interleaved_started_rx) = oneshot::channel();
    let (interleaved_release_tx, interleaved_release_rx) = oneshot::channel();
    let interleaved = actor
        .try_call(InterleavedSequence {
            started: interleaved_started_tx,
            release: interleaved_release_rx,
        })
        .unwrap();
    watchdog(interleaved_started_rx).await.unwrap();
    assert_eq!(watchdog(actor.call(Record("between-polls"))).await, Ok(()));

    owned_release_tx.send(()).unwrap();
    interleaved_release_tx.send(()).unwrap();
    assert_eq!(watchdog(owned).await, Ok(()));
    assert_eq!(watchdog(interleaved).await, Ok(()));
    assert_eq!(watchdog(actor.call(ThenSequence)).await, Ok(2));
    assert_eq!(
        *lock(&log),
        vec![
            "during-owned",
            "interleaved-start",
            "between-polls",
            "interleaved-finish",
            "then"
        ]
    );

    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await,
        ExitReason::Stopped
    );
}

struct HookChild;

impl Actor for HookChild {}

struct StopChild;

impl Message for StopChild {
    type Reply = ();
}

impl Handler<StopChild> for HookChild {
    fn handle(
        &mut self,
        _message: StopChild,
        scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, StopChild> + use<> {
        scope.request_shutdown(Shutdown::Stop);
        reply::ready(())
    }
}

struct ExclusiveActor {
    child_started: Option<oneshot::Sender<ActorRef<HookChild>>>,
    child_hooks: mpsc::UnboundedSender<()>,
}

impl Actor for ExclusiveActor {
    async fn on_start(&mut self, scope: &mut ActorScope<Self>) {
        let child = scope
            .spawn_child(HookChild)
            .expect("on_start accepts children")
            .into_actor_ref();
        if let Some(started) = self.child_started.take() {
            let _ = started.send(child);
        }
    }

    async fn on_child_exit(&mut self, _event: ChildExit, _scope: &mut ActorScope<Self>) {
        let _ = self.child_hooks.send(());
    }
}

impl Handler<PendingOwned> for ExclusiveActor {
    fn handle(
        &mut self,
        message: PendingOwned,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, PendingOwned> + use<> {
        reply::owned(async move {
            let _ = message.entered.send(());
            let _ = message.release.await;
        })
    }
}

struct InterleavedGate {
    dispatched: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl Message for InterleavedGate {
    type Reply = ();
}

impl Handler<InterleavedGate> for ExclusiveActor {
    fn handle(
        &mut self,
        message: InterleavedGate,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, InterleavedGate> + use<> {
        let _ = message.dispatched.send(());
        reply::interleaved(async move { drop(message.release.await) }.into_actor())
    }
}

struct ExclusiveGate {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl Message for ExclusiveGate {
    type Reply = ();
}

impl Handler<ExclusiveGate> for ExclusiveActor {
    fn handle(
        &mut self,
        message: ExclusiveGate,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, ExclusiveGate> + use<> {
        reply::exclusive(
            async move {
                let _ = message.entered.send(());
                let _ = message.release.await;
            }
            .into_actor(),
        )
    }
}

struct Mark(oneshot::Sender<()>);

impl Message for Mark {
    type Reply = ();
}

impl Handler<Mark> for ExclusiveActor {
    fn handle(
        &mut self,
        message: Mark,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, Mark> + use<> {
        let _ = message.0.send(());
        reply::ready(())
    }
}

#[tokio::test]
async fn exclusive_blocks_actor_work_but_owned_continues() {
    let (child_started_tx, child_started_rx) = oneshot::channel();
    let (child_hooks_tx, mut child_hooks_rx) = mpsc::unbounded_channel();
    let owner = spawn(ExclusiveActor {
        child_started: Some(child_started_tx),
        child_hooks: child_hooks_tx,
    });
    let actor = owner.actor_ref();
    let child = watchdog(child_started_rx).await.unwrap();

    let (owned_entered_tx, owned_entered_rx) = oneshot::channel();
    let (owned_release_tx, owned_release_rx) = oneshot::channel();
    let owned = actor
        .try_call(PendingOwned {
            entered: owned_entered_tx,
            release: owned_release_rx,
        })
        .unwrap();
    watchdog(owned_entered_rx).await.unwrap();

    let (interleaved_dispatched_tx, interleaved_dispatched_rx) = oneshot::channel();
    let (interleaved_release_tx, interleaved_release_rx) = oneshot::channel();
    let mut interleaved = Box::pin(
        actor
            .try_call(InterleavedGate {
                dispatched: interleaved_dispatched_tx,
                release: interleaved_release_rx,
            })
            .unwrap(),
    );
    watchdog(interleaved_dispatched_rx).await.unwrap();

    let (exclusive_entered_tx, exclusive_entered_rx) = oneshot::channel();
    let (exclusive_release_tx, exclusive_release_rx) = oneshot::channel();
    let exclusive = actor
        .try_call(ExclusiveGate {
            entered: exclusive_entered_tx,
            release: exclusive_release_rx,
        })
        .unwrap();
    watchdog(exclusive_entered_rx).await.unwrap();

    let (marked_tx, marked_rx) = oneshot::channel();
    let mut later_message = Box::pin(actor.try_call(Mark(marked_tx)).unwrap());
    assert_eq!(watchdog(child.call(StopChild)).await, Ok(()));
    assert_eq!(watchdog(child.closed()).await, ExitReason::Stopped);

    interleaved_release_tx.send(()).unwrap();
    owned_release_tx.send(()).unwrap();
    assert_eq!(watchdog(owned).await, Ok(()));

    assert!(poll_once(interleaved.as_mut()).await.is_pending());
    assert!(poll_once(later_message.as_mut()).await.is_pending());
    assert!(child_hooks_rx.try_recv().is_err());

    exclusive_release_tx.send(()).unwrap();
    assert_eq!(watchdog(exclusive).await, Ok(()));
    assert_eq!(watchdog(interleaved).await, Ok(()));
    assert_eq!(watchdog(later_message).await, Ok(()));
    watchdog(marked_rx).await.unwrap();
    watchdog(child_hooks_rx.recv()).await.unwrap();

    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await,
        ExitReason::Stopped
    );
}

struct StopActor;

impl Actor for StopActor {}

struct StopOwned {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl Message for StopOwned {
    type Reply = ();
}

impl Handler<StopOwned> for StopActor {
    fn handle(
        &mut self,
        message: StopOwned,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, StopOwned> + use<> {
        reply::owned(async move {
            let _ = message.entered.send(());
            let _ = message.release.await;
        })
    }
}

#[tokio::test]
async fn max_in_flight_bounds_dispatch_and_stop_waits_for_all_active_replies() {
    let options = SpawnOptions::default()
        .with_mailbox_capacity(NonZeroUsize::new(4).unwrap())
        .with_max_in_flight(NonZeroUsize::new(2).unwrap());
    let mut owner = spawn_with(StopActor, options);
    let actor = owner.actor_ref();

    let (first_entered_tx, first_entered_rx) = oneshot::channel();
    let (first_release_tx, first_release_rx) = oneshot::channel();
    let first = actor
        .try_call(StopOwned {
            entered: first_entered_tx,
            release: first_release_rx,
        })
        .unwrap();
    let (second_entered_tx, second_entered_rx) = oneshot::channel();
    let (second_release_tx, second_release_rx) = oneshot::channel();
    let second = actor
        .try_call(StopOwned {
            entered: second_entered_tx,
            release: second_release_rx,
        })
        .unwrap();
    watchdog(first_entered_rx).await.unwrap();
    watchdog(second_entered_rx).await.unwrap();

    let (third_entered_tx, third_entered_rx) = oneshot::channel();
    let (_third_release_tx, third_release_rx) = oneshot::channel();
    let third = actor
        .try_call(StopOwned {
            entered: third_entered_tx,
            release: third_release_rx,
        })
        .unwrap();
    let mut third_entered = Box::pin(third_entered_rx);
    assert!(poll_once(third_entered.as_mut()).await.is_pending());

    assert_eq!(
        owner.request_shutdown(Shutdown::Stop),
        loong_actor::ShutdownStatus::Requested
    );
    assert!(watchdog(third_entered).await.is_err());
    assert_eq!(
        watchdog(third).await,
        Err(CallError::BeforeDispatch(ExitReason::Stopped))
    );

    first_release_tx.send(()).unwrap();
    second_release_tx.send(()).unwrap();
    assert_eq!(watchdog(first).await, Ok(()));
    assert_eq!(watchdog(second).await, Ok(()));
    assert_eq!(watchdog(owner.wait()).await, ExitReason::Stopped);
}

struct PanicActor;

impl Actor for PanicActor {}

struct PendingSibling {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl Message for PendingSibling {
    type Reply = ();
}

impl Handler<PendingSibling> for PanicActor {
    fn handle(
        &mut self,
        message: PendingSibling,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, PendingSibling> + use<> {
        reply::owned(async move {
            let _ = message.entered.send(());
            let _ = message.release.await;
        })
    }
}

struct PanicReply {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

impl Message for PanicReply {
    type Reply = ();
}

impl Handler<PanicReply> for PanicActor {
    fn handle(
        &mut self,
        message: PanicReply,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, PanicReply> + use<> {
        reply::owned(async move {
            let _ = message.entered.send(());
            let _ = message.release.await;
            panic!("intentional reply panic");
        })
    }
}

#[tokio::test]
async fn reply_panic_fails_sibling_in_flight_work() {
    let mut owner = spawn(PanicActor);
    let actor = owner.actor_ref();
    let (sibling_entered_tx, sibling_entered_rx) = oneshot::channel();
    let (_sibling_release_tx, sibling_release_rx) = oneshot::channel();
    let sibling = actor
        .try_call(PendingSibling {
            entered: sibling_entered_tx,
            release: sibling_release_rx,
        })
        .unwrap();
    watchdog(sibling_entered_rx).await.unwrap();

    let (panic_entered_tx, panic_entered_rx) = oneshot::channel();
    let (panic_release_tx, panic_release_rx) = oneshot::channel();
    let panicking = actor
        .try_call(PanicReply {
            entered: panic_entered_tx,
            release: panic_release_rx,
        })
        .unwrap();
    watchdog(panic_entered_rx).await.unwrap();
    panic_release_tx.send(()).unwrap();

    assert_eq!(
        watchdog(panicking).await,
        Err(CallError::DuringDispatch(ExitReason::Panicked))
    );
    assert_eq!(
        watchdog(sibling).await,
        Err(CallError::DuringDispatch(ExitReason::Panicked))
    );
    assert_eq!(watchdog(owner.wait()).await, ExitReason::Panicked);
}

struct SelfCaller;

impl Actor for SelfCaller {}

struct Echo(u8);

impl Message for Echo {
    type Reply = u8;
}

impl Handler<Echo> for SelfCaller {
    fn handle(
        &mut self,
        message: Echo,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, Echo> + use<> {
        reply::ready(message.0)
    }
}

struct OwnedSelfCall(u8);

impl Message for OwnedSelfCall {
    type Reply = u8;
}

impl Handler<OwnedSelfCall> for SelfCaller {
    fn handle(
        &mut self,
        message: OwnedSelfCall,
        scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, OwnedSelfCall> + use<> {
        let response = scope.myself().try_call(Echo(message.0)).unwrap();
        reply::owned(async move { response.await.unwrap() })
    }
}

struct SingleSlotSelfCall {
    entered: oneshot::Sender<()>,
}

impl Message for SingleSlotSelfCall {
    type Reply = u8;
}

impl Handler<SingleSlotSelfCall> for SelfCaller {
    fn handle(
        &mut self,
        message: SingleSlotSelfCall,
        scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, SingleSlotSelfCall> + use<> {
        let response = scope.myself().try_call(Echo(1)).unwrap();
        let _ = message.entered.send(());
        reply::owned(async move { response.await.unwrap() })
    }
}

struct InterleavedSelfCall(u8);

impl Message for InterleavedSelfCall {
    type Reply = u8;
}

impl Handler<InterleavedSelfCall> for SelfCaller {
    fn handle(
        &mut self,
        message: InterleavedSelfCall,
        scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, InterleavedSelfCall> + use<> {
        let response = scope.myself().try_call(Echo(message.0)).unwrap();
        reply::interleaved(async move { response.await.unwrap() }.into_actor())
    }
}

struct ExclusiveSelfCall {
    observed: oneshot::Sender<Response<u8>>,
    polled: oneshot::Sender<()>,
}

impl Message for ExclusiveSelfCall {
    type Reply = ();
}

impl Handler<ExclusiveSelfCall> for SelfCaller {
    fn handle(
        &mut self,
        message: ExclusiveSelfCall,
        scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, ExclusiveSelfCall> + use<> {
        let awaited = scope.myself().try_call(Echo(1)).unwrap();
        let observed = scope.myself().try_call(Echo(2)).unwrap();
        let _ = message.observed.send(observed);
        reply::exclusive(
            async move {
                let _ = message.polled.send(());
                awaited.await.unwrap();
            }
            .into_actor(),
        )
    }
}

#[tokio::test]
async fn nonexclusive_self_calls_progress_but_exclusive_self_call_waits() {
    let options = SpawnOptions::default()
        .with_mailbox_capacity(NonZeroUsize::new(8).unwrap())
        .with_max_in_flight(NonZeroUsize::new(4).unwrap());
    let mut owner = spawn_with(SelfCaller, options);
    let actor = owner.actor_ref();

    assert_eq!(watchdog(actor.call(OwnedSelfCall(3))).await, Ok(3));
    assert_eq!(watchdog(actor.call(InterleavedSelfCall(4))).await, Ok(4));

    let (observed_tx, observed_rx) = oneshot::channel();
    let (polled_tx, polled_rx) = oneshot::channel();
    let outer = actor
        .try_call(ExclusiveSelfCall {
            observed: observed_tx,
            polled: polled_tx,
        })
        .unwrap();
    let observed = watchdog(observed_rx).await.unwrap();
    watchdog(polled_rx).await.unwrap();

    assert_eq!(
        owner.request_shutdown(Shutdown::Kill),
        loong_actor::ShutdownStatus::Requested
    );
    assert_eq!(
        watchdog(outer).await,
        Err(CallError::DuringDispatch(ExitReason::Killed))
    );
    assert_eq!(
        watchdog(observed).await,
        Err(CallError::BeforeDispatch(ExitReason::Killed))
    );
    assert_eq!(watchdog(owner.wait()).await, ExitReason::Killed);
}

#[tokio::test]
async fn self_call_waits_when_the_outer_reply_owns_the_only_active_slot() {
    let options = SpawnOptions::default()
        .with_mailbox_capacity(NonZeroUsize::new(2).unwrap())
        .with_max_in_flight(NonZeroUsize::new(1).unwrap());
    let mut owner = spawn_with(SelfCaller, options);
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = oneshot::channel();
    let response = actor
        .try_call(SingleSlotSelfCall {
            entered: entered_tx,
        })
        .unwrap();
    tokio::pin!(response);

    watchdog(entered_rx).await.unwrap();
    assert!(poll_once(response.as_mut()).await.is_pending());
    assert_eq!(
        owner.request_shutdown(Shutdown::Kill),
        loong_actor::ShutdownStatus::Requested
    );
    assert_eq!(
        watchdog(response).await,
        Err(CallError::DuringDispatch(ExitReason::Killed))
    );
    assert_eq!(watchdog(owner.wait()).await, ExitReason::Killed);
}

struct FairActor {
    handled: Arc<AtomicUsize>,
}

impl Actor for FairActor {}

struct ActiveReply {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
    completed_at: Arc<AtomicUsize>,
}

impl Message for ActiveReply {
    type Reply = ();
}

impl Handler<ActiveReply> for FairActor {
    fn handle(
        &mut self,
        message: ActiveReply,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, ActiveReply> + use<> {
        let handled = self.handled.clone();
        reply::owned(async move {
            let _ = message.entered.send(());
            let _ = message.release.await;
            message
                .completed_at
                .store(handled.load(Ordering::SeqCst), Ordering::SeqCst);
        })
    }
}

struct ActiveInterleavedReply {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
    completed_at: Arc<AtomicUsize>,
}

impl Message for ActiveInterleavedReply {
    type Reply = ();
}

impl Handler<ActiveInterleavedReply> for FairActor {
    fn handle(
        &mut self,
        message: ActiveInterleavedReply,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, ActiveInterleavedReply> + use<> {
        reply::interleaved(
            async move {
                let _ = message.entered.send(());
                let _ = message.release.await;
                message.completed_at
            }
            .into_actor()
            .map(|completed_at, actor: &mut Self, _scope| {
                completed_at.store(actor.handled.load(Ordering::SeqCst), Ordering::SeqCst);
            }),
        )
    }
}

struct ReadyWork;

impl Message for ReadyWork {
    type Reply = ();
}

impl Handler<ReadyWork> for FairActor {
    fn handle(
        &mut self,
        _message: ReadyWork,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, ReadyWork> + use<> {
        self.handled.fetch_add(1, Ordering::SeqCst);
        reply::ready(())
    }
}

#[tokio::test]
async fn ready_mailbox_input_does_not_starve_active_reply() {
    let handled = Arc::new(AtomicUsize::new(0));
    let options = SpawnOptions::default()
        .with_mailbox_capacity(NonZeroUsize::new(64).unwrap())
        .with_max_in_flight(NonZeroUsize::new(4).unwrap());
    let owner = spawn_with(
        FairActor {
            handled: handled.clone(),
        },
        options,
    );
    let actor = owner.actor_ref();
    let completed_at = Arc::new(AtomicUsize::new(usize::MAX));
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let active = actor
        .try_call(ActiveReply {
            entered: entered_tx,
            release: release_rx,
            completed_at: completed_at.clone(),
        })
        .unwrap();
    watchdog(entered_rx).await.unwrap();

    let queued: Vec<_> = (0..32)
        .map(|_| actor.try_call(ReadyWork).unwrap())
        .collect();
    release_tx.send(()).unwrap();
    assert_eq!(watchdog(active).await, Ok(()));
    assert!(completed_at.load(Ordering::SeqCst) < queued.len());

    for response in queued {
        assert_eq!(watchdog(response).await, Ok(()));
    }
    assert_eq!(handled.load(Ordering::SeqCst), 32);
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await,
        ExitReason::Stopped
    );
}

#[tokio::test]
async fn ready_mailbox_input_does_not_starve_woken_interleaved_reply() {
    let handled = Arc::new(AtomicUsize::new(0));
    let options = SpawnOptions::default()
        .with_mailbox_capacity(NonZeroUsize::new(64).unwrap())
        .with_max_in_flight(NonZeroUsize::new(4).unwrap());
    let owner = spawn_with(
        FairActor {
            handled: handled.clone(),
        },
        options,
    );
    let actor = owner.actor_ref();
    let completed_at = Arc::new(AtomicUsize::new(usize::MAX));
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let active = actor
        .try_call(ActiveInterleavedReply {
            entered: entered_tx,
            release: release_rx,
            completed_at: completed_at.clone(),
        })
        .unwrap();
    tokio::pin!(active);
    watchdog(entered_rx).await.unwrap();
    assert!(poll_once(active.as_mut()).await.is_pending());

    let queued: Vec<_> = (0..32)
        .map(|_| actor.try_call(ReadyWork).unwrap())
        .collect();
    release_tx.send(()).unwrap();
    assert_eq!(watchdog(active).await, Ok(()));
    assert!(completed_at.load(Ordering::SeqCst) < queued.len());

    for response in queued {
        assert_eq!(watchdog(response).await, Ok(()));
    }
    assert_eq!(handled.load(Ordering::SeqCst), 32);
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await,
        ExitReason::Stopped
    );
}

struct FairChildExitActor {
    child_started: Option<oneshot::Sender<ActorRef<HookChild>>>,
    handled: Arc<AtomicUsize>,
    hook_completed_at: Arc<AtomicUsize>,
    hook_completed: Option<oneshot::Sender<()>>,
}

impl Actor for FairChildExitActor {
    async fn on_start(&mut self, scope: &mut ActorScope<Self>) {
        let child = scope
            .spawn_child(HookChild)
            .expect("on_start accepts children")
            .into_actor_ref();
        if let Some(started) = self.child_started.take() {
            let _ = started.send(child);
        }
    }

    async fn on_child_exit(&mut self, _event: ChildExit, _scope: &mut ActorScope<Self>) {
        self.hook_completed_at
            .store(self.handled.load(Ordering::SeqCst), Ordering::SeqCst);
        if let Some(completed) = self.hook_completed.take() {
            let _ = completed.send(());
        }
    }
}

impl Handler<PendingOwned> for FairChildExitActor {
    fn handle(
        &mut self,
        message: PendingOwned,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, PendingOwned> + use<> {
        reply::owned(async move {
            let _ = message.entered.send(());
            let _ = message.release.await;
        })
    }
}

impl Handler<ExclusiveGate> for FairChildExitActor {
    fn handle(
        &mut self,
        message: ExclusiveGate,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, ExclusiveGate> + use<> {
        reply::exclusive(
            async move {
                let _ = message.entered.send(());
                let _ = message.release.await;
            }
            .into_actor(),
        )
    }
}

impl Handler<ReadyWork> for FairChildExitActor {
    fn handle(
        &mut self,
        _message: ReadyWork,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, ReadyWork> + use<> {
        self.handled.fetch_add(1, Ordering::SeqCst);
        reply::ready(())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn queued_child_exit_progresses_before_ready_mailbox_is_exhausted() {
    let handled = Arc::new(AtomicUsize::new(0));
    let hook_completed_at = Arc::new(AtomicUsize::new(usize::MAX));
    let (child_started_tx, child_started_rx) = oneshot::channel();
    let (hook_completed_tx, hook_completed_rx) = oneshot::channel();
    let options = SpawnOptions::default()
        .with_mailbox_capacity(NonZeroUsize::new(64).unwrap())
        .with_max_in_flight(NonZeroUsize::new(4).unwrap());
    let owner = spawn_with(
        FairChildExitActor {
            child_started: Some(child_started_tx),
            handled: handled.clone(),
            hook_completed_at: hook_completed_at.clone(),
            hook_completed: Some(hook_completed_tx),
        },
        options,
    );
    let actor = owner.actor_ref();
    let child = watchdog(child_started_rx).await.unwrap();

    let (owned_entered_tx, owned_entered_rx) = oneshot::channel();
    let (owned_release_tx, owned_release_rx) = oneshot::channel();
    let owned = actor
        .try_call(PendingOwned {
            entered: owned_entered_tx,
            release: owned_release_rx,
        })
        .unwrap();
    watchdog(owned_entered_rx).await.unwrap();

    let (exclusive_entered_tx, exclusive_entered_rx) = oneshot::channel();
    let (exclusive_release_tx, exclusive_release_rx) = oneshot::channel();
    let exclusive = actor
        .try_call(ExclusiveGate {
            entered: exclusive_entered_tx,
            release: exclusive_release_rx,
        })
        .unwrap();
    watchdog(exclusive_entered_rx).await.unwrap();

    assert_eq!(watchdog(child.call(StopChild)).await, Ok(()));
    // On the current-thread runtime the child publishes its supervisor event
    // before this task resumes; exclusive keeps the parent from consuming it.
    assert_eq!(watchdog(child.closed()).await, ExitReason::Stopped);
    let mut hook_completed = Box::pin(hook_completed_rx);
    assert!(poll_once(hook_completed.as_mut()).await.is_pending());

    let queued: Vec<_> = (0..32)
        .map(|_| actor.try_call(ReadyWork).unwrap())
        .collect();
    exclusive_release_tx.send(()).unwrap();
    assert_eq!(watchdog(exclusive).await, Ok(()));
    watchdog(hook_completed).await.unwrap();
    assert!(hook_completed_at.load(Ordering::SeqCst) < queued.len());

    owned_release_tx.send(()).unwrap();
    assert_eq!(watchdog(owned).await, Ok(()));
    for response in queued {
        assert_eq!(watchdog(response).await, Ok(()));
    }
    assert_eq!(handled.load(Ordering::SeqCst), 32);
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await,
        ExitReason::Stopped
    );
}
