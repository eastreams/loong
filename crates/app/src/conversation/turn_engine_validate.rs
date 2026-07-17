#[cfg(test)]
use super::ToolView;
use super::{
    AppContext, ProviderTurn, TurnEngine, TurnFailure, TurnResult, TurnValidation,
    concealed_provider_tool_denial, effective_visible_tool_name, provider_tool_denial_reason,
    provider_tool_denial_should_conceal_name, tool_intent_is_visible,
    tool_intent_skips_provider_exposed_gate,
};
use loong_runtime::tool_plane::{ToolPath, error::LookupError};

impl TurnEngine {
    #[cfg(test)]
    pub fn evaluate_turn(&self, turn: &ProviderTurn) -> TurnResult {
        let context = crate::test_support::app_context_for_session(
            "turn-validation",
            crate::tools::runtime_tool_view(),
        );
        self.evaluate_turn_in_context(turn, &context)
    }

    #[cfg(test)]
    pub fn evaluate_turn_in_view(&self, turn: &ProviderTurn, tool_view: &ToolView) -> TurnResult {
        let context =
            crate::test_support::app_context_for_session("turn-validation", tool_view.clone());
        self.evaluate_turn_in_context(turn, &context)
    }

    pub fn evaluate_turn_in_context(
        &self,
        turn: &ProviderTurn,
        session_context: &AppContext,
    ) -> TurnResult {
        match self.validate_turn_in_context(turn, session_context) {
            Ok(TurnValidation::FinalText(text)) => TurnResult::FinalText(text),
            Err(failure) => TurnResult::ToolDenied(failure),
            Ok(TurnValidation::ToolExecutionRequired) => {
                TurnResult::policy_denied("app_context_required", "app_context_required")
            }
        }
    }

    #[cfg(test)]
    pub fn validate_turn(&self, turn: &ProviderTurn) -> Result<TurnValidation, TurnFailure> {
        let context = crate::test_support::app_context_for_session(
            "turn-validation",
            crate::tools::runtime_tool_view(),
        );
        self.validate_turn_in_context(turn, &context)
    }

    pub fn validate_turn_in_context(
        &self,
        turn: &ProviderTurn,
        session_context: &AppContext,
    ) -> Result<TurnValidation, TurnFailure> {
        if turn.tool_intents.is_empty() {
            return Ok(TurnValidation::FinalText(turn.assistant_text.clone()));
        }

        let catalog = crate::tools::tool_catalog();
        for intent in &turn.tool_intents {
            let canonical_tool_name = crate::tools::canonical_tool_name(intent.tool_name.as_str());
            let typed_path = ToolPath::from(canonical_tool_name);
            let typed_registered = match session_context.runtime().tool_spec(&typed_path) {
                Ok(_) => true,
                Err(LookupError::NotRegistered { .. }) => false,
                Err(error) => {
                    return Err(TurnFailure::non_retryable(
                        "tool_registry_failed",
                        error.to_string(),
                    ));
                }
            };
            if canonical_tool_name == "tool.invoke" && !typed_registered {
                // An unregistered envelope is not authority of its own. Lease
                // validation, inner lookup, and target visibility stay together
                // in preparation so invalid leases cannot probe registry state.
                continue;
            }
            let legacy_execution = if typed_registered {
                None
            } else {
                crate::tools::resolve_legacy_tool_execution(&intent.tool_name)
            };
            if !typed_registered && legacy_execution.is_none() {
                let raw_reason = format!("tool_not_found: {}", intent.tool_name);
                let reason =
                    provider_tool_denial_reason(raw_reason.as_str(), intent.source.as_str());
                let failure = if intent.source.starts_with("provider_") {
                    TurnFailure::policy_denied_with_discovery_recovery("tool_not_found", reason)
                } else {
                    TurnFailure::policy_denied("tool_not_found", reason)
                };
                return Err(failure);
            }
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
            } else if typed_registered {
                if intent.source.starts_with("provider_") {
                    return Err(concealed_provider_tool_denial());
                }
                if !session_context.tool_view.contains(canonical_tool_name) {
                    let reason = format!("tool_not_visible: {}", intent.tool_name);
                    return Err(TurnFailure::policy_denied("tool_not_visible", reason));
                }
            } else if let Some(resolved_tool) = legacy_execution {
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
