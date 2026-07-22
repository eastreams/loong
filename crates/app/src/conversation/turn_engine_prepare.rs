use super::super::ingress::{ConversationIngressContext, inject_internal_tool_ingress};
use super::payload::augment_tool_payload_for_kernel;
use super::support::{
    LegacyRepairablePreflight, approval_required_tool_decision, render_app_tool_denied_reason,
};
use super::visibility::concealed_provider_tool_denial;
use super::{
    AutonomyTurnBudgetState, Context, LegacyToolDispatchKind, LegacyToolDispatcher,
    LegacyToolExecutionPreflight, LegacyToolPreflightOutcome, ToolDecisionTelemetry, ToolIntent,
    TurnEngine, TurnResult, effective_denied_tool_name, tool_intent_skips_provider_exposed_gate,
};
use loong_contracts::{
    Capability, GovernedSessionMode, ToolCoreRequest, ToolPath, ToolSchedulingClass,
};
use loong_core::policy::context::PolicyContext;
use loong_runtime::tool_plane::{ToolInvocation, ToolRegistration, error::LookupError};
use serde_json::Value;

#[derive(Debug)]
pub(super) enum PreparedToolInvocation<'a> {
    Typed(PreparedTypedToolInvocation<'a>),
    Legacy {
        invocation: PreparedLegacyToolInvocation,
        decision: ToolDecisionTelemetry,
    },
}

/// A registry-owned invocation prepared without legacy bearer evidence.
///
/// Keeping this payload distinct lets the typed executor's signature exclude
/// `CapabilityToken` instead of relying on a mixed executor not to use it.
#[derive(Debug)]
pub(super) struct PreparedTypedToolInvocation<'a> {
    /// The first successful registry lookup stays bound through grant and dispatch.
    pub(super) invocation: ToolInvocation<'a, 'a, crate::RuntimeContextFactory>,
    pub(super) payload: Value,
}

/// An invocation whose owner is still one of the explicit legacy fallbacks.
#[derive(Debug, Clone)]
pub(super) enum PreparedLegacyToolInvocation {
    Core {
        request: ToolCoreRequest,
        trusted_internal_context: bool,
    },
    App {
        request: ToolCoreRequest,
    },
}

#[derive(Debug)]
pub(super) struct PreparedToolIntent<'a> {
    pub(super) intent_sequence: usize,
    pub(super) intent: ToolIntent,
    pub(super) invocation: PreparedToolInvocation<'a>,
    pub(super) capability_action_class: crate::tools::CapabilityActionClass,
    pub(super) scheduling_class: ToolSchedulingClass,
}

#[derive(Debug, Clone)]
pub(super) struct PreparedToolIntentFailure {
    pub(super) intent: ToolIntent,
    pub(super) turn_result: TurnResult,
    pub(super) decision: ToolDecisionTelemetry,
}

