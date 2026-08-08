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

impl Handler<AddAfter> for Counter {
    fn handle(
        &mut self,
        message: AddAfter,
        _scope: &mut ActorScope<Self>,
    ) -> impl IntoReply<Self, AddAfter> + use<> {
        async move {
            message
                .resume
                .await
                .expect("the example retains the resume sender");
            message.amount
        }
        .into_actor()
        .map(|amount, actor: &mut Self, _scope| {
            actor.0 += amount;
            actor.0
        })
        .interleaved()
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct Read;

impl Handler<Read> for Counter {
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
    let counter = owner.actor_ref();
    let (resume_tx, resume_rx) = oneshot::channel();

    let addition = counter.try_call(AddAfter {
        amount: 5,
        resume: resume_rx,
    })?;
    assert_eq!(counter.call(Read).await?, 10);

    resume_tx
        .send(())
        .expect("the interleaved reply retains the resume receiver");
    assert_eq!(addition.await?, 15);
    assert_eq!(counter.call(Read).await?, 15);

    assert_eq!(
        owner.shutdown(loac::Shutdown::Drain).await.reason(),
        loac::ExitReason::Drained
    );
    Ok(())
}
