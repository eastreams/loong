//! Failures owned by the typed runtime tool plane.
//!
//! Lookup and dispatch stay separate because only a pre-grant `NotRegistered`
//! result may ever select legacy fallback. Once dispatch starts, a missing slot
//! is a registry invariant failure rather than another lookup opportunity.

use loong_core::tool::RegisteredToolError;
use thiserror::Error;

use super::ToolPath;

#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegistrationError {
    #[error("tool path is already registered: {path}")]
    AlreadyRegistered { path: ToolPath },
}

#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum LookupError<P> {
    #[error("tool is not registered: {path}")]
    NotRegistered { path: P },
    #[error("tool registry invariant failed for {path}")]
    RegistryInvariant { path: P },
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DispatchError<P> {
    #[error("tool registry invariant failed for {path}")]
    RegistryInvariant { path: P },
    #[error("{source}")]
    Tool {
        #[source]
        source: RegisteredToolError,
    },
}
