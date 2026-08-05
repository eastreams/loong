// An active scheduler requires public mailbox operations.
use loong_actor::{
    Actor, ActorConfig, ActorScope, MessageConfig, ReplySchedulingConfig, scheduling,
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

    fn open(_options: &Self::Options) -> (Self::Sender, Self::Inbox) {
        NoSender::open()
    }
}

impl ReplySchedulingConfig for Manual {
    type Scheduler = scheduling::Fixed<Self, 1>;

    fn open_scheduler(_options: &Self::Options) -> Self::Scheduler {
        scheduling::Fixed::new()
    }
}

impl Actor for Manual {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {}
