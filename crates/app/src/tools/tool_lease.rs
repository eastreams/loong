use loong_contracts::ToolPath;
use loong_contracts::{Capabilities, Capability};

use super::*;

const TOOL_INVOKE_CAPABILITIES_OVERRIDE_FIELD: &str = "capabilities_override";

#[derive(Debug, Clone, Copy)]
pub(crate) struct PeekedToolInvokeRequest<'a> {
    /// Legacy canonical name used only by context-free display/preflight code.
    pub(crate) tool_name: &'a str,
    pub(crate) arguments: &'a Value,
}

/// Parsed legacy ingress envelope before its target owner is known.
///
/// The raw path must reach Runtime lookup unchanged. The typed plane has no
/// alias layer; only a typed miss may enter legacy catalog canonicalization.
/// Lease validation waits for the selected owner to supply the canonical
/// identity and therefore cannot authorize a different tool by reinterpretation.
#[derive(Debug)]
pub(crate) struct ParsedToolInvoke<'a> {
    pub(crate) path: ToolPath,
    pub(crate) requested_name: &'a str,
    payload: Value,
    capabilities_override: Option<Capabilities>,
    lease: &'a str,
    envelope: &'a serde_json::Map<String, Value>,
}

/// Validated contents of the legacy `tool.invoke` ingress envelope.
///
/// Keeping the capability override beside the resolved invocation prevents
/// normalization from silently restoring the selected tool's default authority.
/// The result is typed-plane data; legacy envelopes are constructed only after
/// a later owner selection proves the target has not migrated.
#[derive(Debug, Clone)]
pub(crate) struct ResolvedLeasedInvocation {
    pub(crate) path: ToolPath,
    pub(crate) payload: Value,
    pub(crate) capabilities_override: Option<Capabilities>,
}

impl ParsedToolInvoke<'_> {
    pub(crate) fn resolve(
        self,
        canonical_path: ToolPath,
    ) -> Result<ResolvedLeasedInvocation, String> {
        tool_lease_authority::validate_tool_lease(
            canonical_path.to_string().as_str(),
            self.lease,
            self.envelope,
        )?;
        Ok(ResolvedLeasedInvocation {
            path: canonical_path,
            payload: self.payload,
            capabilities_override: self.capabilities_override,
        })
    }
}

pub(crate) fn merge_trusted_internal_tool_context_into_arguments(
    arguments: &mut serde_json::Map<String, Value>,
    internal_context: &Value,
) -> Result<(), String> {
    let trusted_context = internal_context.as_object().cloned().ok_or_else(|| {
        format!("tool.invoke payload.{LOONG_INTERNAL_TOOL_CONTEXT_KEY} must be an object")
    })?;
    if let Some(offending_key) = reserved_internal_tool_context_key_in_map(arguments) {
        return Err(format!(
            "tool.invoke payload.arguments.{offending_key} is reserved for trusted internal tool context"
        ));
    }
    let merged_context = Value::Object(trusted_context);
    arguments.insert(LOONG_INTERNAL_TOOL_CONTEXT_KEY.to_owned(), merged_context);
    Ok(())
}

pub(crate) fn peek_tool_invoke_request<'a>(
    tool_name: &'a str,
    payload: &'a Value,
) -> Option<PeekedToolInvokeRequest<'a>> {
    if canonical_tool_name(tool_name) != "tool.invoke" {
        return None;
    }

    let payload = payload.as_object()?;
    let raw_tool_name = payload
        .get("tool_id")
        .and_then(Value::as_str)
        .map(canonical_tool_name)?;
    let arguments = payload.get("arguments").unwrap_or(&Value::Null);
    let tool_name = if raw_tool_name == "agent" {
        super::routing::route_hidden_agent_tool_name(arguments)
            .ok()
            .unwrap_or(raw_tool_name)
    } else {
        raw_tool_name
    };

    Some(PeekedToolInvokeRequest {
        tool_name,
        arguments,
    })
}

