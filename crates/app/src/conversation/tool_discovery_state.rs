use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::analytics::parse_conversation_event;

pub(crate) const TOOL_DISCOVERY_REFRESHED_EVENT_NAME: &str = "tool_discovery_refreshed";
const TOOL_DISCOVERY_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ToolDiscoveryEntry {
    pub tool_id: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    #[serde(default)]
    pub required_fields: Vec<String>,
    #[serde(default)]
    pub required_field_groups: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ToolDiscoveryDiagnostics {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ToolDiscoveryState {
    pub schema_version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_tool_id: Option<String>,
    #[serde(default)]
    pub entries: Vec<ToolDiscoveryEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<ToolDiscoveryDiagnostics>,
}

impl ToolDiscoveryState {
    pub(crate) fn from_tool_search_payload(payload: &Value) -> Option<Self> {
        let payload_object = payload.as_object()?;
        let query = trimmed_string(payload_object.get("query"));
        let exact_tool_id = trimmed_string(payload_object.get("exact_tool_id"));
        let diagnostics = payload_object
            .get("diagnostics")
            .and_then(tool_discovery_diagnostics_from_value);
        let entries = payload_object
            .get("results")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(tool_discovery_entry_from_value)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let returned = payload_object.get("returned").and_then(Value::as_u64);
        let has_state = query.is_some()
            || exact_tool_id.is_some()
            || diagnostics.is_some()
            || returned.is_some();

        if !has_state {
            return None;
        }

        Some(Self {
            schema_version: TOOL_DISCOVERY_SCHEMA_VERSION,
            query,
            exact_tool_id,
            entries,
            diagnostics,
        })
    }

    pub(crate) fn from_event_payload(payload: &Value) -> Option<Self> {
        let mut state = serde_json::from_value::<Self>(payload.clone()).ok()?;

        state.query = normalize_optional_string(state.query);
        state.exact_tool_id = normalize_optional_string(state.exact_tool_id);
        state.entries = state
            .entries
            .into_iter()
            .filter_map(normalize_tool_discovery_entry)
            .collect();
        state.diagnostics = state
            .diagnostics
            .and_then(normalize_tool_discovery_diagnostics);
        state.schema_version = TOOL_DISCOVERY_SCHEMA_VERSION;

        let has_state = state.query.is_some()
            || state.exact_tool_id.is_some()
            || state.diagnostics.is_some()
            || !state.entries.is_empty();

        has_state.then_some(state)
    }

    pub(crate) fn render_delta_prompt(&self) -> String {
        let mut sections = Vec::new();
        let mut entry_lines = Vec::new();

        sections.push("[tool_discovery_delta]".to_owned());
        sections.push("Recent discovery state is advisory context only.".to_owned());
        sections.push(
            "Use tool.invoke with a fresh lease from the current tool.search result.".to_owned(),
        );
        sections.push(
            "If you already know the tool id and need a refreshed card, call tool.search with exact_tool_id."
                .to_owned(),
        );

        if let Some(query) = self.query.as_deref() {
            sections.push(format!("Latest search query: {query}"));
        }

        if let Some(exact_tool_id) = self.exact_tool_id.as_deref() {
            sections.push(format!("Latest exact refresh target: {exact_tool_id}"));
        }

        if let Some(diagnostics) = self.diagnostics.as_ref() {
            sections.push(format!(
                "Latest discovery diagnostics: {}",
                diagnostics.reason
            ));
        }

        if self.entries.is_empty() {
            sections
                .push("Latest discovery result returned no currently visible tools.".to_owned());
            return sections.join("\n\n");
        }

        entry_lines.push("Latest discovered tools:".to_owned());

        for entry in &self.entries {
            entry_lines.push(format!("- {}: {}", entry.tool_id, entry.summary));

            if let Some(search_hint) = entry.search_hint.as_deref() {
                entry_lines.push(format!("  search_hint: {search_hint}"));
            }

            if let Some(argument_hint) = entry.argument_hint.as_deref() {
                entry_lines.push(format!("  argument_hint: {argument_hint}"));
            }

            if !entry.required_fields.is_empty() {
                let required_fields = entry.required_fields.join(", ");
                entry_lines.push(format!("  required_fields: {required_fields}"));
            }

            if !entry.required_field_groups.is_empty() {
                let required_groups = entry
                    .required_field_groups
                    .iter()
                    .map(|group| group.join(" + "))
                    .collect::<Vec<_>>()
                    .join(" | ");
                entry_lines.push(format!("  required_groups: {required_groups}"));
            }

            entry_lines.push(format!(
                "  refresh: tool.search {{ \"exact_tool_id\": \"{}\" }}",
                entry.tool_id
            ));
        }

        sections.push(entry_lines.join("\n"));
        sections.join("\n\n")
    }
}

pub(crate) fn latest_tool_discovery_state_from_assistant_contents(
    assistant_contents: &[String],
) -> Option<ToolDiscoveryState> {
    for content in assistant_contents.iter().rev() {
        let Some(record) = parse_conversation_event(content) else {
            continue;
        };

        if record.event != TOOL_DISCOVERY_REFRESHED_EVENT_NAME {
            continue;
        }

        let state = ToolDiscoveryState::from_event_payload(&record.payload);

        if state.is_some() {
            return state;
        }
    }

    None
}

fn tool_discovery_entry_from_value(value: &Value) -> Option<ToolDiscoveryEntry> {
    let entry_object = value.as_object()?;
    let tool_id = trimmed_string(entry_object.get("tool_id"))?;
    let summary = trimmed_string(entry_object.get("summary"))?;
    let search_hint = trimmed_string(entry_object.get("search_hint"));
    let argument_hint = trimmed_string(entry_object.get("argument_hint"));
    let required_fields = string_array(entry_object.get("required_fields"));
    let required_field_groups = nested_string_array(entry_object.get("required_field_groups"));

    Some(ToolDiscoveryEntry {
        tool_id,
        summary,
        search_hint,
        argument_hint,
        required_fields,
        required_field_groups,
    })
}

fn tool_discovery_diagnostics_from_value(value: &Value) -> Option<ToolDiscoveryDiagnostics> {
    let diagnostics_object = value.as_object()?;
    let reason = trimmed_string(diagnostics_object.get("reason"))?;

    Some(ToolDiscoveryDiagnostics { reason })
}

fn normalize_tool_discovery_entry(entry: ToolDiscoveryEntry) -> Option<ToolDiscoveryEntry> {
    let tool_id = normalize_optional_string(Some(entry.tool_id))?;
    let summary = normalize_optional_string(Some(entry.summary))?;
    let search_hint = normalize_optional_string(entry.search_hint);
    let argument_hint = normalize_optional_string(entry.argument_hint);
    let required_fields = normalize_string_list(entry.required_fields);
    let required_field_groups = entry
        .required_field_groups
        .into_iter()
        .map(normalize_string_list)
        .filter(|group| !group.is_empty())
        .collect::<Vec<_>>();

    Some(ToolDiscoveryEntry {
        tool_id,
        summary,
        search_hint,
        argument_hint,
        required_fields,
        required_field_groups,
    })
}

fn normalize_tool_discovery_diagnostics(
    diagnostics: ToolDiscoveryDiagnostics,
) -> Option<ToolDiscoveryDiagnostics> {
    let reason = normalize_optional_string(Some(diagnostics.reason))?;

    Some(ToolDiscoveryDiagnostics { reason })
}

fn trimmed_string(value: Option<&Value>) -> Option<String> {
    let value = value?;
    let value = value.as_str()?;
    let value = value.trim();

    (!value.is_empty()).then(|| value.to_owned())
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    let Some(values) = value.as_array() else {
        return Vec::new();
    };

    values
        .iter()
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn nested_string_array(value: Option<&Value>) -> Vec<Vec<String>> {
    let Some(value) = value else {
        return Vec::new();
    };
    let Some(groups) = value.as_array() else {
        return Vec::new();
    };

    groups
        .iter()
        .filter_map(Value::as_array)
        .map(|group| {
            group
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|group| !group.is_empty())
        .collect()
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    let value = value?;
    let value = value.trim();

    (!value.is_empty()).then(|| value.to_owned())
}

fn normalize_string_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .filter_map(|value| normalize_optional_string(Some(value)))
        .collect()
}

#[cfg(test)]
mod state_recovery_tests {
    use serde_json::json;

    use super::{
        TOOL_DISCOVERY_REFRESHED_EVENT_NAME, latest_tool_discovery_state_from_assistant_contents,
    };

    #[test]
    fn latest_tool_discovery_state_from_assistant_contents_uses_latest_event() {
        let older_event = json!({
            "type": "conversation_event",
            "event": TOOL_DISCOVERY_REFRESHED_EVENT_NAME,
            "payload": {
                "schema_version": 1,
                "query": "older query",
                "entries": [
                    {
                        "tool_id": "file.read",
                        "summary": "Older entry"
                    }
                ]
            }
        });
        let newer_event = json!({
            "type": "conversation_event",
            "event": TOOL_DISCOVERY_REFRESHED_EVENT_NAME,
            "payload": {
                "schema_version": 1,
                "query": "latest query",
                "entries": [
                    {
                        "tool_id": "web.fetch",
                        "summary": "Latest entry"
                    }
                ]
            }
        });
        let assistant_contents = vec![
            "ignore malformed content".to_owned(),
            older_event.to_string(),
            newer_event.to_string(),
        ];

        let state =
            latest_tool_discovery_state_from_assistant_contents(assistant_contents.as_slice())
                .expect("latest discovery state should be extracted");

        assert_eq!(state.query.as_deref(), Some("latest query"));
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.entries[0].tool_id, "web.fetch");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_discovery_state_omits_leases_when_built_from_tool_search_payload() {
        let payload = json!({
            "query": "read note.md",
            "returned": 1,
            "results": [
                {
                    "tool_id": "file.read",
                    "summary": "Read a file.",
                    "search_hint": "Use for UTF-8 text files.",
                    "argument_hint": "path:string",
                    "required_fields": ["path"],
                    "required_field_groups": [["path"]],
                    "lease": "lease-file"
                }
            ]
        });

        let state =
            ToolDiscoveryState::from_tool_search_payload(&payload).expect("tool discovery state");
        let encoded = serde_json::to_value(&state).expect("encode state");
        let entry = encoded["entries"][0].as_object().expect("entry object");

        assert_eq!(state.entries[0].tool_id, "file.read");
        assert!(!entry.contains_key("lease"));
    }

    #[test]
    fn tool_discovery_state_renders_exact_refresh_guidance() {
        let state = ToolDiscoveryState {
            schema_version: TOOL_DISCOVERY_SCHEMA_VERSION,
            query: Some("read note.md".to_owned()),
            exact_tool_id: None,
            entries: vec![ToolDiscoveryEntry {
                tool_id: "file.read".to_owned(),
                summary: "Read a file.".to_owned(),
                search_hint: Some("Use for UTF-8 text files.".to_owned()),
                argument_hint: Some("path:string".to_owned()),
                required_fields: vec!["path".to_owned()],
                required_field_groups: vec![vec!["path".to_owned()]],
            }],
            diagnostics: None,
        };
        let rendered = state.render_delta_prompt();

        assert!(rendered.contains("[tool_discovery_delta]"));
        assert!(rendered.contains("exact_tool_id"));
        assert!(rendered.contains("file.read"));
    }
}
