use std::task::{Context, Poll};

use loong_actor::{
    Actor, ActorConfig, ActorScope, ExitReason, MessageConfig, ReplySchedulingConfig, Shutdown,
    SupervisionConfig, scheduling, spawn, supervision,
    transport::{ErasedEnvelope, RuntimeInbox},
};

struct ManualActor;

#[derive(Default)]
struct ManualInbox {
    closed: bool,
}

impl<A: Actor> RuntimeInbox<A> for ManualInbox {
    fn poll_recv(&mut self, _task: &mut Context<'_>) -> Poll<Option<ErasedEnvelope<A>>> {
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

impl ActorConfig for ManualActor {
    type Options = ();
}

impl MessageConfig for ManualActor {
    type Sender = ();
    type Inbox = ManualInbox;

    fn open(_options: &Self::Options) -> (Self::Sender, Self::Inbox) {
        ((), ManualInbox::default())
    }
}

impl ReplySchedulingConfig for ManualActor {
    type Scheduler = scheduling::Serial<Self>;

    fn open_scheduler(_options: &Self::Options) -> Self::Scheduler {
        scheduling::Serial::new()
    }
}

impl SupervisionConfig for ManualActor {
    type Children = supervision::Disabled;

    fn open_children(_options: &Self::Options) -> Self::Children {
        supervision::Disabled::new()
    }
}

impl Actor for ManualActor {
    type SpawnArgs = ();

    async fn init(_: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

// Manual configs can pair no mailbox with the serial scheduler.
#[tokio::test]
async fn no_mailbox_uses_the_serial_scheduler() {
    let _: scheduling::Serial<ManualActor> = ManualActor::open_scheduler(&());
    let _: supervision::Disabled = ManualActor::open_children(&());
    let owner = spawn::<ManualActor>(());

    assert_eq!(
        owner.shutdown(Shutdown::Stop).await.reason(),
        ExitReason::Stopped
    );
}
