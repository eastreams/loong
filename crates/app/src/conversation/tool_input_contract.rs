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
    if descriptor.name == "read" && request_uses_legacy_file_read_alias(request) {
        return detect_legacy_file_read_issue(&effective_payload);
    }
    detect_tool_input_contract_issue(descriptor, &effective_payload)
}

pub(crate) fn render_tool_input_repair_guidance(
    tool_name: &str,
    request_summary: Option<&Value>,
) -> Option<String> {
    let catalog = tools::tool_catalog();
    let descriptor = catalog.resolve(tool_name)?;
    let request_value = request_summary?;
    if descriptor.name == "read" && uses_legacy_file_read_alias(tool_name) {
        let issue = detect_legacy_file_read_issue(request_value)?;
        return Some(render_tool_input_repair_guidance_for_issue(
            tool_name, descriptor, &issue,
        ));
    }
    let issue = detect_tool_input_contract_issue(descriptor, request_value)?;
    Some(render_tool_input_repair_guidance_for_issue(
        tool_name, descriptor, &issue,
    ))
}

pub(crate) fn render_tool_input_repair_guidance_from_reason(
    tool_name: &str,
    tool_failure_reason: &str,
) -> Option<String> {
    let canonical_tool_name = tools::canonical_tool_name(tool_name);
    if canonical_tool_name == "tool.invoke"
        && tool_failure_reason
            .trim()
            .starts_with("invalid_tool_lease:")
    {
        return Some(render_legacy_tool_invoke_lease_repair_guidance());
    }

    let catalog = tools::tool_catalog();
    let descriptor = catalog.resolve(tool_name)?;
    if descriptor.name == "read" && uses_legacy_file_read_alias(tool_name) {
        let issue = parse_legacy_file_read_issue_from_reason(tool_failure_reason)?;
        return Some(render_tool_input_repair_guidance_for_issue(
            tool_name, descriptor, &issue,
        ));
    }
    render_tool_input_repair_guidance_from_reason_with_descriptor(
        tool_name,
        descriptor,
        tool_failure_reason,
    )
}

pub(crate) fn repair_guidance_visible_tool_name(tool_name: &str) -> String {
    tools::legacy_display_tool_name(tool_name)
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
    let request_tool_name = tools::canonical_tool_name(request.tool_name.as_str());
    if request_tool_name == "tool.invoke" {
        let peeked_request = tools::peek_tool_invoke_request(request)?;
        if peeked_request.tool_name != descriptor.name {
            return None;
        }
        return Some(peeked_request.arguments.clone());
    }

    if request_tool_name == descriptor.name {
        return Some(request.payload.clone());
    }

    let routed_direct_tool_name =
        tools::route_direct_tool_name(request_tool_name, &request.payload).ok()?;
    (routed_direct_tool_name == descriptor.name).then(|| request.payload.clone())
}

fn render_tool_input_repair_guidance_from_reason_with_descriptor(
    tool_name: &str,
    descriptor: &tools::ToolDescriptor,
    tool_failure_reason: &str,
) -> Option<String> {
    if tool_failure_reason
        .trim()
        .starts_with("invalid_tool_lease:")
    {
        return Some(render_invalid_tool_lease_repair_guidance(
            tool_name, descriptor,
        ));
    }

    let issue = parse_tool_input_contract_issue_from_reason(descriptor, tool_failure_reason)?;
    let guidance = render_tool_input_repair_guidance_for_issue(tool_name, descriptor, &issue);
    Some(guidance)
}

fn render_invalid_tool_lease_repair_guidance(
    tool_name: &str,
    descriptor: &tools::ToolDescriptor,
) -> String {
    let visible_tool_name = repair_guidance_visible_tool_name(tool_name);
    let mut lines = Vec::new();
    lines.push(format!("Repair guidance for {visible_tool_name}:"));
    lines.push(
        "The previous hidden-tool lease is stale. Refresh the visible tool request and continue with a fresh lease."
            .to_owned(),
    );
    lines.push(
        "Do not reuse older leases or wrapper-style tool ids from previous rounds.".to_owned(),
    );
    let argument_hint = descriptor.argument_hint();
    let trimmed_hint = argument_hint.trim();
    if !trimmed_hint.is_empty() {
        lines.push(format!("Expected payload shape: {trimmed_hint}."));
    }
    lines.join("\n")
}

