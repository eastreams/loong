use super::{
    ProviderTurn, SessionContext, ToolView, TurnEngine, TurnFailure, TurnResult, TurnValidation,
    concealed_provider_tool_denial, effective_visible_tool_name, provider_tool_denial_reason,
    provider_tool_denial_should_conceal_name, session_context_from_turn, tool_intent_is_visible,
    tool_intent_skips_provider_exposed_gate,
};
use loong_contracts::ToolCoreRequest;

impl TurnEngine {
    /// Evaluate a provider turn and produce a deterministic result.
    /// Does NOT execute tools — just validates and gates.
    pub fn evaluate_turn(&self, turn: &ProviderTurn) -> TurnResult {
        self.evaluate_turn_in_view(turn, &crate::tools::runtime_tool_view())
    }

    pub fn evaluate_turn_in_view(&self, turn: &ProviderTurn, tool_view: &ToolView) -> TurnResult {
        self.evaluate_turn_in_context(turn, &session_context_from_turn(turn, tool_view.clone()))
    }

    pub fn evaluate_turn_in_context(
        &self,
        turn: &ProviderTurn,
        session_context: &SessionContext,
    ) -> TurnResult {
        match self.validate_turn_in_context(turn, session_context) {
            Ok(TurnValidation::FinalText(text)) => TurnResult::FinalText(text),
            Err(failure) => TurnResult::ToolDenied(failure),
            Ok(TurnValidation::ToolExecutionRequired) => {
                TurnResult::policy_denied("kernel_context_required", "kernel_context_required")
            }
        }
    }

    /// Validate a provider turn and describe whether tool execution is needed.
    ///
    /// This phase is pure: it validates the turn shape and tool budget, but it does
    /// not make runtime binding decisions about whether a kernel is available.
    pub fn validate_turn(&self, turn: &ProviderTurn) -> Result<TurnValidation, TurnFailure> {
        self.validate_turn_in_view(turn, &crate::tools::runtime_tool_view())
    }

    pub fn validate_turn_in_view(
        &self,
        turn: &ProviderTurn,
        tool_view: &ToolView,
    ) -> Result<TurnValidation, TurnFailure> {
        self.validate_turn_in_context(turn, &session_context_from_turn(turn, tool_view.clone()))
    }

    pub fn validate_turn_in_context(
        &self,
        turn: &ProviderTurn,
        session_context: &SessionContext,
    ) -> Result<TurnValidation, TurnFailure> {
        if turn.tool_intents.is_empty() {
            return Ok(TurnValidation::FinalText(turn.assistant_text.clone()));
        }

        let catalog = crate::tools::tool_catalog();
        for intent in &turn.tool_intents {
            let outer_request = ToolCoreRequest {
                tool_name: intent.tool_name.clone(),
                payload: intent.args_json.clone(),
            };
            if let Some(peeked_request) = crate::tools::peek_tool_invoke_request(&outer_request) {
                let Some(descriptor) = catalog.resolve(peeked_request.tool_name) else {
                    let reason = provider_tool_denial_reason(
                        "tool_not_found: tool.invoke",
                        intent.source.as_str(),
                    );
                    return Err(TurnFailure::policy_denied_with_discovery_recovery(
                        "tool_not_found",
                        reason,
                    ));
                };

                if descriptor.is_provider_exposed()
                    || crate::tools::direct_tool_name_for_hidden_tool(descriptor.name).is_some()
                {
                    let reason = provider_tool_denial_reason(
                        "tool_not_found: tool.invoke",
                        intent.source.as_str(),
                    );
                    return Err(TurnFailure::policy_denied_with_discovery_recovery(
                        "tool_not_found",
                        reason,
                    ));
                }

                if !session_context.tool_view.contains(descriptor.name) {
                    return Err(concealed_provider_tool_denial());
                }

                continue;
            }

            let Some(resolved_tool) = crate::tools::resolve_tool_execution(&intent.tool_name)
            else {
                let raw_reason = format!("tool_not_found: {}", intent.tool_name);
                let reason =
                    provider_tool_denial_reason(raw_reason.as_str(), intent.source.as_str());
                let failure = if intent.source.starts_with("provider_") {
                    TurnFailure::policy_denied_with_discovery_recovery("tool_not_found", reason)
                } else {
                    TurnFailure::policy_denied("tool_not_found", reason)
                };
                return Err(failure);
            };

            if let Some(descriptor) = catalog.resolve(&intent.tool_name) {
                let tool_is_visible = tool_intent_is_visible(session_context, intent, descriptor);
                if !tool_is_visible {
                    if provider_tool_denial_should_conceal_name(intent, descriptor, false) {
                        return Err(concealed_provider_tool_denial());
                    }
                    let reason = format!(
                        "tool_not_visible: {}",
                        effective_visible_tool_name(intent, descriptor)
                    );
                    return Err(TurnFailure::policy_denied("tool_not_visible", reason));
                }

                if provider_tool_denial_should_conceal_name(intent, descriptor, true) {
                    return Err(concealed_provider_tool_denial());
                }

                if tool_intent_skips_provider_exposed_gate(intent, descriptor) {
                    // Lease validation happens in resolve_tool_invoke_request during execution.
                    // Internal approval-control turns also bypass provider exposure checks for
                    // the approval tools they synthesize.
                } else if !crate::tools::is_provider_exposed_tool_name(&intent.tool_name) {
                    let reason = format!("tool_not_provider_exposed: {}", intent.tool_name);
                    return Err(TurnFailure::policy_denied(
                        "tool_not_provider_exposed",
                        reason,
                    ));
                }
            } else {
                if intent.source.starts_with("provider_") {
                    return Err(concealed_provider_tool_denial());
                }
                if !session_context
                    .tool_view
                    .contains(resolved_tool.canonical_name)
                {
                    let reason = format!("tool_not_visible: {}", intent.tool_name);
                    return Err(TurnFailure::policy_denied("tool_not_visible", reason));
                }
            }
        }

        Ok(TurnValidation::ToolExecutionRequired)
    }
}
