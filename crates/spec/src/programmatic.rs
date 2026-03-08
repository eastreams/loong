use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{FuturesUnordered, StreamExt};
use kernel::{Capability, ConnectorCommand, LoongClawKernel, StaticPolicyEngine};
use serde_json::{json, Value};
use tokio::time::{sleep, Instant as TokioInstant};

use crate::spec_runtime::*;
use crate::CliResult;

#[derive(Debug, Clone, Copy)]
enum ProgrammaticErrorCode {
    InvalidSpec,
    UnknownStep,
    PointerNotFound,
    TemplateError,
    ConnectorNotAllowed,
    CallBudgetExceeded,
    CircuitOpen,
    ConnectorInvokeFailed,
}

impl ProgrammaticErrorCode {
    fn as_str(self) -> &'static str {
        match self {
            ProgrammaticErrorCode::InvalidSpec => "invalid_spec",
            ProgrammaticErrorCode::UnknownStep => "unknown_step",
            ProgrammaticErrorCode::PointerNotFound => "pointer_not_found",
            ProgrammaticErrorCode::TemplateError => "template_error",
            ProgrammaticErrorCode::ConnectorNotAllowed => "connector_not_allowed",
            ProgrammaticErrorCode::CallBudgetExceeded => "call_budget_exceeded",
            ProgrammaticErrorCode::CircuitOpen => "circuit_open",
            ProgrammaticErrorCode::ConnectorInvokeFailed => "connector_invoke_failed",
        }
    }
}

fn programmatic_error(code: ProgrammaticErrorCode, message: impl Into<String>) -> String {
    format!("programmatic_error[{}]: {}", code.as_str(), message.into())
}

fn classify_connector_error_code(error: &str) -> &'static str {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("programmatic_error[circuit_open]") {
        "circuit_open"
    } else if normalized.contains("connector not found") {
        "connector_not_found"
    } else if normalized.contains("not allowed") {
        "connector_not_allowed"
    } else if normalized.contains("missing required capability") {
        "capability_denied"
    } else if normalized.contains("policy denied") {
        "policy_denied"
    } else {
        "connector_execution_error"
    }
}

