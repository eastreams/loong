// Serial actors cannot select interleaved scheduling.
use loac::prelude::*;
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
#[message(raw = ())]
struct Ping;

struct Pending;

impl loac::ActorFuture<Serial> for Pending {
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

impl RawHandler<Ping> for Serial {
    fn handle(
        &mut self,
        _message: Ping,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl IntoReply<Self, Ping> + use<> {
        Pending.interleaved()
    }
}

fn main() {}
