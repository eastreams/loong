use loac::{Actor, ActorScope, ExitReason, Handler, Message, ReplyExt, Shutdown, actor};
use tokio::sync::mpsc;

use super::support::watchdog;

struct Calculator(u64);

#[actor(mailbox)]
impl Actor for Calculator {
    type SpawnArgs = u64;

    async fn init(value: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self(value)
    }
}

#[derive(Message)]
#[message(reply = u64)]
struct Add(u64);

impl Handler<Add> for Calculator {
    fn handle(
        &mut self,
        message: Add,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Add> + use<> {
        self.0 += message.0;
        self.0.ready()
    }
}

#[derive(Message)]
#[message(reply = String)]
struct Describe;

impl Handler<Describe> for Calculator {
    fn handle(
        &mut self,
        _message: Describe,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Describe> + use<> {
        format!("count={}", self.0).ready()
    }
}

#[tokio::test]
async fn one_actor_handles_multiple_typed_message_replies() {
    let owner = loac::spawn::<Calculator>(0);
    let actor = owner.actor_ref();

    assert_eq!(watchdog(actor.call(Add(3))).await.unwrap(), 3);
    assert_eq!(watchdog(actor.call(Add(4))).await.unwrap(), 7);
    assert_eq!(watchdog(actor.call(Describe)).await.unwrap(), "count=7");
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Drain)).await.reason(),
        ExitReason::Drained
    );
}

#[derive(Message)]
#[message(reply = mpsc::Receiver<u8>)]
struct Events;

impl Handler<Events> for Calculator {
    fn handle(
        &mut self,
        _message: Events,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, Events> + use<> {
        let (events, receiver) = mpsc::channel(2);
        events.try_send(1).expect("the stream buffer has room");
        events.try_send(2).expect("the stream buffer has room");
        receiver.ready()
    }
}

#[tokio::test]
async fn a_stream_handle_is_an_ordinary_typed_reply() {
    let owner = loac::spawn::<Calculator>(0);
    let actor = owner.actor_ref();
    let mut events = watchdog(actor.call(Events)).await.unwrap();

    assert_eq!(watchdog(events.recv()).await, Some(1));
    assert_eq!(watchdog(events.recv()).await, Some(2));
    assert_eq!(watchdog(events.recv()).await, None);
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Drain)).await.reason(),
        ExitReason::Drained
    );
}
