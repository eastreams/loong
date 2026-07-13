use std::{
    borrow::Cow,
    sync::{Arc, Mutex},
};

use loong_contracts::{Capabilities, PermissionResolution, PolicyReport};
use loong_core::{PermissionRequestError, PolicyGrantError};

use super::*;
use crate::{FixedClock, InMemoryAuditSink, Kernel, KernelError, PolicyError};

struct PermissionContextFactory;

impl ContextFactory for PermissionContextFactory {
    type Cx<'a> = PermissionPolicyContext<'a>;
}

struct PermissionPolicyContext<'a> {
    allowed_capabilities: &'a Capabilities,
    parent_resolution: Result<PermissionResolution, PermissionRequestError>,
    user_resolution: Result<PermissionResolution, PermissionRequestError>,
    requests: Mutex<Vec<&'static str>>,
}

impl<'a> PermissionPolicyContext<'a> {
    fn new(
        allowed_capabilities: &'a Capabilities,
        parent_resolution: Result<PermissionResolution, PermissionRequestError>,
        user_resolution: Result<PermissionResolution, PermissionRequestError>,
    ) -> Self {
        Self {
            allowed_capabilities,
            parent_resolution,
            user_resolution,
            requests: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl PolicyContext for PermissionPolicyContext<'_> {
    fn allowed_capabilities(&self) -> Cow<'_, Capabilities> {
        Cow::Borrowed(self.allowed_capabilities)
    }

    async fn request_parent_permission(
        &self,
        _action: &dyn ActionMeta,
        _report: &PolicyReport,
    ) -> Result<PermissionResolution, PermissionRequestError> {
        self.requests
            .lock()
            .expect("permission request log")
            .push("parent");
        self.parent_resolution.clone()
    }

    async fn request_user_permission(
        &self,
        _action: &dyn ActionMeta,
        _report: &PolicyReport,
    ) -> Result<PermissionResolution, PermissionRequestError> {
        self.requests
            .lock()
            .expect("permission request log")
            .push("user");
        self.user_resolution.clone()
    }
}

#[tokio::test]
async fn policy_pipeline_parent_permission_is_a_terminal_outcome() {
    let engine = PolicyPipeline::<TestContextFactory>::new()
        .with_pre_policy(StaticAnyPolicy {
            name: "parent-permission",
            decision: PolicyDecision::RequireParentPermission,
            reason: "parent must approve",
        })
        .with_policy::<LegacyKernelAction, _>(StaticTypedPolicy {
            name: "typed-deny",
            decision: PolicyDecision::Deny,
            reason: "must not run",
        });
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(
        &pack,
        &token,
        1,
        ExecutionPlane::Tool,
        PlaneTier::Core,
        None,
    );
    let action = LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]));

    let report = engine.decide(&ctx, &action).await;

    assert_eq!(report.evaluations.len(), 1);
    assert!(matches!(
        report.outcome,
        PolicyOutcome::RequireParentPermission { ref source, .. }
            if source.policy_name == "parent-permission"
    ));
}

#[tokio::test]
async fn policy_pipeline_user_permission_is_a_terminal_outcome() {
    let engine = PolicyPipeline::<TestContextFactory>::new()
        .with_policy::<LegacyKernelAction, _>(StaticTypedPolicy {
            name: "user-permission",
            decision: PolicyDecision::RequireUserPermission,
            reason: "user must approve",
        })
        .with_fallback_policy(StaticAnyPolicy {
            name: "fallback-deny",
            decision: PolicyDecision::Deny,
            reason: "must not run",
        });
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(
        &pack,
        &token,
        1,
        ExecutionPlane::Tool,
        PlaneTier::Core,
        None,
    );
    let action = LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]));

    let report = engine.decide(&ctx, &action).await;

    assert_eq!(report.evaluations.len(), 1);
    assert!(matches!(
        report.outcome,
        PolicyOutcome::RequireUserPermission { ref source, .. }
            if source.policy_name == "user-permission"
    ));
}

#[tokio::test]
async fn policy_engine_grants_after_parent_permission_and_retains_report() {
    let capabilities = Capabilities::from([Capability::InvokeTool]);
    let ctx = PermissionPolicyContext::new(
        &capabilities,
        Ok(PermissionResolution::Approved),
        Ok(PermissionResolution::Denied {
            reason: "user should not be asked".into(),
        }),
    );
    let engine =
        PolicyPipeline::<PermissionContextFactory>::new().with_pre_policy(StaticAnyPolicy {
            name: "parent-permission",
            decision: PolicyDecision::RequireParentPermission,
            reason: "parent must approve",
        });
    let action = LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]));

    let grant = engine
        .grant(&ctx, action)
        .await
        .expect("parent approval should grant the action");

    assert_eq!(
        *ctx.requests.lock().expect("permission request log"),
        vec!["parent"]
    );
    assert!(matches!(
        grant.info.report.outcome,
        PolicyOutcome::RequireParentPermission { .. }
    ));
}

