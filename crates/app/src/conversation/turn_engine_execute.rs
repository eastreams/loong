use std::sync::Arc;

use crate::tools::runtime_events::{
    ToolRuntimeEvent, ToolRuntimeEventSink, with_tool_runtime_event_sink,
};
#[cfg(test)]
use crate::tools::runtime_tool_view;
use loong_core::error::PolicyGrantError;
use loong_runtime::tool_plane::{RegisteredToolError, error::ToolInvocationError};

use super::prepare::{
    PreparedLegacyToolInvocation, PreparedToolIntent, PreparedToolInvocation,
    PreparedTypedToolInvocation,
};
use super::*;

struct ObserverToolRuntimeEventSink {
    observer: ConversationTurnObserverHandle,
    tool_call_id: String,
}

impl ToolRuntimeEventSink for ObserverToolRuntimeEventSink {
    fn emit(&self, event: ToolRuntimeEvent) {
        let runtime_event = ConversationTurnRuntimeEvent::new(self.tool_call_id.clone(), event);
        self.observer.on_runtime(runtime_event);
    }
}

/// Classify failures after orchestration has selected the legacy ingress.
///
/// Typed invocation failures never cross this conversion; they retain their
/// runtime-owned sources in the separate conversion below.
impl From<crate::tools::LegacyToolRequestError> for TurnFailure {
    fn from(error: crate::tools::LegacyToolRequestError) -> Self {
        match error {
            crate::tools::LegacyToolRequestError::RuntimeMismatch => TurnFailure::non_retryable(
                "legacy_runtime_mismatch",
                "legacy dispatcher belongs to another Runtime",
            ),
            crate::tools::LegacyToolRequestError::Legacy(error) => {
                if let KernelError::ToolPlane(ToolPlaneError::Execution(reason)) = &error
                    && let Some(stripped) = LegacyRepairablePreflight::parse(reason.as_str())
                {
                    let human_reason = LegacyRepairablePreflight::render(stripped);
                    return TurnFailure::retryable("tool_preflight_denied", human_reason);
                }

                let reason = render_kernel_error_reason(&error);
                match classify_kernel_error(&error) {
                    KernelFailureClass::PolicyDenied => {
                        TurnFailure::policy_denied("kernel_policy_denied", reason)
                    }
                    KernelFailureClass::RetryableExecution => {
                        TurnFailure::retryable("tool_execution_failed", reason)
                    }
                    KernelFailureClass::NonRetryable => {
                        TurnFailure::non_retryable("kernel_execution_failed", reason)
                    }
                }
            }
            crate::tools::LegacyToolRequestError::Input(reason) => {
                TurnFailure::retryable("tool_input_invalid", reason)
            }
            crate::tools::LegacyToolRequestError::ReservedContext(reason) => {
                TurnFailure::policy_denied("tool_context_denied", reason)
            }
        }
    }
}

/// Project a typed invocation failure into the turn scheduler's retry/deny model.
///
/// This is the app orchestration boundary: it preserves typed classification
/// long enough to decide batch continuation without converting through a
/// legacy `KernelError` or string classifier.
impl TurnFailure {
    fn from_typed_tool_invocation(
        error: ToolInvocationError,
        path: loong_contracts::ToolPath,
        provider_name: String,
        argument_hint: Option<String>,
    ) -> Self {
        let reason = error.to_string();
        match error {
            ToolInvocationError::CapabilityOverride(_) => {
                TurnFailure::policy_denied("tool_capability_override_denied", reason)
            }
            ToolInvocationError::Authorization(
                PolicyGrantError::MissingCapability { .. }
                | PolicyGrantError::Denied { .. }
                | PolicyGrantError::PermissionDenied { .. },
            ) => TurnFailure::policy_denied("tool_authorization_denied", reason),
            ToolInvocationError::Dispatch {
                source: RegisteredToolError::Denied { .. },
                ..
            } => TurnFailure::policy_denied("tool_execution_denied", reason),
            ToolInvocationError::Dispatch {
                source: RegisteredToolError::Input(error),
                ..
            } => TurnFailure::input_repair_required(
                reason,
                ToolInputFailure {
                    path,
                    provider_name,
                    argument_hint,
                    error,
                },
            ),
            ToolInvocationError::CapabilityNarrowing(_) => {
                TurnFailure::non_retryable("tool_capability_narrowing_failed", reason)
            }
            ToolInvocationError::Authorization(_) => {
                TurnFailure::non_retryable("tool_authorization_failed", reason)
            }
            ToolInvocationError::CapabilityOverrideAndAudit { .. }
            | ToolInvocationError::StartAudit { .. }
            | ToolInvocationError::CompletedAudit { .. }
            | ToolInvocationError::DispatchAndAudit { .. } => {
                TurnFailure::non_retryable("tool_execution_audit_failed", reason)
            }
            ToolInvocationError::Dispatch { .. } => {
                TurnFailure::non_retryable("tool_execution_failed", reason)
            }
        }
    }
}

