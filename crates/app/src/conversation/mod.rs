mod orchestrator;
mod persistence;
mod runtime;

pub use orchestrator::ConversationOrchestrator;
#[allow(unused_imports)]
pub use runtime::{ConversationRuntime, DefaultConversationRuntime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorMode {
    #[cfg_attr(
        not(any(feature = "channel-telegram", feature = "channel-feishu")),
        allow(dead_code)
    )]
    Propagate,
    InlineMessage,
}

#[cfg(test)]
mod tests;
