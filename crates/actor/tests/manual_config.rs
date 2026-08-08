use std::mem::{needs_drop, size_of};

use loac::{
    Actor, ActorConfig, ActorScope, ExitReason, MessageConfig, Shutdown, SupervisionConfig,
    scheduling, supervision,
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
// A manual no-mailbox config omits reply runtime state and still runs.
#[tokio::test]
async fn no_mailbox_omits_reply_runtime_state() {
    let (_, _, scheduler) = ManualActor::open(&());
    let _: supervision::Disabled = ManualActor::open_children(&());
    assert_eq!(size_of::<scheduling::Disabled>(), 0);
    assert!(!needs_drop::<scheduling::Disabled>());
    let _: scheduling::Disabled = scheduler;

    let owner = loac::spawn::<ManualActor>(());

    assert_eq!(
        owner.shutdown(Shutdown::Stop).await.reason(),
        ExitReason::Stopped
    );
}

/// A manual no-mailbox actor may still supervise children.
struct Parent;

impl ActorConfig for Parent {
    type Options = ();
}

impl MessageConfig for Parent {
    type Sender = NoSender;
    type Inbox = NoInbox;
    type Scheduler = scheduling::Disabled;

    fn open(_options: &Self::Options) -> (Self::Sender, Self::Inbox, Self::Scheduler) {
        let (sender, inbox) = NoSender::open();
        (sender, inbox, scheduling::Disabled::new())
    }
}

impl SupervisionConfig for Parent {
    type Children = supervision::Fixed<2>;

    fn open_children(_options: &Self::Options) -> Self::Children {
        supervision::Fixed::<2>::new()
    }
}

impl Actor for Parent {
    type SpawnArgs = ();

    async fn init(_: (), scope: &mut ActorScope<'_, Self>) -> Self {
        // No mailbox does not disable child ownership.
        let _child = scope.spawn_child::<ManualActor>(());
        Self
    }
}

// The manual no-mailbox recipe keeps lifecycle and children.
#[tokio::test]
async fn manual_no_mailbox_keeps_lifecycle_and_children() {
    let owner = loac::spawn::<Parent>(());

    // The actor ref keeps lifecycle methods.
    assert!(owner.actor_ref().exit_status().is_none());

    assert_eq!(
        owner.shutdown(Shutdown::Drain).await.reason(),
        ExitReason::Drained
    );
}
