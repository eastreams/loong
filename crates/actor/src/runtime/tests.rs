use std::{
    future::Future,
    num::NonZeroUsize,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};

use tokio::sync::mpsc;

use crate::{
    Actor, ActorFuture, ActorRef, ActorScope, ChildExit, ChildId, ExitReason, IntoActorFuture,
    Shutdown, ShutdownStatus, SpawnOptions,
    mailbox::{ActorMailbox, Control, DynEnvelope, Envelope, Mode},
    scheduler::ReplyScheduler,
};

use super::{
    ChildSet, DiscardOutcome, OwnedActor, TEARDOWN_DROP_BUDGET, Turn, TurnCursor, Work, actor_turn,
    close_and_discard, graceful_finish, kill_actor, spawn_actor,
};

struct TestActor;

impl Actor for TestActor {}

struct CountPendingExclusive(Arc<AtomicUsize>);

impl ActorFuture<TestActor> for CountPendingExclusive {
    type Output = ();

    fn poll(
        self: Pin<&mut Self>,
        _actor: &mut TestActor,
        _scope: &mut ActorScope<TestActor>,
        _task: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }
}

struct CountEnvelope(Arc<AtomicUsize>);

impl Envelope<TestActor> for CountEnvelope {
    fn is_abandoned(&self) -> bool {
        false
    }

    fn dispatch(
        self: Box<Self>,
        _actor: &mut TestActor,
        _scope: &mut ActorScope<TestActor>,
        _scheduler: &mut ReplyScheduler<TestActor>,
    ) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct ChildKillDropProbe {
    child: Arc<Control>,
    observed_kill: Arc<AtomicBool>,
}

impl Future for ChildKillDropProbe {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _task: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

impl Drop for ChildKillDropProbe {
    fn drop(&mut self) {
        self.observed_kill.store(
            matches!(
                self.child.mode(),
                Mode::Killing | Mode::Exited(ExitReason::Killed)
            ),
            Ordering::SeqCst,
        );
    }
}

impl Envelope<TestActor> for ChildKillDropProbe {
    fn is_abandoned(&self) -> bool {
        false
    }

    fn dispatch(
        self: Box<Self>,
        _actor: &mut TestActor,
        _scope: &mut ActorScope<TestActor>,
        _scheduler: &mut ReplyScheduler<TestActor>,
    ) {
        unreachable!("Kill discards queued envelopes")
    }
}

enum TeardownEnvelope {
    Noop,
    RequestKill(Arc<Control>),
    MarkDropped(Arc<AtomicBool>),
}

impl Drop for TeardownEnvelope {
    fn drop(&mut self) {
        match self {
            Self::Noop => {}
            Self::RequestKill(control) => {
                control.request(Shutdown::Kill);
            }
            Self::MarkDropped(observed) => observed.store(true, Ordering::SeqCst),
        }
    }
}

impl Envelope<TestActor> for TeardownEnvelope {
    fn is_abandoned(&self) -> bool {
        false
    }

