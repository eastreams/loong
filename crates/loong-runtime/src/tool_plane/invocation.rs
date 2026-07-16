//! Governed typed tool invocation owned by the runtime boundary.

use async_trait::async_trait;
use loong_contracts::{ActionExecutionEvent, AuditEventKind, Capabilities, Capability};
use loong_core::{
    kernel::Kernel as CoreKernel,
    policy::{
        action::Action,
        context::{ContextFactory, PolicyContext},
        engine::PolicyEngine,
        grant::Granted,
    },
};
use loong_kernel::Kernel;
use serde_json::Value;

use super::{
    RegisteredToolError, ToolInvocationAction, ToolPath,
    error::{CapabilityOverrideError, ToolInvocationError},
    registered::RegisteredTool,
};

/// Context capability required by runtime-owned tool invocation.
///
/// This is the only direct contract between the runtime wrapper and an
/// app-defined context. It exists to derive a same-type child with narrower
/// authority; it does not expose Runtime, Kernel, audit, or a context factory.
pub trait ToolInvocationContext: PolicyContext + Sized {
    fn derive_tool_child(
        &self,
        capabilities: Capabilities,
    ) -> Result<Self, super::error::CapabilityNarrowingError>;
}

/// One looked-up typed tool invocation bound to its recursive execution context.
///
/// Construction is runtime-owned so ordinary callers cannot obtain registered
/// entry execution capability. Use the app context's `tool(path)` entrypoint.
pub struct ToolInvocation<'runtime, 'context, C>
where
    C: ContextFactory,
    C: 'context,
{
    kernel: &'runtime Kernel<C>,
    tool: &'runtime RegisteredTool<C>,
    context: &'runtime C::Cx<'context>,
    path: ToolPath,
    declared_capabilities: Capabilities,
    capability_override: Option<Capabilities>,
}

impl<'runtime, 'context, C> ToolInvocation<'runtime, 'context, C>
where
    C: ContextFactory,
    C: 'context,
    C::Cx<'context>: ToolInvocationContext,
{
    pub(crate) fn new(
        kernel: &'runtime Kernel<C>,
        tool: &'runtime RegisteredTool<C>,
        context: &'runtime C::Cx<'context>,
        path: ToolPath,
        declared_capabilities: Capabilities,
    ) -> Self {
        Self {
            kernel,
            tool,
            context,
            path,
            declared_capabilities,
            capability_override: None,
        }
    }

    /// Request a replacement for the tool's declared domain capabilities.
    ///
    /// Validation belongs to `invoke` so an escalation attempt cannot fail
    /// before the runtime records its rejection.
    #[must_use]
    pub fn with_capabilities_override(mut self, capabilities: Capabilities) -> Self {
        self.capability_override = Some(capabilities);
        self
    }

    /// Grant, audit, and dispatch this invocation as one indivisible runtime boundary.
    pub async fn invoke(self, payload: Value) -> Result<Value, ToolInvocationError> {
        let Self {
            kernel,
            tool,
            context,
            path,
            declared_capabilities,
            capability_override,
        } = self;
        let tool_capabilities = match capability_override {
            Some(requested) if !requested.is_subset(&declared_capabilities) => {
                let rejection = CapabilityOverrideError {
                    path,
                    requested,
                    declared: declared_capabilities,
                };
                let subject = context.authorization_subject();
                let actor_id = subject.actor_id.clone();
                let path_display = rejection.path.to_string();
                return match kernel.record_audit_event(
                    Some(actor_id.as_str()),
                    AuditEventKind::ToolCapabilityOverrideRejected {
                        subject,
                        path_display,
                        requested: rejection.requested.clone(),
                        declared: rejection.declared.clone(),
                    },
                ) {
                    Ok(()) => Err(ToolInvocationError::CapabilityOverride(rejection)),
                    Err(audit_source) => Err(ToolInvocationError::CapabilityOverrideAndAudit {
                        rejection,
                        audit_source,
                    }),
                };
            }
            Some(requested) => requested,
            None => declared_capabilities,
        };
        let required_capabilities: Capabilities = std::iter::once(Capability::InvokeTool)
            .chain(tool_capabilities.iter())
            .collect();
        let parent_capabilities = context.allowed_capabilities();
        let child_capabilities: Capabilities = required_capabilities
            .intersection(parent_capabilities.as_ref())
            .collect();
        let child = context
            .derive_tool_child(child_capabilities.clone())
            .map_err(ToolInvocationError::from)?;
        let derived_capabilities = child.allowed_capabilities();
        if !derived_capabilities.is_subset(&child_capabilities) {
            return Err(ToolInvocationError::CapabilityNarrowing(
                super::error::CapabilityNarrowingError {
                    allowed: child_capabilities,
                    derived: derived_capabilities.into_owned(),
                },
            ));
        }
        let action = ToolInvocationAction::new(path, required_capabilities, payload);
        let grant = kernel
            .policy_engine()
            .grant(&child, action)
            .await
            .map_err(ToolInvocationError::from)?;
        grant
            .into_granted()
            .run(&ToolInvocationExecution {
                kernel,
                tool,
                context: &child,
            })
            .await
    }
}

