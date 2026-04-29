use std::collections::BTreeMap;

use crate::PluginPreflightResult;
use crate::{CliResult, PluginInventoryResult, kernel};
use serde::{Deserialize, Serialize};

pub(crate) const PROCESS_STDIO_NATIVE_EXTENSION_CONTRACT: &str = "process_stdio_json_line_v1";
pub(crate) const PROCESS_STDIO_NATIVE_EXTENSION_FAMILY: &str =
    kernel::GOVERNED_NATIVE_RUNTIME_EXTENSION_FAMILY;
pub(crate) const PROCESS_STDIO_NATIVE_EXTENSION_TRUST_LANE: &str =
    kernel::GOVERNED_SIDECAR_EXTENSION_TRUST_LANE;
pub(crate) const PROCESS_STDIO_NATIVE_EXTENSION_FACETS: &[&str] =
    &["events", "commands", "resources"];
pub(crate) const PROCESS_STDIO_NATIVE_EXTENSION_METHODS: &[&str] =
    &["extension/event", "extension/command", "extension/resource"];
pub(crate) const PROCESS_STDIO_NATIVE_EXTENSION_EVENTS: &[&str] = &["session_start"];
pub(crate) const PROCESS_STDIO_NATIVE_EXTENSION_HOST_HOOKS: &[&str] = &[];
pub(crate) const PROCESS_STDIO_NATIVE_EXTENSION_HOST_ACTIONS: &[&str] = &[];

