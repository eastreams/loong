//! Type-erased named channels between agents.
//!
//! A channel is an application-level protocol, not a single `loac` message
//! capability. It hides the concrete [`Agent<C, P>`](super::Agent) type
//! behind one stable trait object and can grow methods (for example `cancel`
//! or `status`) without changing how channels are registered.

use std::{future::Future, pin::Pin, sync::Arc};

use async_trait::async_trait;
use contracts::provider::{Request, StreamItem};
use loac::ActorRef;
use provider::StreamError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tool_host::{ToolImpl, ToolRegistry};

use super::{Agent, ContextStore, Prompt, Provider, ProviderOut};

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

impl<C, P> ChannelTarget for ActorRef<Agent<C, P>>
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

/// Tool input for an agent channel: one prompt to the target agent.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ChannelPromptInput {
    pub prompt: String,
}

/// Concrete tool registered by [`AgentBuilder::with_channel`](
/// super::builder::AgentBuilder::with_channel).
pub struct ChannelTool {
    name: &'static str,
    target: Arc<dyn ChannelTarget>,
}

impl ChannelTool {
    #[must_use]
    pub fn new(name: &'static str, target: Arc<dyn ChannelTarget>) -> Self {
        Self { name, target }
    }
}

#[async_trait]
impl ToolImpl<ToolRegistry> for ChannelTool {
    type Input = ChannelPromptInput;
    type Output = String;
    type Error = ChannelError;

    fn name(&self) -> &'static str {
        self.name
    }

    fn description(&self) -> &'static str {
        "Ask another agent a prompt and return its streamed answer."
    }

    async fn execute(
        &self,
        _ctx: &<ToolRegistry as tool_host::ToolHost>::ToolCx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, Self::Error> {
        self.target.ask(input.prompt).await
    }
}
