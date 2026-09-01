use super::*;

struct Counter(u8);

#[actor(mailbox)]
impl Actor for Counter {
    type SpawnArgs = u8;

    async fn init(value: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self(value)
    }
}

#[derive(Message)]
#[message(raw = u8)]
struct Increment;

impl RawHandler<Increment> for Counter {
    fn handle(
        &mut self,
        _message: Increment,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Increment> + use<> {
        let __reply = {
            self.0 += 1;
            self.0
        };
        __reply.ready()
    }
}

#[tokio::test]
async fn sync_handler_mutates_actor_and_replies_immediately() {
    let owner = loac::spawn::<Counter>(0);
    let actor = owner.actor_ref();

    assert_eq!(watchdog(actor.call(Increment)).await, Ok(1));
    assert_eq!(watchdog(actor.call(Increment)).await, Ok(2));
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await.reason(),
        ExitReason::Stopped
    );
}

#[derive(Message)]
#[message(raw = u8)]
struct ChooseReply(bool);

impl RawHandler<ChooseReply> for Counter {
    fn handle(
        &mut self,
        message: ChooseReply,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, ChooseReply> + use<> {
        if message.0 {
            reply::Either::Left(1.ready())
        } else {
            reply::Either::Right(async { 2 })
        }
    }
}

#[tokio::test]
async fn either_selects_between_reply_strategies_without_boxing() {
    let owner = loac::spawn::<Counter>(0);
    let actor = owner.actor_ref();

    assert_eq!(watchdog(actor.call(ChooseReply(true))).await, Ok(1));
    assert_eq!(watchdog(actor.call(ChooseReply(false))).await, Ok(2));
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Stop)).await.reason(),
        ExitReason::Stopped
    );
}
