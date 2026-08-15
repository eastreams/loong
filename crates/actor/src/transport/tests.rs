use std::{
    num::NonZeroUsize,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use crate::{
    Actor, ActorScope,
    mailbox::{ActorInner, Control, Envelope},
    owned::OwnedTasks,
    scheduling::ActorScheduler,
};

use super::{
    BoundedSender, ErasedEnvelope, MessageReservation, MessageSender, NoInbox, NoSender,
    RuntimeInbox, TryReserveError, UnboundedSender,
};

struct TestActor;

#[crate::actor]
impl Actor for TestActor {
    type SpawnArgs = ();

    async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct ProbeEnvelope {
    id: usize,
    discarded: Arc<Mutex<Vec<usize>>>,
}

impl Envelope<TestActor> for ProbeEnvelope {
    fn dispatch(
        self: Box<Self>,
        _actor: &mut TestActor,
        _scope: &mut ActorScope<TestActor>,
        _owned: &OwnedTasks<TestActor>,
        _scheduler: &mut ActorScheduler<TestActor>,
        _inner: &Arc<ActorInner<TestActor>>,
    ) {
        panic!("transport tests never dispatch carriers");
    }

    fn discard(self: Box<Self>, _control: &Control) {
        self.discarded
            .lock()
            .expect("the probe lock remains healthy")
            .push(self.id);
    }
}

fn envelope(id: usize, discarded: &Arc<Mutex<Vec<usize>>>) -> ErasedEnvelope<TestActor> {
    ErasedEnvelope::new(Box::new(ProbeEnvelope {
        id,
        discarded: Arc::clone(discarded),
    }))
}

#[test]
fn bounded_reservations_hold_capacity_until_enqueue() {
    let (sender, mut inbox) =
        BoundedSender::<TestActor>::open(NonZeroUsize::new(1).expect("one is nonzero"));
    let reservation = sender.try_reserve().expect("the first slot is available");
    assert!(matches!(sender.try_reserve(), Err(TryReserveError::Full)));

    let discarded = Arc::new(Mutex::new(Vec::new()));
    assert!(reservation.enqueue(envelope(1, &discarded)).is_ok());
    let accepted = inbox.try_recv().expect("one carrier was accepted");
    accepted.discard(&Control::new());

    assert!(sender.try_reserve().is_ok());
    assert_eq!(
        *discarded.lock().expect("the probe lock remains healthy"),
        vec![1]
    );
}

#[test]
fn unbounded_enqueue_returns_ownership_after_close() {
    let (sender, mut inbox) = UnboundedSender::<TestActor>::open();
    let reservation = sender.try_reserve().expect("the inbox starts open");
    inbox.close();
    let discarded = Arc::new(Mutex::new(Vec::new()));

    let carrier = reservation
        .enqueue(envelope(7, &discarded))
        .expect_err("closure returns the original carrier");
    carrier.discard(&Control::new());

    assert_eq!(
        *discarded.lock().expect("the probe lock remains healthy"),
        vec![7]
    );
}

#[test]
fn absent_inbox_never_reports_message_capability() {
    let mut inbox = NoInbox;
    let mut task = Context::from_waker(std::task::Waker::noop());

    assert_eq!(std::mem::size_of::<NoSender>(), 0);
    assert_eq!(std::mem::size_of::<NoInbox>(), 0);
    assert!(matches!(
        <NoInbox as RuntimeInbox<TestActor>>::poll_recv(&mut inbox, &mut task),
        Poll::Pending
    ));
    assert!(<NoInbox as RuntimeInbox<TestActor>>::try_recv(&mut inbox).is_none());
    assert!(<NoInbox as RuntimeInbox<TestActor>>::is_empty(&inbox));
}
