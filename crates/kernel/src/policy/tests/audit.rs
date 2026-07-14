use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use loong_contracts::{
    AuthorizationAttempt, AuthorizationAttemptEvent, AuthorizationEvidence,
    AuthorizationPermissionInteraction, AuthorizationPolicyEvent, AuthorizationScope,
    AuthorizationSubject, AuthorizationTerminalOutcome, Capabilities, PermissionResolution,
    PolicyReport,
};
use loong_core::{
    AuthorizationError, PermissionRequestError, PolicyGrantError, kernel::Kernel as _,
};
use serde_json::json;

use super::*;
use crate::{
    AccessCx, AuditError, AuditEvent, AuditEventKind, AuditSink, FixedClock, InMemoryAuditSink,
    Kernel,
    access::fs::{FsAccessError, FsPathPolicyContext, FsResolutionContext},
};

struct AuditContext {
    capabilities: Capabilities,
}

impl PolicyContext for AuditContext {
    fn allowed_capabilities(&self) -> Cow<'_, Capabilities> {
        Cow::Borrowed(&self.capabilities)
    }

    fn authorization_subject(&self) -> AuthorizationSubject {
        AuthorizationSubject {
            actor_id: "test:kernel:audit:actor".to_owned(),
            scope: AuthorizationScope::Session {
                session_id: "test:kernel:audit:session".to_owned(),
            },
        }
    }
}

struct AuditContextFactory;

impl ContextFactory for AuditContextFactory {
    type Cx<'a> = AuditContext;
}

struct PermissionAuditContext {
    capabilities: Capabilities,
    resolution: PermissionResolution,
}

#[async_trait]
impl PolicyContext for PermissionAuditContext {
    fn allowed_capabilities(&self) -> Cow<'_, Capabilities> {
        Cow::Borrowed(&self.capabilities)
    }

    fn authorization_subject(&self) -> AuthorizationSubject {
        AuthorizationSubject {
            actor_id: "test:kernel:permission-audit:actor".to_owned(),
            scope: AuthorizationScope::Session {
                session_id: "test:kernel:permission-audit:session".to_owned(),
            },
        }
    }

    async fn request_user_permission(
        &self,
        _action: &dyn ActionMeta,
        _report: &PolicyReport,
    ) -> Result<PermissionResolution, PermissionRequestError> {
        Ok(self.resolution.clone())
    }
}

struct PermissionAuditContextFactory;

impl ContextFactory for PermissionAuditContextFactory {
    type Cx<'a> = PermissionAuditContext;
}

#[derive(Debug)]
struct AuditAction {
    operation: &'static str,
    required_capabilities: Vec<Capability>,
}

impl AuditAction {
    fn invoke_tool(operation: &'static str) -> Self {
        Self {
            operation,
            required_capabilities: vec![Capability::InvokeTool],
        }
    }

    fn filesystem_read() -> Self {
        Self {
            operation: "read",
            required_capabilities: vec![Capability::FilesystemRead],
        }
    }
}

impl ActionMeta for AuditAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "test.audit",
            operation: Cow::Borrowed(self.operation),
            required_capabilities: Cow::Borrowed(&self.required_capabilities),
        }
    }

    fn payload(&self) -> Cow<'_, serde_json::Value> {
        Cow::Owned(json!({}))
    }
}

mod failure;
mod flow;
