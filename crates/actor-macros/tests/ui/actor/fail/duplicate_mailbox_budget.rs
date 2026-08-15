// Repeated budgets must not silently select one cutoff.
struct Duplicate;

#[actor_api::actor(mailbox, mailbox_budget = 4, mailbox_budget = 8)]
impl actor_api::Actor for Duplicate {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {}
