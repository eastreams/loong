//! A message without an explicit reply is send-only; `call` must not compile.
use loac::{Actor, ActorScope, Cx, Handler};

struct Echo;

#[loac::actor(mailbox, interleaved = unbounded)]
impl Actor for Echo {
    type SpawnArgs = ();

    async fn init(_: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(loac::Message)]
struct Ping;

impl Handler<Ping> for Echo {
    async fn handle(_message: Ping, _cx: Cx<'_, Self>) {}
}

fn main() {
    let owner = loac::spawn::<Echo>(());
    let actor_ref = owner.actor_ref();
    let _ = actor_ref.call(Ping);
    let recipient = actor_ref.recipient::<Ping>();
    let _ = recipient.call(Ping);
    let _ = actor_ref.send(Ping);
}
