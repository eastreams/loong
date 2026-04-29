use serde::{Deserialize, Serialize};

const fn default_compact_preserve_recent_turns() -> usize {
    6
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConversationConfig {
    #[serde(default)]
    pub context_engine: Option<String>,
    #[serde(default)]
    pub turn_middlewares: Vec<String>,
    #[serde(default = "default_true")]
    pub compact_enabled: bool,
    #[serde(default)]
    pub compact_min_messages: Option<usize>,
    #[serde(default)]
    pub compact_trigger_estimated_tokens: Option<usize>,
    #[serde(default = "default_compact_preserve_recent_turns")]
    pub compact_preserve_recent_turns: usize,
    #[serde(default)]
    pub compact_preserve_recent_estimated_tokens: Option<usize>,
    #[serde(default = "default_true")]
    pub compact_fail_open: bool,
    #[serde(default)]
    pub turn_loop: ConversationTurnLoopConfig,
}

impl Default for ConversationConfig {
    fn default() -> Self {
        Self {
            context_engine: None,
            turn_middlewares: Vec::new(),
            compact_enabled: default_true(),
            compact_min_messages: None,
            compact_trigger_estimated_tokens: None,
            compact_preserve_recent_turns: default_compact_preserve_recent_turns(),
            compact_preserve_recent_estimated_tokens: None,
            compact_fail_open: default_true(),
            turn_loop: ConversationTurnLoopConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConversationTurnLoopConfig {
    #[serde(default = "default_turn_loop_max_followup_tool_payload_chars")]
    pub max_followup_tool_payload_chars: usize,
    #[serde(default = "default_turn_loop_max_followup_tool_payload_chars_total")]
    pub max_followup_tool_payload_chars_total: usize,
}

impl Default for ConversationTurnLoopConfig {
    fn default() -> Self {
        Self {
            max_followup_tool_payload_chars: default_turn_loop_max_followup_tool_payload_chars(),
            max_followup_tool_payload_chars_total:
                default_turn_loop_max_followup_tool_payload_chars_total(),
        }
    }
}

impl ConversationConfig {
    pub fn context_engine_id(&self) -> Option<String> {
        self.context_engine
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
    }

    pub fn turn_middleware_ids(&self) -> Vec<String> {
        let mut seen = std::collections::BTreeSet::new();
        let mut ids = Vec::new();

        for raw in &self.turn_middlewares {
            let normalized = raw.trim().to_ascii_lowercase();
            if normalized.is_empty() || !seen.insert(normalized.clone()) {
                continue;
            }
            ids.push(normalized);
        }

        ids
    }

    pub fn compact_min_messages(&self) -> Option<usize> {
        self.compact_min_messages.filter(|value| *value > 0)
    }

    pub fn compact_trigger_estimated_tokens(&self) -> Option<usize> {
        self.compact_trigger_estimated_tokens
            .filter(|value| *value > 0)
    }

    pub fn compact_preserve_recent_turns(&self) -> usize {
        self.compact_preserve_recent_turns.max(1)
    }

    pub fn compact_preserve_recent_estimated_tokens(&self) -> Option<usize> {
        self.compact_preserve_recent_estimated_tokens
            .filter(|value| *value > 0)
            .or_else(|| self.compact_trigger_estimated_tokens())
    }

    pub fn should_compact(&self, message_count: usize) -> bool {
        self.should_compact_with_estimate(message_count, None)
    }

    pub fn should_compact_with_estimate(
        &self,
        message_count: usize,
        estimated_tokens: Option<usize>,
    ) -> bool {
        if !self.compact_enabled {
            return false;
        }

        let min_messages = self.compact_min_messages();
        let trigger_tokens = self.compact_trigger_estimated_tokens();

        if min_messages.is_none() && trigger_tokens.is_none() {
            return false;
        }

        let messages_triggered = min_messages.is_some_and(|threshold| message_count >= threshold);
        let tokens_triggered = trigger_tokens
            .zip(estimated_tokens)
            .is_some_and(|(threshold, actual)| actual >= threshold);

        messages_triggered || tokens_triggered
    }

    pub fn compaction_fail_open(&self) -> bool {
        self.compact_fail_open
    }
}

const fn default_true() -> bool {
    true
}

const fn default_turn_loop_max_followup_tool_payload_chars() -> usize {
    8_000
}

const fn default_turn_loop_max_followup_tool_payload_chars_total() -> usize {
    20_000
}
