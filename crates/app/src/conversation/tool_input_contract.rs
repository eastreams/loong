use loongclaw_contracts::ToolCoreRequest;
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
    Some(render_repair_guidance_for_issue(
        tool_name, descriptor, &issue,
    ))
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

    let resolved = tools::resolve_tool_invoke_request(request).ok()?;
    let (_, inner_request) = resolved;
    let inner_tool_name = inner_request.tool_name.as_str();

    if inner_tool_name != descriptor_tool_name {
        return None;
    }

    let payload = inner_request.payload;
    Some(payload)
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
    let mut lines = Vec::new();
    let heading = format!("Repair guidance for {tool_name}:");
    lines.push(heading);

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
                .map(|value| format!(" as a {value}"))
                .unwrap_or_default();
            let line = format!("Add required field `{field_path}`{expected_suffix}.");
            lines.push(line);
        }
        ToolInputContractIssue::InvalidFieldType {
            field,
            expected_type,
        } => {
            let field_path = format!("payload.{field}");
            let line = format!("Set `{field_path}` to a {expected_type} value.");
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
        render_tool_input_repair_guidance,
    };
    use crate::tools;
    use loongclaw_contracts::ToolCoreRequest;
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

        assert!(guidance.contains("Repair guidance for file.read:"));
        assert!(guidance.contains("Add required field `payload.path` as a string."));
        assert!(guidance.contains("Expected payload shape: path:string,max_bytes?:integer."));
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
}
