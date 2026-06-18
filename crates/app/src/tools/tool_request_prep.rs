use std::collections::BTreeSet;

use loong_kernel::{Capability, ToolCoreRequest};
use serde_json::Value;

use crate::tools::inject_tool_lease_binding;

pub(crate) const TOOL_SEARCH_GRANTED_CAPABILITIES_FIELD: &str = "_granted_capabilities";

pub fn summarize_tool_request_for_display(tool_name: &str, payload: Value) -> Value {
    let canonical_tool_name = super::canonical_tool_name(tool_name);

    #[cfg(feature = "tool-shell")]
    let payload = super::shell_request_prep::normalize_shell_payload_for_request(
        canonical_tool_name,
        payload,
    );

    let is_shell_like_request =
        canonical_tool_name == super::SHELL_EXEC_TOOL_NAME || canonical_tool_name == "bash.exec";
    if !is_shell_like_request {
        return payload;
    }

    #[cfg(feature = "tool-shell")]
    {
        super::shell_request_prep::summarize_shell_request_for_display(payload)
    }
    #[cfg(not(feature = "tool-shell"))]
    {
        payload
    }
}

pub(crate) fn prepare_kernel_tool_request(
    mut request: ToolCoreRequest,
    granted_capabilities: &BTreeSet<Capability>,
    token_id: Option<&str>,
    session_id: Option<&str>,
    turn_id: Option<&str>,
) -> ToolCoreRequest {
    #[cfg(feature = "tool-shell")]
    {
        request = super::shell_request_prep::normalize_shell_request_for_execution(request);
    }
    let canonical_tool_name = super::canonical_tool_name(request.tool_name.as_str());
    if !matches!(canonical_tool_name, "tool.search" | "tool.invoke") {
        return request;
    }

    if let Value::Object(payload) = &mut request.payload {
        if canonical_tool_name == "tool.search" {
            let granted_capabilities_json =
                serde_json::to_value(granted_capabilities.iter().copied().collect::<Vec<_>>());
            let granted_capabilities_json =
                granted_capabilities_json.unwrap_or_else(|_| Value::Array(Vec::new()));
            payload.insert(
                TOOL_SEARCH_GRANTED_CAPABILITIES_FIELD.to_owned(),
                granted_capabilities_json,
            );
        }
        inject_tool_lease_binding(payload, token_id, session_id, turn_id);
    }

    request
}
