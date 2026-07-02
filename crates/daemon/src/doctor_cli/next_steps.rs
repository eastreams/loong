use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::env;
use std::ffi::OsStr;
use std::path::Path;

use loong_app as mvp;
use serde_json::Value;

use crate::plugin_bridge_account_summary::plugin_bridge_account_summary;
use crate::provider::credential_policy as provider_credential_policy;
use crate::provider::model_probe_policy as provider_model_probe_policy;

use super::{
    DoctorCheck, DoctorCheckLevel, check_level_json, doctor_ready_for_first_turn,
    managed_bridge_duplicate_plugin_id_counts, managed_bridge_plugin_label,
    render_managed_bridge_compatible_plugin_labels, render_managed_bridge_configured_plugin_labels,
    render_u32_list,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DoctorRuntimeAttentionReason {
    Retrying,
    Stale,
    DuplicateRuntimeInstances,
}

impl DoctorRuntimeAttentionReason {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Retrying => "retrying",
            Self::Stale => "stale",
            Self::DuplicateRuntimeInstances => "duplicate_runtime_instances",
        }
    }

    pub(super) fn remediation(self) -> &'static str {
        match self {
            Self::Retrying => "inspect_bridge_connectivity",
            Self::Stale => "restart_stale_runtime",
            Self::DuplicateRuntimeInstances => "stop_duplicate_runtimes",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ManagedBridgeRuntimeAttention<'a> {
    pub(super) channel_id: &'static str,
    pub(super) channel_label: &'a str,
    pub(super) account_ids: BTreeSet<String>,
    pub(super) reasons: BTreeSet<&'static str>,
    pub(super) preferred_owner_pids: BTreeSet<u32>,
    pub(super) cleanup_owner_pids: BTreeSet<u32>,
    pub(super) last_duplicate_reclaim_at: Option<u64>,
    pub(super) last_duplicate_reclaim_cleanup_owner_pids: BTreeSet<u32>,
    pub(super) recent_incidents: Vec<DoctorRuntimeIncident>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DoctorRuntimeIncident {
    pub(super) account_id: Option<String>,
    pub(super) account_label: Option<String>,
    pub(super) kind: &'static str,
    pub(super) at_ms: u64,
    pub(super) detail: Option<String>,
    pub(super) owner_pids: Vec<u32>,
}

pub(super) fn doctor_runtime_attention_reason(
    check: &DoctorCheck,
) -> Option<DoctorRuntimeAttentionReason> {
    if check.detail.contains("retrying after transient failures") {
        return Some(DoctorRuntimeAttentionReason::Retrying);
    }
    if check.detail.contains("stale runtime detected") {
        return Some(DoctorRuntimeAttentionReason::Stale);
    }
    if check.detail.contains("multiple runtime instances detected") {
        return Some(DoctorRuntimeAttentionReason::DuplicateRuntimeInstances);
    }
    None
}

fn doctor_runtime_attention_channel_id(check: &DoctorCheck) -> Option<String> {
    for suffix in [
        " bridge serve runtime",
        " serve runtime",
        " channel runtime",
    ] {
        if let Some(channel_id) = check.name.strip_suffix(suffix) {
            let trimmed = channel_id.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
    }

    None
}

pub(super) fn managed_bridge_runtime_attention_surfaces<'a>(
    channel_surfaces: &'a [mvp::channel::ChannelSurface],
) -> Vec<ManagedBridgeRuntimeAttention<'a>> {
    let mut surfaces = Vec::new();

    for surface in channel_surfaces {
        let mut reasons = BTreeSet::new();
        let mut account_ids = BTreeSet::new();
        let mut preferred_owner_pids = BTreeSet::new();
        let mut cleanup_owner_pids = BTreeSet::new();
        let mut last_duplicate_reclaim_at = None;
        let mut last_duplicate_reclaim_cleanup_owner_pids = BTreeSet::new();
        let mut recent_incidents = Vec::new();

        for snapshot in surface
            .configured_accounts
            .iter()
            .filter(|snapshot| snapshot.enabled)
            .filter(|snapshot| snapshot_has_external_plugin_bridge_owner(snapshot))
        {
            let Some(runtime) = snapshot
                .operation(mvp::channel::CHANNEL_OPERATION_SERVE_ID)
                .and_then(|operation| operation.runtime.as_ref())
            else {
                continue;
            };

            if runtime.consecutive_failures > 0 {
                reasons.insert("retrying");
            }
            if runtime.stale {
                reasons.insert("stale");
            }
            if runtime.running_instances > 1 {
                reasons.insert("duplicate_runtime_instances");
                if let Some(pid) = runtime.pid {
                    preferred_owner_pids.insert(pid);
                }
                for owner_pid in &runtime.duplicate_owner_pids {
                    if Some(*owner_pid) == runtime.pid {
                        continue;
                    }
                    cleanup_owner_pids.insert(*owner_pid);
                }
            }
            if runtime.last_duplicate_reclaim_at.is_some_and(|value| {
                last_duplicate_reclaim_at
                    .map(|current| value > current)
                    .unwrap_or(true)
            }) {
                last_duplicate_reclaim_at = runtime.last_duplicate_reclaim_at;
                last_duplicate_reclaim_cleanup_owner_pids.clear();
                for owner_pid in &runtime.last_duplicate_reclaim_cleanup_owner_pids {
                    last_duplicate_reclaim_cleanup_owner_pids.insert(*owner_pid);
                }
            }
            recent_incidents.extend(runtime.recent_incidents.iter().map(|incident| {
                DoctorRuntimeIncident {
                    account_id: runtime.account_id.clone(),
                    account_label: runtime.account_label.clone(),
                    kind: match incident.kind {
                        mvp::channel::ChannelOperationRuntimeIncidentKind::Failure => "failure",
                        mvp::channel::ChannelOperationRuntimeIncidentKind::Recovery => "recovery",
                        mvp::channel::ChannelOperationRuntimeIncidentKind::DuplicateReclaim => {
                            "duplicate_reclaim"
                        }
                    },
                    at_ms: incident.at_ms,
                    detail: incident.detail.clone(),
                    owner_pids: incident.owner_pids.clone(),
                }
            }));
            if runtime.stale || runtime.running_instances > 1 || runtime.consecutive_failures > 0 {
                account_ids.insert(snapshot.configured_account_id.clone());
            }
        }

        if reasons.is_empty() {
            continue;
        }

        recent_incidents.sort_by_key(|incident| std::cmp::Reverse(incident.at_ms));
        recent_incidents.truncate(5);
        surfaces.push(ManagedBridgeRuntimeAttention {
            channel_id: surface.catalog.id,
            channel_label: surface.catalog.label,
            account_ids,
            reasons,
            preferred_owner_pids,
            cleanup_owner_pids,
            last_duplicate_reclaim_at,
            last_duplicate_reclaim_cleanup_owner_pids,
            recent_incidents,
        });
    }

    surfaces
}

