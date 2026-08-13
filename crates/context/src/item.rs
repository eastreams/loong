//! The minimal context item shape.

use serde::{Deserialize, Serialize};

/// One item of agent context.
///
/// The shape is intentionally minimal. It only supports log replay and
/// snapshot projection today. Richer kinds grow here when the product needs
/// them.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContextItem {
    pub role: Role,
    pub text: String,
}

/// Who produced a context item.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Role {
    #[serde(rename = "user")]
    User,
    #[serde(rename = "assistant")]
    Assistant,
    #[serde(rename = "developer")]
    Developer,
}
