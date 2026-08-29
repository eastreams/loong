//! Type-erased named channels between agents.

use std::{future::Future, pin::Pin, sync::Arc};

use contracts::provider::StreamItem;
use loac::ActorRef;

use super::{Agent, Prompt, PromptError};

/// Why a channel call failed.
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    #[error("channel call failed: {0}")]
    Call(#[from] loac::CallError),
    #[error("channel prompt failed: {0}")]
    Prompt(#[from] PromptError),
}

/// A type-erased endpoint for one named agent channel.
pub trait ChannelTarget: Send + Sync {
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

impl ChannelTarget for ActorRef<Agent> {
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