pub(super) fn doctor_checks_json_payload(
    checks: &[DoctorCheck],
    channel_surfaces: &[mvp::channel::ChannelSurface],
) -> Vec<Value> {
    let account_summaries = doctor_plugin_bridge_account_summaries(channel_surfaces);
    let runtime_attention_surfaces = managed_bridge_runtime_attention_surfaces(channel_surfaces);
    let mut payload = Vec::with_capacity(checks.len());

    for check in checks {
        let mut object = serde_json::Map::new();
        let level = check_level_json(check.level).to_owned();
        let account_summary = account_summaries.get(check.name.as_str());

        object.insert("name".to_owned(), Value::String(check.name.clone()));
        object.insert("level".to_owned(), Value::String(level));
        object.insert("detail".to_owned(), Value::String(check.detail.clone()));

        if let Some(account_summary) = account_summary {
            object.insert(
                "plugin_bridge_account_summary".to_owned(),
                Value::String(account_summary.clone()),
            );
        }

        if let Some(reason) = doctor_runtime_attention_reason(check) {
            let mut runtime_attention = serde_json::Map::new();
            runtime_attention.insert(
                "reason".to_owned(),
                Value::String(reason.as_str().to_owned()),
            );
            runtime_attention.insert(
                "remediation".to_owned(),
                Value::String(reason.remediation().to_owned()),
            );
            if let Some(channel_id) = doctor_runtime_attention_channel_id(check) {
                runtime_attention
                    .insert("channel_id".to_owned(), Value::String(channel_id.clone()));
                if let Some(surface) = runtime_attention_surfaces
                    .iter()
                    .find(|surface| surface.channel_id == channel_id.as_str())
                {
                    if !surface.preferred_owner_pids.is_empty() {
                        runtime_attention.insert(
                            "preferred_owner_pids".to_owned(),
                            serde_json::json!(surface.preferred_owner_pids),
                        );
                    }
                    if !surface.cleanup_owner_pids.is_empty() {
                        runtime_attention.insert(
                            "cleanup_owner_pids".to_owned(),
                            serde_json::json!(surface.cleanup_owner_pids),
                        );
                    }
                    if let Some(last_duplicate_reclaim_at) = surface.last_duplicate_reclaim_at {
                        runtime_attention.insert(
                            "last_duplicate_reclaim_at".to_owned(),
                            serde_json::json!(last_duplicate_reclaim_at),
                        );
                    }
                    if !surface.last_duplicate_reclaim_cleanup_owner_pids.is_empty() {
                        runtime_attention.insert(
                            "last_duplicate_reclaim_cleanup_owner_pids".to_owned(),
                            serde_json::json!(surface.last_duplicate_reclaim_cleanup_owner_pids),
                        );
                    }
                    if !surface.recent_incidents.is_empty() {
                        runtime_attention.insert(
                            "recent_incidents".to_owned(),
                            Value::Array(
                                surface
                                    .recent_incidents
                                    .iter()
                                    .map(|incident| {
                                        serde_json::json!({
                                            "account_id": incident.account_id,
                                            "account_label": incident.account_label,
                                            "kind": incident.kind,
                                            "at_ms": incident.at_ms,
                                            "detail": incident.detail,
                                            "owner_pids": incident.owner_pids,
                                        })
                                    })
                                    .collect(),
                            ),
                        );
                    }
                }
            }
            object.insert(
                "runtime_attention".to_owned(),
                Value::Object(runtime_attention),
            );
        }

        payload.push(Value::Object(object));
    }

    payload
}

