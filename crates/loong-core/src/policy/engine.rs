use std::{error::Error, marker::PhantomData};

use async_trait::async_trait;
use loong_contracts::{
    AuthorizationActionSnapshot, AuthorizationAttempt, AuthorizationAttemptEvent,
    AuthorizationAttemptId, AuthorizationDenial, AuthorizationEvidence, AuthorizationFailure,
    AuthorizationPermissionAuthority, AuthorizationPermissionInteraction, AuthorizationPolicyEvent,
    AuthorizationSubject, AuthorizationTerminalOutcome, Capability, GrantId, PermissionResolution,
    PolicyOutcome, PolicyReport,
};

use crate::{
    error::{AuthorizationIdentityKind, PermissionRequestError, PolicyGrantError},
    policy::{
        action::ActionMeta,
        context::{ContextFactory, PolicyContext},
        grant::{ActionGrant, ActionGrantInfo},
    },
};

/// Implementor-facing inputs to the sealed authorization algorithm.
///
/// A backend evaluates policy, allocates correlation identities, and durably
/// writes the evidence supplied by core. It cannot mint grants or reorder the
/// capability, policy, permission, audit, and mint stages owned by
/// [`PolicyEngine::grant`].
#[async_trait]
pub trait PolicyEngineBackend<C: ContextFactory>: Sync {
    /// Concrete durability or identity-allocation failure.
    type AuditError: Error + Send + Sync + 'static;

    /// Evaluate the registered policy chain without minting or writing evidence.
    async fn decide<A: ActionMeta + 'static>(&self, ctx: &C::Cx<'_>, action: &A) -> PolicyReport;

    /// Reserve the identity shared by every event from one authorization attempt.
    fn reserve_authorization_attempt_id(&self) -> Result<AuthorizationAttemptId, Self::AuditError>;

    /// Reserve an identity for a prospective allow.
    ///
    /// Allocation alone is not a grant: core only mints after the matching
    /// terminal allow evidence has been accepted.
    fn reserve_grant_id(&self) -> Result<GrantId, Self::AuditError>;

    /// Durably accept exactly the evidence envelope constructed by core.
    fn write_authorization_evidence(
        &self,
        evidence: &AuthorizationEvidence,
    ) -> Result<(), Self::AuditError>;
}

mod sealed {
    use super::{ContextFactory, PolicyEngineBackend};

    pub trait Sealed<C: ContextFactory> {}

    impl<C, B> Sealed<C> for B
    where
        C: ContextFactory,
        B: PolicyEngineBackend<C>,
    {
    }
}

/// Caller-facing authorization engine.
///
/// The trait is sealed so backend implementors provide decisions and durable
/// evidence I/O while core remains the only owner of grant sequencing.
#[async_trait]
pub trait PolicyEngine<C: ContextFactory>: sealed::Sealed<C> + Sync {
    /// Authorize `action`, durably record terminal allow, and mint its grant.
    ///
    /// A successful return does not mean an outer caller released or consumed
    /// the grant, nor that the action was executed.
    async fn grant<A: ActionMeta>(
        &self,
        ctx: &C::Cx<'_>,
        action: A,
    ) -> Result<ActionGrant<A>, PolicyGrantError>;
}

/// One live authorization attempt and the immutable facts shared by its evidence.
///
/// Keeping evidence construction on this state prevents individual policy,
/// permission, or error branches from inventing a different subject, action,
/// or attempt identity.
struct AuthorizationAttemptState<'engine, C, B>
where
    C: ContextFactory,
    B: PolicyEngineBackend<C>,
{
    backend: &'engine B,
    attempt_id: AuthorizationAttemptId,
    subject: AuthorizationSubject,
    action: AuthorizationActionSnapshot,
    _context: PhantomData<fn() -> C>,
}

