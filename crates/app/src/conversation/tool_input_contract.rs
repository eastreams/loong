use loong_contracts::ToolCoreRequest;
use serde_json::Value;

use crate::tools;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolInputContractIssue {
    PayloadMustBeObject,
    MissingRequiredField {
        field: &'static str,
        expected_type: Option<&'static str>,
    },
    InvalidFieldType {
        field: &'static str,
        expected_type: &'static str,
    },
}

impl ToolInputContractIssue {
    pub(crate) fn reason(&self, tool_name: &str) -> String {
        match self {
            Self::PayloadMustBeObject => {
                format!("{tool_name} payload must be an object")
            }
            Self::MissingRequiredField {
                field,
                expected_type,
            } => {
                let field_path = format!("payload.{field}");
                let expected_suffix = expected_type
                    .map(|value| format!(" ({value})"))
                    .unwrap_or_default();
                format!("{tool_name} {field_path} is required{expected_suffix}")
            }
            Self::InvalidFieldType {
                field,
                expected_type,
            } => {
                let field_path = format!("payload.{field}");
                format!("{tool_name} {field_path} must be {expected_type}")
            }
        }
    }
}

pub(crate) fn detect_repairable_tool_request_issue(
    descriptor: &tools::ToolDescriptor,
    request: &ToolCoreRequest,
) -> Option<ToolInputContractIssue> {
    if descriptor.execution_kind != tools::ToolExecutionKind::Core {
        return None;
    }

    let effective_payload = effective_payload_for_descriptor(descriptor, request)?;
    detect_tool_input_contract_issue(descriptor, &effective_payload)
}

pub(crate) fn render_tool_input_repair_guidance(
    tool_name: &str,
    request_summary: Option<&Value>,
) -> Option<String> {
    let catalog = tools::tool_catalog();
    let descriptor = catalog.resolve(tool_name)?;
    let request_value = request_summary?;
    let issue = detect_tool_input_contract_issue(descriptor, request_value)?;
    Some(render_tool_input_repair_guidance_for_issue(
        tool_name, descriptor, &issue,
    ))
}

pub(crate) fn render_tool_input_repair_guidance_from_reason(
    tool_name: &str,
    tool_failure_reason: &str,
) -> Option<String> {
    let catalog = tools::tool_catalog();
    let descriptor = catalog.resolve(tool_name)?;
    render_tool_input_repair_guidance_from_reason_with_descriptor(
        tool_name,
        descriptor,
        tool_failure_reason,
    )
}

pub(crate) fn repair_guidance_visible_tool_name(tool_name: &str) -> String {
    tools::user_visible_tool_name(tool_name)
}

fn render_tool_input_repair_guidance_for_issue(
    tool_name: &str,
    descriptor: &tools::ToolDescriptor,
    issue: &ToolInputContractIssue,
) -> String {
    render_repair_guidance_for_issue(tool_name, descriptor, issue)
}

fn effective_payload_for_descriptor(
    descriptor: &tools::ToolDescriptor,
    request: &ToolCoreRequest,
) -> Option<Value> {
    let descriptor_tool_name = descriptor.name;
    let request_tool_name = tools::canonical_tool_name(request.tool_name.as_str());

    if request_tool_name == descriptor_tool_name {
        let payload = request.payload.clone();
        return Some(payload);
    }

    if request_tool_name != "tool.invoke" {
        return None;
    }

    let request_object = request.payload.as_object()?;
    let inner_tool_name = request_object
        .get("tool_id")
        .or_else(|| request_object.get("tool_name"))
        .and_then(Value::as_str)
        .map(tools::canonical_tool_name)?;

    if inner_tool_name != descriptor_tool_name {
        return None;
    }

    let payload = request_object
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    Some(payload)
}

fn render_tool_input_repair_guidance_from_reason_with_descriptor(
    tool_name: &str,
    descriptor: &tools::ToolDescriptor,
    tool_failure_reason: &str,
) -> Option<String> {
    if let Some(guidance) =
        render_invalid_tool_lease_repair_guidance(tool_name, tool_failure_reason)
    {
        return Some(guidance);
    }

    let issue = parse_tool_input_contract_issue_from_reason(descriptor, tool_failure_reason)?;
    let guidance = render_tool_input_repair_guidance_for_issue(tool_name, descriptor, &issue);
    Some(guidance)
}

