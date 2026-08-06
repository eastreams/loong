use std::{
    cell::Cell,
    future::{self, Future},
    marker::PhantomData,
    rc::Rc,
    task::{Context, Poll},
};

use loac::{
    Actor, ActorConfig, ActorFuture, ActorFutureExt, ActorScope, ExitReason, Handler, HasChildren,
    HasInterleaving, InterleavedFutureExt, IntoActorFuture, IntoReply, Message, MessageConfig,
    Shutdown, SupervisionConfig, SyncHandler, scheduling, spawn_with, supervision,
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

struct ManualSender<A: Actor>(mpsc::UnboundedSender<ErasedEnvelope<A>>);

impl<A: Actor> ManualSender<A> {
    fn open() -> (Self, ManualInbox<A>) {
        let (sender, inbox) = mpsc::unbounded_channel();
        (Self(sender), ManualInbox(inbox))
    }
}

impl<A: Actor> Clone for ManualSender<A> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

struct ManualInbox<A: Actor>(mpsc::UnboundedReceiver<ErasedEnvelope<A>>);

struct ManualReservation<'a, A: Actor> {
    sender: &'a ManualSender<A>,
    _local: PhantomData<Rc<()>>,
}

impl<A: Actor> MessageReservation<A> for ManualReservation<'_, A> {
    fn enqueue(self, envelope: ErasedEnvelope<A>) -> Result<(), ErasedEnvelope<A>> {
        self.sender.0.send(envelope).map_err(|error| error.0)
    }
}

impl<A: Actor> MessageReservation<A> for ManualSender<A> {
    fn enqueue(self, envelope: ErasedEnvelope<A>) -> Result<(), ErasedEnvelope<A>> {
        self.0.send(envelope).map_err(|error| error.0)
    }
}

impl<A: Actor> MessageSender<A> for ManualSender<A> {
    type Reservation<'a> = ManualReservation<'a, A>;
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

    fn reserve_owned(
        &self,
    ) -> impl Future<Output = Option<Self::OwnedReservation>> + Send + use<A> {
        future::ready((!self.0.is_closed()).then(|| self.clone()))
    }
}

impl<A: Actor> RuntimeInbox<A> for ManualInbox<A> {
    fn poll_recv(&mut self, context: &mut Context<'_>) -> Poll<Option<ErasedEnvelope<A>>> {
        self.0.poll_recv(context)
    }

    fn try_recv(&mut self) -> Option<ErasedEnvelope<A>> {
        self.0.try_recv().ok()
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn close(&mut self) {
        self.0.close();
    }
}

impl<A: Actor> MessageInbox<A> for ManualInbox<A> {}

impl ActorConfig for ManualActor {
    type Options = ManualOptions;
}

impl MessageConfig for ManualActor {
    type Sender = ManualSender<Self>;
    type Inbox = ManualInbox<Self>;
    type Scheduler = scheduling::Fixed<Self, 1>;

    fn open(options: &Self::Options) -> (Self::Sender, Self::Inbox, Self::Scheduler) {
        options.opened.set(true);
        let (sender, inbox) = ManualSender::open();
        (sender, inbox, scheduling::Fixed::new())
    }
}

impl SupervisionConfig for ManualActor {
    type Children = supervision::Fixed<1>;

    fn open_children(_options: &Self::Options) -> Self::Children {
        supervision::Fixed::new()
    }
}

impl Actor for ManualActor {
    type SpawnArgs = u64;

    async fn init(value: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self(value)
    }
}

struct ManualSerial;

impl ActorConfig for ManualSerial {
    type Options = ();
}

impl MessageConfig for ManualSerial {
    type Sender = ManualSender<Self>;
    type Inbox = ManualInbox<Self>;
    type Scheduler = scheduling::Serial<Self>;

    fn open(_options: &Self::Options) -> (Self::Sender, Self::Inbox, Self::Scheduler) {
        let (sender, inbox) = ManualSender::open();
        (sender, inbox, scheduling::Serial::new())
    }
}

impl SupervisionConfig for ManualSerial {
    type Children = supervision::Disabled;

