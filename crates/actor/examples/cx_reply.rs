//! Reply futures with `cx`: a plain `Future` that can still touch actor state
//! inside synchronous `with` scopes.

use loac::prelude::*;

struct Accumulator(u64);

#[actor(mailbox, interleaved = unbounded)]
impl Actor for Accumulator {
    type SpawnArgs = u64;

    async fn init(initial: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self(initial)
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct AddAfterYield(u64);

impl Handler<AddAfterYield> for Accumulator {
    async fn handle(message: AddAfterYield, mut cx: Cx<'_, Self>) -> u64 {
        tokio::task::yield_now().await;
        cx.with(|actor, _| {
            actor.0 += message.0;
            actor.0
        })
    }
}

#[derive(Message)]
#[message(stream = u8, reply = u64)]
struct StreamAndCount(u8);

impl StreamHandler<StreamAndCount> for Accumulator {
    async fn handle<W>(message: StreamAndCount, mut out: W, mut cx: Cx<'_, Self>) -> u64
    where
        W: Writer<u8> + Send + 'static,
    {
        for item in 0..message.0 {
            if out.write(item).await.is_err() {
                break;
            }
            tokio::task::yield_now().await;
        }
        cx.with(|actor, _| {
            actor.0 += 1;
            actor.0
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owner = loac::spawn::<Accumulator>(40);

    assert_eq!(owner.call(AddAfterYield(2)).await?, 42);

    let mut reply = owner.call(StreamAndCount(3)).await?;
    let mut items = Vec::new();
    while let Some(item) = reply.recv().await {
        items.push(item);
    }
    assert_eq!(items, vec![0, 1, 2]);
    assert_eq!(reply.finish().await?, 43);

    let status = owner.shutdown(loac::Shutdown::Drain).await;
    assert_eq!(status.reason(), loac::ExitReason::Drained);
    Ok(())
}
