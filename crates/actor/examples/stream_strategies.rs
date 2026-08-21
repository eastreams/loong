//! Streaming reply scheduling strategies.
//!
//! Each provider below handles a `#[message(stream = ...)]` message with a
//! different [`IntoStreamReply`] strategy. Run with:
//!
//! ```console
//! cargo run -p loac --example stream_strategies
//! ```
//!
//! The strategies mirror ordinary [`IntoReply`] scheduling:
//!
//! - a bare future runs as an owned Tokio task,
//! - [`ReplyExt::ready`] completes the final value immediately,
//! - [`ReplyExt::exclusive`] runs an actor-aware future with the mailbox paused,
//! - [`InterleavedFutureExt::interleaved`] runs an actor-aware future fairly
//!   with other actor work, and
//! - [`loac::reply::Either`] chooses between two strategies at runtime.

use std::time::Duration;

use loac::prelude::*;

#[derive(Message)]
#[message(stream = u8, reply = u8)]
struct OwnedStream(u8);

struct OwnedProvider;

#[actor(mailbox)]
impl Actor for OwnedProvider {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

impl StreamHandler<OwnedStream> for OwnedProvider {
    fn handle<W>(
        &mut self,
        message: OwnedStream,
        mut out: W,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoStreamReply<Self, OwnedStream> + use<W>
    where
        W: loac::Writer<u8> + Send + 'static,
    {
        // A bare future selects owned scheduling: Tokio polls it in a
        // separate task, so a long stream never blocks this actor's mailbox.
        async move {
            for item in 0..message.0 {
                if out.write(item).await.is_err() {
                    break;
                }
            }
            message.0
        }
    }
}

#[derive(Message)]
#[message(stream = u8, reply = u8)]
struct BranchStream(u8);

struct BranchProvider;

#[actor(mailbox)]
impl Actor for BranchProvider {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

impl StreamHandler<BranchStream> for BranchProvider {
    fn handle<W>(
        &mut self,
        message: BranchStream,
        mut out: W,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoStreamReply<Self, BranchStream> + use<W>
    where
        W: loac::Writer<u8> + Send + 'static,
    {
        if message.0 == 0 {
            // Ready fast path: final is already known, so no future is
            // scheduled. Dropping `out` closes the caller's item stream.
            loac::reply::Either::Left(message.0.ready())
        } else {
            // Owned branch: stream items, then finish.
            loac::reply::Either::Right(async move {
                for item in 0..message.0 {
                    if out.write(item).await.is_err() {
                        break;
                    }
                }
                message.0
            })
        }
    }
}

#[derive(Message)]
#[message(stream = u8, reply = u8)]
struct ExclusiveStream(u8);

struct ExclusiveProvider;

#[actor(mailbox)]
impl Actor for ExclusiveProvider {
    type SpawnArgs = ();

    async fn init(_args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

impl StreamHandler<ExclusiveStream> for ExclusiveProvider {
    fn handle<W>(
        &mut self,
        message: ExclusiveStream,
        mut out: W,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoStreamReply<Self, ExclusiveStream> + use<W>
    where
        W: loac::Writer<u8> + Send + 'static,
    {
        // An actor-aware future polled by the actor scheduler. The mailbox is
        // paused while it runs, so keep exclusive stream work short.
        async move {
            for item in 0..message.0 {
                if out.write(item).await.is_err() {
                    break;
                }
            }
            message.0
        }
        .into_actor()
        .exclusive()
    }
}

#[derive(Message)]
#[message(stream = u8, reply = u8)]
struct InterleavedStream;

struct InterleavedProvider {
    seed: u8,
}

#[actor(mailbox, interleaved)]
impl Actor for InterleavedProvider {
    type SpawnArgs = u8;

    async fn init(seed: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self { seed }
    }
}

impl StreamHandler<InterleavedStream> for InterleavedProvider {
    fn handle<W>(
        &mut self,
        _message: InterleavedStream,
        out: W,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoStreamReply<Self, InterleavedStream> + use<W>
    where
        W: loac::Writer<u8> + Send + 'static,
    {
        // Interleaved actor-aware work. The first stage sleeps without holding
        // the mailbox, then `then` reads actor state and returns the second
        // stage that owns the writer and produces the items.
        async { tokio::time::sleep(Duration::from_millis(10)).await }
            .into_actor()
            .then(move |(), actor: &mut Self, _scope| {
                let seed = actor.seed;
                let mut out = out;
                async move {
                    for item in seed..seed + 3 {
                        if out.write(item).await.is_err() {
                            break;
                        }
                    }
                    seed
                }
                .into_actor()
            })
            .interleaved()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let owner = loac::spawn::<OwnedProvider>(());
    let provider = owner.actor_ref();
    let mut reply = provider.call(OwnedStream(3)).await?;
    assert_eq!(reply.recv().await, Some(0));
    assert_eq!(reply.recv().await, Some(1));
    assert_eq!(reply.recv().await, Some(2));
    assert_eq!(reply.recv().await, None);
    assert_eq!(reply.finish().await?, 3);
    drain(owner).await;

    let owner = loac::spawn::<BranchProvider>(());
    let provider = owner.actor_ref();

    let mut reply = provider.call(BranchStream(0)).await?;
    assert_eq!(reply.recv().await, None);
    assert_eq!(reply.finish().await?, 0);

    let mut reply = provider.call(BranchStream(2)).await?;
    assert_eq!(reply.recv().await, Some(0));
    assert_eq!(reply.recv().await, Some(1));
    assert_eq!(reply.recv().await, None);
    assert_eq!(reply.finish().await?, 2);
    drain(owner).await;

    let owner = loac::spawn::<ExclusiveProvider>(());
    let provider = owner.actor_ref();
    let mut reply = provider.call(ExclusiveStream(3)).await?;
    assert_eq!(reply.recv().await, Some(0));
    assert_eq!(reply.recv().await, Some(1));
    assert_eq!(reply.recv().await, Some(2));
    assert_eq!(reply.recv().await, None);
    assert_eq!(reply.finish().await?, 3);
    drain(owner).await;

    let owner = loac::spawn::<InterleavedProvider>(10);
    let provider = owner.actor_ref();
    let mut reply = provider.call(InterleavedStream).await?;
    assert_eq!(reply.recv().await, Some(10));
    assert_eq!(reply.recv().await, Some(11));
    assert_eq!(reply.recv().await, Some(12));
    assert_eq!(reply.recv().await, None);
    assert_eq!(reply.finish().await?, 10);
    drain(owner).await;

    Ok(())
}

async fn drain<A>(owner: loac::ActorOwner<A>)
where
    A: loac::Actor,
{
    let status = owner.shutdown(loac::Shutdown::Drain).await;
    assert_eq!(status.reason(), loac::ExitReason::Drained);
}