fn should_reduce_programmatic_budget(
    concurrency: &ProgrammaticConcurrencyPolicy,
    error_code: &str,
) -> bool {
    let triggers = &concurrency.adaptive_reduce_on;
    if triggers.contains(&ProgrammaticAdaptiveReduceOn::AnyError) {
        return true;
    }
    let mapped = match error_code {
        "connector_execution_error" => ProgrammaticAdaptiveReduceOn::ConnectorExecutionError,
        "circuit_open" => ProgrammaticAdaptiveReduceOn::CircuitOpen,
        "connector_not_found" => ProgrammaticAdaptiveReduceOn::ConnectorNotFound,
        "connector_not_allowed" => ProgrammaticAdaptiveReduceOn::ConnectorNotAllowed,
        "capability_denied" => ProgrammaticAdaptiveReduceOn::CapabilityDenied,
        "policy_denied" => ProgrammaticAdaptiveReduceOn::PolicyDenied,
        _ => return false,
    };
    triggers.contains(&mapped)
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_programmatic_tool_call(
    kernel: &LoongClawKernel<StaticPolicyEngine>,
    pack_id: &str,
    token: &kernel::CapabilityToken,
    caller: &str,
    max_calls: usize,
    include_intermediate: bool,
    allowed_connectors: &BTreeSet<String>,
    connector_rate_limits: &BTreeMap<String, ProgrammaticConnectorRateLimit>,
    connector_circuit_breakers: &BTreeMap<String, ProgrammaticCircuitBreakerPolicy>,
    concurrency: &ProgrammaticConcurrencyPolicy,
    return_step: Option<&str>,
    steps: &[ProgrammaticStep],
) -> CliResult<Value> {
    if max_calls == 0 {
        return Err(programmatic_error(
            ProgrammaticErrorCode::InvalidSpec,
            "programmatic_tool_call max_calls must be greater than 0",
        ));
    }
    if steps.is_empty() {
        return Err(programmatic_error(
            ProgrammaticErrorCode::InvalidSpec,
            "programmatic_tool_call requires at least one step",
        ));
    }
    validate_programmatic_rate_limits(connector_rate_limits)?;
    validate_programmatic_circuit_breakers(connector_circuit_breakers)?;
    validate_programmatic_concurrency_policy(concurrency)?;

    let mut step_outputs = BTreeMap::<String, Value>::new();
    let mut connector_calls = 0_usize;
    let mut last_step_id: Option<String> = None;
    let mut step_trace = Vec::<Value>::new();
    let connector_rate_state = Arc::new(tokio::sync::Mutex::new(
        BTreeMap::<String, TokioInstant>::new(),
    ));
    let connector_circuit_state = Arc::new(tokio::sync::Mutex::new(BTreeMap::<
        String,
        ProgrammaticCircuitRuntimeState,
    >::new()));

    for (index, step) in steps.iter().enumerate() {
        match step {
            ProgrammaticStep::SetLiteral { step_id, value } => {
                ensure_new_step_id(step_id, &step_outputs)?;
                step_outputs.insert(step_id.clone(), value.clone());
                last_step_id = Some(step_id.clone());
                if include_intermediate {
                    step_trace.push(json!({
                        "step_id": step_id,
                        "kind": "set_literal",
                        "output": value,
                    }));
                }
            }
            ProgrammaticStep::JsonPointer {
                step_id,
                from_step,
                pointer,
            } => {
                ensure_new_step_id(step_id, &step_outputs)?;
                let source = step_outputs.get(from_step).ok_or_else(|| {
                    programmatic_error(
                        ProgrammaticErrorCode::UnknownStep,
                        format!(
                            "programmatic step {step_id} references unknown from_step {from_step}"
                        ),
                    )
                })?;
                let extracted = source.pointer(pointer).cloned().ok_or_else(|| {
                    programmatic_error(
                        ProgrammaticErrorCode::PointerNotFound,
                        format!(
                            "programmatic step {step_id} pointer {pointer} not found in {from_step}"
                        ),
                    )
                })?;
                step_outputs.insert(step_id.clone(), extracted.clone());
                last_step_id = Some(step_id.clone());
                if include_intermediate {
                    step_trace.push(json!({
                        "step_id": step_id,
                        "kind": "json_pointer",
                        "from_step": from_step,
                        "pointer": pointer,
                        "output": extracted,
                    }));
                }
            }
            ProgrammaticStep::ConnectorCall {
                step_id,
                connector_name,
                operation,
                required_capabilities,
                retry,
                priority_class,
                payload,
            } => {
                ensure_new_step_id(step_id, &step_outputs)?;
                if !allowed_connectors.is_empty() && !allowed_connectors.contains(connector_name) {
                    return Err(programmatic_error(
                        ProgrammaticErrorCode::ConnectorNotAllowed,
                        format!(
                            "programmatic step {step_id} connector {connector_name} is not in allowed_connectors"
                        ),
                    ));
                }
                if connector_calls >= max_calls {
                    return Err(programmatic_error(
                        ProgrammaticErrorCode::CallBudgetExceeded,
                        format!(
                            "programmatic connector call budget exceeded: max_calls={max_calls}"
                        ),
                    ));
                }
                let retry_policy = validate_retry_policy(retry.as_ref(), step_id, None)?;

                let resolved_payload =
                    resolve_programmatic_payload_templates(payload, &step_outputs)?;
                let payload_with_provenance = attach_programmatic_payload_provenance(
                    resolved_payload,
                    caller,
                    step_id,
                    index,
                    None,
                );

                let (dispatch, metrics) = invoke_programmatic_connector_with_resilience(
                    kernel,
                    pack_id,
                    token,
                    connector_name,
                    operation,
                    required_capabilities.clone(),
                    payload_with_provenance,
                    &retry_policy,
                    connector_rate_limits,
                    &connector_rate_state,
                    connector_circuit_breakers,
                    &connector_circuit_state,
                    step_id,
                    None,
                    *priority_class,
                )
                .await?;

                connector_calls = connector_calls.saturating_add(1);
                let step_result = json!({
                    "connector_name": dispatch.connector_name,
                    "outcome": dispatch.outcome,
                    "priority_class": priority_class.as_str(),
                    "execution": metrics,
                });
                step_outputs.insert(step_id.clone(), step_result.clone());
                last_step_id = Some(step_id.clone());
                if include_intermediate {
                    step_trace.push(json!({
                        "step_id": step_id,
                        "kind": "connector_call",
                        "connector_name": connector_name,
                        "operation": operation,
                        "output": step_result,
                    }));
                }
            }
            ProgrammaticStep::ConnectorBatch {
                step_id,
                parallel,
                continue_on_error,
                calls,
            } => {
                ensure_new_step_id(step_id, &step_outputs)?;
                if calls.is_empty() {
                    return Err(programmatic_error(
                        ProgrammaticErrorCode::InvalidSpec,
                        format!("programmatic batch step {step_id} requires at least one call"),
                    ));
                }
                let batch_calls = calls.len();
                if connector_calls.saturating_add(batch_calls) > max_calls {
                    return Err(programmatic_error(
                        ProgrammaticErrorCode::CallBudgetExceeded,
                        format!(
                            "programmatic connector call budget exceeded: max_calls={max_calls}, attempted={}",
                            connector_calls.saturating_add(batch_calls)
                        ),
                    ));
                }

                let prepared_calls = prepare_programmatic_batch_calls(
                    calls,
                    &step_outputs,
                    allowed_connectors,
                    caller,
                    step_id,
                    index,
                )?;
                connector_calls = connector_calls.saturating_add(prepared_calls.len());

                let (call_reports, scheduler) = execute_programmatic_batch_calls(
                    kernel,
                    pack_id,
                    token,
                    step_id,
                    *parallel,
                    *continue_on_error,
                    prepared_calls,
                    concurrency,
                    connector_rate_limits,
                    &connector_rate_state,
                    connector_circuit_breakers,
                    &connector_circuit_state,
                )
                .await?;

                let failed_calls = call_reports
                    .iter()
                    .filter(|report| report["status"] == Value::String("error".to_owned()))
                    .count();
                let success_calls = call_reports.len().saturating_sub(failed_calls);

                let mut by_call = serde_json::Map::new();
                for report in &call_reports {
                    if let Some(call_id) = report.get("call_id").and_then(Value::as_str) {
                        by_call.insert(call_id.to_owned(), report.clone());
                    }
                }

                let step_result = json!({
                    "parallel": parallel,
                    "continue_on_error": continue_on_error,
                    "total_calls": call_reports.len(),
                    "success_calls": success_calls,
                    "failed_calls": failed_calls,
                    "calls": call_reports,
                    "by_call": by_call,
                    "scheduler": scheduler,
                });
                step_outputs.insert(step_id.clone(), step_result.clone());
                last_step_id = Some(step_id.clone());
                if include_intermediate {
                    step_trace.push(json!({
                        "step_id": step_id,
                        "kind": "connector_batch",
                        "parallel": parallel,
                        "continue_on_error": continue_on_error,
                        "output": step_result,
                    }));
                }
            }
            ProgrammaticStep::Conditional {
                step_id,
                from_step,
                pointer,
                equals,
                exists,
                when_true,
                when_false,
            } => {
                ensure_new_step_id(step_id, &step_outputs)?;
                if equals.is_none() && exists.is_none() {
                    return Err(programmatic_error(
                        ProgrammaticErrorCode::InvalidSpec,
                        format!(
                            "programmatic conditional step {step_id} requires at least one predicate (equals or exists)"
                        ),
                    ));
                }

                let source = step_outputs.get(from_step).ok_or_else(|| {
                    programmatic_error(
                        ProgrammaticErrorCode::UnknownStep,
                        format!(
                            "programmatic step {step_id} references unknown from_step {from_step}"
                        ),
                    )
                })?;

                let evaluated = if let Some(pointer) = pointer.as_deref() {
                    let normalized_pointer = normalize_programmatic_pointer(pointer)?;
                    source.pointer(&normalized_pointer).cloned()
                } else {
                    Some(source.clone())
                };

                let mut matched = true;
                if let Some(expected_exists) = exists {
                    matched &= evaluated.is_some() == *expected_exists;
                }
                if let Some(expected_equals) = equals {
                    matched &= evaluated.as_ref() == Some(expected_equals);
                }

                let selected = if matched {
                    when_true
                } else {
                    when_false.as_ref().unwrap_or(&Value::Null)
                };
                let resolved_output =
                    resolve_programmatic_payload_templates(selected, &step_outputs)?;

                step_outputs.insert(step_id.clone(), resolved_output.clone());
                last_step_id = Some(step_id.clone());
                if include_intermediate {
                    step_trace.push(json!({
                        "step_id": step_id,
                        "kind": "conditional",
                        "from_step": from_step,
                        "pointer": pointer,
                        "exists": exists,
                        "equals": equals,
                        "evaluated": evaluated,
                        "matched": matched,
                        "output": resolved_output,
                    }));
                }
            }
        }
    }

    let final_step_id = match return_step.map(str::trim).filter(|value| !value.is_empty()) {
        Some(step_id) => step_id.to_owned(),
        None => last_step_id.ok_or_else(|| {
            programmatic_error(
                ProgrammaticErrorCode::InvalidSpec,
                "programmatic workflow produced no step output",
            )
        })?,
    };

    let final_output = step_outputs.get(&final_step_id).cloned().ok_or_else(|| {
        programmatic_error(
            ProgrammaticErrorCode::UnknownStep,
            format!("programmatic return_step {final_step_id} does not exist"),
        )
    })?;

    let mut response = json!({
        "caller": caller,
        "max_calls": max_calls,
        "connector_calls": connector_calls,
        "return_step": final_step_id,
        "result": final_output,
    });
    if !connector_rate_limits.is_empty() {
        let rate_limits = connector_rate_limits
            .iter()
            .map(|(connector, limit)| {
                (
                    connector.clone(),
                    json!({"min_interval_ms": limit.min_interval_ms}),
                )
            })
            .collect::<serde_json::Map<String, Value>>();
        response["connector_rate_limits"] = Value::Object(rate_limits);
    }
    if !connector_circuit_breakers.is_empty() {
        let breakers = connector_circuit_breakers
            .iter()
            .map(|(connector, policy)| {
                (
                    connector.clone(),
                    json!({
                        "failure_threshold": policy.failure_threshold,
                        "cooldown_ms": policy.cooldown_ms,
                        "half_open_max_calls": policy.half_open_max_calls,
                        "success_threshold": policy.success_threshold,
                    }),
                )
            })
            .collect::<serde_json::Map<String, Value>>();
        response["connector_circuit_breakers"] = Value::Object(breakers);
    }
    response["concurrency"] = json!({
        "max_in_flight": concurrency.max_in_flight,
        "min_in_flight": concurrency.min_in_flight,
        "fairness": concurrency.fairness.as_str(),
        "adaptive_budget": concurrency.adaptive_budget,
        "high_weight": concurrency.high_weight,
        "normal_weight": concurrency.normal_weight,
        "low_weight": concurrency.low_weight,
        "adaptive_recovery_successes": concurrency.adaptive_recovery_successes,
        "adaptive_upshift_step": concurrency.adaptive_upshift_step,
        "adaptive_downshift_step": concurrency.adaptive_downshift_step,
        "adaptive_reduce_on": concurrency
            .adaptive_reduce_on
            .iter()
            .map(|rule| rule.as_str())
            .collect::<Vec<_>>(),
    });

    if include_intermediate {
        response["steps"] = Value::Array(step_trace);
        let step_outputs_value = Value::Object(
            step_outputs
                .into_iter()
                .collect::<serde_json::Map<String, Value>>(),
        );
        response["step_outputs"] = step_outputs_value;
    }

    Ok(response)
}

fn ensure_new_step_id(step_id: &str, outputs: &BTreeMap<String, Value>) -> CliResult<()> {
    if step_id.trim().is_empty() {
        return Err(programmatic_error(
            ProgrammaticErrorCode::InvalidSpec,
            "programmatic step_id cannot be empty",
        ));
    }
    if outputs.contains_key(step_id) {
        return Err(programmatic_error(
            ProgrammaticErrorCode::InvalidSpec,
            format!("programmatic step_id {step_id} is duplicated"),
        ));
    }
    Ok(())
}

fn prepare_programmatic_batch_calls(
    calls: &[ProgrammaticBatchCall],
    outputs: &BTreeMap<String, Value>,
    allowed_connectors: &BTreeSet<String>,
    caller: &str,
    step_id: &str,
    step_index: usize,
) -> CliResult<Vec<PreparedProgrammaticBatchCall>> {
    let mut seen_call_ids = BTreeSet::new();
    let mut prepared = Vec::with_capacity(calls.len());

    for call in calls {
        let call_id = call.call_id.trim();
        if call_id.is_empty() {
            return Err(programmatic_error(
                ProgrammaticErrorCode::InvalidSpec,
                format!("programmatic batch step {step_id} has a call with empty call_id"),
            ));
        }
        if !seen_call_ids.insert(call_id.to_owned()) {
            return Err(programmatic_error(
                ProgrammaticErrorCode::InvalidSpec,
                format!("programmatic batch step {step_id} has duplicated call_id {call_id}"),
            ));
        }

        if !allowed_connectors.is_empty() && !allowed_connectors.contains(&call.connector_name) {
            return Err(programmatic_error(
                ProgrammaticErrorCode::ConnectorNotAllowed,
                format!(
                    "programmatic batch step {step_id} connector {} is not in allowed_connectors",
                    call.connector_name
                ),
            ));
        }
        let retry_policy = validate_retry_policy(call.retry.as_ref(), step_id, Some(call_id))?;

        let resolved_payload = resolve_programmatic_payload_templates(&call.payload, outputs)?;
        let payload_with_provenance = attach_programmatic_payload_provenance(
            resolved_payload,
            caller,
            step_id,
            step_index,
            Some(call_id),
        );

        prepared.push(PreparedProgrammaticBatchCall {
            call_id: call_id.to_owned(),
            connector_name: call.connector_name.clone(),
            operation: call.operation.clone(),
            required_capabilities: call.required_capabilities.clone(),
            retry_policy,
            priority_class: call.priority_class,
            payload: payload_with_provenance,
        });
    }

    Ok(prepared)
}

#[allow(clippy::too_many_arguments)]
async fn execute_programmatic_batch_calls(
    kernel: &LoongClawKernel<StaticPolicyEngine>,
    pack_id: &str,
    token: &kernel::CapabilityToken,
    step_id: &str,
    parallel: bool,
    continue_on_error: bool,
    prepared_calls: Vec<PreparedProgrammaticBatchCall>,
    concurrency: &ProgrammaticConcurrencyPolicy,
    connector_rate_limits: &BTreeMap<String, ProgrammaticConnectorRateLimit>,
    connector_rate_state: &Arc<tokio::sync::Mutex<BTreeMap<String, TokioInstant>>>,
    connector_circuit_breakers: &BTreeMap<String, ProgrammaticCircuitBreakerPolicy>,
    connector_circuit_state: &Arc<
        tokio::sync::Mutex<BTreeMap<String, ProgrammaticCircuitRuntimeState>>,
    >,
) -> CliResult<(Vec<Value>, ProgrammaticBatchExecutionSummary)> {
    let call_order: Vec<String> = prepared_calls
        .iter()
        .map(|call| call.call_id.clone())
        .collect();
    let mut reports_by_call = BTreeMap::<String, Value>::new();
    let mut first_error: Option<String> = None;

    let mut scheduler = ProgrammaticBatchExecutionSummary {
        mode: if parallel && prepared_calls.len() > 1 {
            "parallel".to_owned()
        } else {
            "serial".to_owned()
        },
        fairness: concurrency.fairness.as_str().to_owned(),
        adaptive_budget: parallel && prepared_calls.len() > 1 && concurrency.adaptive_budget,
        configured_max_in_flight: concurrency.max_in_flight,
        configured_min_in_flight: concurrency.min_in_flight,
        peak_in_flight: 0,
        final_in_flight_budget: 1,
        budget_reductions: 0,
        budget_increases: 0,
        adaptive_upshift_step: concurrency.adaptive_upshift_step,
        adaptive_downshift_step: concurrency.adaptive_downshift_step,
        adaptive_reduce_on: concurrency
            .adaptive_reduce_on
            .iter()
            .map(|rule| rule.as_str().to_owned())
            .collect(),
        scheduler_wait_cycles: 0,
        dispatch_order: Vec::new(),
        priority_dispatch_counts: BTreeMap::from([
            ("high".to_owned(), 0_usize),
            ("normal".to_owned(), 0_usize),
            ("low".to_owned(), 0_usize),
        ]),
    };

    if parallel && prepared_calls.len() > 1 {
        let weighted_cycle = build_programmatic_weighted_cycle(concurrency);
        let mut fairness_cursor = 0_usize;
        let mut inflight = FuturesUnordered::new();
        let mut current_budget = concurrency.max_in_flight;
        let mut consecutive_successes = 0_usize;
        let mut pending = group_programmatic_calls_by_priority(prepared_calls);
        scheduler.final_in_flight_budget = current_budget;

        while has_pending_programmatic_calls(&pending) || !inflight.is_empty() {
            let pending_before_dispatch = pending_programmatic_call_count(&pending);
            while inflight.len() < current_budget {
                let Some(call) = pop_next_programmatic_call(
                    &mut pending,
                    concurrency.fairness,
                    &weighted_cycle,
                    &mut fairness_cursor,
                ) else {
                    break;
                };

                let call_id = call.call_id.clone();
                let connector_name = call.connector_name.clone();
                let operation = call.operation.clone();
                let required_capabilities = call.required_capabilities.clone();
                let retry_policy = call.retry_policy.clone();
                let priority_class = call.priority_class;
                let payload = call.payload.clone();
                let connector_rate_limits = connector_rate_limits.clone();
                let connector_rate_state = connector_rate_state.clone();
                let connector_circuit_breakers = connector_circuit_breakers.clone();
                let connector_circuit_state = connector_circuit_state.clone();
                scheduler.dispatch_order.push(call_id.clone());
                if let Some(count) = scheduler
                    .priority_dispatch_counts
                    .get_mut(priority_class.as_str())
                {
                    *count = count.saturating_add(1);
                }
                inflight.push(async move {
                    let dispatch = invoke_programmatic_connector_with_resilience(
                        kernel,
                        pack_id,
                        token,
                        &connector_name,
                        &operation,
                        required_capabilities,
                        payload,
                        &retry_policy,
                        &connector_rate_limits,
                        &connector_rate_state,
                        &connector_circuit_breakers,
                        &connector_circuit_state,
                        step_id,
                        Some(&call_id),
                        priority_class,
                    )
                    .await;
                    (call_id, connector_name, operation, priority_class, dispatch)
                });
                scheduler.peak_in_flight = scheduler.peak_in_flight.max(inflight.len());
            }

            if inflight.is_empty() {
                break;
            }
            if pending_before_dispatch > 0 && inflight.len() >= current_budget {
                scheduler.scheduler_wait_cycles = scheduler.scheduler_wait_cycles.saturating_add(1);
            }

            if let Some((call_id, connector_name, operation, _, dispatch)) = inflight.next().await {
                let mut success = false;
                let mut failure_error_code: Option<String> = None;
                match dispatch {
                    Ok((dispatch, metrics)) => {
                        success = true;
                        reports_by_call.insert(
                            call_id.clone(),
                            json!({
                                "call_id": call_id,
                                "status": "ok",
                                "connector_name": dispatch.connector_name,
                                "operation": operation,
                                "outcome": dispatch.outcome,
                                "execution": metrics,
                            }),
                        );
                    }
                    Err(error) => {
                        let error_code = classify_connector_error_code(&error).to_owned();
                        failure_error_code = Some(error_code.clone());
                        reports_by_call.insert(
                            call_id.clone(),
                            json!({
                                "call_id": call_id,
                                "status": "error",
                                "connector_name": connector_name,
                                "operation": operation,
                                "error": error,
                                "error_code": error_code,
                            }),
                        );
                        if first_error.is_none() {
                            first_error = Some(programmatic_error(
                                ProgrammaticErrorCode::ConnectorInvokeFailed,
                                format!(
                                    "programmatic batch step {step_id} call {call_id} failed: {error}"
                                ),
                            ));
                        }
                    }
                }

                if scheduler.adaptive_budget {
                    if success {
                        consecutive_successes = consecutive_successes.saturating_add(1);
                        if current_budget < concurrency.max_in_flight
                            && consecutive_successes >= concurrency.adaptive_recovery_successes
                        {
                            current_budget = current_budget
                                .saturating_add(concurrency.adaptive_upshift_step)
                                .min(concurrency.max_in_flight);
                            scheduler.budget_increases =
                                scheduler.budget_increases.saturating_add(1);
                            consecutive_successes = 0;
                        }
                    } else {
                        consecutive_successes = 0;
                        if let Some(error_code) = failure_error_code.as_deref() {
                            if should_reduce_programmatic_budget(concurrency, error_code)
                                && current_budget > concurrency.min_in_flight
                            {
                                current_budget = current_budget
                                    .saturating_sub(concurrency.adaptive_downshift_step)
                                    .max(concurrency.min_in_flight);
                                scheduler.budget_reductions =
                                    scheduler.budget_reductions.saturating_add(1);
                            }
                        }
                    }
                }
                scheduler.final_in_flight_budget = current_budget;
            }
        }

        if let Some(error) = first_error {
            if !continue_on_error {
                return Err(error);
            }
        }
    } else {
        for call in prepared_calls {
            let call_id = call.call_id.clone();
            let connector_name = call.connector_name.clone();
            let operation = call.operation.clone();
            let priority_class = call.priority_class;
            scheduler.dispatch_order.push(call_id.clone());
            if let Some(count) = scheduler
                .priority_dispatch_counts
                .get_mut(priority_class.as_str())
            {
                *count = count.saturating_add(1);
            }
            let dispatch = invoke_programmatic_connector_with_resilience(
                kernel,
                pack_id,
                token,
                &connector_name,
                &operation,
                call.required_capabilities,
                call.payload,
                &call.retry_policy,
                connector_rate_limits,
                connector_rate_state,
                connector_circuit_breakers,
                connector_circuit_state,
                step_id,
                Some(&call_id),
                priority_class,
            )
            .await;
            scheduler.peak_in_flight = 1;

            match dispatch {
                Ok((dispatch, metrics)) => {
                    reports_by_call.insert(
                        call_id.clone(),
                        json!({
                            "call_id": call_id,
                            "status": "ok",
                            "connector_name": dispatch.connector_name,
                            "operation": operation,
                            "outcome": dispatch.outcome,
                            "execution": metrics,
                        }),
                    );
                }
                Err(error) => {
                    let error_code = classify_connector_error_code(&error).to_owned();
                    if !continue_on_error {
                        return Err(programmatic_error(
                            ProgrammaticErrorCode::ConnectorInvokeFailed,
                            format!(
                                "programmatic batch step {step_id} call {call_id} failed: {error}"
                            ),
                        ));
                    }
                    reports_by_call.insert(
                        call_id.clone(),
                        json!({
                            "call_id": call_id,
                            "status": "error",
                            "connector_name": connector_name,
                            "operation": operation,
                            "error": error,
                            "error_code": error_code,
                        }),
                    );
                }
            }
        }
        scheduler.final_in_flight_budget = 1;
    }

    Ok((
        call_order
            .iter()
            .filter_map(|call_id| reports_by_call.get(call_id).cloned())
            .collect(),
        scheduler,
    ))
}

fn group_programmatic_calls_by_priority(
    calls: Vec<PreparedProgrammaticBatchCall>,
) -> BTreeMap<ProgrammaticPriorityClass, VecDeque<PreparedProgrammaticBatchCall>> {
    let mut grouped = BTreeMap::from([
        (ProgrammaticPriorityClass::High, VecDeque::new()),
        (ProgrammaticPriorityClass::Normal, VecDeque::new()),
        (ProgrammaticPriorityClass::Low, VecDeque::new()),
    ]);
    for call in calls {
        grouped
            .entry(call.priority_class)
            .or_insert_with(VecDeque::new)
            .push_back(call);
    }
    grouped
}

fn has_pending_programmatic_calls(
    grouped: &BTreeMap<ProgrammaticPriorityClass, VecDeque<PreparedProgrammaticBatchCall>>,
) -> bool {
    grouped.values().any(|queue| !queue.is_empty())
}

fn pending_programmatic_call_count(
    grouped: &BTreeMap<ProgrammaticPriorityClass, VecDeque<PreparedProgrammaticBatchCall>>,
) -> usize {
    grouped.values().map(VecDeque::len).sum()
}

fn build_programmatic_weighted_cycle(
    concurrency: &ProgrammaticConcurrencyPolicy,
) -> Vec<ProgrammaticPriorityClass> {
    let mut cycle = Vec::new();
    cycle.extend(std::iter::repeat_n(
        ProgrammaticPriorityClass::High,
        concurrency.high_weight.max(1),
    ));
    cycle.extend(std::iter::repeat_n(
        ProgrammaticPriorityClass::Normal,
        concurrency.normal_weight.max(1),
    ));
    cycle.extend(std::iter::repeat_n(
        ProgrammaticPriorityClass::Low,
        concurrency.low_weight.max(1),
    ));
    cycle
}

fn pop_next_programmatic_call(
    grouped: &mut BTreeMap<ProgrammaticPriorityClass, VecDeque<PreparedProgrammaticBatchCall>>,
    fairness: ProgrammaticFairnessPolicy,
    weighted_cycle: &[ProgrammaticPriorityClass],
    cursor: &mut usize,
) -> Option<PreparedProgrammaticBatchCall> {
    let strict_order = [
        ProgrammaticPriorityClass::High,
        ProgrammaticPriorityClass::Normal,
        ProgrammaticPriorityClass::Low,
    ];

    match fairness {
        ProgrammaticFairnessPolicy::StrictRoundRobin => {
            for offset in 0..strict_order.len() {
                let index = cursor.saturating_add(offset) % strict_order.len();
                let priority = strict_order[index];
                if let Some(call) = grouped.get_mut(&priority).and_then(VecDeque::pop_front) {
                    *cursor = (index + 1) % strict_order.len();
                    return Some(call);
                }
            }
            None
        }
        ProgrammaticFairnessPolicy::WeightedRoundRobin => {
            if weighted_cycle.is_empty() {
                return grouped
                    .get_mut(&ProgrammaticPriorityClass::Normal)
                    .and_then(VecDeque::pop_front);
            }
            for offset in 0..weighted_cycle.len() {
                let index = cursor.saturating_add(offset) % weighted_cycle.len();
                let priority = weighted_cycle[index];
                if let Some(call) = grouped.get_mut(&priority).and_then(VecDeque::pop_front) {
                    *cursor = (index + 1) % weighted_cycle.len();
                    return Some(call);
                }
            }
            None
        }
    }
}

fn validate_retry_policy(
    policy: Option<&ProgrammaticRetryPolicy>,
    step_id: &str,
    call_id: Option<&str>,
) -> CliResult<ProgrammaticRetryPolicy> {
    let resolved = policy.cloned().unwrap_or(ProgrammaticRetryPolicy {
        max_attempts: default_programmatic_retry_max_attempts(),
        initial_backoff_ms: default_programmatic_retry_initial_backoff_ms(),
        max_backoff_ms: default_programmatic_retry_max_backoff_ms(),
        jitter_ratio: default_programmatic_retry_jitter_ratio(),
        adaptive_jitter: default_true(),
    });
    if resolved.max_attempts == 0 {
        return Err(programmatic_error(
            ProgrammaticErrorCode::InvalidSpec,
            format!(
                "programmatic step {step_id} retry.max_attempts must be greater than 0{}",
                call_id
                    .map(|id| format!(" (call_id={id})"))
                    .unwrap_or_default()
            ),
        ));
    }
    if resolved.max_backoff_ms < resolved.initial_backoff_ms {
        return Err(programmatic_error(
            ProgrammaticErrorCode::InvalidSpec,
            format!(
                "programmatic step {step_id} retry.max_backoff_ms must be >= retry.initial_backoff_ms{}",
                call_id
                    .map(|id| format!(" (call_id={id})"))
                    .unwrap_or_default()
            ),
        ));
    }
    if !resolved.jitter_ratio.is_finite() || !(0.0..=1.0).contains(&resolved.jitter_ratio) {
        return Err(programmatic_error(
            ProgrammaticErrorCode::InvalidSpec,
            format!(
                "programmatic step {step_id} retry.jitter_ratio must be within [0.0, 1.0]{}",
                call_id
                    .map(|id| format!(" (call_id={id})"))
                    .unwrap_or_default()
            ),
        ));
    }
    Ok(resolved)
}

fn validate_programmatic_rate_limits(
    rate_limits: &BTreeMap<String, ProgrammaticConnectorRateLimit>,
) -> CliResult<()> {
    for (connector_name, limit) in rate_limits {
        if connector_name.trim().is_empty() {
            return Err(programmatic_error(
                ProgrammaticErrorCode::InvalidSpec,
                "connector_rate_limits contains an empty connector key",
            ));
        }
        if limit.min_interval_ms == 0 {
            return Err(programmatic_error(
                ProgrammaticErrorCode::InvalidSpec,
                format!(
                    "connector_rate_limits[{connector_name}] min_interval_ms must be greater than 0"
                ),
            ));
        }
    }
    Ok(())
}

fn validate_programmatic_circuit_breakers(
    policies: &BTreeMap<String, ProgrammaticCircuitBreakerPolicy>,
) -> CliResult<()> {
    for (connector_name, policy) in policies {
        if connector_name.trim().is_empty() {
            return Err(programmatic_error(
                ProgrammaticErrorCode::InvalidSpec,
                "connector_circuit_breakers contains an empty connector key",
            ));
        }
        if policy.failure_threshold == 0 {
            return Err(programmatic_error(
                ProgrammaticErrorCode::InvalidSpec,
                format!(
                    "connector_circuit_breakers[{connector_name}] failure_threshold must be greater than 0"
                ),
            ));
        }
        if policy.cooldown_ms == 0 {
            return Err(programmatic_error(
                ProgrammaticErrorCode::InvalidSpec,
                format!(
                    "connector_circuit_breakers[{connector_name}] cooldown_ms must be greater than 0"
                ),
            ));
        }
        if policy.half_open_max_calls == 0 {
            return Err(programmatic_error(
                ProgrammaticErrorCode::InvalidSpec,
                format!(
                    "connector_circuit_breakers[{connector_name}] half_open_max_calls must be greater than 0"
                ),
            ));
        }
        if policy.success_threshold == 0 {
            return Err(programmatic_error(
                ProgrammaticErrorCode::InvalidSpec,
                format!(
                    "connector_circuit_breakers[{connector_name}] success_threshold must be greater than 0"
                ),
            ));
        }
        if policy.success_threshold > policy.half_open_max_calls {
            return Err(programmatic_error(
                ProgrammaticErrorCode::InvalidSpec,
                format!(
                    "connector_circuit_breakers[{connector_name}] success_threshold must be <= half_open_max_calls"
                ),
            ));
        }
    }
    Ok(())
}

fn validate_programmatic_concurrency_policy(
    concurrency: &ProgrammaticConcurrencyPolicy,
) -> CliResult<()> {
    if concurrency.max_in_flight == 0 {
        return Err(programmatic_error(
            ProgrammaticErrorCode::InvalidSpec,
            "programmatic concurrency.max_in_flight must be greater than 0",
        ));
    }
    if concurrency.min_in_flight == 0 {
        return Err(programmatic_error(
            ProgrammaticErrorCode::InvalidSpec,
            "programmatic concurrency.min_in_flight must be greater than 0",
        ));
    }
    if concurrency.min_in_flight > concurrency.max_in_flight {
        return Err(programmatic_error(
            ProgrammaticErrorCode::InvalidSpec,
            "programmatic concurrency.min_in_flight must be <= max_in_flight",
        ));
    }
    if concurrency.high_weight == 0 || concurrency.normal_weight == 0 || concurrency.low_weight == 0
    {
        return Err(programmatic_error(
            ProgrammaticErrorCode::InvalidSpec,
            "programmatic concurrency priority weights must all be greater than 0",
        ));
    }
    if concurrency.adaptive_upshift_step == 0 || concurrency.adaptive_downshift_step == 0 {
        return Err(programmatic_error(
            ProgrammaticErrorCode::InvalidSpec,
            "programmatic concurrency adaptive_upshift_step and adaptive_downshift_step must be greater than 0",
        ));
    }
    if concurrency.adaptive_budget && concurrency.adaptive_recovery_successes == 0 {
        return Err(programmatic_error(
            ProgrammaticErrorCode::InvalidSpec,
            "programmatic concurrency.adaptive_recovery_successes must be greater than 0 when adaptive_budget is enabled",
        ));
    }
    if concurrency.adaptive_budget && concurrency.adaptive_reduce_on.is_empty() {
        return Err(programmatic_error(
            ProgrammaticErrorCode::InvalidSpec,
            "programmatic concurrency.adaptive_reduce_on must include at least one trigger when adaptive_budget is enabled",
        ));
    }
    Ok(())
}

async fn apply_programmatic_rate_limit(
    connector_name: &str,
    rate_limits: &BTreeMap<String, ProgrammaticConnectorRateLimit>,
    rate_state: &Arc<tokio::sync::Mutex<BTreeMap<String, TokioInstant>>>,
) -> CliResult<u64> {
    let Some(limit) = rate_limits.get(connector_name) else {
        return Ok(0);
    };

    let wait_duration = {
        let mut guard = rate_state.lock().await;
        let now = TokioInstant::now();
        let scheduled = guard.get(connector_name).copied().unwrap_or(now);
        let wait = if scheduled > now {
            scheduled.duration_since(now)
        } else {
            Duration::from_millis(0)
        };
        let base = if scheduled > now { scheduled } else { now };
        let next = base + Duration::from_millis(limit.min_interval_ms);
        guard.insert(connector_name.to_owned(), next);
        wait
    };

    if !wait_duration.is_zero() {
        sleep(wait_duration).await;
    }
    Ok(wait_duration.as_millis() as u64)
}

#[allow(clippy::too_many_arguments)]
async fn invoke_programmatic_connector_with_resilience(
    kernel: &LoongClawKernel<StaticPolicyEngine>,
    pack_id: &str,
    token: &kernel::CapabilityToken,
    connector_name: &str,
    operation: &str,
    required_capabilities: BTreeSet<Capability>,
    payload: Value,
    retry_policy: &ProgrammaticRetryPolicy,
    connector_rate_limits: &BTreeMap<String, ProgrammaticConnectorRateLimit>,
    connector_rate_state: &Arc<tokio::sync::Mutex<BTreeMap<String, TokioInstant>>>,
    connector_circuit_breakers: &BTreeMap<String, ProgrammaticCircuitBreakerPolicy>,
    connector_circuit_state: &Arc<
        tokio::sync::Mutex<BTreeMap<String, ProgrammaticCircuitRuntimeState>>,
    >,
    step_id: &str,
    call_id: Option<&str>,
    priority_class: ProgrammaticPriorityClass,
) -> CliResult<(kernel::ConnectorDispatch, ProgrammaticInvocationMetrics)> {
    let circuit_phase_before = acquire_programmatic_circuit_slot(
        connector_name,
        connector_circuit_breakers,
        connector_circuit_state,
        step_id,
        call_id,
    )
    .await?;

    let mut backoff_ms_total = 0_u64;
    let mut rate_wait_ms_total = 0_u64;
    let mut last_error: Option<String> = None;

    for attempt in 1..=retry_policy.max_attempts {
        let waited = apply_programmatic_rate_limit(
            connector_name,
            connector_rate_limits,
            connector_rate_state,
        )
        .await?;
        rate_wait_ms_total = rate_wait_ms_total.saturating_add(waited);

        let dispatch = kernel
            .invoke_connector(
                pack_id,
                token,
                ConnectorCommand {
                    connector_name: connector_name.to_owned(),
                    operation: operation.to_owned(),
                    required_capabilities: required_capabilities.clone(),
                    payload: payload.clone(),
                },
            )
            .await;

        match dispatch {
            Ok(dispatch) => {
                let circuit_phase_after = record_programmatic_circuit_outcome(
                    connector_name,
                    true,
                    connector_circuit_breakers,
                    connector_circuit_state,
                )
                .await;
                return Ok((
                    dispatch,
                    ProgrammaticInvocationMetrics {
                        attempts: attempt,
                        retries: attempt.saturating_sub(1),
                        priority_class: priority_class.as_str().to_owned(),
                        rate_wait_ms_total,
                        backoff_ms_total,
                        circuit_phase_before: circuit_phase_before.to_owned(),
                        circuit_phase_after,
                    },
                ));
            }
            Err(error) => {
                let error_string = error.to_string();
                let connector_error_code = classify_connector_error_code(&error_string);
                if attempt >= retry_policy.max_attempts {
                    let circuit_phase_after = record_programmatic_circuit_outcome(
                        connector_name,
                        false,
                        connector_circuit_breakers,
                        connector_circuit_state,
                    )
                    .await;
                    let call_label = call_id
                        .map(|id| format!(" call_id={id}"))
                        .unwrap_or_default();
                    return Err(programmatic_error(
                        ProgrammaticErrorCode::ConnectorInvokeFailed,
                        format!(
                            "programmatic step {step_id}{call_label} connector {connector_name} invoke failed after attempts={attempt}: {error_string} (connector_error_code={connector_error_code}, circuit_phase_before={circuit_phase_before}, circuit_phase_after={circuit_phase_after})"
                        ),
                    ));
                }

                last_error = Some(error_string);
                let backoff = compute_programmatic_backoff_ms(
                    retry_policy,
                    connector_name,
                    step_id,
                    call_id,
                    attempt,
                );
                if backoff > 0 {
                    backoff_ms_total = backoff_ms_total.saturating_add(backoff);
                    sleep(Duration::from_millis(backoff)).await;
                }
            }
        }
    }

    Err(programmatic_error(
        ProgrammaticErrorCode::ConnectorInvokeFailed,
        format!(
            "programmatic step {step_id} exhausted retries without terminal dispatch (last_error={})",
            last_error.unwrap_or_else(|| "unknown".to_owned())
        ),
    ))
}

pub async fn acquire_programmatic_circuit_slot(
    connector_name: &str,
    policies: &BTreeMap<String, ProgrammaticCircuitBreakerPolicy>,
    state: &Arc<tokio::sync::Mutex<BTreeMap<String, ProgrammaticCircuitRuntimeState>>>,
    step_id: &str,
    call_id: Option<&str>,
) -> CliResult<&'static str> {
    let Some(policy) = policies.get(connector_name) else {
        return Ok("disabled");
    };

    let mut guard = state.lock().await;
    let entry = guard.entry(connector_name.to_owned()).or_default();
    let now = TokioInstant::now();

    if entry.phase == ProgrammaticCircuitPhase::Open {
        if let Some(until) = entry.open_until {
            if until > now {
                let remaining_ms = until.duration_since(now).as_millis();
                let call_label = call_id
                    .map(|id| format!(" call_id={id}"))
                    .unwrap_or_default();
                return Err(programmatic_error(
                    ProgrammaticErrorCode::CircuitOpen,
                    format!(
                        "programmatic step {step_id}{call_label} connector {connector_name} is circuit-open (remaining_cooldown_ms={remaining_ms})"
                    ),
                ));
            }
        }
        entry.phase = ProgrammaticCircuitPhase::HalfOpen;
        entry.open_until = None;
        entry.half_open_remaining_calls = policy.half_open_max_calls;
        entry.half_open_successes = 0;
    }

    if entry.phase == ProgrammaticCircuitPhase::HalfOpen {
        if entry.half_open_remaining_calls == 0 {
            entry.phase = ProgrammaticCircuitPhase::Open;
            entry.open_until = Some(now + Duration::from_millis(policy.cooldown_ms));
            let call_label = call_id
                .map(|id| format!(" call_id={id}"))
                .unwrap_or_default();
            return Err(programmatic_error(
                ProgrammaticErrorCode::CircuitOpen,
                format!(
                    "programmatic step {step_id}{call_label} connector {connector_name} half-open window exhausted and re-opened"
                ),
            ));
        }
        entry.half_open_remaining_calls = entry.half_open_remaining_calls.saturating_sub(1);
        return Ok("half_open");
    }

    Ok("closed")
}

