struct NonIntegerCapacity;

#[actor_api::actor(mailbox = "8")]
impl actor_api::Actor for NonIntegerCapacity {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {}
