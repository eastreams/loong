use serde::{Deserialize, Serialize};

use crate::{Capability, GrantId, PolicyReport};

/// Stable authority scope requesting authorization.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorizationScope {
    /// Typed runtime execution owned by a real Session.
    Session { session_id: String },
    /// Token-bearing ingress explicitly quarantined from typed Session authority.
    LegacyToken {
        boundary: String,
        pack_id: String,
        token_id: String,
    },
}

/// Stable identity of the actor and authority scope requesting authorization.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationSubject {
    pub actor_id: String,
    pub scope: AuthorizationScope,
}

/// Backend-assigned identity shared by all evidence from one authorization attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AuthorizationAttemptId(pub u64);

/// Owned action facts captured before policy or permission evaluation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationActionSnapshot {
    pub kind: String,
    pub operation: String,
    pub resource: Option<String>,
    pub required_capabilities: Vec<Capability>,
}

/// Authority that may resolve a permission request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorizationPermissionAuthority {
    Parent,
    User,
}

/// Non-terminal evidence emitted while obtaining permission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorizationPermissionInteraction {
    Requested {
        authority: AuthorizationPermissionAuthority,
    },
    Approved {
        authority: AuthorizationPermissionAuthority,
    },
    Denied {
        authority: AuthorizationPermissionAuthority,
        reason: String,
    },
    /// Parent declined to decide and delegated consent to the user.
    EscalatedToUser,
    Failed {
        authority: AuthorizationPermissionAuthority,
        reason: String,
    },
}

/// Authorization rejection caused by insufficient authority or policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorizationDenial {
    MissingCapability {
        capability: Capability,
    },
    Policy {
        reason: String,
    },
    Permission {
        authority: AuthorizationPermissionAuthority,
        reason: String,
    },
}

/// Authorization could not reach an allow-or-deny decision reliably.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorizationFailure {
    GrantAllocation,
    PermissionRequest {
        authority: AuthorizationPermissionAuthority,
        reason: String,
    },
    EscalationUnavailable {
        authority: AuthorizationPermissionAuthority,
    },
}

/// Final outcome of one authorization attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorizationTerminalOutcome {
    /// Authorization reached terminal allow for this reserved grant identity.
    ///
    /// Core only mints after every configured sink accepts the write. A sink
    /// may still contain this event when a later fanout sink fails, so the event
    /// alone does not prove that an `ActionGrant` was returned or consumed.
    Allow {
        grant_id: GrantId,
    },
    Deny {
        reason: AuthorizationDenial,
    },
    Failure {
        reason: AuthorizationFailure,
    },
}

/// Event produced after policy evaluation has yielded a complete report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorizationPolicyEvent {
    Permission(AuthorizationPermissionInteraction),
    Terminal(AuthorizationTerminalOutcome),
}

/// Stage-specific evidence from an allocated authorization attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorizationAttemptEvent {
    /// Initial capability gate rejected the action before policy evaluation.
    CapabilityDenied { capability: Capability },
    /// Policy has run, so every later interaction and terminal carries its report.
    Policy {
        report: PolicyReport,
        event: AuthorizationPolicyEvent,
    },
}

/// Attempt correlation and event state.
///
/// `StartFailed` cannot carry a policy or terminal event, while `Started`
/// always carries the id allocated for the event it encloses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorizationAttempt {
    StartFailed,
    Started {
        id: AuthorizationAttemptId,
        event: AuthorizationAttemptEvent,
    },
}

/// One typed authorization evidence envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationEvidence {
    pub subject: AuthorizationSubject,
    pub action: AuthorizationActionSnapshot,
    pub attempt: AuthorizationAttempt,
}