pub(crate) fn parse_tool_invoke_request<'a>(
    tool_name: &str,
    payload: &'a Value,
) -> Result<ParsedToolInvoke<'a>, String> {
    if canonical_tool_name(tool_name) != "tool.invoke" {
        return Err(format!(
            "tool_invoke_required: expected `tool.invoke`, got `{}`",
            tool_name
        ));
    }

    let payload = payload
        .as_object()
        .ok_or_else(|| "tool.invoke payload must be an object".to_owned())?;
    let raw_tool_name = payload
        .get("tool_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "tool.invoke requires payload.tool_id".to_owned())?;
    let raw_tool_name = if raw_tool_name == "agent" {
        let arguments = payload.get("arguments").unwrap_or(&Value::Null);
        super::routing::route_hidden_agent_tool_name(arguments).unwrap_or(raw_tool_name)
    } else {
        raw_tool_name
    };
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
        if let Some(internal_context) = payload.get(LOONG_INTERNAL_TOOL_CONTEXT_KEY) {
            merge_trusted_internal_tool_context_into_arguments(arguments_object, internal_context)?;
        }
    }

    let capabilities_override = match payload.get(TOOL_INVOKE_CAPABILITIES_OVERRIDE_FIELD) {
        None | Some(Value::Null) => None,
        Some(raw_override) => {
            let values = raw_override.as_array().ok_or_else(|| {
                "tool.invoke payload.capabilities_override must be an array of capability strings"
                    .to_owned()
            })?;
            let capabilities = values
                .iter()
                .map(|value| {
                    let raw_capability = value.as_str().ok_or_else(|| {
                        "tool.invoke payload.capabilities_override must contain only strings"
                            .to_owned()
                    })?;
                    Capability::parse(raw_capability).ok_or_else(|| {
                        format!(
                            "tool.invoke payload.capabilities_override contains unknown capability `{raw_capability}`"
                        )
                    })
                })
                .collect::<Result<Capabilities, String>>()?;
            Some(capabilities)
        }
    };

    // Leading slash is the unambiguous canonical contracts form. Legacy
    // provider/catalog names contain no slash and remain one opaque segment
    // until an explicit typed miss selects legacy canonicalization.
    let path = if raw_tool_name.starts_with('/') {
        raw_tool_name.parse::<ToolPath>()
    } else {
        ToolPath::new([raw_tool_name])
    }
    .map_err(|error| {
        format!("tool.invoke payload.tool_id is not a valid tool identity: {error}")
    })?;

    Ok(ParsedToolInvoke {
        path,
        requested_name: raw_tool_name,
        payload: arguments,
        capabilities_override,
        lease,
        envelope: payload,
    })
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
    let parsed = parse_tool_invoke_request(request.tool_name.as_str(), &request.payload)?;
    let canonical_name = canonical_tool_name(parsed.requested_name).to_owned();
    let canonical_path = ToolPath::new([canonical_name.as_str()])
        .map_err(|error| format!("legacy tool identity is invalid: {error}"))?;
    let resolved = parsed.resolve(canonical_path)?;
    if resolved.capabilities_override.is_some() {
        return Err(format!(
            "tool.invoke capabilities_override requires a registered typed tool; `{}` is legacy-only",
            resolved.path
        ));
    }
    let execution = resolve_legacy_tool_execution(canonical_name.as_str())
        .ok_or_else(|| format!("tool_not_found: unknown tool `{canonical_name}`"))?;
    match execution {
        ResolvedLegacyToolExecution::Core { .. } => execute_discoverable_tool_core_with_config(
            ToolCoreRequest {
                tool_name: canonical_name,
                payload: resolved.payload,
            },
            config,
        ),
        ResolvedLegacyToolExecution::App { canonical_name } => {
            Err(format!("tool_requires_app_dispatcher: {}", canonical_name))
        }
    }
}

pub(crate) fn issue_tool_lease(
    path: &ToolPath,
    payload: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    tool_lease_authority::issue_tool_lease(path.to_string().as_str(), payload)
}

pub(crate) fn bridge_provider_tool_call_with_scope(
    tool_name: &str,
    args_json: Value,
    session_id: Option<&str>,
    turn_id: Option<&str>,
) -> (String, Value) {
    let canonical_name = canonical_tool_name(tool_name).to_owned();
    let preserve_hidden_shell_exec = canonical_name == SHELL_EXEC_TOOL_NAME;
    if !preserve_hidden_shell_exec
        && let Some(direct_tool_name) = direct_tool_name_for_hidden_tool(canonical_name.as_str())
    {
        return (direct_tool_name.to_owned(), args_json);
    }
    let Some(entry) = catalog::find_tool_catalog_entry(canonical_name.as_str()) else {
        return (canonical_name, args_json);
    };
    if !entry.is_discoverable() {
        return (canonical_name, args_json);
    }

    let mut lease_payload = serde_json::Map::new();
    inject_tool_lease_binding(&mut lease_payload, None, session_id, turn_id);
    let tool_id = entry.canonical_name;
    let lease = match ToolPath::new([tool_id])
        .map_err(|error| error.to_string())
        .and_then(|path| issue_tool_lease(&path, &lease_payload))
    {
        Ok(lease) => lease,
        Err(error) => format!("tool-lease-error:{error}"),
    };
    let arguments = args_json;

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

#[cfg(test)]
#[path = "tool_lease/tests.rs"]
mod tests;
