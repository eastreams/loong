use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contracts::Capability;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PolicyContext {
    pub conversation_hash: Option<String>,
    pub call_depth: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyRequest {
    pub tool_name: String,
    pub parameters: Value,
    pub pack_id: String,
    pub agent_id: String,
    pub capabilities_used: BTreeSet<Capability>,
    pub context: PolicyContext,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyDecision {
    Allow,
    Deny(String),
}
