//! Tool sets: cohesive groups of tools.

use tool_host::{RegistrationError, ToolRegistry};

/// A collection of tools registered into one agent.
pub trait ToolSet: Send + Sync + 'static {
    /// Stable tool-set name used in error messages.
    fn name(&self) -> &'static str;

    /// Registers the concrete tools into the agent's registry.
    fn register(&self, registry: &mut ToolRegistry) -> Result<(), RegistrationError>;
}

/// The built-in filesystem tool set: `read_file` and `write_file`.
#[derive(Debug, Clone, Copy, Default)]
pub struct FileTools;

impl ToolSet for FileTools {
    fn name(&self) -> &'static str {
        "FileTools"
    }

    fn register(&self, registry: &mut ToolRegistry) -> Result<(), RegistrationError> {
        registry.register("read_file".to_owned(), tools::ReadFileTool)?;
        registry.register("write_file".to_owned(), tools::WriteFileTool)?;
        Ok(())
    }
}
