use std::borrow::Cow;

/// Stable identifier for an action grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GrantId(pub u64);

pub type PolicyId = u64;

/// Policy decision for a single action policy evaluation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny,
    Abstain,
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
