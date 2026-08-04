use std::{
    cell::Cell,
    future::{self, Future},
    marker::PhantomData,
    num::NonZeroUsize,
    rc::Rc,
    task::{Context, Poll},
};

use loong_actor::{
    Actor, ActorConfig, ActorScope, ExitReason, InterleavingConfig, Message, MessageConfig,
    Shutdown, SyncHandler, spawn_with,
    transport::{
        ErasedEnvelope, MessageInbox, MessageReservation, MessageSender, RuntimeInbox,
        TryReserveError,
    },
};
use tokio::sync::mpsc;

struct ManualActor(u64);

struct ManualOptions {
    opened: Rc<Cell<bool>>,
}

impl Default for ManualOptions {
    fn default() -> Self {
        Self {
            opened: Rc::new(Cell::new(false)),
        }
    }
}

struct ManualSender(mpsc::UnboundedSender<ErasedEnvelope<ManualActor>>);

impl Clone for ManualSender {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

struct ManualInbox(mpsc::UnboundedReceiver<ErasedEnvelope<ManualActor>>);

struct ManualReservation<'a> {
    sender: &'a ManualSender,
    _local: PhantomData<Rc<()>>,
}

impl MessageReservation<ManualActor> for ManualReservation<'_> {
    fn enqueue(
        self,
        envelope: ErasedEnvelope<ManualActor>,
    ) -> Result<(), ErasedEnvelope<ManualActor>> {
        self.sender.0.send(envelope).map_err(|error| error.0)
    }
}

impl MessageReservation<ManualActor> for ManualSender {
    fn enqueue(
        self,
        envelope: ErasedEnvelope<ManualActor>,
    ) -> Result<(), ErasedEnvelope<ManualActor>> {
        self.0.send(envelope).map_err(|error| error.0)
    }
}

impl MessageSender<ManualActor> for ManualSender {
    type Reservation<'a> = ManualReservation<'a>;
    type OwnedReservation = Self;

    fn try_reserve(&self) -> Result<Self::Reservation<'_>, TryReserveError> {
        if self.0.is_closed() {
            Err(TryReserveError::Closed)
        } else {
            Ok(ManualReservation {
                sender: self,
                _local: PhantomData,
            })
        }
    }

    fn reserve_owned(&self) -> impl Future<Output = Option<Self::OwnedReservation>> + Send + use<> {
        future::ready((!self.0.is_closed()).then(|| self.clone()))
    }
}

impl RuntimeInbox<ManualActor> for ManualInbox {
    fn poll_recv(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Option<ErasedEnvelope<ManualActor>>> {
        self.0.poll_recv(context)
    }

    fn try_recv(&mut self) -> Option<ErasedEnvelope<ManualActor>> {
        self.0.try_recv().ok()
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn close(&mut self) {
        self.0.close();
    }
}

impl MessageInbox<ManualActor> for ManualInbox {}

impl ActorConfig for ManualActor {
    type Options = ManualOptions;
}

impl MessageConfig for ManualActor {
    type Sender = ManualSender;
    type Inbox = ManualInbox;

    fn open(options: &Self::Options) -> (Self::Sender, Self::Inbox) {
        options.opened.set(true);
        let (sender, inbox) = mpsc::unbounded_channel();
        (ManualSender(sender), ManualInbox(inbox))
    }
}

impl InterleavingConfig for ManualActor {
    fn max_in_flight(_options: &Self::Options) -> NonZeroUsize {
        NonZeroUsize::MIN
    }
}

impl Actor for ManualActor {
    type SpawnArgs = u64;

    async fn init(value: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self(value)
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct Add(u64);

impl SyncHandler<Add> for ManualActor {
    fn handle(&mut self, message: Add, _scope: &mut ActorScope<'_, Self>) -> u64 {
        self.0 += message.0;
        self.0
    }
}

#[derive(Debug, Message)]
struct Notify(u64);

impl SyncHandler<Notify> for ManualActor {
    fn handle(&mut self, message: Notify, _scope: &mut ActorScope<'_, Self>) {
        self.0 += message.0;
    }
}

fn assert_send<T: Send>(_: &T) {}

// This proves manual transports use only stable public extension points.
// The non-Send options must be consumed before Tokio owns the actor task.
#[tokio::test]
async fn manual_transport_round_trips_with_local_spawn_options() {
    let options = ManualOptions::default();
    let opened = Rc::clone(&options.opened);
    let owner = spawn_with::<ManualActor>(2, options);
    assert!(opened.get());

    let actor = owner.actor_ref();
    let call = actor.call(Add(3));
    assert_send(&call);
    assert_eq!(call.await, Ok(5));

    let send = actor.send(Notify(4));
    assert_send(&send);
    send.await.expect("the manual transport remains open");
    assert_eq!(actor.call(Add(0)).await, Ok(9));
    assert_eq!(
        owner.shutdown(Shutdown::Drain).await.reason(),
        ExitReason::Drained
    );
}