pub async fn record_programmatic_circuit_outcome(
    connector_name: &str,
    success: bool,
    policies: &BTreeMap<String, ProgrammaticCircuitBreakerPolicy>,
    state: &Arc<tokio::sync::Mutex<BTreeMap<String, ProgrammaticCircuitRuntimeState>>>,
) -> String {
    let Some(policy) = policies.get(connector_name) else {
        return "disabled".to_owned();
    };

    let mut guard = state.lock().await;
    let entry = guard.entry(connector_name.to_owned()).or_default();
    let now = TokioInstant::now();

    match entry.phase {
        ProgrammaticCircuitPhase::Closed => {
            if success {
                entry.consecutive_failures = 0;
            } else {
                entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
                if entry.consecutive_failures >= policy.failure_threshold {
                    entry.phase = ProgrammaticCircuitPhase::Open;
                    entry.open_until = Some(now + Duration::from_millis(policy.cooldown_ms));
                    entry.half_open_remaining_calls = 0;
                    entry.half_open_successes = 0;
                }
            }
        }
        ProgrammaticCircuitPhase::HalfOpen => {
            if success {
                entry.half_open_successes = entry.half_open_successes.saturating_add(1);
                if entry.half_open_successes >= policy.success_threshold {
                    entry.phase = ProgrammaticCircuitPhase::Closed;
                    entry.consecutive_failures = 0;
                    entry.open_until = None;
                    entry.half_open_remaining_calls = 0;
                    entry.half_open_successes = 0;
                } else if entry.half_open_remaining_calls == 0 {
                    entry.phase = ProgrammaticCircuitPhase::Open;
                    entry.open_until = Some(now + Duration::from_millis(policy.cooldown_ms));
                    entry.half_open_successes = 0;
                }
            } else {
                entry.phase = ProgrammaticCircuitPhase::Open;
                entry.open_until = Some(now + Duration::from_millis(policy.cooldown_ms));
                entry.half_open_remaining_calls = 0;
                entry.half_open_successes = 0;
            }
        }
        ProgrammaticCircuitPhase::Open => {}
    }

    match entry.phase {
        ProgrammaticCircuitPhase::Closed => "closed".to_owned(),
        ProgrammaticCircuitPhase::Open => "open".to_owned(),
        ProgrammaticCircuitPhase::HalfOpen => "half_open".to_owned(),
    }
}

