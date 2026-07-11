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
use loong_access::fs::{FsPathPolicyContext, FsReadAction, FsResolvePathAction};
use loong_contracts::{
    Capability, CapabilityToken, GrantId, PolicyDecision, PolicyEntry, PolicyEvaluation,
    PolicyGrant, PolicyId, PolicyOutcome, PolicyReport, VerticalPackManifest,
};
use loong_core::{
    error::AuthorizationError,
    policy::action::{ActionMeta, ActionMetadata},
    policy::{
        context::{CapabilityContext, ContextFactory},
        engine::PolicyEngine,
        policy::{Policy, PolicyAny},
    },
    tool::ToolInvocationAction,
};

use crate::{
    errors::PolicyError,
    policy_ext::{PolicyExtension, PolicyExtensionChain, PolicyExtensionContext},
};

const DEFAULT_DENY_REASON: &str = "No matching policy.";

pub trait KernelInvocationContext: CapabilityContext {
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

    fn payload(&self) -> Cow<'_, serde_json::Value> {
        Cow::Owned(serde_json::json!({
            "operation": self.operation.as_str(),
        }))
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

impl<C: ContextFactory> PolicyPipeline<C> {
    /// Construct a default-deny pipeline.
    ///
    /// Without a terminal allow policy, unmatched actions produce a deny report.
    /// Runtime bootstraps that still need legacy compatibility must opt into
    /// `new_legacy_allow_fallback`.
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

    #[must_use]
    pub fn new_legacy_allow_fallback() -> Self {
        Self::new().with_fallback_policy(LegacyAllowPolicy)
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

    pub fn push_fs_read_allow_policy(&mut self) {
        self.push_policy::<FsReadAction, _>(FsReadAllowPolicy);
    }

    #[must_use]
    pub fn with_fs_path_policy(mut self) -> Self
    where
        for<'a> C::Cx<'a>: FsPathPolicyContext,
    {
        self.push_fs_path_policy();
        self
    }

    pub fn push_fs_path_policy(&mut self)
    where
        for<'a> C::Cx<'a>: FsPathPolicyContext,
    {
        self.push_policy::<FsResolvePathAction, _>(FsResolvePathAllowedRootsPolicy);
    }

    pub fn push_tool_invocation_allow_policy(&mut self) {
        self.push_policy::<ToolInvocationAction, _>(ToolInvocationAllowPolicy);
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
pub(crate) fn policy_engine_error(error: impl Into<AuthorizationError>) -> PolicyError {
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
                policy_stage: Cow::Borrowed("pre"),
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
                    policy_stage: Cow::Borrowed("action"),
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
                policy_stage: Cow::Borrowed("fallback"),
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

#[derive(Debug, Default, Clone, Copy)]
pub struct LegacyAllowPolicy;

#[async_trait]
impl<C> PolicyAny<C> for LegacyAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("legacy-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, action: &dyn ActionMeta) -> PolicyGrant {
        if action.metadata().kind == "action.legacy" {
            return PolicyGrant {
                decision: PolicyDecision::Allow,
                predicate: Some("action kind is legacy kernel operation".into()),
                reason: "legacy kernel operation allowed by migration fallback".into(),
            };
        }

        PolicyGrant {
            decision: PolicyDecision::Continue,
            predicate: Some("action kind is not legacy kernel operation".into()),
            reason: "legacy fallback does not grant typed actions".into(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct ToolInvocationAllowPolicy;

#[async_trait]
impl<C> Policy<C, ToolInvocationAction> for ToolInvocationAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("tool-invocation-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &ToolInvocationAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("tool invocation passed capability gate".into()),
            reason: "tool invocation allowed by app policy".into(),
        }
    }
}

#[derive(Debug, Clone)]
struct FsReadFilenameDenyPolicy {
    denied_filenames: BTreeSet<String>,
}

#[derive(Debug, Default, Clone, Copy)]
struct FsReadAllowPolicy;

#[derive(Debug, Default, Clone, Copy)]
struct FsResolvePathAllowedRootsPolicy;

/// Default fs path containment policy.
///
/// `loong-access` prepares resolved path facts, but containment is a policy
/// decision owned by the kernel pipeline so denials produce `PolicyReport`
/// evidence instead of domain action errors.
#[async_trait]
impl<C> Policy<C, FsResolvePathAction> for FsResolvePathAllowedRootsPolicy
where
    C: ContextFactory + Send + Sync,
    for<'a> C::Cx<'a>: FsPathPolicyContext,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-resolve-path-allowed-roots")
    }

    async fn grant(&self, ctx: &C::Cx<'_>, action: &FsResolvePathAction) -> PolicyGrant {
        let allowed_roots = ctx.fs_allowed_roots();
        if resolved_path_starts_with_allowed_root(action.resolved_path(), allowed_roots) {
            return PolicyGrant {
                decision: PolicyDecision::Allow,
                predicate: Some("resolved fs path starts with an allowed root".into()),
                reason: "resolved fs path is within allowed roots".into(),
            };
        }

        PolicyGrant {
            decision: PolicyDecision::Deny,
            predicate: Some("resolved fs path must start with an allowed root".into()),
            reason: format!(
                "filesystem path {} escapes allowed filesystem roots [{}]",
                action.resolved_path().display(),
                display_path_list(allowed_roots)
            )
            .into(),
        }
    }
}

fn resolved_path_starts_with_allowed_root(
    resolved_path: &std::path::Path,
    allowed_roots: &[std::path::PathBuf],
) -> bool {
    allowed_roots
        .iter()
        .any(|allowed_root| resolved_path.starts_with(allowed_root))
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

fn normalize_policy_filename(filename: &str) -> Option<String> {
    let normalized = filename.trim().to_ascii_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

fn display_path_list(paths: &[std::path::PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests;
