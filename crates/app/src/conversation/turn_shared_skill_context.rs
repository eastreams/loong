use serde_json::Value;
use std::path::PathBuf;

use super::ToolResultLine;
use super::tool_result::envelope_uses_skill_context;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillContext {
    pub skill_id: String,
    pub display_name: String,
    pub instructions: String,
    pub skill_root: Option<PathBuf>,
    pub allowed_tools: Vec<String>,
    pub blocked_tools: Vec<String>,
}

pub fn parse_skill_context(tool_result_text: &str) -> Option<SkillContext> {
    tool_result_text
        .trim()
        .lines()
        .filter_map(parse_skill_context_line)
        .next()
}

pub fn skill_context_from_payload_summary(payload_json: &Value) -> Option<SkillContext> {
    let instructions = payload_json
        .get("instructions")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_owned();
    let skill_id = payload_json
        .get("skill_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("external-skill")
        .to_owned();
    let display_name = payload_json
        .get("display_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(skill_id.as_str())
        .to_owned();
    let skill_root = payload_json
        .get("skill_root")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let metadata = payload_json.get("metadata").and_then(Value::as_object);
    let allowed_tools = metadata
        .and_then(|metadata| metadata.get("allowed_tools"))
        .map(parse_external_skill_tool_restrictions)
        .unwrap_or_default();
    let blocked_tools = metadata
        .and_then(|metadata| metadata.get("blocked_tools"))
        .map(parse_external_skill_tool_restrictions)
        .unwrap_or_default();
    Some(SkillContext {
        skill_id,
        display_name,
        instructions,
        skill_root,
        allowed_tools,
        blocked_tools,
    })
}

fn parse_skill_context_line(line: &str) -> Option<SkillContext> {
    let tool_result_line = ToolResultLine::parse(line)?;
    let envelope = serde_json::to_value(tool_result_line.envelope()).ok()?;
    if !envelope_uses_skill_context(&envelope) {
        return None;
    }
    if tool_result_line.payload_truncated() {
        return None;
    }
    let payload_json = tool_result_line.payload_summary_json()?;
    skill_context_from_payload_summary(&payload_json)
}

fn parse_external_skill_tool_restrictions(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}
