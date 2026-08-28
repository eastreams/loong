//! Tool adapter that exposes one named channel as a callable tool.

use std::sync::Arc;

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
impl ToolImpl for ChannelTool {
    type Input = ChannelPromptInput;
    type Output = String;
    type Error = ChannelError;

    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &'static str {
        "Ask another agent a prompt and return its streamed answer."
    }

    async fn execute(
        &self,
        _ctx: &ToolContext<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, Self::Error> {
        self.target.ask(input.prompt).await
    }
}
