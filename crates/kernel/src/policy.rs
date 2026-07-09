use std::{
    borrow::Cow,
    collections::BTreeSet,
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use loong_access::fs::action::FsReadAction;
use loong_contracts::{
    Capability, CapabilityToken, GrantId, PolicyDecision, PolicyEntry, PolicyEvaluation,
    PolicyGrant, PolicyId, PolicyOutcome, PolicyReport, VerticalPackManifest,
};
use loong_core::{
    error::AuthorizationError,
    policy::action::{ActionMeta, ActionMetadata},
    policy::{
        context::{ContextFactory, PolicyContext},
        engine::PolicyEngine,
        policy::{Policy, PolicyAny},
    },
};

use crate::{
    errors::PolicyError,
    policy_ext::{PolicyExtension, PolicyExtensionChain, PolicyExtensionContext},
};

const DEFAULT_DENY_REASON: &str = "No matching policy.";

pub trait KernelInvocationContext: PolicyContext {
    fn pack(&self) -> &VerticalPackManifest;

    fn token(&self) -> &CapabilityToken;

    fn now_epoch_s(&self) -> u64;

    fn request_parameters(&self) -> Option<&serde_json::Value>;
}

#[derive(Debug)]
pub struct LegacyKernelAction {
    operation: String,
    required_capabilities: Vec<Capability>,
}

impl LegacyKernelAction {
    pub fn new(operation: impl Into<String>, required_capabilities: BTreeSet<Capability>) -> Self {
        Self {
            operation: operation.into(),
            required_capabilities: required_capabilities.into_iter().collect(),
        }
    }
}

impl ActionMeta for LegacyKernelAction {
    fn metadata(&self) -> ActionMetadata<'_> {
        ActionMetadata {
            kind: "action.legacy",
            operation: Cow::Borrowed(self.operation.as_str()),
            required_capabilities: Cow::Borrowed(self.required_capabilities.as_slice()),
        }
    }

    fn payload(&self) -> serde_json::Value {
        serde_json::json!({
            "operation": self.operation.as_str(),
        })
    }
}

/// Kernel policy engine.
///
/// Evaluation order is fixed across three subchains:
///
/// 1. `pre`: broad [`PolicyAny`] gates that run before action-specific policy.
/// 2. `action`: policies registered for the concrete action type.
/// 3. `fallback`: broad [`PolicyAny`] policy used after typed policy.
///
/// [`PolicyDecision::Allow`] and [`PolicyDecision::Deny`] stop the whole
/// pipeline. [`PolicyDecision::Continue`] evaluates the next policy in the
/// current subchain. [`PolicyDecision::Advance`] skips the rest of the current
/// subchain and moves to the next one. If no terminal decision is produced,
/// the pipeline returns default deny. The returned [`PolicyReport`] records the
/// evaluated policy chain.
pub struct PolicyPipeline<C: ContextFactory> {
    pre_policies: Vec<RegisteredAnyPolicy<C>>,
    typed_policies: anymap::Map<dyn anymap::any::Any + Send + Sync>,
    fallback_policies: Vec<RegisteredAnyPolicy<C>>,
    policy_extensions: PolicyExtensionChain,
    next_policy_id: PolicyId,
    grant_seq: AtomicU64,
    _context: PhantomData<fn() -> C>,
}

impl<C: ContextFactory> Default for PolicyPipeline<C> {
    fn default() -> Self {
        Self::new().with_fallback_policy(AllowPolicy)
    }
}

