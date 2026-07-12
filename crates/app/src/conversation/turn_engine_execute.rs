use std::sync::Arc;

use crate::tools::runtime_events::{
    ToolRuntimeEvent, ToolRuntimeEventSink, with_tool_runtime_event_sink,
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

fn build_observer_tool_runtime_event_sink(
    observer: &ConversationTurnObserverHandle,
    tool_call_id: &str,
) -> Arc<dyn ToolRuntimeEventSink> {
    let observer_sink = ObserverToolRuntimeEventSink {
        observer: Arc::clone(observer),
        tool_call_id: tool_call_id.to_owned(),
    };
    Arc::new(observer_sink)
}

async fn execute_tool_intent_via_kernel(
    request: ToolCoreRequest,
    kernel_ctx: &KernelContext,
    trusted_internal_context: bool,
) -> Result<ToolCoreOutcome, TurnFailure> {
    crate::tools::execute_kernel_tool_request(kernel_ctx, request, trusted_internal_context)
        .await
        .map_err(|error| {
            if let KernelError::ToolPlane(ToolPlaneError::Execution(reason)) = &error
                && let Some(stripped) = RepairableToolPreflight::parse(reason.as_str())
            {
                let human_reason = RepairableToolPreflight::render(stripped);
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
        })
}

pub(super) fn session_context_from_turn(
    turn: &ProviderTurn,
    tool_view: ToolView,
) -> SessionContext {
    let session_id = turn
        .tool_intents
        .first()
        .map(|intent| intent.session_id.as_str())
        .unwrap_or("default");
    SessionContext::root_with_tool_view(session_id, tool_view)
}

impl TurnEngine {
    fn tool_batch_harness(&self) -> ToolBatchHarness<'_> {
        ToolBatchHarness::new(self)
    }

    pub async fn execute_turn(
        &self,
        turn: &ProviderTurn,
        kernel_ctx: &KernelContext,
    ) -> TurnResult {
        self.execute_turn_in_view(
            turn,
            &runtime_tool_view(),
            ConversationRuntimeBinding::kernel(kernel_ctx),
        )
        .await
    }

    pub async fn execute_turn_in_view(
        &self,
        turn: &ProviderTurn,
        tool_view: &ToolView,
        binding: ConversationRuntimeBinding<'_>,
    ) -> TurnResult {
        self.execute_turn_in_context(
            turn,
            &session_context_from_turn(turn, tool_view.clone()),
            &DefaultAppToolDispatcher::runtime(),
            binding,
            None,
        )
        .await
    }

    pub async fn execute_turn_with_ingress(
        &self,
        turn: &ProviderTurn,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
    ) -> TurnResult {
        self.execute_turn_in_context(
            turn,
            &session_context_from_turn(turn, runtime_tool_view()),
            &DefaultAppToolDispatcher::runtime(),
            binding,
            ingress,
        )
        .await
    }

    pub async fn execute_turn_in_context<D: AppToolDispatcher + ?Sized>(
        &self,
        turn: &ProviderTurn,
        session_context: &SessionContext,
        app_dispatcher: &D,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
    ) -> TurnResult {
        self.execute_turn_in_context_with_trace(
            turn,
            session_context,
            app_dispatcher,
            binding,
            ingress,
            None,
        )
        .await
        .0
    }

    pub(crate) async fn execute_turn_in_context_with_trace<D: AppToolDispatcher + ?Sized>(
        &self,
        turn: &ProviderTurn,
        session_context: &SessionContext,
        app_dispatcher: &D,
        binding: ConversationRuntimeBinding<'_>,
        ingress: Option<&ConversationIngressContext>,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> (TurnResult, Option<ToolBatchExecutionTrace>) {
        match self.validate_turn_in_context(turn, session_context) {
            Ok(TurnValidation::FinalText(text)) => return (TurnResult::FinalText(text), None),
            Err(failure) => return (TurnResult::ToolDenied(failure), None),
            Ok(TurnValidation::ToolExecutionRequired) => {}
        }

        let tool_batch_harness = self.tool_batch_harness();
        let mut trace = tool_batch_harness.trace_empty_batch(turn.tool_intents.len());
        let mut prepared = Vec::new();
        let mut autonomy_budget_state = AutonomyTurnBudgetState::default();
        for (intent_sequence, intent) in turn.tool_intents.iter().enumerate() {
            match self
                .prepare_tool_intent(
                    intent,
                    intent_sequence,
                    session_context,
                    app_dispatcher,
                    binding,
                    &autonomy_budget_state,
                    ingress,
                )
                .await
            {
                Ok(prepared_intent) => {
                    let decision_record = build_tool_decision_trace_record(
                        &prepared_intent.intent,
                        prepared_intent.decision.clone(),
                    );
                    trace.decision_records.push(decision_record);
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
                &prepared,
                &batch_segments,
                session_context,
                app_dispatcher,
                binding,
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

    pub(super) async fn prepare_tool_intent<D: AppToolDispatcher + ?Sized>(
        &self,
        intent: &ToolIntent,
        intent_sequence: usize,
        session_context: &SessionContext,
        app_dispatcher: &D,
        binding: ConversationRuntimeBinding<'_>,
        budget_state: &AutonomyTurnBudgetState,
        ingress: Option<&ConversationIngressContext>,
    ) -> Result<PreparedToolIntent, PreparedToolIntentFailure> {
        let memory_config = app_dispatcher
            .memory_config()
            .unwrap_or(store::current_session_store_config());
        let preparation_harness = ToolIntentPreparationHarness::new(
            session_context,
            memory_config,
            app_dispatcher,
            binding,
            budget_state,
            ingress,
        );
        preparation_harness.prepare(intent, intent_sequence).await
    }

    pub(super) async fn execute_prepared_tool_intent<D: AppToolDispatcher + ?Sized>(
        &self,
        prepared_intent: &PreparedToolIntent,
        session_context: &SessionContext,
        app_dispatcher: &D,
        binding: ConversationRuntimeBinding<'_>,
        observer: Option<&ConversationTurnObserverHandle>,
    ) -> PreparedToolExecutionOutcome {
        match prepared_intent.execution_kind {
            ToolExecutionKind::Core => {
                let Some(kernel_ctx) = binding.kernel_context() else {
                    return PreparedToolExecutionOutcome::Interrupted(TurnResult::policy_denied(
                        "no_kernel_context",
                        "no_kernel_context",
                    ));
                };
                let execution = execute_tool_intent_via_kernel(
                    prepared_intent.request.clone(),
                    kernel_ctx,
                    prepared_intent.trusted_internal_context,
                );
                let outcome = match observer {
                    Some(observer) => {
                        let sink = build_observer_tool_runtime_event_sink(
                            observer,
                            prepared_intent.intent.tool_call_id.as_str(),
                        );

                        with_tool_runtime_event_sink(sink, execution).await
                    }
                    None => execution.await,
                };

                match outcome {
                    Ok(outcome) => PreparedToolExecutionOutcome::Completed(outcome),
                    Err(failure) if failure.kind == TurnFailureKind::PolicyDenied => {
                        PreparedToolExecutionOutcome::Denied(failure)
                    }
                    Err(failure) => PreparedToolExecutionOutcome::Interrupted(
                        turn_result_from_tool_execution_failure(failure),
                    ),
                }
            }
            ToolExecutionKind::App => match app_dispatcher
                .execute_app_tool(session_context, prepared_intent.request.clone(), binding)
                .await
            {
                Ok(outcome) => PreparedToolExecutionOutcome::Completed(outcome),
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
                    let policy_reason = provider_tool_denial_reason(
                        reason.as_str(),
                        prepared_intent.intent.source.as_str(),
                    );
                    let failure = TurnFailure::policy_denied("tool_not_found", policy_reason);
                    PreparedToolExecutionOutcome::Denied(failure)
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
                    TurnResult::non_retryable_tool_error("app_tool_execution_failed", reason),
                ),
            },
        }
    }
}

#[cfg(test)]
mod execution_tests {
    use super::*;
    use serde_json::json;

    struct MissingProviderAppToolDispatcher;

    #[async_trait::async_trait]
    impl AppToolDispatcher for MissingProviderAppToolDispatcher {
        async fn execute_app_tool(
            &self,
            _session_context: &SessionContext,
            request: ToolCoreRequest,
            _binding: ConversationRuntimeBinding<'_>,
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
                tool_name: "sessions_list".to_owned(),
                args_json: json!({}),
                source: "provider_tool_call".to_owned(),
                session_id: session_id.to_owned(),
                turn_id: turn_id.to_owned(),
                tool_call_id: "call-provider-app-tool-not-found".to_owned(),
            },
            request: ToolCoreRequest {
                tool_name: "sessions_list".to_owned(),
                payload: json!({}),
            },
            execution_kind: ToolExecutionKind::App,
            capability_action_class: crate::tools::CapabilityActionClass::ExecuteExisting,
            scheduling_class: crate::tools::ToolSchedulingClass::SerialOnly,
            trusted_internal_context: false,
            decision: ToolDecisionTelemetry::allow(
                "sessions_list",
                "prepared for execution",
                "test_allow",
            ),
        };
        let session_context = SessionContext::root_with_tool_view(session_id, runtime_tool_view());

        let result = TurnEngine::new(4)
            .execute_prepared_tool_intent(
                &prepared_intent,
                &session_context,
                &MissingProviderAppToolDispatcher,
                ConversationRuntimeBinding::advisory_only(),
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
