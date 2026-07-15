use std::{borrow::Cow, fmt};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

/// Process-independent correlation identity for one action grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GrantId(Uuid);

impl GrantId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for GrantId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for GrantId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Serialize for GrantId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for GrantId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Uuid::parse_str(&value)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

pub type PolicyId = u64;

/// Source location where a policy entered its runtime pipeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRegistrationSource {
    pub file: String,
    pub line: u32,
    pub column: u32,
}

/// Runtime registration facts attached to every evaluated policy.
///
/// `order` is pipeline-wide rather than stage-local, so reports can reconstruct
/// registration order even when evaluation jumps between subchains.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRegistration {
    pub order: u64,
    pub registered_at_unix_ms: u64,
    pub source: PolicyRegistrationSource,
}

/// Decision returned by one policy evaluation.
///
/// `Allow`, `Deny`, and both permission requests are terminal decisions for the
/// whole pipeline. `Continue` and `Advance` are control-flow decisions:
/// `Continue` evaluates the next policy in the current subchain, while
/// `Advance` skips the rest of the current subchain and moves to the next one.
/// Advancing from the final subchain leaves the pipeline without a terminal
/// decision, so the caller's default-deny behavior applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyDecision {
    /// Stop the whole pipeline and authorize the action.
    Allow,
    /// Stop the whole pipeline and reject the action.
    Deny,
    /// Stop policy evaluation and require consent from the parent session.
    RequireParentPermission,
    /// Stop policy evaluation and require consent from the user.
    RequireUserPermission,
    /// Keep evaluating policies in the current subchain.
    Continue,
    /// Stop the current subchain and evaluate the next subchain.
    Advance,
}

/// Result of asking an authority to consent to an already-evaluated action.
///
/// Permission can satisfy consent only. It does not alter action capabilities
/// or replace the policy report that requested it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum PermissionResolution {
    Approved,
    Denied {
        reason: Cow<'static, str>,
    },
    /// Ask the next authority. Parent permission may escalate to the user;
    /// user permission has no higher authority and must resolve terminally.
    Escalate,
}

impl<'de> Deserialize<'de> for PermissionResolution {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        enum Representation {
            Approved,
            Denied { reason: String },
            Escalate,
        }

        Ok(match Representation::deserialize(deserializer)? {
            Representation::Approved => Self::Approved,
            Representation::Denied { reason } => Self::Denied {
                reason: Cow::Owned(reason),
            },
            Representation::Escalate => Self::Escalate,
        })
    }
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
    pub registration: PolicyRegistration,
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
            registration: PolicyRegistration,
        }

        let helper = Helper::deserialize(deserializer)?;
        Ok(Self {
            policy_name: Cow::Owned(helper.policy_name),
            policy_id: helper.policy_id,
            registration: helper.registration,
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
    RequireParentPermission {
        source: PolicyEntry,
        reason: Cow<'static, str>,
    },
    RequireUserPermission {
        source: PolicyEntry,
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
            RequireParentPermission {
                source: PolicyEntry,
                reason: String,
            },
            RequireUserPermission {
                source: PolicyEntry,
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
            Helper::RequireParentPermission { source, reason } => {
                Ok(Self::RequireParentPermission {
                    source,
                    reason: Cow::Owned(reason),
                })
            }
            Helper::RequireUserPermission { source, reason } => Ok(Self::RequireUserPermission {
                source,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn policy_entry() -> PolicyEntry {
        PolicyEntry {
            policy_name: Cow::Borrowed("permission-policy"),
            policy_id: 7,
            registration: PolicyRegistration {
                order: 3,
                registered_at_unix_ms: 11,
                source: PolicyRegistrationSource {
                    file: "policy.rs".to_owned(),
                    line: 5,
                    column: 9,
                },
            },
        }
    }

    #[test]
    fn permission_policy_decisions_round_trip_through_json() {
        for decision in [
            PolicyDecision::RequireParentPermission,
            PolicyDecision::RequireUserPermission,
        ] {
            let encoded = serde_json::to_string(&decision).expect("serialize policy decision");
            let decoded =
                serde_json::from_str::<PolicyDecision>(&encoded).expect("deserialize decision");

            assert_eq!(decoded, decision);
        }
    }

    #[test]
    fn permission_resolutions_round_trip_through_json() {
        for resolution in [
            PermissionResolution::Approved,
            PermissionResolution::Denied {
                reason: Cow::Borrowed("user denied"),
            },
            PermissionResolution::Escalate,
        ] {
            let encoded = serde_json::to_string(&resolution).expect("serialize resolution");
            let decoded = serde_json::from_str::<PermissionResolution>(&encoded)
                .expect("deserialize resolution");

            assert_eq!(decoded, resolution);
        }
    }

    #[test]
    fn permission_policy_outcomes_round_trip_through_json() {
        for outcome in [
            PolicyOutcome::RequireParentPermission {
                source: policy_entry(),
                reason: Cow::Borrowed("parent must approve"),
            },
            PolicyOutcome::RequireUserPermission {
                source: policy_entry(),
                reason: Cow::Borrowed("user must approve"),
            },
        ] {
            let encoded = serde_json::to_string(&outcome).expect("serialize policy outcome");
            let decoded =
                serde_json::from_str::<PolicyOutcome>(&encoded).expect("deserialize outcome");

            assert_eq!(decoded, outcome);
        }
    }

    #[test]
    fn grant_id_round_trips_as_uuid() {
        let grant_id = GrantId::new();
        let encoded = serde_json::to_string(&grant_id).expect("grant id should serialize");
        let decoded =
            serde_json::from_str::<GrantId>(&encoded).expect("grant id should deserialize");

        assert_eq!(decoded, grant_id);
    }
}
