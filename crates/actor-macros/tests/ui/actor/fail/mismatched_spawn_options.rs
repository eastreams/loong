// Spawn options remain bound to their configured actor type.
struct First;

#[actor_api::actor(mailbox)]
impl actor_api::Actor for First {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct Second;

#[actor_api::actor(mailbox)]
impl actor_api::Actor for Second {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let options = actor_api::SpawnOptions::<First>::default();
    let _ = actor_api::spawn_with::<Second>((), options);
}
