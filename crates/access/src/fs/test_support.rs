use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use loong_contracts::{
    AuthorizationAttemptId, AuthorizationEvidence, AuthorizationScope, AuthorizationSubject,
    Capabilities, Capability, GrantId, PolicyEntry, PolicyId, PolicyOutcome, PolicyRegistration,
    PolicyRegistrationSource, PolicyReport,
};
use loong_core::{
    kernel::Kernel,
    policy::{
        action::ActionMeta,
        context::{ContextFactory, PolicyContext},
        engine::{PolicyEngine, PolicyEngineBackend},
    },
};

use super::{
    FsPathPolicyContext, FsResolutionContext,
    access::FsAccess,
    path::{FsPathAction, FsResolvePathAction, GrantedPath},
};

#[derive(Debug, Clone)]
pub(super) struct FsAccessTestContext {
    resolution_root: PathBuf,
    allowed_roots: Vec<PathBuf>,
    authority_ceiling_roots: Vec<PathBuf>,
    capabilities: Capabilities,
}

impl FsAccessTestContext {
    pub(super) fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            resolution_root: root.clone(),
            allowed_roots: vec![root.clone()],
            authority_ceiling_roots: vec![root],
            capabilities: Capabilities::from([
                Capability::FilesystemRead,
                Capability::FilesystemWrite,
            ]),
        }
    }

    pub(super) fn with_authority_ceiling(
        root: impl Into<PathBuf>,
        authority_ceiling: impl Into<PathBuf>,
    ) -> Self {
        let root = root.into();
        Self {
            resolution_root: root.clone(),
            allowed_roots: vec![root],
            authority_ceiling_roots: vec![authority_ceiling.into()],
            capabilities: Capabilities::from([
                Capability::FilesystemRead,
                Capability::FilesystemWrite,
            ]),
        }
    }
}

impl PolicyContext for FsAccessTestContext {
    fn allowed_capabilities(&self) -> Cow<'_, Capabilities> {
        Cow::Borrowed(&self.capabilities)
    }

    fn authorization_subject(&self) -> AuthorizationSubject {
        AuthorizationSubject {
            actor_id: "test:access:fs:actor".to_owned(),
            scope: AuthorizationScope::Session {
                session_id: "test:access:fs:session".to_owned(),
            },
        }
    }
}

impl FsResolutionContext for FsAccessTestContext {
    fn fs_resolution_root(&self) -> &Path {
        &self.resolution_root
    }
}

impl FsPathPolicyContext for FsAccessTestContext {
    fn fs_allowed_roots(&self) -> &[PathBuf] {
        &self.allowed_roots
    }

    fn fs_authority_ceiling_roots(&self) -> &[PathBuf] {
        &self.authority_ceiling_roots
    }
}

pub(super) struct FsAccessTestContextFactory;

impl ContextFactory for FsAccessTestContextFactory {
    type Cx<'a> = FsAccessTestContext;
}

pub(super) struct FsAccessPolicyEngine {
    attempt_seq: AtomicU64,
    pub(super) evidence: Mutex<Vec<AuthorizationEvidence>>,
    allow: bool,
}

