use std::{
    borrow::Cow,
    collections::BTreeSet,
    marker::PhantomData,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::access::fs::{
    FsContentSearchAction, FsGlobAction, FsInspectPathAction, FsReadDirAction,
    FsRemoveDirAllAction, FsRenameAction,
};
use crate::{
    audit::SharedAuditState,
    errors::{AuditError, PolicyError},
};
use async_trait::async_trait;
use loong_contracts::{
    AuthorizationAttemptId, AuthorizationEvidence, Capability, GrantId, PolicyDecision,
    PolicyEntry, PolicyEvaluation, PolicyGrant, PolicyId, PolicyOutcome, PolicyRegistration,
    PolicyRegistrationSource, PolicyReport,
};
use loong_core::{
    error::AuthorizationError,
    policy::action::{ActionMeta, ActionMetadata},
    policy::{
        context::ContextFactory,
        engine::PolicyEngineBackend,
        policy::{Policy, PolicyAny},
    },
};

const DEFAULT_DENY_REASON: &str = "No matching policy.";

/// Compatibility action for legacy kernel envelopes.
///
/// `operation` remains routing metadata while `payload` preserves the original request body.
#[derive(Debug)]
pub struct LegacyKernelAction {
    operation: String,
    required_capabilities: Vec<Capability>,
    payload: serde_json::Value,
}

impl LegacyKernelAction {
    pub fn new(
        operation: impl Into<String>,
        required_capabilities: BTreeSet<Capability>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            operation: operation.into(),
            required_capabilities: required_capabilities.into_iter().collect(),
            payload,
        }
    }

    /// Consume the granted legacy action and recover its sole owned request body.
    pub fn into_payload(self) -> serde_json::Value {
        self.payload
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
        Cow::Borrowed(&self.payload)
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
/// [`PolicyDecision::Allow`], [`PolicyDecision::Deny`], and both permission
/// decisions stop the whole pipeline. [`PolicyDecision::Continue`] evaluates
/// the next policy in the current subchain. [`PolicyDecision::Advance`] skips
/// the rest of the current subchain and moves to the next one. If no terminal
/// decision is produced, the pipeline returns default deny. The returned
/// [`PolicyReport`] records the evaluated policy chain.
pub(crate) struct PolicyPipeline<C: ContextFactory> {
    registry: PolicyRegistry<C>,
    audit_state: Arc<SharedAuditState>,
}

/// Registration-only policy pipeline input.
///
/// This type deliberately does not implement
/// [`loong_core::policy::engine::PolicyEngine`]: only Kernel can install it
/// with the shared audit owner and produce a runnable [`PolicyPipeline`].
pub struct PolicyPipelineBuilder<C: ContextFactory> {
    registry: PolicyRegistry<C>,
}

struct PolicyRegistry<C: ContextFactory> {
    pre_policies: Vec<RegisteredAnyPolicy<C>>,
    typed_policies: anymap::Map<dyn anymap::any::Any + Send + Sync>,
    fallback_policies: Vec<RegisteredAnyPolicy<C>>,
    next_policy_order: u64,
    _context: PhantomData<fn() -> C>,
}

impl<C: ContextFactory> PolicyPipelineBuilder<C> {
    /// Construct a default-deny pipeline.
    ///
    /// Without a terminal allow policy, unmatched actions produce a deny report.
    /// Runtime bootstraps that still need legacy compatibility must opt into
    /// `new_legacy_allow_fallback`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            registry: PolicyRegistry {
                pre_policies: Vec::new(),
                typed_policies: anymap::Map::new(),
                fallback_policies: Vec::new(),
                next_policy_order: 0,
                _context: PhantomData,
            },
        }
    }

    #[must_use]
    pub fn new_legacy_allow_fallback() -> Self {
        Self::new().with_policy(LegacyAllowPolicy)
    }

    /// Register a policy for exactly one action type.
    ///
    /// Use this when the policy needs typed action data, such as a canonical fs
    /// path. Policies registered here will not see other action types.
    #[must_use]
    #[track_caller]
    pub fn with_policy<A, P>(mut self, policy: P) -> Self
    where
        A: ActionMeta + 'static,
        P: Policy<C, A> + 'static,
    {
        self.push_policy::<A, P>(policy);
        self
    }

    /// Add a typed policy to an existing pipeline.
    #[track_caller]
    pub fn push_policy<A, P>(&mut self, policy: P)
    where
        A: ActionMeta + 'static,
        P: Policy<C, A> + 'static,
    {
        let (id, registration) = self.allocate_registration();
        let entries = self
            .registry
            .typed_policies
            .entry::<TypedPolicyEntries<C, A>>()
            .or_insert_with(TypedPolicyEntries::default);
        entries.policies.push(RegisteredPolicy {
            id,
            registration,
            policy: Arc::new(policy),
        });
    }

    /// Register a broad gate before typed action policy.
    ///
    /// Use this for policy that should be able to stop an action before typed
    /// policy runs. Keep action-specific checks in `with_policy` so unrelated
    /// actions do not share unnecessary context requirements.
    #[must_use]
    #[track_caller]
    pub fn with_pre_policy<P>(mut self, policy: P) -> Self
    where
        P: PolicyAny<C> + 'static,
    {
        self.push_pre_policy(policy);
        self
    }

    /// Add a broad gate before typed action policy.
    #[track_caller]
    pub fn push_pre_policy<P>(&mut self, policy: P)
    where
        P: PolicyAny<C> + 'static,
    {
        let (id, registration) = self.allocate_registration();
        self.registry.pre_policies.push(RegisteredAnyPolicy {
            id,
            registration,
            policy: Arc::new(policy),
        });
    }

    /// Register broad policy after typed action policy.
    ///
    /// The default allow policy belongs here: typed policies must get a chance
    /// to deny before the compatibility fallback grants legacy actions.
    #[must_use]
    #[track_caller]
    pub fn with_fallback_policy<P>(mut self, policy: P) -> Self
    where
        P: PolicyAny<C> + 'static,
    {
        self.push_fallback_policy(policy);
        self
    }

    /// Add broad policy after typed action policy.
    #[track_caller]
    pub fn push_fallback_policy<P>(&mut self, policy: P)
    where
        P: PolicyAny<C> + 'static,
    {
        let (id, registration) = self.allocate_registration();
        self.registry.fallback_policies.push(RegisteredAnyPolicy {
            id,
            registration,
            policy: Arc::new(policy),
        });
    }

    #[track_caller]
    fn allocate_registration(&mut self) -> (PolicyId, PolicyRegistration) {
        let order = self.registry.next_policy_order;
        self.registry.next_policy_order = self.registry.next_policy_order.saturating_add(1);
        let id = PolicyId::new(order);
        let caller = std::panic::Location::caller();
        let registered_at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| {
                u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
            });
        let registration = PolicyRegistration {
            order,
            registered_at_unix_ms,
            source: PolicyRegistrationSource {
                file: caller.file().to_owned(),
                line: caller.line(),
                column: caller.column(),
            },
        };
        (id, registration)
    }
}

