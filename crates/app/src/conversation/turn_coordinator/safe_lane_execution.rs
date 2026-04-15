use super::*;

pub(super) struct SafeLanePlanNodeExecutor<'a> {
    pub(super) tool_intents: &'a [ToolIntent],
    pub(super) session_context: &'a SessionContext,
    pub(super) app_dispatcher: &'a dyn AppToolDispatcher,
    pub(super) binding: ConversationRuntimeBinding<'a>,
    pub(super) ingress: Option<&'a ConversationIngressContext>,
    pub(super) verify_output_non_empty: bool,
    pub(super) tool_outputs: Mutex<Vec<String>>,
    pub(super) tool_result_payload_summary_limit_chars: usize,
}

impl<'a> SafeLanePlanNodeExecutor<'a> {
    pub(super) fn new(
        tool_intents: &'a [ToolIntent],
        session_context: &'a SessionContext,
        app_dispatcher: &'a dyn AppToolDispatcher,
        binding: ConversationRuntimeBinding<'a>,
        ingress: Option<&'a ConversationIngressContext>,
        verify_output_non_empty: bool,
        seed_tool_outputs: Vec<String>,
        tool_result_payload_summary_limit_chars: usize,
    ) -> Self {
        Self {
            tool_intents,
            session_context,
            app_dispatcher,
            binding,
            ingress,
            verify_output_non_empty,
            tool_outputs: Mutex::new(seed_tool_outputs),
            tool_result_payload_summary_limit_chars,
        }
    }

    pub(super) async fn tool_outputs_snapshot(&self) -> Vec<String> {
        self.tool_outputs.lock().await.clone()
    }
}

#[async_trait]
impl PlanNodeExecutor for SafeLanePlanNodeExecutor<'_> {
    async fn execute(&self, node: &PlanNode, _attempt: u8) -> Result<(), PlanNodeError> {
        match node.kind {
            PlanNodeKind::Tool => {
                let index = parse_tool_node_index(node.id.as_str())?;
                let intent = self.tool_intents.get(index).ok_or_else(|| {
                    PlanNodeError::non_retryable(format!(
                        "missing tool intent for node `{}`",
                        node.id
                    ))
                })?;
                let output = execute_single_tool_intent(
                    intent,
                    self.session_context,
                    self.app_dispatcher,
                    self.binding,
                    self.ingress,
                    self.tool_result_payload_summary_limit_chars,
                )
                .await?;
                self.tool_outputs.lock().await.push(output);
                Ok(())
            }
            PlanNodeKind::Verify => {
                if !self.verify_output_non_empty {
                    return Ok(());
                }
                let outputs = self.tool_outputs.lock().await;
                if outputs.is_empty() || outputs.iter().any(|line| line.trim().is_empty()) {
                    return Err(PlanNodeError::non_retryable(
                        "verify_failed:empty_tool_output".to_owned(),
                    ));
                }
                Ok(())
            }
            PlanNodeKind::Transform | PlanNodeKind::Respond => Ok(()),
        }
    }
}

pub(super) fn parse_tool_node_index(node_id: &str) -> Result<usize, PlanNodeError> {
    let suffix = node_id
        .strip_prefix("tool-")
        .ok_or_else(|| PlanNodeError::non_retryable(format!("invalid tool node id `{node_id}`")))?;
    let parsed = suffix.parse::<usize>().map_err(|error| {
        PlanNodeError::non_retryable(format!("invalid tool node id `{node_id}`: {error}"))
    })?;
    if parsed == 0 {
        return Err(PlanNodeError::non_retryable(format!(
            "invalid tool node ordinal in `{node_id}`"
        )));
    }
    Ok(parsed - 1)
}

pub(super) async fn execute_single_tool_intent(
    intent: &ToolIntent,
    session_context: &SessionContext,
    app_dispatcher: &dyn AppToolDispatcher,
    binding: ConversationRuntimeBinding<'_>,
    ingress: Option<&ConversationIngressContext>,
    payload_summary_limit_chars: usize,
) -> Result<String, PlanNodeError> {
    let engine = TurnEngine::with_tool_result_payload_summary_limit(1, payload_summary_limit_chars);
    let turn = ProviderTurn {
        assistant_text: String::new(),
        tool_intents: vec![intent.clone()],
        raw_meta: Value::Null,
    };

    match engine
        .execute_turn_in_context(&turn, session_context, app_dispatcher, binding, ingress)
        .await
    {
        TurnResult::FinalText(output) => Ok(output),
        TurnResult::StreamingText(text) => Ok(text),
        TurnResult::StreamingDone(text) => Ok(text),
        TurnResult::NeedsApproval(requirement) => Err(PlanNodeError::policy_denied(
            format_approval_required_reply("", &requirement),
        )),
        TurnResult::ToolDenied(failure) => Err(PlanNodeError::policy_denied(failure.reason)),
        TurnResult::ToolError(failure) => Err(PlanNodeError {
            kind: match failure.kind {
                TurnFailureKind::Retryable => PlanNodeErrorKind::Retryable,
                TurnFailureKind::PolicyDenied
                | TurnFailureKind::NonRetryable
                | TurnFailureKind::Provider => PlanNodeErrorKind::NonRetryable,
            },
            message: failure.reason,
        }),
        TurnResult::ProviderError(failure) => Err(PlanNodeError {
            kind: PlanNodeErrorKind::NonRetryable,
            message: failure.reason,
        }),
    }
}
