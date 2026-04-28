use loong_contracts::ToolCoreRequest;
use super::super::ingress::{ConversationIngressContext, inject_internal_tool_ingress};
use super::payload::augment_tool_payload_for_kernel;
use super::support::{
    RepairableToolPreflight, approval_required_tool_decision, render_app_tool_denied_reason,
};
use super::target::{prepare_conversation_kernel_tool_request, resolve_effective_tool_metadata};
use super::visibility::provider_tool_denial_reason;
use super::{
    AppToolDispatcher, AutonomyTurnBudgetState, ConversationRuntimeBinding, SessionContext,
    SessionStoreConfig, ToolDecisionTelemetry, ToolExecutionKind, ToolExecutionPreflight,
    ToolIntent, ToolPreflightOutcome, TurnResult, effective_denied_tool_name,
};

#[derive(Debug, Clone)]
pub(super) struct PreparedToolIntent {
    pub(super) intent_sequence: usize,
    pub(super) intent: ToolIntent,
    pub(super) request: ToolCoreRequest,
    pub(super) execution_kind: ToolExecutionKind,
    pub(super) capability_action_class: crate::tools::CapabilityActionClass,
    pub(super) scheduling_class: crate::tools::ToolSchedulingClass,
    pub(super) trusted_internal_context: bool,
    pub(super) decision: ToolDecisionTelemetry,
}

#[derive(Debug, Clone)]
pub(super) struct PreparedToolIntentFailure {
    pub(super) intent: ToolIntent,
    pub(super) turn_result: TurnResult,
    pub(super) decision: ToolDecisionTelemetry,
}

#[derive(Clone, Copy)]
pub(super) struct ToolIntentPreparationHarness<'a, 'b, D: AppToolDispatcher + ?Sized> {
    session_context: &'a SessionContext,
    memory_config: &'a SessionStoreConfig,
    app_dispatcher: &'a D,
    binding: ConversationRuntimeBinding<'b>,
    budget_state: &'a AutonomyTurnBudgetState,
    ingress: Option<&'a ConversationIngressContext>,
}

impl<'a, 'b, D: AppToolDispatcher + ?Sized> ToolIntentPreparationHarness<'a, 'b, D> {
    pub(super) fn new(
        session_context: &'a SessionContext,
        memory_config: &'a SessionStoreConfig,
        app_dispatcher: &'a D,
        binding: ConversationRuntimeBinding<'b>,
        budget_state: &'a AutonomyTurnBudgetState,
        ingress: Option<&'a ConversationIngressContext>,
    ) -> Self {
        Self {
            session_context,
            memory_config,
            app_dispatcher,
            binding,
            budget_state,
            ingress,
        }
    }

