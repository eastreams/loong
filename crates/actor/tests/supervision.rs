mod support;

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::task::Poll;

use loong_actor::{
    Actor, ActorRef, ActorScope, CallError, ChildExit, ExitReason, Handler, IntoActorFuture,
    Message, ReplyExt, Shutdown, spawn,
};
use tokio::sync::{mpsc, oneshot};

use support::watchdog;

struct ChildActor;

impl Actor for ChildActor {}

struct StopSelf;

impl Message for StopSelf {
    type Reply = ();
}

impl Handler<StopSelf> for ChildActor {
    fn handle(
        &mut self,
        _message: StopSelf,
        scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, StopSelf> + use<> {
        scope.request_shutdown(Shutdown::Stop);
        ().ready()
    }
}

struct PanicSelf;

impl Message for PanicSelf {
    type Reply = ();
}

impl Handler<PanicSelf> for ChildActor {
    fn handle(
        &mut self,
        _message: PanicSelf,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, PanicSelf> + use<> {
        panic!("intentional child panic");
        #[allow(unreachable_code)]
        ().ready()
    }
}

struct Supervisor {
    child_started: Option<oneshot::Sender<ActorRef<ChildActor>>>,
    events: mpsc::UnboundedSender<ChildExit>,
    observed: Arc<AtomicUsize>,
}

impl Actor for Supervisor {
    async fn on_start<'a>(&'a mut self, scope: &'a mut ActorScope<Self>) {
        let child = scope
            .spawn_child(ChildActor)
            .expect("on_start accepts children");
        if let Some(started) = self.child_started.take() {
            let _ = started.send(child.into_actor_ref());
        }
    }

    async fn on_child_exit<'a>(&'a mut self, event: ChildExit, _scope: &'a mut ActorScope<Self>) {
        self.observed.fetch_add(1, Ordering::SeqCst);
        let _ = self.events.send(event);
    }
}

struct Observed;

impl Message for Observed {
    type Reply = usize;
}

impl Handler<Observed> for Supervisor {
    fn handle(
        &mut self,
        _message: Observed,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, Observed> + use<> {
        self.observed.load(Ordering::SeqCst).ready()
    }
}

struct ChildExitBarrier;

impl Message for ChildExitBarrier {
    type Reply = ();
}

impl Handler<ChildExitBarrier> for Supervisor {
    fn handle(
        &mut self,
        _message: ChildExitBarrier,
        _scope: &mut ActorScope<Self>,
    ) -> impl loong_actor::IntoReply<Self, ChildExitBarrier> + use<> {
        let mut yielded = false;
        std::future::poll_fn(move |task| {
            if yielded {
                Poll::Ready(())
            } else {
                yielded = true;
                task.waker().wake_by_ref();
                Poll::Pending
            }
        })
        .into_actor()
        .interleaved()
    }
}

fn spawn_supervisor() -> (
    loong_actor::ActorOwner<Supervisor>,
    oneshot::Receiver<ActorRef<ChildActor>>,
    mpsc::UnboundedReceiver<ChildExit>,
) {
    let (child_tx, child_rx) = oneshot::channel();
    let (events_tx, events_rx) = mpsc::unbounded_channel();
    let owner = spawn(Supervisor {
        child_started: Some(child_tx),
        events: events_tx,
        observed: Arc::new(AtomicUsize::new(0)),
    });
    (owner, child_rx, events_rx)
}

#[tokio::test(flavor = "current_thread")]
async fn clean_child_exit_is_reported_exactly_once() {
    let (owner, child_rx, mut events) = spawn_supervisor();
    let supervisor = owner.actor_ref();
    let child = watchdog(child_rx).await.unwrap();

    assert_eq!(watchdog(child.call(StopSelf)).await, Ok(()));
    let event = watchdog(events.recv()).await.unwrap();
    assert_eq!(event.reason(), ExitReason::Stopped);
    assert_eq!(watchdog(child.closed()).await, ExitReason::Stopped);

    // The barrier remains pending across a child-exit scheduling opportunity, so
    // a duplicate queued behind the first hook cannot hide behind mailbox work.
    assert_eq!(watchdog(supervisor.call(ChildExitBarrier)).await, Ok(()));
    assert_eq!(watchdog(supervisor.call(Observed)).await.unwrap(), 1);
    assert!(events.try_recv().is_err());
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await,
        ExitReason::Stopped
    );
}

#[tokio::test]
async fn child_panic_is_reported_without_stopping_the_parent() {
    let (owner, child_rx, mut events) = spawn_supervisor();
    let supervisor = owner.actor_ref();
    let child = watchdog(child_rx).await.unwrap();

    assert_eq!(
        watchdog(child.call(PanicSelf)).await,
        Err(CallError::DuringDispatch(ExitReason::Panicked))
    );
    let event = watchdog(events.recv()).await.unwrap();
    assert_eq!(event.reason(), ExitReason::Panicked);
    assert_eq!(watchdog(supervisor.call(Observed)).await.unwrap(), 1);

    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await,
        ExitReason::Stopped
    );
}