pub(super) fn doctor_plugin_bridge_account_summaries(
    channel_surfaces: &[mvp::channel::ChannelSurface],
) -> BTreeMap<String, String> {
    let mut summaries = BTreeMap::new();

    for surface in channel_surfaces {
        let account_summary = plugin_bridge_account_summary(surface);
        let Some(account_summary) = account_summary else {
            continue;
        };

        let check_name = format!("{} managed bridge discovery", surface.catalog.id);
        summaries.insert(check_name, account_summary);
    }

    summaries
}

#[cfg(test)]
pub(super) fn build_doctor_next_steps(
    checks: &[DoctorCheck],
    config_path: &Path,
    config: &mvp::config::LoongConfig,
    fix_requested: bool,
) -> Vec<String> {
    let path_env = env::var_os("PATH");
    build_doctor_next_steps_with_path_env(
        checks,
        config_path,
        config,
        fix_requested,
        path_env.as_deref(),
    )
}

#[cfg(test)]
pub(super) fn build_doctor_next_steps_with_path_env(
    checks: &[DoctorCheck],
    config_path: &Path,
    config: &mvp::config::LoongConfig,
    fix_requested: bool,
    path_env: Option<&OsStr>,
) -> Vec<String> {
    let inventory = mvp::channel::channel_inventory(config);
    build_doctor_next_steps_with_channel_surfaces_and_path_env(
        checks,
        config_path,
        config,
        &inventory.channel_surfaces,
        fix_requested,
        path_env,
    )
}