fn strip_tool_input_reason_prefix(reason: &str) -> &str {
    let trimmed_reason = reason.trim();
    let tool_preflight_prefix = "tool_preflight_denied: tool input needs repair: ";

    if let Some(stripped_reason) = trimmed_reason.strip_prefix(tool_preflight_prefix) {
        return stripped_reason;
    }

    let followup_prefix = "tool input needs repair: ";
    let stripped_followup_reason = trimmed_reason.strip_prefix(followup_prefix);
    stripped_followup_reason.unwrap_or(trimmed_reason)
}

fn render_invalid_tool_lease_repair_guidance(
    tool_name: &str,
    tool_failure_reason: &str,
) -> Option<String> {
    if tool_name != "tool.invoke" {
        return None;
    }

    let stripped_reason = tool_failure_reason.trim();
    let mentions_invalid_tool_lease = stripped_reason.contains("invalid_tool_lease:");
    if !mentions_invalid_tool_lease {
        return None;
    }

    Some([
        "Repair guidance for tool.invoke:".to_owned(),
        "The lease is invalid, expired, or scoped to a different turn/session.".to_owned(),
        "Refresh the tool card with `tool.search` before retrying `tool.invoke`.".to_owned(),
        "If you already know the tool id, call `tool.search` with `exact_tool_id` to fetch a fresh lease.".to_owned(),
        "Do not reuse older leases from earlier search results.".to_owned(),
        "Expected payload shape: tool_id:string,lease:string,arguments:object.".to_owned(),
    ]
    .join("\n"))
}

fn parse_tool_input_contract_issue_from_reason(
    descriptor: &tools::ToolDescriptor,
    tool_failure_reason: &str,
) -> Option<ToolInputContractIssue> {
    let tool_name = descriptor.name;
    let reason = strip_tool_input_reason_prefix(tool_failure_reason);
    let object_reason = format!("{tool_name} payload must be an object");

    if reason == object_reason {
        return Some(ToolInputContractIssue::PayloadMustBeObject);
    }

    let prefix = format!("{tool_name} payload.");
    let suffix = reason.strip_prefix(prefix.as_str())?;
    let missing_issue = parse_missing_required_field_issue(descriptor, suffix);

    if missing_issue.is_some() {
        return missing_issue;
    }

    parse_invalid_field_type_issue(descriptor, suffix)
}

fn parse_missing_required_field_issue(
    descriptor: &tools::ToolDescriptor,
    reason_suffix: &str,
) -> Option<ToolInputContractIssue> {
    let split = reason_suffix.split_once(" is required")?;
    let field_name = split.0;
    let type_suffix = split.1;
    let field = descriptor_required_field_name(descriptor, field_name)?;
    let expected_type = expected_type_for_field(descriptor, field);
    let has_type_suffix = !type_suffix.is_empty();

    if has_type_suffix {
        let parsed_type = type_suffix.strip_prefix(" (")?;
        let parsed_type = parsed_type.strip_suffix(')')?;
        let expected_type_matches = expected_type == Some(parsed_type);

        if !expected_type_matches {
            return None;
        }
    }

    let issue = ToolInputContractIssue::MissingRequiredField {
        field,
        expected_type,
    };
    Some(issue)
}

fn parse_invalid_field_type_issue(
    descriptor: &tools::ToolDescriptor,
    reason_suffix: &str,
) -> Option<ToolInputContractIssue> {
    let split = reason_suffix.split_once(" must be ")?;
    let field_name = split.0;
    let expected_type = split.1;
    let field = descriptor_parameter_field_name(descriptor, field_name)?;
    let descriptor_expected_type = expected_type_for_field(descriptor, field)?;
    let expected_type_matches = descriptor_expected_type == expected_type;

    if !expected_type_matches {
        return None;
    }

    let issue = ToolInputContractIssue::InvalidFieldType {
        field,
        expected_type: descriptor_expected_type,
    };
    Some(issue)
}

fn descriptor_required_field_name(
    descriptor: &tools::ToolDescriptor,
    field_name: &str,
) -> Option<&'static str> {
    for required_field in descriptor.required_fields() {
        let is_match = *required_field == field_name;

        if is_match {
            return Some(*required_field);
        }
    }

    None
}