fn compute_programmatic_backoff_ms(
    policy: &ProgrammaticRetryPolicy,
    connector_name: &str,
    step_id: &str,
    call_id: Option<&str>,
    attempt: usize,
) -> u64 {
    if attempt == 0 {
        return 0;
    }
    let exponent = attempt.saturating_sub(1) as u32;
    let growth = 2_u64.saturating_pow(exponent);
    let base = policy
        .initial_backoff_ms
        .saturating_mul(growth)
        .min(policy.max_backoff_ms);
    if !policy.adaptive_jitter || base == 0 || policy.jitter_ratio <= 0.0 {
        return base;
    }
    let jitter_cap = ((base as f64) * policy.jitter_ratio).round() as u64;
    if jitter_cap == 0 {
        return base;
    }
    let seed = format!(
        "{connector_name}|{step_id}|{}|attempt-{attempt}",
        call_id.unwrap_or("no-call-id")
    );
    let jitter = deterministic_jitter_ms(seed.as_bytes(), jitter_cap.saturating_add(1));
    base.saturating_add(jitter).min(policy.max_backoff_ms)
}

fn deterministic_jitter_ms(seed: &[u8], modulus: u64) -> u64 {
    if modulus == 0 {
        return 0;
    }
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET_BASIS;
    for byte in seed {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash % modulus
}

fn resolve_programmatic_payload_templates(
    payload: &Value,
    outputs: &BTreeMap<String, Value>,
) -> CliResult<Value> {
    match payload {
        Value::Object(map) => {
            let mut resolved = serde_json::Map::new();
            for (key, value) in map {
                resolved.insert(
                    key.clone(),
                    resolve_programmatic_payload_templates(value, outputs)?,
                );
            }
            Ok(Value::Object(resolved))
        }
        Value::Array(list) => {
            let mut resolved = Vec::with_capacity(list.len());
            for value in list {
                resolved.push(resolve_programmatic_payload_templates(value, outputs)?);
            }
            Ok(Value::Array(resolved))
        }
        Value::String(raw) => resolve_programmatic_template_string(raw, outputs),
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(payload.clone()),
    }
}

fn resolve_programmatic_template_string(
    raw: &str,
    outputs: &BTreeMap<String, Value>,
) -> CliResult<Value> {
    let Some(first_start) = raw.find("{{") else {
        return Ok(Value::String(raw.to_owned()));
    };
    let Some(first_end_rel) = raw[first_start + 2..].find("}}") else {
        return Err(programmatic_error(
            ProgrammaticErrorCode::TemplateError,
            format!("template placeholder is missing closing braces in: {raw}"),
        ));
    };
    let first_end = first_start + 2 + first_end_rel;
    if first_start == 0 && first_end + 2 == raw.len() {
        let expr = &raw[first_start + 2..first_end];
        return resolve_programmatic_template_expr(expr, outputs);
    }

    let mut cursor = 0_usize;
    let mut rendered = String::new();
    while let Some(start_rel) = raw[cursor..].find("{{") {
        let start = cursor + start_rel;
        rendered.push_str(&raw[cursor..start]);
        let end_rel = raw[start + 2..].find("}}").ok_or_else(|| {
            programmatic_error(
                ProgrammaticErrorCode::TemplateError,
                format!("template placeholder is missing closing braces in: {raw}"),
            )
        })?;
        let end = start + 2 + end_rel;
        let expr = &raw[start + 2..end];
        let value = resolve_programmatic_template_expr(expr, outputs)?;
        match value {
            Value::String(string) => rendered.push_str(&string),
            other => rendered.push_str(&other.to_string()),
        }
        cursor = end + 2;
    }
    rendered.push_str(&raw[cursor..]);
    Ok(Value::String(rendered))
}

fn resolve_programmatic_template_expr(
    raw_expr: &str,
    outputs: &BTreeMap<String, Value>,
) -> CliResult<Value> {
    let expr = raw_expr.trim();
    if expr.is_empty() {
        return Err(programmatic_error(
            ProgrammaticErrorCode::TemplateError,
            "template expression cannot be empty",
        ));
    }

    let (step_id, pointer) = match expr.split_once('#') {
        Some((step_id, pointer)) => (step_id.trim(), Some(pointer.trim())),
        None => (expr, None),
    };
    if step_id.is_empty() {
        return Err(programmatic_error(
            ProgrammaticErrorCode::TemplateError,
            format!("template expression {expr} has empty step_id"),
        ));
    }

    let value = outputs.get(step_id).ok_or_else(|| {
        programmatic_error(
            ProgrammaticErrorCode::UnknownStep,
            format!("template expression references unknown step {step_id}"),
        )
    })?;

    if let Some(pointer) = pointer {
        let normalized_pointer = normalize_programmatic_pointer(pointer)?;
        value.pointer(&normalized_pointer).cloned().ok_or_else(|| {
            programmatic_error(
                ProgrammaticErrorCode::PointerNotFound,
                format!("template pointer {normalized_pointer} not found in step {step_id}"),
            )
        })
    } else {
        Ok(value.clone())
    }
}

fn normalize_programmatic_pointer(pointer: &str) -> CliResult<String> {
    let pointer = pointer.trim();
    if pointer.is_empty() {
        return Err(programmatic_error(
            ProgrammaticErrorCode::InvalidSpec,
            "json pointer cannot be empty",
        ));
    }
    if pointer.starts_with('/') {
        Ok(pointer.to_owned())
    } else {
        Ok(format!("/{pointer}"))
    }
}

fn attach_programmatic_payload_provenance(
    payload: Value,
    caller: &str,
    step_id: &str,
    step_index: usize,
    call_id: Option<&str>,
) -> Value {
    let mut payload_map = match payload {
        Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("input".to_owned(), other);
            map
        }
    };

    let mut meta = payload_map
        .remove("_loongclaw")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    meta.insert("caller".to_owned(), Value::String(caller.to_owned()));
    meta.insert(
        "mode".to_owned(),
        Value::String("programmatic_tool_call".to_owned()),
    );
    meta.insert("step_id".to_owned(), Value::String(step_id.to_owned()));
    meta.insert("step_index".to_owned(), json!(step_index));
    if let Some(call_id) = call_id {
        meta.insert("call_id".to_owned(), Value::String(call_id.to_owned()));
    }
    payload_map.insert("_loongclaw".to_owned(), Value::Object(meta));

    Value::Object(payload_map)
}
