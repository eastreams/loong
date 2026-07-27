use std::{
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use tokio::sync::mpsc;

use crate::{
    Actor, ActorRef, ActorScope, ChildExit, ChildId, ExitReason, IntoActorFuture,
    mailbox::{ActorMailbox, DynEnvelope, Envelope, Mode},
    scheduler::ReplyScheduler,
};

use super::{ChildSet, Turn, TurnCursor, actor_turn};

struct TestActor;

impl Actor for TestActor {}

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
