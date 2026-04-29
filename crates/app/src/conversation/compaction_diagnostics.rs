use serde::{Deserialize, Serialize};

use super::compaction_preparation::CompactionPreparationDiagnostics;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextCompactionDiagnostics {
    pub summary_turn_count: usize,
    pub retained_turn_count: usize,
    pub demoted_recent_turn_count: usize,
    pub total_turns: usize,
    pub assistant_turns: usize,
    pub low_signal_turns: usize,
    pub tool_result_line_prunes: usize,
    pub tool_outcome_record_prunes: usize,
}

impl From<CompactionPreparationDiagnostics> for ContextCompactionDiagnostics {
    fn from(value: CompactionPreparationDiagnostics) -> Self {
        Self {
            summary_turn_count: value.summary_turn_count,
            retained_turn_count: value.retained_turn_count,
            demoted_recent_turn_count: value.demoted_recent_turn_count,
            total_turns: value.pruning.total_turns,
            assistant_turns: value.pruning.assistant_turns,
            low_signal_turns: value.pruning.low_signal_turns,
            tool_result_line_prunes: value.pruning.tool_result_line_prunes,
            tool_outcome_record_prunes: value.pruning.tool_outcome_record_prunes,
        }
    }
}

impl ContextCompactionDiagnostics {
    pub fn compact_summary(&self) -> String {
        format!(
            "summary:{} retained:{} demoted:{} low_signal:{} tool_results:{} tool_outcomes:{}",
            self.summary_turn_count,
            self.retained_turn_count,
            self.demoted_recent_turn_count,
            self.low_signal_turns,
            self.tool_result_line_prunes,
            self.tool_outcome_record_prunes,
        )
    }

    pub fn key_value_pairs(&self) -> Vec<(&'static str, String)> {
        vec![
            ("summary turns", self.summary_turn_count.to_string()),
            ("retained turns", self.retained_turn_count.to_string()),
            ("demoted recent", self.demoted_recent_turn_count.to_string()),
            ("low signal", self.low_signal_turns.to_string()),
            (
                "tool result prunes",
                self.tool_result_line_prunes.to_string(),
            ),
            (
                "tool outcome prunes",
                self.tool_outcome_record_prunes.to_string(),
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn compact_summary_renders_stable_compaction_hygiene_rollup() {
        let diagnostics = super::ContextCompactionDiagnostics {
            summary_turn_count: 6,
            retained_turn_count: 3,
            demoted_recent_turn_count: 1,
            total_turns: 9,
            assistant_turns: 4,
            low_signal_turns: 2,
            tool_result_line_prunes: 1,
            tool_outcome_record_prunes: 0,
        };

        let rendered = diagnostics.compact_summary();

        assert_eq!(
            rendered,
            "summary:6 retained:3 demoted:1 low_signal:2 tool_results:1 tool_outcomes:0"
        );
    }

    #[test]
    fn key_value_pairs_surface_all_operator_facing_compaction_fields() {
        let diagnostics = super::ContextCompactionDiagnostics {
            summary_turn_count: 4,
            retained_turn_count: 3,
            demoted_recent_turn_count: 1,
            total_turns: 7,
            assistant_turns: 3,
            low_signal_turns: 1,
            tool_result_line_prunes: 1,
            tool_outcome_record_prunes: 0,
        };

        let pairs = diagnostics.key_value_pairs();

        assert_eq!(pairs.len(), 6);
        assert_eq!(pairs[0], ("summary turns", "4".to_owned()));
        assert_eq!(pairs[1], ("retained turns", "3".to_owned()));
        assert_eq!(pairs[5], ("tool outcome prunes", "0".to_owned()));
    }
}
