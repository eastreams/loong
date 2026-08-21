//! Agent-side types for the `loong` product.

use context::ContextStore;
use contracts::provider::{Request, StreamItem};
use contracts::transcript::{Role, TranscriptItem};
use loac::prelude::*;
use provider::{Provider, StreamError};
use tokio::sync::mpsc;

/// Writer that receives streamed provider items.
pub type ProviderOut = mpsc::Sender<StreamItem>;

/// Scaffold actor that composes context storage and an upstream provider.
///
/// `P: Clone` keeps stream handlers able to capture the provider they were
/// started with, so switching the actor's provider never interrupts streams
/// that are already running.
pub struct Agent<C, P>
where
    C: ContextStore,
    P: Provider<Request, StreamItem, ProviderOut> + Clone,
{
    store: C,
    provider: P,
}

/// Replaces the provider used by subsequent streams.
///
/// The frontend builds a new provider (for example an OpenAI provider with a
/// different model) and sends it to the agent. Streams that already started
/// hold their own clone, so they keep their original provider.
#[derive(loac::Message)]
#[message(reply = ())]
pub struct SwitchProvider<P>(pub P);

/// Asks the agent to stream a provider reply for one user message.
///
/// The agent appends the user message to its context store before streaming,
/// then appends the assistant text after the stream commits.
#[derive(loac::Message)]
#[message(stream = StreamItem, reply = Result<(), StreamError<Request>>)]
pub struct Prompt {
    /// The user message to append and send.
    pub text: String,
}

#[actor(mailbox, interleaved)]
impl<C, P> Actor for Agent<C, P>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
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
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
{
    fn handle(&mut self, message: SwitchProvider<P>, _scope: &mut ActorScope<'_, Self>) {
        self.provider = message.0;
    }
}

impl<C, P> StreamHandler<Prompt> for Agent<C, P>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
{
    fn handle<W>(
        &mut self,
        message: Prompt,
        mut out: W,
        _scope: &mut ActorScope<'_, Self>,
    ) -> impl loac::IntoStreamReply<Self, Prompt> + use<C, P, W>
    where
        W: loac::Writer<StreamItem> + Send + 'static,
    {
        let provider = self.provider.clone();

        let user_item = TranscriptItem::Message {
            role: Role::User,
            text: message.text,
        };
        let request = {
            let mut messages = self.store.snapshot().items;
            messages.push(user_item.clone());
            Request {
                messages,
                tools: Vec::new(),
            }
        };

        let (mut local_tx, mut local_rx) = mpsc::channel::<StreamItem>(8);

        async move {
            let (result, items) = tokio::join!(
                async move { provider.stream(request, &mut local_tx).await },
                async move {
                    let mut items = Vec::new();
                    while let Some(item) = local_rx.recv().await {
                        let _ = out.write(item.clone()).await;
                        items.push(item);
                    }
                    items
                },
            );

            let assistant = collect_assistant_items(items);

            (assistant, result)
        }
        .into_actor()
        .map(|(assistant, result), actor: &mut Self, _scope| {
            if !assistant.is_empty() {
                let _ = actor
                    .store
                    .append(std::iter::once(user_item).chain(assistant).collect());
            }
            result
        })
        .interleaved()
    }
}

/// Collects streamed items into the assistant transcript suffix.
///
/// Tool calls are forwarded to the frontend but not persisted until the
/// tool loop lands; this keeps the minimal path text-only.
fn collect_assistant_items(items: Vec<StreamItem>) -> Vec<TranscriptItem> {
    let mut text = String::new();
    for item in items {
        if let StreamItem::Text { delta } = item {
            text.push_str(&delta);
        }
    }

    if text.is_empty() {
        Vec::new()
    } else {
        vec![TranscriptItem::Message {
            role: Role::Assistant,
            text,
        }]
    }
}

#[cfg(test)]
mod tests;
