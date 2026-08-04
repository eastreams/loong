// A dynamic default cannot represent an unusable zero limit.
struct Zero;

#[actor_api::actor(mailbox = dynamic(0))]
impl actor_api::Actor for Zero {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {}
