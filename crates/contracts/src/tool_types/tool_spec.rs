use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolTier {
    Core,
    Extension,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ToolSpec {
    // path is not present, because path is managed by TRIE
    pub name: String,
    pub provider_name: String,
    pub aliases: Vec<String>,
    pub description: String,
    pub tier: ToolTier,
    // TODO: add schemas, visibilities, ...
}
