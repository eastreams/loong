use super::super::ingress::{ConversationIngressContext, inject_internal_tool_ingress};
use super::payload::augment_tool_payload_for_kernel;
use super::support::{
    RepairableToolPreflight, approval_required_tool_decision, render_app_tool_denied_reason,
};
use super::visibility::{concealed_provider_tool_denial, provider_tool_denial_reason};
use super::{
    AppContext, AppToolDispatcher, AutonomyTurnBudgetState, ConversationRuntimeBinding,
    SessionStoreConfig, ToolDecisionTelemetry, ToolDispatchKind, ToolExecutionKind,
    ToolExecutionPreflight, ToolIntent, ToolPreflightOutcome, TurnResult,
    effective_denied_tool_name,
};
use loong_contracts::{Capabilities, ToolCoreRequest};
use loong_runtime::tool_plane::{ToolPath, error::LookupError};

#[derive(Debug, Clone)]
pub(super) struct PreparedToolIntent {
    pub(super) intent_sequence: usize,
    pub(super) intent: ToolIntent,
    pub(super) request: ToolCoreRequest,
    pub(super) capabilities_override: Option<Capabilities>,
    pub(super) dispatch_kind: ToolDispatchKind,
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
    session_context: &'a AppContext,
    memory_config: &'a SessionStoreConfig,
    app_dispatcher: &'a D,
    binding: ConversationRuntimeBinding<'b>,
    budget_state: &'a AutonomyTurnBudgetState,
    ingress: Option<&'a ConversationIngressContext>,
}

