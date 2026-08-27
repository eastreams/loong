//! OpenAI-compatible provider implementation.
//!
//! This crate adapts [`contracts::provider::Request`] to an OpenAI-compatible
//! chat completion endpoint and emits [`contracts::provider::StreamItem`]s.

mod client;
mod request;
mod sse;

pub use client::{OpenAiConfig, OpenAiProvider};
