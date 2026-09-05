//! Uses `SyncHandler` when a reply is complete during dispatch.

use loac::prelude::*;

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

#[loac::sync_handler]
impl SyncHandler<Add> for Counter {
    fn handle(&mut self, message: Add, _scope: &mut ActorScope<Self>) -> u64 {
        self.0 += message.0;
        self.0
    }
}

#[derive(Message)]
#[message(reply = ())]
struct Reset;

#[loac::sync_handler]
impl SyncHandler<Reset> for Counter {
    fn handle(&mut self, _message: Reset, _scope: &mut ActorScope<Self>) {
        self.0 = 0;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owner = loac::spawn::<Counter>(1);

    assert_eq!(owner.call(Add(2)).await?, 3);
    owner.send(Reset).await?;
    assert_eq!(owner.call(Add(4)).await?, 4);

    assert_eq!(
        owner.shutdown(loac::Shutdown::Drain).await.reason(),
        loac::ExitReason::Drained
    );
    Ok(())
}
