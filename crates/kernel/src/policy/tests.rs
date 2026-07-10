use super::*;
use loong_contracts::{ExecutionPlane, PlaneTier};
use loong_core::PolicyGrantError;

struct TestContextFactory;

impl ContextFactory for TestContextFactory {
    type Cx<'a> = TestPolicyContext<'a>;
}

struct TestPolicyContext<'a> {
    pack: &'a VerticalPackManifest,
    token: &'a CapabilityToken,
    now_epoch_s: u64,
    request_parameters: Option<&'a serde_json::Value>,
}

impl<'a> TestPolicyContext<'a> {
    fn new(
        pack: &'a VerticalPackManifest,
        token: &'a CapabilityToken,
        now_epoch_s: u64,
        _plane: ExecutionPlane,
        _tier: PlaneTier,
        request_parameters: Option<&'a serde_json::Value>,
    ) -> Self {
        Self {
            pack,
            token,
            now_epoch_s,
            request_parameters,
        }
    }
}

impl PolicyContext for TestPolicyContext<'_> {
    fn capabilities(&self) -> BTreeSet<Capability> {
        self.token.allowed_capabilities.clone()
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

    fn request_parameters(&self) -> Option<&serde_json::Value> {
        self.request_parameters
    }
}

struct DenyNetworkExtension;

impl PolicyExtension for DenyNetworkExtension {
    fn name(&self) -> &str {
        "deny-network"
    }

