use super::*;

use std::{
    borrow::Cow,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use loong_contracts::{AuthorizationScope, AuthorizationSubject, Capabilities};
use loong_core::{PolicyGrantError, kernel::Kernel as _, policy::engine::PolicyEngine};
use serde_json::json;

mod audit;
mod permission;

/// Grant tests install policy through the production Kernel boundary; pure
/// evaluation tests inspect the registry directly and need no audit fixture.
fn kernel_with_policy<C>(policy: PolicyPipelineBuilder<C>) -> crate::Kernel<C>
where
    C: ContextFactory,
{
    crate::Kernel::with_policy_runtime(
        policy,
        Arc::new(crate::FixedClock::new(1)),
        Arc::new(crate::InMemoryAuditSink::default()),
    )
}

struct TestContextFactory;

impl ContextFactory for TestContextFactory {
    type Cx<'a> = TestPolicyContext<'a>;
}

struct TestPolicyContext<'a> {
    pack: &'a VerticalPackManifest,
    token: &'a CapabilityToken,
    now_epoch_s: u64,
}

impl<'a> TestPolicyContext<'a> {
    fn new(pack: &'a VerticalPackManifest, token: &'a CapabilityToken, now_epoch_s: u64) -> Self {
        Self {
            pack,
            token,
            now_epoch_s,
        }
    }
}

impl PolicyContext for TestPolicyContext<'_> {
    fn allowed_capabilities(&self) -> Cow<'_, Capabilities> {
        Cow::Owned(self.token.allowed_capabilities.iter().copied().collect())
    }

    fn authorization_subject(&self) -> AuthorizationSubject {
        AuthorizationSubject {
            actor_id: self.token.agent_id.clone(),
            scope: AuthorizationScope::LegacyToken {
                boundary: "kernel.policy.test".to_owned(),
                pack_id: self.token.pack_id.clone(),
                token_id: self.token.token_id.clone(),
            },
        }
    }
}

impl KernelInvocationContext for TestPolicyContext<'_> {
    fn pack(&self) -> &VerticalPackManifest {
        self.pack
    }

    fn token(&self) -> &CapabilityToken {
        self.token
    }

    fn now_epoch_s(&self) -> u64 {
        self.now_epoch_s
    }
}

#[derive(Clone)]
struct StaticAnyPolicy {
    name: &'static str,
    decision: PolicyDecision,
    reason: &'static str,
}

#[async_trait]
impl<C> PolicyAny<C> for StaticAnyPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed(self.name)
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &dyn ActionMeta) -> PolicyGrant {
        PolicyGrant {
            decision: self.decision,
            predicate: None,
            reason: Cow::Borrowed(self.reason),
        }
    }
}

#[derive(Clone)]
struct StaticTypedPolicy {
    name: &'static str,
    decision: PolicyDecision,
    reason: &'static str,
}

#[async_trait]
impl<C, A> Policy<C, A> for StaticTypedPolicy
where
    C: ContextFactory + Send + Sync,
    A: ActionMeta + Sync + 'static,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed(self.name)
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &A) -> PolicyGrant {
        PolicyGrant {
            decision: self.decision,
            predicate: None,
            reason: Cow::Borrowed(self.reason),
        }
    }
}

struct CountingAnyPolicy {
    calls: Arc<AtomicU64>,
}

#[async_trait]
impl<C> PolicyAny<C> for CountingAnyPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("counting-any")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &dyn ActionMeta) -> PolicyGrant {
        self.calls.fetch_add(1, Ordering::Relaxed);
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: None,
            reason: Cow::Borrowed("counted"),
        }
    }
}

struct TypedOnlyAction;

impl ActionMeta for TypedOnlyAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "test.typed_only",
            operation: Cow::Borrowed("typed_only"),
            required_capabilities: Cow::Borrowed(&[Capability::InvokeTool]),
        }
    }

    fn payload(&self) -> Cow<'_, serde_json::Value> {
        Cow::Owned(serde_json::json!({}))
    }
}

struct TypedLegacyKindAction;

impl ActionMeta for TypedLegacyKindAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "action.legacy",
            operation: Cow::Borrowed("typed_spoof"),
            required_capabilities: Cow::Borrowed(&[Capability::InvokeTool]),
        }
    }

    fn payload(&self) -> Cow<'_, serde_json::Value> {
        Cow::Owned(serde_json::json!({}))
    }
}