#[tokio::test]
async fn policy_engine_parent_escalation_requests_user_permission() {
    let capabilities = Capabilities::from([Capability::InvokeTool]);
    let ctx = PermissionPolicyContext::new(
        &capabilities,
        Ok(PermissionResolution::Escalate),
        Ok(PermissionResolution::Approved),
    );
    let engine =
        PolicyPipeline::<PermissionContextFactory>::new().with_pre_policy(StaticAnyPolicy {
            name: "parent-permission",
            decision: PolicyDecision::RequireParentPermission,
            reason: "parent or user must approve",
        });
    let action = LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]));

    engine
        .grant(&ctx, action)
        .await
        .expect("user approval should grant after parent escalation");

    assert_eq!(
        *ctx.requests.lock().expect("permission request log"),
        vec!["parent", "user"]
    );
}

#[tokio::test]
async fn policy_engine_direct_user_permission_skips_parent() {
    let capabilities = Capabilities::from([Capability::InvokeTool]);
    let ctx = PermissionPolicyContext::new(
        &capabilities,
        Ok(PermissionResolution::Denied {
            reason: "parent should not be asked".into(),
        }),
        Ok(PermissionResolution::Approved),
    );
    let engine =
        PolicyPipeline::<PermissionContextFactory>::new().with_pre_policy(StaticAnyPolicy {
            name: "user-permission",
            decision: PolicyDecision::RequireUserPermission,
            reason: "user must approve",
        });
    let action = LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]));

    engine
        .grant(&ctx, action)
        .await
        .expect("user approval should grant the action");

    assert_eq!(
        *ctx.requests.lock().expect("permission request log"),
        vec!["user"]
    );
}

#[tokio::test]
async fn policy_engine_permission_denial_retains_policy_report() {
    let capabilities = Capabilities::from([Capability::InvokeTool]);
    let ctx = PermissionPolicyContext::new(
        &capabilities,
        Ok(PermissionResolution::Denied {
            reason: "parent refused".into(),
        }),
        Ok(PermissionResolution::Approved),
    );
    let engine =
        PolicyPipeline::<PermissionContextFactory>::new().with_pre_policy(StaticAnyPolicy {
            name: "parent-permission",
            decision: PolicyDecision::RequireParentPermission,
            reason: "parent must approve",
        });
    let action = LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]));

    let error = engine
        .grant(&ctx, action)
        .await
        .expect_err("parent denial should reject the action");

    assert!(matches!(
        error,
        PolicyGrantError::PermissionDenied { ref report, ref reason }
            if reason == "parent refused"
                && matches!(report.outcome, PolicyOutcome::RequireParentPermission { .. })
    ));
}

#[tokio::test]
async fn policy_engine_permission_request_failure_retains_policy_report() {
    let capabilities = Capabilities::from([Capability::InvokeTool]);
    let ctx = PermissionPolicyContext::new(
        &capabilities,
        Err(PermissionRequestError::Unavailable {
            reason: "parent channel is offline".into(),
        }),
        Ok(PermissionResolution::Approved),
    );
    let engine =
        PolicyPipeline::<PermissionContextFactory>::new().with_pre_policy(StaticAnyPolicy {
            name: "parent-permission",
            decision: PolicyDecision::RequireParentPermission,
            reason: "parent must approve",
        });
    let action = LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]));

    let error = engine
        .grant(&ctx, action)
        .await
        .expect_err("an unavailable permission surface must reject the action");

    assert!(matches!(
        error,
        PolicyGrantError::PermissionRequest {
            ref report,
            source: PermissionRequestError::Unavailable { ref reason },
        } if reason == "parent channel is offline"
            && matches!(report.outcome, PolicyOutcome::RequireParentPermission { .. })
    ));
}

#[tokio::test]
async fn policy_engine_user_permission_cannot_escalate() {
    let capabilities = Capabilities::from([Capability::InvokeTool]);
    let ctx = PermissionPolicyContext::new(
        &capabilities,
        Ok(PermissionResolution::Approved),
        Ok(PermissionResolution::Escalate),
    );
    let engine =
        PolicyPipeline::<PermissionContextFactory>::new().with_pre_policy(StaticAnyPolicy {
            name: "user-permission",
            decision: PolicyDecision::RequireUserPermission,
            reason: "user must approve",
        });
    let action = LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]));

    let error = engine
        .grant(&ctx, action)
        .await
        .expect_err("the root user authority has nowhere to escalate");

    assert!(matches!(
        error,
        PolicyGrantError::PermissionRequest {
            ref report,
            source: PermissionRequestError::EscalationUnavailable,
        } if matches!(report.outcome, PolicyOutcome::RequireUserPermission { .. })
    ));
}