fn descriptor_parameter_field_name(
    descriptor: &tools::ToolDescriptor,
    field_name: &str,
) -> Option<&'static str> {
    for (candidate_field_name, _) in descriptor.parameter_types() {
        let is_match = *candidate_field_name == field_name;

        if is_match {
            return Some(*candidate_field_name);
        }
    }

    None
}

fn indefinite_article(expected_type: &str) -> &'static str {
    match expected_type {
        "array" | "integer" | "object" => "an",
        _ => "a",
    }
}

fn detect_tool_input_contract_issue(
    descriptor: &tools::ToolDescriptor,
    request_value: &Value,
) -> Option<ToolInputContractIssue> {
    let request_object = match request_value.as_object() {
        Some(value) => value,
        None => return Some(ToolInputContractIssue::PayloadMustBeObject),
    };

    for required_field in descriptor.required_fields() {
        let expected_type = expected_type_for_field(descriptor, required_field);
        let value = request_object.get(*required_field);
        let missing = required_field_is_missing(value, expected_type);

        if missing {
            let issue = ToolInputContractIssue::MissingRequiredField {
                field: required_field,
                expected_type,
            };
            return Some(issue);
        }
    }

    for (field_name, expected_type) in descriptor.parameter_types() {
        let value = match request_object.get(*field_name) {
            Some(value) => value,
            None => continue,
        };
        let matches_expected_type = value_matches_expected_type(value, expected_type);

        if !matches_expected_type {
            let issue = ToolInputContractIssue::InvalidFieldType {
                field: field_name,
                expected_type,
            };
            return Some(issue);
        }
    }

    None
}

fn expected_type_for_field(
    descriptor: &tools::ToolDescriptor,
    field_name: &str,
) -> Option<&'static str> {
    for (candidate_field_name, expected_type) in descriptor.parameter_types() {
        let is_match = *candidate_field_name == field_name;

        if is_match {
            return Some(*expected_type);
        }
    }

    None
}

fn required_field_is_missing(value: Option<&Value>, expected_type: Option<&str>) -> bool {
    let value = match value {
        Some(value) => value,
        None => return true,
    };

    if value.is_null() {
        return true;
    }

    let requires_non_empty_string = expected_type == Some("string");

    if !requires_non_empty_string {
        return false;
    }

    let string_value = match value.as_str() {
        Some(value) => value,
        None => return false,
    };
    let trimmed_value = string_value.trim();
    trimmed_value.is_empty()
}

fn value_matches_expected_type(value: &Value, expected_type: &str) -> bool {
    match expected_type {
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => true,
    }
}