fn pack() -> VerticalPackManifest {
    VerticalPackManifest {
        pack_id: "pack".to_owned(),
        domain: "test".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: loong_contracts::ExecutionRoute {
            harness_kind: loong_contracts::HarnessKind::EmbeddedPi,
            adapter: None,
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([Capability::InvokeTool]),
        metadata: Default::default(),
    }
}

fn token() -> CapabilityToken {
    CapabilityToken {
        token_id: "tok".to_owned(),
        pack_id: "pack".to_owned(),
        agent_id: "agent".to_owned(),
        allowed_capabilities: BTreeSet::from([Capability::InvokeTool]),
        issued_at_epoch_s: 1,
        expires_at_epoch_s: 10,
        generation: 1,
    }
}

#[tokio::test]
async fn policy_pipeline_new_has_no_fallback_allow() {
    let registry = PolicyPipelineBuilder::<TestContextFactory>::new().registry;
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(&pack, &token, 1);
    let action =
        LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]), json!({}));

    let report = registry.decide(&ctx, &action).await;

    assert!(report.evaluations.is_empty());
    assert!(matches!(
        report.outcome,
        PolicyOutcome::Deny {
            grant_source: None,
            ref reason,
        } if reason == DEFAULT_DENY_REASON
    ));
}

#[tokio::test]
async fn policy_pipeline_new_legacy_allow_fallback_grants_legacy_action() {
    let registry =
        PolicyPipelineBuilder::<TestContextFactory>::new_legacy_allow_fallback().registry;
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(&pack, &token, 1);
    let action =
        LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]), json!({}));

    let report = registry.decide(&ctx, &action).await;

    assert_eq!(report.evaluations.len(), 1);
    assert_eq!(report.evaluations[0].policy_stage, "action");
    assert!(matches!(
        report.outcome,
        PolicyOutcome::Allow { ref source, .. } if source.policy_name == "legacy-allow"
    ));
}

#[tokio::test]
async fn policy_pipeline_new_legacy_allow_fallback_does_not_grant_typed_actions() {
    let registry =
        PolicyPipelineBuilder::<TestContextFactory>::new_legacy_allow_fallback().registry;
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(&pack, &token, 1);

    let report = registry.decide(&ctx, &TypedOnlyAction).await;

    assert!(report.evaluations.is_empty());
    assert!(matches!(
        report.outcome,
        PolicyOutcome::Deny {
            grant_source: None,
            ..
        }
    ));
}

#[tokio::test]
async fn policy_pipeline_legacy_allow_cannot_be_spoofed_by_action_kind() {
    let registry =
        PolicyPipelineBuilder::<TestContextFactory>::new_legacy_allow_fallback().registry;
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(&pack, &token, 1);

    let report = registry.decide(&ctx, &TypedLegacyKindAction).await;

    assert!(report.evaluations.is_empty());
    assert!(matches!(
        report.outcome,
        PolicyOutcome::Deny {
            grant_source: None,
            ..
        }
    ));
}

#[tokio::test]
async fn policy_pipeline_grants_actions_allowed_by_registered_policy() {
    let kernel = kernel_with_policy(
        PolicyPipelineBuilder::<TestContextFactory>::new().with_fallback_policy(AllowPolicy),
    );
    let pack = pack();
    let token = token();
    let required_capabilities = BTreeSet::from([Capability::InvokeTool]);
    let ctx = TestPolicyContext::new(&pack, &token, 1);
    let action = LegacyKernelAction::new("tool", required_capabilities, json!({}));

    let grant = kernel
        .policy_engine()
        .grant(&ctx, action)
        .await
        .expect("allow policy should grant action");

    assert_eq!(grant.id.0, 1);
    assert_eq!(grant.info.report.evaluations.len(), 1);
    assert!(matches!(
        grant.info.report.outcome,
        PolicyOutcome::Allow { ref source, .. } if source.policy_name == "allow"
    ));
    assert_eq!(grant.granted.into_action().metadata().operation, "tool");
}

