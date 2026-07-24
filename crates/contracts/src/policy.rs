//! Policies form multiple chains, where each chain contains
//! a set of policies that are evaluated in order.
//!
//! The types below describe those decisions. Evaluation has no operational
//! error channel: failures return deny, while `Abstain` and `SkipChain` are
//! deliberate chain-control decisions. Grant-recording and parent-request
//! failures belong to the policy engine.

use alloc::string::String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecisionFinal {
    Allow,
    Deny,
    RequiresApproval,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecisionMiddle {
    /// Go to the next policy in the chain
    Abstain,
    /// Skip the rest of this chain and go to the next chain
    SkipChain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Final(PolicyDecisionFinal),
    Middle(PolicyDecisionMiddle),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyResult {
    pub decision: PolicyDecision,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyResultFinal {
    pub decision: PolicyDecisionFinal,
    pub reason: Option<String>,
}
