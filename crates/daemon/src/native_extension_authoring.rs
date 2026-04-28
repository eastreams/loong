use crate::{CliResult, PluginInventoryResult, kernel};
use serde::Serialize;

pub(crate) const PROCESS_STDIO_NATIVE_EXTENSION_CONTRACT: &str = "process_stdio_json_line_v1";
pub(crate) const PROCESS_STDIO_NATIVE_EXTENSION_FACETS: &[&str] =
    &["events", "commands", "resources"];
pub(crate) const PROCESS_STDIO_NATIVE_EXTENSION_METHODS: &[&str] =
    &["extension/event", "extension/command", "extension/resource"];
pub(crate) const PROCESS_STDIO_NATIVE_EXTENSION_EVENTS: &[&str] = &["session_start"];
pub(crate) const PROCESS_STDIO_NATIVE_EXTENSION_HOST_ACTIONS: &[&str] = &[];

#[derive(Debug, Clone, Copy)]
pub(crate) struct RuntimeScaffoldTemplateFile {
    pub relative_path: &'static str,
    pub contents: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProcessStdioNativeExtensionLanguageProfile {
    pub command: &'static str,
    pub args: &'static [&'static str],
    pub process_timeout_ms: u64,
    pub smoke_allow_command: &'static str,
    pub example_package_root: &'static str,
    pub scaffold_files: &'static [RuntimeScaffoldTemplateFile],
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct NativeExtensionAuthoringGuidanceView {
    pub plugin_id: String,
    pub package_root: String,
    pub source_language: String,
    pub bridge_kind: String,
    pub reference_example_path: String,
    pub inventory_command: String,
    pub smoke_allow_command: String,
    pub smoke_test_command: String,
    pub extension_contract: Option<String>,
    pub extension_methods: Vec<String>,
    pub extension_events: Vec<String>,
    pub extension_host_actions: Vec<String>,
    pub extension_metadata_issues: Vec<String>,
}

const PYTHON_EXTENSION_SCAFFOLD_FILES: &[RuntimeScaffoldTemplateFile] =
    &[RuntimeScaffoldTemplateFile {
        relative_path: "index.py",
        contents: PYTHON_EXTENSION_STUB,
    }];
const JAVASCRIPT_EXTENSION_SCAFFOLD_FILES: &[RuntimeScaffoldTemplateFile] =
    &[RuntimeScaffoldTemplateFile {
        relative_path: "index.js",
        contents: JAVASCRIPT_EXTENSION_STUB,
    }];
const GO_EXTENSION_SCAFFOLD_FILES: &[RuntimeScaffoldTemplateFile] =
    &[RuntimeScaffoldTemplateFile {
        relative_path: "main.go",
        contents: GO_EXTENSION_STUB,
    }];
const RUST_EXTENSION_SCAFFOLD_FILES: &[RuntimeScaffoldTemplateFile] = &[
    RuntimeScaffoldTemplateFile {
        relative_path: "Cargo.toml",
        contents: "",
    },
    RuntimeScaffoldTemplateFile {
        relative_path: "src/main.rs",
        contents: RUST_EXTENSION_MAIN_RS,
    },
];

const PYTHON_EXTENSION_ARGS: &[&str] = &["index.py"];
const JAVASCRIPT_EXTENSION_ARGS: &[&str] = &["index.js"];
const GO_EXTENSION_ARGS: &[&str] = &["run", "main.go"];
const RUST_EXTENSION_ARGS: &[&str] = &["run", "--quiet", "--manifest-path", "Cargo.toml"];

pub(crate) fn process_stdio_native_extension_language_profile(
    scaffold_defaults: &kernel::PluginRuntimeScaffoldDefaults,
) -> CliResult<Option<ProcessStdioNativeExtensionLanguageProfile>> {
    if scaffold_defaults.bridge_kind != kernel::PluginBridgeKind::ProcessStdio {
        return Ok(None);
    }

    match scaffold_defaults.source_language.as_deref() {
        Some("python") => Ok(Some(ProcessStdioNativeExtensionLanguageProfile {
            command: "python3",
            args: PYTHON_EXTENSION_ARGS,
            process_timeout_ms: 5_000,
            smoke_allow_command: "python3",
            example_package_root: "examples/plugins-process/native-extension-python",
            scaffold_files: PYTHON_EXTENSION_SCAFFOLD_FILES,
        })),
        Some("javascript") => Ok(Some(ProcessStdioNativeExtensionLanguageProfile {
            command: "node",
            args: JAVASCRIPT_EXTENSION_ARGS,
            process_timeout_ms: 15_000,
            smoke_allow_command: "node",
            example_package_root: "examples/plugins-process/native-extension-javascript",
            scaffold_files: JAVASCRIPT_EXTENSION_SCAFFOLD_FILES,
        })),
        Some("go") => Ok(Some(ProcessStdioNativeExtensionLanguageProfile {
            command: "go",
            args: GO_EXTENSION_ARGS,
            process_timeout_ms: 15_000,
            smoke_allow_command: "go",
            example_package_root: "examples/plugins-process/native-extension-go",
            scaffold_files: GO_EXTENSION_SCAFFOLD_FILES,
        })),
        Some("rust") => Ok(Some(ProcessStdioNativeExtensionLanguageProfile {
            command: "cargo",
            args: RUST_EXTENSION_ARGS,
            process_timeout_ms: 60_000,
            smoke_allow_command: "cargo",
            example_package_root: "examples/plugins-process/native-extension-rust",
            scaffold_files: RUST_EXTENSION_SCAFFOLD_FILES,
        })),
        Some(source_language) => Err(format!(
            "plugins init only scaffolds runnable process_stdio extension entrypoints for source_language `python`, `javascript`, `go`, or `rust`; got `{source_language}`"
        )),
        None => Ok(None),
    }
}

pub(crate) fn process_stdio_scaffold_args(
    profile: ProcessStdioNativeExtensionLanguageProfile,
) -> Vec<String> {
    profile
        .args
        .iter()
        .map(|value| (*value).to_owned())
        .collect()
}

pub(crate) fn render_authoring_doctor_command(package_root: &str) -> String {
    format!("loong plugins doctor --root \"{package_root}\" --profile sdk-release")
}

pub(crate) fn render_authoring_inventory_command(package_root: &str) -> String {
    format!("loong plugins inventory --root \"{package_root}\"")
}

pub(crate) fn render_authoring_actions_command(package_root: &str) -> String {
    format!("loong plugins actions --root \"{package_root}\" --profile sdk-release")
}

pub(crate) fn render_authoring_smoke_test_command(
    package_root: &str,
    plugin_id: &str,
    allow_command: &str,
) -> String {
    format!(
        "loong plugins invoke-extension --root \"{package_root}\" --plugin-id \"{plugin_id}\" --method extension/event --payload '{{\"event\":\"session_start\"}}' --allow-command {allow_command}"
    )
}

pub(crate) fn build_native_extension_authoring_guidance(
    plugin: &PluginInventoryResult,
) -> Option<NativeExtensionAuthoringGuidanceView> {
    let bridge_kind = kernel::PluginBridgeKind::parse_label(&plugin.bridge_kind)?;
    let scaffold_defaults =
        kernel::plugin_runtime_scaffold_defaults(bridge_kind, plugin.source_language.as_deref())
            .ok()?;
    let profile = process_stdio_native_extension_language_profile(&scaffold_defaults).ok()??;
    let source_language = scaffold_defaults.source_language?;

    Some(NativeExtensionAuthoringGuidanceView {
        plugin_id: plugin.plugin_id.clone(),
        package_root: plugin.package_root.clone(),
        source_language,
        bridge_kind: plugin.bridge_kind.clone(),
        reference_example_path: profile.example_package_root.to_owned(),
        inventory_command: render_authoring_inventory_command(plugin.package_root.as_str()),
        smoke_allow_command: profile.smoke_allow_command.to_owned(),
        smoke_test_command: render_authoring_smoke_test_command(
            plugin.package_root.as_str(),
            plugin.plugin_id.as_str(),
            profile.smoke_allow_command,
        ),
        extension_contract: plugin.extension_contract.clone(),
        extension_methods: plugin.extension_methods.clone(),
        extension_events: plugin.extension_events.clone(),
        extension_host_actions: plugin.extension_host_actions.clone(),
        extension_metadata_issues: plugin.extension_metadata_issues.clone(),
    })
}

pub(crate) fn render_rust_extension_cargo_toml(plugin_id: &str) -> String {
    format!(
        "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nserde_json = \"1\"\n\n[workspace]\n",
        rust_package_name_for_plugin(plugin_id)
    )
}

fn rust_package_name_for_plugin(plugin_id: &str) -> String {
    let normalized = plugin_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    let trimmed = normalized.trim_matches(|ch| ch == '-' || ch == '_');
    if trimmed.is_empty() {
        return "native-extension-rust".to_owned();
    }
    trimmed.to_owned()
}

const PYTHON_EXTENSION_STUB: &str = r#"#!/usr/bin/env python3
import json
import sys


def build_extension_payload(operation, payload):
    if operation == "extension/event":
        return {
            "ok": True,
            "handled_event": payload.get("event", "unknown"),
        }
    if operation == "extension/command":
        command_name = payload.get("command_name", "extension")
        return {
            "text": f"{command_name} command stub"
        }
    if operation == "extension/resource":
        return {
            "commands": [],
            "tools": []
        }
    return {
        "error": f"unsupported method: {operation}"
    }


for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    request = json.loads(line)
    method = request.get("method", "")
    payload = request.get("payload") or {}
    if method == "tools/call":
        operation = payload.get("operation", "")
        extension_payload = payload.get("payload") or {}
        response_payload = build_extension_payload(operation, extension_payload)
    else:
        response_payload = {"error": f"unsupported transport method: {method}"}
    response = {"method": method, "id": request.get("id"), "payload": response_payload}
    print(json.dumps(response), flush=True)
"#;

const JAVASCRIPT_EXTENSION_STUB: &str = r#"#!/usr/bin/env node
function buildExtensionPayload(operation, payload) {
  if (operation === 'extension/event') {
    return {
      ok: true,
      handled_event: payload.event ?? 'unknown',
    };
  }
  if (operation === 'extension/command') {
    const commandName = payload.command_name ?? 'extension';
    return {
      text: `${commandName} command stub`,
    };
  }
  if (operation === 'extension/resource') {
    return {
      commands: [],
      tools: [],
    };
  }
  return {
    error: `unsupported method: ${operation}`,
  };
}

function emitResponse(line) {
  const trimmed = line.trim();
  if (!trimmed) {
    return;
  }
  const request = JSON.parse(trimmed);
  const method = request.method ?? '';
  const payload = request.payload ?? {};
  const responsePayload = method === 'tools/call'
    ? buildExtensionPayload(payload.operation ?? '', payload.payload ?? {})
    : { error: `unsupported transport method: ${method}` };
  const response = {
    method,
    id: request.id ?? null,
    payload: responsePayload,
  };
  process.stdout.write(`${JSON.stringify(response)}\n`);
}

process.stdin.setEncoding('utf8');
let buffered = '';

process.stdin.on('data', (chunk) => {
  buffered += chunk;
  let newlineIndex = buffered.indexOf('\n');
  while (newlineIndex !== -1) {
    const line = buffered.slice(0, newlineIndex);
    buffered = buffered.slice(newlineIndex + 1);
    emitResponse(line);
    newlineIndex = buffered.indexOf('\n');
  }
});

process.stdin.on('end', () => {
  if (buffered.trim()) {
    emitResponse(buffered);
  }
});

process.stdin.resume();
"#;

const GO_EXTENSION_STUB: &str = r#"package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
)

