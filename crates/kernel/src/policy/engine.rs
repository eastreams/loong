//! Three-stage policy pipeline.
//!
//! Policies are evaluated in three stages:
//! 1. `inbound` global policies see every action. They are filters: a denial
//!    stops the pipeline, while abstention or allowance continues to the next
//!    stage.
//! 2. `action` policies are typed to one action. The first final `Allow`
//!    grants the action; a final `Deny` stops the pipeline; abstention
//!    continues.
//! 3. `outbound` global policies are the default grant/deny layer for actions
//!    no typed policy handled.
//!
//! The pipeline records every policy evaluation in a [`PolicyReport`] so the
//! decision is auditable.

use std::collections::VecDeque;

use anymap2::SendSyncAnyMap;
use contracts::policy::{PolicyDecision, PolicyDecisionFinal, PolicyDecisionMiddle, PolicyResult};
use uuid::Uuid;

use super::{
    Policy, PolicyAny, PolicyContext,
    action::{ActionMeta, Denied, Granted},
};

type PolicyId = u64;

/// A policy registered for one concrete action type.
struct RegisteredActionPolicy<A: ActionMeta> {
    id: PolicyId,
    name: &'static str,
    inner: Box<dyn Policy<A>>,
}

/// A policy registered for every action type.
struct RegisteredPolicyAny {
    id: PolicyId,
    name: &'static str,
    inner: Box<dyn PolicyAny>,
}

impl RegisteredPolicyAny {
    fn new<P>(id: PolicyId, policy: P) -> Self
    where
        P: PolicyAny + 'static,
    {
        Self {
            id,
            name: policy.name(),
            inner: Box::new(policy),
        }
    }
}

/// One policy evaluation recorded by the pipeline.
#[derive(Debug, Clone)]
pub struct PolicyEvaluation {
    pub stage: &'static str,
    pub policy_id: PolicyId,
    pub policy_name: &'static str,
    pub result: PolicyResult,
}

/// The final pipeline outcome.
#[derive(Debug, Clone)]
pub enum PolicyReportOutcome {
    Allow {
        policy_id: PolicyId,
        policy_name: &'static str,
        reason: Option<String>,
    },
    Deny {
        reason: Option<String>,
    },
}

/// A full pipeline decision with every intermediate evaluation.
#[derive(Debug, Clone)]
pub struct PolicyReport {
    pub evaluations: Vec<PolicyEvaluation>,
    pub outcome: PolicyReportOutcome,
}

