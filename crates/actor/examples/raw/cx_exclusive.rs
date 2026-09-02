//! Builds raw-handler cx futures on the exclusive lane.
//!
//! `RawHandler` and `RawStreamHandler` normally choose an explicit reply
//! strategy. The `ActorScope` constructors `cx_exclusive` and
//! `cx_stream_exclusive` keep the `Cx::with` access style while selecting
//! exclusive scheduling: the actor task polls the returned future with mailbox
//! dispatch paused, and owned tasks may continue.
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
#[message(raw = u64)]
struct Read;

impl RawHandler<Read> for Counter {
    fn handle(
        &mut self,
        _message: Read,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Read> + use<> {
        self.0.ready()
    }
}

#[derive(Message)]
#[message(raw = u64)]
struct AddExclusive {
    amount: u64,
    started: oneshot::Sender<()>,
    resume: oneshot::Receiver<()>,
}

impl RawHandler<AddExclusive> for Counter {
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
#[message(raw_stream = u8, reply = u8)]
struct StreamExclusive;

impl RawStreamHandler<StreamExclusive> for Counter {
    fn handle<W>(
        &mut self,
        _message: StreamExclusive,
        mut out: W,
        scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoStreamReply<Self, StreamExclusive> + use<W>
    where
        W: loac::Writer<u8> + Send + 'static,
    {
        scope.cx_stream_exclusive(self, move |mut cx| {
            Box::pin(async move {
                let next = cx.with(|actor, _| {
                    actor.0 += 1;
                    actor.0 as u8
                });
                let _ = out.write(next).await;
                next
            })
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owner = loac::spawn::<Counter>(0);
    let counter = owner.actor_ref();
    let (started_tx, started_rx) = oneshot::channel();
    let (resume_tx, resume_rx) = oneshot::channel();

    let addition = counter.try_call(AddExclusive {
        amount: 5,
        started: started_tx,
        resume: resume_rx,
    })?;
    started_rx.await?;

    let mut read = counter.try_call(Read)?;
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

    let mut stream = counter.call(StreamExclusive).await?;
    assert_eq!(stream.recv().await, Some(6));
    assert_eq!(stream.recv().await, None);
    assert_eq!(stream.finish().await?, 6);

    assert_eq!(counter.call(Read).await?, 6);
    let status = owner.shutdown(loac::Shutdown::Drain).await;
    assert_eq!(status.reason(), loac::ExitReason::Drained);
    Ok(())
}