pub(super) fn build_doctor_next_steps_with_channel_surfaces_and_path_env(
    checks: &[DoctorCheck],
    config_path: &Path,
    config: &mvp::config::LoongConfig,
    channel_surfaces: &[mvp::channel::ChannelSurface],
    fix_requested: bool,
    path_env: Option<&OsStr>,
) -> Vec<String> {
    let mut steps = Vec::new();
    let config_path_display = config_path.display().to_string();
    let rerun_command =
        crate::cli_handoff::format_subcommand_with_config("doctor", &config_path_display);
    let rerun_onboard_command =
        crate::cli_handoff::format_subcommand_with_config("onboard", &config_path_display);

    if !fix_requested
        && checks.iter().any(|check| {
            check.detail.contains("rerun with --fix")
                || matches!(
                    check.name.as_str(),
                    "memory path" | "tool file root" | "tool file root policy"
                )
                || check.name.ends_with("policy")
        })
    {
        push_unique_step(
            &mut steps,
            format!("Apply safe local repairs: {rerun_command} --fix"),
        );
    }

    if checks
        .iter()
        .any(|check| check.name == "provider credentials" && check.level != DoctorCheckLevel::Pass)
    {
        let hints = provider_credential_policy::provider_credential_env_hints(&config.provider);
        if !hints.is_empty() {
            push_unique_step(
                &mut steps,
                format!("Set provider credentials in env: {}", hints.join(" or ")),
            );
        }
    }

    for surface in managed_bridge_runtime_attention_surfaces(channel_surfaces) {
        if surface.reasons.contains("retrying") {
            push_unique_step(
                &mut steps,
                format!(
                    "Inspect {} bridge connectivity, upstream session health, and external bridge logs, then rerun diagnostics: {rerun_command}",
                    surface.channel_label
                ),
            );
        }
        if surface.reasons.contains("stale") {
            let stop_command = super::managed_bridge_runtime_serve_control_command(
                &surface,
                config_path_display.as_str(),
                false,
            );
            push_unique_step(
                &mut steps,
                match stop_command {
                    Some(stop_command) => format!(
                        "Restart the stale {} runtime or external bridge owner: {stop_command}",
                        surface.channel_label
                    ),
                    None => format!(
                        "Restart the stale {} runtime or external bridge owner, then rerun diagnostics: {rerun_command}",
                        surface.channel_label
                    ),
                },
            );
        }
        if surface.reasons.contains("duplicate_runtime_instances") {
            let stop_command = super::managed_bridge_runtime_serve_control_command(
                &surface,
                config_path_display.as_str(),
                true,
            );
            let keep_pid_note = if surface.preferred_owner_pids.len() == 1 {
                let pid = surface
                    .preferred_owner_pids
                    .first()
                    .copied()
                    .unwrap_or_default();
                format!("keep pid={pid}; ")
            } else {
                String::new()
            };
            let cleanup_pid_note = if surface.cleanup_owner_pids.is_empty() {
                String::new()
            } else {
                let rendered_cleanup = surface
                    .cleanup_owner_pids
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                format!("cleanup pids={rendered_cleanup}; ")
            };
            let auto_reclaim_note = if let Some(last_duplicate_reclaim_at) =
                surface.last_duplicate_reclaim_at
            {
                let rendered_cleanup = render_u32_list(
                    &surface
                        .last_duplicate_reclaim_cleanup_owner_pids
                        .iter()
                        .copied()
                        .collect::<Vec<_>>(),
                );
                format!(
                    "last auto reclaim at={last_duplicate_reclaim_at}; last auto cleanup pids={rendered_cleanup}; "
                )
            } else {
                String::new()
            };
            push_unique_step(
                &mut steps,
                match stop_command {
                    Some(stop_command) => format!(
                        "Stop duplicate {} runtime instances so only one serve owner remains ({auto_reclaim_note}{keep_pid_note}{cleanup_pid_note}run {stop_command})",
                        surface.channel_label
                    ),
                    None => format!(
                        "Stop duplicate {} runtime instances so only one serve owner remains ({auto_reclaim_note}{keep_pid_note}{cleanup_pid_note}then rerun diagnostics: {rerun_command})",
                        surface.channel_label
                    ),
                },
            );
        }
    }

    if checks.iter().any(|check| {
        check.name == crate::access_terms::QUERY_SEARCH_PROVIDER_LABEL
            && check.level != DoctorCheckLevel::Pass
    }) {
        for step in crate::query_search_surface::query_search_repair_steps(
            config,
            rerun_onboard_command.as_str(),
        ) {
            push_unique_step(&mut steps, step);
        }
    }

    let provider_model_probe_recovery =
        super::provider_model_probe_recovery_advice_for_checks(checks, config);
    if let Some(provider_model_probe_recovery) = provider_model_probe_recovery {
        let provider_model_probe_policy::ProviderModelProbeRecoveryAdvice {
            kind: provider_model_probe_kind,
            region_endpoint_hint,
        } = provider_model_probe_recovery;
        let is_transport_failure = matches!(
            provider_model_probe_kind,
            provider_model_probe_policy::ProviderModelProbeFailureKind::TransportFailure
        );
        if is_transport_failure {
            if checks.iter().any(|check| {
                check.name == crate::provider::route_diagnostics::PROVIDER_ROUTE_PROBE_CHECK_NAME
                    && check.level != DoctorCheckLevel::Pass
            }) {
                push_unique_step(
                    &mut steps,
                    format!(
                        "Fix the active provider route (DNS / proxy / TUN), then re-run diagnostics: {rerun_command}"
                    ),
                );
                if checks.iter().any(|check| {
                    check.name
                        == crate::provider::route_diagnostics::PROVIDER_ROUTE_PROBE_CHECK_NAME
                        && check.detail.contains("fake-ip-style")
                }) {
                    push_unique_step(
                        &mut steps,
                        "If the provider host should bypass proxying, add it to your direct/bypass rules; otherwise keep the fake-ip/TUN proxy healthy before retrying.".to_owned(),
                    );
                }
            } else {
                push_unique_step(
                    &mut steps,
                    format!(
                        "Re-run diagnostics after checking the active provider route: {rerun_command}"
                    ),
                );
            }
        } else {
            match provider_model_probe_kind {
                provider_model_probe_policy::ProviderModelProbeFailureKind::TransportFailure => {}
                provider_model_probe_policy::ProviderModelProbeFailureKind::RequiresExplicitModel {
                    recommended_onboarding_model: Some(model),
                } => {
                    push_unique_step(
                        &mut steps,
                        format!(
                            "Rerun onboarding and accept reviewed model `{model}`: {rerun_onboard_command}"
                        ),
                    );
                    push_unique_step(
                        &mut steps,
                        format!(
                            "Or set `provider.model` / `preferred_models` explicitly, then re-run diagnostics: {rerun_command}"
                        ),
                    );
                }
                provider_model_probe_policy::ProviderModelProbeFailureKind::RequiresExplicitModel {
                    recommended_onboarding_model: None,
                } => {
                    push_unique_step(
                        &mut steps,
                        format!(
                            "Set `provider.model` / `preferred_models` explicitly, then re-run diagnostics: {rerun_command}"
                        ),
                    );
                }
                provider_model_probe_policy::ProviderModelProbeFailureKind::ExplicitModel { .. }
                | provider_model_probe_policy::ProviderModelProbeFailureKind::PreferredModels {
                    ..
                } => {
                    push_unique_step(
                        &mut steps,
                        format!(
                            "Retry provider probe only after credentials are ready: {rerun_command}"
                        ),
                    );
                    push_unique_step(
                        &mut steps,
                        format!(
                            "If your provider blocks model listing during setup, retry with: {rerun_command} --skip-model-probe"
                        ),
                    );
                }
            }
            if let Some(hint) = region_endpoint_hint {
                push_unique_step(&mut steps, hint);
            }
        }
    }

    if checks
        .iter()
        .any(|check| check.name == "audit retention" && check.level == DoctorCheckLevel::Warn)
    {
        push_unique_step(
            &mut steps,
            "Switch to durable audit retention: set [audit].mode = \"fanout\"".to_owned(),
        );
    }

    if checks
        .iter()
        .any(|check| check.name == "audit retention" && check.level == DoctorCheckLevel::Fail)
    {
        push_unique_step(
            &mut steps,
            format!(
                "Point [audit].path at a writable journal file path, then re-run diagnostics: {rerun_command}"
            ),
        );
    }

    let runtime_snapshot_json_command = format!(
        "{} runtime snapshot --json --config {}",
        mvp::config::CLI_COMMAND_NAME,
        crate::cli_handoff::shell_quote_argument(&config_path_display),
    );
    if checks.iter().any(|check| {
        check.name == "runtime plugins runtime" && check.level != DoctorCheckLevel::Pass
    }) {
        let runtime_plugins_disabled = !config.runtime_plugins.enabled;
        if runtime_plugins_disabled {
            push_unique_step(
                &mut steps,
                format!(
                    "Enable runtime plugins by setting [runtime_plugins].enabled = true, then re-run diagnostics: {rerun_command}"
                ),
            );
        } else {
            push_unique_step(
                &mut steps,
                format!(
                    "Review runtime plugin roots and support policy in config, then re-run diagnostics: {rerun_command}"
                ),
            );
            push_unique_step(
                &mut steps,
                format!("Inspect runtime plugin inventory: {runtime_snapshot_json_command}"),
            );
        }
    }
    if checks.iter().any(|check| {
        check.name == "runtime plugins inventory" && check.level != DoctorCheckLevel::Pass
    }) {
        push_unique_step(
            &mut steps,
            format!("Inspect runtime plugin inventory: {runtime_snapshot_json_command}"),
        );
        push_unique_step(
            &mut steps,
            format!(
                "Review [runtime_plugins].roots, [runtime_plugins].supported_bridges, [runtime_plugins].supported_adapter_families, and package manifests, then re-run diagnostics: {rerun_command}"
            ),
        );
    }

    push_managed_bridge_discovery_next_steps(&mut steps, channel_surfaces, &rerun_command);

    let channel_actions =
        crate::migration::channels::collect_channel_next_actions(config, &config_path_display);
    if checks.iter().any(|check| {
        check.level != DoctorCheckLevel::Pass
            && (check.name.contains("channel")
                || check.name.contains("default account policy")
                || channel_actions
                    .iter()
                    .any(|action| check.name.to_ascii_lowercase().contains(action.id)))
    }) {
        for action in &channel_actions {
            push_unique_step(
                &mut steps,
                format!("Bring {} online: {}", action.label, action.command),
            );
        }
    }

    if doctor_ready_for_first_turn(checks) {
        for action in select_doctor_first_turn_actions(
            crate::next_actions::collect_setup_next_actions_with_path_env(
                config,
                &config_path_display,
                path_env,
            ),
        ) {
            let prefix = match action.kind {
                crate::next_actions::SetupNextActionKind::Ask => "Get a first answer",
                crate::next_actions::SetupNextActionKind::Chat => "Continue in chat",
                crate::next_actions::SetupNextActionKind::Personalize => {
                    "Set your working preferences"
                }
                crate::next_actions::SetupNextActionKind::Channel => "Open a channel",
                crate::next_actions::SetupNextActionKind::Doctor => "Run diagnostics",
            };
            push_unique_step(&mut steps, format!("{prefix}: {}", action.command));
        }
    }

    if (!checks.is_empty() && steps.is_empty())
        || checks
            .iter()
            .any(|check| check.level != DoctorCheckLevel::Pass)
    {
        push_unique_step(&mut steps, format!("Re-run diagnostics: {rerun_command}"));
    }

    steps
}