/// The kernel's policy engine.
pub struct PolicyEngine {
    inbound: VecDeque<RegisteredPolicyAny>,
    outbound: VecDeque<RegisteredPolicyAny>,
    action_policies: SendSyncAnyMap,
    next_policy_id: PolicyId,
    next_grant: u128,
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl PolicyEngine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inbound: VecDeque::new(),
            outbound: VecDeque::new(),
            action_policies: SendSyncAnyMap::new(),
            next_policy_id: 1,
            next_grant: 0,
        }
    }

    /// Allows actions whose requirements fit the caller's capability ceiling.
    ///
    /// The inbound stage enforces the capability envelope and the outbound
    /// stage grants everything that passed the envelope.
    #[must_use]
    pub fn allow_capabilities() -> Self {
        let mut engine = Self::new();
        engine.append_inbound(CapabilityEnvelopePolicy);
        engine.append_outbound(AllowAllPolicy);
        engine
    }

    pub fn prepend_inbound<P>(&mut self, policy: P)
    where
        P: PolicyAny + 'static,
    {
        let id = self.allocate_policy_id();
        self.inbound
            .push_front(RegisteredPolicyAny::new(id, policy));
    }

    pub fn append_inbound<P>(&mut self, policy: P)
    where
        P: PolicyAny + 'static,
    {
        let id = self.allocate_policy_id();
        self.inbound.push_back(RegisteredPolicyAny::new(id, policy));
    }

    pub fn prepend_outbound<P>(&mut self, policy: P)
    where
        P: PolicyAny + 'static,
    {
        let id = self.allocate_policy_id();
        self.outbound
            .push_front(RegisteredPolicyAny::new(id, policy));
    }

    pub fn append_outbound<P>(&mut self, policy: P)
    where
        P: PolicyAny + 'static,
    {
        let id = self.allocate_policy_id();
        self.outbound
            .push_back(RegisteredPolicyAny::new(id, policy));
    }

    pub fn prepend_action<A, P>(&mut self, policy: P)
    where
        A: ActionMeta,
        P: Policy<A> + 'static,
    {
        let id = self.allocate_policy_id();
        self.action_policies::<A>()
            .push_front(RegisteredActionPolicy {
                id,
                name: policy.name(),
                inner: Box::new(policy),
            });
    }

    pub fn append_action<A, P>(&mut self, policy: P)
    where
        A: ActionMeta,
        P: Policy<A> + 'static,
    {
        let id = self.allocate_policy_id();
        self.action_policies::<A>()
            .push_back(RegisteredActionPolicy {
                id,
                name: policy.name(),
                inner: Box::new(policy),
            });
    }

    /// Evaluates `action` and returns the full audit report.
    pub fn decide<A: ActionMeta>(&self, context: &PolicyContext, action: &A) -> PolicyReport {
        let mut evaluations = Vec::new();

        for policy in &self.inbound {
            let result = policy.inner.evaluate(context, action);
            evaluations.push(PolicyEvaluation {
                stage: "inbound",
                policy_id: policy.id,
                policy_name: policy.name,
                result: result.clone(),
            });
            let PolicyResult { decision, reason } = result;
            match decision {
                PolicyDecision::Final(PolicyDecisionFinal::Allow) => {
                    return PolicyReport {
                        evaluations,
                        outcome: PolicyReportOutcome::Allow {
                            policy_id: policy.id,
                            policy_name: policy.name,
                            reason,
                        },
                    };
                }
                PolicyDecision::Final(
                    PolicyDecisionFinal::Deny | PolicyDecisionFinal::RequiresApproval,
                ) => {
                    return PolicyReport {
                        evaluations,
                        outcome: PolicyReportOutcome::Deny {
                            reason: Some(
                                reason.unwrap_or_else(|| "inbound policy denied action".into()),
                            ),
                        },
                    };
                }
                PolicyDecision::Middle(PolicyDecisionMiddle::Abstain) => {}
                PolicyDecision::Middle(PolicyDecisionMiddle::SkipChain) => break,
            }
        }

        if let Some(policies) = self
            .action_policies
            .get::<VecDeque<RegisteredActionPolicy<A>>>()
        {
            for policy in policies {
                let result = policy.inner.evaluate(context, action);
                evaluations.push(PolicyEvaluation {
                    stage: "action",
                    policy_id: policy.id,
                    policy_name: policy.name,
                    result: result.clone(),
                });
                let PolicyResult { decision, reason } = result;
                match decision {
                    PolicyDecision::Final(PolicyDecisionFinal::Allow) => {
                        return PolicyReport {
                            evaluations,
                            outcome: PolicyReportOutcome::Allow {
                                policy_id: policy.id,
                                policy_name: policy.name,
                                reason,
                            },
                        };
                    }
                    PolicyDecision::Final(
                        PolicyDecisionFinal::Deny | PolicyDecisionFinal::RequiresApproval,
                    ) => {
                        return PolicyReport {
                            evaluations,
                            outcome: PolicyReportOutcome::Deny {
                                reason: Some(
                                    reason.unwrap_or_else(|| "action policy denied action".into()),
                                ),
                            },
                        };
                    }
                    PolicyDecision::Middle(PolicyDecisionMiddle::Abstain) => {}
                    PolicyDecision::Middle(PolicyDecisionMiddle::SkipChain) => break,
                }
            }
        }

        for policy in &self.outbound {
            let result = policy.inner.evaluate(context, action);
            evaluations.push(PolicyEvaluation {
                stage: "outbound",
                policy_id: policy.id,
                policy_name: policy.name,
                result: result.clone(),
            });
            let PolicyResult { decision, reason } = result;
            match decision {
                PolicyDecision::Final(PolicyDecisionFinal::Allow) => {
                    return PolicyReport {
                        evaluations,
                        outcome: PolicyReportOutcome::Allow {
                            policy_id: policy.id,
                            policy_name: policy.name,
                            reason,
                        },
                    };
                }
                PolicyDecision::Final(
                    PolicyDecisionFinal::Deny | PolicyDecisionFinal::RequiresApproval,
                ) => {
                    return PolicyReport {
                        evaluations,
                        outcome: PolicyReportOutcome::Deny {
                            reason: Some(
                                reason.unwrap_or_else(|| "outbound policy denied action".into()),
                            ),
                        },
                    };
                }
                PolicyDecision::Middle(PolicyDecisionMiddle::Abstain) => {}
                PolicyDecision::Middle(PolicyDecisionMiddle::SkipChain) => break,
            }
        }

        PolicyReport {
            evaluations,
            outcome: PolicyReportOutcome::Deny {
                reason: Some("No matching policy.".into()),
            },
        }
    }

    pub(crate) fn grant<A: ActionMeta>(
        &mut self,
        context: PolicyContext,
        action: A,
    ) -> Result<Granted<A>, Denied> {
        let report = self.decide(&context, &action);
        match report.outcome {
            PolicyReportOutcome::Allow { .. } => {
                let Some(next_grant) = self.next_grant.checked_add(1) else {
                    return Err(Denied {
                        reason: Some("grant id counter exhausted.".into()),
                    });
                };
                self.next_grant = next_grant;
                Ok(Granted::new(Uuid::from_u128(next_grant), action))
            }
            PolicyReportOutcome::Deny { reason } => Err(Denied {
                reason: Some(reason.unwrap_or_else(|| "No matching policy.".into())),
            }),
        }
    }

    fn allocate_policy_id(&mut self) -> PolicyId {
        let id = self.next_policy_id;
        self.next_policy_id += 1;
        id
    }

    fn action_policies<A>(&mut self) -> &mut VecDeque<RegisteredActionPolicy<A>>
    where
        A: ActionMeta,
    {
        self.action_policies.entry().or_default()
    }
}

