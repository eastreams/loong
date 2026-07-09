use std::borrow::Cow;

use serde::{Deserialize, Deserializer, Serialize};

/// Stable identifier for an action grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PolicyGrant {
    pub decision: PolicyDecision,
    /// The predicate, such as `path starts with "/tmp"`
    pub predicate: Option<Cow<'static, str>>,
    pub reason: Cow<'static, str>,
}

impl<'de> Deserialize<'de> for PolicyGrant {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Helper {
            decision: PolicyDecision,
            predicate: Option<String>,
            reason: String,
        }

        let helper = Helper::deserialize(deserializer)?;
        Ok(Self {
            decision: helper.decision,
            predicate: helper.predicate.map(Cow::Owned),
            reason: Cow::Owned(helper.reason),
        })
    }
}

/// One ordered policy evaluation in an action policy report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PolicyEvaluation {
    pub source: PolicyEntry,
    /// Pipeline subchain that produced this evaluation.
    ///
    /// Current kernel stages are `pre`, `action`, and `fallback`.
    pub policy_stage: Cow<'static, str>,
    pub grant: PolicyGrant,
}

impl<'de> Deserialize<'de> for PolicyEvaluation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Helper {
            source: PolicyEntry,
            policy_stage: String,
            grant: PolicyGrant,
        }

        let helper = Helper::deserialize(deserializer)?;
        Ok(Self {
            source: helper.source,
            policy_stage: Cow::Owned(helper.policy_stage),
            grant: helper.grant,
        })
    }
}

/// Used to reference a policy entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PolicyEntry {
    pub policy_name: Cow<'static, str>,
    pub policy_id: PolicyId,
}

impl<'de> Deserialize<'de> for PolicyEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Helper {
            policy_name: String,
            policy_id: PolicyId,
        }

        let helper = Helper::deserialize(deserializer)?;
        Ok(Self {
            policy_name: Cow::Owned(helper.policy_name),
            policy_id: helper.policy_id,
        })
    }
}

/// Final policy outcome for an action authorization attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
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

impl<'de> Deserialize<'de> for PolicyOutcome {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        enum Helper {
            Allow {
                source: PolicyEntry,
                reason: String,
            },
            Deny {
                grant_source: Option<PolicyEntry>,
                reason: String,
            },
        }

        match Helper::deserialize(deserializer)? {
            Helper::Allow { source, reason } => Ok(Self::Allow {
                source,
                reason: Cow::Owned(reason),
            }),
            Helper::Deny {
                grant_source,
                reason,
            } => Ok(Self::Deny {
                grant_source,
                reason: Cow::Owned(reason),
            }),
        }
    }
}

/// Complete report for one action authorization attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyReport {
    pub evaluations: Vec<PolicyEvaluation>,
    pub outcome: PolicyOutcome,
}
