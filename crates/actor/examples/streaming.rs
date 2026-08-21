//! A provider actor streams items through a runtime-created channel.
//! `call` returns a `StreamReply`; `recv` reads items and `finish` waits
//! for the handler's final reply value. The stream ends when the handler
//! future stops producing and drops its `out` writer.

use futures_util::StreamExt;
use loac::prelude::*;

struct Provider;

#[actor(mailbox)]
impl Actor for Provider {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
#[message(stream = u8)]
struct Subscribe;

impl StreamHandler<Subscribe> for Provider {
    fn handle<W>(
        &mut self,
        _message: Subscribe,
        mut out: W,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoStreamReply<Self, Subscribe> + use<W>
    where
        W: loac::Writer<u8> + Send + 'static,
    {
        async move {
            for i in 0..4 {
                // A failed write means the caller dropped its `StreamReply`.
                if out.write(i).await.is_err() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owner = loac::spawn::<Provider>(());
    let provider = owner.actor_ref();

    let t0 = tokio::time::Instant::now();

    // The runtime creates the item channel; `call` returns the receiver side.
    let mut stream = provider.call(Subscribe).await?;

    // Intervals run slightly longer than the sleep: Tokio timer overshoot
    // is stable and accumulates across items.
    // `items` borrows the receiver. Keep one enumerated stream alive for the
    // whole loop: recreating `enumerate` every iteration would reset the
    // index to 0 each time.
    let mut items = stream.items().enumerate();
    while let Some((i, item)) = items.next().await {
        assert_eq!(i, item as usize);
        println!("{:.3}s", t0.elapsed().as_secs_f32());
    }

    assert_eq!(stream.recv().await, None);

    // The item stream closed because the handler future finished and dropped
    // its writer. `finish` returns the final reply value after the actor's
    // handler future completes.
    assert_eq!(stream.finish().await?, ());

    // Drain finishes the actor; it would wait for a running task.
    let status = owner.shutdown(loac::Shutdown::Drain).await;
    assert_eq!(status.reason(), loac::ExitReason::Drained);
    Ok(())
}