impl Default for FsAccessPolicyEngine {
    fn default() -> Self {
        Self {
            attempt_seq: AtomicU64::new(0),
            evidence: Mutex::new(Vec::new()),
            allow: true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum FsAccessAuditError {
    #[error("filesystem access test evidence collector is poisoned: {reason}")]
    EvidencePoisoned { reason: String },
}

#[async_trait]
impl PolicyEngineBackend<FsAccessTestContextFactory> for FsAccessPolicyEngine {
    type AuditError = FsAccessAuditError;

    async fn decide<A: ActionMeta + 'static>(
        &self,
        _ctx: &<FsAccessTestContextFactory as ContextFactory>::Cx<'_>,
        _action: &A,
    ) -> PolicyReport {
        if self.allow {
            return PolicyReport {
                evaluations: Vec::new(),
                outcome: PolicyOutcome::Allow {
                    source: PolicyEntry {
                        policy_name: Cow::Borrowed("allow-all"),
                        policy_id: PolicyId::new(1),
                        registration: PolicyRegistration {
                            order: 1,
                            registered_at_unix_ms: 1,
                            source: PolicyRegistrationSource {
                                file: "fs/test_support.rs".to_owned(),
                                line: 1,
                                column: 1,
                            },
                        },
                    },
                    reason: Cow::Borrowed("allowed"),
                },
            };
        }

        PolicyReport {
            evaluations: Vec::new(),
            outcome: PolicyOutcome::Deny {
                grant_source: None,
                reason: Cow::Borrowed("denied by test policy"),
            },
        }
    }

    fn reserve_authorization_attempt_id(&self) -> Result<AuthorizationAttemptId, Self::AuditError> {
        Ok(AuthorizationAttemptId(
            self.attempt_seq.fetch_add(1, Ordering::Relaxed) + 1,
        ))
    }

    fn reserve_grant_id(&self) -> Result<GrantId, Self::AuditError> {
        Ok(GrantId::new())
    }

    fn write_authorization_evidence(
        &self,
        evidence: &AuthorizationEvidence,
    ) -> Result<(), Self::AuditError> {
        self.evidence
            .lock()
            .map_err(|error| FsAccessAuditError::EvidencePoisoned {
                reason: error.to_string(),
            })?
            .push(evidence.clone());
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct FsAccessTestKernel {
    pub(super) policy: FsAccessPolicyEngine,
}

impl FsAccessTestKernel {
    pub(super) fn denying() -> Self {
        Self {
            policy: FsAccessPolicyEngine {
                attempt_seq: AtomicU64::new(0),
                evidence: Mutex::new(Vec::new()),
                allow: false,
            },
        }
    }
}

impl Kernel<FsAccessTestContextFactory> for FsAccessTestKernel {
    fn policy_engine(&self) -> &impl PolicyEngine<FsAccessTestContextFactory> {
        &self.policy
    }
}

// Target-path action tests intentionally exercise the production resolve ->
// path protocol instead of minting the crate-private typestate directly. Keep
// that protocol in one test helper rather than duplicating it in every module.
pub(super) async fn grant_target_path(
    kernel: &FsAccessTestKernel,
    ctx: &FsAccessTestContext,
    path: impl AsRef<Path>,
) -> GrantedPath {
    let resolved = kernel
        .policy_engine()
        .grant(ctx, FsResolvePathAction::target(path, ctx))
        .await
        .expect("test policy should grant target path resolution")
        .into_granted()
        .run(ctx)
        .await
        .expect("granted target resolve action should run");
    kernel
        .policy_engine()
        .grant(ctx, FsPathAction::new(resolved))
        .await
        .expect("test policy should grant target path")
        .into_granted()
        .run(ctx)
        .await
        .expect("granted target path action should run")
}

pub(super) struct FsAccessToolCx<'a> {
    kernel: &'a FsAccessTestKernel,
    ctx: FsAccessTestContext,
}

pub(super) struct FsAccessTestCx<'a> {
    kernel: &'a FsAccessTestKernel,
    ctx: &'a FsAccessTestContext,
}

impl<'a> FsAccessToolCx<'a> {
    pub(super) fn new(kernel: &'a FsAccessTestKernel, workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            kernel,
            ctx: FsAccessTestContext::new(workspace_root),
        }
    }

    pub(super) fn access(&self) -> FsAccessTestCx<'_> {
        FsAccessTestCx {
            kernel: self.kernel,
            ctx: &self.ctx,
        }
    }
}

impl<'a> FsAccessTestCx<'a> {
    pub(super) fn fs(
        self,
    ) -> FsAccess<'a, 'a, FsAccessTestContextFactory, impl PolicyEngine<FsAccessTestContextFactory>>
    {
        FsAccess::new(self.kernel.policy_engine(), self.ctx)
    }
}

#[cfg(unix)]
pub(super) fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

pub(super) fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
}