impl<'engine, C, B> AuthorizationAttemptState<'engine, C, B>
where
    C: ContextFactory,
    B: PolicyEngineBackend<C>,
{
    fn begin<A: ActionMeta>(
        backend: &'engine B,
        ctx: &C::Cx<'_>,
        action: &A,
    ) -> Result<Self, PolicyGrantError> {
        let metadata = action.metadata();
        let action_snapshot = AuthorizationActionSnapshot {
            kind: metadata.kind.to_owned(),
            operation: metadata.operation.into_owned(),
            resource: action
                .audit_resource()
                .map(|resource| resource.into_owned()),
            required_capabilities: metadata.required_capabilities.into_owned(),
        };
        let subject = ctx.authorization_subject();
        let attempt_id = match backend.reserve_authorization_attempt_id() {
            Ok(attempt_id) => attempt_id,
            Err(source) => {
                let evidence = AuthorizationEvidence {
                    subject,
                    action: action_snapshot,
                    attempt: AuthorizationAttempt::StartFailed,
                };
                if let Err(write_source) = backend.write_authorization_evidence(&evidence) {
                    return Err(PolicyGrantError::IdentityAllocationAndAudit {
                        identity: AuthorizationIdentityKind::Attempt,
                        evidence: Box::new(evidence),
                        allocation_source: Box::new(source),
                        audit_source: Box::new(write_source),
                    });
                }
                return Err(PolicyGrantError::IdentityAllocation {
                    identity: AuthorizationIdentityKind::Attempt,
                    evidence: Box::new(evidence),
                    source: Box::new(source),
                });
            }
        };

        Ok(Self {
            backend,
            attempt_id,
            subject,
            action: action_snapshot,
            _context: PhantomData,
        })
    }

    fn evidence(&self, event: AuthorizationAttemptEvent) -> AuthorizationEvidence {
        AuthorizationEvidence {
            subject: self.subject.clone(),
            action: self.action.clone(),
            attempt: AuthorizationAttempt::Started {
                id: self.attempt_id,
                event,
            },
        }
    }

    fn policy_evidence(
        &self,
        report: &PolicyReport,
        event: AuthorizationPolicyEvent,
    ) -> AuthorizationEvidence {
        self.evidence(AuthorizationAttemptEvent::Policy {
            report: report.clone(),
            event,
        })
    }

    fn write_evidence(&self, evidence: AuthorizationEvidence) -> Result<(), PolicyGrantError> {
        self.backend
            .write_authorization_evidence(&evidence)
            .map_err(|source| PolicyGrantError::Audit {
                evidence: Box::new(evidence),
                source: Box::new(source),
            })
    }

    fn write_policy(
        &self,
        report: &PolicyReport,
        event: AuthorizationPolicyEvent,
    ) -> Result<(), PolicyGrantError> {
        self.write_evidence(self.policy_evidence(report, event))
    }

    fn require_capabilities(
        &self,
        ctx: &(impl PolicyContext + ?Sized),
        report: Option<&PolicyReport>,
    ) -> Result<(), PolicyGrantError> {
        let Some(capability) = missing_required_capability(ctx, &self.action.required_capabilities)
        else {
            return Ok(());
        };
        let evidence = report.map_or_else(
            || self.evidence(AuthorizationAttemptEvent::CapabilityDenied { capability }),
            |report| {
                self.policy_evidence(
                    report,
                    AuthorizationPolicyEvent::Terminal(AuthorizationTerminalOutcome::Deny {
                        reason: AuthorizationDenial::MissingCapability { capability },
                    }),
                )
            },
        );
        self.write_evidence(evidence)?;
        Err(PolicyGrantError::MissingCapability { capability })
    }

    async fn request_permission<A: ActionMeta>(
        &self,
        ctx: &C::Cx<'_>,
        action: &A,
        report: &PolicyReport,
        mut authority: AuthorizationPermissionAuthority,
    ) -> Result<
        (
            AuthorizationPermissionAuthority,
            Result<PermissionResolution, PermissionRequestError>,
        ),
        PolicyGrantError,
    > {
        self.permission_requested(authority, report)?;
        let mut resolution = match authority {
            AuthorizationPermissionAuthority::Parent => {
                ctx.request_parent_permission(action, report).await
            }
            AuthorizationPermissionAuthority::User => {
                ctx.request_user_permission(action, report).await
            }
        };

        if authority == AuthorizationPermissionAuthority::Parent
            && matches!(resolution, Ok(PermissionResolution::Escalate))
        {
            self.write_policy(
                report,
                AuthorizationPolicyEvent::Permission(
                    AuthorizationPermissionInteraction::EscalatedToUser,
                ),
            )?;
            authority = AuthorizationPermissionAuthority::User;
            self.permission_requested(authority, report)?;
            resolution = ctx.request_user_permission(action, report).await;
        }

        Ok((authority, resolution))
    }

    fn permission_requested(
        &self,
        authority: AuthorizationPermissionAuthority,
        report: &PolicyReport,
    ) -> Result<(), PolicyGrantError> {
        self.write_policy(
            report,
            AuthorizationPolicyEvent::Permission(AuthorizationPermissionInteraction::Requested {
                authority,
            }),
        )
    }

    fn permission_approved(
        &self,
        authority: AuthorizationPermissionAuthority,
        report: &PolicyReport,
    ) -> Result<(), PolicyGrantError> {
        self.write_policy(
            report,
            AuthorizationPolicyEvent::Permission(AuthorizationPermissionInteraction::Approved {
                authority,
            }),
        )
    }

    fn permission_denied(
        &self,
        authority: AuthorizationPermissionAuthority,
        report: &PolicyReport,
        reason: std::borrow::Cow<'static, str>,
    ) -> Result<(), PolicyGrantError> {
        self.write_policy(
            report,
            AuthorizationPolicyEvent::Permission(AuthorizationPermissionInteraction::Denied {
                authority,
                reason: reason.to_string(),
            }),
        )?;
        self.write_policy(
            report,
            AuthorizationPolicyEvent::Terminal(AuthorizationTerminalOutcome::Deny {
                reason: AuthorizationDenial::Permission {
                    authority,
                    reason: reason.to_string(),
                },
            }),
        )?;
        Err(PolicyGrantError::PermissionDenied {
            report: Box::new(report.clone()),
            reason,
        })
    }

    fn permission_escalation_failed(
        &self,
        authority: AuthorizationPermissionAuthority,
        report: &PolicyReport,
    ) -> Result<(), PolicyGrantError> {
        self.write_policy(
            report,
            AuthorizationPolicyEvent::Permission(AuthorizationPermissionInteraction::Failed {
                authority,
                reason: "permission escalation is unavailable for user authority".to_owned(),
            }),
        )?;
        self.write_policy(
            report,
            AuthorizationPolicyEvent::Terminal(AuthorizationTerminalOutcome::Failure {
                reason: AuthorizationFailure::EscalationUnavailable { authority },
            }),
        )?;
        Err(PolicyGrantError::PermissionRequest {
            report: Box::new(report.clone()),
            source: PermissionRequestError::EscalationUnavailable,
        })
    }

    fn permission_request_failed(
        &self,
        authority: AuthorizationPermissionAuthority,
        report: &PolicyReport,
        source: PermissionRequestError,
    ) -> Result<(), PolicyGrantError> {
        let reason = source.to_string();
        self.write_policy(
            report,
            AuthorizationPolicyEvent::Permission(AuthorizationPermissionInteraction::Failed {
                authority,
                reason: reason.clone(),
            }),
        )?;
        self.write_policy(
            report,
            AuthorizationPolicyEvent::Terminal(AuthorizationTerminalOutcome::Failure {
                reason: AuthorizationFailure::PermissionRequest { authority, reason },
            }),
        )?;
        Err(PolicyGrantError::PermissionRequest {
            report: Box::new(report.clone()),
            source,
        })
    }

    async fn resolve_permission<A: ActionMeta>(
        &self,
        ctx: &C::Cx<'_>,
        action: &A,
        report: &PolicyReport,
        authority: AuthorizationPermissionAuthority,
    ) -> Result<(), PolicyGrantError> {
        let (authority, resolution) = self
            .request_permission(ctx, action, report, authority)
            .await?;
        match resolution {
            Ok(PermissionResolution::Approved) => self.permission_approved(authority, report),
            Ok(PermissionResolution::Denied { reason }) => {
                self.permission_denied(authority, report, reason)
            }
            Ok(PermissionResolution::Escalate) => {
                self.permission_escalation_failed(authority, report)
            }
            Err(source) => self.permission_request_failed(authority, report, source),
        }
    }

    async fn apply_policy_outcome<A: ActionMeta>(
        &self,
        ctx: &C::Cx<'_>,
        action: &A,
        report: &PolicyReport,
    ) -> Result<(), PolicyGrantError> {
        match report.outcome.clone() {
            PolicyOutcome::Allow { .. } => Ok(()),
            PolicyOutcome::Deny { reason, .. } => {
                self.write_policy(
                    report,
                    AuthorizationPolicyEvent::Terminal(AuthorizationTerminalOutcome::Deny {
                        reason: AuthorizationDenial::Policy {
                            reason: reason.to_string(),
                        },
                    }),
                )?;
                Err(PolicyGrantError::Denied {
                    report: Box::new(report.clone()),
                    reason,
                })
            }
            PolicyOutcome::RequireParentPermission { .. } => {
                self.resolve_permission(
                    ctx,
                    action,
                    report,
                    AuthorizationPermissionAuthority::Parent,
                )
                .await
            }
            PolicyOutcome::RequireUserPermission { .. } => {
                self.resolve_permission(ctx, action, report, AuthorizationPermissionAuthority::User)
                    .await
            }
        }
    }

    fn reserve_grant_id(&self, report: &PolicyReport) -> Result<GrantId, PolicyGrantError> {
        match self.backend.reserve_grant_id() {
            Ok(grant_id) => Ok(grant_id),
            Err(source) => {
                let evidence = self.policy_evidence(
                    report,
                    AuthorizationPolicyEvent::Terminal(AuthorizationTerminalOutcome::Failure {
                        reason: AuthorizationFailure::GrantAllocation,
                    }),
                );
                if let Err(write_source) = self.backend.write_authorization_evidence(&evidence) {
                    return Err(PolicyGrantError::IdentityAllocationAndAudit {
                        identity: AuthorizationIdentityKind::Grant,
                        evidence: Box::new(evidence),
                        allocation_source: Box::new(source),
                        audit_source: Box::new(write_source),
                    });
                }
                Err(PolicyGrantError::IdentityAllocation {
                    identity: AuthorizationIdentityKind::Grant,
                    evidence: Box::new(evidence),
                    source: Box::new(source),
                })
            }
        }
    }
}

