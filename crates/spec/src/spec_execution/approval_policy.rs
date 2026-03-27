use std::fs;

use serde_json::Value;

use crate::BUNDLED_APPROVAL_RISK_PROFILE;
use crate::spec_runtime::*;

pub(super) fn evaluate_approval_guard(spec: &RunnerSpec) -> ApprovalDecisionReport {
    let policy = spec.approval.clone().unwrap_or_default();
    let now_epoch_s = super::current_epoch_s();
    let operation_key = operation_approval_key(&spec.operation);
    let operation_kind = operation_approval_kind(&spec.operation);
    let target_in_scope = is_operation_in_approval_scope(&spec.operation, policy.scope);
    let denylisted = is_operation_preapproved(&operation_key, &policy.denied_calls);

    let (risk_level, matched_keywords, risk_score) =
        match operation_risk_profile(&spec.operation, &policy) {
            (ApprovalRiskLevel::High, matched, score) => (ApprovalRiskLevel::High, matched, score),
            (_, _, score) => (ApprovalRiskLevel::Low, Vec::new(), score),
        };

    if denylisted {
        return ApprovalDecisionReport {
            mode: policy.mode,
            strategy: policy.strategy,
            scope: policy.scope,
            now_epoch_s,
            operation_key,
            operation_kind,
            risk_level,
            risk_score,
            denylisted: true,
            requires_human_approval: true,
            approved: false,
            reason: "operation is denylisted by human approval policy".to_owned(),
            matched_keywords,
        };
    }

    let one_time_full_access_active = policy.one_time_full_access_granted
        && policy
            .one_time_full_access_expires_at_epoch_s
            .map(|deadline| now_epoch_s <= deadline)
            .unwrap_or(true)
        && policy
            .one_time_full_access_remaining_uses
            .map(|remaining| remaining > 0)
            .unwrap_or(true);

    let one_time_full_access_rejected_reason = if policy.one_time_full_access_granted {
        if let Some(deadline) = policy.one_time_full_access_expires_at_epoch_s {
            if now_epoch_s > deadline {
                Some(format!(
                    "one-time full access grant expired at {deadline}, now is {now_epoch_s}"
                ))
            } else {
                None
            }
        } else if matches!(policy.one_time_full_access_remaining_uses, Some(0)) {
            Some("one-time full access grant has no remaining uses".to_owned())
        } else {
            None
        }
    } else {
        None
    };

    let requires_human_approval = if !target_in_scope {
        false
    } else {
        match policy.mode {
            HumanApprovalMode::Disabled => false,
            HumanApprovalMode::MediumBalanced => matches!(risk_level, ApprovalRiskLevel::High),
            HumanApprovalMode::Strict => true,
        }
    };

    let (approved, reason) = if !requires_human_approval {
        (
            true,
            "operation is allowed by default medium-balanced approval policy".to_owned(),
        )
    } else {
        match policy.strategy {
            HumanApprovalStrategy::OneTimeFullAccess if one_time_full_access_active => (
                true,
                "human granted one-time full access for this execution".to_owned(),
            ),
            HumanApprovalStrategy::PerCall
                if is_operation_preapproved(&operation_key, &policy.approved_calls) =>
            {
                (
                    true,
                    format!("operation {operation_key} is pre-approved by human policy"),
                )
            }
            HumanApprovalStrategy::PerCall => (
                false,
                format!(
                    "human approval required for high-risk operation {operation_key}; \
                     add to approval.approved_calls or switch to one_time_full_access"
                ),
            ),
            HumanApprovalStrategy::OneTimeFullAccess => (false, one_time_full_access_rejected_reason
                .unwrap_or_else(|| {
                    format!(
                        "human one-time full access is not granted for high-risk operation {operation_key}"
                    )
                })),
        }
    };

    ApprovalDecisionReport {
        mode: policy.mode,
        strategy: policy.strategy,
        scope: policy.scope,
        now_epoch_s,
        operation_key,
        operation_kind,
        risk_level,
        risk_score,
        denylisted: false,
        requires_human_approval,
        approved,
        reason,
        matched_keywords,
    }
}

