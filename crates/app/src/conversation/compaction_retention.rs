use crate::memory::WindowTurn;

use super::compaction_pruning::PrunedCompactionTurn;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RetainedTailPlan {
    pub(super) summary_appended_turns: Vec<WindowTurn>,
    pub(super) retained_turns: Vec<WindowTurn>,
}

pub(super) fn prepare_retained_tail_for_session_local_recall(
    turns: &[PrunedCompactionTurn],
    token_budget: Option<usize>,
) -> RetainedTailPlan {
    let mut retained_candidates = turns.to_vec();
    let mut summary_appended_turns = Vec::new();

    let Some(token_budget) = token_budget else {
        return RetainedTailPlan {
            summary_appended_turns,
            retained_turns: retained_candidates
                .into_iter()
                .map(|candidate| candidate.turn)
                .collect(),
        };
    };

    while retained_candidates.len() > 1
        && estimate_retention_candidate_token_cost(retained_candidates.as_slice()) > token_budget
    {
        let removal_index =
            oldest_low_signal_retention_index(retained_candidates.as_slice()).unwrap_or(0);
        let removed = retained_candidates.remove(removal_index);
        summary_appended_turns.push(removed.turn);
    }

    RetainedTailPlan {
        summary_appended_turns,
        retained_turns: retained_candidates
            .into_iter()
            .map(|candidate| candidate.turn)
            .collect(),
    }
}

fn oldest_low_signal_retention_index(candidates: &[PrunedCompactionTurn]) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .take(candidates.len().saturating_sub(1))
        .find_map(|(index, candidate)| candidate.low_signal.then_some(index))
}

fn estimate_retention_candidate_token_cost(candidates: &[PrunedCompactionTurn]) -> usize {
    candidates
        .iter()
        .map(|candidate| estimate_retained_turn_tokens(&candidate.turn))
        .sum()
}

fn estimate_retained_turn_tokens(turn: &WindowTurn) -> usize {
    let role_chars = turn.role.chars().count();
    let content_chars = turn.content.chars().count();
    (role_chars + content_chars).div_ceil(4) + 4
}
