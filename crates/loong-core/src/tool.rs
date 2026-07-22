use std::error::Error;

use async_trait::async_trait;
use loong_contracts::{ToolInputError, ToolSpec};
use serde_json::Value;

use crate::policy::context::ContextFactory;

/// Runtime control-flow class for one concrete tool failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolFailureKind {
    /// A governed inner action denied execution; sibling tools may continue.
    Denied,
    /// Input was accepted, but concrete execution failed.
    Execution,
}

#[async_trait]
pub trait ToolImpl<C: ContextFactory>: Send + Sync + 'static {
    type Input: Send + 'static;
    type Output: Send + Into<Value> + 'static;
    type Error: Error + Send + Sync + 'static;

    fn spec(&self) -> ToolSpec;

    fn parse_input(&self, payload: Value) -> Result<Self::Input, ToolInputError>;

    /// Classify a concrete execution error before type erasure.
    ///
    /// The concrete tool owns this distinction because the erased runtime must
    /// not infer batch control flow by scanning arbitrary error source chains.
    fn failure_kind(&self, _error: &Self::Error) -> ToolFailureKind {
        ToolFailureKind::Execution
    }

    async fn execute(
        &self,
        ctx: &C::Cx<'_>,
        input: Self::Input,
    ) -> Result<Self::Output, Self::Error>;
}