impl<'a, 'b, D: AppToolDispatcher + ?Sized> ToolIntentPreparationHarness<'a, 'b, D> {
    pub(super) fn new(
        session_context: &'a AppContext,
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
        let outer_request = ToolCoreRequest {
            tool_name: intent.tool_name.clone(),
            payload: intent.args_json.clone(),
        };
        let outer_tool_name =
            crate::tools::canonical_tool_name(intent.tool_name.as_str()).to_owned();
        let outer_path = ToolPath::from(outer_tool_name.clone());
        let outer_registered = match self.session_context.runtime().tool_spec(&outer_path) {
            Ok(_) => true,
            Err(LookupError::NotRegistered { .. }) => false,
            Err(error) => {
                let reason = error.to_string();
                return Err(PreparedToolIntentFailure {
                    intent: intent.clone(),
                    turn_result: TurnResult::non_retryable_tool_error(
                        "tool_registry_failed",
                        reason.clone(),
                    ),
                    decision: ToolDecisionTelemetry::deny(
                        outer_tool_name,
                        reason,
                        "tool_registry_failed",
                    ),
                });
            }
        };
        // `tool.invoke` is only a legacy envelope when no concrete registered
        // tool owns that exact path. Lease validation still precedes inner-path
        // lookup, so an invalid lease cannot probe registry membership.
        let leased_invocation = !outer_registered && outer_tool_name == "tool.invoke";
        let (requested_tool_name, raw_payload, capabilities_override) = if leased_invocation {
            match crate::tools::resolve_tool_invoke_request(
                &outer_request,
                crate::tools::ToolInvokeProviderExposure::RejectProviderExposed,
            ) {
                Ok(resolved) => (
                    resolved.request.tool_name,
                    resolved.request.payload,
                    resolved.capabilities_override,
                ),
                Err(reason) => {
                    let (turn_result, rule_id) = if reason.starts_with("invalid_tool_lease:") {
                        (
                            TurnResult::ToolDenied(
                                super::TurnFailure::policy_denied_with_discovery_recovery(
                                    "invalid_tool_lease",
                                    reason.clone(),
                                ),
                            ),
                            "invalid_tool_lease",
                        )
                    } else if reason.starts_with("tool_not_provider_exposed:")
                        || reason.starts_with("tool_not_found:")
                    {
                        let recovery_reason = provider_tool_denial_reason(
                            "tool_not_found: tool.invoke",
                            intent.source.as_str(),
                        );
                        (
                            TurnResult::ToolDenied(
                                super::TurnFailure::policy_denied_with_discovery_recovery(
                                    "tool_not_found",
                                    recovery_reason,
                                ),
                            ),
                            "tool_not_found",
                        )
                    } else {
                        (
                            TurnResult::non_retryable_tool_error(
                                "tool_invoke_resolution_failed",
                                reason.clone(),
                            ),
                            "tool_invoke_resolution_failed",
                        )
                    };
                    let decision = ToolDecisionTelemetry::deny(
                        effective_denied_tool_name(intent),
                        reason,
                        rule_id,
                    );

                    return Err(PreparedToolIntentFailure {
                        intent: intent.clone(),
                        turn_result,
                        decision,
                    });
                }
            }
        } else {
            (outer_tool_name, intent.args_json.clone(), None)
        };

        let typed_path = ToolPath::from(requested_tool_name.as_str());
        let typed_registered = if outer_registered {
            true
        } else if leased_invocation {
            match self.session_context.runtime().tool_spec(&typed_path) {
                Ok(_) => true,
                Err(LookupError::NotRegistered { .. }) => false,
                Err(error) => {
                    let reason = error.to_string();
                    return Err(PreparedToolIntentFailure {
                        intent: intent.clone(),
                        turn_result: TurnResult::non_retryable_tool_error(
                            "tool_registry_failed",
                            reason.clone(),
                        ),
                        decision: ToolDecisionTelemetry::deny(
                            requested_tool_name.as_str(),
                            reason,
                            "tool_registry_failed",
                        ),
                    });
                }
            }
        } else {
            false
        };
        let (dispatch_kind, effective_tool_name) = if typed_registered {
            (ToolDispatchKind::Typed, requested_tool_name)
        } else if let Some(resolved) =
            crate::tools::resolve_legacy_tool_execution(requested_tool_name.as_str())
        {
            match resolved.execution_kind {
                ToolExecutionKind::Core => (
                    ToolDispatchKind::LegacyCore,
                    resolved.canonical_name.to_owned(),
                ),
                ToolExecutionKind::App => (
                    ToolDispatchKind::LegacyApp,
                    resolved.canonical_name.to_owned(),
                ),
            }
        } else {
            let raw_reason = format!("tool_not_found: {requested_tool_name}");
            let reason = provider_tool_denial_reason(raw_reason.as_str(), intent.source.as_str());
            let failure = if intent.source.starts_with("provider_") {
                super::TurnFailure::policy_denied_with_discovery_recovery(
                    "tool_not_found",
                    reason.clone(),
                )
            } else {
                super::TurnFailure::policy_denied("tool_not_found", reason.clone())
            };
            return Err(PreparedToolIntentFailure {
                intent: intent.clone(),
                turn_result: TurnResult::ToolDenied(failure),
                decision: ToolDecisionTelemetry::deny(
                    requested_tool_name,
                    reason,
                    "tool_not_found",
                ),
            });
        };
        if leased_invocation
            && !self
                .session_context
                .tool_view
                .contains(effective_tool_name.as_str())
        {
            let failure = concealed_provider_tool_denial();
            let decision = ToolDecisionTelemetry::deny(
                effective_tool_name,
                failure.reason.clone(),
                failure.code.clone(),
            );
            return Err(PreparedToolIntentFailure {
                intent: intent.clone(),
                turn_result: TurnResult::ToolDenied(failure),
                decision,
            });
        }
        let descriptor = crate::tools::tool_catalog()
            .resolve(effective_tool_name.as_str())
            .copied();

        let injected =
            inject_internal_tool_ingress(effective_tool_name.as_str(), raw_payload, self.ingress);
        let normalized_payload = crate::tools::normalize_shell_payload_for_request(
            effective_tool_name.as_str(),
            injected.payload,
        );
        let injected_trusted_internal_context = injected.trusted_internal_context;
        let injected_payload_uses_reserved_internal_context =
            crate::tools::payload_uses_reserved_internal_tool_context(&normalized_payload);
        if capabilities_override.is_some() && dispatch_kind != ToolDispatchKind::Typed {
            let reason = format!(
                "tool.invoke capabilities_override requires a registered typed tool; `{typed_path}` is legacy-only"
            );
            return Err(PreparedToolIntentFailure {
                intent: intent.clone(),
                turn_result: TurnResult::retryable_tool_error("tool_input_invalid", reason.clone()),
                decision: ToolDecisionTelemetry::deny(
                    effective_tool_name.as_str(),
                    reason,
                    "tool_input_invalid",
                ),
            });
        }
        if dispatch_kind != ToolDispatchKind::Typed
            && descriptor.is_some_and(|descriptor| descriptor.is_direct())
            && let Err(reason) = crate::tools::route_direct_tool_name(
                effective_tool_name.as_str(),
                &normalized_payload,
            )
        {
            let human_reason = RepairableToolPreflight::render(reason.as_str());
            let turn_result =
                TurnResult::retryable_tool_error("tool_preflight_denied", human_reason.clone());
            let decision = ToolDecisionTelemetry::deny(
                effective_tool_name.as_str(),
                human_reason,
                "tool_preflight_denied",
            );
            return Err(PreparedToolIntentFailure {
                intent: intent.clone(),
                turn_result,
                decision,
            });
        }
        let augmented_payload = augment_tool_payload_for_kernel(
            effective_tool_name.as_str(),
            normalized_payload.clone(),
            self.session_context,
            self.memory_config,
        );
        let augmented_payload_uses_reserved_internal_context =
            crate::tools::payload_uses_reserved_internal_tool_context(&augmented_payload.payload);
        let prepared_trusted_internal_context = injected_trusted_internal_context
            || augmented_payload.trusted_internal_context
            || (!injected_payload_uses_reserved_internal_context
                && augmented_payload_uses_reserved_internal_context);
        let effective_request = ToolCoreRequest {
            tool_name: effective_tool_name.clone(),
            payload: augmented_payload.payload,
        };
        let effective_intent = ToolIntent {
            tool_name: effective_tool_name.clone(),
            args_json: normalized_payload,
            source: intent.source.clone(),
            session_id: intent.session_id.clone(),
            turn_id: intent.turn_id.clone(),
            tool_call_id: intent.tool_call_id.clone(),
        };
        let capability_action_class = descriptor.map_or(
            crate::tools::CapabilityActionClass::ExecuteExisting,
            |descriptor| descriptor.capability_action_class(),
        );
        // TODO(typed-tool-metadata): take scheduling from the registered ToolSpec
        // once that type becomes the metadata owner. Descriptorless typed tools
        // remain serial until then so this dispatch commit cannot over-parallelize.
        let scheduling_class = descriptor.map_or(
            crate::tools::ToolSchedulingClass::SerialOnly,
            |descriptor| descriptor.scheduling_class(),
        );

        // TODO(typed-tool-permission): Migrated tools still use the old autonomy
        // approval surface until its decisions become ToolInvocationAction
        // policies. A descriptorless registration is governed only by the
        // runtime PolicyEngine and must not acquire a synthetic catalog row.
        let preflight_decision = match descriptor.as_ref() {
            Some(descriptor) => {
                self.app_dispatcher
                    .preflight_tool_intent_with_binding(
                        self.session_context,
                        &effective_intent,
                        &effective_request,
                        prepared_trusted_internal_context,
                        descriptor,
                        dispatch_kind,
                        capabilities_override.as_ref(),
                        self.binding,
                        self.budget_state,
                    )
                    .await
            }
            None if dispatch_kind == ToolDispatchKind::Typed => {
                Ok(ToolPreflightOutcome::Allow(ToolDecisionTelemetry::allow(
                    effective_tool_name.as_str(),
                    "typed tool authorization deferred to runtime policy",
                    "typed_runtime_policy",
                )))
            }
            None => {
                let reason = format!("tool_descriptor_missing: {effective_tool_name}");
                return Err(PreparedToolIntentFailure {
                    intent: effective_intent,
                    turn_result: TurnResult::non_retryable_tool_error(
                        "tool_descriptor_missing",
                        reason.clone(),
                    ),
                    decision: ToolDecisionTelemetry::deny(
                        effective_tool_name,
                        reason,
                        "tool_descriptor_missing",
                    ),
                });
            }
        };
        let decision = match preflight_decision {
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

        let requires_kernel_binding = match dispatch_kind {
            ToolDispatchKind::Typed => false,
            ToolDispatchKind::LegacyCore => true,
            ToolDispatchKind::LegacyApp => descriptor
                .as_ref()
                .is_some_and(|descriptor| descriptor.requires_kernel_binding()),
        };

        if requires_kernel_binding && self.binding.context().is_none() {
            let turn_result = TurnResult::policy_denied("no_app_context", "no_app_context");
            let denial_decision = ToolDecisionTelemetry::deny(
                effective_tool_name.as_str(),
                "no_app_context",
                "no_app_context",
            );

            return Err(PreparedToolIntentFailure {
                intent: effective_intent,
                turn_result,
                decision: denial_decision,
            });
        }

        let preflight = if dispatch_kind == ToolDispatchKind::Typed {
            Ok(ToolExecutionPreflight::ready(effective_request))
        } else if let Some(descriptor) = descriptor.as_ref() {
            self.app_dispatcher
                .preflight_tool_execution_with_binding(
                    self.session_context,
                    &effective_intent,
                    effective_request,
                    descriptor,
                    self.binding,
                )
                .await
        } else {
            let reason = format!("tool_descriptor_missing: {effective_tool_name}");
            return Err(PreparedToolIntentFailure {
                intent: effective_intent,
                turn_result: TurnResult::non_retryable_tool_error(
                    "tool_descriptor_missing",
                    reason.clone(),
                ),
                decision: ToolDecisionTelemetry::deny(
                    effective_tool_name,
                    reason,
                    "tool_descriptor_missing",
                ),
            });
        };

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
                let turn_result = TurnResult::non_retryable_tool_error(
                    "app_tool_preflight_failed",
                    reason.clone(),
                );
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

        let trusted_internal_context =
            prepared_trusted_internal_context || trusted_preflight_context;

        Ok(PreparedToolIntent {
            intent_sequence,
            intent: effective_intent,
            request: effective_request,
            capabilities_override,
            dispatch_kind,
            capability_action_class,
            scheduling_class,
            trusted_internal_context,
            decision,
        })
    }
}
