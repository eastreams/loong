//! The canonical transcript vocabulary for one session.
//!
//! Every Loong component that builds or reads agent context speaks this
//! vocabulary. Storage, adapters, and channels map to their own formats at
//! the edge; they never invent parallel item shapes here.
//!
//! `transcript` is deliberate. The items record a full session in order,
//! including tool interactions. `history` would collide later with the
//! compacted vs working context split.

use alloc::string::String;

use serde::{Deserialize, Serialize};

/// One entry of a session transcript.
///
/// The order of items in the log is the source of truth; individual items do
/// not carry a transcript-wide id. Tool invocations carry a `call_id` on both
/// the `ToolCall` and its `ToolResult`, which is the only cross-item identity
/// this vocabulary needs.
///
/// The `kind` tag keeps every JSON line flat while naming the variant.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TranscriptItem {
    /// Plain text from one participant.
    Message { role: Role, text: String },
    /// The assistant asked a tool to run.
    ///
    /// `call_id` links this call to its eventual `ToolResult`.
    /// `arguments` is pre-serialized JSON text owned by the caller. The
    /// store replays bytes; it never interprets tool payloads.
    ToolCall {
        call_id: String,
        name: String,
        arguments: String,
    },
    /// A tool finished and produced output.
    ///
    /// `call_id` points at the matching `ToolCall`.
    ToolResult { call_id: String, output: String },
}

/// Who produced a message.
///
/// This applies to messages only. Tool records have no role.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[cfg(test)]
mod tests;