#[tokio::test]
async fn policy_engine_default_parent_permission_hook_returns_unavailable() {
    let engine = PolicyPipeline::<TestContextFactory>::new().with_pre_policy(StaticAnyPolicy {
        name: "parent-permission",
        decision: PolicyDecision::RequireParentPermission,
        reason: "parent must approve",
    });
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(
        &pack,
        &token,
        1,
        ExecutionPlane::Tool,
        PlaneTier::Core,
        None,
    );
    let action = LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]));

    let error = engine
        .grant(&ctx, action)
        .await
        .expect_err("the default parent permission hook must fail closed");

    assert!(matches!(
        error,
        PolicyGrantError::PermissionRequest {
            ref report,
            source: PermissionRequestError::Unavailable { ref reason },
        } if reason == "parent permission interaction is unavailable"
            && matches!(report.outcome, PolicyOutcome::RequireParentPermission { .. })
    ));
}

#[tokio::test]
async fn policy_engine_default_user_permission_hook_returns_unavailable() {
    let engine = PolicyPipeline::<TestContextFactory>::new().with_pre_policy(StaticAnyPolicy {
        name: "user-permission",
        decision: PolicyDecision::RequireUserPermission,
        reason: "user must approve",
    });
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(
        &pack,
        &token,
        1,
        ExecutionPlane::Tool,
        PlaneTier::Core,
        None,
    );
    let action = LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]));

    let error = engine
        .grant(&ctx, action)
        .await
        .expect_err("the default user permission hook must fail closed");

    assert!(matches!(
        error,
        PolicyGrantError::PermissionRequest {
            ref report,
            source: PermissionRequestError::Unavailable { ref reason },
        } if reason == "user permission interaction is unavailable"
            && matches!(report.outcome, PolicyOutcome::RequireUserPermission { .. })
    ));
}

#[tokio::test]
async fn policy_engine_capability_gate_precedes_permission_request() {
    let capabilities = Capabilities::from([Capability::InvokeTool]);
    let ctx = PermissionPolicyContext::new(
        &capabilities,
        Ok(PermissionResolution::Approved),
        Ok(PermissionResolution::Approved),
    );
    let engine =
        PolicyPipeline::<PermissionContextFactory>::new().with_pre_policy(StaticAnyPolicy {
            name: "user-permission",
            decision: PolicyDecision::RequireUserPermission,
            reason: "user must approve",
        });
    let action = LegacyKernelAction::new("read", BTreeSet::from([Capability::FilesystemRead]));

    let error = engine
        .grant(&ctx, action)
        .await
        .expect_err("missing capability should reject before permission");

    assert!(matches!(
        error,
        PolicyGrantError::MissingCapability {
            capability: Capability::FilesystemRead
        }
    ));
    assert!(
        ctx.requests
            .lock()
            .expect("permission request log")
            .is_empty()
    );
}

struct ChangingAuthorityContextFactory;

impl ContextFactory for ChangingAuthorityContextFactory {
    type Cx<'a> = ChangingAuthorityContext<'a>;
}

// This context models authority changing while an external permission request
// is pending. Kernel must observe the change before returning the grant.
struct ChangingAuthorityContext<'a> {
    kernel: &'a Kernel<ChangingAuthorityContextFactory>,
    clock: &'a FixedClock,
    pack: &'a VerticalPackManifest,
    token: &'a CapabilityToken,
    change: AuthorityChange,
}

#[derive(Clone, Copy)]
enum AuthorityChange {
    RevokeToken,
    AdvanceClock(u64),
}

#[async_trait]
impl PolicyContext for ChangingAuthorityContext<'_> {
    fn allowed_capabilities(&self) -> Cow<'_, Capabilities> {
        Cow::Owned(self.token.allowed_capabilities.iter().copied().collect())
    }

    async fn request_user_permission(
        &self,
        _action: &dyn ActionMeta,
        _report: &PolicyReport,
    ) -> Result<PermissionResolution, PermissionRequestError> {
        match self.change {
            AuthorityChange::RevokeToken => {
                self.kernel
                    .revoke_token(&self.token.token_id, Some(&self.token.agent_id))
                    .map_err(|error| PermissionRequestError::Failed {
                        reason: error.to_string().into(),
                    })?;
            }
            AuthorityChange::AdvanceClock(delta_s) => self.clock.advance_by(delta_s),
        }
        Ok(PermissionResolution::Approved)
    }
}

