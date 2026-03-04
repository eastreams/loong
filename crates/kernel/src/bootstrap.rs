use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::plugin_ir::{PluginActivationPlan, PluginActivationStatus, PluginBridgeKind};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapPolicy {
    pub allow_http_json_auto_apply: bool,
    pub allow_process_stdio_auto_apply: bool,
    pub allow_native_ffi_auto_apply: bool,
    pub allow_wasm_component_auto_apply: bool,
    pub allow_mcp_server_auto_apply: bool,
    pub enforce_ready_execution: bool,
    pub max_tasks: usize,
}

impl Default for BootstrapPolicy {
    fn default() -> Self {
        Self {
            allow_http_json_auto_apply: true,
            allow_process_stdio_auto_apply: false,
            allow_native_ffi_auto_apply: false,
            allow_wasm_component_auto_apply: false,
            allow_mcp_server_auto_apply: false,
            enforce_ready_execution: false,
            max_tasks: 256,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapTaskStatus {
    Applied,
    DeferredUnsupportedAutoApply,
    SkippedNotReady,
    SkippedByPolicyLimit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapTask {
    pub plugin_id: String,
    pub source_path: String,
    pub bridge_kind: PluginBridgeKind,
    pub adapter_family: String,
    pub bootstrap_hint: String,
    pub status: BootstrapTaskStatus,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BootstrapReport {
    pub total_tasks: usize,
    pub applied_tasks: usize,
    pub deferred_tasks: usize,
    pub skipped_tasks: usize,
    pub blocked: bool,
    pub block_reason: Option<String>,
    pub applied_plugin_keys: BTreeSet<(String, String)>,
    pub tasks: Vec<BootstrapTask>,
}

#[derive(Debug, Default)]
pub struct PluginBootstrapExecutor;

impl PluginBootstrapExecutor {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    #[must_use]
    pub fn execute(
        &self,
        plan: &PluginActivationPlan,
        policy: &BootstrapPolicy,
    ) -> BootstrapReport {
        let mut report = BootstrapReport::default();
        let mut ready_handled = 0_usize;

        for candidate in &plan.candidates {
            report.total_tasks = report.total_tasks.saturating_add(1);

            if !matches!(candidate.status, PluginActivationStatus::Ready) {
                report.skipped_tasks = report.skipped_tasks.saturating_add(1);
                report.tasks.push(BootstrapTask {
                    plugin_id: candidate.plugin_id.clone(),
                    source_path: candidate.source_path.clone(),
                    bridge_kind: candidate.bridge_kind,
                    adapter_family: candidate.adapter_family.clone(),
                    bootstrap_hint: candidate.bootstrap_hint.clone(),
                    status: BootstrapTaskStatus::SkippedNotReady,
                    reason: "activation status is not ready".to_owned(),
                });
                continue;
            }

            if ready_handled >= policy.max_tasks {
                report.skipped_tasks = report.skipped_tasks.saturating_add(1);
                report.tasks.push(BootstrapTask {
                    plugin_id: candidate.plugin_id.clone(),
                    source_path: candidate.source_path.clone(),
                    bridge_kind: candidate.bridge_kind,
                    adapter_family: candidate.adapter_family.clone(),
                    bootstrap_hint: candidate.bootstrap_hint.clone(),
                    status: BootstrapTaskStatus::SkippedByPolicyLimit,
                    reason: format!("max bootstrap task limit reached: {}", policy.max_tasks),
                });
                continue;
            }
            ready_handled = ready_handled.saturating_add(1);

            if bridge_auto_apply_allowed(candidate.bridge_kind, policy) {
                report.applied_tasks = report.applied_tasks.saturating_add(1);
                report
                    .applied_plugin_keys
                    .insert((candidate.source_path.clone(), candidate.plugin_id.clone()));
                report.tasks.push(BootstrapTask {
                    plugin_id: candidate.plugin_id.clone(),
                    source_path: candidate.source_path.clone(),
                    bridge_kind: candidate.bridge_kind,
                    adapter_family: candidate.adapter_family.clone(),
                    bootstrap_hint: candidate.bootstrap_hint.clone(),
                    status: BootstrapTaskStatus::Applied,
                    reason: "bridge is allowed for automatic bootstrap apply".to_owned(),
                });
            } else {
                report.deferred_tasks = report.deferred_tasks.saturating_add(1);
                report.tasks.push(BootstrapTask {
                    plugin_id: candidate.plugin_id.clone(),
                    source_path: candidate.source_path.clone(),
                    bridge_kind: candidate.bridge_kind,
                    adapter_family: candidate.adapter_family.clone(),
                    bootstrap_hint: candidate.bootstrap_hint.clone(),
                    status: BootstrapTaskStatus::DeferredUnsupportedAutoApply,
                    reason: "bridge is ready but auto-apply is disabled by bootstrap policy"
                        .to_owned(),
                });
            }
        }

        if policy.enforce_ready_execution && report.deferred_tasks > 0 {
            report.blocked = true;
            report.block_reason = Some(format!(
                "bootstrap policy blocked {} ready plugin(s) that were not auto-applied",
                report.deferred_tasks
            ));
        }

        report
    }
}

fn bridge_auto_apply_allowed(bridge: PluginBridgeKind, policy: &BootstrapPolicy) -> bool {
    match bridge {
        PluginBridgeKind::HttpJson => policy.allow_http_json_auto_apply,
        PluginBridgeKind::ProcessStdio => policy.allow_process_stdio_auto_apply,
        PluginBridgeKind::NativeFfi => policy.allow_native_ffi_auto_apply,
        PluginBridgeKind::WasmComponent => policy.allow_wasm_component_auto_apply,
        PluginBridgeKind::McpServer => policy.allow_mcp_server_auto_apply,
        PluginBridgeKind::Unknown => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_ir::{
        PluginActivationCandidate, PluginActivationPlan, PluginActivationStatus, PluginBridgeKind,
    };

    fn sample_plan() -> PluginActivationPlan {
        PluginActivationPlan {
            total_plugins: 2,
            ready_plugins: 2,
            blocked_plugins: 0,
            candidates: vec![
                PluginActivationCandidate {
                    plugin_id: "http-plugin".to_owned(),
                    source_path: "/tmp/http.rs".to_owned(),
                    bridge_kind: PluginBridgeKind::HttpJson,
                    adapter_family: "http-adapter".to_owned(),
                    status: PluginActivationStatus::Ready,
                    reason: "ready".to_owned(),
                    bootstrap_hint: "register http".to_owned(),
                },
                PluginActivationCandidate {
                    plugin_id: "ffi-plugin".to_owned(),
                    source_path: "/tmp/ffi.rs".to_owned(),
                    bridge_kind: PluginBridgeKind::NativeFfi,
                    adapter_family: "rust-ffi-adapter".to_owned(),
                    status: PluginActivationStatus::Ready,
                    reason: "ready".to_owned(),
                    bootstrap_hint: "load ffi".to_owned(),
                },
            ],
        }
    }

    #[test]
    fn default_policy_applies_http_and_defers_ffi() {
        let executor = PluginBootstrapExecutor::new();
        let report = executor.execute(&sample_plan(), &BootstrapPolicy::default());

        assert_eq!(report.applied_tasks, 1);
        assert_eq!(report.deferred_tasks, 1);
        assert!(!report.blocked);
        assert!(report
            .applied_plugin_keys
            .contains(&("/tmp/http.rs".to_owned(), "http-plugin".to_owned())));
        assert!(!report
            .applied_plugin_keys
            .contains(&("/tmp/ffi.rs".to_owned(), "ffi-plugin".to_owned())));
    }

    #[test]
    fn enforce_ready_execution_blocks_when_ready_tasks_are_deferred() {
        let executor = PluginBootstrapExecutor::new();
        let mut policy = BootstrapPolicy::default();
        policy.enforce_ready_execution = true;

        let report = executor.execute(&sample_plan(), &policy);
        assert!(report.blocked);
        assert!(report.block_reason.is_some());
    }

    #[test]
    fn allow_all_bridges_applies_all_ready_tasks() {
        let executor = PluginBootstrapExecutor::new();
        let mut policy = BootstrapPolicy::default();
        policy.allow_native_ffi_auto_apply = true;

        let report = executor.execute(&sample_plan(), &policy);
        assert_eq!(report.applied_tasks, 2);
        assert_eq!(report.deferred_tasks, 0);
        assert!(!report.blocked);
    }
}
