use loac::{Actor, ActorScope, HasMailbox};

// Transport capability supplies this implementation automatically.
struct Mailboxed;

#[loac::actor(mailbox)]
impl Actor for Mailboxed {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

impl HasMailbox for Mailboxed {}

fn main() {}