type requestFrame struct {
	Method  string         `json:"method"`
	ID      any            `json:"id"`
	Payload map[string]any `json:"payload"`
}

type responseFrame struct {
	Method  string `json:"method"`
	ID      any    `json:"id"`
	Payload any    `json:"payload"`
}

func buildExtensionPayload(operation string, payload map[string]any) any {
	switch operation {
	case "extension/event":
		event, _ := payload["event"].(string)
		if event == "" {
			event = "unknown"
		}
		return map[string]any{
			"ok":            true,
			"handled_event": event,
		}
	case "extension/command":
		commandName, _ := payload["command_name"].(string)
		if commandName == "" {
			commandName = "extension"
		}
		return map[string]any{
			"text": fmt.Sprintf("%s command stub", commandName),
		}
	case "extension/resource":
		return map[string]any{
			"commands": []any{},
			"tools":    []any{},
		}
	default:
		return map[string]any{
			"error": fmt.Sprintf("unsupported method: %s", operation),
		}
	}
}

func main() {
	scanner := bufio.NewScanner(os.Stdin)
	for scanner.Scan() {
		line := scanner.Text()
		if line == "" {
			continue
		}

		var request requestFrame
		if err := json.Unmarshal([]byte(line), &request); err != nil {
			continue
		}

		payload := request.Payload
		if payload == nil {
			payload = map[string]any{}
		}

		var responsePayload any
		if request.Method == "tools/call" {
			operation, _ := payload["operation"].(string)
			extensionPayload, _ := payload["payload"].(map[string]any)
			if extensionPayload == nil {
				extensionPayload = map[string]any{}
			}
			responsePayload = buildExtensionPayload(operation, extensionPayload)
		} else {
			responsePayload = map[string]any{
				"error": fmt.Sprintf("unsupported transport method: %s", request.Method),
			}
		}

		response := responseFrame{
			Method:  request.Method,
			ID:      request.ID,
			Payload: responsePayload,
		}
		encoded, err := json.Marshal(response)
		if err != nil {
			continue
		}
		fmt.Println(string(encoded))
	}
}
"#;

