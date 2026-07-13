use std::collections::BTreeSet;

use async_trait::async_trait;
use loong_contracts::{Capability, GrantId, PermissionResolution, PolicyOutcome, PolicyReport};

use crate::{
    error::{PermissionRequestError, PolicyGrantError},
    policy::{
        action::ActionMeta,
        context::{ContextFactory, PolicyContext},
        grant::{ActionGrant, ActionGrantInfo},
    },
};

/// Authorization engine for typed actions.
///
/// Implementors decide actions and provide grant context. Core turns an allow
/// report into a [`Granted`] token and turns deny reports into structured
/// authorization errors without inventing policy reasons.
#[async_trait]
pub trait PolicyEngine<C: ContextFactory>: Sync {
    /// Evaluate a borrowed action without consuming it.
    async fn decide<A: ActionMeta + 'static>(&self, ctx: &C::Cx<'_>, action: &A) -> PolicyReport;

    /// Allocate the next grant id for an allowed action.
    async fn next_grant_id(&self) -> GrantId;

    /// Authorize `action` and return grant metadata plus a token that can be
    /// consumed by access code.
    ///
    /// The capability gate is deliberately built in here so policies only run
    /// after the caller already has every capability declared by the action.
    async fn grant<A: ActionMeta>(
        &self,
        ctx: &C::Cx<'_>,
        action: A,
    ) -> Result<ActionGrant<A>, PolicyGrantError>
    where
        A: 'static,
    {
        let metadata = action.metadata();
        let granted_capabilities = ctx.allowed_capabilities();
        for capability in metadata.required_capabilities.iter().copied() {
            if !granted_capabilities.contains(&capability) {
                return Err(PolicyGrantError::MissingCapability { capability });
            }
        }

        let report = self.decide(ctx, &action).await;
        let permission_resolution = match report.outcome.clone() {
            PolicyOutcome::Allow {
                source: _,
                reason: _,
            } => {
                return Ok(ActionGrant::new(
                    self.next_grant_id().await,
                    ActionGrantInfo { report },
                    action,
                ));
            }
            PolicyOutcome::Deny {
                grant_source: _,
                reason,
            } => {
                return Err(PolicyGrantError::Denied {
                    report: Box::new(report),
                    reason,
                });
            }
            PolicyOutcome::RequireParentPermission { .. } => {
                match ctx.request_parent_permission(&action, &report).await {
                    Ok(PermissionResolution::Escalate) => {
                        ctx.request_user_permission(&action, &report).await
                    }
                    resolution => resolution,
                }
            }
            PolicyOutcome::RequireUserPermission { .. } => {
                ctx.request_user_permission(&action, &report).await
            }
        };

        let permission_resolution = match permission_resolution {
            Ok(resolution) => resolution,
            Err(source) => {
                return Err(PolicyGrantError::PermissionRequest {
                    report: Box::new(report),
                    source,
                });
            }
        };

        match permission_resolution {
            PermissionResolution::Approved => {
                // Permission awaits external input. Recheck authority before
                // minting a grant so consent cannot restore capabilities lost
                // while the request was pending.
                let granted_capabilities = ctx.allowed_capabilities();
                let metadata = action.metadata();
                for capability in metadata.required_capabilities.iter().copied() {
                    if !granted_capabilities.contains(&capability) {
                        return Err(PolicyGrantError::MissingCapability { capability });
                    }
                }

                Ok(ActionGrant::new(
                    self.next_grant_id().await,
                    ActionGrantInfo { report },
                    action,
                ))
            }
            PermissionResolution::Denied { reason } => Err(PolicyGrantError::PermissionDenied {
                report: Box::new(report),
                reason,
            }),
            PermissionResolution::Escalate => Err(PolicyGrantError::PermissionRequest {
                report: Box::new(report),
                source: PermissionRequestError::EscalationUnavailable,
            }),
        }
    }
}

impl PolicyContext for () {
    fn allowed_capabilities(&self) -> &BTreeSet<Capability> {
        static EMPTY: BTreeSet<Capability> = BTreeSet::new();
        &EMPTY
    }
}