fn operation_approval_key(operation: &OperationSpec) -> String {
    match operation {
        OperationSpec::Task { task_id, .. } => format!("task:{task_id}"),
        OperationSpec::ConnectorLegacy {
            connector_name,
            operation,
            ..
        } => {
            format!("connector_legacy:{connector_name}:{operation}")
        }
        OperationSpec::ConnectorCore {
            connector_name,
            operation,
            ..
        } => {
            format!("connector_core:{connector_name}:{operation}")
        }
        OperationSpec::ConnectorExtension {
            connector_name,
            operation,
            extension,
            ..
        } => {
            format!("connector_extension:{extension}:{connector_name}:{operation}")
        }
        OperationSpec::RuntimeCore { action, .. } => format!("runtime_core:{action}"),
        OperationSpec::RuntimeExtension {
            extension, action, ..
        } => {
            format!("runtime_extension:{extension}:{action}")
        }
        OperationSpec::ToolCore { tool_name, .. } => format!("tool_core:{tool_name}"),
        OperationSpec::ToolExtension {
            extension,
            extension_action,
            ..
        } => {
            format!("tool_extension:{extension}:{extension_action}")
        }
        OperationSpec::MemoryCore { operation, .. } => format!("memory_core:{operation}"),
        OperationSpec::MemoryExtension {
            extension,
            operation,
            ..
        } => {
            format!("memory_extension:{extension}:{operation}")
        }
        OperationSpec::ToolSearch {
            query, trust_tiers, ..
        } => {
            if trust_tiers.is_empty() {
                format!("tool_search:{query}")
            } else {
                let trust_scope = trust_tiers
                    .iter()
                    .map(|tier| tier.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                format!("tool_search:{query}:trust:{trust_scope}")
            }
        }
        OperationSpec::PluginInventory { query, .. } => format!("plugin_inventory:{query}"),
        OperationSpec::PluginPreflight { query, profile, .. } => {
            format!("plugin_preflight:{}:{query}", profile.as_str())
        }
        OperationSpec::ProgrammaticToolCall { caller, .. } => {
            format!("programmatic_tool_call:{caller}")
        }
    }
}

fn operation_approval_kind(operation: &OperationSpec) -> &'static str {
    match operation {
        OperationSpec::Task { .. } => "task",
        OperationSpec::ConnectorLegacy { .. } => "connector_legacy",
        OperationSpec::ConnectorCore { .. } => "connector_core",
        OperationSpec::ConnectorExtension { .. } => "connector_extension",
        OperationSpec::RuntimeCore { .. } => "runtime_core",
        OperationSpec::RuntimeExtension { .. } => "runtime_extension",
        OperationSpec::ToolCore { .. } => "tool_core",
        OperationSpec::ToolExtension { .. } => "tool_extension",
        OperationSpec::MemoryCore { .. } => "memory_core",
        OperationSpec::MemoryExtension { .. } => "memory_extension",
        OperationSpec::ToolSearch { .. } => "tool_search",
        OperationSpec::PluginInventory { .. } => "plugin_inventory",
        OperationSpec::PluginPreflight { .. } => "plugin_preflight",
        OperationSpec::ProgrammaticToolCall { .. } => "programmatic_tool_call",
    }
}

fn is_operation_in_approval_scope(operation: &OperationSpec, scope: HumanApprovalScope) -> bool {
    match scope {
        HumanApprovalScope::ToolCalls => matches!(
            operation,
            OperationSpec::ToolCore { .. }
                | OperationSpec::ToolExtension { .. }
                | OperationSpec::ProgrammaticToolCall { .. }
        ),
        HumanApprovalScope::AllOperations => true,
    }
}

