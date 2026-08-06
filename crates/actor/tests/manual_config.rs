use std::mem::{needs_drop, size_of};

use loong_actor::{
    Actor, ActorConfig, ActorScope, ExitReason, MessageConfig, Shutdown, SupervisionConfig,
    scheduling, spawn, supervision,
    transport::{NoInbox, NoSender},
};

struct ManualActor;

impl ActorConfig for ManualActor {
    type Options = ();
}

impl MessageConfig for ManualActor {
    type Sender = NoSender;
    type Inbox = NoInbox;
    type Scheduler = scheduling::Disabled;

    fn open(_options: &Self::Options) -> (Self::Sender, Self::Inbox, Self::Scheduler) {
        let (sender, inbox) = NoSender::open();
        (sender, inbox, scheduling::Disabled::new())
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

// Manual configs can omit the complete reply scheduler state.
#[tokio::test]
async fn no_mailbox_omits_reply_runtime_state() {
    let (_, _, scheduler) = ManualActor::open(&());
    let _: supervision::Disabled = ManualActor::open_children(&());
    assert_eq!(size_of::<scheduling::Disabled>(), 0);
    assert!(!needs_drop::<scheduling::Disabled>());
    let _: scheduling::Disabled = scheduler;

    let owner = spawn::<ManualActor>(());

    assert_eq!(
        owner.shutdown(Shutdown::Stop).await.reason(),
        ExitReason::Stopped
    );
}