/// Runtime-private execution dependencies for one already granted invocation.
struct ToolInvocationExecution<'a, 'context, C>
where
    C: ContextFactory + 'context,
{
    kernel: &'a Kernel<C>,
    tool: &'a RegisteredTool<C>,
    context: &'a C::Cx<'context>,
}

/// Owns the interval between accepted `Started` evidence and an observed result.
///
/// This private guard is not an authorization receipt or public audit handle.
/// Dropping the invocation future closes its audit trail with the only fact the
/// runtime still knows: the concrete outcome was not observed. Release panic is
/// process-aborting and therefore intentionally outside this in-process guard.
struct StartedToolInvocation<'a, C>
where
    C: ContextFactory,
{
    kernel: &'a Kernel<C>,
    granted: &'a Granted<ToolInvocationAction>,
    armed: bool,
}

impl<'a, C> StartedToolInvocation<'a, C>
where
    C: ContextFactory,
{
    fn new(kernel: &'a Kernel<C>, granted: &'a Granted<ToolInvocationAction>) -> Self {
        Self {
            kernel,
            granted,
            armed: true,
        }
    }

    fn outcome_observed(mut self) {
        self.armed = false;
    }
}

impl<C> Drop for StartedToolInvocation<'_, C>
where
    C: ContextFactory,
{
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // A dropped future has no caller left to receive an audit error. The
        // synchronous write is necessarily best effort; durable delivery and
        // recovery after process abort remain the sink/supervisor boundary.
        let _ = self
            .kernel
            .record_granted_action_execution(self.granted, ActionExecutionEvent::OutcomeUnknown);
    }
}

#[async_trait]
impl<'a, 'context, C> Action<ToolInvocationExecution<'a, 'context, C>> for ToolInvocationAction
where
    C: ContextFactory + 'context,
    C::Cx<'context>: ToolInvocationContext,
{
    type Output = Value;
    type Error = ToolInvocationError;

    async fn run(
        granted: Granted<Self>,
        execution: &ToolInvocationExecution<'a, 'context, C>,
    ) -> Result<Self::Output, Self::Error> {
        let grant_id = granted.id();
        execution
            .kernel
            .record_granted_action_execution(&granted, ActionExecutionEvent::Started)
            .map_err(|source| ToolInvocationError::StartAudit { grant_id, source })?;

        let started = StartedToolInvocation::new(execution.kernel, &granted);
        let dispatch = execution
            .tool
            .invoke(execution.context, granted.as_ref().payload().clone())
            .await;
        started.outcome_observed();
        match dispatch {
            Ok(output) => match execution
                .kernel
                .record_granted_action_execution(&granted, ActionExecutionEvent::Completed)
            {
                Ok(()) => Ok(output),
                Err(source) => Err(ToolInvocationError::CompletedAudit {
                    grant_id,
                    output,
                    source,
                }),
            },
            Err(dispatch_source) => {
                let event = match &dispatch_source {
                    RegisteredToolError::Input(error) => ActionExecutionEvent::InputRejected {
                        error: error.clone(),
                    },
                    RegisteredToolError::Denied { source }
                    | RegisteredToolError::Execution { source } => ActionExecutionEvent::Failed {
                        reason: source.to_string(),
                    },
                };
                match execution
                    .kernel
                    .record_granted_action_execution(&granted, event)
                {
                    Ok(()) => Err(ToolInvocationError::Dispatch {
                        grant_id,
                        source: dispatch_source,
                    }),
                    Err(audit_source) => Err(ToolInvocationError::DispatchAndAudit {
                        grant_id,
                        dispatch_source,
                        audit_source,
                    }),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
