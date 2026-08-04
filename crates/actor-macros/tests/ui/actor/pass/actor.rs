// The runtime dependency uses a Cargo alias.
// Both attribute forms must resolve that alias.
use std::num::NonZeroUsize;

use actor_api::{
    ActorConfig, DynamicMailboxOptions, InterleavingConfig, MessageConfig, prelude::*,
};

struct Bare;

#[actor]
impl Actor for Bare {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct Messaging;

#[actor_api::actor(mailbox = 8, mailbox_budget = 3)]
impl Actor for Messaging {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct Supervisor;

#[actor_api::actor(children = unbounded)]
impl Actor for Supervisor {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct DynamicMailbox;

#[actor_api::actor(mailbox = dynamic)]
impl Actor for DynamicMailbox {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct DefaultCapabilities;

#[actor_api::actor(mailbox, children, interleaved)]
impl Actor for DefaultCapabilities {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

const MAILBOX_CAPACITY: usize = 16;
const MAILBOX_BUDGET: usize = 5;

mod limits {
    pub const CHILD_CAPACITY: usize = 8;
}

struct Combined;

#[actor_api::actor(
    mailbox = MAILBOX_CAPACITY,
    mailbox_budget = MAILBOX_BUDGET,
    interleaved = 1 << 2,
    children = dynamic(limits::CHILD_CAPACITY),
)]
impl Actor for Combined {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct Generic<T, const N: usize>(T);

#[actor_api::actor(
    mailbox = dynamic(N),
    mailbox_budget = N,
    interleaved = dynamic(N),
    children = N,
)]
impl<T, const N: usize> Actor for Generic<T, N>
where
    T: Send + 'static,
{
    type SpawnArgs = T;

    async fn init(value: T, _: &mut ActorScope<'_, Self>) -> Self {
        Self(value)
    }
}

// Generated companion items must inherit conditional compilation.
#[cfg(any())]
struct Disabled;

#[actor_api::actor(mailbox = dynamic)]
#[cfg(any())]
impl Actor for Disabled {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn assert_config<A>()
where
    A: ActorConfig + InterleavingConfig + MessageConfig,
{
}

fn override_mailbox_capacity<A>(options: A::Options) -> A::Options
where
    A: ActorConfig,
    A::Options: DynamicMailboxOptions,
{
    options.with_mailbox_capacity(NonZeroUsize::MIN)
}

fn main() {
    assert_config::<Bare>();
    assert_config::<Messaging>();
    assert_config::<Supervisor>();
    assert_config::<DynamicMailbox>();
    assert_config::<DefaultCapabilities>();
    assert_config::<Combined>();
    assert_config::<Generic<u8, 6>>();
    assert_eq!(Messaging::MAILBOX_DISPATCH_BUDGET.get(), 3);
    assert_eq!(Combined::MAILBOX_DISPATCH_BUDGET.get(), MAILBOX_BUDGET);
    assert_eq!(Generic::<u8, 6>::MAILBOX_DISPATCH_BUDGET.get(), 6);

    let _ = actor_api::SpawnOptions::<DynamicMailbox>::default()
        .with_mailbox_capacity(NonZeroUsize::MIN);
    let _ = override_mailbox_capacity::<Generic<u8, 6>>(
        actor_api::SpawnOptions::<Generic<u8, 6>>::default(),
    );
}
