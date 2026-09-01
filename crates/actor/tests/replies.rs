mod support;

use std::{
    future::Future,
    num::NonZeroUsize,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};

use loac::{
    Actor, ActorFutureExt, ActorRef, ActorScope, CallError, ChildExit, ExitReason,
    InterleavedFutureExt, IntoActorFuture, Message, RawHandler, ReplyExt, Response, Shutdown,
    SpawnOptions, actor, reply, spawn_with,
};
use tokio::sync::{mpsc, oneshot};

use support::{lock, watchdog};

async fn poll_once<F: Future>(mut future: Pin<&mut F>) -> Poll<F::Output> {
    std::future::poll_fn(|task| Poll::Ready(future.as_mut().poll(task))).await
}

#[derive(Message)]
#[message(raw = ())]
struct PendingOwned {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

struct HookChild;

#[actor(mailbox)]
impl Actor for HookChild {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
#[message(raw = ())]
struct StopChild;

impl RawHandler<StopChild> for HookChild {
    fn handle(
        &mut self,
        _message: StopChild,
        scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, StopChild> + use<> {
        scope.request_shutdown(Shutdown::Stop);
        ().ready()
    }
}

#[derive(Message)]
#[message(raw = ())]
struct ExclusiveGate {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

#[path = "replies/basic.rs"]
mod basic;
#[path = "replies/cancellation.rs"]
mod cancellation;
#[path = "replies/cx_exclusive.rs"]
mod cx_exclusive;
#[path = "replies/exclusive.rs"]
mod exclusive;
#[path = "replies/fairness.rs"]
mod fairness;
#[path = "replies/owned.rs"]
mod owned;
#[path = "replies/panic.rs"]
mod panic;
#[path = "replies/self_call.rs"]
mod self_call;
#[path = "replies/stop.rs"]
mod stop;
