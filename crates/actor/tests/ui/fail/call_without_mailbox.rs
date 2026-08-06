// A no-mailbox actor cannot call or send through its ref.
use loong_actor::{Actor, ActorScope, spawn};

struct Bare;

#[loong_actor::actor]
impl Actor for Bare {
    type SpawnArgs = ();

    async fn init(_: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(loong_actor::Message)]
struct Ping;

fn main() {
    let owner = spawn::<Bare>(());
    let actor_ref = owner.actor_ref();
    let _ = actor_ref.call(Ping);
    let _ = actor_ref.send(Ping);
}