pub fn operation_risk_profile(
    operation: &OperationSpec,
    policy: &HumanApprovalSpec,
) -> (ApprovalRiskLevel, Vec<String>, u8) {
    let profile = resolve_approval_risk_profile(policy);
    let keywords = normalize_signal_list(profile.high_risk_keywords);
    let high_risk_tool_names = normalize_signal_list(profile.high_risk_tool_names);
    let high_risk_payload_keys = normalize_signal_list(profile.high_risk_payload_keys);
    let scoring = sanitize_risk_scoring(profile.scoring);

    let haystack = operation_risk_haystack(operation);
    let haystack_lower = haystack.to_ascii_lowercase();

    let matched_keywords: Vec<String> = keywords
        .iter()
        .filter(|keyword| haystack_lower.contains(keyword.as_str()))
        .cloned()
        .collect();

    let matched_tool_name = operation_tool_name(operation)
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| high_risk_tool_names.iter().any(|value| value == name))
        .map(|name| vec![format!("tool:{name}")])
        .unwrap_or_default();

    let payload_keys = operation_payload_keys(operation);
    let matched_payload_keys: Vec<String> = payload_keys
        .iter()
        .map(|key| key.trim().to_ascii_lowercase())
        .filter(|key| high_risk_payload_keys.iter().any(|value| value == key))
        .map(|key| format!("payload_key:{key}"))
        .collect();

    let mut matched = Vec::new();
    matched.extend(matched_keywords.clone());
    matched.extend(matched_tool_name.clone());
    matched.extend(matched_payload_keys.clone());
    matched.sort();
    matched.dedup();

    let keyword_score = (matched_keywords.len().min(scoring.keyword_hit_cap) as u16)
        * u16::from(scoring.keyword_weight);
    let tool_score = if matched_tool_name.is_empty() {
        0
    } else {
        u16::from(scoring.tool_name_weight)
    };
    let payload_key_score = (matched_payload_keys.len().min(scoring.payload_key_hit_cap) as u16)
        * u16::from(scoring.payload_key_weight);
    let risk_score = keyword_score
        .saturating_add(tool_score)
        .saturating_add(payload_key_score)
        .min(100) as u8;

    if matched.is_empty() || risk_score < scoring.high_risk_threshold {
        (ApprovalRiskLevel::Low, Vec::new(), 0)
    } else {
        (ApprovalRiskLevel::High, matched, risk_score)
    }
}

fn resolve_approval_risk_profile(policy: &HumanApprovalSpec) -> ApprovalRiskProfile {
    let mut profile = policy
        .risk_profile_path
        .as_deref()
        .and_then(load_approval_risk_profile_from_path)
        .unwrap_or_else(bundled_approval_risk_profile);

    if !policy.high_risk_keywords.is_empty() {
        profile.high_risk_keywords = policy.high_risk_keywords.clone();
    }
    if !policy.high_risk_tool_names.is_empty() {
        profile.high_risk_tool_names = policy.high_risk_tool_names.clone();
    }
    if !policy.high_risk_payload_keys.is_empty() {
        profile.high_risk_payload_keys = policy.high_risk_payload_keys.clone();
    }

    profile
}

fn load_approval_risk_profile_from_path(path: &str) -> Option<ApprovalRiskProfile> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str::<ApprovalRiskProfile>(&content).ok()
}

fn bundled_approval_risk_profile() -> ApprovalRiskProfile {
    BUNDLED_APPROVAL_RISK_PROFILE
        .get_or_init(|| {
            let raw = include_str!("../../config/approval-medium-balanced.json");
            serde_json::from_str(raw)
                .or_else(|_| serde_json::from_str::<ApprovalRiskProfile>("{}"))
                .unwrap_or_else(|_| ApprovalRiskProfile {
                    high_risk_keywords: Vec::new(),
                    high_risk_tool_names: Vec::new(),
                    high_risk_payload_keys: Vec::new(),
                    scoring: ApprovalRiskScoring::default(),
                })
        })
        .clone()
}

