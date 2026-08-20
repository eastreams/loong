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
/// Streaming providers emit deltas; non-streaming providers emit the whole
/// assistant message as a single item.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StreamItem(pub String);
