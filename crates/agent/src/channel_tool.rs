//! Tool adapter that exposes one named channel as a callable tool.

use std::{borrow::Cow, sync::Arc};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tool_host::{ToolContext, ToolImpl};

use crate::channel::{ChannelError, ChannelTarget};

/// Tool input for an agent channel: one prompt to the target agent.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ChannelPromptInput {
    pub prompt: String,
}

/// Concrete tool registered by [`AgentBuilder::with_channel`](
/// super::builder::AgentBuilder::with_channel).
pub struct ChannelTool {
    name: String,
    target: Arc<dyn ChannelTarget>,
}

impl ChannelTool {
    #[must_use]
    pub fn new(name: impl Into<String>, target: Arc<dyn ChannelTarget>) -> Self {
        Self {
            name: name.into(),
            target,
        }
    }
}

#[async_trait]
impl ToolImpl for ChannelTool {
    type Input = ChannelPromptInput;
    type Output = String;
    type Error = ChannelError;

    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(&self.name)
    }

    fn description(&self) -> Cow<'_, str> {
        Cow::Borrowed("Ask another agent a prompt and return its streamed answer.")
    }

    async fn execute(
        &self,
        _ctx: &ToolContext<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, Self::Error> {
        self.target.ask(input.prompt).await
    }
}
