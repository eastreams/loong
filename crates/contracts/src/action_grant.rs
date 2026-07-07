use std::borrow::Cow;

/// Stable identifier for an action grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GrantId(pub u64);

pub type PolicyId = u64;

/// Decision returned by one policy evaluation.
///
/// `Allow` and `Deny` are terminal decisions for the whole pipeline.
/// `Continue` and `Advance` are control-flow decisions: `Continue` evaluates
/// the next policy in the current subchain, while `Advance` skips the rest of
/// the current subchain and moves to the next one. Advancing from the final
/// subchain leaves the pipeline without a terminal decision, so the caller's
/// default-deny behavior applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyDecision {
    /// Stop the whole pipeline and authorize the action.
    Allow,
    /// Stop the whole pipeline and reject the action.
    Deny,
    /// Keep evaluating policies in the current subchain.
    Continue,
    /// Stop the current subchain and evaluate the next subchain.
    Advance,
}

/// Result returned by one single action policy.
///
/// `reason` is authored by the policy that produced this grant. Callers should
/// preserve it instead of generating replacement explanations in orchestration
/// code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyGrant {
    pub decision: PolicyDecision,
    /// The predicate, such as `path starts with "/tmp"`
    pub predicate: Option<Cow<'static, str>>,
    pub reason: Cow<'static, str>,
}

/// One ordered policy evaluation in an action policy report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyEvaluation {
    pub source: PolicyEntry,
    /// Pipeline subchain that produced this evaluation.
    ///
    /// Current kernel stages are `pre`, `action`, and `fallback`.
    pub policy_stage: &'static str,
    pub grant: PolicyGrant,
}

/// Used to reference a policy entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyEntry {
    pub policy_name: Cow<'static, str>,
    pub policy_id: PolicyId,
}

/// Final policy outcome for an action authorization attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyOutcome {
    Allow {
        /// Default Deny, so this field should exist.
        source: PolicyEntry,
        reason: Cow<'static, str>,
    },
    Deny {
        grant_source: Option<PolicyEntry>,
        reason: Cow<'static, str>,
    },
}

/// Complete report for one action authorization attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyReport {
    pub evaluations: Vec<PolicyEvaluation>,
    pub outcome: PolicyOutcome,
}
