//! Uses `SyncHandler` when a reply is complete during dispatch.

use loac::{ExitReason, Shutdown, prelude::*, spawn};

struct Counter(u64);

#[actor(mailbox)]
impl Actor for Counter {
    type SpawnArgs = u64;

    async fn init(value: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self(value)
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct Add(u64);

impl SyncHandler<Add> for Counter {
    fn handle(&mut self, message: Add, _scope: &mut ActorScope<Self>) -> u64 {
        self.0 += message.0;
        self.0
    }
}

#[derive(Message)]
struct Reset;

impl SyncHandler<Reset> for Counter {
    fn handle(&mut self, _message: Reset, _scope: &mut ActorScope<Self>) {
        self.0 = 0;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owner = spawn::<Counter>(1);
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
