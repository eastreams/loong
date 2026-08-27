//! Concrete tool host that owns the tool registry and invocation context.
//!
//! The agent invokes tools through [`ToolRegistry`], never through the kernel facade
//! directly. Tools implement [`ToolImpl`] and receive a [`ToolContext`] that
//! exposes narrow access facades such as [`fs`](ToolContext::fs).

mod registry;
pub mod tool;

pub use registry::{RegisteredTool, ToolRegistration, ToolRegistry, ToolRegistryContext};
pub use tool::{InvocationParams, RegistrationError, ToolContext, ToolError, ToolHost, ToolImpl};