fn push_managed_bridge_discovery_next_steps(
    steps: &mut Vec<String>,
    channel_surfaces: &[mvp::channel::ChannelSurface],
    rerun_command: &str,
) {
    for surface in channel_surfaces {
        let has_plugin_bridge_contract = surface.catalog.plugin_bridge_contract.is_some();

        if !has_plugin_bridge_contract {
            continue;
        }

        let has_enabled_account = surface
            .configured_accounts
            .iter()
            .any(|snapshot| snapshot.enabled);

        if !has_enabled_account {
            continue;
        }

        let Some(discovery) = surface.plugin_bridge_discovery.as_ref() else {
            continue;
        };

        push_managed_bridge_ambiguity_next_step(steps, surface, discovery);
        push_managed_bridge_selection_next_step(steps, surface, discovery);
        push_managed_bridge_incomplete_setup_next_steps(steps, surface, discovery);
    }

    let has_managed_bridge_guidance = steps.iter().any(|step| {
        step.contains("Resolve managed bridge ambiguity")
            || step.contains("Fix managed bridge selection")
            || step.contains("Complete managed bridge setup")
    });

    if has_managed_bridge_guidance {
        push_unique_step(steps, format!("Re-run diagnostics: {rerun_command}"));
    }
}

fn push_managed_bridge_selection_next_step(
    steps: &mut Vec<String>,
    surface: &mvp::channel::ChannelSurface,
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
) {
    let selection_status = discovery.selection_status;
    let Some(selection_status) = selection_status else {
        return;
    };

    match selection_status {
        mvp::channel::ChannelPluginBridgeSelectionStatus::ConfiguredPluginNotFound => {
            let configured_plugin_id = crate::render_line_safe_optional_text_value(
                discovery.configured_plugin_id.as_deref(),
            );
            let compatible_plugin_ids = render_managed_bridge_compatible_plugin_labels(discovery);
            let step = format!(
                "Fix managed bridge selection for {}: configured managed_bridge_plugin_id={} was not found; compatible plugins={compatible_plugin_ids}",
                surface.catalog.id, configured_plugin_id
            );

            push_unique_step(steps, step);
        }
        mvp::channel::ChannelPluginBridgeSelectionStatus::ConfiguredPluginIdDuplicated => {
            let configured_plugin_id = crate::render_line_safe_optional_text_value(
                discovery.configured_plugin_id.as_deref(),
            );
            let matching_plugin_labels = render_managed_bridge_configured_plugin_labels(discovery);
            let step = format!(
                "Fix managed bridge selection for {}: configured managed_bridge_plugin_id={} matches multiple managed packages={matching_plugin_labels}; keep one package per plugin_id or rename duplicates",
                surface.catalog.id, configured_plugin_id
            );

            push_unique_step(steps, step);
        }
        mvp::channel::ChannelPluginBridgeSelectionStatus::ConfiguredPluginIncompatible => {
            let configured_plugin_id = crate::render_line_safe_optional_text_value(
                discovery.configured_plugin_id.as_deref(),
            );
            let step = format!(
                "Fix managed bridge selection for {}: configured managed_bridge_plugin_id={} does not satisfy the channel bridge contract",
                surface.catalog.id, configured_plugin_id
            );

            push_unique_step(steps, step);
        }
        mvp::channel::ChannelPluginBridgeSelectionStatus::NotConfigured => {}
        mvp::channel::ChannelPluginBridgeSelectionStatus::SingleCompatibleMatch => {}
        mvp::channel::ChannelPluginBridgeSelectionStatus::SelectedCompatible => {}
        mvp::channel::ChannelPluginBridgeSelectionStatus::ConfiguredPluginIncomplete => {}
    }
}