/// Inbound filter: the action's required capabilities must fit the caller's
/// ceiling.
pub struct CapabilityEnvelopePolicy;

impl PolicyAny for CapabilityEnvelopePolicy {
    fn name(&self) -> &'static str {
        "policy.capability_envelope"
    }

    fn evaluate(&self, context: &PolicyContext, action: &dyn ActionMeta) -> PolicyResult {
        if context.capabilities.covers(action.required_capabilities()) {
            PolicyResult {
                decision: PolicyDecision::Middle(PolicyDecisionMiddle::Abstain),
                reason: None,
            }
        } else {
            PolicyResult {
                decision: PolicyDecision::Final(PolicyDecisionFinal::Deny),
                reason: Some("action exceeds declared capability envelope".into()),
            }
        }
    }
}

/// Outbound default: grant anything that reached this stage.
pub struct AllowAllPolicy;

impl PolicyAny for AllowAllPolicy {
    fn name(&self) -> &'static str {
        "policy.default_allow"
    }

    fn evaluate(&self, _context: &PolicyContext, _action: &dyn ActionMeta) -> PolicyResult {
        PolicyResult {
            decision: PolicyDecision::Final(PolicyDecisionFinal::Allow),
            reason: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use contracts::capability::{Capabilities, Capability};
    use serde_json::Value;

    use super::*;

    #[derive(Debug)]
    struct TestAction {
        caps: Capabilities,
    }

    impl ActionMeta for TestAction {
        fn name(&self) -> Cow<'_, str> {
            Cow::Borrowed("test")
        }

        fn payload(&self) -> Cow<'_, Value> {
            Cow::Owned(Value::Null)
        }

        fn required_capabilities(&self) -> Capabilities {
            self.caps
        }
    }

    struct SkipInbound;

    impl PolicyAny for SkipInbound {
        fn name(&self) -> &'static str {
            "test.skip_inbound"
        }

        fn evaluate(&self, _ctx: &PolicyContext, _action: &dyn ActionMeta) -> PolicyResult {
            PolicyResult {
                decision: PolicyDecision::Middle(PolicyDecisionMiddle::SkipChain),
                reason: None,
            }
        }
    }

    struct DenyInbound;

    impl PolicyAny for DenyInbound {
        fn name(&self) -> &'static str {
            "test.deny_inbound"
        }

        fn evaluate(&self, _ctx: &PolicyContext, _action: &dyn ActionMeta) -> PolicyResult {
            PolicyResult {
                decision: PolicyDecision::Final(PolicyDecisionFinal::Deny),
                reason: Some("inbound denied".into()),
            }
        }
    }

    struct AllowTestAction;

    impl Policy<TestAction> for AllowTestAction {
        fn name(&self) -> &'static str {
            "test.allow_action"
        }

        fn evaluate(&self, _ctx: &PolicyContext, _action: &TestAction) -> PolicyResult {
            PolicyResult {
                decision: PolicyDecision::Final(PolicyDecisionFinal::Allow),
                reason: Some("action allowed".into()),
            }
        }
    }

    #[test]
    fn allow_capabilities_grants_within_envelope_and_denies_outside() {
        let mut engine = PolicyEngine::allow_capabilities();
        let ctx = PolicyContext::new(
            Capability::FsRead.into(),
            std::sync::Arc::new(crate::resource::Resources::new()),
        );

        let granted = engine
            .grant(
                ctx.clone(),
                TestAction {
                    caps: Capability::FsRead.into(),
                },
            )
            .unwrap();
        assert_eq!(granted.action().name(), "test");

        let denied = engine
            .grant(
                ctx,
                TestAction {
                    caps: Capability::FsWrite.into(),
                },
            )
            .unwrap_err();
        assert!(denied.reason.is_some());
    }

    #[test]
    fn skip_chain_moves_to_next_stage_without_running_later_inbound() {
        let mut engine = PolicyEngine::new();
        engine.append_inbound(SkipInbound);
        engine.append_inbound(DenyInbound);
        engine.append_outbound(AllowAllPolicy);

        let ctx = PolicyContext::new(
            Capabilities::empty(),
            std::sync::Arc::new(crate::resource::Resources::new()),
        );
        let action = TestAction {
            caps: Capabilities::empty(),
        };

        let report = engine.decide(&ctx, &action);
        assert!(matches!(report.outcome, PolicyReportOutcome::Allow { .. }));
        assert_eq!(report.evaluations.len(), 2);
        assert_eq!(report.evaluations[0].stage, "inbound");
        assert_eq!(report.evaluations[1].stage, "outbound");
    }

    #[test]
    fn typed_action_policy_grants_without_outbound() {
        let mut engine = PolicyEngine::new();
        engine.append_action::<TestAction, _>(AllowTestAction);

        let ctx = PolicyContext::new(
            Capabilities::empty(),
            std::sync::Arc::new(crate::resource::Resources::new()),
        );
        let action = TestAction {
            caps: Capabilities::empty(),
        };

        let report = engine.decide(&ctx, &action);
        assert!(matches!(report.outcome, PolicyReportOutcome::Allow { .. }));
    }

    #[test]
    fn empty_engine_denies_without_match() {
        let engine = PolicyEngine::new();
        let ctx = PolicyContext::new(
            Capabilities::empty(),
            std::sync::Arc::new(crate::resource::Resources::new()),
        );
        let action = TestAction {
            caps: Capabilities::empty(),
        };

        let report = engine.decide(&ctx, &action);
        assert!(matches!(report.outcome, PolicyReportOutcome::Deny { .. }));
    }
}