fn render_legacy_tool_invoke_lease_repair_guidance() -> String {
    [
        "Repair guidance for tool.invoke:",
        "The previous hidden-tool lease is stale. Refresh the visible tool request and continue with a fresh lease.",
        "Do not reuse older leases or wrapper-style tool ids from previous rounds.",
        "Expected payload shape: tool_id:string,lease:string,arguments:object.",
    ]
    .join("\n")
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

fn uses_legacy_file_read_alias(tool_name: &str) -> bool {
    matches!(tool_name, "file.read" | "file_read")
}

fn legacy_file_read_argument_hint() -> &'static str {
    "path:string,offset?:integer,limit?:integer,max_bytes?:integer"
}

fn request_uses_legacy_file_read_alias(request: &ToolCoreRequest) -> bool {
    if uses_legacy_file_read_alias(request.tool_name.as_str()) {
        return true;
    }
    request
        .payload
        .get("tool_id")
        .and_then(Value::as_str)
        .is_some_and(uses_legacy_file_read_alias)
}

fn detect_legacy_file_read_issue(request_value: &Value) -> Option<ToolInputContractIssue> {
    let request_object = match request_value.as_object() {
        Some(request_object) => request_object,
        None => return Some(ToolInputContractIssue::PayloadMustBeObject),
    };
    let path_value = request_object.get("path");
    match path_value {
        None => Some(ToolInputContractIssue::MissingRequiredField {
            field: "path",
            expected_type: Some("string"),
        }),
        Some(Value::String(path)) if path.trim().is_empty() => {
            Some(ToolInputContractIssue::MissingRequiredField {
                field: "path",
                expected_type: Some("string"),
            })
        }
        Some(Value::String(_)) => None,
        Some(_) => Some(ToolInputContractIssue::InvalidFieldType {
            field: "path",
            expected_type: "string",
        }),
    }
}

fn parse_legacy_file_read_issue_from_reason(
    tool_failure_reason: &str,
) -> Option<ToolInputContractIssue> {
    let reason = strip_tool_input_reason_prefix(tool_failure_reason);
    let file_read_reason = reason
        .strip_prefix("file.read ")
        .or_else(|| reason.strip_prefix("file_read "))
        .map(|suffix| format!("read {suffix}"))
        .unwrap_or_else(|| reason.to_owned());
    if file_read_reason == "read payload must be an object" {
        return Some(ToolInputContractIssue::PayloadMustBeObject);
    }
    if file_read_reason == "read payload.path is required"
        || file_read_reason == "read payload.path is required (string)"
    {
        return Some(ToolInputContractIssue::MissingRequiredField {
            field: "path",
            expected_type: Some("string"),
        });
    }
    if file_read_reason == "read payload.path must be string" {
        return Some(ToolInputContractIssue::InvalidFieldType {
            field: "path",
            expected_type: "string",
        });
    }
    None
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
    let legacy_file_read_alias = uses_legacy_file_read_alias(tool_name);
    let canonical_tool_name = tools::canonical_tool_name(tool_name);
    let visible_tool_name = repair_guidance_visible_tool_name(tool_name);
    let mut lines = Vec::new();
    let heading = format!("Repair guidance for {visible_tool_name}:");
    lines.push(heading);

    if legacy_file_read_alias {
        lines.push(
            "Prefer the direct `read` surface instead of hidden `file.read` when possible."
                .to_owned(),
        );
    } else if visible_tool_name != canonical_tool_name {
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

    let argument_hint = if legacy_file_read_alias {
        legacy_file_read_argument_hint()
    } else {
        descriptor.argument_hint()
    };
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
    use super::{render_tool_input_repair_guidance, render_tool_input_repair_guidance_from_reason};
    use serde_json::json;

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
    fn render_tool_input_repair_guidance_from_reason_preserves_array_type_guidance() {
        let guidance = render_tool_input_repair_guidance_from_reason(
            "shell.exec",
            "tool_preflight_denied: tool input needs repair: shell.exec payload.args must be array",
        )
        .expect("guidance");

        assert!(guidance.contains("Repair guidance for bash:"));
        assert!(guidance.contains(
            "Prefer the direct `bash` surface instead of hidden `shell.exec` when possible."
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
        assert!(guidance.contains("stale"));
        assert!(guidance.contains("fresh lease"));
        assert!(guidance.contains("Do not reuse older leases"));
        assert!(
            guidance
                .contains("Expected payload shape: tool_id:string,lease:string,arguments:object.")
        );
    }
}