fn push_managed_bridge_ambiguity_next_step(
    steps: &mut Vec<String>,
    surface: &mvp::channel::ChannelSurface,
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
) {
    let ambiguity_status = discovery.ambiguity_status;
    let Some(ambiguity_status) = ambiguity_status else {
        return;
    };

    let step = match ambiguity_status {
        mvp::channel::ChannelPluginBridgeDiscoveryAmbiguityStatus::MultipleCompatiblePlugins => {
            let compatible_plugin_ids = render_managed_bridge_compatible_plugin_labels(discovery);

            format!(
                "Resolve managed bridge ambiguity for {}: keep exactly one compatible plugin ({compatible_plugin_ids})",
                surface.catalog.id
            )
        }
        mvp::channel::ChannelPluginBridgeDiscoveryAmbiguityStatus::DuplicateCompatiblePluginIds => {
            let compatible_plugin_ids = render_managed_bridge_compatible_plugin_labels(discovery);

            format!(
                "Resolve managed bridge ambiguity for {}: duplicate compatible plugin_id values were discovered ({compatible_plugin_ids}); keep one package per plugin_id or rename duplicates",
                surface.catalog.id
            )
        }
    };

    push_unique_step(steps, step);
}

fn push_managed_bridge_incomplete_setup_next_steps(
    steps: &mut Vec<String>,
    surface: &mvp::channel::ChannelSurface,
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
) {
    let duplicate_plugin_id_counts = managed_bridge_duplicate_plugin_id_counts(&discovery.plugins);

    for plugin in &discovery.plugins {
        let is_incomplete = matches!(
            plugin.status,
            mvp::channel::ChannelDiscoveredPluginBridgeStatus::CompatibleIncompleteContract
                | mvp::channel::ChannelDiscoveredPluginBridgeStatus::MissingSetupSurface
        );

        if !is_incomplete {
            continue;
        }

        let step =
            managed_bridge_incomplete_setup_step(surface, plugin, &duplicate_plugin_id_counts);
        push_unique_step(steps, step);
    }
}

