//! Reuses one const generic across actor options.
//! `mailbox = dynamic(N)` makes `N` the spawn default.
//! `interleaved = N` fixes the limit for each actor type.

use loac::prelude::*;

struct Service<const N: usize>;

#[actor(mailbox = dynamic(N), interleaved = N)]
impl<const N: usize> Actor for Service<N> {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

#[derive(Message)]
#[message(reply = usize)]
struct ReadTypeParameter;

impl<const N: usize> DispatchHandler<ReadTypeParameter> for Service<N> {
    fn handle(
        &mut self,
        _message: ReadTypeParameter,
        _scope: &mut ActorScope<Self>,
    ) -> impl loac::IntoReply<Self, ReadTypeParameter> + use<N> {
        N.ready()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owner = loac::spawn::<Service<8>>(());
    assert_eq!(owner.call(ReadTypeParameter).await?, 8);

    assert_eq!(
        owner.shutdown(loac::Shutdown::Drain).await.reason(),
        loac::ExitReason::Drained
    );
    Ok(())
}