impl<C: ContextFactory> PolicyPipeline<C> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            pre_policies: Vec::new(),
            typed_policies: anymap::Map::new(),
            fallback_policies: Vec::new(),
            policy_extensions: PolicyExtensionChain::new(),
            next_policy_id: 0,
            grant_seq: AtomicU64::new(0),
            _context: PhantomData,
        }
    }

    /// Register a policy for exactly one action type.
    ///
    /// Use this when the policy needs typed action data, such as a canonical fs
    /// path. Policies registered here will not see other action types.
    #[must_use]
    pub fn with_policy<A, P>(mut self, policy: P) -> Self
    where
        A: ActionMeta + 'static,
        P: Policy<C, A> + 'static,
    {
        self.push_policy::<A, P>(policy);
        self
    }

    /// Add a typed policy to an existing pipeline.
    pub fn push_policy<A, P>(&mut self, policy: P)
    where
        A: ActionMeta + 'static,
        P: Policy<C, A> + 'static,
    {
        let id = self.allocate_policy_id();
        let entries = self
            .typed_policies
            .entry::<TypedPolicyEntries<C, A>>()
            .or_insert_with(TypedPolicyEntries::default);
        entries.policies.push(RegisteredPolicy {
            id,
            policy: Arc::new(policy),
        });
    }

    pub fn push_fs_read_filename_deny_policy(&mut self, denied_filenames: BTreeSet<String>) {
        if denied_filenames.is_empty() {
            return;
        }

        self.push_policy::<FsReadAction, _>(FsReadFilenameDenyPolicy::new(denied_filenames));
    }

    /// Register a broad gate before typed action policy.
    ///
    /// Use this for policy that should be able to stop an action before typed
    /// policy runs. Keep action-specific checks in `with_policy` so unrelated
    /// actions do not share unnecessary context requirements.
    #[must_use]
    pub fn with_pre_policy<P>(mut self, policy: P) -> Self
    where
        P: PolicyAny<C> + 'static,
    {
        self.push_pre_policy(policy);
        self
    }

    /// Add a broad gate before typed action policy.
    pub fn push_pre_policy<P>(&mut self, policy: P)
    where
        P: PolicyAny<C> + 'static,
    {
        let id = self.allocate_policy_id();
        self.pre_policies.push(RegisteredAnyPolicy {
            id,
            policy: Arc::new(policy),
        });
    }

    /// Register broad policy after typed action policy.
    ///
    /// The default allow policy belongs here: typed policies must get a chance
    /// to deny before the compatibility fallback grants legacy actions.
    #[must_use]
    pub fn with_fallback_policy<P>(mut self, policy: P) -> Self
    where
        P: PolicyAny<C> + 'static,
    {
        self.push_fallback_policy(policy);
        self
    }

    /// Add broad policy after typed action policy.
    pub fn push_fallback_policy<P>(&mut self, policy: P)
    where
        P: PolicyAny<C> + 'static,
    {
        let id = self.allocate_policy_id();
        self.fallback_policies.push(RegisteredAnyPolicy {
            id,
            policy: Arc::new(policy),
        });
    }

    pub fn register_policy_extension<E: PolicyExtension + 'static>(&mut self, extension: E) {
        self.policy_extensions.register(extension);
    }

    /// Authorize legacy kernel operations that still use policy extensions.
    ///
    /// New access-backed side effects should prefer `PolicyEngine::grant` on a
    /// typed action and consume the resulting grant inside the access module.
    pub async fn authorize_kernel_action<A: ActionMeta>(
        &self,
        ctx: &C::Cx<'_>,
        action: A,
    ) -> Result<(), PolicyError>
    where
        for<'a> C::Cx<'a>: KernelInvocationContext,
    {
        let required_capabilities = action
            .metadata()
            .required_capabilities
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        self.grant(ctx, action).await.map_err(policy_engine_error)?;

        self.policy_extensions.authorize(&PolicyExtensionContext {
            pack: ctx.pack(),
            token: ctx.token(),
            now_epoch_s: ctx.now_epoch_s(),
            required_capabilities: &required_capabilities,
            request_parameters: ctx.request_parameters(),
        })
    }

    fn allocate_policy_id(&mut self) -> PolicyId {
        let id = self.next_policy_id;
        self.next_policy_id = self.next_policy_id.saturating_add(1);
        id
    }

    fn next_grant_id_sync(&self) -> GrantId {
        let seq = self.grant_seq.fetch_add(1, Ordering::Relaxed) + 1;
        GrantId(seq)
    }
}

struct RegisteredAnyPolicy<C: ContextFactory> {
    id: PolicyId,
    // TODO(policy-registration-metadata): Carry registration metadata here,
    // such as registered_at, registration_order, and source. Keep this in sync
    // with typed entries so PolicyReport can explain how each policy entered
    // the pipeline, not only what it decided.
    policy: Arc<dyn PolicyAny<C>>,
}

struct RegisteredPolicy<C: ContextFactory, A: ActionMeta> {
    id: PolicyId,
    // TODO(policy-registration-metadata): Mirror RegisteredAnyPolicy metadata
    // when typed policy registration records registered_at/source data.
    policy: Arc<dyn Policy<C, A>>,
}

struct TypedPolicyEntries<C: ContextFactory, A: ActionMeta> {
    policies: Vec<RegisteredPolicy<C, A>>,
}

impl<C: ContextFactory, A: ActionMeta> Default for TypedPolicyEntries<C, A> {
    fn default() -> Self {
        Self {
            policies: Vec::new(),
        }
    }
}

// Legacy bridge for `authorize_kernel_action`, whose caller still expects the
// old extension-oriented `PolicyError` surface. New access-backed side effects
// should keep typed grant errors and convert them at the caller boundary.
fn policy_engine_error(error: impl Into<AuthorizationError>) -> PolicyError {
    let error = error.into();
    PolicyError::ExtensionDenied {
        extension: "policy-engine".to_owned(),
        reason: error.to_string(),
    }
}