pub(super) fn normalize_signal_list(list: Vec<String>) -> Vec<String> {
    let mut normalized: Vec<String> = list
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn sanitize_risk_scoring(mut scoring: ApprovalRiskScoring) -> ApprovalRiskScoring {
    if scoring.keyword_hit_cap == 0 {
        scoring.keyword_hit_cap = 1;
    }
    if scoring.payload_key_hit_cap == 0 {
        scoring.payload_key_hit_cap = 1;
    }
    if scoring.high_risk_threshold == 0 {
        scoring.high_risk_threshold = 20;
    }
    scoring
}

fn operation_tool_name(operation: &OperationSpec) -> Option<&str> {
    #[allow(clippy::wildcard_enum_match_arm)]
    match operation {
        OperationSpec::ToolCore { tool_name, .. } => Some(tool_name.as_str()),
        OperationSpec::ToolExtension {
            extension_action, ..
        } => Some(extension_action.as_str()),
        OperationSpec::ProgrammaticToolCall { caller, .. } => Some(caller.as_str()),
        _ => None,
    }
}

fn operation_payload_keys(operation: &OperationSpec) -> Vec<String> {
    match operation {
        OperationSpec::Task { payload, .. }
        | OperationSpec::ConnectorLegacy { payload, .. }
        | OperationSpec::ConnectorCore { payload, .. }
        | OperationSpec::ConnectorExtension { payload, .. }
        | OperationSpec::RuntimeCore { payload, .. }
        | OperationSpec::RuntimeExtension { payload, .. }
        | OperationSpec::ToolCore { payload, .. }
        | OperationSpec::ToolExtension { payload, .. }
        | OperationSpec::MemoryCore { payload, .. }
        | OperationSpec::MemoryExtension { payload, .. } => {
            let mut keys = Vec::new();
            collect_json_keys(payload, &mut keys);
            keys
        }
        OperationSpec::ToolSearch { .. } => {
            let mut keys = Vec::new();
            keys.extend(
                [
                    "query",
                    "limit",
                    "trust_tiers",
                    "include_deferred",
                    "include_examples",
                ]
                .iter()
                .map(|value| (*value).to_owned()),
            );
            keys
        }
        OperationSpec::PluginInventory { .. } => {
            let mut keys = Vec::new();
            keys.extend(
                [
                    "query",
                    "limit",
                    "include_ready",
                    "include_blocked",
                    "include_deferred",
                    "include_examples",
                ]
                .iter()
                .map(|value| (*value).to_owned()),
            );
            keys
        }
        OperationSpec::PluginPreflight {
            policy_path,
            policy_sha256,
            policy_signature,
            ..
        } => {
            let mut keys = Vec::new();
            keys.extend(
                [
                    "query",
                    "limit",
                    "profile",
                    "include_passed",
                    "include_warned",
                    "include_blocked",
                    "include_deferred",
                    "include_examples",
                ]
                .iter()
                .map(|value| (*value).to_owned()),
            );
            if policy_path.is_some() {
                keys.push("policy_path".to_owned());
            }
            if policy_sha256.is_some() {
                keys.push("policy_sha256".to_owned());
            }
            if policy_signature.is_some() {
                keys.push("policy_signature".to_owned());
                keys.push("policy_signature.algorithm".to_owned());
                keys.push("policy_signature.public_key_base64".to_owned());
                keys.push("policy_signature.signature_base64".to_owned());
            }
            keys
        }
        OperationSpec::ProgrammaticToolCall {
            allowed_connectors,
            connector_rate_limits,
            connector_circuit_breakers,
            concurrency,
            steps,
            ..
        } => {
            let mut keys = Vec::new();
            keys.extend(
                [
                    "caller",
                    "max_calls",
                    "include_intermediate",
                    "connector_rate_limits",
                    "connector_circuit_breakers",
                    "concurrency",
                    "return_step",
                    "steps",
                ]
                .iter()
                .map(|value| (*value).to_owned()),
            );
            keys.push("max_in_flight".to_owned());
            keys.push(concurrency.max_in_flight.to_string());
            keys.push("min_in_flight".to_owned());
            keys.push(concurrency.min_in_flight.to_string());
            keys.push("fairness".to_owned());
            keys.push(concurrency.fairness.as_str().to_owned());
            keys.push("adaptive_budget".to_owned());
            keys.push(concurrency.adaptive_budget.to_string());
            keys.push("high_weight".to_owned());
            keys.push(concurrency.high_weight.to_string());
            keys.push("normal_weight".to_owned());
            keys.push(concurrency.normal_weight.to_string());
            keys.push("low_weight".to_owned());
            keys.push(concurrency.low_weight.to_string());
            keys.push("adaptive_recovery_successes".to_owned());
            keys.push(concurrency.adaptive_recovery_successes.to_string());
            keys.push("adaptive_upshift_step".to_owned());
            keys.push(concurrency.adaptive_upshift_step.to_string());
            keys.push("adaptive_downshift_step".to_owned());
            keys.push(concurrency.adaptive_downshift_step.to_string());
            keys.push("adaptive_reduce_on".to_owned());
            for rule in &concurrency.adaptive_reduce_on {
                keys.push(rule.as_str().to_owned());
            }
            keys.extend(allowed_connectors.iter().cloned());
            for (connector_name, limit) in connector_rate_limits {
                keys.push("connector_name".to_owned());
                keys.push(connector_name.clone());
                keys.push("min_interval_ms".to_owned());
                keys.push(limit.min_interval_ms.to_string());
            }
            for (connector_name, policy) in connector_circuit_breakers {
                keys.push("connector_name".to_owned());
                keys.push(connector_name.clone());
                keys.push("failure_threshold".to_owned());
                keys.push(policy.failure_threshold.to_string());
                keys.push("cooldown_ms".to_owned());
                keys.push(policy.cooldown_ms.to_string());
                keys.push("half_open_max_calls".to_owned());
                keys.push(policy.half_open_max_calls.to_string());
                keys.push("success_threshold".to_owned());
                keys.push(policy.success_threshold.to_string());
            }
            for step in steps {
                keys.push("step_id".to_owned());
                match step {
                    ProgrammaticStep::SetLiteral { value, .. } => {
                        collect_json_keys(value, &mut keys);
                    }
                    ProgrammaticStep::JsonPointer { .. } => {
                        keys.push("pointer".to_owned());
                    }
                    ProgrammaticStep::ConnectorCall {
                        connector_name,
                        operation,
                        priority_class,
                        retry,
                        payload,
                        ..
                    } => {
                        keys.push("connector_name".to_owned());
                        keys.push("operation".to_owned());
                        keys.push("priority_class".to_owned());
                        keys.push(connector_name.clone());
                        keys.push(operation.clone());
                        keys.push(priority_class.as_str().to_owned());
                        if let Some(retry) = retry {
                            keys.push("retry".to_owned());
                            keys.push("max_attempts".to_owned());
                            keys.push("initial_backoff_ms".to_owned());
                            keys.push("max_backoff_ms".to_owned());
                            keys.push("jitter_ratio".to_owned());
                            keys.push("adaptive_jitter".to_owned());
                            keys.push(retry.max_attempts.to_string());
                            keys.push(retry.initial_backoff_ms.to_string());
                            keys.push(retry.max_backoff_ms.to_string());
                            keys.push(retry.jitter_ratio.to_string());
                            keys.push(retry.adaptive_jitter.to_string());
                        }
                        collect_json_keys(payload, &mut keys);
                    }
                    ProgrammaticStep::ConnectorBatch {
                        parallel,
                        continue_on_error,
                        calls,
                        ..
                    } => {
                        keys.push("parallel".to_owned());
                        keys.push(parallel.to_string());
                        keys.push("continue_on_error".to_owned());
                        keys.push(continue_on_error.to_string());
                        keys.push("calls".to_owned());
                        for call in calls {
                            keys.push("call_id".to_owned());
                            keys.push(call.call_id.clone());
                            keys.push("connector_name".to_owned());
                            keys.push("operation".to_owned());
                            keys.push("priority_class".to_owned());
                            keys.push(call.connector_name.clone());
                            keys.push(call.operation.clone());
                            keys.push(call.priority_class.as_str().to_owned());
                            if let Some(retry) = &call.retry {
                                keys.push("retry".to_owned());
                                keys.push("max_attempts".to_owned());
                                keys.push("initial_backoff_ms".to_owned());
                                keys.push("max_backoff_ms".to_owned());
                                keys.push("jitter_ratio".to_owned());
                                keys.push("adaptive_jitter".to_owned());
                                keys.push(retry.max_attempts.to_string());
                                keys.push(retry.initial_backoff_ms.to_string());
                                keys.push(retry.max_backoff_ms.to_string());
                                keys.push(retry.jitter_ratio.to_string());
                                keys.push(retry.adaptive_jitter.to_string());
                            }
                            collect_json_keys(&call.payload, &mut keys);
                        }
                    }
                    ProgrammaticStep::Conditional {
                        from_step,
                        pointer,
                        equals,
                        exists,
                        when_true,
                        when_false,
                        ..
                    } => {
                        keys.push("from_step".to_owned());
                        keys.push(from_step.clone());
                        if let Some(pointer) = pointer {
                            keys.push("pointer".to_owned());
                            keys.push(pointer.clone());
                        }
                        if let Some(equals) = equals {
                            keys.push("equals".to_owned());
                            collect_json_keys(equals, &mut keys);
                        }
                        if let Some(exists) = exists {
                            keys.push("exists".to_owned());
                            keys.push(exists.to_string());
                        }
                        keys.push("when_true".to_owned());
                        collect_json_keys(when_true, &mut keys);
                        if let Some(when_false) = when_false {
                            keys.push("when_false".to_owned());
                            collect_json_keys(when_false, &mut keys);
                        }
                    }
                }
            }
            keys
        }
    }
}

fn collect_json_keys(value: &Value, keys: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                keys.push(key.clone());
                collect_json_keys(child, keys);
            }
        }
        Value::Array(list) => {
            for child in list {
                collect_json_keys(child, keys);
            }
        }
        Value::String(_) | Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn operation_risk_haystack(operation: &OperationSpec) -> String {
    let mut text = String::new();
    text.push_str(operation_approval_kind(operation));
    text.push(' ');
    text.push_str(&operation_approval_key(operation));
    text.push(' ');
    for value in operation_payload_strings(operation) {
        text.push_str(&value);
        text.push(' ');
    }
    text
}

