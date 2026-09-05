//! Uses `DispatchHandler` when a reply is complete during dispatch.

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

impl DispatchHandler<Add> for Counter {
    fn handle(
        &mut self,
        message: Add,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Add> + use<> {
        self.0 += message.0;
        self.0.ready()
    }
}

#[derive(Message)]
#[message(reply = ())]
struct Reset;

impl DispatchHandler<Reset> for Counter {
    fn handle(
        &mut self,
        _message: Reset,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Reset> + use<> {
        self.0 = 0;
        ().ready()
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
