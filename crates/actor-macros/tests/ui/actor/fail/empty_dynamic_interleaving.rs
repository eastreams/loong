// Parentheses must provide one interleaved default.
struct Empty;

#[actor_api::actor(mailbox, interleaved = dynamic())]
impl actor_api::Actor for Empty {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {}
