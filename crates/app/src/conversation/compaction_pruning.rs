use crate::memory::WindowTurn;
use serde_json::Value;

use super::tool_result_compaction::compact_tool_result_payload_value;
use super::tool_result_reduction::reduce_tool_result_text_for_model;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CompactionPruneKind {
    ToolResultLine,
    ToolOutcomeRecord,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CompactionPruneDiagnostics {
    pub(crate) total_turns: usize,
    pub(crate) assistant_turns: usize,
    pub(crate) low_signal_turns: usize,
    pub(crate) tool_result_line_prunes: usize,
    pub(crate) tool_outcome_record_prunes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PrunedCompactionTurn {
    pub(super) turn: WindowTurn,
    pub(super) low_signal: bool,
    pub(super) prune_kind: Option<CompactionPruneKind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PrunedCompactionWindow {
    pub(super) turns: Vec<PrunedCompactionTurn>,
    pub(super) diagnostics: CompactionPruneDiagnostics,
}

#[cfg(test)]
pub(super) fn prune_compaction_window_inputs(turns: &[WindowTurn]) -> Vec<PrunedCompactionTurn> {
    inspect_compaction_window_inputs(turns).turns
}

pub(super) fn inspect_compaction_window_inputs(turns: &[WindowTurn]) -> PrunedCompactionWindow {
    let turns = turns.iter().map(prune_compaction_turn).collect::<Vec<_>>();
    let mut diagnostics = CompactionPruneDiagnostics {
        total_turns: turns.len(),
        ..CompactionPruneDiagnostics::default()
    };
    for turn in &turns {
        if turn.turn.role == "assistant" {
            diagnostics.assistant_turns += 1;
        }
        if turn.low_signal {
            diagnostics.low_signal_turns += 1;
        }
        match turn.prune_kind {
            Some(CompactionPruneKind::ToolResultLine) => diagnostics.tool_result_line_prunes += 1,
            Some(CompactionPruneKind::ToolOutcomeRecord) => {
                diagnostics.tool_outcome_record_prunes += 1
            }
            None => {}
        }
    }

    PrunedCompactionWindow { turns, diagnostics }
}

fn prune_compaction_turn(turn: &WindowTurn) -> PrunedCompactionTurn {
    let (pruned_content, prune_kind) = prune_assistant_turn_content(turn);
    let low_signal = pruned_content
        .as_deref()
        .is_some_and(|content| content != turn.content);
    let turn = WindowTurn {
        role: turn.role.clone(),
        content: pruned_content.unwrap_or_else(|| turn.content.clone()),
        ts: turn.ts,
    };

    PrunedCompactionTurn {
        turn,
        low_signal,
        prune_kind,
    }
}

fn prune_assistant_turn_content(
    turn: &WindowTurn,
) -> (Option<String>, Option<CompactionPruneKind>) {
    if turn.role != "assistant" {
        return (None, None);
    }

    if let Some(reduced_tool_result_text) = reduce_tool_result_text_for_model(turn.content.as_str())
    {
        return (
            Some(reduced_tool_result_text),
            Some(CompactionPruneKind::ToolResultLine),
        );
    }

    let Some(parsed) = serde_json::from_str::<Value>(turn.content.as_str()).ok() else {
        return (None, None);
    };
    let Some(record_type) = parsed.get("type").and_then(Value::as_str) else {
        return (None, None);
    };
    if record_type != "tool_outcome" {
        return (None, None);
    }

    (
        compact_tool_outcome_record_content(parsed),
        Some(CompactionPruneKind::ToolOutcomeRecord),
    )
}

fn compact_tool_outcome_record_content(parsed: Value) -> Option<String> {
    let record_object = parsed.as_object()?;
    let outcome_object = record_object.get("outcome")?.as_object()?;
    let tool_name = outcome_object.get("tool_name")?.as_str()?;
    let payload = outcome_object.get("payload")?;
    let compacted_payload = compact_tool_result_payload_value(tool_name, payload);
    if compacted_payload == *payload {
        return None;
    }

    let mut compacted_record = parsed;
    let outcome_payload = compacted_record
        .get_mut("outcome")?
        .as_object_mut()?
        .get_mut("payload")?;
    *outcome_payload = compacted_payload;
    serde_json::to_string(&compacted_record).ok()
}