impl<C: ContextFactory> PolicyPipeline<C> {
    pub(crate) fn install(
        builder: PolicyPipelineBuilder<C>,
        audit_state: Arc<SharedAuditState>,
    ) -> Self {
        Self {
            registry: builder.registry,
            audit_state,
        }
    }
}

struct RegisteredAnyPolicy<C: ContextFactory> {
    id: PolicyId,
    registration: PolicyRegistration,
    policy: Arc<dyn PolicyAny<C>>,
}

struct RegisteredPolicy<C: ContextFactory, A: ActionMeta> {
    id: PolicyId,
    registration: PolicyRegistration,
    policy: Arc<dyn Policy<C, A>>,
}

struct TypedPolicyEntries<C: ContextFactory, A: ActionMeta> {
    policies: Vec<RegisteredPolicy<C, A>>,
}

enum PolicyStageControl {
    Continue,
    Advance,
    Terminal(PolicyOutcome),
}

/// Accumulates one ordered report while policy stages advance or terminate.
///
/// Keeping this transition state explicit ensures broad and typed stages cannot
/// drift in how they convert a decision into report evidence.
#[derive(Default)]
struct PolicyEvaluationTrace {
    evaluations: Vec<PolicyEvaluation>,
}

impl PolicyEvaluationTrace {
    fn record(
        &mut self,
        stage: &'static str,
        policy_name: Cow<'static, str>,
        policy_id: PolicyId,
        registration: &PolicyRegistration,
        grant: PolicyGrant,
    ) -> PolicyStageControl {
        let source = PolicyEntry {
            policy_name,
            policy_id,
            registration: registration.clone(),
        };
        let outcome_source = source.clone();
        let decision = grant.decision;
        let reason = grant.reason.clone();
        self.evaluations.push(PolicyEvaluation {
            source,
            policy_stage: Cow::Borrowed(stage),
            grant,
        });

        match decision {
            PolicyDecision::Allow => PolicyStageControl::Terminal(PolicyOutcome::Allow {
                source: outcome_source,
                reason,
            }),
            PolicyDecision::Deny => PolicyStageControl::Terminal(PolicyOutcome::Deny {
                grant_source: Some(outcome_source),
                reason,
            }),
            PolicyDecision::RequireParentPermission => {
                PolicyStageControl::Terminal(PolicyOutcome::RequireParentPermission {
                    source: outcome_source,
                    reason,
                })
            }
            PolicyDecision::RequireUserPermission => {
                PolicyStageControl::Terminal(PolicyOutcome::RequireUserPermission {
                    source: outcome_source,
                    reason,
                })
            }
            PolicyDecision::Continue => PolicyStageControl::Continue,
            PolicyDecision::Advance => PolicyStageControl::Advance,
        }
    }