fn render_repair_guidance_for_issue(
    tool_name: &str,
    descriptor: &tools::ToolDescriptor,
    issue: &ToolInputContractIssue,
) -> String {
    let canonical_tool_name = tools::canonical_tool_name(tool_name);
    let visible_tool_name = repair_guidance_visible_tool_name(canonical_tool_name);
    let mut lines = Vec::new();
    let heading = format!("Repair guidance for {visible_tool_name}:");
    lines.push(heading);

    if visible_tool_name != canonical_tool_name {
        lines.push(format!(
            "Prefer the direct `{visible_tool_name}` surface instead of hidden `{canonical_tool_name}` when possible."
        ));
    }

    match issue {
        ToolInputContractIssue::PayloadMustBeObject => {
            let line = "Send a JSON object payload instead of a scalar or list.".to_owned();
            lines.push(line);
        }
        ToolInputContractIssue::MissingRequiredField {
            field,
            expected_type,
        } => {
            let field_path = format!("payload.{field}");
            let expected_suffix = expected_type
                .map(|value| {
                    let article = indefinite_article(value);
                    format!(" as {article} {value}")
                })
                .unwrap_or_default();
            let line = format!("Add required field `{field_path}`{expected_suffix}.");
            lines.push(line);
        }
        ToolInputContractIssue::InvalidFieldType {
            field,
            expected_type,
        } => {
            let field_path = format!("payload.{field}");
            let article = indefinite_article(expected_type);
            let line = format!("Set `{field_path}` to {article} {expected_type} value.");
            lines.push(line);
        }
    }

    let argument_hint = descriptor.argument_hint();
    let trimmed_hint = argument_hint.trim();
    let has_argument_hint = !trimmed_hint.is_empty();

    if has_argument_hint {
        let line = format!("Expected payload shape: {trimmed_hint}.");
        lines.push(line);
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::{
        ToolInputContractIssue, detect_repairable_tool_request_issue,
        render_tool_input_repair_guidance, render_tool_input_repair_guidance_from_reason,
    };
    use crate::tools;
    use loong_contracts::ToolCoreRequest;
    use serde_json::json;

    #[test]
    fn detect_repairable_tool_request_issue_unwraps_tool_invoke_for_core_tools() {
        let (tool_name, payload) = tools::synthesize_test_provider_tool_call_with_scope(
            "file.read",
            json!({}),
            Some("session-a"),
            Some("turn-a"),
        );
        let descriptor = tools::tool_catalog()
            .resolve("file.read")
            .expect("file.read descriptor");
        let request = ToolCoreRequest { tool_name, payload };

        let issue = detect_repairable_tool_request_issue(descriptor, &request);

        assert_eq!(
            issue,
            Some(ToolInputContractIssue::MissingRequiredField {
                field: "path",
                expected_type: Some("string"),
            })
        );
    }

    #[test]
    fn render_tool_input_repair_guidance_uses_descriptor_argument_hint() {
        let summary = json!({
            "tool": "file.read",
            "request": {}
        });
        let guidance = render_tool_input_repair_guidance("file.read", summary.get("request"))
            .expect("guidance");

        assert!(guidance.contains("Repair guidance for read:"));
        assert!(guidance.contains(
            "Prefer the direct `read` surface instead of hidden `file.read` when possible."
        ));
        assert!(guidance.contains("Add required field `payload.path` as a string."));
        assert!(guidance.contains(
            "Expected payload shape: path:string,offset?:integer,limit?:integer,max_bytes?:integer."
        ));
    }

    #[test]
    fn detect_repairable_tool_request_issue_preserves_invalid_required_field_types() {
        let (tool_name, payload) = tools::synthesize_test_provider_tool_call_with_scope(
            "file.read",
            json!({
                "path": 7
            }),
            Some("session-a"),
            Some("turn-a"),
        );
        let descriptor = tools::tool_catalog()
            .resolve("file.read")
            .expect("file.read descriptor");
        let request = ToolCoreRequest { tool_name, payload };

        let issue = detect_repairable_tool_request_issue(descriptor, &request);

        assert_eq!(
            issue,
            Some(ToolInputContractIssue::InvalidFieldType {
                field: "path",
                expected_type: "string",
            })
        );
    }

    #[test]
    fn detect_repairable_tool_request_issue_marks_scalar_tool_invoke_arguments_repairable() {
        let descriptor = tools::tool_catalog()
            .resolve("file.read")
            .expect("file.read descriptor");
        let request = ToolCoreRequest {
            tool_name: "tool.invoke".to_owned(),
            payload: json!({
                "tool_id": "file.read",
                "lease": "lease-a",
                "arguments": "README.md"
            }),
        };

        let issue = detect_repairable_tool_request_issue(descriptor, &request);

        assert_eq!(issue, Some(ToolInputContractIssue::PayloadMustBeObject));
    }

    #[test]
    fn render_tool_input_repair_guidance_from_reason_preserves_array_type_guidance() {
        let guidance = render_tool_input_repair_guidance_from_reason(
            "shell.exec",
            "tool_preflight_denied: tool input needs repair: shell.exec payload.args must be array",
        )
        .expect("guidance");

        assert!(guidance.contains("Repair guidance for exec:"));
        assert!(guidance.contains(
            "Prefer the direct `exec` surface instead of hidden `shell.exec` when possible."
        ));
        assert!(guidance.contains("Set `payload.args` to an array value."));
        assert!(guidance.contains(
            "Expected payload shape: command:string,args?:string[],timeout_ms?:integer,cwd?:string."
        ));
    }

    #[test]
    fn render_tool_input_repair_guidance_from_reason_recovers_invalid_tool_lease_refresh_steps() {
        let guidance = render_tool_input_repair_guidance_from_reason(
            "tool.invoke",
            "invalid_tool_lease: expired lease",
        )
        .expect("guidance");

        assert!(guidance.contains("Repair guidance for tool.invoke:"));
        assert!(guidance.contains("tool.search"));
        assert!(guidance.contains("exact_tool_id"));
        assert!(guidance.contains("Do not reuse older leases"));
        assert!(
            guidance
                .contains("Expected payload shape: tool_id:string,lease:string,arguments:object.")
        );
    }
}
