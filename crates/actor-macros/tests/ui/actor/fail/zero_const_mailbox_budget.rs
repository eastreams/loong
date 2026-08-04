// Const paths are checked when their associated const is read.
const ZERO: usize = 0;

struct ConstZero;

#[actor_api::actor(mailbox, mailbox_budget = ZERO)]
impl actor_api::Actor for ConstZero {
    type SpawnArgs = ();

    async fn init(_: (), _: &mut actor_api::ActorScope<'_, Self>) -> Self {
        Self
    }
}

const _: usize =
    <ConstZero as actor_api::MessageConfig>::MAILBOX_DISPATCH_BUDGET.get();

fn main() {}
