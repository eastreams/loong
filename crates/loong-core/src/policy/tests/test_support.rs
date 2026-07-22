use std::{
    borrow::Cow,
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use loong_contracts::{
    AuthorizationAttemptId, AuthorizationEvidence, AuthorizationScope, AuthorizationSubject,
    Capabilities, Capability, GrantId, PermissionResolution, PolicyEntry, PolicyId, PolicyOutcome,
    PolicyRegistration, PolicyRegistrationSource, PolicyReport,
};
use serde_json::{Value, json};

use crate::{
    error::PermissionRequestError,
    policy::{
        action::{ActionMeta, ActionMetadata},
        context::{ContextFactory, PolicyContext},
        engine::PolicyEngineBackend,
    },
};

pub(super) struct TestContext {
    capabilities: Mutex<Capabilities>,
    pub(super) revoke_on_user_permission: bool,
    pub(super) post_user_permission_capability_snapshots: Mutex<VecDeque<Capabilities>>,
    user_permission_completed: AtomicBool,
    pub(super) capability_reads: AtomicUsize,
    parent_resolutions: Mutex<VecDeque<Result<PermissionResolution, PermissionRequestError>>>,
    user_resolutions: Mutex<VecDeque<Result<PermissionResolution, PermissionRequestError>>>,
}

impl TestContext {
    pub(super) fn with_capabilities(capabilities: Capabilities) -> Self {
        Self {
            capabilities: Mutex::new(capabilities),
            revoke_on_user_permission: false,
            post_user_permission_capability_snapshots: Mutex::new(VecDeque::new()),
            user_permission_completed: AtomicBool::new(false),
            capability_reads: AtomicUsize::new(0),
            parent_resolutions: Mutex::new(VecDeque::new()),
            user_resolutions: Mutex::new(VecDeque::new()),
        }
    }

    pub(super) fn with_permissions(
        capabilities: Capabilities,
        parent: impl IntoIterator<Item = Result<PermissionResolution, PermissionRequestError>>,
        user: impl IntoIterator<Item = Result<PermissionResolution, PermissionRequestError>>,
    ) -> Self {
        Self {
            capabilities: Mutex::new(capabilities),
            revoke_on_user_permission: false,
            post_user_permission_capability_snapshots: Mutex::new(VecDeque::new()),
            user_permission_completed: AtomicBool::new(false),
            capability_reads: AtomicUsize::new(0),
            parent_resolutions: Mutex::new(parent.into_iter().collect()),
            user_resolutions: Mutex::new(user.into_iter().collect()),
        }
    }
}

#[async_trait]
impl PolicyContext for TestContext {
    fn allowed_capabilities(&self) -> Cow<'_, Capabilities> {
        self.capability_reads.fetch_add(1, Ordering::Relaxed);
        if self.user_permission_completed.load(Ordering::Relaxed)
            && let Some(snapshot) = self
                .post_user_permission_capability_snapshots
                .lock()
                .expect("capability snapshot lock")
                .pop_front()
        {
            return Cow::Owned(snapshot);
        }
        Cow::Owned(self.capabilities.lock().expect("capability lock").clone())
    }

    fn authorization_subject(&self) -> AuthorizationSubject {
        AuthorizationSubject {
            actor_id: "actor:test:core-policy".to_owned(),
            scope: AuthorizationScope::Session {
                session_id: "session:test:core-policy".to_owned(),
            },
        }
    }

    async fn request_parent_permission(
        &self,
        _action: &dyn ActionMeta,
        _report: &PolicyReport,
    ) -> Result<PermissionResolution, PermissionRequestError> {
        self.parent_resolutions
            .lock()
            .expect("parent resolution lock")
            .pop_front()
            .expect("parent permission resolution should be configured")
    }

    async fn request_user_permission(
        &self,
        _action: &dyn ActionMeta,
        _report: &PolicyReport,
    ) -> Result<PermissionResolution, PermissionRequestError> {
        self.user_permission_completed
            .store(true, Ordering::Relaxed);
        if self.revoke_on_user_permission {
            *self.capabilities.lock().expect("capability lock") = Capabilities::new();
        }
        self.user_resolutions
            .lock()
            .expect("user resolution lock")
            .pop_front()
            .expect("user permission resolution should be configured")
    }
}

pub(super) struct TestContextFactory;

impl ContextFactory for TestContextFactory {
    type Cx<'a> = TestContext;
}

#[derive(Debug)]
pub(super) struct TestAction;