    fn finish(self, outcome: PolicyOutcome) -> PolicyReport {
        PolicyReport {
            evaluations: self.evaluations,
            outcome,
        }
    }

    fn deny_without_match(self) -> PolicyReport {
        self.finish(PolicyOutcome::Deny {
            grant_source: None,
            reason: DEFAULT_DENY_REASON.into(),
        })
    }
}

impl<C: ContextFactory, A: ActionMeta> Default for TypedPolicyEntries<C, A> {
    fn default() -> Self {
        Self {
            policies: Vec::new(),
        }
    }
}

impl<C, A> TypedPolicyEntries<C, A>
where
    C: ContextFactory,
    A: ActionMeta,
{
    async fn evaluate(
        &self,
        ctx: &C::Cx<'_>,
        action: &A,
        trace: &mut PolicyEvaluationTrace,
    ) -> Option<PolicyOutcome> {
        for registered in &self.policies {
            let grant = registered.policy.grant(ctx, action).await;
            match trace.record(
                "action",
                registered.policy.name(),
                registered.id,
                &registered.registration,
                grant,
            ) {
                PolicyStageControl::Continue => {}
                PolicyStageControl::Advance => return None,
                PolicyStageControl::Terminal(outcome) => return Some(outcome),
            }
        }
        None
    }
}

impl<C> PolicyRegistry<C>
where
    C: ContextFactory,
{
    async fn decide<A: ActionMeta + 'static>(&self, ctx: &C::Cx<'_>, action: &A) -> PolicyReport {
        let mut trace = PolicyEvaluationTrace::default();

        if let Some(outcome) = self
            .evaluate_any_stage("pre", &self.pre_policies, ctx, action, &mut trace)
            .await
        {
            return trace.finish(outcome);
        }
        if let Some(entries) = self.typed_policies.get::<TypedPolicyEntries<C, A>>()
            && let Some(outcome) = entries.evaluate(ctx, action, &mut trace).await
        {
            return trace.finish(outcome);
        }
        if let Some(outcome) = self
            .evaluate_any_stage("fallback", &self.fallback_policies, ctx, action, &mut trace)
            .await
        {
            return trace.finish(outcome);
        }

        trace.deny_without_match()
    }

    /// Pre and fallback contain the same broad policy shape; only their stage
    /// identity and registration slice differ.
    async fn evaluate_any_stage<A: ActionMeta>(
        &self,
        stage: &'static str,
        policies: &[RegisteredAnyPolicy<C>],
        ctx: &C::Cx<'_>,
        action: &A,
        trace: &mut PolicyEvaluationTrace,
    ) -> Option<PolicyOutcome> {
        for registered in policies {
            let grant = registered.policy.grant(ctx, action).await;
            match trace.record(
                stage,
                registered.policy.name(),
                registered.id,
                &registered.registration,
                grant,
            ) {
                PolicyStageControl::Continue => {}
                PolicyStageControl::Advance => return None,
                PolicyStageControl::Terminal(outcome) => return Some(outcome),
            }
        }
        None
    }
}