#[async_trait]
impl<C, B> PolicyEngine<C> for B
where
    C: ContextFactory,
    B: PolicyEngineBackend<C>,
{
    async fn grant<A: ActionMeta>(
        &self,
        ctx: &C::Cx<'_>,
        action: A,
    ) -> Result<ActionGrant<A>, PolicyGrantError> {
        let attempt = AuthorizationAttemptState::begin(self, ctx, &action)?;
        attempt.require_capabilities(ctx, None)?;
        let report = self.decide(ctx, &action).await;
        attempt.apply_policy_outcome(ctx, &action, &report).await?;
        // Policy and permission decisions may both await external work. Take
        // one fresh authority snapshot before any successful path can mint.
        attempt.require_capabilities(ctx, Some(&report))?;
        let grant_id = attempt.reserve_grant_id(&report)?;
        attempt.write_policy(
            &report,
            AuthorizationPolicyEvent::Terminal(AuthorizationTerminalOutcome::Allow { grant_id }),
        )?;
        // Minting happens only after the terminal allow is durably accepted.
        Ok(ActionGrant::new(
            grant_id,
            ActionGrantInfo {
                report,
                subject: attempt.subject.clone(),
                action: attempt.action.clone(),
            },
            action,
        ))
    }
}

/// Read the context once so one capability gate cannot combine observations
/// from different authority snapshots.
fn missing_required_capability(
    ctx: &(impl PolicyContext + ?Sized),
    required_capabilities: &[Capability],
) -> Option<Capability> {
    let granted_capabilities = ctx.allowed_capabilities();
    required_capabilities
        .iter()
        .copied()
        .find(|capability| !granted_capabilities.contains(*capability))
}
