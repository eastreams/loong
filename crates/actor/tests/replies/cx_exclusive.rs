use super::*;

struct CxExclusiveCounter(u8);

#[actor(mailbox)]
impl Actor for CxExclusiveCounter {
    type SpawnArgs = u8;

    async fn init(value: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self(value)
    }
}

#[derive(Message)]
#[message(raw = u8)]
struct CxExclusiveIncrement;

impl RawHandler<CxExclusiveIncrement> for CxExclusiveCounter {
    fn handle(
        &mut self,
        _message: CxExclusiveIncrement,
        scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, CxExclusiveIncrement> + use<> {
        scope.cx_exclusive(self, |mut cx| {
            Box::pin(async move {
                cx.with(|actor, _| {
                    actor.0 += 1;
                    actor.0
                })
            })
        })
    }
}

#[tokio::test]
async fn cx_exclusive_runs_without_interleaving() {
    let owner = loac::spawn::<CxExclusiveCounter>(0);
    let actor = owner.actor_ref();

    assert_eq!(watchdog(actor.call(CxExclusiveIncrement)).await, Ok(1));
    assert_eq!(watchdog(actor.call(CxExclusiveIncrement)).await, Ok(2));
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await.reason(),
        ExitReason::Stopped
    );
}

#[derive(Message)]
#[message(raw_stream = u8, reply = u8)]
struct CxExclusiveStream(u8);

impl loac::RawStreamHandler<CxExclusiveStream> for CxExclusiveCounter {
    fn handle<W>(
        &mut self,
        message: CxExclusiveStream,
        mut out: W,
        scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoStreamReply<Self, CxExclusiveStream> + use<W>
    where
        W: loac::Writer<u8> + Send + 'static,
    {
        let base = message.0;
        scope.cx_stream_exclusive(self, move |mut cx| {
            Box::pin(async move {
                let doubled = cx.with(|actor, _| {
                    actor.0 += base;
                    actor.0 * 2
                });
                let _ = out.write(doubled).await;
                doubled
            })
        })
    }
}

#[tokio::test]
async fn cx_stream_exclusive_writes_items_and_finishes() {
    let owner = loac::spawn::<CxExclusiveCounter>(0);
    let actor = owner.actor_ref();

    let mut reply = watchdog(actor.call(CxExclusiveStream(5)))
        .await
        .expect("the stream call commits");
    assert_eq!(watchdog(reply.recv()).await, Some(10));
    assert_eq!(watchdog(reply.recv()).await, None);
    assert_eq!(watchdog(reply.finish()).await, Ok(10));

    assert_eq!(watchdog(actor.call(CxExclusiveIncrement)).await, Ok(6));
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await.reason(),
        ExitReason::Stopped
    );
}
