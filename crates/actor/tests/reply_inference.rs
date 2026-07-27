mod support;

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use loong_actor::{
    Actor, ActorFuture, ActorScope, ExitReason, Handler, IntoReply, Message, Shutdown, reply, spawn,
};

use support::watchdog;

struct Read;

impl Message for Read {
    type Reply = u8;
}

struct FirstActor(u8);

impl Actor for FirstActor {}

struct SecondActor(u8);

impl Actor for SecondActor {}

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

impl Handler<Read> for FirstActor {
    fn handle(
        &mut self,
        _message: Read,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, Read> + use<> {
        reply::interleaved(SharedFuture)
    }
}

impl Handler<Read> for SecondActor {
    fn handle(
        &mut self,
        _message: Read,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, Read> + use<> {
        reply::interleaved(SharedFuture)
    }
}

#[tokio::test]
async fn handler_context_selects_the_actor_future_implementation() {
    let first_owner = spawn(FirstActor(1));
    let second_owner = spawn(SecondActor(2));
    let first = first_owner.actor_ref();
    let second = second_owner.actor_ref();

    assert_eq!(watchdog(first.call(Read)).await, Ok(1));
    assert_eq!(watchdog(second.call(Read)).await, Ok(2));
    assert_eq!(
        watchdog(first_owner.shutdown(Shutdown::Stop)).await,
        ExitReason::Stopped
    );
    assert_eq!(
        watchdog(second_owner.shutdown(Shutdown::Stop)).await,
        ExitReason::Stopped
    );
}