    fn open_children(_options: &Self::Options) -> Self::Children {
        supervision::Disabled::new()
    }
}

impl Actor for ManualSerial {
    type SpawnArgs = ();

    async fn init(_: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct ManualDynamic;

impl ActorConfig for ManualDynamic {
    type Options = ();
}

impl MessageConfig for ManualDynamic {
    type Sender = ManualSender<Self>;
    type Inbox = ManualInbox<Self>;
    type Scheduler = scheduling::Dynamic<Self>;

    fn open(_options: &Self::Options) -> (Self::Sender, Self::Inbox, Self::Scheduler) {
        let (sender, inbox) = ManualSender::open();
        (
            sender,
            inbox,
            scheduling::Dynamic::new(std::num::NonZeroUsize::MIN),
        )
    }
}

impl SupervisionConfig for ManualDynamic {
    type Children = supervision::Dynamic;

    fn open_children(_options: &Self::Options) -> Self::Children {
        supervision::Dynamic::new(std::num::NonZeroUsize::MIN)
    }
}

impl Actor for ManualDynamic {
    type SpawnArgs = ();

    async fn init(_: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct ManualUnbounded;

impl ActorConfig for ManualUnbounded {
    type Options = ();
}

impl MessageConfig for ManualUnbounded {
    type Sender = ManualSender<Self>;
    type Inbox = ManualInbox<Self>;
    type Scheduler = scheduling::Unbounded<Self>;

    fn open(_options: &Self::Options) -> (Self::Sender, Self::Inbox, Self::Scheduler) {
        let (sender, inbox) = ManualSender::open();
        (sender, inbox, scheduling::Unbounded::new())
    }
}

impl SupervisionConfig for ManualUnbounded {
    type Children = supervision::Unbounded;

    fn open_children(_options: &Self::Options) -> Self::Children {
        supervision::Unbounded::new()
    }
}

impl Actor for ManualUnbounded {
    type SpawnArgs = ();

    async fn init(_: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct Add(u64);

impl Handler<Add> for ManualActor {
    fn handle(
        &mut self,
        message: Add,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl IntoReply<Self, Add> + use<> {
        generic_interleaved_reply(std::future::ready(message.0).into_actor().map(
            |amount, actor: &mut Self, _scope| {
                actor.0 += amount;
                actor.0
            },
        ))
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
fn assert_has_children<A: HasChildren>() {}
fn assert_has_interleaving<A: HasInterleaving>() {}

fn generic_interleaved_reply<A, M, F>(future: F) -> impl IntoReply<A, M>
where
    A: HasInterleaving,
    M: Message,
    F: ActorFuture<A, Output = M::Reply> + Send + 'static,
{
    future.interleaved()
}

// A manual transport may select every active scheduler profile.
#[test]
fn manual_transport_selects_each_public_scheduler() {
    let options = ManualOptions::default();
    let (_, _, fixed): (_, _, scheduling::Fixed<ManualActor, 1>) = ManualActor::open(&options);
    let (_, _, serial): (_, _, scheduling::Serial<ManualSerial>) = ManualSerial::open(&());
    let (_, _, dynamic): (_, _, scheduling::Dynamic<ManualDynamic>) = ManualDynamic::open(&());
    let (_, _, unbounded): (_, _, scheduling::Unbounded<ManualUnbounded>) =
        ManualUnbounded::open(&());
    drop((fixed, serial, dynamic, unbounded));
}

// Manual configs can select every built-in supervision profile.
#[test]
fn manual_configs_select_each_public_supervisor() {
    let options = ManualOptions::default();
    let _: supervision::Fixed<1> = ManualActor::open_children(&options);
    let _: supervision::Disabled = ManualSerial::open_children(&());
    let _: supervision::Dynamic = ManualDynamic::open_children(&());
    let _: supervision::Unbounded = ManualUnbounded::open_children(&());

    assert_has_children::<ManualActor>();
    assert_has_children::<ManualDynamic>();
    assert_has_children::<ManualUnbounded>();
}

// This proves manual configs use stable public extension points.
// The non-Send options must be consumed before Tokio owns the actor task.
#[tokio::test]
async fn manual_transport_round_trips_with_local_spawn_options() {
    assert_has_interleaving::<ManualActor>();
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