    fn dispatch(
        self: Box<Self>,
        _actor: &mut TestActor,
        _scope: &mut ActorScope<TestActor>,
        _scheduler: &mut ReplyScheduler<TestActor>,
    ) {
        unreachable!("teardown discards queued envelopes")
    }
}

#[tokio::test]
async fn committed_kill_prevents_a_graceful_child_request() {
    // The child mode distinguishes biased Kill observation from an incorrect
    // first poll of the guarded future: request_all would synchronously commit
    // Stop before graceful_finish could return Work::Killed.
    let child_control = Control::new();
    let mut children = ChildSet::default();
    children.insert(
        ChildId::new(),
        OwnedActor {
            control: Arc::clone(&child_control),
            join: None,
        },
    );

    let (mailbox, _inbox) = ActorMailbox::<TestActor>::channel(1);
    let control = Arc::clone(&mailbox.control);
    let actor_ref = ActorRef::new(Arc::downgrade(&mailbox), control.subscribe_mode());
    let (supervisor_tx, _supervisor_rx) = mpsc::unbounded_channel();
    let mut scope = ActorScope {
        actor_ref,
        control: Arc::clone(&control),
        children,
        accepts_children: true,
        supervisor_tx,
    };
    let mut mode = control.subscribe_mode();
    let mut actor = TestActor;

    assert_eq!(control.request(Shutdown::Kill), ShutdownStatus::Requested);
    assert!(matches!(
        graceful_finish(
            &mut actor,
            &mut scope,
            Shutdown::Stop,
            ExitReason::Stopped,
            &mut mode,
        )
        .await,
        Work::Killed
    ));
    assert_eq!(child_control.mode(), Mode::Running);
}

#[tokio::test]
async fn ordinary_cursor_visits_all_eligible_classes_before_repeating_mailbox() {
    // All four ordinary classes are eligible before the first turn. Reusing one
    // cursor is the contract under test; its starting class and order are not.
    let mailbox_dispatches = Arc::new(AtomicUsize::new(0));
    let (mailbox, mut inbox) = ActorMailbox::<TestActor>::channel(5);
    for _ in 0..5 {
        mailbox
            .sender
            .try_send(
                Box::new(CountEnvelope(Arc::clone(&mailbox_dispatches))) as DynEnvelope<TestActor>
            )
            .unwrap();
    }

    let control = Arc::clone(&mailbox.control);
    let actor_ref = ActorRef::new(Arc::downgrade(&mailbox), control.subscribe_mode());
    let (supervisor_tx, mut supervisor_rx) = mpsc::unbounded_channel();
    let mut scope = ActorScope {
        actor_ref,
        control: Arc::clone(&control),
        children: ChildSet::default(),
        accepts_children: true,
        supervisor_tx: supervisor_tx.clone(),
    };
    supervisor_tx
        .send(ChildExit::new(ChildId::new(), ExitReason::Stopped))
        .unwrap();

    let owned_completed = Arc::new(AtomicBool::new(false));
    let interleaved_completed = Arc::new(AtomicBool::new(false));
    let mut scheduler = ReplyScheduler::new(NonZeroUsize::new(4).unwrap());
    scheduler.push_owned({
        let completed = Arc::clone(&owned_completed);
        async move {
            completed.store(true, Ordering::SeqCst);
        }
    });
    scheduler.push_interleaved({
        let completed = Arc::clone(&interleaved_completed);
        async move {
            completed.store(true, Ordering::SeqCst);
        }
        .into_actor()
    });

    let mut actor = TestActor;
    let mut mode = control.subscribe_mode();
    let mut cursor = TurnCursor::default();
    let mut mailbox_turns = 0;
    let mut reply_turns = 0;
    let mut child_turns = 0;

    for _ in 0..4 {
        match actor_turn(
            &mut actor,
            &mut scope,
            &mut inbox,
            &mut supervisor_rx,
            &mut mode,
            &mut scheduler,
            true,
            Mode::Running,
            &mut cursor,
        )
        .await
        {
            Turn::Message => mailbox_turns += 1,
            Turn::ReplyProgress => reply_turns += 1,
            Turn::Child(event) => {
                assert_eq!(event.reason(), ExitReason::Stopped);
                child_turns += 1;
            }
            Turn::Mode => panic!("the lifecycle mode changed unexpectedly"),
            Turn::InboxClosed => panic!("the mailbox closed unexpectedly"),
        }
    }

    assert_eq!(mailbox_turns, 1);
    assert_eq!(reply_turns, 2);
    assert_eq!(child_turns, 1);
    assert!(owned_completed.load(Ordering::SeqCst));
    assert!(interleaved_completed.load(Ordering::SeqCst));
    assert_eq!(mailbox_dispatches.load(Ordering::SeqCst), 1);
    assert_eq!(inbox.len(), 4);
}

#[tokio::test]
async fn exclusive_progresses_while_owned_replies_keep_completing() {
    // A fixed owned-first order would return after each ready owned reply and
    // never poll the continuously eligible exclusive reply.
    let (mailbox, mut inbox) = ActorMailbox::<TestActor>::channel(1);
    let control = Arc::clone(&mailbox.control);
    let actor_ref = ActorRef::new(Arc::downgrade(&mailbox), control.subscribe_mode());
    let (supervisor_tx, mut supervisor_rx) = mpsc::unbounded_channel();
    let mut scope = ActorScope {
        actor_ref,
        control: Arc::clone(&control),
        children: ChildSet::default(),
        accepts_children: true,
        supervisor_tx,
    };
    let exclusive_polls = Arc::new(AtomicUsize::new(0));
    let mut scheduler = ReplyScheduler::new(NonZeroUsize::new(2).unwrap());
    scheduler.push_exclusive(CountPendingExclusive(Arc::clone(&exclusive_polls)));
    let mut actor = TestActor;
    let mut mode = control.subscribe_mode();
    let mut cursor = TurnCursor::default();

    for expected_polls in 1..=4 {
        scheduler.push_owned(std::future::ready(()));
        assert!(matches!(
            actor_turn(
                &mut actor,
                &mut scope,
                &mut inbox,
                &mut supervisor_rx,
                &mut mode,
                &mut scheduler,
                true,
                Mode::Running,
                &mut cursor,
            )
            .await,
            Turn::ReplyProgress
        ));
        assert_eq!(exclusive_polls.load(Ordering::SeqCst), expected_polls);
    }
}

#[tokio::test]
async fn child_kill_commits_before_actor_work_is_dropped() {
    // Active replies and queued envelopes may both run arbitrary destructors.
    // Observing the child mode from each Drop rejects any teardown that merely
    // waits for children after clearing local work instead of cancelling first.
    let (_child_ref, child) = spawn_actor(TestActor, SpawnOptions::default(), None);
    let child_control = Arc::clone(&child.control);
    let mut children = ChildSet::default();
    children.insert(ChildId::new(), child);

    let active_observed_kill = Arc::new(AtomicBool::new(false));
    let queued_observed_kill = Arc::new(AtomicBool::new(false));
    let (mailbox, mut inbox) = ActorMailbox::<TestActor>::channel(1);
    mailbox
        .sender
        .try_send(Box::new(ChildKillDropProbe {
            child: Arc::clone(&child_control),
            observed_kill: Arc::clone(&queued_observed_kill),
        }))
        .unwrap();
    assert_eq!(
        mailbox.control.request(Shutdown::Kill),
        ShutdownStatus::Requested
    );

    let actor_ref = ActorRef::new(Arc::downgrade(&mailbox), mailbox.control.subscribe_mode());
    let (supervisor_tx, _supervisor_rx) = mpsc::unbounded_channel();
    let mut scope = ActorScope {
        actor_ref,
        control: Arc::clone(&mailbox.control),
        children,
        accepts_children: true,
        supervisor_tx,
    };
    let mut scheduler = ReplyScheduler::new(NonZeroUsize::new(1).unwrap());
    scheduler.push_owned(ChildKillDropProbe {
        child: child_control,
        observed_kill: Arc::clone(&active_observed_kill),
    });

    assert_eq!(
        kill_actor(&mut scope, &mut inbox, &mut scheduler).await,
        ExitReason::Killed
    );
    assert!(active_observed_kill.load(Ordering::SeqCst));
    assert!(queued_observed_kill.load(Ordering::SeqCst));
}

#[tokio::test]
async fn stop_discard_observes_kill_from_each_envelope_drop() {
    // The first destructor upgrades Stop to Kill. Leaving the second envelope
    // queued proves teardown observes the transition between individual Drops,
    // before hard teardown takes ownership of the remaining work.
    let (mailbox, mut inbox) = ActorMailbox::<TestActor>::channel(2);
    let second_dropped = Arc::new(AtomicBool::new(false));
    mailbox
        .sender
        .try_send(Box::new(TeardownEnvelope::RequestKill(Arc::clone(
            &mailbox.control,
        ))))
        .unwrap();
    mailbox
        .sender
        .try_send(Box::new(TeardownEnvelope::MarkDropped(Arc::clone(
            &second_dropped,
        ))))
        .unwrap();
    assert_eq!(
        mailbox.control.request(Shutdown::Stop),
        ShutdownStatus::Requested
    );

    assert_eq!(
        close_and_discard(&mut inbox, &mailbox.control, Mode::Stopping).await,
        DiscardOutcome::ModeChanged
    );
    assert_eq!(mailbox.control.mode(), Mode::Killing);
    assert!(!second_dropped.load(Ordering::SeqCst));
    assert_eq!(inbox.len(), 1);

    assert_eq!(
        close_and_discard(&mut inbox, &mailbox.control, Mode::Killing).await,
        DiscardOutcome::Complete
    );
    assert!(second_dropped.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "current_thread")]
async fn queued_discard_yields_after_its_fixed_drop_budget() {
    // One manual poll must stop before the seventeenth Drop, and the next must
    // complete it. This locks the exact cooperative boundary while rejecting
    // both an unbounded drain and an implementation that yields too frequently.
    let (mailbox, mut inbox) = ActorMailbox::<TestActor>::channel(TEARDOWN_DROP_BUDGET + 1);
    for _ in 0..TEARDOWN_DROP_BUDGET {
        mailbox
            .sender
            .try_send(Box::new(TeardownEnvelope::Noop))
            .unwrap();
    }

    let last_dropped = Arc::new(AtomicBool::new(false));
    mailbox
        .sender
        .try_send(Box::new(TeardownEnvelope::MarkDropped(Arc::clone(
            &last_dropped,
        ))))
        .unwrap();
    assert_eq!(
        mailbox.control.request(Shutdown::Stop),
        ShutdownStatus::Requested
    );

    let discard = close_and_discard(&mut inbox, &mailbox.control, Mode::Stopping);
    tokio::pin!(discard);
    let mut task = Context::from_waker(Waker::noop());

    assert_eq!(discard.as_mut().poll(&mut task), Poll::Pending);
    assert!(!last_dropped.load(Ordering::SeqCst));
    assert_eq!(
        discard.as_mut().poll(&mut task),
        Poll::Ready(DiscardOutcome::Complete)
    );
    assert!(last_dropped.load(Ordering::SeqCst));
}