impl ActionMeta for TestAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "test.action",
            operation: Cow::Borrowed("read"),
            required_capabilities: Cow::Borrowed(&[Capability::FilesystemRead]),
        }
    }

    fn audit_resource(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed("fixture://core-policy"))
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({}))
    }
}

#[derive(Debug)]
pub(super) struct MultiCapabilityAction;

impl ActionMeta for MultiCapabilityAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "test.multi_capability_action",
            operation: Cow::Borrowed("read_and_write"),
            required_capabilities: Cow::Borrowed(&[
                Capability::FilesystemRead,
                Capability::FilesystemWrite,
            ]),
        }
    }

    fn payload(&self) -> Cow<'_, Value> {
        Cow::Owned(json!({}))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(super) enum TestAuditError {
    #[error("authorization attempt allocation failed")]
    Attempt,
    #[error("grant id allocation failed")]
    Grant,
    #[error("authorization evidence write failed")]
    Write,
}

pub(super) struct CollectingBackend {
    attempts: AtomicU64,
    pub(super) grants: AtomicU64,
    pub(super) decisions: AtomicUsize,
    report: PolicyReport,
    pub(super) fail_attempt: bool,
    pub(super) fail_grant: bool,
    pub(super) fail_write_at: Option<usize>,
    pub(super) revoke_capabilities_during_decide: bool,
    writes: AtomicUsize,
    pub(super) evidence: Mutex<Vec<AuthorizationEvidence>>,
}

impl CollectingBackend {
    pub(super) fn new(report: PolicyReport) -> Self {
        Self {
            attempts: AtomicU64::new(0),
            grants: AtomicU64::new(0),
            decisions: AtomicUsize::new(0),
            report,
            fail_attempt: false,
            fail_grant: false,
            fail_write_at: None,
            revoke_capabilities_during_decide: false,
            writes: AtomicUsize::new(0),
            evidence: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl PolicyEngineBackend<TestContextFactory> for CollectingBackend {
    type AuditError = TestAuditError;

    async fn decide<A: ActionMeta + 'static>(
        &self,
        ctx: &<TestContextFactory as ContextFactory>::Cx<'_>,
        _action: &A,
    ) -> PolicyReport {
        self.decisions.fetch_add(1, Ordering::Relaxed);
        if self.revoke_capabilities_during_decide {
            tokio::task::yield_now().await;
            *ctx.capabilities.lock().expect("capability lock") = Capabilities::new();
        }
        self.report.clone()
    }

    fn reserve_authorization_attempt_id(&self) -> Result<AuthorizationAttemptId, Self::AuditError> {
        if self.fail_attempt {
            return Err(TestAuditError::Attempt);
        }
        Ok(AuthorizationAttemptId(
            self.attempts.fetch_add(1, Ordering::Relaxed) + 1,
        ))
    }

    fn reserve_grant_id(&self) -> Result<GrantId, Self::AuditError> {
        if self.fail_grant {
            return Err(TestAuditError::Grant);
        }
        self.grants.fetch_add(1, Ordering::Relaxed);
        Ok(GrantId::new())
    }

    fn write_authorization_evidence(
        &self,
        evidence: &AuthorizationEvidence,
    ) -> Result<(), Self::AuditError> {
        let write_number = self.writes.fetch_add(1, Ordering::Relaxed) + 1;
        if self.fail_write_at == Some(write_number) {
            return Err(TestAuditError::Write);
        }
        self.evidence
            .lock()
            .expect("evidence lock should remain available")
            .push(evidence.clone());
        Ok(())
    }
}

pub(super) fn policy_entry(name: &'static str) -> PolicyEntry {
    PolicyEntry {
        policy_name: Cow::Borrowed(name),
        policy_id: PolicyId::new(1),
        registration: PolicyRegistration {
            order: 1,
            registered_at_unix_ms: 1,
            source: PolicyRegistrationSource {
                file: "policy/tests/test_support.rs".to_owned(),
                line: 1,
                column: 1,
            },
        },
    }
}

pub(super) fn allow_report() -> PolicyReport {
    PolicyReport {
        evaluations: Vec::new(),
        outcome: PolicyOutcome::Allow {
            source: policy_entry("test-allow"),
            reason: Cow::Borrowed("allowed by test policy"),
        },
    }
}

pub(super) fn user_permission_report() -> PolicyReport {
    PolicyReport {
        evaluations: Vec::new(),
        outcome: PolicyOutcome::RequireUserPermission {
            source: policy_entry("test-user-permission"),
            reason: Cow::Borrowed("user approval required"),
        },
    }
}
