// A disabled transport cannot select an active scheduler profile.
use loac::{
    Actor, ActorConfig, ActorScope, MessageConfig, SupervisionConfig, scheduling, supervision,
    transport::{NoInbox, NoSender},
};

struct Manual;

impl ActorConfig for Manual {
    type Options = ();
}

impl MessageConfig for Manual {
    type Sender = NoSender;
    type Inbox = NoInbox;
    type Scheduler = scheduling::Serial<Self>;

    fn open(_: &Self::Options) -> (Self::Sender, Self::Inbox, Self::Scheduler) {
        let (sender, inbox) = NoSender::open();
        (sender, inbox, scheduling::Serial::new())
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
