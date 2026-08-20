//! Fixed vocabulary for upstream model providers.

use alloc::{string::String, vec::Vec};

use serde::{Deserialize, Serialize};

use crate::{tool::ToolSpec, transcript::TranscriptItem};

/// Provider-agnostic chat request.
///
/// The shape deliberately mirrors an OpenAI-compatible chat completion
/// request (`model`, `messages`, and `tools`). Providers map this vocabulary
/// to their own wire format at the adapter edge; it is not a wire type.
///
/// The provider contract always streams items, so `stream` is not a request
/// field. Non-streaming providers simply emit the whole assistant message as
/// one [`StreamItem`]. Empty `tools` requests a plain completion.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub model: String,
    pub messages: Vec<TranscriptItem>,
    pub tools: Vec<ToolSpec>,
}

/// One item streamed from an upstream provider.
///
/// Streaming providers emit text deltas and complete tool calls. Non-streaming
/// providers emit the whole assistant message as a single [`StreamItem::Text`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamItem {
    /// One text delta, or a whole assistant message for non-streaming
    /// providers.
    Text {
        /// The delta text.
        delta: String,
    },
    /// One complete assistant tool call.
    ///
    /// Streaming adapters reassemble tool-call deltas before emitting, so
    /// callers never observe partial JSON arguments.
    ToolCall {
        /// Provider-issued tool call id.
        id: String,
        /// Tool name.
        name: String,
        /// Complete pre-serialized JSON arguments.
        arguments: String,
    },
}
