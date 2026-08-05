// Spawn options remain bound to their configured actor type.
struct First;

#[loong_actor::actor(mailbox)]
impl loong_actor::Actor for First {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut loong_actor::ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct Second;

#[loong_actor::actor(mailbox)]
impl loong_actor::Actor for Second {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut loong_actor::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let options = loong_actor::SpawnOptions::<First>::default();
    let _ = loong_actor::spawn_with::<Second>((), options);
}
