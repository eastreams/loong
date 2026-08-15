// A dynamic mailbox default cannot have zero capacity.
struct Zero;

#[actor_api::actor(mailbox = dynamic(0))]
impl actor_api::Actor for Zero {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {}