pub(super) fn managed_bridge_incomplete_setup_step(
    surface: &mvp::channel::ChannelSurface,
    plugin: &mvp::channel::ChannelDiscoveredPluginBridge,
    duplicate_plugin_id_counts: &BTreeMap<String, usize>,
) -> String {
    let mut segments = Vec::new();
    let plugin_label = managed_bridge_plugin_label(plugin, duplicate_plugin_id_counts);
    let rendered_plugin_label = crate::render_line_safe_text_value(&plugin_label);
    let prefix = format!(
        "Complete managed bridge setup for {} plugin {}",
        surface.catalog.id, rendered_plugin_label
    );
    segments.push(prefix);

    if !plugin.missing_fields.is_empty() {
        let missing_fields = crate::render_line_safe_text_values(
            plugin.missing_fields.iter().map(String::as_str),
            ",",
        );
        segments.push(format!("missing contract fields: {missing_fields}"));
    }

    if !plugin.required_env_vars.is_empty() {
        let required_env_vars = crate::render_line_safe_text_values(
            plugin.required_env_vars.iter().map(String::as_str),
            ",",
        );
        segments.push(format!("required env: {required_env_vars}"));
    }

    if !plugin.required_config_keys.is_empty() {
        let required_config_keys = crate::render_line_safe_text_values(
            plugin.required_config_keys.iter().map(String::as_str),
            ",",
        );
        segments.push(format!("required config keys: {required_config_keys}"));
    }

    if let Some(default_env_var) = &plugin.default_env_var {
        let rendered_default_env_var = crate::render_line_safe_text_value(default_env_var);
        segments.push(format!("default env var: {rendered_default_env_var}"));
    }

    if !plugin.setup_docs_urls.is_empty() {
        let setup_docs_urls = crate::render_line_safe_text_values(
            plugin.setup_docs_urls.iter().map(String::as_str),
            ",",
        );
        segments.push(format!("docs: {setup_docs_urls}"));
    }

    if let Some(setup_remediation) = &plugin.setup_remediation {
        let rendered_setup_remediation = crate::render_line_safe_text_value(setup_remediation);
        segments.push(format!("remediation: {rendered_setup_remediation}"));
    }

    let has_only_prefix = segments.len() == 1;

    if has_only_prefix {
        segments.push(
            "verify setup.surface plus bridge metadata (transport_family / target_contract) in the managed plugin manifest"
                .to_owned(),
        );
    }

    segments.join("; ")
}

