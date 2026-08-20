//! Agent-side types for the `loong` product.

use context::ContextStore;
use contracts::provider::{Request, StreamItem};
use loac::prelude::*;
use provider::Provider;
use tokio::sync::mpsc;

/// Writer that receives streamed provider items.
type ProviderOut = mpsc::Sender<StreamItem>;

/// Scaffold actor that composes context storage and an upstream provider.
#[allow(dead_code)]
struct Agent<C, P>
where
    C: ContextStore,
    P: Provider<Request, StreamItem, ProviderOut>,
{
    store: C,
    provider: P,
}

#[actor]
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
