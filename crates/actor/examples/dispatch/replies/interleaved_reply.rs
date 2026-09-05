//! Builds a dispatch-handler cx future on the interleaved lane.
//!
//! `scope.cx_reply` pairs `Cx::with` access with interleaved scheduling:
//! the actor task polls the returned future fairly with mailbox work, so a
//! waiting reply does not block `Read` from making progress.

use loac::prelude::*;
use tokio::sync::oneshot;

struct Counter(u64);

#[actor(mailbox, interleaved)]
impl Actor for Counter {
    type SpawnArgs = u64;

    async fn init(value: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self(value)
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct AddAfter {
    amount: u64,
    resume: oneshot::Receiver<()>,
}

impl DispatchHandler<AddAfter> for Counter {
    fn handle(
        &mut self,
        message: AddAfter,
        scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, AddAfter> + use<> {
        scope.cx_reply(self, move |mut cx| {
            Box::pin(async move {
                message
                    .resume
                    .await
                    .expect("the example retains the resume sender");
                cx.with(|actor, _| {
                    actor.0 += message.amount;
                    actor.0
                })
            })
        })
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct Read;

impl DispatchHandler<Read> for Counter {
    fn handle(
        &mut self,
        _message: Read,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, Read> + use<> {
        self.0.ready()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owner = loac::spawn::<Counter>(10);
    let (resume_tx, resume_rx) = oneshot::channel();

    let addition = owner.try_call(AddAfter {
        amount: 5,
        resume: resume_rx,
    })?;
    assert_eq!(owner.call(Read).await?, 10);

    resume_tx
        .send(())
        .expect("the interleaved reply retains the resume receiver");
    assert_eq!(addition.await?, 15);
    assert_eq!(owner.call(Read).await?, 15);

    assert_eq!(
        owner.shutdown(loac::Shutdown::Drain).await.reason(),
        loac::ExitReason::Drained
    );
    Ok(())
}
