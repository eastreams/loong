use super::sealed::Sealed;
use super::{ToolAdapter, ToolOutcome, ToolRequest};
use loong_contracts::ToolSpec;

pub trait ToolImpl {
    type Input;
    type Output;

    const NAME: &str;
    const DESC: &str;
}
