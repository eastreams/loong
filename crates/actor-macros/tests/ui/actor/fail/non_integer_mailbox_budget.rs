// A budget must have an integer const type.
struct NonInteger;

#[actor_api::actor(mailbox, mailbox_budget = "8")]
impl actor_api::Actor for NonInteger {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {}
