// A zero budget cannot make mailbox progress.
struct Zero;

#[actor_api::actor(mailbox, mailbox_budget = 0)]
impl actor_api::Actor for Zero {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {}
