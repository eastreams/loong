use super::*;

pub(super) async fn emit_safe_lane_event<R: ConversationRuntime + ?Sized>(
    config: &LoongConfig,
    runtime: &R,
    session_id: &str,
    event_name: &str,
    payload: Value,
    binding: ConversationRuntimeBinding<'_>,
) {
    if !should_emit_safe_lane_event(config, event_name, &payload) {
        return;
    }
    let _ = persist_conversation_event(runtime, session_id, event_name, payload, binding).await;
    if let Some(ctx) = binding.kernel_context() {
        let _ = ctx.kernel.record_audit_event(
            Some(ctx.agent_id()),
            AuditEventKind::PlaneInvoked {
                pack_id: ctx.pack_id().to_owned(),
                plane: ExecutionPlane::Runtime,
                tier: PlaneTier::Core,
                primary_adapter: "conversation.safe_lane".to_owned(),
                delegated_core_adapter: None,
                operation: format!("conversation.safe_lane.{event_name}"),
                required_capabilities: Vec::new(),
            },
        );
    }
}

pub(super) async fn emit_fast_lane_tool_batch_event<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    trace: &ToolBatchExecutionTrace,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    persist_conversation_event(
        runtime,
        session_id,
        "fast_lane_tool_batch",
        trace.as_event_payload(),
        binding,
    )
    .await
}

pub(super) async fn persist_fast_lane_tool_trace<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    trace: &ToolBatchExecutionTrace,
    binding: ConversationRuntimeBinding<'_>,
) -> CliResult<()> {
    for record in &trace.decision_records {
        persist_tool_decision(
            runtime,
            session_id,
            &record.turn_id,
            &record.tool_call_id,
            &record.decision,
            binding,
        )
        .await?;
    }

    for record in &trace.outcome_records {
        persist_tool_outcome(
            runtime,
            session_id,
            &record.turn_id,
            &record.tool_call_id,
            &record.outcome,
            binding,
        )
        .await?;
    }

    Ok(())
}

pub(super) async fn emit_turn_ingress_event<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    ingress: Option<&ConversationIngressContext>,
    binding: ConversationRuntimeBinding<'_>,
) {
    let Some(ingress) = ingress else {
        return;
    };
    let _ = persist_conversation_event(
        runtime,
        session_id,
        "turn_ingress",
        ingress.as_event_payload(),
        binding,
    )
    .await;
}

pub(super) fn should_emit_safe_lane_event(
    _config: &LoongConfig,
    event_name: &str,
    payload: &Value,
) -> bool {
    if event_name == "plan_round_completed"
        && payload.get("status").and_then(Value::as_str) == Some("succeeded")
        && safe_lane_failure_pressure(payload) == 0
    {
        return false;
    }

    true
}

pub(super) fn safe_lane_failure_pressure(payload: &Value) -> u64 {
    let mut pressure = 0u64;

    if payload
        .get("status")
        .and_then(Value::as_str)
        .map(|status| status == "failed")
        .unwrap_or(false)
    {
        pressure = pressure.saturating_add(1);
    }

    if payload
        .get("failure_kind")
        .and_then(Value::as_str)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        pressure = pressure.saturating_add(1);
    }

    if payload
        .get("route_decision")
        .and_then(Value::as_str)
        .map(|decision| decision == "replan" || decision == "terminal")
        .unwrap_or(false)
    {
        pressure = pressure.saturating_add(1);
    }

    if payload
        .get("failure_code")
        .and_then(Value::as_str)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        pressure = pressure.saturating_add(1);
    }

    if payload
        .get("tool_output_stats")
        .and_then(|stats| stats.get("truncated_result_lines"))
        .and_then(Value::as_u64)
        .unwrap_or_default()
        > 0
    {
        pressure = pressure.saturating_add(1);
    }

    if payload
        .get("metrics")
        .and_then(|metrics| metrics.get("verify_failures"))
        .and_then(Value::as_u64)
        .unwrap_or_default()
        > 0
    {
        pressure = pressure.saturating_add(1);
    }

    pressure
}
