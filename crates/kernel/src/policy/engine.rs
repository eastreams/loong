use uuid::Uuid;

use crate::Facade;

use super::{
    PolicyAny,
    action::{ActionMeta, Denied, Granted},
};

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
            let _ = policy.evaluate(ctx, &action);
            todo!();
        }
        let _ = Granted::<A>::new(Uuid::nil(), action);
        todo!()
    }
}
