//! Agent-side types for the `loong` product.

use context::ContextStore;
use contracts::provider::{Request, StreamItem};
use loac::prelude::*;
use provider::Provider;
use tokio::sync::mpsc;

/// Writer that receives streamed provider items.
pub type ProviderOut = mpsc::Sender<StreamItem>;

/// Scaffold actor that composes context storage and an upstream provider.
pub struct Agent<C, P>
where
    C: ContextStore,
    P: Provider<Request, StreamItem, ProviderOut>,
{
    // Owned by the actor; request handlers will read it.
    #[allow(dead_code)]
    store: C,
    provider: P,
}

/// Replaces the provider used by subsequent streams.
///
/// The frontend builds a new provider (for example an OpenAI provider with a
/// different model) and sends it to the agent. Already-running streams keep
/// their original provider.
#[derive(loac::Message)]
#[message(reply = ())]
pub struct SwitchProvider<P>(pub P);

#[actor(mailbox)]
impl<C, P> Actor for Agent<C, P>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + 'static,
{
    type SpawnArgs = (C, P);

    async fn init(args: Self::SpawnArgs, _scope: &mut ActorScope<'_, Self>) -> Self {
        Self {
            store: args.0,
            provider: args.1,
        }
    }
}

impl<C, P> SyncHandler<SwitchProvider<P>> for Agent<C, P>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + 'static,
{
    fn handle(&mut self, message: SwitchProvider<P>, _scope: &mut ActorScope<'_, Self>) {
        self.provider = message.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use context::memory::MemoryStore;
    use loac::{ExitReason, Shutdown};
    use provider::{Provider, StreamError};

    struct DummyProvider;

    #[async_trait]
    impl Provider<Request, StreamItem, ProviderOut> for DummyProvider {
        async fn stream(
            &self,
            _req: Request,
            _out: &mut ProviderOut,
        ) -> Result<(), StreamError<Request>> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn switch_provider_message_is_accepted() {
        let owner =
            loac::spawn::<Agent<MemoryStore, DummyProvider>>((MemoryStore::new(), DummyProvider));
        let actor_ref = owner.actor_ref();

        actor_ref.call(SwitchProvider(DummyProvider)).await.unwrap();

        let status = owner.shutdown(Shutdown::Drain).await;
        assert_eq!(status.reason(), ExitReason::Drained);
    }
}
