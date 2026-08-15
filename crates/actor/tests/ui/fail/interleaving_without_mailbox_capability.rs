// An active scheduler requires public mailbox operations.
use loac::{
    Actor, ActorConfig, ActorScope, MessageConfig, SupervisionConfig, scheduling, supervision,
    transport::{NoInbox, NoSender},
};

struct Manual;

#[derive(Default)]
struct Options;

impl ActorConfig for Manual {
    type Options = Options;
}

impl MessageConfig for Manual {
    type Sender = NoSender;
    type Inbox = NoInbox;
    type Scheduler = scheduling::Fixed<Self, 1>;

    fn open(_options: &Self::Options) -> (Self::Sender, Self::Inbox, Self::Scheduler) {
        let (sender, inbox) = NoSender::open();
        (sender, inbox, scheduling::Fixed::new())
    }
}

impl SupervisionConfig for Manual {
    type Children = supervision::Disabled;

    fn open_children(_options: &Self::Options) -> Self::Children {
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
