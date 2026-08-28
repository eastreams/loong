//! Concrete tool host that owns the tool registry and invocation context.
//!
//! The agent invokes tools through [`ToolRegistry`], never through the kernel facade
//! directly. Tools implement [`ToolImpl`] and receive a [`ToolContext`] that
//! exposes resources such as [`fs`](ToolContext::fs).

mod registry;
pub mod tool;

pub use registry::{RegisteredTool, ToolRegistration, ToolRegistry, ToolSnapshot};
pub use tool::{RegistrationError, ToolContext, ToolError, ToolImpl};
