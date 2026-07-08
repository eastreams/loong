use std::{
    borrow::Cow,
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use loong_access::fs::{access::FsAccessContext, action::FsReadAction};
use loong_contracts::{
    Capability, CapabilityToken, ExecutionPlane, GrantId, PlaneTier, PolicyDecision, PolicyEntry,
    PolicyEvaluation, PolicyGrant, PolicyId, PolicyOutcome, PolicyReport, VerticalPackManifest,
};
use loong_core::{
    error::AuthorizationError,
    policy::action::Action,
    policy::{
        context::{ActionContext, PolicyContext},
        engine::PolicyEngine,
        policy::{Policy, PolicyAny},
    },
};

use crate::{
    errors::PolicyError,
    policy_ext::{PolicyExtension, PolicyExtensionChain, PolicyExtensionContext},
};

const DEFAULT_DENY_REASON: &str = "No matching policy.";

/// Unified policy/action context assembled by the kernel for one invocation.
///
/// Add global execution facts here when they are shared by tools, policies, and
/// access actions. Do not put action-owned data here, and do not make actions
/// carry runtime roots just because one policy needs them.
pub struct KernelPolicyContext<'a> {
    pub pack: &'a VerticalPackManifest,
    pub token: &'a CapabilityToken,
    pub now_epoch_s: u64,
    pub plane: ExecutionPlane,
    pub tier: PlaneTier,
    pub request_parameters: Option<&'a serde_json::Value>,
    pub fs_resolution_root: PathBuf,
    pub fs_allowed_roots: Vec<PathBuf>,
}

impl<'a> KernelPolicyContext<'a> {
    #[must_use]
    pub fn new(
        pack: &'a VerticalPackManifest,
        token: &'a CapabilityToken,
        now_epoch_s: u64,
        plane: ExecutionPlane,
        tier: PlaneTier,
        request_parameters: Option<&'a serde_json::Value>,
    ) -> Self {
        Self {
            pack,
            token,
            now_epoch_s,
            plane,
            tier,
            request_parameters,
            fs_resolution_root: default_fs_resolution_root(),
            fs_allowed_roots: Vec::new(),
        }
    }

    /// Add the filesystem view required by fs access and fs policies.
    ///
    /// The resolution root answers "where do relative paths start"; allowed
    /// roots answer "which absolute locations may this invocation touch".
    #[must_use]
    pub fn with_fs_root_view(
        mut self,
        fs_resolution_root: PathBuf,
        fs_allowed_roots: Vec<PathBuf>,
    ) -> Self {
        self.fs_resolution_root = fs_resolution_root;
        self.fs_allowed_roots = fs_allowed_roots;
        self
    }
}

impl PolicyContext for KernelPolicyContext<'_> {
    fn capabilities(&self) -> BTreeSet<Capability> {
        self.token.allowed_capabilities.clone()
    }
}

impl ActionContext for KernelPolicyContext<'_> {
    fn execution_plane(&self) -> ExecutionPlane {
        self.plane
    }

    fn plane_tier(&self) -> PlaneTier {
        self.tier
    }
}

impl FsAccessContext for KernelPolicyContext<'_> {
    fn fs_resolution_root(&self) -> &Path {
        self.fs_resolution_root.as_path()
    }

    fn fs_allowed_roots(&self) -> &[PathBuf] {
        self.fs_allowed_roots.as_slice()
    }
}

fn default_fs_resolution_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

#[derive(Debug)]
pub struct LegacyKernelAction {
    operation: String,
    required_capabilities: BTreeSet<Capability>,
}

impl LegacyKernelAction {
    pub fn new(operation: impl Into<String>, required_capabilities: BTreeSet<Capability>) -> Self {
        Self {
            operation: operation.into(),
            required_capabilities,
        }
    }
}

