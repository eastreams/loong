//! Combines fixed, dynamic, and unbounded profiles.
//! The dynamic mailbox limit may change per spawn.
//! The mailbox dispatch budget belongs to the actor type.
//! Unbounded profiles leave resource growth to the application.
//! See [`#[actor(...)]`](macro@loac::actor) for every option.

use std::num::NonZeroUsize;

use loac::{ExitReason, Shutdown, SpawnOptions, prelude::*, spawn_with};

const DEFAULT_MAILBOX_CAPACITY: usize = 32;
const MAILBOX_DISPATCH_BUDGET: usize = 8;
const MAX_IN_FLIGHT: usize = 16;

struct Service;

#[actor(
    mailbox = dynamic(DEFAULT_MAILBOX_CAPACITY),
    mailbox_budget = MAILBOX_DISPATCH_BUDGET,
    interleaved = MAX_IN_FLIGHT,
    children = unbounded,
)]
impl Actor for Service {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
#[message(reply = &'static str)]
struct HealthCheck;

impl SyncHandler<HealthCheck> for Service {
    fn handle(&mut self, _message: HealthCheck, _scope: &mut ActorScope<Self>) -> &'static str {
        "ready"
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = SpawnOptions::<Service>::default()
        .with_mailbox_capacity(NonZeroUsize::new(64).expect("capacity is non-zero"));

    let owner = spawn_with::<Service>((), options);
    assert_eq!(owner.actor_ref().call(HealthCheck).await?, "ready");

    assert_eq!(
        owner.shutdown(Shutdown::Drain).await.reason(),
        ExitReason::Drained
    );
    Ok(())
}