impl TurnEngine {
    pub(crate) async fn execute_turn_in_context<D: LegacyToolDispatcher + ?Sized>(
        &self,
        turn: &ProviderTurn,
        session_context: &Context<'_>,
        legacy_dispatcher: &D,
        ingress: Option<&ConversationIngressContext>,
    ) -> TurnResult {
        self.execute_turn_in_context_with_trace(
            turn,
            session_context,
            legacy_dispatcher,
            ingress,
            None,
        )
        .await
        .0
    }

    pub(crate) async fn execute_turn_in_context_with_trace<D: LegacyToolDispatcher + ?Sized>(
        &self,
        turn: &ProviderTurn,
        session_context: &Context<'_>,
        legacy_dispatcher: &D,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> (TurnResult, Option<ToolBatchExecutionTrace>) {
        match self.classify_turn(turn) {
            TurnValidation::FinalText(text) => return (TurnResult::FinalText(text), None),
            TurnValidation::ToolExecutionRequired => {}
        }

        let tool_batch_harness = ToolBatchHarness::new(self);
        let mut trace = tool_batch_harness.trace_empty_batch(turn.tool_intents.len());
        let mut prepared = Vec::new();
        let mut autonomy_budget_state = AutonomyTurnBudgetState::default();
        for (intent_sequence, intent) in turn.tool_intents.iter().enumerate() {
            match self
                .prepare_tool_intent(
                    intent,
                    intent_sequence,
                    session_context,
                    legacy_dispatcher,
                    &autonomy_budget_state,
                    ingress,
                )
                .await
            {
                Ok(prepared_intent) => {
                    if let PreparedToolInvocation::Legacy { decision, .. } =
                        &prepared_intent.invocation
                    {
                        let decision_record = build_tool_decision_trace_record(
                            &prepared_intent.intent,
                            decision.clone(),
                        );
                        trace.decision_records.push(decision_record);
                    }
                    autonomy_budget_state.record_action(prepared_intent.capability_action_class);
                    prepared.push(prepared_intent);
                }
                Err(failure) => {
                    let decision_record =
                        build_tool_decision_trace_record(&failure.intent, failure.decision);
                    trace.decision_records.push(decision_record);
                    let intent_outcome =
                        build_tool_intent_failure_trace(&failure.intent, &failure.turn_result);
                    if let Some(intent_outcome) = intent_outcome {
                        trace.intent_outcomes.push(intent_outcome);
                    }
                    return (failure.turn_result, Some(trace));
                }
            }
        }
        let batch_segments = tool_batch_harness.prepared_batch_segments(&prepared);
        tool_batch_harness.populate_trace_segments(&mut trace, &batch_segments);

        let outputs = match tool_batch_harness
            .execute_prepared_batch(
                prepared,
                &batch_segments,
                session_context,
                legacy_dispatcher,
                &mut trace,
                observer,
            )
            .await
        {
            Ok(outputs) => outputs,
            Err(result) => return (result, Some(trace)),
        };

        (TurnResult::FinalText(outputs.join("\n")), Some(trace))
    }
}

impl<'a> PreparedTypedToolInvocation<'a> {
    /// Consume the entry already resolved during preparation.
    pub(super) async fn execute(
        self,
        intent: &ToolIntent,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> PreparedToolExecutionOutcome {
        let path = self.invocation.path().clone();
        let provider_name = intent.tool_name().to_owned();
        let argument_hint = self.invocation.spec().argument_hint.clone();
        let execution = async {
            match self.invocation.invoke(self.payload).await {
                Ok(payload) => PreparedToolExecutionOutcome::Completed {
                    status: "ok".to_owned(),
                    payload,
                },
                Err(error) => {
                    let failure = TurnFailure::from_typed_tool_invocation(
                        error,
                        path,
                        provider_name,
                        argument_hint,
                    );
                    if failure.kind == TurnFailureKind::PolicyDenied {
                        PreparedToolExecutionOutcome::Denied(failure)
                    } else {
                        PreparedToolExecutionOutcome::Interrupted(failure.into_turn_result())
                    }
                }
            }
        };

        match observer {
            Some(observer) => {
                let sink: Arc<dyn ToolRuntimeEventSink> = Arc::new(ObserverToolRuntimeEventSink {
                    observer: Arc::clone(observer),
                    tool_call_id: intent.tool_call_id.clone(),
                });
                with_tool_runtime_event_sink(sink, execution).await
            }
            None => execution.await,
        }
    }
}

impl<'a> PreparedToolIntent<'a> {
    /// Consume one prepared owner: typed entries stay bound, while only the
    /// explicit legacy variant may call the bearer-backed dispatcher.
    pub(super) async fn execute<D: LegacyToolDispatcher + ?Sized>(
        self,
        engine: &TurnEngine,
        session_context: &Context<'_>,
        legacy_dispatcher: &D,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> (ToolIntent, PreparedToolExecutionOutcome) {
        let Self {
            intent_sequence,
            intent,
            invocation,
            ..
        } = self;
        let outcome = match invocation {
            PreparedToolInvocation::Typed(typed) => typed.execute(&intent, observer).await,
            PreparedToolInvocation::Legacy {
                invocation: legacy, ..
            } => {
                engine
                    .execute_legacy_tool_invocation(
                        &legacy,
                        &intent,
                        intent_sequence,
                        session_context,
                        legacy_dispatcher,
                        observer,
                    )
                    .await
            }
        };
        (intent, outcome)
    }
}

impl TurnEngine {
    /// Execute only an owner-selected legacy fallback with its bearer token.
    pub(super) async fn execute_legacy_tool_invocation<D: LegacyToolDispatcher + ?Sized>(
        &self,
        prepared: &PreparedLegacyToolInvocation,
        intent: &ToolIntent,
        intent_sequence: usize,
        session_context: &Context<'_>,
        legacy_dispatcher: &D,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> PreparedToolExecutionOutcome {
        let execution = async {
            match prepared {
                PreparedLegacyToolInvocation::Core {
                    request,
                    trusted_internal_context,
                } => match legacy_dispatcher
                    .execute_core_tool(session_context, request.clone(), *trusted_internal_context)
                    .await
                {
                    Ok(outcome) => {
                        legacy_dispatcher
                            .after_tool_execution(
                                session_context,
                                intent,
                                intent_sequence,
                                request,
                                &outcome,
                            )
                            .await;
                        let ToolCoreOutcome { status, payload } = outcome;
                        PreparedToolExecutionOutcome::Completed { status, payload }
                    }
                    Err(error) => {
                        let failure = TurnFailure::from(error);
                        if failure.kind == TurnFailureKind::PolicyDenied {
                            PreparedToolExecutionOutcome::Denied(failure)
                        } else {
                            PreparedToolExecutionOutcome::Interrupted(failure.into_turn_result())
                        }
                    }
                },
                PreparedLegacyToolInvocation::App { request } => {
                    match legacy_dispatcher
                        .execute_app_tool(session_context, request.clone())
                        .await
                    {
                        Ok(outcome) => {
                            legacy_dispatcher
                                .after_tool_execution(
                                    session_context,
                                    intent,
                                    intent_sequence,
                                    request,
                                    &outcome,
                                )
                                .await;
                            let ToolCoreOutcome { status, payload } = outcome;
                            PreparedToolExecutionOutcome::Completed { status, payload }
                        }
                        Err(reason) if reason.starts_with("tool_not_visible:") => {
                            PreparedToolExecutionOutcome::Denied(TurnFailure::policy_denied(
                                "tool_not_visible",
                                reason,
                            ))
                        }
                        Err(reason)
                            if reason.starts_with("tool_not_found:")
                                || reason.starts_with("app_tool_not_found:") =>
                        {
                            PreparedToolExecutionOutcome::Denied(TurnFailure::policy_denied(
                                "tool_not_found",
                                reason,
                            ))
                        }
                        Err(reason) if reason.starts_with("app_tool_disabled:") => {
                            PreparedToolExecutionOutcome::Denied(TurnFailure::policy_denied(
                                "app_tool_disabled",
                                reason,
                            ))
                        }
                        Err(reason) if reason.starts_with("app_tool_denied:") => {
                            let human_reason = render_app_tool_denied_reason(reason.as_str());
                            PreparedToolExecutionOutcome::Denied(TurnFailure::policy_denied(
                                "app_tool_denied",
                                human_reason,
                            ))
                        }
                        Err(reason) => PreparedToolExecutionOutcome::Interrupted(
                            TurnResult::non_retryable_tool_error(
                                "app_tool_execution_failed",
                                reason,
                            ),
                        ),
                    }
                }
            }
        };

        match observer {
            Some(observer) => {
                let sink: Arc<dyn ToolRuntimeEventSink> = Arc::new(ObserverToolRuntimeEventSink {
                    observer: Arc::clone(observer),
                    tool_call_id: intent.tool_call_id.clone(),
                });
                with_tool_runtime_event_sink(sink, execution).await
            }
            None => execution.await,
        }
    }
}

#[cfg(test)]
mod execution_tests {
    use super::*;
    use serde_json::json;

