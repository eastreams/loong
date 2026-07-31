//! Uses `SyncHandler` when a reply is complete during dispatch.

use loong_actor::{ExitReason, Shutdown, prelude::*, spawn};

struct Counter(u64);

impl Actor for Counter {}

struct Add(u64);

impl Message for Add {
    type Reply = u64;
}

impl SyncHandler<Add> for Counter {
    fn handle(&mut self, message: Add, _scope: &mut ActorScope<Self>) -> u64 {
        self.0 += message.0;
        self.0
    }
}

struct Reset;

impl Message for Reset {
    type Reply = ();
}

impl SyncHandler<Reset> for Counter {
    fn handle(&mut self, _message: Reset, _scope: &mut ActorScope<Self>) {
        self.0 = 0;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owner = spawn(Counter(1));
    let counter = owner.actor_ref();

    assert_eq!(counter.call(Add(2)).await?, 3);
    counter.send(Reset).await?;
    assert_eq!(counter.call(Add(4)).await?, 4);

    assert_eq!(
        owner.shutdown(Shutdown::Drain).await.reason(),
        ExitReason::Drained
    );
    Ok(())
}
