use contracts::policy::{
    PolicyDecision, PolicyDecisionFinal, PolicyDecisionMiddle, PolicyResult,
};
use uuid::Uuid;

use super::{
    PolicyAny, PolicyContext,
    action::{ActionMeta, Denied, Granted},
};

#[derive(Default)]
pub struct PolicyEngine {
    inbound: Vec<Box<dyn PolicyAny>>,
    next_grant: u128,
}

impl PolicyEngine {
    /// Allows actions whose requirements fit the caller's capability ceiling.
    /// A grant names the allowed action; capabilities gate admission.
    ///
    /// This is the explicit policy used by the deterministic application
    /// probe.
    #[must_use]
    pub fn allow_capabilities() -> Self {
        Self {
            inbound: vec![Box::new(AllowPolicy)],
            next_grant: 0,
        }
    }

    pub(crate) fn grant<A: ActionMeta>(
        &mut self,
        context: PolicyContext,
        action: A,
    ) -> Result<Granted<A>, Denied> {
        if !context.capabilities.covers(action.required_capabilities()) {
            return Err(Denied {
                reason: Some("Capabilities not allowed.".into()),
            });
        }

        let mut allowed = false;
        let mut denied_reason = None;

        for policy in &self.inbound {
            let PolicyResult { decision, reason } = policy.evaluate(&context, &action);
            match decision {
                PolicyDecision::Final(PolicyDecisionFinal::Allow) => {
                    allowed = true;
                    break;
                }
                PolicyDecision::Final(
                    PolicyDecisionFinal::Deny | PolicyDecisionFinal::RequiresApproval,
                ) => {
                    denied_reason = reason;
                    break;
                }
                PolicyDecision::Middle(PolicyDecisionMiddle::Abstain) => continue,
                PolicyDecision::Middle(PolicyDecisionMiddle::SkipChain) => break,
            };
        }

        if allowed {
            let Some(next_grant) = self.next_grant.checked_add(1) else {
                return Err(Denied {
                    reason: Some("grant id counter exhausted.".into()),
                });
            };
            self.next_grant = next_grant;
            return Ok(Granted::new(Uuid::from_u128(next_grant), action));
        }

        Err(Denied {
            reason: Some(denied_reason.unwrap_or_else(|| String::from("No matching policy."))),
        })
    }
}

struct AllowPolicy;

impl PolicyAny for AllowPolicy {
    fn evaluate(&self, _context: &PolicyContext, _action: &dyn ActionMeta) -> PolicyResult {
        PolicyResult {
            decision: PolicyDecision::Final(PolicyDecisionFinal::Allow),
            reason: None,
        }
    }
}