impl TurnEngine {
    pub(super) async fn prepare_tool_intent<'a, D: LegacyToolDispatcher + ?Sized>(
        &self,
        intent: &ToolIntent,
        intent_sequence: usize,
        session_context: &Context<'a>,
        legacy_dispatcher: &D,
        budget_state: &AutonomyTurnBudgetState,
        ingress: Option<&ConversationIngressContext>,
    ) -> Result<PreparedToolIntent<'a>, PreparedToolIntentFailure> {
        // Typed lookup receives the raw ingress path. Its identity is exactly
        // the path registered by Runtime; only the explicit legacy branch below
        // may canonicalize legacy names after a typed miss.
        let registered_target = intent.tool_name.registered_path().cloned();
        let outer_path = match registered_target.clone() {
            Some(path) => path,
            None => match ToolPath::new([intent.tool_name()]) {
                Ok(path) => path,
                Err(error) => {
                    let reason = error.to_string();
                    return Err(PreparedToolIntentFailure {
                        intent: intent.clone(),
                        turn_result: TurnResult::non_retryable_tool_error(
                            "tool_path_invalid",
                            reason.clone(),
                        ),
                        decision: ToolDecisionTelemetry::deny(
                            intent.tool_name(),
                            reason,
                            "tool_path_invalid",
                        ),
                    });
                }
            },
        };
        let outer_invocation = match session_context.tool(outer_path.clone()) {
            Ok(invocation) => Some(invocation),
            Err(LookupError::NotRegistered { .. }) if registered_target.is_none() => None,
            Err(LookupError::NotRegistered { .. }) => {
                let reason =
                    format!("provider-selected registered tool is no longer present: {outer_path}");
                return Err(PreparedToolIntentFailure {
                    intent: intent.clone(),
                    turn_result: TurnResult::non_retryable_tool_error(
                        "tool_registry_failed",
                        reason.clone(),
                    ),
                    decision: ToolDecisionTelemetry::deny(
                        intent.tool_name(),
                        reason,
                        "tool_registry_failed",
                    ),
                });
            }
            Err(error) => {
                let reason = error.to_string();
                return Err(PreparedToolIntentFailure {
                    intent: intent.clone(),
                    turn_result: TurnResult::non_retryable_tool_error(
                        "tool_registry_failed",
                        reason.clone(),
                    ),
                    decision: ToolDecisionTelemetry::deny(
                        intent.tool_name(),
                        reason,
                        "tool_registry_failed",
                    ),
                });
            }
        };
        // `tool.invoke` is an ingress envelope only when no concrete registered
        // tool owns that outer path. No lookup result is exposed until an inner
        // lease validates against the exact runtime-owned path.
        let outer_registered = outer_invocation.is_some();
        let leased_invocation = !outer_registered
            && crate::tools::canonical_tool_name(intent.tool_name()) == "tool.invoke";
        let mut typed_invocation = outer_invocation;
        let (requested_tool_name, typed_path, raw_payload, capabilities_override) =
            if let Some(invocation) = typed_invocation.as_ref() {
                let canonical_path = invocation.path().clone();
                // Provider wire identity and plane lookup identity are distinct.
                // A resolved request keeps both instead of deriving one from the other.
                let provider_name = match &intent.tool_name {
                    super::ToolIntentTarget::Registered { provider_name, .. } => {
                        provider_name.clone()
                    }
                    super::ToolIntentTarget::Unresolved { .. } => match invocation.registration() {
                        ToolRegistration::Direct { provider_name } => provider_name.clone(),
                        ToolRegistration::Discoverable { discovery_name } => discovery_name.clone(),
                    },
                };
                (
                    provider_name,
                    canonical_path,
                    intent.args_json.clone(),
                    None,
                )
            } else if leased_invocation {
                let parsed = match crate::tools::parse_tool_invoke_request(
                    intent.tool_name(),
                    &intent.args_json,
                ) {
                    Ok(parsed) => parsed,
                    Err(reason) => {
                        return Err(PreparedToolIntentFailure {
                            intent: intent.clone(),
                            turn_result: TurnResult::non_retryable_tool_error(
                                "tool_invoke_resolution_failed",
                                reason.clone(),
                            ),
                            decision: ToolDecisionTelemetry::deny(
                                effective_denied_tool_name(intent),
                                reason,
                                "tool_invoke_resolution_failed",
                            ),
                        });
                    }
                };
                let inner_lookup = session_context.tool(parsed.path.clone());
                let (canonical_path, resolved_invocation, requested_name) = match inner_lookup {
                    Ok(invocation) => (
                        invocation.path().clone(),
                        Some(invocation),
                        parsed.requested_name.to_owned(),
                    ),
                    Err(LookupError::NotRegistered { .. }) => {
                        let legacy_name =
                            crate::tools::canonical_tool_name(parsed.requested_name).to_owned();
                        let path = match ToolPath::new([legacy_name.as_str()]) {
                            Ok(path) => path,
                            Err(error) => {
                                let reason = error.to_string();
                                return Err(PreparedToolIntentFailure {
                                    intent: intent.clone(),
                                    turn_result: TurnResult::non_retryable_tool_error(
                                        "tool_path_invalid",
                                        reason.clone(),
                                    ),
                                    decision: ToolDecisionTelemetry::deny(
                                        parsed.requested_name,
                                        reason,
                                        "tool_path_invalid",
                                    ),
                                });
                            }
                        };
                        (path, None, legacy_name)
                    }
                    Err(error) => {
                        let reason = error.to_string();
                        return Err(PreparedToolIntentFailure {
                            intent: intent.clone(),
                            turn_result: TurnResult::non_retryable_tool_error(
                                "tool_registry_failed",
                                reason.clone(),
                            ),
                            decision: ToolDecisionTelemetry::deny(
                                parsed.path.to_string(),
                                reason,
                                "tool_registry_failed",
                            ),
                        });
                    }
                };
                let resolved = match parsed.resolve(canonical_path) {
                    Ok(resolved) => resolved,
                    Err(reason) => {
                        let turn_result = if reason.starts_with("invalid_tool_lease:") {
                            TurnResult::ToolDenied(
                                super::TurnFailure::policy_denied_with_discovery_recovery(
                                    "invalid_tool_lease",
                                    reason.clone(),
                                ),
                            )
                        } else {
                            TurnResult::non_retryable_tool_error(
                                "tool_invoke_resolution_failed",
                                reason.clone(),
                            )
                        };
                        let rule_id = if reason.starts_with("invalid_tool_lease:") {
                            "invalid_tool_lease"
                        } else {
                            "tool_invoke_resolution_failed"
                        };
                        return Err(PreparedToolIntentFailure {
                            intent: intent.clone(),
                            turn_result,
                            decision: ToolDecisionTelemetry::deny(
                                effective_denied_tool_name(intent),
                                reason,
                                rule_id,
                            ),
                        });
                    }
                };
                typed_invocation = resolved_invocation;
                (
                    requested_name,
                    resolved.path,
                    resolved.payload,
                    resolved.capabilities_override,
                )
            } else {
                let legacy_name = crate::tools::canonical_tool_name(intent.tool_name()).to_owned();
                let legacy_path = match ToolPath::new([legacy_name.as_str()]) {
                    Ok(path) => path,
                    Err(error) => {
                        let reason = error.to_string();
                        return Err(PreparedToolIntentFailure {
                            intent: intent.clone(),
                            turn_result: TurnResult::non_retryable_tool_error(
                                "tool_path_invalid",
                                reason.clone(),
                            ),
                            decision: ToolDecisionTelemetry::deny(
                                legacy_name,
                                reason,
                                "tool_path_invalid",
                            ),
                        });
                    }
                };
                (
                    legacy_name.clone(),
                    legacy_path,
                    intent.args_json.clone(),
                    None,
                )
            };
        if let Some(invocation) = typed_invocation {
            let direct = matches!(invocation.registration(), ToolRegistration::Direct { .. });
            if leased_invocation && direct {
                let reason = format!(
                    "tool_not_provider_exposed: {requested_tool_name} must be called directly"
                );
                let failure = super::TurnFailure::policy_denied_with_discovery_recovery(
                    "tool_not_provider_exposed",
                    reason.clone(),
                );
                let decision = ToolDecisionTelemetry::deny(
                    requested_tool_name,
                    reason,
                    "tool_not_provider_exposed",
                );
                return Err(PreparedToolIntentFailure {
                    intent: intent.clone(),
                    turn_result: TurnResult::ToolDenied(failure),
                    decision,
                });
            }
            // Provider calls have exactly two typed ingress forms: direct
            // registrations use their own path, discoverable registrations use
            // the already validated `tool.invoke` envelope above.
            if !leased_invocation && intent.source.starts_with("provider_") && !direct {
                let failure = concealed_provider_tool_denial();
                let decision = ToolDecisionTelemetry::deny(
                    requested_tool_name,
                    failure.reason.clone(),
                    failure.code.clone(),
                );
                return Err(PreparedToolIntentFailure {
                    intent: intent.clone(),
                    turn_result: TurnResult::ToolDenied(failure),
                    decision,
                });
            }
            let scheduling_class = invocation.spec().scheduling;
            let invocation = match capabilities_override {
                Some(capabilities) => invocation.with_capabilities_override(capabilities),
                None => invocation,
            };
            let effective_intent = ToolIntent {
                tool_name: super::ToolIntentTarget::registered(
                    typed_path,
                    requested_tool_name.clone(),
                ),
                args_json: raw_payload.clone(),
                source: intent.source.clone(),
                turn_id: intent.turn_id.clone(),
                tool_call_id: intent.tool_call_id.clone(),
            };
            return Ok(PreparedToolIntent {
                intent_sequence,
                intent: effective_intent,
                invocation: PreparedToolInvocation::Typed(PreparedTypedToolInvocation {
                    invocation,
                    payload: raw_payload,
                }),
                capability_action_class: crate::tools::CapabilityActionClass::ExecuteExisting,
                scheduling_class,
            });
        }

        // Typed preparation returns above. Legacy-only configuration must not
        // shape registry-owned invocations or become a hidden typed dependency.
        let memory_config = legacy_dispatcher
            .memory_config()
            .unwrap_or(crate::session::store::current_session_store_config());
        let (dispatch_kind, effective_tool_name) = if let Some(resolved) =
            crate::tools::resolve_legacy_tool_execution(requested_tool_name.as_str())
        {
            match resolved {
                crate::tools::ResolvedLegacyToolExecution::Core { canonical_name } => (
                    LegacyToolDispatchKind::LegacyCore,
                    canonical_name.to_owned(),
                ),
                crate::tools::ResolvedLegacyToolExecution::App { canonical_name } => {
                    (LegacyToolDispatchKind::LegacyApp, canonical_name.to_owned())
                }
            }
        } else {
            let failure = if intent.source.starts_with("provider_") {
                concealed_provider_tool_denial()
            } else {
                super::TurnFailure::policy_denied(
                    "tool_not_found",
                    format!("tool_not_found: {requested_tool_name}"),
                )
            };
            let reason = failure.reason.clone();
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
            && !session_context
                .session()
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
        if !leased_invocation {
            match descriptor.as_ref() {
                Some(descriptor) => {
                    if intent.source.starts_with("provider_") && !descriptor.is_provider_exposed() {
                        let failure = concealed_provider_tool_denial();
                        return Err(PreparedToolIntentFailure {
                            intent: intent.clone(),
                            turn_result: TurnResult::ToolDenied(failure.clone()),
                            decision: ToolDecisionTelemetry::deny(
                                effective_tool_name,
                                failure.reason,
                                failure.code,
                            ),
                        });
                    }
                    if !descriptor.is_provider_exposed()
                        && !session_context
                            .session()
                            .tool_view
                            .contains(descriptor.name)
                    {
                        let reason = format!("tool_not_visible: {}", descriptor.name);
                        return Err(PreparedToolIntentFailure {
                            intent: intent.clone(),
                            turn_result: TurnResult::policy_denied(
                                "tool_not_visible",
                                reason.clone(),
                            ),
                            decision: ToolDecisionTelemetry::deny(
                                descriptor.name,
                                reason,
                                "tool_not_visible",
                            ),
                        });
                    }
                    if !tool_intent_skips_provider_exposed_gate(intent, descriptor)
                        && !crate::tools::is_provider_exposed_tool_name(intent.tool_name())
                    {
                        let reason = format!("tool_not_provider_exposed: {}", intent.tool_name());
                        return Err(PreparedToolIntentFailure {
                            intent: intent.clone(),
                            turn_result: TurnResult::policy_denied(
                                "tool_not_provider_exposed",
                                reason.clone(),
                            ),
                            decision: ToolDecisionTelemetry::deny(
                                effective_tool_name,
                                reason,
                                "tool_not_provider_exposed",
                            ),
                        });
                    }
                }
                None => {
                    if intent.source.starts_with("provider_") {
                        let failure = concealed_provider_tool_denial();
                        return Err(PreparedToolIntentFailure {
                            intent: intent.clone(),
                            turn_result: TurnResult::ToolDenied(failure.clone()),
                            decision: ToolDecisionTelemetry::deny(
                                effective_tool_name,
                                failure.reason,
                                failure.code,
                            ),
                        });
                    }
                    if !session_context
                        .session()
                        .tool_view
                        .contains(effective_tool_name.as_str())
                    {
                        let reason = format!("tool_not_visible: {effective_tool_name}");
                        return Err(PreparedToolIntentFailure {
                            intent: intent.clone(),
                            turn_result: TurnResult::policy_denied(
                                "tool_not_visible",
                                reason.clone(),
                            ),
                            decision: ToolDecisionTelemetry::deny(
                                effective_tool_name,
                                reason,
                                "tool_not_visible",
                            ),
                        });
                    }
                }
            }
        }

        let injected =
            inject_internal_tool_ingress(effective_tool_name.as_str(), raw_payload, ingress);
        let normalized_payload = crate::tools::normalize_shell_payload_for_request(
            effective_tool_name.as_str(),
            injected.payload,
        );
        let injected_trusted_internal_context = injected.trusted_internal_context;
        let injected_payload_uses_reserved_internal_context =
            crate::tools::payload_uses_reserved_internal_tool_context(&normalized_payload);
        if capabilities_override.is_some() {
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
        if descriptor.is_some_and(|descriptor| descriptor.is_direct())
            && let Err(reason) = crate::tools::route_direct_tool_name(
                effective_tool_name.as_str(),
                &normalized_payload,
            )
        {
            let human_reason = LegacyRepairablePreflight::render(reason.as_str());
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
            session_context,
            memory_config,
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
            tool_name: effective_tool_name.clone().into(),
            args_json: normalized_payload,
            source: intent.source.clone(),
            turn_id: intent.turn_id.clone(),
            tool_call_id: intent.tool_call_id.clone(),
        };
        let capability_action_class = descriptor.map_or(
            crate::tools::CapabilityActionClass::ExecuteExisting,
            |descriptor| descriptor.capability_action_class(),
        );
        let scheduling_class = descriptor.map_or(ToolSchedulingClass::SerialOnly, |descriptor| {
            descriptor.scheduling_class()
        });
        let Some(descriptor) = descriptor.as_ref() else {
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
        let requires_kernel_binding = match dispatch_kind {
            LegacyToolDispatchKind::LegacyCore => true,
            LegacyToolDispatchKind::LegacyApp => descriptor.requires_kernel_binding(),
        };
        let context_allows_legacy_execution = session_context.session().session_mode
            == GovernedSessionMode::MutatingCapable
            && session_context
                .allowed_capabilities()
                .contains(Capability::InvokeTool);
        if requires_kernel_binding && !context_allows_legacy_execution {
            let reason = "session lacks mutating legacy tool authority";
            let turn_result = TurnResult::policy_denied("legacy_tool_authority_denied", reason);
            let denial_decision = ToolDecisionTelemetry::deny(
                effective_tool_name.as_str(),
                reason,
                "legacy_tool_authority_denied",
            );

            return Err(PreparedToolIntentFailure {
                intent: effective_intent,
                turn_result,
                decision: denial_decision,
            });
        }
        let preflight_decision = legacy_dispatcher
            .preflight_tool_intent(
                session_context,
                &effective_intent,
                &effective_request,
                prepared_trusted_internal_context,
                descriptor,
                dispatch_kind,
                budget_state,
            )
            .await;
        let decision = match preflight_decision {
            Ok(LegacyToolPreflightOutcome::Allow(decision)) => decision,
            Ok(LegacyToolPreflightOutcome::NeedsApproval {
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
            Ok(LegacyToolPreflightOutcome::Denied { failure, decision }) => {
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

        let preflight = legacy_dispatcher
            .preflight_tool_execution(
                session_context,
                &effective_intent,
                effective_request,
                descriptor,
            )
            .await;

        let (effective_request, trusted_preflight_context) = match preflight {
            Ok(LegacyToolExecutionPreflight::Ready {
                request,
                trusted_internal_context,
            }) => (request, trusted_internal_context),
            Ok(LegacyToolExecutionPreflight::NeedsApproval(requirement)) => {
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
            Err(reason) if LegacyRepairablePreflight::parse(reason.as_str()).is_some() => {
                let stripped =
                    LegacyRepairablePreflight::parse(reason.as_str()).unwrap_or(reason.as_str());
                let human_reason = LegacyRepairablePreflight::render(stripped);
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

        let invocation = match dispatch_kind {
            LegacyToolDispatchKind::LegacyCore => PreparedLegacyToolInvocation::Core {
                request: effective_request,
                trusted_internal_context,
            },
            LegacyToolDispatchKind::LegacyApp => PreparedLegacyToolInvocation::App {
                request: effective_request,
            },
        };
        Ok(PreparedToolIntent {
            intent_sequence,
            intent: effective_intent,
            invocation: PreparedToolInvocation::Legacy {
                invocation,
                decision,
            },
            capability_action_class,
            scheduling_class,
        })
    }
}