#[async_trait]
impl<C> PolicyEngine<C> for PolicyPipeline<C>
where
    C: ContextFactory + Send + Sync,
{
    async fn decide<A: ActionMeta>(&self, ctx: &C::Cx<'_>, action: &A) -> PolicyReport {
        let mut evaluations = Vec::new();

        for registered in &self.pre_policies {
            let grant = registered.policy.grant(ctx, action).await;
            let source = PolicyEntry {
                policy_name: registered.policy.name(),
                policy_id: registered.id,
            };
            let decision = grant.decision;
            let reason = grant.reason.clone();
            let outcome_source = source.clone();
            evaluations.push(PolicyEvaluation {
                source,
                policy_stage: "pre",
                grant,
            });

            match decision {
                PolicyDecision::Allow => {
                    let outcome = PolicyOutcome::Allow {
                        source: outcome_source,
                        reason,
                    };
                    return PolicyReport {
                        evaluations,
                        outcome,
                    };
                }
                PolicyDecision::Deny => {
                    let outcome = PolicyOutcome::Deny {
                        grant_source: Some(outcome_source),
                        reason,
                    };
                    return PolicyReport {
                        evaluations,
                        outcome,
                    };
                }
                PolicyDecision::Continue => {}
                PolicyDecision::Advance => break,
            }
        }

        if let Some(entries) = self.typed_policies.get::<TypedPolicyEntries<C, A>>() {
            for registered in &entries.policies {
                let grant = registered.policy.grant(ctx, action).await;
                let source = PolicyEntry {
                    policy_name: registered.policy.name(),
                    policy_id: registered.id,
                };
                let decision = grant.decision;
                let reason = grant.reason.clone();
                let outcome_source = source.clone();
                evaluations.push(PolicyEvaluation {
                    source,
                    policy_stage: "action",
                    grant,
                });

                match decision {
                    PolicyDecision::Allow => {
                        let outcome = PolicyOutcome::Allow {
                            source: outcome_source,
                            reason,
                        };
                        return PolicyReport {
                            evaluations,
                            outcome,
                        };
                    }
                    PolicyDecision::Deny => {
                        let outcome = PolicyOutcome::Deny {
                            grant_source: Some(outcome_source),
                            reason,
                        };
                        return PolicyReport {
                            evaluations,
                            outcome,
                        };
                    }
                    PolicyDecision::Continue => {}
                    PolicyDecision::Advance => break,
                }
            }
        }

        for registered in &self.fallback_policies {
            let grant = registered.policy.grant(ctx, action).await;
            let source = PolicyEntry {
                policy_name: registered.policy.name(),
                policy_id: registered.id,
            };
            let decision = grant.decision;
            let reason = grant.reason.clone();
            let outcome_source = source.clone();
            evaluations.push(PolicyEvaluation {
                source,
                policy_stage: "fallback",
                grant,
            });

            match decision {
                PolicyDecision::Allow => {
                    let outcome = PolicyOutcome::Allow {
                        source: outcome_source,
                        reason,
                    };
                    return PolicyReport {
                        evaluations,
                        outcome,
                    };
                }
                PolicyDecision::Deny => {
                    let outcome = PolicyOutcome::Deny {
                        grant_source: Some(outcome_source),
                        reason,
                    };
                    return PolicyReport {
                        evaluations,
                        outcome,
                    };
                }
                PolicyDecision::Continue => {}
                PolicyDecision::Advance => break,
            }
        }

        let outcome = PolicyOutcome::Deny {
            grant_source: None,
            reason: DEFAULT_DENY_REASON.into(),
        };

        PolicyReport {
            evaluations,
            outcome,
        }
    }

    async fn next_grant_id(&self) -> GrantId {
        self.next_grant_id_sync()
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AllowPolicy;

#[async_trait]
impl<C> PolicyAny<C> for AllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &dyn ActionMeta) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: None,
            reason: "allowed by allow policy".into(),
        }
    }
}

#[derive(Debug, Clone)]
struct FsReadFilenameDenyPolicy {
    denied_filenames: BTreeSet<String>,
}

impl FsReadFilenameDenyPolicy {
    fn new(denied_filenames: BTreeSet<String>) -> Self {
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

fn normalize_policy_filename(filename: &str) -> Option<String> {
    let normalized = filename.trim().to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

#[cfg(test)]
mod tests {
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

        fn authorize_extension(
            &self,
            context: &PolicyExtensionContext<'_>,
        ) -> Result<(), PolicyError> {
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

        fn payload(&self) -> serde_json::Value {
            serde_json::json!({})
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
        let legacy_action =
            LegacyKernelAction::new("tool", BTreeSet::from([Capability::InvokeTool]));

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
        let engine =
            PolicyPipeline::<TestContextFactory>::new().with_pre_policy(CountingAnyPolicy {
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
}