pub(super) fn select_doctor_first_turn_actions(
    actions: Vec<crate::next_actions::SetupNextAction>,
) -> Vec<crate::next_actions::SetupNextAction> {
    let mut prioritized = Vec::new();

    push_first_matching_action(&mut prioritized, &actions, |action| {
        action.kind == crate::next_actions::SetupNextActionKind::Ask
    });
    push_first_matching_action(&mut prioritized, &actions, |action| {
        action.kind == crate::next_actions::SetupNextActionKind::Chat
    });
    push_first_matching_action(&mut prioritized, &actions, |action| {
        action.kind == crate::next_actions::SetupNextActionKind::Personalize
    });
    push_first_matching_action(&mut prioritized, &actions, |action| {
        is_channel_catalog_action(action)
    });

    for action in actions {
        if action.kind == crate::next_actions::SetupNextActionKind::Doctor {
            continue;
        }

        push_unique_action(&mut prioritized, action);
        if prioritized.len() == 3 {
            break;
        }
    }

    prioritized.truncate(3);
    prioritized
}

pub(super) fn is_channel_catalog_action(action: &crate::next_actions::SetupNextAction) -> bool {
    let kind = &action.kind;
    let channel_action_id = action.channel_action_id;
    *kind == crate::next_actions::SetupNextActionKind::Channel
        && channel_action_id == Some(crate::migration::channels::CHANNEL_CATALOG_ACTION_ID)
}

fn push_first_matching_action<F>(
    prioritized: &mut Vec<crate::next_actions::SetupNextAction>,
    actions: &[crate::next_actions::SetupNextAction],
    predicate: F,
) where
    F: Fn(&crate::next_actions::SetupNextAction) -> bool,
{
    if let Some(action) = actions.iter().find(|action| predicate(action)) {
        push_unique_action(prioritized, action.clone());
    }
}

fn push_unique_action(
    prioritized: &mut Vec<crate::next_actions::SetupNextAction>,
    action: crate::next_actions::SetupNextAction,
) {
    if prioritized
        .iter()
        .all(|existing| existing.command != action.command)
    {
        prioritized.push(action);
    }
}

fn push_unique_step(steps: &mut Vec<String>, step: String) {
    if !steps.iter().any(|existing| existing == &step) {
        steps.push(step);
    }
}

fn snapshot_has_external_plugin_bridge_owner(
    snapshot: &mvp::channel::ChannelStatusSnapshot,
) -> bool {
    let bridge_runtime_owner = super::snapshot_note_value(snapshot, "bridge_runtime_owner");
    bridge_runtime_owner == Some("external_plugin")
}