    struct MissingProviderLegacyToolDispatcher;

    #[async_trait::async_trait]
    impl LegacyToolDispatcher for MissingProviderLegacyToolDispatcher {
        async fn execute_core_tool(
            &self,
            _session_context: &Context<'_>,
            request: ToolCoreRequest,
            _trusted_internal_context: bool,
        ) -> Result<ToolCoreOutcome, crate::tools::LegacyToolRequestError> {
            Err(crate::tools::LegacyToolRequestError::Input(format!(
                "unexpected core tool: {}",
                request.tool_name
            )))
        }

        async fn execute_app_tool(
            &self,
            _session_context: &Context<'_>,
            request: ToolCoreRequest,
        ) -> Result<ToolCoreOutcome, String> {
            Err(format!("app_tool_not_found: {}", request.tool_name))
        }
    }

    #[tokio::test]
    async fn provider_app_tool_not_found_is_plain_policy_denial() {
        let session_id = "provider-app-tool-not-found";
        let turn_id = "turn-provider-app-tool-not-found";
        let prepared_intent = PreparedToolIntent {
            intent_sequence: 0,
            intent: ToolIntent {
                tool_name: "sessions_list".into(),
                args_json: json!({}),
                source: "provider_tool_call".to_owned(),
                turn_id: turn_id.to_owned(),
                tool_call_id: "call-provider-app-tool-not-found".to_owned(),
            },
            invocation: PreparedToolInvocation::Legacy {
                invocation: PreparedLegacyToolInvocation::App {
                    request: ToolCoreRequest {
                        tool_name: "sessions_list".to_owned(),
                        payload: json!({}),
                    },
                },
                decision: ToolDecisionTelemetry::allow(
                    "sessions_list",
                    "prepared for execution",
                    "test_allow",
                ),
            },
            capability_action_class: crate::tools::CapabilityActionClass::ExecuteExisting,
            scheduling_class: loong_contracts::ToolSchedulingClass::SerialOnly,
        };
        let owner = crate::test_support::runtime_session_for_test(session_id, runtime_tool_view());
        let session_context = owner.context();

        let result = TurnEngine::new(4)
            .execute_legacy_tool_invocation(
                match &prepared_intent.invocation {
                    PreparedToolInvocation::Legacy { invocation, .. } => invocation,
                    PreparedToolInvocation::Typed(_) => panic!("expected legacy invocation"),
                },
                &prepared_intent.intent,
                prepared_intent.intent_sequence,
                &session_context,
                &MissingProviderLegacyToolDispatcher,
                None,
            )
            .await;

        let PreparedToolExecutionOutcome::Denied(failure) = result else {
            panic!("expected tool denial");
        };
        assert_eq!(failure.code, "tool_not_found");
        assert!(!failure.supports_discovery_recovery);
        assert!(
            failure.reason.starts_with("app_tool_not_found:"),
            "unexpected reason: {}",
            failure.reason
        );
    }
}
