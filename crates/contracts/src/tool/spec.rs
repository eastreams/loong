use alloc::borrow::Cow;

use schemars::Schema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    // These fields need no partial patches, so
    // Cow is suitable.
    /// The descriptive name.
    pub name: Cow<'static, str>,
    pub description: Cow<'static, str>,
    pub input_schema: Schema,
    pub output_schema: Schema,
}