#[tokio::test]
async fn policy_pipeline_pre_policy_can_block_legacy_actions() {
    let mut pipeline = PolicyPipelineBuilder::<TestContextFactory>::new_legacy_allow_fallback();
    pipeline.push_pre_policy(StaticAnyPolicy {
        name: "deny-network",
        decision: PolicyDecision::Deny,
        reason: "network egress denied by test policy",
    });
    let kernel = kernel_with_policy(pipeline);
    let pack = pack();
    let mut token = token();
    token.allowed_capabilities.insert(Capability::NetworkEgress);
    let required_capabilities = BTreeSet::from([Capability::NetworkEgress]);
    let ctx = TestPolicyContext::new(&pack, &token, 1);
    let action = LegacyKernelAction::new("fetch", required_capabilities, json!({}));

    let error = kernel
        .policy_engine()
        .grant(&ctx, action)
        .await
        .map_err(policy_engine_error)
        .expect_err("pre policy should deny the action");

    assert!(matches!(
        error,
        PolicyError::ExtensionDenied {
            ref extension,
            ref reason,
        } if extension == "policy-engine"
            && reason.contains("network egress denied by test policy")
    ));
}

#[tokio::test]
async fn policy_pipeline_grant_denies_action_missing_required_capability() {
    let kernel = kernel_with_policy(
        PolicyPipelineBuilder::<TestContextFactory>::new_legacy_allow_fallback(),
    );
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(&pack, &token, 1);
    let action = LegacyKernelAction::new(
        "read",
        BTreeSet::from([Capability::FilesystemRead]),
        json!({}),
    );

    let error = kernel
        .policy_engine()
        .grant(&ctx, action)
        .await
        .expect_err("typed action grant should require token capabilities");

    assert!(matches!(
        error,
        PolicyGrantError::MissingCapability {
            capability: Capability::FilesystemRead
        }
    ));
}

#[tokio::test]
async fn policy_pipeline_report_preserves_pre_and_action_evaluation_stages() {
    let registry = PolicyPipelineBuilder::<TestContextFactory>::new()
        .with_pre_policy(StaticAnyPolicy {
            name: "pre-continue",
            decision: PolicyDecision::Continue,
            reason: "no opinion",
        })
        .with_policy::<LegacyKernelAction, _>(StaticTypedPolicy {
            name: "typed-allow",
            decision: PolicyDecision::Allow,
            reason: "typed allowed",
        })
        .registry;
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(&pack, &token, 1);
    let action =
        LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]), json!({}));

    let report = registry.decide(&ctx, &action).await;

    assert_eq!(report.evaluations.len(), 2);
    assert_eq!(report.evaluations[0].policy_stage, "pre");
    assert_eq!(report.evaluations[1].policy_stage, "action");
    assert!(matches!(
        report.outcome,
        PolicyOutcome::Allow { ref source, .. } if source.policy_name == "typed-allow"
    ));
}

#[tokio::test]
async fn policy_pipeline_report_preserves_registration_metadata() {
    let registry = PolicyPipelineBuilder::<TestContextFactory>::new()
        .with_pre_policy(StaticAnyPolicy {
            name: "pre-continue",
            decision: PolicyDecision::Continue,
            reason: "no opinion",
        })
        .with_policy::<LegacyKernelAction, _>(StaticTypedPolicy {
            name: "typed-allow",
            decision: PolicyDecision::Allow,
            reason: "typed allowed",
        })
        .registry;
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(&pack, &token, 1);
    let action =
        LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]), json!({}));

    let report = registry.decide(&ctx, &action).await;
    let pre_registration = &report.evaluations[0].source.registration;
    let typed_registration = &report.evaluations[1].source.registration;

    assert_eq!(pre_registration.order, 0);
    assert_eq!(typed_registration.order, 1);
    assert!(pre_registration.registered_at_unix_ms > 0);
    assert!(typed_registration.registered_at_unix_ms > 0);
    assert!(
        pre_registration
            .source
            .file
            .ends_with("crates/kernel/src/policy/tests.rs")
    );
    assert!(pre_registration.source.line > 0);
    assert!(pre_registration.source.column > 0);
    assert!(
        typed_registration
            .source
            .file
            .ends_with("crates/kernel/src/policy/tests.rs")
    );
    assert!(typed_registration.source.line > pre_registration.source.line);
    assert!(typed_registration.source.column > 0);
}

