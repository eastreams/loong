// A budget always needs an explicit const expression.
struct KeyOnly;

#[actor_api::actor(mailbox, mailbox_budget)]
impl actor_api::Actor for KeyOnly {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {}
