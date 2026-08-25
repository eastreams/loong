//! Concrete tool host that owns the tool registry and invocation context.
//!
//! The agent invokes tools through [`App`], never through the kernel facade
//! directly. Tools implement [`ToolImpl`] and receive a [`ToolContext`] that
//! exposes narrow access facades such as [`fs`](ToolContext::fs).

pub mod tool;
pub mod tools;

pub use tool::{
    App, AppToolContext, InputError, InvocationParams, OutputClassification, RegisteredTool,
    RegistrationError, ToolContext, ToolError, ToolHost, ToolImpl, ToolOutcome, ToolRegistration,
};