fn operation_payload_strings(operation: &OperationSpec) -> Vec<String> {
    match operation {
        OperationSpec::Task { payload, .. }
        | OperationSpec::ConnectorLegacy { payload, .. }
        | OperationSpec::ConnectorCore { payload, .. }
        | OperationSpec::ConnectorExtension { payload, .. }
        | OperationSpec::RuntimeCore { payload, .. }
        | OperationSpec::RuntimeExtension { payload, .. }
        | OperationSpec::ToolCore { payload, .. }
        | OperationSpec::ToolExtension { payload, .. }
        | OperationSpec::MemoryCore { payload, .. }
        | OperationSpec::MemoryExtension { payload, .. } => {
            let mut values = Vec::new();
            collect_json_strings(payload, &mut values);
            values
        }
        OperationSpec::ToolSearch {
            query,
            limit,
            trust_tiers,
            include_deferred,
            include_examples,
        } => {
            let mut values = vec![
                query.clone(),
                limit.to_string(),
                include_deferred.to_string(),
                include_examples.to_string(),
            ];
            values.extend(trust_tiers.iter().map(|tier| tier.as_str().to_owned()));
            values
        }
        OperationSpec::PluginInventory {
            query,
            limit,
            include_ready,
            include_blocked,
            include_deferred,
            include_examples,
        } => {
            vec![
                query.clone(),
                limit.to_string(),
                include_ready.to_string(),
                include_blocked.to_string(),
                include_deferred.to_string(),
                include_examples.to_string(),
            ]
        }
        OperationSpec::PluginPreflight {
            query,
            limit,
            profile,
            policy_path,
            policy_sha256,
            policy_signature,
            include_passed,
            include_warned,
            include_blocked,
            include_deferred,
            include_examples,
        } => {
            vec![
                query.clone(),
                limit.to_string(),
                profile.as_str().to_owned(),
                policy_path.clone().unwrap_or_default(),
                policy_sha256.clone().unwrap_or_default(),
                policy_signature
                    .as_ref()
                    .map(|signature| signature.algorithm.clone())
                    .unwrap_or_default(),
                include_passed.to_string(),
                include_warned.to_string(),
                include_blocked.to_string(),
                include_deferred.to_string(),
                include_examples.to_string(),
            ]
        }
        OperationSpec::ProgrammaticToolCall {
            caller,
            max_calls,
            include_intermediate,
            allowed_connectors,
            connector_rate_limits,
            connector_circuit_breakers,
            concurrency,
            return_step,
            steps,
        } => {
            let mut values = vec![
                caller.clone(),
                max_calls.to_string(),
                include_intermediate.to_string(),
                concurrency.max_in_flight.to_string(),
                concurrency.min_in_flight.to_string(),
                concurrency.fairness.as_str().to_owned(),
                concurrency.adaptive_budget.to_string(),
                concurrency.high_weight.to_string(),
                concurrency.normal_weight.to_string(),
                concurrency.low_weight.to_string(),
                concurrency.adaptive_recovery_successes.to_string(),
                concurrency.adaptive_upshift_step.to_string(),
                concurrency.adaptive_downshift_step.to_string(),
            ];
            for rule in &concurrency.adaptive_reduce_on {
                values.push(rule.as_str().to_owned());
            }
            values.extend(allowed_connectors.iter().cloned());
            for (connector_name, limit) in connector_rate_limits {
                values.push(connector_name.clone());
                values.push(limit.min_interval_ms.to_string());
            }
            for (connector_name, policy) in connector_circuit_breakers {
                values.push(connector_name.clone());
                values.push(policy.failure_threshold.to_string());
                values.push(policy.cooldown_ms.to_string());
                values.push(policy.half_open_max_calls.to_string());
                values.push(policy.success_threshold.to_string());
            }
            if let Some(return_step) = return_step {
                values.push(return_step.clone());
            }
            for step in steps {
                match step {
                    ProgrammaticStep::SetLiteral { step_id, value } => {
                        values.push(step_id.clone());
                        collect_json_strings(value, &mut values);
                    }
                    ProgrammaticStep::JsonPointer {
                        step_id,
                        from_step,
                        pointer,
                    } => {
                        values.push(step_id.clone());
                        values.push(from_step.clone());
                        values.push(pointer.clone());
                    }
                    ProgrammaticStep::ConnectorCall {
                        step_id,
                        connector_name,
                        operation,
                        priority_class,
                        retry,
                        payload,
                        ..
                    } => {
                        values.push(step_id.clone());
                        values.push(connector_name.clone());
                        values.push(operation.clone());
                        values.push(priority_class.as_str().to_owned());
                        if let Some(retry) = retry {
                            values.push(retry.max_attempts.to_string());
                            values.push(retry.initial_backoff_ms.to_string());
                            values.push(retry.max_backoff_ms.to_string());
                            values.push(retry.jitter_ratio.to_string());
                            values.push(retry.adaptive_jitter.to_string());
                        }
                        collect_json_strings(payload, &mut values);
                    }
                    ProgrammaticStep::ConnectorBatch {
                        step_id,
                        parallel,
                        continue_on_error,
                        calls,
                    } => {
                        values.push(step_id.clone());
                        values.push(parallel.to_string());
                        values.push(continue_on_error.to_string());
                        for call in calls {
                            values.push(call.call_id.clone());
                            values.push(call.connector_name.clone());
                            values.push(call.operation.clone());
                            values.push(call.priority_class.as_str().to_owned());
                            if let Some(retry) = &call.retry {
                                values.push(retry.max_attempts.to_string());
                                values.push(retry.initial_backoff_ms.to_string());
                                values.push(retry.max_backoff_ms.to_string());
                                values.push(retry.jitter_ratio.to_string());
                                values.push(retry.adaptive_jitter.to_string());
                            }
                            collect_json_strings(&call.payload, &mut values);
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
                        values.push(step_id.clone());
                        values.push(from_step.clone());
                        if let Some(pointer) = pointer {
                            values.push(pointer.clone());
                        }
                        if let Some(exists) = exists {
                            values.push(exists.to_string());
                        }
                        if let Some(equals) = equals {
                            collect_json_strings(equals, &mut values);
                        }
                        collect_json_strings(when_true, &mut values);
                        if let Some(when_false) = when_false {
                            collect_json_strings(when_false, &mut values);
                        }
                    }
                }
            }
            values
        }
    }
}

fn collect_json_strings(value: &Value, values: &mut Vec<String>) {
    match value {
        Value::String(string) => values.push(string.clone()),
        Value::Array(array) => {
            for entry in array {
                collect_json_strings(entry, values);
            }
        }
        Value::Object(map) => {
            for (key, entry) in map {
                values.push(key.clone());
                collect_json_strings(entry, values);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn is_operation_preapproved(operation_key: &str, approvals: &[String]) -> bool {
    let operation_key_lower = operation_key.to_ascii_lowercase();
    approvals.iter().any(|raw| {
        let normalized = raw.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return false;
        }
        if normalized == "*" {
            return true;
        }
        if let Some(prefix) = normalized.strip_suffix('*') {
            return operation_key_lower.starts_with(prefix);
        }
        normalized == operation_key_lower
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_preflight_payload_keys_only_include_present_optional_fields() {
        let operation = OperationSpec::PluginPreflight {
            query: "search".to_owned(),
            limit: 5,
            profile: PluginPreflightProfile::RuntimeActivation,
            policy_path: None,
            policy_sha256: None,
            policy_signature: None,
            include_passed: true,
            include_warned: true,
            include_blocked: true,
            include_deferred: false,
            include_examples: false,
        };

        let keys = operation_payload_keys(&operation);

        assert!(!keys.iter().any(|key| key == "policy_path"));
        assert!(!keys.iter().any(|key| key == "policy_sha256"));
        assert!(!keys.iter().any(|key| key == "policy_signature"));
    }

    #[test]
    fn plugin_preflight_payload_keys_include_nested_signature_fields() {
        let operation = OperationSpec::PluginPreflight {
            query: "search".to_owned(),
            limit: 5,
            profile: PluginPreflightProfile::RuntimeActivation,
            policy_path: Some("/tmp/policy.json".to_owned()),
            policy_sha256: Some("abc123".to_owned()),
            policy_signature: Some(SecurityProfileSignatureSpec {
                algorithm: "ed25519".to_owned(),
                public_key_base64: "cHVibGljLWtleQ==".to_owned(),
                signature_base64: "c2lnbmF0dXJl".to_owned(),
            }),
            include_passed: true,
            include_warned: true,
            include_blocked: true,
            include_deferred: false,
            include_examples: false,
        };

        let keys = operation_payload_keys(&operation);

        assert!(keys.iter().any(|key| key == "policy_path"));
        assert!(keys.iter().any(|key| key == "policy_sha256"));
        assert!(keys.iter().any(|key| key == "policy_signature"));
        assert!(keys.iter().any(|key| key == "policy_signature.algorithm"));
        assert!(
            keys.iter()
                .any(|key| key == "policy_signature.public_key_base64")
        );
        assert!(
            keys.iter()
                .any(|key| key == "policy_signature.signature_base64")
        );
    }

    #[test]
    fn plugin_preflight_payload_keys_keep_algorithm_field_when_signature_is_present() {
        let operation = OperationSpec::PluginPreflight {
            query: "search".to_owned(),
            limit: 5,
            profile: PluginPreflightProfile::RuntimeActivation,
            policy_path: None,
            policy_sha256: None,
            policy_signature: Some(SecurityProfileSignatureSpec {
                algorithm: String::new(),
                public_key_base64: "cHVibGljLWtleQ==".to_owned(),
                signature_base64: "c2lnbmF0dXJl".to_owned(),
            }),
            include_passed: true,
            include_warned: true,
            include_blocked: true,
            include_deferred: false,
            include_examples: false,
        };

        let keys = operation_payload_keys(&operation);

        assert!(keys.iter().any(|key| key == "policy_signature.algorithm"));
    }
}
