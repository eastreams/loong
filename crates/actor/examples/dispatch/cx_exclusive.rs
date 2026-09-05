//! Builds dispatch-handler cx futures on the exclusive lane.
//!
//! A `DispatchHandler` chooses an explicit reply strategy. The `ActorScope`
//! constructors `cx_exclusive` and `cx_stream_exclusive` keep the `Cx::with`
//! access style while selecting exclusive scheduling: the actor task polls the
//! returned future with mailbox dispatch paused, and owned tasks may continue.
//!
//! ```console
//! cargo run -p loac --example cx_exclusive
//! ```

use std::time::Duration;

use loac::prelude::*;
use tokio::sync::oneshot;

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
struct Read;

impl DispatchHandler<Read> for Counter {
    fn handle(
        &mut self,
        _message: Read,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Read> + use<> {
        self.0.ready()
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct AddExclusive {
    amount: u64,
    started: oneshot::Sender<()>,
    resume: oneshot::Receiver<()>,
}

impl DispatchHandler<AddExclusive> for Counter {
    fn handle(
        &mut self,
        message: AddExclusive,
        scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, AddExclusive> + use<> {
        scope.cx_exclusive(self, move |mut cx| {
            Box::pin(async move {
                let _ = message.started.send(());
                let _ = message.resume.await;
                cx.with(|actor, _| {
                    actor.0 += message.amount;
                    actor.0
                })
            })
        })
    }
}

#[derive(Message)]
#[message(stream = u8, reply = u8)]
struct StreamExclusive;

impl DispatchHandler<StreamExclusive, StreamKind> for Counter {
    fn handle(
        &mut self,
        _message: StreamExclusive,
        scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, StreamExclusive> + use<> {
        let (item_tx, item_rx) = tokio::sync::mpsc::channel::<u8>(8);
        let (final_tx, final_rx) = tokio::sync::oneshot::channel::<u8>();
        let strategy = scope.cx_stream_exclusive(self, move |mut cx| {
            Box::pin(async move {
                let mut out = item_tx;
                let next = cx.with(|actor, _| {
                    actor.0 += 1;
                    actor.0 as u8
                });
                let _ = out.write(next).await;
                next
            })
        });
        loac::StreamDispatch::new(strategy, item_rx, final_tx, final_rx)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owner = loac::spawn::<Counter>(0);
    let (started_tx, started_rx) = oneshot::channel();
    let (resume_tx, resume_rx) = oneshot::channel();

    let addition = owner.try_call(AddExclusive {
        amount: 5,
        started: started_tx,
        resume: resume_rx,
    })?;
    started_rx.await?;

    let mut read = owner.try_call(Read)?;
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut read)
            .await
            .is_err(),
        "exclusive cx work pauses mailbox dispatch"
    );

    resume_tx
        .send(())
        .expect("the exclusive cx future retains the resume receiver");
    assert_eq!(addition.await?, 5);
    assert_eq!(read.await?, 5);

    let mut stream = owner.call(StreamExclusive).await?;
    assert_eq!(stream.recv().await, Some(6));
    assert_eq!(stream.recv().await, None);
    assert_eq!(stream.finish().await?, 6);

    assert_eq!(owner.call(Read).await?, 6);
    let status = owner.shutdown(loac::Shutdown::Drain).await;
    assert_eq!(status.reason(), loac::ExitReason::Drained);
    Ok(())
}
