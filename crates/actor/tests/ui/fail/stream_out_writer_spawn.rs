// A stream handler cannot move its lifetime-bound writer into a `'static` task.
use loac::{Actor, ActorScope, Cx, StreamHandler, StreamOut, Writer};

struct Streamer;

#[loac::actor(mailbox, interleaved = unbounded)]
impl Actor for Streamer {
    type SpawnArgs = ();

    async fn init(_: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(loac::Message)]
#[message(stream = u8, reply = u8)]
struct StreamNumbers(u8);

impl StreamHandler<StreamNumbers> for Streamer {
    async fn handle<'a, W>(
        _message: StreamNumbers,
        mut out: StreamOut<'a, W>,
        _cx: Cx<'a, Self>,
    ) -> u8
    where
        W: Writer<u8> + Send + 'a,
    {
        tokio::spawn(async move {
            let _ = out.write(1).await;
        });
        0
    }
}

fn main() {}
