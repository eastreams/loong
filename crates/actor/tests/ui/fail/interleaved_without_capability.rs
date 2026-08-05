// Serial actors cannot select interleaved scheduling.
use loong_actor::prelude::*;
use std::{
    pin::Pin,
    task::{Context, Poll},
};

struct Serial;

#[actor(mailbox)]
impl Actor for Serial {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
struct Ping;

struct Pending;

impl loong_actor::ActorFuture<Serial> for Pending {
    type Output = ();

    fn poll(
        self: Pin<&mut Self>,
        _actor: &mut Serial,
        _scope: &mut ActorScope<'_, Serial>,
        _task: &mut Context<'_>,
    ) -> Poll<()> {
        Poll::Pending
    }
}

impl Handler<Ping> for Serial {
    fn handle(
        &mut self,
        _message: Ping,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl IntoReply<Self, Ping> + use<> {
        Pending.interleaved()
    }
}

fn main() {}
