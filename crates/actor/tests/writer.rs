use std::sync::{Arc, Mutex};

use loac::{Actor, ActorScope, Message, Shutdown, SyncHandler, Writer, actor};

struct Collector {
    seen: Arc<Mutex<Vec<u8>>>,
}

#[actor(mailbox)]
impl Actor for Collector {
    type SpawnArgs = Arc<Mutex<Vec<u8>>>;

    async fn init(seen: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self { seen }
    }
}

#[derive(Debug, PartialEq, Message)]
struct Token(u8);

impl SyncHandler<Token> for Collector {
    fn handle(&mut self, message: Token, _scope: &mut ActorScope<'_, Self>) {
        self.seen.lock().unwrap().push(message.0);
    }
}

#[tokio::test]
async fn actor_handles_are_writers() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let owner = loac::spawn::<Collector>(Arc::clone(&seen));
    let mut actor_ref = owner.actor_ref();
    let mut recipient = owner.actor_ref().recipient::<Token>();

    actor_ref.write(Token(1)).await.unwrap();
    recipient.write(Token(2)).await.unwrap();

    let _ = owner.shutdown(Shutdown::Drain).await;

    assert_eq!(actor_ref.write(Token(3)).await, Err(Token(3)));
    assert_eq!(recipient.write(Token(4)).await, Err(Token(4)));

    assert_eq!(*seen.lock().unwrap(), vec![1, 2]);
}
