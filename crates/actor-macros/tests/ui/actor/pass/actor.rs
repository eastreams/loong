// The runtime dependency uses a Cargo alias.
// Both attribute forms must resolve that alias.
use actor_api::{ActorConfig, prelude::*};

struct Bare;

#[actor]
impl Actor for Bare {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct Messaging;

#[actor_api::actor(mailbox = 8)]
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

mod limits {
    pub const CHILD_CAPACITY: usize = 8;
}

struct Combined;

#[actor_api::actor(
    mailbox = MAILBOX_CAPACITY,
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

#[actor_api::actor(mailbox = N, interleaved = dynamic(N), children = N)]
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

#[actor_api::actor]
#[cfg(any())]
impl Actor for Disabled {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn assert_config<A: ActorConfig>() {}

fn main() {
    assert_config::<Bare>();
    assert_config::<Messaging>();
    assert_config::<Supervisor>();
    assert_config::<DynamicMailbox>();
    assert_config::<DefaultCapabilities>();
    assert_config::<Combined>();
    assert_config::<Generic<u8, 6>>();
}