    fn authorize_extension(&self, context: &PolicyExtensionContext<'_>) -> Result<(), PolicyError> {
        if context
            .required_capabilities
            .contains(&Capability::NetworkEgress)
        {
            return Err(PolicyError::ExtensionDenied {
                extension: self.name().to_owned(),
                reason: "network egress denied by test extension".to_owned(),
            });
        }
        Ok(())
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
    C: ContextFactory,
    A: ActionMeta,
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
async fn policy_pipeline_grants_actions_allowed_by_registered_policy() {
    let engine = PolicyPipeline::<TestContextFactory>::new().with_fallback_policy(AllowPolicy);
    let pack = pack();
    let token = token();
    let required_capabilities = BTreeSet::from([Capability::InvokeTool]);
    let ctx = TestPolicyContext::new(
        &pack,
        &token,
        1,
        ExecutionPlane::Tool,
        PlaneTier::Core,
        None,
    );
    let action = LegacyKernelAction::new("tool", required_capabilities);

    let grant = engine
        .grant(&ctx, action)
        .await
        .expect("allow policy should grant action");

    assert_eq!(grant.id.0, 1);
    let _info = grant.info;
    assert_eq!(grant.granted.into_action().metadata().operation, "tool");
}

#[tokio::test]
async fn policy_pipeline_runs_registered_policy_extensions() {
    let mut engine = PolicyPipeline::<TestContextFactory>::default();
    engine.register_policy_extension(DenyNetworkExtension);
    let pack = pack();
    let mut token = token();
    token.allowed_capabilities.insert(Capability::NetworkEgress);
    let required_capabilities = BTreeSet::from([Capability::NetworkEgress]);
    let ctx = TestPolicyContext::new(
        &pack,
        &token,
        1,
        ExecutionPlane::Runtime,
        PlaneTier::Core,
        None,
    );
    let action = LegacyKernelAction::new("fetch", required_capabilities);

    let error = engine
        .authorize_kernel_action(&ctx, action)
        .await
        .expect_err("registered policy extension should deny the action");

    assert_eq!(
        error,
        PolicyError::ExtensionDenied {
            extension: "deny-network".to_owned(),
            reason: "network egress denied by test extension".to_owned(),
        }
    );
}

#[tokio::test]
async fn policy_pipeline_grant_denies_action_missing_required_capability() {
    let engine = PolicyPipeline::<TestContextFactory>::default();
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
    let action = LegacyKernelAction::new("read", BTreeSet::from([Capability::FilesystemRead]));

    let error = engine
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
    let engine = PolicyPipeline::<TestContextFactory>::new()
        .with_pre_policy(StaticAnyPolicy {
            name: "pre-continue",
            decision: PolicyDecision::Continue,
            reason: "no opinion",
        })
        .with_policy::<LegacyKernelAction, _>(StaticTypedPolicy {
            name: "typed-allow",
            decision: PolicyDecision::Allow,
            reason: "typed allowed",
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

    assert_eq!(report.evaluations.len(), 2);
    assert_eq!(report.evaluations[0].policy_stage, "pre");
    assert_eq!(report.evaluations[1].policy_stage, "action");
    assert!(matches!(
        report.outcome,
        PolicyOutcome::Allow { ref source, .. } if source.policy_name == "typed-allow"
    ));
}

#[tokio::test]
async fn policy_pipeline_typed_policy_only_matches_registered_action_type() {
    let engine = PolicyPipeline::<TestContextFactory>::new().with_policy::<TypedOnlyAction, _>(
        StaticTypedPolicy {
            name: "typed-only",
            decision: PolicyDecision::Allow,
            reason: "typed only allowed",
        },
    );
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
    let legacy_action = LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]));

    let legacy_report = engine.decide(&ctx, &legacy_action).await;
    let typed_report = engine.decide(&ctx, &TypedOnlyAction).await;

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
    let engine = PolicyPipeline::<TestContextFactory>::new()
        .with_pre_policy(StaticAnyPolicy {
            name: "pre-deny",
            decision: PolicyDecision::Deny,
            reason: "pre denied",
        })
        .with_policy::<LegacyKernelAction, _>(StaticTypedPolicy {
            name: "typed-allow",
            decision: PolicyDecision::Allow,
            reason: "typed allowed",
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
    let engine = PolicyPipeline::<TestContextFactory>::new()
        .with_pre_policy(StaticAnyPolicy {
            name: "pre-allow",
            decision: PolicyDecision::Allow,
            reason: "pre allowed",
        })
        .with_policy::<LegacyKernelAction, _>(StaticTypedPolicy {
            name: "typed-deny",
            decision: PolicyDecision::Deny,
            reason: "typed denied",
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
    assert_eq!(report.evaluations[0].policy_stage, "pre");
    assert!(matches!(
        report.outcome,
        PolicyOutcome::Allow { ref source, .. } if source.policy_name == "pre-allow"
    ));
}

#[tokio::test]
async fn policy_pipeline_typed_deny_prevents_fallback_allow() {
    let engine = PolicyPipeline::<TestContextFactory>::new()
        .with_policy::<LegacyKernelAction, _>(StaticTypedPolicy {
            name: "typed-deny",
            decision: PolicyDecision::Deny,
            reason: "typed denied",
        })
        .with_fallback_policy(AllowPolicy);
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
    let engine = PolicyPipeline::<TestContextFactory>::new()
        .with_policy::<LegacyKernelAction, _>(StaticTypedPolicy {
            name: "typed-allow",
            decision: PolicyDecision::Allow,
            reason: "typed allowed",
        })
        .with_fallback_policy(StaticAnyPolicy {
            name: "fallback-deny",
            decision: PolicyDecision::Deny,
            reason: "fallback denied",
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
    assert_eq!(report.evaluations[0].policy_stage, "action");
    assert!(matches!(
        report.outcome,
        PolicyOutcome::Allow { ref source, .. } if source.policy_name == "typed-allow"
    ));
}

#[tokio::test]
async fn policy_pipeline_advance_skips_rest_of_current_subchain() {
    let engine = PolicyPipeline::<TestContextFactory>::new()
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

    assert_eq!(report.evaluations.len(), 2);
    assert_eq!(report.evaluations[0].source.policy_name, "pre-advance");
    assert_eq!(report.evaluations[1].source.policy_name, "typed-allow");
    assert!(matches!(report.outcome, PolicyOutcome::Allow { .. }));
}

#[tokio::test]
async fn policy_pipeline_fallback_advance_defaults_to_deny() {
    let engine =
        PolicyPipeline::<TestContextFactory>::new().with_fallback_policy(StaticAnyPolicy {
            name: "fallback-advance",
            decision: PolicyDecision::Advance,
            reason: "no next chain",
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
    let engine = PolicyPipeline::<TestContextFactory>::new().with_pre_policy(StaticAnyPolicy {
        name: "pre-continue",
        decision: PolicyDecision::Continue,
        reason: "no opinion",
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
        PolicyOutcome::Deny {
            grant_source: None,
            ref reason,
        } if reason == DEFAULT_DENY_REASON
    ));
}

#[tokio::test]
async fn policy_pipeline_missing_required_capability_denies_before_policy_execution() {
    let calls = Arc::new(AtomicU64::new(0));
    let engine = PolicyPipeline::<TestContextFactory>::new().with_pre_policy(CountingAnyPolicy {
        calls: calls.clone(),
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
    let action = LegacyKernelAction::new("read", BTreeSet::from([Capability::FilesystemRead]));

    let error = engine
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
