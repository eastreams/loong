// A real mailbox cannot select the disabled scheduler profile.
use loac::{
    Actor, ActorConfig, ActorScope, MessageConfig, SupervisionConfig, scheduling, supervision,
    transport::{UnboundedInbox, UnboundedSender},
};

struct Manual;

impl ActorConfig for Manual {
    type Options = ();
}

impl MessageConfig for Manual {
    type Sender = UnboundedSender<Self>;
    type Inbox = UnboundedInbox<Self>;
    type Scheduler = scheduling::Disabled;

    fn open(_: &Self::Options) -> (Self::Sender, Self::Inbox, Self::Scheduler) {
        let (sender, inbox) = UnboundedSender::open();
        (sender, inbox, scheduling::Disabled::new())
    }
}

impl SupervisionConfig for Manual {
    type Children = supervision::Disabled;

    fn open_children(_: &Self::Options) -> Self::Children {
        supervision::Disabled::new()
    }
}

impl Actor for Manual {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {}
