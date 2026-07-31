use loong_actor::{ExitReason, Shutdown, prelude::*, spawn};
use tokio::sync::oneshot;

struct Counter(u64);

impl Actor for Counter {}

struct AddAfter {
    amount: u64,
    resume: oneshot::Receiver<()>,
}

impl Message for AddAfter {
    type Reply = u64;
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

struct Read;

impl Message for Read {
    type Reply = u64;
}

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
    let owner = spawn(Counter(10));
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
        owner.shutdown(Shutdown::Drain).await.reason(),
        ExitReason::Drained
    );
    Ok(())
}
