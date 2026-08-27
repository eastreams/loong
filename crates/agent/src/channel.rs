//! Type-erased named channels between agents.
//!
//! A channel is an application-level protocol, not a single `loac` message
//! capability. It hides the concrete [`AgentRuntime<C, P>`](super::AgentRuntime) type
//! behind one stable trait object and can grow methods (for example `cancel`
//! or `status`) without changing how channels are registered.

use std::{future::Future, pin::Pin, sync::Arc};

use contracts::provider::{Request, StreamItem};
use loac::ActorRef;
use provider::StreamError;

use super::{AgentRuntime, ContextStore, Prompt, Provider, ProviderOut};

/// Why a channel call failed.
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    #[error("channel call failed: {0}")]
    Call(#[from] loac::CallError),
    #[error("channel stream failed: {0}")]
    Stream(#[from] StreamError<Request>),
}

/// A type-erased endpoint for one named agent channel.
///
/// Channels are looked up by name at registration time. This trait keeps the
/// target agent's concrete type out of that lookup, while `ask` still streams
/// the target's reply and collects it into one final answer.
pub trait ChannelTarget: Send + Sync {
    /// Asks the target agent one prompt and returns its streamed text.
    fn ask(
        &self,
        text: String,
    ) -> Pin<Box<dyn Future<Output = Result<String, ChannelError>> + Send + '_>>;
}

impl ChannelTarget for Arc<dyn ChannelTarget> {
    fn ask(
        &self,
        text: String,
    ) -> Pin<Box<dyn Future<Output = Result<String, ChannelError>> + Send + '_>> {
        self.as_ref().ask(text)
    }
}

impl<C, P> ChannelTarget for ActorRef<AgentRuntime<C, P>>
where
    C: ContextStore + 'static,
    P: Provider<Request, StreamItem, ProviderOut> + Clone + 'static,
{
    fn ask(
        &self,
        text: String,
    ) -> Pin<Box<dyn Future<Output = Result<String, ChannelError>> + Send + '_>> {
        let this = self.clone();
        Box::pin(async move {
            let mut reply = this.call(Prompt { text }).await?;
            let mut answer = String::new();
            while let Some(item) = reply.recv().await {
                if let StreamItem::Text { delta } = item {
                    answer.push_str(&delta);
                }
            }
            reply.finish().await??;
            Ok(answer)
        })
    }
}
