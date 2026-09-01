mod support;

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use loac::{
    Actor, ActorFuture, ActorScope, ExitReason, InterleavedFutureExt, IntoReply, Message,
    RawHandler, Shutdown, actor,
};

use support::watchdog;

#[derive(Message)]
#[message(raw = u8)]
struct Read;

struct FirstActor(u8);

#[actor(mailbox, interleaved)]
impl Actor for FirstActor {
    type SpawnArgs = u8;

    async fn init(value: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self(value)
    }
}

struct SecondActor(u8);

#[actor(mailbox, interleaved)]
impl Actor for SecondActor {
    type SpawnArgs = u8;

    async fn init(value: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self(value)
    }
}

struct SharedFuture;

impl ActorFuture<FirstActor> for SharedFuture {
    type Output = u8;

    fn poll(
        self: Pin<&mut Self>,
        actor: &mut FirstActor,
        _scope: &mut ActorScope<FirstActor>,
        _task: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        Poll::Ready(actor.0)
    }
}

impl ActorFuture<SecondActor> for SharedFuture {
    type Output = u8;

    fn poll(
        self: Pin<&mut Self>,
        actor: &mut SecondActor,
        _scope: &mut ActorScope<SecondActor>,
        _task: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        Poll::Ready(actor.0)
    }
}

impl RawHandler<Read> for FirstActor {
    fn handle(
        &mut self,
        _message: Read,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, Read> + use<> {
        SharedFuture.interleaved()
    }
}

impl RawHandler<Read> for SecondActor {
    fn handle(
        &mut self,
        _message: Read,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, Read> + use<> {
        SharedFuture.interleaved()
    }
}

#[tokio::test]
async fn handler_context_selects_the_actor_future_implementation() {
    // One concrete future implements ActorFuture for both actors. Driving both
    // handlers in one expression ensures each IntoReply context selects its
    // actor-specific implementation without an explicit type annotation.
    let first_owner = loac::spawn::<FirstActor>(1);
    let second_owner = loac::spawn::<SecondActor>(2);
    let first = first_owner.actor_ref();
    let second = second_owner.actor_ref();

    let (first_reply, second_reply) =
        tokio::join!(watchdog(first.call(Read)), watchdog(second.call(Read)),);
    assert_eq!(first_reply, Ok(1));
    assert_eq!(second_reply, Ok(2));

    let (first_exit, second_exit) = tokio::join!(
        watchdog(first_owner.shutdown(Shutdown::Stop)),
        watchdog(second_owner.shutdown(Shutdown::Stop)),
    );
    assert_eq!(first_exit.reason(), ExitReason::Stopped);
    assert_eq!(second_exit.reason(), ExitReason::Stopped);
}
