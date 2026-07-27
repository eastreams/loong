use std::{
    future::Future,
    num::NonZeroUsize,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};

use tokio::sync::mpsc;

use crate::{
    Actor, ActorFuture, ActorRef, ActorScope, ChildExit, ChildId, ExitReason, IntoActorFuture,
    Shutdown, ShutdownStatus, SpawnOptions,
    mailbox::{ActorMailbox, Control, DynEnvelope, Envelope, Mode},
    scheduler::ReplyScheduler,
};

use super::{ChildSet, Turn, TurnCursor, actor_turn, kill_actor, spawn_actor};

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