impl Action for LegacyKernelAction {
    fn kind(&self) -> &'static str {
        "action.legacy"
    }

    fn operation(&self) -> Cow<'static, str> {
        self.operation.clone().into()
    }

    fn required_capabilities(&self) -> BTreeSet<Capability> {
        self.required_capabilities.clone()
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
pub struct PolicyPipeline {
    pre_policies: Vec<RegisteredAnyPolicy>,
    typed_policies: anymap::Map<dyn anymap::any::Any + Send + Sync>,
    fallback_policies: Vec<RegisteredAnyPolicy>,
    policy_extensions: PolicyExtensionChain,
    next_policy_id: PolicyId,
    grant_seq: AtomicU64,
}

impl Default for PolicyPipeline {
    fn default() -> Self {
        Self::new().with_fallback_policy(AllowPolicy)
    }
}

impl PolicyPipeline {
    #[must_use]
    pub fn new() -> Self {
        Self {
            pre_policies: Vec::new(),
            typed_policies: anymap::Map::new(),
            fallback_policies: Vec::new(),
            policy_extensions: PolicyExtensionChain::new(),
            next_policy_id: 0,
            grant_seq: AtomicU64::new(0),
        }
    }

    /// Register a policy for exactly one action type.
    ///
    /// Use this when the policy needs typed action data, such as a canonical fs
    /// path. Policies registered here will not see other action types.
    #[must_use]
    pub fn with_policy<A, P>(mut self, policy: P) -> Self
    where
        A: Action + 'static,
        P: Policy<Self, A> + 'static,
    {
        self.push_policy::<A, P>(policy);
        self
    }

    /// Add a typed policy to an existing pipeline.
    pub fn push_policy<A, P>(&mut self, policy: P)
    where
        A: Action + 'static,
        P: Policy<Self, A> + 'static,
    {
        let id = self.allocate_policy_id();
        let entries = self
            .typed_policies
            .entry::<TypedPolicyEntries<A>>()
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
        P: PolicyAny<Self> + 'static,
    {
        self.push_pre_policy(policy);
        self
    }

    /// Add a broad gate before typed action policy.
    pub fn push_pre_policy<P>(&mut self, policy: P)
    where
        P: PolicyAny<Self> + 'static,
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
        P: PolicyAny<Self> + 'static,
    {
        self.push_fallback_policy(policy);
        self
    }

    /// Add broad policy after typed action policy.
    pub fn push_fallback_policy<P>(&mut self, policy: P)
    where
        P: PolicyAny<Self> + 'static,
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
    pub async fn authorize_kernel_action<A: Action>(
        &self,
        ctx: &KernelPolicyContext<'_>,
        action: A,
    ) -> Result<(), PolicyError> {
        let required_capabilities = action.required_capabilities();
        self.grant(ctx, action).await.map_err(policy_engine_error)?;

        self.policy_extensions.authorize(&PolicyExtensionContext {
            pack: ctx.pack,
            token: ctx.token,
            now_epoch_s: ctx.now_epoch_s,
            required_capabilities: &required_capabilities,
            request_parameters: ctx.request_parameters,
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

struct RegisteredAnyPolicy {
    id: PolicyId,
    // TODO(policy-registration-metadata): Carry registration metadata here,
    // such as registered_at, registration_order, and source. Keep this in sync
    // with typed entries so PolicyReport can explain how each policy entered
    // the pipeline, not only what it decided.
    policy: Arc<dyn PolicyAny<PolicyPipeline>>,
}

struct RegisteredPolicy<A: Action> {
    id: PolicyId,
    // TODO(policy-registration-metadata): Mirror RegisteredAnyPolicy metadata
    // when typed policy registration records registered_at/source data.
    policy: Arc<dyn Policy<PolicyPipeline, A>>,
}

struct TypedPolicyEntries<A: Action> {
    policies: Vec<RegisteredPolicy<A>>,
}

impl<A: Action> Default for TypedPolicyEntries<A> {
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
impl PolicyEngine for PolicyPipeline {
    type Cx<'a> = KernelPolicyContext<'a>;

    async fn decide<A: Action + 'static>(&self, ctx: &Self::Cx<'_>, action: &A) -> PolicyReport {
        let mut evaluations = Vec::new();

        for registered in &self.pre_policies {
            let grant = registered.policy.grant(ctx, action).await;
            let source = PolicyEntry {
                policy_name: registered.policy.name().into(),
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

        if let Some(entries) = self.typed_policies.get::<TypedPolicyEntries<A>>() {
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
                policy_name: registered.policy.name().into(),
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
impl<P: PolicyEngine> PolicyAny<P> for AllowPolicy {
    fn name(&self) -> &'static str {
        "allow"
    }

    async fn grant(&self, _ctx: &P::Cx<'_>, _action: &dyn Action) -> PolicyGrant {
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
impl Policy<PolicyPipeline, FsReadAction> for FsReadFilenameDenyPolicy {
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-read-filename-deny")
    }

    async fn grant(&self, _ctx: &KernelPolicyContext<'_>, action: &FsReadAction) -> PolicyGrant {
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
    use loong_core::PolicyGrantError;

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
    impl<P: PolicyEngine> PolicyAny<P> for StaticAnyPolicy {
        fn name(&self) -> &'static str {
            self.name
        }

        async fn grant(&self, _ctx: &P::Cx<'_>, _action: &dyn Action) -> PolicyGrant {
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
    impl<A> Policy<PolicyPipeline, A> for StaticTypedPolicy
    where
        A: Action + Send + Sync,
    {
        fn name(&self) -> Cow<'static, str> {
            Cow::Borrowed(self.name)
        }

        async fn grant(&self, _ctx: &KernelPolicyContext<'_>, _action: &A) -> PolicyGrant {
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
    impl<P: PolicyEngine> PolicyAny<P> for CountingAnyPolicy {
        fn name(&self) -> &'static str {
            "counting-any"
        }

        async fn grant(&self, _ctx: &P::Cx<'_>, _action: &dyn Action) -> PolicyGrant {
            self.calls.fetch_add(1, Ordering::Relaxed);
            PolicyGrant {
                decision: PolicyDecision::Allow,
                predicate: None,
                reason: Cow::Borrowed("counted"),
            }
        }
    }

    struct TypedOnlyAction;

    impl Action for TypedOnlyAction {
        fn kind(&self) -> &'static str {
            "test.typed_only"
        }

        fn operation(&self) -> Cow<'static, str> {
            Cow::Borrowed("typed_only")
        }

        fn required_capabilities(&self) -> BTreeSet<Capability> {
            BTreeSet::from([Capability::InvokeTool])
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
        let engine = PolicyPipeline::new().with_fallback_policy(AllowPolicy);
        let pack = pack();
        let token = token();
        let required_capabilities = BTreeSet::from([Capability::InvokeTool]);
        let ctx = KernelPolicyContext::new(
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
        assert_eq!(grant.granted.into_action().operation(), "tool");
    }

    #[tokio::test]
    async fn policy_pipeline_runs_registered_policy_extensions() {
        let mut engine = PolicyPipeline::default();
        engine.register_policy_extension(DenyNetworkExtension);
        let pack = pack();
        let mut token = token();
        token.allowed_capabilities.insert(Capability::NetworkEgress);
        let required_capabilities = BTreeSet::from([Capability::NetworkEgress]);
        let ctx = KernelPolicyContext::new(
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
        let engine = PolicyPipeline::default();
        let pack = pack();
        let token = token();
        let ctx = KernelPolicyContext::new(
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
        let engine = PolicyPipeline::new()
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
        let ctx = KernelPolicyContext::new(
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
        let engine = PolicyPipeline::new().with_policy::<TypedOnlyAction, _>(StaticTypedPolicy {
            name: "typed-only",
            decision: PolicyDecision::Allow,
            reason: "typed only allowed",
        });
        let pack = pack();
        let token = token();
        let ctx = KernelPolicyContext::new(
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
        let engine = PolicyPipeline::new()
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
        let ctx = KernelPolicyContext::new(
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
        let engine = PolicyPipeline::new()
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
        let ctx = KernelPolicyContext::new(
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
        let engine = PolicyPipeline::new()
            .with_policy::<LegacyKernelAction, _>(StaticTypedPolicy {
                name: "typed-deny",
                decision: PolicyDecision::Deny,
                reason: "typed denied",
            })
            .with_fallback_policy(AllowPolicy);
        let pack = pack();
        let token = token();
        let ctx = KernelPolicyContext::new(
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
        let engine = PolicyPipeline::new()
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
        let ctx = KernelPolicyContext::new(
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
        let engine = PolicyPipeline::new()
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
        let ctx = KernelPolicyContext::new(
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
        let engine = PolicyPipeline::new().with_fallback_policy(StaticAnyPolicy {
            name: "fallback-advance",
            decision: PolicyDecision::Advance,
            reason: "no next chain",
        });
        let pack = pack();
        let token = token();
        let ctx = KernelPolicyContext::new(
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
        let engine = PolicyPipeline::new().with_pre_policy(StaticAnyPolicy {
            name: "pre-continue",
            decision: PolicyDecision::Continue,
            reason: "no opinion",
        });
        let pack = pack();
        let token = token();
        let ctx = KernelPolicyContext::new(
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
        let engine = PolicyPipeline::new().with_pre_policy(CountingAnyPolicy {
            calls: calls.clone(),
        });
        let pack = pack();
        let token = token();
        let ctx = KernelPolicyContext::new(
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
