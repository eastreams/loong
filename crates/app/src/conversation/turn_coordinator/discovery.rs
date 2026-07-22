use super::*;

pub(super) async fn persist_tool_discovery_refresh_event_if_needed<
    R: ConversationRuntime + ?Sized,
>(
    _runtime: &R,
    _session_id: &str,
    _intent: &ToolIntent,
    _intent_sequence: usize,
    _tool_name: &str,
    _outcome: &loong_contracts::ToolCoreOutcome,
) {
    // Discovery-first tool refresh is no longer part of the provider followup contract.
}
