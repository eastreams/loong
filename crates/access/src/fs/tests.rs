use std::{
    borrow::Cow,
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use loong_contracts::{Capability, GrantId, PolicyEntry, PolicyOutcome, PolicyReport};
use loong_core::{
    kernel::Kernel,
    policy::{
        action::ActionMeta,
        context::{CapabilityContext, ContextFactory},
        engine::PolicyEngine,
    },
};

use super::{
    FsPathKind, FsPathPolicyContext, FsResolutionContext,
    access::{FsAccess, FsAccessError},
    action::{
        FsAction, FsAtomicWriteAction, FsContentSearchAction, FsContentSearchOptions,
        FsCopyFileAction, FsCreateDirAllAction, FsGlobAction, FsInspectPathAction, FsReadAction,
        FsReadDirAction, FsRemoveDirAllAction, FsRemoveFileAction, FsRenameAction,
        FsResolvePathAction, FsWriteAction, FsWriteOptions,
    },
    path::GrantedPath,
    remove::FsRemoveFileKind,
};

#[derive(Debug, Clone)]
struct FsAccessPolicyContext {
    resolution_root: PathBuf,
    allowed_roots: Vec<PathBuf>,
    capabilities: BTreeSet<Capability>,
}

impl FsAccessPolicyContext {
    fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            resolution_root: root.clone(),
            allowed_roots: vec![root],
            capabilities: BTreeSet::from([Capability::FilesystemRead, Capability::FilesystemWrite]),
        }
    }
}

impl CapabilityContext for FsAccessPolicyContext {
    fn allowed_capabilities(&self) -> BTreeSet<Capability> {
        self.capabilities.clone()
    }
}

impl FsResolutionContext for FsAccessPolicyContext {
    fn fs_resolution_root(&self) -> &Path {
        &self.resolution_root
    }
}

impl FsPathPolicyContext for FsAccessPolicyContext {
    fn fs_allowed_roots(&self) -> &[PathBuf] {
        &self.allowed_roots
    }
}

struct FsAccessTestContextFactory;

impl ContextFactory for FsAccessTestContextFactory {
    type Cx<'a> = FsAccessPolicyContext;
}

struct FsAccessPolicyEngine {
    next_grant_id: AtomicU64,
    allow: bool,
}

impl Default for FsAccessPolicyEngine {
    fn default() -> Self {
        Self {
            next_grant_id: AtomicU64::new(0),
            allow: true,
        }
    }
}

#[async_trait]
impl PolicyEngine<FsAccessTestContextFactory> for FsAccessPolicyEngine {
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
                        policy_id: 1,
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

    async fn next_grant_id(&self) -> GrantId {
        GrantId(self.next_grant_id.fetch_add(1, Ordering::Relaxed) + 1)
    }
}

#[derive(Default)]
struct FsAccessTestKernel {
    policy: FsAccessPolicyEngine,
}

impl FsAccessTestKernel {
    fn denying() -> Self {
        Self {
            policy: FsAccessPolicyEngine {
                next_grant_id: AtomicU64::new(0),
                allow: false,
            },
        }
    }
}

#[async_trait]
impl Kernel<FsAccessTestContextFactory> for FsAccessTestKernel {
    type PolicyEngine = FsAccessPolicyEngine;

    fn policy_engine(&self) -> &Self::PolicyEngine {
        &self.policy
    }
}

struct FsAccessToolCx<'a> {
    kernel: &'a FsAccessTestKernel,
    ctx: FsAccessPolicyContext,
}

struct FsAccessTestCx<'a> {
    kernel: &'a FsAccessTestKernel,
    ctx: &'a FsAccessPolicyContext,
}

impl<'a> FsAccessToolCx<'a> {
    fn new(kernel: &'a FsAccessTestKernel, workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            kernel,
            ctx: FsAccessPolicyContext::new(workspace_root),
        }
    }

    fn access(&self) -> FsAccessTestCx<'_> {
        FsAccessTestCx {
            kernel: self.kernel,
            ctx: &self.ctx,
        }
    }
}

impl<'a> FsAccessTestCx<'a> {
    fn fs(self) -> FsAccess<'a, 'a, FsAccessTestContextFactory, FsAccessPolicyEngine> {
        FsAccess::new(self.kernel.policy_engine(), self.ctx)
    }
}

mod copy;
mod create_dir;
mod inspect;
mod path;
mod read;
mod remove;
mod rename;
mod search;
mod write;

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
}
