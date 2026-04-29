use crate::memory::WindowTurn;

use super::compaction::CompactPolicy;
use super::compaction_pruning::{CompactionPruneDiagnostics, inspect_compaction_window_inputs};
use super::compaction_retention::prepare_retained_tail_for_session_local_recall;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompactionPreparationDiagnostics {
    pub(crate) summary_turn_count: usize,
    pub(crate) retained_turn_count: usize,
    pub(crate) demoted_recent_turn_count: usize,
    pub(crate) pruning: CompactionPruneDiagnostics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CompactionPreparation {
    pub(super) summary_turns: Vec<WindowTurn>,
    pub(super) retained_turns: Vec<WindowTurn>,
    pub(crate) diagnostics: CompactionPreparationDiagnostics,
}

impl CompactionPreparation {
    pub(super) fn compacted_turn_count(&self) -> usize {
        self.summary_turns.len()
    }
}

pub(super) fn prepare_compaction_window(
    turns: &[WindowTurn],
    policy: CompactPolicy,
) -> Option<CompactionPreparation> {
    let pruned_window = inspect_compaction_window_inputs(turns);
    prepare_pruned_compaction_window(pruned_window, policy)
}

fn prepare_pruned_compaction_window(
    pruned_window: super::compaction_pruning::PrunedCompactionWindow,
    policy: CompactPolicy,
) -> Option<CompactionPreparation> {
    let turns = pruned_window.turns;
    let preserve = policy.preserve_recent_turns().min(turns.len());
    if turns.len() <= preserve {
        return None;
    }

    let split_at = turns.len() - preserve;
    let summary_turns_slice = turns.get(..split_at)?;
    let recent_turns = turns.get(split_at..)?;
    let mut summary_turns = summary_turns_slice
        .iter()
        .map(|turn| turn.turn.clone())
        .collect::<Vec<_>>();
    let retained_tail = prepare_retained_tail_for_session_local_recall(
        recent_turns,
        policy.preserve_recent_estimated_tokens(),
    );
    let demoted_recent_turn_count = retained_tail.summary_appended_turns.len();
    summary_turns.extend(retained_tail.summary_appended_turns);
    let retained_turns = retained_tail.retained_turns;
    let diagnostics = CompactionPreparationDiagnostics {
        summary_turn_count: summary_turns.len(),
        retained_turn_count: retained_turns.len(),
        demoted_recent_turn_count,
        pruning: pruned_window.diagnostics,
    };

    (!summary_turns.is_empty()).then_some(CompactionPreparation {
        summary_turns,
        retained_turns,
        diagnostics,
    })
}
