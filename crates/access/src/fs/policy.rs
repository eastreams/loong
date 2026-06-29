//! Filesystem resource policies.
//!
//! These policies compare canonical action paths with canonical policy operands:
//! exact paths use `CanonicalPath`, prefix policies use `CanonicalPrefix`, and
//! workspace policies use the canonical workspace path carried by the policy
//! context.

use std::path::PathBuf;

use super::action::{CanonicalPath, CanonicalPrefix};

// TODO: implement Policy for these Policies

/// Policy that allows the shared filesystem parent action inside the workspace.
pub struct AllowWorkspaceFsPolicy;

/// Policy that only allows reading one exact file path.
pub struct AllowExactFileReadPolicy {
    allowed: CanonicalPath,
}

impl AllowExactFileReadPolicy {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            allowed: CanonicalPath::existing(path.into()).expect("read policy path must exist"),
        }
    }
}

/// Policy that allows file reads under a prefix.
pub struct AllowFileReadPrefixPolicy {
    prefix: CanonicalPrefix,
}

impl AllowFileReadPrefixPolicy {
    pub fn new(prefix: impl Into<PathBuf>) -> Self {
        Self {
            prefix: CanonicalPrefix::existing(prefix.into()).expect("read prefix must exist"),
        }
    }
}
/// Policy that allows file reads under the current workspace root.
pub struct AllowWorkspaceReadPolicy;

/// Policy that only allows writing one exact file path.
pub struct AllowExactFileWritePolicy {
    allowed: CanonicalPath,
}

impl AllowExactFileWritePolicy {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            allowed: CanonicalPath::existing_or_parent(path.into())
                .expect("write policy path parent must exist"),
        }
    }
}
/// Policy that allows file writes under a prefix.
pub struct AllowFileWritePrefixPolicy {
    prefix: CanonicalPrefix,
}

impl AllowFileWritePrefixPolicy {
    pub fn new(prefix: impl Into<PathBuf>) -> Self {
        Self {
            prefix: CanonicalPrefix::existing(prefix.into()).expect("write prefix must exist"),
        }
    }
}

/// Policy that allows file writes under the current workspace root.
pub struct AllowWorkspaceWritePolicy;