#[tokio::test]
async fn policy_pipeline_typed_policy_only_matches_registered_action_type() {
    let registry = PolicyPipelineBuilder::<TestContextFactory>::new()
        .with_policy::<TypedOnlyAction, _>(StaticTypedPolicy {
            name: "typed-only",
            decision: PolicyDecision::Allow,
            reason: "typed only allowed",
        })
        .registry;
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(&pack, &token, 1);
    let legacy_action =
        LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]), json!({}));

    let legacy_report = registry.decide(&ctx, &legacy_action).await;
    let typed_report = registry.decide(&ctx, &TypedOnlyAction).await;

    assert!(legacy_report.evaluations.is_empty());
    assert!(matches!(
        legacy_report.outcome,
        PolicyOutcome::Deny {
            grant_source: None,
            ..
        }
    ));
    assert_eq!(typed_report.evaluations.len(), 1);
    assert_eq!(typed_report.evaluations[0].policy_stage, "action");
}

#[tokio::test]
async fn policy_pipeline_pre_deny_prevents_typed_allow() {
    let kernel = kernel_with_policy(
        PolicyPipelineBuilder::<TestContextFactory>::new()
            .with_pre_policy(StaticAnyPolicy {
                name: "pre-deny",
                decision: PolicyDecision::Deny,
                reason: "pre denied",
            })
            .with_policy::<LegacyKernelAction, _>(StaticTypedPolicy {
                name: "typed-allow",
                decision: PolicyDecision::Allow,
                reason: "typed allowed",
            }),
    );
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(&pack, &token, 1);
    let action =
        LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]), json!({}));

    let error = kernel
        .policy_engine()
        .grant(&ctx, action)
        .await
        .expect_err("pre deny should stop before typed allow");

    assert!(matches!(
        error,
        PolicyGrantError::Denied { ref report, .. }
            if report.evaluations.len() == 1
                && report.evaluations[0].policy_stage == "pre"
                && matches!(report.outcome, PolicyOutcome::Deny { .. })
    ));
}

#[tokio::test]
async fn policy_pipeline_allow_short_circuits_before_later_typed_deny() {
    let registry = PolicyPipelineBuilder::<TestContextFactory>::new()
        .with_pre_policy(StaticAnyPolicy {
            name: "pre-allow",
            decision: PolicyDecision::Allow,
            reason: "pre allowed",
        })
        .with_policy::<LegacyKernelAction, _>(StaticTypedPolicy {
            name: "typed-deny",
            decision: PolicyDecision::Deny,
            reason: "typed denied",
        })
        .registry;
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(&pack, &token, 1);
    let action =
        LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]), json!({}));

    let report = registry.decide(&ctx, &action).await;

    assert_eq!(report.evaluations.len(), 1);
    assert_eq!(report.evaluations[0].policy_stage, "pre");
    assert!(matches!(
        report.outcome,
        PolicyOutcome::Allow { ref source, .. } if source.policy_name == "pre-allow"
    ));
}

#[tokio::test]
async fn policy_pipeline_typed_deny_prevents_fallback_allow() {
    let registry = PolicyPipelineBuilder::<TestContextFactory>::new()
        .with_policy::<LegacyKernelAction, _>(StaticTypedPolicy {
            name: "typed-deny",
            decision: PolicyDecision::Deny,
            reason: "typed denied",
        })
        .with_fallback_policy(AllowPolicy)
        .registry;
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(&pack, &token, 1);
    let action =
        LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]), json!({}));

    let report = registry.decide(&ctx, &action).await;

    assert_eq!(report.evaluations.len(), 1);
    assert_eq!(report.evaluations[0].policy_stage, "action");
    assert!(matches!(
        report.outcome,
        PolicyOutcome::Deny {
            grant_source: Some(ref source),
            ..
        } if source.policy_name == "typed-deny"
    ));
}