const RUST_EXTENSION_MAIN_RS: &str = r#"use serde_json::{Map, Value, json};
use std::io::{self, BufRead};

fn build_extension_payload(operation: &str, payload: &Map<String, Value>) -> Value {
    match operation {
        "extension/event" => {
            let handled_event = payload
                .get("event")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            json!({
                "ok": true,
                "handled_event": handled_event,
            })
        }
        "extension/command" => {
            let command_name = payload
                .get("command_name")
                .and_then(Value::as_str)
                .unwrap_or("extension");
            json!({
                "text": format!("{command_name} command stub"),
            })
        }
        "extension/resource" => json!({
            "commands": [],
            "tools": [],
        }),
        other => json!({
            "error": format!("unsupported method: {other}"),
        }),
    }
}

fn main() {
    let stdin = io::stdin();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request = match serde_json::from_str::<Value>(trimmed) {
            Ok(request) => request,
            Err(_) => continue,
        };
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let payload = request
            .get("payload")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();

        let response_payload = if method == "tools/call" {
            let operation = payload
                .get("operation")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let extension_payload = payload
                .get("payload")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            build_extension_payload(operation, &extension_payload)
        } else {
            json!({
                "error": format!("unsupported transport method: {method}"),
            })
        };

        println!(
            "{}",
            json!({
                "method": method,
                "id": id,
                "payload": response_payload,
            })
        );
    }
}
"#;
