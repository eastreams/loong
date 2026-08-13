//! A provider actor streams items into a subscriber-owned channel.
//! The subscribe message carries the sender; the reply task produces the items.
//! The stream ends when that task stops.

use loac::prelude::*;
use tokio::sync::mpsc;

struct Provider;

#[actor(mailbox)]
impl Actor for Provider {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
struct Subscribe(mpsc::Sender<u8>);

impl Handler<Subscribe> for Provider {
    fn handle(
        &mut self,
        Subscribe(tx): Subscribe,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, Subscribe> + use<> {
        async move {
            for i in 0..4 {
                // A failed send means the subscriber dropped its receiver.
                if tx.send(i).await.is_err() {
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

    // The subscriber owns the stream's capacity and lifetime.
    let (tx, mut rx) = mpsc::channel(4);

    let t0 = tokio::time::Instant::now();

    // One-way `send` admits the subscription. The receiver is read while
    // the provider still produces.
    provider.send(Subscribe(tx)).await?;

    // Intervals run slightly longer than the sleep: Tokio timer overshoot
    // is stable and accumulates across items.
    for i in 0..4 {
        assert_eq!(rx.recv().await, Some(i));
        println!("{:.3}s", t0.elapsed().as_secs_f32());
    }

    assert_eq!(rx.recv().await, None);

    // The production task finished, so its sender dropped and the stream
    // closed. Drain finishes the actor; it would wait for a running task.
    let status = owner.shutdown(loac::Shutdown::Drain).await;
    assert_eq!(status.reason(), loac::ExitReason::Drained);
    Ok(())
}
