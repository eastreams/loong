use std::task::{Context, Poll};

use loong_actor::{
    Actor, ActorConfig, ActorScope, MessageConfig, ReplySchedulingConfig,
    scheduling::ReplyScheduler,
    transport::{ErasedEnvelope, RuntimeInbox},
};

struct ForeignScheduler;
struct ForeignActor;
struct ForeignInbox {
    closed: bool,
}

impl<A: Actor> RuntimeInbox<A> for ForeignInbox {
    fn poll_recv(&mut self, _: &mut Context<'_>) -> Poll<Option<ErasedEnvelope<A>>> {
        if self.closed {
            Poll::Ready(None)
        } else {
            Poll::Pending
        }
    }

    fn try_recv(&mut self) -> Option<ErasedEnvelope<A>> {
        None
    }

    fn is_empty(&self) -> bool {
        true
    }

    fn close(&mut self) {
        self.closed = true;
    }
}

impl ActorConfig for ForeignActor {
    type Options = ();
}

impl MessageConfig for ForeignActor {
    type Sender = ();
    type Inbox = ForeignInbox;

    fn open(_: &Self::Options) -> (Self::Sender, Self::Inbox) {
        ((), ForeignInbox { closed: false })
    }
}

impl ReplySchedulingConfig for ForeignActor {
    type Scheduler = ForeignScheduler;

    fn open_scheduler(_: &Self::Options) -> Self::Scheduler {
        ForeignScheduler
    }
}

impl<A> ReplyScheduler<A> for ForeignScheduler
where
    A: Actor + ReplySchedulingConfig<Scheduler = ForeignScheduler>,
{
}

// A manual config must select a built-in scheduler profile.
impl Actor for ForeignActor {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {}
