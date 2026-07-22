use std::{
    borrow::Cow,
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use loong_contracts::{Capability, PolicyDecision, PolicyGrant};
use loong_core::{
    error::PolicyGrantError,
    policy::{
        action::{Action, ActionMeta, ActionMetadata},
        context::ContextFactory,
        engine::PolicyEngine,
        grant::Granted,
        policy::Policy,
    },
};
use serde_json::{Value, json};
use thiserror::Error;

use super::{
    access::FsAccess,
    path::{FsPathPolicyContext, FsResolutionContext, GrantedPath},
};

#[cfg(test)]
mod tests;

const FS_READ_REQUIRED_CAPABILITIES: [Capability; 1] = [Capability::FilesystemRead];

#[derive(Debug, Error)]
pub enum FsReadError {
    #[error(transparent)]
    Path(#[from] super::path::FsPathError),
    #[error(transparent)]
    Authorization(#[from] PolicyGrantError),
    #[error("failed to read file {path}: {source}", path = .path.display())]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Typed action for reading one governed filesystem path.
///
/// The action declares the required capability and audit resource. It does not
/// carry workspace roots; roots belong to path resolution. Its constructor
/// accepts `GrantedPath` so path policy cannot be bypassed with a raw or merely
/// canonical `PathBuf`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsReadAction {
    path: GrantedPath,
}

impl FsReadAction {
    #[must_use]
    pub fn new(path: GrantedPath) -> Self {
        Self { path }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }
}

impl ActionMeta for FsReadAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "fs.read",
            operation: Cow::Borrowed("read_file"),
            required_capabilities: Cow::Borrowed(&FS_READ_REQUIRED_CAPABILITIES),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(self.path.as_path().display().to_string().into())
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({
            "path": self.path.as_path().display().to_string(),
        }))
    }
}

/// Configurable filename deny policy evaluated before the terminal read allow.
#[derive(Debug, Clone)]
pub struct FsReadFilenameDenyPolicy {
    denied_filenames: BTreeSet<String>,
}

impl FsReadFilenameDenyPolicy {
    #[must_use]
    pub fn new(denied_filenames: BTreeSet<String>) -> Self {
        let denied_filenames = denied_filenames
            .into_iter()
            .filter_map(|filename| normalize_policy_filename(filename.as_str()))
            .collect();
        Self { denied_filenames }
    }
}

#[async_trait]
impl<C> Policy<C, FsReadAction> for FsReadFilenameDenyPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-read-filename-deny")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, action: &FsReadAction) -> PolicyGrant {
        let denied_filename = action
            .path()
            .file_name()
            .and_then(|filename| filename.to_str())
            .and_then(normalize_policy_filename)
            .filter(|filename| self.denied_filenames.contains(filename));

        if let Some(filename) = denied_filename {
            return PolicyGrant {
                decision: PolicyDecision::Deny,
                predicate: Some(format!("fs.read filename == {filename:?}").into()),
                reason: format!("file read denied by configured filename policy: {filename}")
                    .into(),
            };
        }

        PolicyGrant {
            decision: PolicyDecision::Continue,
            predicate: None,
            reason: "filename did not match configured read deny policy".into(),
        }
    }
}

/// Terminal read policy installed after configured deny policies.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsReadAllowPolicy;

#[async_trait]
impl<C> Policy<C, FsReadAction> for FsReadAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-read-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &FsReadAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("fs.read reached terminal allow policy".into()),
            reason: "filesystem read allowed after configured deny policies".into(),
        }
    }
}

impl<'a, 'ctx, C, P> FsAccess<'a, 'ctx, C, P>
where
    C: ContextFactory + 'ctx,
    P: PolicyEngine<C>,
    C::Cx<'ctx>: FsResolutionContext + FsPathPolicyContext,
{
    /// Read a file through resolution, path authorization, and read policy.
    ///
    /// A granted resolve action prepares filesystem facts; path policy decides
    /// whether those facts are allowed; only then can `FsReadAction` receive a
    /// `GrantedPath` and perform the file read.
    pub async fn read_file(self, path: impl AsRef<Path>) -> Result<FsReadOutput, FsReadError> {
        let path = self.grant_target_path(path).await?;

        let action = FsReadAction::new(path);
        let grant = self.policy_engine.grant(self.ctx, action).await?;
        grant.into_granted().run(self.ctx).await
    }
}

/// Execute an already-authorized fs read.
///
/// This is the concrete side-effect boundary for fs reads. It deliberately
/// consumes `Granted<FsReadAction>` so raw actions cannot reach the filesystem.
#[async_trait]
impl<Cx> Action<Cx> for FsReadAction
where
    Cx: Sync,
{
    type Output = FsReadOutput;
    type Error = FsReadError;

    async fn run(granted: Granted<Self>, _ctx: &Cx) -> Result<Self::Output, Self::Error> {
        let action = granted.into_action();
        let path = action.path().to_path_buf();
        let bytes = std::fs::read(&path).map_err(|source| FsReadError::ReadFile {
            path: path.clone(),
            source,
        })?;
        Ok(FsReadOutput { path, bytes })
    }
}

/// Bytes returned by a governed fs read.
///
/// `path` is the resolved path actually read, suitable for response metadata
/// and audit output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsReadOutput {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

// Normalize configured and observed names through one rule so policy matching
// cannot become asymmetric across the two inputs.
fn normalize_policy_filename(filename: &str) -> Option<String> {
    let normalized = filename.trim().to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}