#[tokio::test]
async fn policy_pipeline_typed_allow_prevents_fallback_deny() {
    let registry = PolicyPipelineBuilder::<TestContextFactory>::new()
        .with_policy::<LegacyKernelAction, _>(StaticTypedPolicy {
            name: "typed-allow",
            decision: PolicyDecision::Allow,
            reason: "typed allowed",
        })
        .with_fallback_policy(StaticAnyPolicy {
            name: "fallback-deny",
            decision: PolicyDecision::Deny,
            reason: "fallback denied",
        })
        .registry;
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(&pack, &token, 1);
    let action =
        LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]), json!({}));

    let report = registry.decide(&ctx, &action).await;

    assert_eq!(report.evaluations.len(), 1);
    assert_eq!(report.evaluations[0].policy_stage, "action");
    assert!(matches!(
        report.outcome,
        PolicyOutcome::Allow { ref source, .. } if source.policy_name == "typed-allow"
    ));
}

#[tokio::test]
async fn policy_pipeline_advance_skips_rest_of_current_subchain() {
    let registry = PolicyPipelineBuilder::<TestContextFactory>::new()
        .with_pre_policy(StaticAnyPolicy {
            name: "pre-advance",
            decision: PolicyDecision::Advance,
            reason: "advance to typed policy",
        })
        .with_pre_policy(StaticAnyPolicy {
            name: "pre-deny",
            decision: PolicyDecision::Deny,
            reason: "should be skipped",
        })
        .with_policy::<LegacyKernelAction, _>(StaticTypedPolicy {
            name: "typed-allow",
            decision: PolicyDecision::Allow,
            reason: "typed allowed",
        })
        .registry;
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(&pack, &token, 1);
    let action =
        LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]), json!({}));

    let report = registry.decide(&ctx, &action).await;

    assert_eq!(report.evaluations.len(), 2);
    assert_eq!(report.evaluations[0].source.policy_name, "pre-advance");
    assert_eq!(report.evaluations[1].source.policy_name, "typed-allow");
    assert!(matches!(report.outcome, PolicyOutcome::Allow { .. }));
}

#[tokio::test]
async fn policy_pipeline_fallback_advance_defaults_to_deny() {
    let registry = PolicyPipelineBuilder::<TestContextFactory>::new()
        .with_fallback_policy(StaticAnyPolicy {
            name: "fallback-advance",
            decision: PolicyDecision::Advance,
            reason: "no next chain",
        })
        .registry;
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(&pack, &token, 1);
    let action =
        LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]), json!({}));

    let report = registry.decide(&ctx, &action).await;

    assert_eq!(report.evaluations.len(), 1);
    assert_eq!(report.evaluations[0].policy_stage, "fallback");
    assert!(matches!(
        report.outcome,
        PolicyOutcome::Deny {
            grant_source: None,
            ref reason,
        } if reason == DEFAULT_DENY_REASON
    ));
}

#[tokio::test]
async fn policy_pipeline_all_continue_defaults_to_deny() {
    let registry = PolicyPipelineBuilder::<TestContextFactory>::new()
        .with_pre_policy(StaticAnyPolicy {
            name: "pre-continue",
            decision: PolicyDecision::Continue,
            reason: "no opinion",
        })
        .registry;
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(&pack, &token, 1);
    let action =
        LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]), json!({}));

    let report = registry.decide(&ctx, &action).await;

    assert_eq!(report.evaluations.len(), 1);
    assert!(matches!(
        report.outcome,
        PolicyOutcome::Deny {
            grant_source: None,
            ref reason,
        } if reason == DEFAULT_DENY_REASON
    ));
}

#[tokio::test]
async fn policy_pipeline_missing_required_capability_denies_before_policy_execution() {
    let calls = Arc::new(AtomicU64::new(0));
    let kernel = kernel_with_policy(
        PolicyPipelineBuilder::<TestContextFactory>::new().with_pre_policy(CountingAnyPolicy {
            calls: calls.clone(),
        }),
    );
    let pack = pack();
    let token = token();
    let ctx = TestPolicyContext::new(&pack, &token, 1);
    let action = LegacyKernelAction::new(
        "read",
        BTreeSet::from([Capability::FilesystemRead]),
        json!({}),
    );

    let error = kernel
        .policy_engine()
        .grant(&ctx, action)
        .await
        .expect_err("capability gate should run before policies");

    assert!(matches!(
        error,
        PolicyGrantError::MissingCapability {
            capability: Capability::FilesystemRead
        }
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}