#[derive(Debug, Clone, Copy)]
pub(crate) struct RuntimeScaffoldTemplateFile {
    pub relative_path: &'static str,
    pub contents: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProcessStdioNativeExtensionLanguageProfile {
    pub source_language_arg: &'static str,
    pub source_language: &'static str,
    pub command: &'static str,
    pub args: &'static [&'static str],
    pub process_timeout_ms: u64,
    pub smoke_allow_command: &'static str,
    pub example_package_root: &'static str,
    pub scaffold_files: &'static [RuntimeScaffoldTemplateFile],
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NativeExtensionAuthoringGuidanceView {
    pub plugin_id: String,
    pub package_root: String,
    pub source_language_arg: String,
    pub source_language: String,
    pub bridge_kind: String,
    pub reference_example_path: String,
    pub doctor_command: String,
    pub inventory_command: String,
    pub actions_command: String,
    pub smoke_allow_command: String,
    pub smoke_test_command: String,
    pub extension_contract: Option<String>,
    pub extension_family: Option<String>,
    pub extension_trust_lane: Option<String>,
    pub extension_methods: Vec<String>,
    pub extension_events: Vec<String>,
    pub extension_host_hooks: Vec<String>,
    pub extension_host_actions: Vec<String>,
    pub extension_metadata_issues: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub author_remediation_actions: Vec<NativeExtensionAuthoringActionView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_ready: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remediation_classes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recommended_action_summaries: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub author_remediation_hints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NativeExtensionAuthoringActionView {
    pub kind: String,
    pub role: String,
    pub execution_kind: String,
    pub agent_runnable: bool,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_command: Option<String>,
    pub requires_allow_command: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_path: Option<String>,
    pub blocking: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeExtensionAuthoringSummaryView {
    pub guided_plugins: usize,
    pub plugins_with_metadata_issues: usize,
    pub total_remediation_actions: usize,
    pub action_roles: BTreeMap<String, usize>,
    pub action_execution_kinds: BTreeMap<String, usize>,
    pub runnable_action_count: usize,
    pub allow_command_gated_action_count: usize,
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
const TYPESCRIPT_EXTENSION_SCAFFOLD_FILES: &[RuntimeScaffoldTemplateFile] =
    &[RuntimeScaffoldTemplateFile {
        relative_path: "index.ts",
        contents: TYPESCRIPT_EXTENSION_STUB,
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
const TYPESCRIPT_EXTENSION_ARGS: &[&str] = &["--experimental-strip-types", "index.ts"];
const GO_EXTENSION_ARGS: &[&str] = &["run", "main.go"];
const RUST_EXTENSION_ARGS: &[&str] = &["run", "--quiet", "--manifest-path", "Cargo.toml"];

const SUPPORTED_PROCESS_STDIO_AUTHORING_PROFILES: &[ProcessStdioNativeExtensionLanguageProfile] = &[
    ProcessStdioNativeExtensionLanguageProfile {
        source_language_arg: "py",
        source_language: "python",
        command: "python3",
        args: PYTHON_EXTENSION_ARGS,
        process_timeout_ms: 5_000,
        smoke_allow_command: "python3",
        example_package_root: "examples/plugins-process/native-extension-python",
        scaffold_files: PYTHON_EXTENSION_SCAFFOLD_FILES,
    },
    ProcessStdioNativeExtensionLanguageProfile {
        source_language_arg: "js",
        source_language: "javascript",
        command: "node",
        args: JAVASCRIPT_EXTENSION_ARGS,
        process_timeout_ms: 15_000,
        smoke_allow_command: "node",
        example_package_root: "examples/plugins-process/native-extension-javascript",
        scaffold_files: JAVASCRIPT_EXTENSION_SCAFFOLD_FILES,
    },
    ProcessStdioNativeExtensionLanguageProfile {
        source_language_arg: "ts",
        source_language: "typescript",
        command: "node",
        args: TYPESCRIPT_EXTENSION_ARGS,
        process_timeout_ms: 15_000,
        smoke_allow_command: "node",
        example_package_root: "examples/plugins-process/native-extension-typescript",
        scaffold_files: TYPESCRIPT_EXTENSION_SCAFFOLD_FILES,
    },
    ProcessStdioNativeExtensionLanguageProfile {
        source_language_arg: "go",
        source_language: "go",
        command: "go",
        args: GO_EXTENSION_ARGS,
        process_timeout_ms: 15_000,
        smoke_allow_command: "go",
        example_package_root: "examples/plugins-process/native-extension-go",
        scaffold_files: GO_EXTENSION_SCAFFOLD_FILES,
    },
    ProcessStdioNativeExtensionLanguageProfile {
        source_language_arg: "rs",
        source_language: "rust",
        command: "cargo",
        args: RUST_EXTENSION_ARGS,
        process_timeout_ms: 60_000,
        smoke_allow_command: "cargo",
        example_package_root: "examples/plugins-process/native-extension-rust",
        scaffold_files: RUST_EXTENSION_SCAFFOLD_FILES,
    },
];

#[cfg(test)]
pub(crate) fn supported_process_stdio_authoring_profiles()
-> &'static [ProcessStdioNativeExtensionLanguageProfile] {
    SUPPORTED_PROCESS_STDIO_AUTHORING_PROFILES
}

pub(crate) fn process_stdio_native_extension_language_profile(
    scaffold_defaults: &kernel::PluginRuntimeScaffoldDefaults,
) -> CliResult<Option<ProcessStdioNativeExtensionLanguageProfile>> {
    if scaffold_defaults.bridge_kind != kernel::PluginBridgeKind::ProcessStdio {
        return Ok(None);
    }

    let Some(source_language) = scaffold_defaults.source_language.as_deref() else {
        return Ok(None);
    };
    if let Some(profile) = SUPPORTED_PROCESS_STDIO_AUTHORING_PROFILES
        .iter()
        .find(|profile| profile.source_language == source_language)
        .copied()
    {
        return Ok(Some(profile));
    }
    Err(format!(
        "plugins init only scaffolds runnable process_stdio extension entrypoints for source_language `python`, `javascript`, `go`, or `rust`; got `{source_language}`"
    ))
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
    build_native_extension_authoring_guidance_from_runtime_profile(
        plugin.package_root.as_str(),
        plugin.plugin_id.as_str(),
        plugin.bridge_kind.as_str(),
        plugin.source_language.as_deref()?,
        plugin.extension_contract.clone(),
        plugin.extension_family.clone(),
        plugin.extension_trust_lane.clone(),
        plugin.extension_methods.clone(),
        plugin.extension_events.clone(),
        plugin.extension_host_hooks.clone(),
        plugin.extension_host_actions.clone(),
        plugin.extension_metadata_issues.clone(),
    )
}

pub(crate) fn build_native_extension_authoring_guidance_from_runtime_profile(
    package_root: &str,
    plugin_id: &str,
    bridge_kind: &str,
    source_language: &str,
    extension_contract: Option<String>,
    extension_family: Option<String>,
    extension_trust_lane: Option<String>,
    extension_methods: Vec<String>,
    extension_events: Vec<String>,
    extension_host_hooks: Vec<String>,
    extension_host_actions: Vec<String>,
    extension_metadata_issues: Vec<String>,
) -> Option<NativeExtensionAuthoringGuidanceView> {
    let bridge_kind = kernel::PluginBridgeKind::parse_label(bridge_kind)?;
    let scaffold_defaults =
        kernel::plugin_runtime_scaffold_defaults(bridge_kind, Some(source_language)).ok()?;
    let profile = process_stdio_native_extension_language_profile(&scaffold_defaults).ok()??;
    Some(build_native_extension_authoring_view(
        package_root,
        plugin_id,
        profile,
        source_language,
        bridge_kind.as_str(),
        extension_contract,
        extension_family,
        extension_trust_lane,
        extension_methods,
        extension_events,
        extension_host_hooks,
        extension_host_actions,
        extension_metadata_issues,
    ))
}

pub(crate) fn build_native_extension_authoring_doctor_guidance(
    result: &PluginPreflightResult,
) -> Option<NativeExtensionAuthoringGuidanceView> {
    let plugin = &result.plugin;
    let bridge_kind = kernel::PluginBridgeKind::parse_label(&plugin.bridge_kind)?;
    let scaffold_defaults =
        kernel::plugin_runtime_scaffold_defaults(bridge_kind, plugin.source_language.as_deref())
            .ok()?;
    let profile = process_stdio_native_extension_language_profile(&scaffold_defaults).ok()??;
    let mut guidance = build_native_extension_authoring_guidance(plugin)?;
    guidance.verdict = Some(result.verdict.clone());
    guidance.activation_ready = Some(result.activation_ready);
    guidance.policy_summary = Some(result.policy_summary.clone());
    guidance.remediation_classes = result
        .remediation_classes
        .iter()
        .map(|value| value.as_str().to_owned())
        .collect();
    guidance.recommended_action_summaries = result
        .recommended_actions
        .iter()
        .map(|action| action.summary.clone())
        .collect();
    guidance.author_remediation_actions = native_extension_author_remediation_actions(
        guidance.package_root.as_str(),
        guidance.plugin_id.as_str(),
        profile,
        &guidance.extension_metadata_issues,
        Some(&result.recommended_actions),
    );
    guidance.author_remediation_hints = native_extension_author_remediation_hints(
        &guidance.extension_metadata_issues,
        &guidance.recommended_action_summaries,
    );
    Some(guidance)
}

pub(crate) fn build_native_extension_authoring_view_from_profile(
    package_root: &str,
    plugin_id: &str,
    bridge_kind: &str,
    source_language: &str,
    profile: ProcessStdioNativeExtensionLanguageProfile,
) -> NativeExtensionAuthoringGuidanceView {
    build_native_extension_authoring_view(
        package_root,
        plugin_id,
        profile,
        source_language,
        bridge_kind,
        Some(PROCESS_STDIO_NATIVE_EXTENSION_CONTRACT.to_owned()),
        Some(PROCESS_STDIO_NATIVE_EXTENSION_FAMILY.to_owned()),
        Some(PROCESS_STDIO_NATIVE_EXTENSION_TRUST_LANE.to_owned()),
        PROCESS_STDIO_NATIVE_EXTENSION_METHODS
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>(),
        PROCESS_STDIO_NATIVE_EXTENSION_EVENTS
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>(),
        PROCESS_STDIO_NATIVE_EXTENSION_HOST_HOOKS
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>(),
        PROCESS_STDIO_NATIVE_EXTENSION_HOST_ACTIONS
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>(),
        Vec::new(),
    )
}

fn build_native_extension_authoring_view(
    package_root: &str,
    plugin_id: &str,
    profile: ProcessStdioNativeExtensionLanguageProfile,
    source_language: &str,
    bridge_kind: &str,
    extension_contract: Option<String>,
    extension_family: Option<String>,
    extension_trust_lane: Option<String>,
    extension_methods: Vec<String>,
    extension_events: Vec<String>,
    extension_host_hooks: Vec<String>,
    extension_host_actions: Vec<String>,
    extension_metadata_issues: Vec<String>,
) -> NativeExtensionAuthoringGuidanceView {
    let author_remediation_actions = native_extension_author_remediation_actions(
        package_root,
        plugin_id,
        profile,
        &extension_metadata_issues,
        None,
    );
    let author_remediation_hints =
        native_extension_author_remediation_hints(&extension_metadata_issues, &[]);
    NativeExtensionAuthoringGuidanceView {
        plugin_id: plugin_id.to_owned(),
        package_root: package_root.to_owned(),
        source_language_arg: profile.source_language_arg.to_owned(),
        source_language: source_language.to_owned(),
        bridge_kind: bridge_kind.to_owned(),
        reference_example_path: profile.example_package_root.to_owned(),
        doctor_command: render_authoring_doctor_command(package_root),
        inventory_command: render_authoring_inventory_command(package_root),
        actions_command: render_authoring_actions_command(package_root),
        smoke_allow_command: profile.smoke_allow_command.to_owned(),
        smoke_test_command: render_authoring_smoke_test_command(
            package_root,
            plugin_id,
            profile.smoke_allow_command,
        ),
        extension_contract,
        extension_family,
        extension_trust_lane,
        extension_methods,
        extension_events,
        extension_host_hooks,
        extension_host_actions,
        extension_metadata_issues,
        author_remediation_actions,
        verdict: None,
        activation_ready: None,
        policy_summary: None,
        remediation_classes: Vec::new(),
        recommended_action_summaries: Vec::new(),
        author_remediation_hints,
    }
}

fn native_extension_author_remediation_hints(
    extension_metadata_issues: &[String],
    recommended_action_summaries: &[String],
) -> Vec<String> {
    let mut hints = extension_metadata_issues
        .iter()
        .map(|issue| format!("Repair native extension declaration metadata: {issue}"))
        .collect::<Vec<_>>();
    if !extension_metadata_issues.is_empty() {
        hints.push(
            "After fixing native extension declaration metadata, rerun `loong plugins doctor` and `loong plugins inventory`."
                .to_owned(),
        );
    }
    hints.extend(recommended_action_summaries.iter().cloned());
    hints.sort();
    hints.dedup();
    hints
}

pub(crate) fn summarize_native_extension_authoring_guidance(
    guidance: &[NativeExtensionAuthoringGuidanceView],
) -> Option<NativeExtensionAuthoringSummaryView> {
    if guidance.is_empty() {
        return None;
    }

    let mut plugins_with_metadata_issues = 0_usize;
    let mut total_remediation_actions = 0_usize;
    let mut action_roles = BTreeMap::new();
    let mut action_execution_kinds = BTreeMap::new();
    let mut runnable_action_count = 0_usize;
    let mut allow_command_gated_action_count = 0_usize;

    for plugin in guidance {
        if !plugin.extension_metadata_issues.is_empty() {
            plugins_with_metadata_issues = plugins_with_metadata_issues.saturating_add(1);
        }

        total_remediation_actions =
            total_remediation_actions.saturating_add(plugin.author_remediation_actions.len());

        for action in &plugin.author_remediation_actions {
            *action_roles.entry(action.role.clone()).or_insert(0) += 1;
            *action_execution_kinds
                .entry(action.execution_kind.clone())
                .or_insert(0) += 1;
            if action.agent_runnable {
                runnable_action_count = runnable_action_count.saturating_add(1);
            }
            if action.requires_allow_command {
                allow_command_gated_action_count =
                    allow_command_gated_action_count.saturating_add(1);
            }
        }
    }

    Some(NativeExtensionAuthoringSummaryView {
        guided_plugins: guidance.len(),
        plugins_with_metadata_issues,
        total_remediation_actions,
        action_roles,
        action_execution_kinds,
        runnable_action_count,
        allow_command_gated_action_count,
    })
}

fn native_extension_author_remediation_actions(
    package_root: &str,
    plugin_id: &str,
    profile: ProcessStdioNativeExtensionLanguageProfile,
    extension_metadata_issues: &[String],
    recommended_actions: Option<&[crate::PluginPreflightRecommendedAction]>,
) -> Vec<NativeExtensionAuthoringActionView> {
    let mut actions = extension_metadata_issues
        .iter()
        .map(|issue| NativeExtensionAuthoringActionView {
            kind: "repair_extension_metadata".to_owned(),
            role: "author".to_owned(),
            execution_kind: "manual_edit".to_owned(),
            agent_runnable: false,
            summary: format!("Repair native extension declaration metadata: {issue}"),
            command: None,
            allow_command: None,
            requires_allow_command: false,
            field_path: parse_metadata_field_path_from_issue(issue),
            blocking: true,
        })
        .collect::<Vec<_>>();

    if !extension_metadata_issues.is_empty() {
        actions.push(NativeExtensionAuthoringActionView {
            kind: "rerun_doctor".to_owned(),
            role: "verification".to_owned(),
            execution_kind: "read_only_cli".to_owned(),
            agent_runnable: true,
            summary: "Rerun doctor after repairing native extension declaration metadata."
                .to_owned(),
            command: Some(render_authoring_doctor_command(package_root)),
            allow_command: None,
            requires_allow_command: false,
            field_path: None,
            blocking: false,
        });
        actions.push(NativeExtensionAuthoringActionView {
            kind: "rerun_inventory".to_owned(),
            role: "verification".to_owned(),
            execution_kind: "read_only_cli".to_owned(),
            agent_runnable: true,
            summary: "Rerun inventory to confirm the repaired native extension declaration truth."
                .to_owned(),
            command: Some(render_authoring_inventory_command(package_root)),
            allow_command: None,
            requires_allow_command: false,
            field_path: None,
            blocking: false,
        });
        actions.push(NativeExtensionAuthoringActionView {
            kind: "rerun_smoke_test".to_owned(),
            role: "verification".to_owned(),
            execution_kind: "governed_smoke_probe".to_owned(),
            agent_runnable: true,
            summary:
                "Rerun the governed smoke probe after repairing native extension declaration metadata."
                    .to_owned(),
            command: Some(render_authoring_smoke_test_command(
                package_root,
                plugin_id,
                profile.smoke_allow_command,
            )),
            allow_command: Some(profile.smoke_allow_command.to_owned()),
            requires_allow_command: true,
            field_path: None,
            blocking: false,
        });
    }

    if let Some(recommended_actions) = recommended_actions {
        actions.extend(recommended_actions.iter().map(|action| {
            NativeExtensionAuthoringActionView {
                kind: format!("preflight_{}", action.remediation_class.as_str()),
                role: if action.operator_action.is_some() {
                    "operator".to_owned()
                } else {
                    "author".to_owned()
                },
                execution_kind: if action.operator_action.is_some() {
                    "operator_follow_up".to_owned()
                } else {
                    "author_follow_up".to_owned()
                },
                agent_runnable: false,
                summary: action.summary.clone(),
                command: action
                    .operator_action
                    .as_ref()
                    .map(|_| render_authoring_actions_command(package_root)),
                allow_command: None,
                requires_allow_command: false,
                field_path: action.field_path.clone(),
                blocking: action.blocking,
            }
        }));
    }

    actions.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.summary.cmp(&right.summary))
            .then_with(|| left.field_path.cmp(&right.field_path))
    });
    actions.dedup_by(|left, right| {
        left.kind == right.kind
            && left.role == right.role
            && left.execution_kind == right.execution_kind
            && left.agent_runnable == right.agent_runnable
            && left.summary == right.summary
            && left.command == right.command
            && left.allow_command == right.allow_command
            && left.requires_allow_command == right.requires_allow_command
            && left.field_path == right.field_path
            && left.blocking == right.blocking
    });
    actions
}

fn parse_metadata_field_path_from_issue(issue: &str) -> Option<String> {
    let key = issue
        .split('`')
        .nth(1)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(format!("metadata.{key}"))
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

const TYPESCRIPT_EXTENSION_STUB: &str = r#"#!/usr/bin/env node
type PayloadMap = Record<string, unknown>;

function buildExtensionPayload(operation: string, payload: PayloadMap): unknown {
  if (operation === 'extension/event') {
    const handledEvent = typeof payload.event === 'string' ? payload.event : 'unknown';
    return {
      ok: true,
      handled_event: handledEvent,
    };
  }
  if (operation === 'extension/command') {
    const commandName =
      typeof payload.command_name === 'string' ? payload.command_name : 'extension';
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

function emitResponse(line: string): void {
  const trimmed = line.trim();
  if (!trimmed) {
    return;
  }
  const request = JSON.parse(trimmed) as {
    method?: string;
    id?: unknown;
    payload?: PayloadMap;
  };
  const method = typeof request.method === 'string' ? request.method : '';
  const payload = request.payload ?? {};
  const nestedPayload =
    payload.payload && typeof payload.payload === 'object'
      ? (payload.payload as PayloadMap)
      : {};
  const operation = typeof payload.operation === 'string' ? payload.operation : '';
  const responsePayload =
    method === 'tools/call'
      ? buildExtensionPayload(operation, nestedPayload)
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

process.stdin.on('data', (chunk: string) => {
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
