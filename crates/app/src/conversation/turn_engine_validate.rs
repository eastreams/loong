use super::{ProviderTurn, TurnEngine, TurnValidation};

impl TurnEngine {
    /// Classify whether a provider turn is final text or requires tool preparation.
    ///
    /// Tool ownership cannot be validated here: unresolved paths must first be
    /// looked up in Runtime, and only a typed miss may enter legacy validation.
    pub fn classify_turn(&self, turn: &ProviderTurn) -> TurnValidation {
        if turn.tool_intents.is_empty() {
            return TurnValidation::FinalText(turn.assistant_text.clone());
        }
        TurnValidation::ToolExecutionRequired
    }
}
