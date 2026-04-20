use super::*;

pub(crate) fn resolve_tool_invoke_request(
    request: &ToolCoreRequest,
) -> Result<(ResolvedToolExecution, ToolCoreRequest), String> {
    if canonical_tool_name(request.tool_name.as_str()) != "tool.invoke" {
        return Err(format!(
            "tool_invoke_required: expected `tool.invoke`, got `{}`",
            request.tool_name
        ));
    }

    let payload = request
        .payload
        .as_object()
        .ok_or_else(|| "tool.invoke payload must be an object".to_owned())?;
    let tool_id = payload
        .get("tool_id")
        .and_then(Value::as_str)
        .map(canonical_tool_name)
        .ok_or_else(|| "tool.invoke requires payload.tool_id".to_owned())?;
    let lease = payload
        .get("lease")
        .and_then(Value::as_str)
        .ok_or_else(|| "tool.invoke requires payload.lease".to_owned())?;
    let mut arguments = payload
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    {
        let arguments_object = arguments
            .as_object_mut()
            .ok_or_else(|| "tool.invoke payload.arguments must be an object".to_owned())?;
        if let Some(internal_context) = payload
            .get(LOONG_INTERNAL_TOOL_CONTEXT_KEY)
            .or_else(|| payload.get(LOONG_INTERNAL_TOOL_CONTEXT_KEY))
        {
            merge_trusted_internal_tool_context_into_arguments(arguments_object, internal_context)?;
        }
    }

    let routed_hidden_tool_name = super::route_hidden_discoverable_tool_name(tool_id, &arguments);
    let tool_lease_id = match routed_hidden_tool_name {
        Ok(_resolved_hidden_tool_name)
            if matches!(
                tool_id,
                super::HIDDEN_AGENT_TOOL_NAME
                    | super::HIDDEN_SKILLS_TOOL_NAME
                    | super::HIDDEN_CHANNEL_TOOL_NAME
            ) =>
        {
            tool_id
        }
        _ => tool_id,
    };
    tool_lease_authority::validate_tool_lease(tool_lease_id, lease, payload)?;

    if matches!(
        tool_id,
        super::HIDDEN_AGENT_TOOL_NAME
            | super::HIDDEN_SKILLS_TOOL_NAME
            | super::HIDDEN_CHANNEL_TOOL_NAME
    ) && let Some(arguments_object) = arguments.as_object_mut()
    {
        arguments_object.remove("operation");
    }

    let resolved_tool_name = routed_hidden_tool_name.unwrap_or(tool_id);
    let resolved = resolve_tool_execution(resolved_tool_name)
        .ok_or_else(|| format!("tool_not_found: unknown tool `{resolved_tool_name}`"))?;
    let resolved_tool_name = resolved.canonical_name;
    if is_provider_exposed_tool_name(resolved_tool_name) {
        return Err(format!(
            "tool_not_provider_exposed: {} must be called directly as a core tool",
            resolved_tool_name
        ));
    }

    Ok((
        resolved,
        ToolCoreRequest {
            tool_name: resolved_tool_name.to_owned(),
            payload: arguments,
        },
    ))
}

pub(crate) fn execute_tool_invoke_tool_with_config(
    request: ToolCoreRequest,
    config: &runtime_config::ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let inner_arguments = request.payload.get("arguments").unwrap_or(&Value::Null);
    ensure_untrusted_payload_does_not_use_reserved_internal_tool_context(
        request.tool_name.as_str(),
        inner_arguments,
        "payload.arguments",
    )?;
    let (entry, effective_request) = resolve_tool_invoke_request(&request)?;
    match entry.execution_kind {
        ToolExecutionKind::Core => {
            execute_discoverable_tool_core_with_config(effective_request, config)
        }
        ToolExecutionKind::App => Err(format!(
            "tool_requires_app_dispatcher: {}",
            entry.canonical_name
        )),
    }
}

pub(crate) fn issue_tool_lease(
    tool_id: &str,
    payload: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    tool_lease_authority::issue_tool_lease(tool_id, payload)
}

#[allow(dead_code)]
pub(crate) fn bridge_provider_tool_call_with_scope(
    tool_name: &str,
    args_json: Value,
    session_id: Option<&str>,
    turn_id: Option<&str>,
) -> (String, Value) {
    let canonical_name = canonical_tool_name(tool_name).to_owned();
    let Some(entry) = catalog::find_tool_catalog_entry(canonical_name.as_str()) else {
        return (canonical_name, args_json);
    };
    if !entry.is_discoverable() {
        return (canonical_name, args_json);
    }

    let mut lease_payload = serde_json::Map::new();
    inject_tool_lease_binding(&mut lease_payload, None, session_id, turn_id);
    let grouped_hidden_tool_name = hidden_facade_tool_name_for_hidden_tool(entry.canonical_name);
    let tool_id = grouped_hidden_tool_name.unwrap_or(entry.canonical_name);
    let lease = match tool_lease_authority::issue_tool_lease(tool_id, &lease_payload) {
        Ok(lease) => lease,
        Err(error) => format!("tool-lease-error:{error}"),
    };
    let mut arguments = args_json;
    if let Some(operation) = hidden_operation_for_tool_name(entry.canonical_name)
        && let Some(arguments_object) = arguments.as_object_mut()
    {
        arguments_object
            .entry("operation".to_owned())
            .or_insert_with(|| json!(operation));
    }

    let mut outer_payload = serde_json::Map::new();
    outer_payload.insert("tool_id".to_owned(), json!(tool_id));
    outer_payload.insert("lease".to_owned(), json!(lease));
    outer_payload.insert("arguments".to_owned(), arguments);
    for (key, value) in lease_payload {
        outer_payload.insert(key, value);
    }
    ("tool.invoke".to_owned(), Value::Object(outer_payload))
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn synthesize_test_provider_tool_call(
    tool_name: &str,
    args_json: Value,
) -> (String, Value) {
    bridge_provider_tool_call_with_scope(tool_name, args_json, None, None)
}

#[cfg(test)]
pub(crate) fn synthesize_test_provider_tool_call_with_scope(
    tool_name: &str,
    args_json: Value,
    session_id: Option<&str>,
    turn_id: Option<&str>,
) -> (String, Value) {
    bridge_provider_tool_call_with_scope(tool_name, args_json, session_id, turn_id)
}
