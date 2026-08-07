use loong_contracts::policy::{
    PolicyDecision, PolicyDecisionFinal, PolicyDecisionMiddle, PolicyResult,
};
use uuid::Uuid;

use crate::Facade;

use super::{
    PolicyAny,
    action::{ActionMeta, Denied, Granted},
};

#[derive(Default)]
pub struct PolicyEngine {
    inbound: Vec<Box<dyn PolicyAny>>,
}

impl PolicyEngine {
    pub fn grant<A: ActionMeta>(&self, ctx: &Facade, action: A) -> Result<Granted<A>, Denied> {
        if !ctx.capabilities.covers(action.required_capabilities()) {
            return Err(Denied {
                reason: Some("Capabilities not allowed.".into()),
            });
        }

        for policy in &self.inbound {
            let PolicyResult { decision, reason } = policy.evaluate(ctx, &action);
            return match decision {
                PolicyDecision::Final(PolicyDecisionFinal::Allow) => {
                    Ok(Granted::<A>::new(Uuid::nil(), action))
                }
                PolicyDecision::Final(
                    PolicyDecisionFinal::Deny | PolicyDecisionFinal::RequiresApproval,
                ) => Err(Denied { reason }),
                PolicyDecision::Middle(PolicyDecisionMiddle::Abstain) => continue,
                PolicyDecision::Middle(PolicyDecisionMiddle::SkipChain) => break,
            };
        }

        Err(Denied {
            reason: Some(String::from("No matching policy.")),
        })
    }
}