    pub(super) async fn prepare(
        self,
        intent: &ToolIntent,
        intent_sequence: usize,
    ) -> Result<PreparedToolIntent, PreparedToolIntentFailure> {
        let Some(resolved_tool) = crate::tools::resolve_tool_execution(&intent.tool_name) else {
            let denied_tool_name = effective_denied_tool_name(intent);
            let raw_reason = format!("tool_not_found: {denied_tool_name}");
            let reason = provider_tool_denial_reason(raw_reason.as_str(), intent.source.as_str());
            let failure = if intent.source.starts_with("provider_") {
                super::TurnFailure::policy_denied_with_discovery_recovery(
                    "tool_not_found",
                    reason.clone(),
                )
            } else {
                super::TurnFailure::policy_denied("tool_not_found", reason.clone())
            };
            let turn_result = TurnResult::ToolDenied(failure);
            let decision =
                ToolDecisionTelemetry::deny(denied_tool_name.as_str(), reason, "tool_not_found");

            return Err(PreparedToolIntentFailure {
                intent: intent.clone(),
                turn_result,
                decision,
            });
        };

        let injected = inject_internal_tool_ingress(
            resolved_tool.canonical_name,
            intent.args_json.clone(),
            self.ingress,
        );
        let normalized_payload = crate::tools::normalize_shell_payload_for_request(
            resolved_tool.canonical_name,
            injected.payload,
        );
        let injected_payload_uses_reserved_internal_context =
            crate::tools::payload_uses_reserved_internal_tool_context(&normalized_payload);
        let augmented_payload = augment_tool_payload_for_kernel(
            resolved_tool.canonical_name,
            normalized_payload.clone(),
            self.session_context,
            self.memory_config,
        );
        let augmented_payload_uses_reserved_internal_context =
            crate::tools::payload_uses_reserved_internal_tool_context(&augmented_payload.payload);
        let request = ToolCoreRequest {
            tool_name: resolved_tool.canonical_name.to_owned(),
            payload: augmented_payload.payload,
        };
        let request = prepare_conversation_kernel_tool_request(request, self.binding, intent);
        let normalized_intent = ToolIntent {
            tool_name: resolved_tool.canonical_name.to_owned(),
            args_json: normalized_payload,
            source: intent.source.clone(),
            session_id: intent.session_id.clone(),
            turn_id: intent.turn_id.clone(),
            tool_call_id: intent.tool_call_id.clone(),
        };
        let effective_tool_metadata =
            resolve_effective_tool_metadata(resolved_tool, request, normalized_intent, intent);
        let effective_tool_metadata = match effective_tool_metadata {
            Ok(metadata) => metadata,
            Err(error) => {
                let effective_target = error.effective_target;
                let effective_tool_name = effective_target.tool_name;
                let effective_intent = effective_target.intent;
                let reason = format!("tool_descriptor_missing: {}", effective_tool_name);
                let turn_result =
                    TurnResult::non_retryable_tool_error("tool_descriptor_missing", reason.clone());
                let decision = ToolDecisionTelemetry::deny(
                    effective_tool_name.as_str(),
                    reason,
                    "tool_descriptor_missing",
                );

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision,
                });
            }
        };
        let effective_execution_kind = effective_tool_metadata.execution_kind;
        let effective_request = effective_tool_metadata.request;
        let effective_intent = effective_tool_metadata.intent;
        let effective_tool_name = effective_tool_metadata.tool_name;
        let descriptor = effective_tool_metadata.descriptor;
        let capability_action_class = effective_tool_metadata.capability_action_class;
        let scheduling_class = effective_tool_metadata.scheduling_class;

        let decision = match self
            .app_dispatcher
            .preflight_tool_intent_with_binding(
                self.session_context,
                &effective_intent,
                &descriptor,
                self.binding,
                self.budget_state,
            )
            .await
        {
            Ok(ToolPreflightOutcome::Allow(decision)) => decision,
            Ok(ToolPreflightOutcome::NeedsApproval {
                requirement,
                decision,
            }) => {
                let turn_result = TurnResult::NeedsApproval(requirement);

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision,
                });
            }
            Ok(ToolPreflightOutcome::Denied { failure, decision }) => {
                let turn_result = TurnResult::ToolDenied(failure);

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision,
                });
            }
            Err(reason) if reason.starts_with("app_tool_denied:") => {
                let human_reason = render_app_tool_denied_reason(reason.as_str());
                let turn_result =
                    TurnResult::policy_denied("app_tool_denied", human_reason.clone());
                let denial_decision = ToolDecisionTelemetry::deny(
                    effective_tool_name.as_str(),
                    human_reason,
                    "app_tool_denied",
                );

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision: denial_decision,
                });
            }
            Err(reason) => {
                let turn_result =
                    TurnResult::non_retryable_tool_error("tool_preflight_failed", reason.clone());
                let denial_decision = ToolDecisionTelemetry::deny(
                    effective_tool_name.as_str(),
                    reason,
                    "tool_preflight_failed",
                );

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision: denial_decision,
                });
            }
        };

        let requires_kernel_binding = match effective_execution_kind {
            ToolExecutionKind::Core => true,
            ToolExecutionKind::App => descriptor.requires_kernel_binding(),
        };

        if requires_kernel_binding && self.binding.kernel_context().is_none() {
            let turn_result = TurnResult::policy_denied("no_kernel_context", "no_kernel_context");
            let denial_decision = ToolDecisionTelemetry::deny(
                effective_tool_name.as_str(),
                "no_kernel_context",
                "no_kernel_context",
            );

            return Err(PreparedToolIntentFailure {
                intent: effective_intent,
                turn_result,
                decision: denial_decision,
            });
        }

        let preflight = self
            .app_dispatcher
            .preflight_tool_execution_with_binding(
                self.session_context,
                &effective_intent,
                effective_request,
                &descriptor,
                self.binding,
            )
            .await;

        let (effective_request, trusted_preflight_context) = match preflight {
            Ok(ToolExecutionPreflight::Ready {
                request,
                trusted_internal_context,
            }) => (request, trusted_internal_context),
            Ok(ToolExecutionPreflight::NeedsApproval(requirement)) => {
                let turn_result = TurnResult::NeedsApproval(requirement.clone());
                let approval_decision =
                    approval_required_tool_decision(effective_tool_name.as_str(), &requirement);

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision: approval_decision,
                });
            }
            Err(reason) if reason.starts_with("app_tool_denied:") => {
                let human_reason = render_app_tool_denied_reason(reason.as_str());
                let turn_result =
                    TurnResult::policy_denied("app_tool_denied", human_reason.clone());
                let denial_decision = ToolDecisionTelemetry::deny(
                    effective_tool_name.as_str(),
                    human_reason,
                    "app_tool_denied",
                );

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision: denial_decision,
                });
            }
            Err(reason) if RepairableToolPreflight::parse(reason.as_str()).is_some() => {
                let stripped =
                    RepairableToolPreflight::parse(reason.as_str()).unwrap_or(reason.as_str());
                let human_reason = RepairableToolPreflight::render(stripped);
                let turn_result =
                    TurnResult::retryable_tool_error("tool_preflight_denied", human_reason.clone());
                let denial_decision = ToolDecisionTelemetry::deny(
                    effective_tool_name.as_str(),
                    human_reason,
                    "tool_preflight_denied",
                );

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision: denial_decision,
                });
            }
            Err(reason) if reason.starts_with("tool_preflight_denied:") => {
                let turn_result =
                    TurnResult::policy_denied("tool_preflight_denied", reason.clone());
                let denial_decision = ToolDecisionTelemetry::deny(
                    effective_tool_name.as_str(),
                    reason,
                    "tool_preflight_denied",
                );

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision: denial_decision,
                });
            }
            Err(reason) => {
                let turn_result =
                    TurnResult::non_retryable_tool_error("app_tool_preflight_failed", reason.clone());
                let denial_decision = ToolDecisionTelemetry::deny(
                    effective_tool_name.as_str(),
                    reason,
                    "app_tool_preflight_failed",
                );

                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result,
                    decision: denial_decision,
                });
            }
        };

        let injected_trusted_internal_context = injected.trusted_internal_context
            || augmented_payload.trusted_internal_context
            || (!injected_payload_uses_reserved_internal_context
                && augmented_payload_uses_reserved_internal_context);
        let trusted_internal_context =
            injected_trusted_internal_context || trusted_preflight_context;

        Ok(PreparedToolIntent {
            intent_sequence,
            intent: effective_intent,
            request: effective_request,
            execution_kind: effective_execution_kind,
            capability_action_class,
            scheduling_class,
            trusted_internal_context,
            decision,
        })
    }
}
