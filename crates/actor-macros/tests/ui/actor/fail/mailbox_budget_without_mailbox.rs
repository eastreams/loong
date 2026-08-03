// A dispatch budget has no meaning without public messaging.
struct NoMailbox;

#[actor_api::actor(mailbox_budget = 4)]
impl actor_api::Actor for NoMailbox {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {}
