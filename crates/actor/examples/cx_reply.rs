//! Reply futures with `cx`: a plain `Future` that can still touch actor state
//! inside synchronous `with_actor` / `with_scope` scopes.

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
    fn handle(
        &mut self,
        message: AddAfterYield,
        scope: &mut ActorScope<'_, Self>,
    ) -> impl IntoReply<Self, AddAfterYield> + use<> {
        scope.cx_reply(self, |mut cx| {
            Box::pin(async move {
                tokio::task::yield_now().await;
                cx.with_actor(|actor| {
                    actor.0 += message.0;
                    actor.0
                })
            })
        })
    }
}

#[derive(Message)]
#[message(stream = u8, reply = u64)]
struct StreamAndCount(u8);

impl StreamHandler<StreamAndCount> for Accumulator {
    fn handle<W>(
        &mut self,
        message: StreamAndCount,
        mut out: W,
        scope: &mut ActorScope<'_, Self>,
    ) -> impl IntoStreamReply<Self, StreamAndCount> + use<W>
    where
        W: Writer<u8> + Send + 'static,
    {
        scope.cx_stream(self, |mut cx| {
            Box::pin(async move {
                for item in 0..message.0 {
                    if out.write(item).await.is_err() {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
                cx.with_actor(|actor| {
                    actor.0 += 1;
                    actor.0
                })
            })
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owner = loac::spawn::<Accumulator>(40);
    let actor = owner.actor_ref();

    assert_eq!(actor.call(AddAfterYield(2)).await?, 42);

    let mut reply = actor.call(StreamAndCount(3)).await?;
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
