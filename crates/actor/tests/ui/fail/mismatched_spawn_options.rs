// Spawn options remain bound to their configured actor type.
struct First;

#[loac::actor(mailbox)]
impl loac::Actor for First {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut loac::ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct Second;

#[loac::actor(mailbox)]
impl loac::Actor for Second {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut loac::ActorScope<'_, Self>) -> Self {
        Self
    }
}

fn main() {
    let options = loac::SpawnOptions::<First>::default();
    let _ = loac::spawn_with::<Second>((), options);
}