impl KernelInvocationContext for ChangingAuthorityContext<'_> {
    fn pack(&self) -> &VerticalPackManifest {
        self.pack
    }

    fn token(&self) -> &CapabilityToken {
        self.token
    }

    fn now_epoch_s(&self) -> u64 {
        self.kernel.now_epoch_s()
    }

    fn request_parameters(&self) -> Option<&serde_json::Value> {
        None
    }
}

#[tokio::test]
async fn kernel_rechecks_token_after_permission_approval() {
    let policy =
        PolicyPipeline::<ChangingAuthorityContextFactory>::new().with_pre_policy(StaticAnyPolicy {
            name: "user-permission",
            decision: PolicyDecision::RequireUserPermission,
            reason: "user must approve",
        });
    let clock = Arc::new(FixedClock::new(1));
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut kernel = Kernel::with_policy_runtime(policy, clock.clone(), audit);
    let pack = pack();
    kernel
        .register_pack(pack.clone())
        .expect("test pack should register");
    let token = kernel
        .issue_token(&pack.pack_id, "agent", 120)
        .expect("test token should issue");
    let ctx = ChangingAuthorityContext {
        kernel: &kernel,
        clock: &clock,
        pack: &pack,
        token: &token,
        change: AuthorityChange::RevokeToken,
    };

    let error = kernel
        .grant_action(
            &pack.pack_id,
            &token,
            LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool])),
            &ctx,
        )
        .await
        .expect_err("revocation during permission must prevent the grant");

    assert!(matches!(
        error,
        KernelError::Policy(PolicyError::RevokedToken { ref token_id })
            if token_id == &token.token_id
    ));
}

#[tokio::test]
async fn legacy_kernel_authorization_rechecks_token_after_permission_approval() {
    let policy =
        PolicyPipeline::<ChangingAuthorityContextFactory>::new().with_pre_policy(StaticAnyPolicy {
            name: "user-permission",
            decision: PolicyDecision::RequireUserPermission,
            reason: "user must approve",
        });
    let clock = Arc::new(FixedClock::new(1));
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut kernel = Kernel::with_policy_runtime(policy, clock.clone(), audit);
    let pack = pack();
    kernel
        .register_pack(pack.clone())
        .expect("test pack should register");
    let token = kernel
        .issue_token(&pack.pack_id, "agent", 120)
        .expect("test token should issue");
    let ctx = ChangingAuthorityContext {
        kernel: &kernel,
        clock: &clock,
        pack: &pack,
        token: &token,
        change: AuthorityChange::RevokeToken,
    };

    let error = kernel
        .authorize_operation(
            &pack.pack_id,
            &token,
            ExecutionPlane::Tool,
            PlaneTier::Core,
            "test-adapter",
            None,
            "legacy-tool",
            &BTreeSet::from([Capability::InvokeTool]),
            &ctx,
        )
        .await
        .expect_err("legacy authorization must reject a token revoked during permission");

    assert!(matches!(
        error,
        KernelError::Policy(PolicyError::RevokedToken { ref token_id })
            if token_id == &token.token_id
    ));
}

#[tokio::test]
async fn legacy_kernel_authorization_rechecks_expiry_after_permission_approval() {
    let policy =
        PolicyPipeline::<ChangingAuthorityContextFactory>::new().with_pre_policy(StaticAnyPolicy {
            name: "user-permission",
            decision: PolicyDecision::RequireUserPermission,
            reason: "user must approve",
        });
    let clock = Arc::new(FixedClock::new(1));
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut kernel = Kernel::with_policy_runtime(policy, clock.clone(), audit);
    let pack = pack();
    kernel
        .register_pack(pack.clone())
        .expect("test pack should register");
    let token = kernel
        .issue_token(&pack.pack_id, "agent", 120)
        .expect("test token should issue");
    let ctx = ChangingAuthorityContext {
        kernel: &kernel,
        clock: &clock,
        pack: &pack,
        token: &token,
        change: AuthorityChange::AdvanceClock(121),
    };

    let error = kernel
        .authorize_operation(
            &pack.pack_id,
            &token,
            ExecutionPlane::Tool,
            PlaneTier::Core,
            "test-adapter",
            None,
            "legacy-tool",
            &BTreeSet::from([Capability::InvokeTool]),
            &ctx,
        )
        .await
        .expect_err("legacy authorization must reject a token expired during permission");

    assert!(matches!(
        error,
        KernelError::Policy(PolicyError::ExpiredToken { ref token_id, .. })
            if token_id == &token.token_id
    ));
}
