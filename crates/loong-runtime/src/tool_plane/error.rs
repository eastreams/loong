//! Failures owned by the typed runtime tool plane.
//!
//! Only a pre-grant `NotRegistered` result may select legacy fallback. A
//! successful lookup binds the registered entry into `ToolInvocation`, so
//! execution cannot produce a second registry lookup failure.

use std::fmt;

use loong_contracts::{AuditError, Capabilities, GrantId, ToolPath};
use loong_core::error::PolicyGrantError;
use serde_json::Value;
use thiserror::Error;

use super::RegisteredToolError;

#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegistrationError {
    #[error("tool path is already registered: {path}")]
    AlreadyRegistered { path: ToolPath },
}

#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum LookupError {
    #[error("tool is not registered: {path}")]
    NotRegistered { path: ToolPath },
    #[error("tool registry invariant failed for {path}")]
    RegistryInvariant { path: ToolPath },
}

/// A requested tool override exceeded the tool's declared default authority.
#[derive(Debug, Error, PartialEq, Eq)]
#[error(
    "tool capability override for {path} requested {requested:?}, which is not a subset of declared capabilities {declared:?}"
)]
pub struct CapabilityOverrideError {
    pub path: ToolPath,
    pub requested: Capabilities,
    pub declared: Capabilities,
}

/// A child execution scope attempted to exceed its parent authority.
#[derive(Debug, Error, PartialEq, Eq)]
#[error(
    "derived child tool context capabilities {derived:?} exceed allowed capabilities {allowed:?}"
)]
pub struct CapabilityNarrowingError {
    pub allowed: Capabilities,
    pub derived: Capabilities,
}

/// Failure from the runtime-owned typed tool invocation boundary.
///
/// Variants distinguish failures before dispatch from failures after the tool
/// may have produced side effects. Callers must not retry either audit-failure
/// variant as though dispatch had never started.
#[derive(Error)]
pub enum ToolInvocationError {
    #[error(transparent)]
    CapabilityOverride(#[from] CapabilityOverrideError),
    #[error(
        "tool capability override was rejected: {rejection}; recording that rejection also failed: {audit_source}"
    )]
    CapabilityOverrideAndAudit {
        #[source]
        rejection: CapabilityOverrideError,
        audit_source: AuditError,
    },
    #[error(transparent)]
    CapabilityNarrowing(#[from] CapabilityNarrowingError),
    #[error(transparent)]
    Authorization(#[from] PolicyGrantError),
    #[error("tool dispatch did not start because execution audit failed: {source}")]
    StartAudit {
        grant_id: GrantId,
        #[source]
        source: AuditError,
    },
    #[error("tool dispatch failed: {source}")]
    Dispatch {
        grant_id: GrantId,
        #[source]
        source: RegisteredToolError,
    },
    #[error("tool completed, but terminal execution audit failed: {source}")]
    CompletedAudit {
        grant_id: GrantId,
        output: Value,
        #[source]
        source: AuditError,
    },
    #[error(
        "tool dispatch failed: {dispatch_source}; terminal execution audit also failed: {audit_source}"
    )]
    DispatchAndAudit {
        grant_id: GrantId,
        #[source]
        dispatch_source: RegisteredToolError,
        audit_source: AuditError,
    },
}

impl fmt::Debug for ToolInvocationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapabilityOverride(source) => formatter
                .debug_tuple("CapabilityOverride")
                .field(source)
                .finish(),
            Self::CapabilityOverrideAndAudit {
                rejection,
                audit_source,
            } => formatter
                .debug_struct("CapabilityOverrideAndAudit")
                .field("rejection", rejection)
                .field("audit_source", audit_source)
                .finish(),
            Self::CapabilityNarrowing(source) => formatter
                .debug_tuple("CapabilityNarrowing")
                .field(source)
                .finish(),
            Self::Authorization(source) => formatter
                .debug_tuple("Authorization")
                .field(source)
                .finish(),
            Self::StartAudit { grant_id, source } => formatter
                .debug_struct("StartAudit")
                .field("grant_id", grant_id)
                .field("source", source)
                .finish(),
            Self::Dispatch { grant_id, source } => formatter
                .debug_struct("Dispatch")
                .field("grant_id", grant_id)
                .field("source", source)
                .finish(),
            Self::CompletedAudit {
                grant_id,
                output: _,
                source,
            } => formatter
                .debug_struct("CompletedAudit")
                .field("grant_id", grant_id)
                .field("output", &"[redacted]")
                .field("source", source)
                .finish(),
            Self::DispatchAndAudit {
                grant_id,
                dispatch_source,
                audit_source,
            } => formatter
                .debug_struct("DispatchAndAudit")
                .field("grant_id", grant_id)
                .field("dispatch_source", dispatch_source)
                .field("audit_source", audit_source)
                .finish(),
        }
    }
}
