use std::fmt;

use serde::{Deserialize, Serialize};

use crate::contracts::Capability;
use crate::errors::{KernelError, PolicyError};

/// Structured error type for kernel dispatch failures.
///
/// Unlike `KernelError` (which covers all kernel operations including setup),
/// `Fault` represents runtime dispatch failures that callers can match on
/// to decide recovery strategy.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fault {
    Panic {
        message: String,
    },
    CapabilityViolation {
        token_id: String,
        capability: Capability,
    },
    BudgetExhausted {
        resource: String,
        limit: u64,
        used: u64,
    },
    TokenExpired {
        token_id: String,
        expires_at_epoch_s: u64,
    },
    ProtocolViolation {
        detail: String,
    },
    PolicyDenied {
        reason: String,
    },
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Panic { message } => write!(f, "panic: {message}"),
            Self::CapabilityViolation {
                token_id,
                capability,
            } => {
                write!(
                    f,
                    "capability violation: token {token_id} missing {capability:?}"
                )
            }
            Self::BudgetExhausted {
                resource,
                limit,
                used,
            } => {
                write!(f, "budget exhausted: {resource} used {used}/{limit}")
            }
            Self::TokenExpired {
                token_id,
                expires_at_epoch_s,
            } => {
                write!(f, "token {token_id} expired at {expires_at_epoch_s}")
            }
            Self::ProtocolViolation { detail } => write!(f, "protocol violation: {detail}"),
            Self::PolicyDenied { reason } => write!(f, "policy denied: {reason}"),
        }
    }
}

impl std::error::Error for Fault {}

impl Fault {
    pub fn from_policy_error(err: PolicyError) -> Self {
        match err {
            PolicyError::ExpiredToken {
                token_id,
                expires_at_epoch_s,
            } => Self::TokenExpired {
                token_id,
                expires_at_epoch_s,
            },
            PolicyError::MissingCapability {
                token_id,
                capability,
            } => Self::CapabilityViolation {
                token_id,
                capability,
            },
            PolicyError::RevokedToken { token_id } => Self::PolicyDenied {
                reason: format!("token {token_id} revoked"),
            },
            PolicyError::PackMismatch {
                token_pack_id,
                runtime_pack_id,
            } => Self::PolicyDenied {
                reason: format!("pack mismatch: token={token_pack_id} runtime={runtime_pack_id}"),
            },
            PolicyError::ExtensionDenied { extension, reason } => Self::PolicyDenied {
                reason: format!("extension {extension}: {reason}"),
            },
            PolicyError::ToolCallDenied { tool_name, reason } => Self::PolicyDenied {
                reason: format!("tool {tool_name}: {reason}"),
            },
            PolicyError::ToolCallApprovalRequired { tool_name, prompt } => Self::PolicyDenied {
                reason: format!("tool {tool_name} requires approval: {prompt}"),
            },
        }
    }

    pub fn from_kernel_error(err: KernelError) -> Self {
        match err {
            KernelError::Policy(policy_err) => Self::from_policy_error(policy_err),
            KernelError::PackCapabilityBoundary {
                capability,
                pack_id,
            } => Self::CapabilityViolation {
                token_id: format!("pack:{pack_id}"),
                capability,
            },
            KernelError::PackNotFound(_)
            | KernelError::DuplicatePack(_)
            | KernelError::ConnectorNotAllowed { .. }
            | KernelError::Pack(_)
            | KernelError::Harness(_)
            | KernelError::Connector(_)
            | KernelError::RuntimePlane(_)
            | KernelError::ToolPlane(_)
            | KernelError::MemoryPlane(_)
            | KernelError::Integration(_)
            | KernelError::Audit(_) => Self::Panic {
                message: err.to_string(),
            },
        }
    }
}