// TODO(deprecate-legacy-policy-error): add `#[deprecated]` after callers stop
// expecting the old extension-oriented `PolicyError` surface. New access-backed
// side effects should keep typed grant errors at their owning boundary.
pub(crate) fn policy_engine_error(error: impl Into<AuthorizationError>) -> PolicyError {
    let error = error.into();
    PolicyError::ExtensionDenied {
        extension: "policy-engine".to_owned(),
        reason: error.to_string(),
    }
}

#[async_trait]
impl<C> PolicyEngineBackend<C> for PolicyPipeline<C>
where
    C: ContextFactory + Send + Sync,
{
    type AuditError = AuditError;

    async fn decide<A: ActionMeta + 'static>(&self, ctx: &C::Cx<'_>, action: &A) -> PolicyReport {
        self.registry.decide(ctx, action).await
    }

    fn reserve_authorization_attempt_id(&self) -> Result<AuthorizationAttemptId, Self::AuditError> {
        self.audit_state.reserve_authorization_attempt_id()
    }

    fn reserve_grant_id(&self) -> Result<GrantId, Self::AuditError> {
        self.audit_state.reserve_grant_id()
    }

    fn write_authorization_evidence(
        &self,
        evidence: &AuthorizationEvidence,
    ) -> Result<(), Self::AuditError> {
        self.audit_state.record_authorization(evidence)
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
struct LegacyAllowPolicy;

// Bind migration fallback to the concrete envelope type. Action metadata is
// caller-defined policy input and must never be treated as proof of legacy origin.
#[async_trait]
impl<C> Policy<C, LegacyKernelAction> for LegacyAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("legacy-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &LegacyKernelAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("action has concrete LegacyKernelAction type".into()),
            reason: "legacy kernel operation allowed by migration fallback".into(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FsRemoveDirAllAllowPolicy;

#[derive(Debug, Default, Clone, Copy)]
pub struct FsRenameAllowPolicy;

#[derive(Debug, Default, Clone, Copy)]
pub struct FsInspectPathAllowPolicy;

#[derive(Debug, Default, Clone, Copy)]
pub struct FsGlobAllowPolicy;

#[derive(Debug, Default, Clone, Copy)]
pub struct FsReadDirAllowPolicy;

#[derive(Debug, Default, Clone, Copy)]
pub struct FsContentSearchAllowPolicy;

#[async_trait]
impl<C> Policy<C, FsRemoveDirAllAction> for FsRemoveDirAllAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-remove-dir-all-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &FsRemoveDirAllAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("fs.remove_dir_all reached terminal allow policy".into()),
            reason: "filesystem directory removal allowed after configured deny policies".into(),
        }
    }
}

#[async_trait]
impl<C> Policy<C, FsRenameAction> for FsRenameAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-rename-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &FsRenameAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("fs.rename reached terminal allow policy".into()),
            reason: "filesystem rename allowed after configured deny policies".into(),
        }
    }
}

#[async_trait]
impl<C> Policy<C, FsInspectPathAction> for FsInspectPathAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-inspect-path-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &FsInspectPathAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("fs.inspect_path reached terminal allow policy".into()),
            reason: "filesystem path inspection allowed after configured deny policies".into(),
        }
    }
}

#[async_trait]
impl<C> Policy<C, FsGlobAction> for FsGlobAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-glob-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &FsGlobAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("fs.glob reached terminal allow policy".into()),
            reason: "filesystem glob allowed after configured deny policies".into(),
        }
    }
}

#[async_trait]
impl<C> Policy<C, FsReadDirAction> for FsReadDirAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-read-dir-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &FsReadDirAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("fs.read_dir reached terminal allow policy".into()),
            reason: "filesystem directory listing allowed after path policy".into(),
        }
    }
}

#[async_trait]
impl<C> Policy<C, FsContentSearchAction> for FsContentSearchAllowPolicy
where
    C: ContextFactory + Send + Sync,
{
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("fs-content-search-allow")
    }

    async fn grant(&self, _ctx: &C::Cx<'_>, _action: &FsContentSearchAction) -> PolicyGrant {
        PolicyGrant {
            decision: PolicyDecision::Allow,
            predicate: Some("fs.content_search reached terminal allow policy".into()),
            reason: "filesystem content search allowed after configured deny policies".into(),
        }
    }
}

#[cfg(test)]
mod tests;
