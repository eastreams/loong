use std::collections::{BTreeMap, BTreeSet};

use crate::{
    PluginInventoryResult, PluginPreflightResult,
    plugins_cli::{
        PluginsActionPlanItemView, PluginsActionsExecution, PluginsBridgeProfilesExecution,
        PluginsBridgeShimProfileDeltaView, PluginsBridgeTemplateExecution, PluginsCommandExecution,
        PluginsDoctorExecution, PluginsDoctorSummaryView, PluginsInitExecution,
        PluginsInventoryExecution, PluginsInventorySummaryView, PluginsPreflightExecution,
        PluginsPreflightSummaryView,
    },
};

pub(super) fn summarize_plugin_inventory_results(
    results: &[PluginInventoryResult],
) -> PluginsInventorySummaryView {
    let returned_plugins = results.len();
    let mut ready_plugins = 0;
    let mut setup_incomplete_plugins = 0;
    let mut blocked_plugins = 0;
    let mut deferred_plugins = 0;
    let mut loaded_plugins = 0;
    let mut source_kind_distribution = BTreeMap::new();
    let mut bridge_kind_distribution = BTreeMap::new();
    let mut source_language_distribution = BTreeMap::new();
    let mut setup_surface_distribution = BTreeMap::new();
    let mut activation_status_distribution = BTreeMap::new();

    for result in results {
        let activation_status = result.activation_status.as_deref();
        if activation_status == Some("ready") {
            ready_plugins += 1;
        }
        if activation_status == Some("setup_incomplete") {
            setup_incomplete_plugins += 1;
        }
        if activation_status.is_some_and(plugin_inventory_status_is_blocked) {
            blocked_plugins += 1;
        }
        if result.deferred {
            deferred_plugins += 1;
        }
        if result.loaded {
            loaded_plugins += 1;
        }

        increment_rollup_count(&mut source_kind_distribution, result.source_kind.as_str());
        increment_rollup_count(&mut bridge_kind_distribution, result.bridge_kind.as_str());
        increment_rollup_count(
            &mut source_language_distribution,
            result.source_language.as_deref().unwrap_or("unknown"),
        );
        increment_rollup_count(
            &mut setup_surface_distribution,
            inventory_result_setup_surface_label(result),
        );
        increment_rollup_count(
            &mut activation_status_distribution,
            inventory_result_status_label(result),
        );
    }

    PluginsInventorySummaryView {
        returned_plugins,
        ready_plugins,
        setup_incomplete_plugins,
        blocked_plugins,
        deferred_plugins,
        loaded_plugins,
        source_kind_distribution,
        bridge_kind_distribution,
        source_language_distribution,
        setup_surface_distribution,
        activation_status_distribution,
    }
}

pub(super) fn summarize_plugin_doctor_results(
    results: &[PluginPreflightResult],
    preflight_summary: &PluginsPreflightSummaryView,
) -> PluginsDoctorSummaryView {
    let mut activation_ready_plugins = 0_usize;
    let mut setup_incomplete_plugins = 0_usize;
    let mut deferred_plugins = 0_usize;
    let mut loaded_plugins = 0_usize;
    let mut packages_with_operator_actions = 0_usize;
    let mut total_recommended_actions = 0_usize;
    let mut total_operator_actions = 0_usize;
    let mut bridge_kind_distribution = BTreeMap::new();
    let mut source_language_distribution = BTreeMap::new();
    let mut setup_surface_distribution = BTreeMap::new();
    let mut activation_status_distribution = BTreeMap::new();

    for result in results {
        let plugin = &result.plugin;
        if result.activation_ready {
            activation_ready_plugins = activation_ready_plugins.saturating_add(1);
        }
        if plugin.activation_status.as_deref() == Some("setup_incomplete") {
            setup_incomplete_plugins = setup_incomplete_plugins.saturating_add(1);
        }
        if plugin.deferred {
            deferred_plugins = deferred_plugins.saturating_add(1);
        }
        if plugin.loaded {
            loaded_plugins = loaded_plugins.saturating_add(1);
        }

        total_recommended_actions =
            total_recommended_actions.saturating_add(result.recommended_actions.len());
        let operator_action_count = count_preflight_result_operator_actions(result);
        total_operator_actions = total_operator_actions.saturating_add(operator_action_count);
        if operator_action_count > 0 {
            packages_with_operator_actions = packages_with_operator_actions.saturating_add(1);
        }

        increment_rollup_count(&mut bridge_kind_distribution, plugin.bridge_kind.as_str());
        increment_rollup_count(
            &mut source_language_distribution,
            plugin.source_language.as_deref().unwrap_or("unknown"),
        );
        increment_rollup_count(
            &mut setup_surface_distribution,
            inventory_result_setup_surface_label(plugin),
        );
        increment_rollup_count(
            &mut activation_status_distribution,
            inventory_result_status_label(plugin),
        );
    }

    PluginsDoctorSummaryView {
        matched_plugins: preflight_summary.matched_plugins,
        returned_plugins: results.len(),
        passed_plugins: preflight_summary.passed_plugins,
        warned_plugins: preflight_summary.warned_plugins,
        blocked_plugins: preflight_summary.blocked_plugins,
        activation_ready_plugins,
        setup_incomplete_plugins,
        deferred_plugins,
        loaded_plugins,
        packages_requiring_author_attention: preflight_summary
            .warned_plugins
            .saturating_add(preflight_summary.blocked_plugins),
        packages_with_operator_actions,
        total_recommended_actions,
        total_operator_actions,
        remediation_counts: preflight_summary.remediation_counts.clone(),
        bridge_kind_distribution,
        source_language_distribution,
        setup_surface_distribution,
        activation_status_distribution,
    }
}

pub(super) fn summarize_filtered_actions(
    actions: &[PluginsActionPlanItemView],
) -> (
    BTreeMap<String, usize>,
    BTreeMap<String, usize>,
    usize,
    usize,
) {
    let mut by_surface = BTreeMap::new();
    let mut by_kind = BTreeMap::new();
    let mut requiring_reload = 0_usize;
    let mut without_reload = 0_usize;
    for item in actions {
        *by_surface.entry(item.action.surface.clone()).or_default() += 1;
        *by_kind.entry(item.action.kind.clone()).or_default() += 1;
        if item.action.requires_reload {
            requiring_reload = requiring_reload.saturating_add(1);
        } else {
            without_reload = without_reload.saturating_add(1);
        }
    }
    (by_surface, by_kind, requiring_reload, without_reload)
}

pub(super) fn render_plugins_cli_text(execution: &PluginsCommandExecution) -> String {
    let (title, body) = match execution {
        PluginsCommandExecution::Init(execution) => {
            ("plugins init", render_plugins_init_text(execution))
        }
        PluginsCommandExecution::Inventory(execution) => (
            "plugins inventory",
            render_plugins_inventory_text(execution),
        ),
        PluginsCommandExecution::Doctor(execution) => {
            ("plugins doctor", render_plugins_doctor_text(execution))
        }
        PluginsCommandExecution::BridgeProfiles(execution) => (
            "bridge profiles",
            render_plugins_bridge_profiles_text(execution),
        ),
        PluginsCommandExecution::BridgeTemplate(execution) => (
            "bridge template",
            render_plugins_bridge_template_text(execution),
        ),
        PluginsCommandExecution::Preflight(execution) => (
            "plugins preflight",
            render_plugins_preflight_text(execution),
        ),
        PluginsCommandExecution::Actions(execution) => {
            ("operator actions", render_plugins_actions_text(execution))
        }
    };
    crate::render_operator_shell_surface_from_body(title, "operator plugins", body)
}

fn render_plugins_init_text(execution: &PluginsInitExecution) -> String {
    let source_language = execution.source_language.as_deref().unwrap_or("-");
    [
        format!(
            "plugins init package_root={} plugin_id={} provider_id={} connector_name={}",
            execution.package_root,
            execution.plugin_id,
            execution.provider_id,
            execution.connector_name
        ),
        format!(
            "- bridge_kind={} source_language={} adapter_family={} entrypoint={}",
            execution.bridge_kind, source_language, execution.adapter_family, execution.entrypoint
        ),
        format!("- manifest={}", execution.manifest_path),
        format!("- readme={}", execution.readme_path),
        format!(
            "- next_steps=loong plugins doctor --root \"{}\" --profile sdk-release",
            execution.package_root
        ),
        format!(
            "- operator_actions=loong plugins actions --root \"{}\" --profile sdk-release",
            execution.package_root
        ),
    ]
    .join("\n")
}

fn render_plugins_inventory_text(execution: &PluginsInventoryExecution) -> String {
    let mut lines = vec![format!(
        "plugins inventory query={} roots={} returned_plugins={} ready={} setup_incomplete={} blocked={} deferred={} loaded={}",
        display_text_or_dash(Some(execution.query.as_str())),
        execution.scan_roots.join(","),
        execution.returned_results,
        execution.summary.ready_plugins,
        execution.summary.setup_incomplete_plugins,
        execution.summary.blocked_plugins,
        execution.summary.deferred_plugins,
        execution.summary.loaded_plugins
    )];
    lines.push(format!(
        "bridge_support source={} sha256={}",
        display_text_or_dash(execution.bridge_support_source.as_deref()),
        display_text_or_dash(execution.bridge_support_sha256.as_deref())
    ));
    lines.push(format!(
        "bridge_support_delta source={} sha256={}",
        display_text_or_dash(execution.bridge_support_delta_source.as_deref()),
        display_text_or_dash(execution.bridge_support_delta_sha256.as_deref())
    ));
    lines.push(format!(
        "ecosystem source_kind={} bridge={} language={} setup_surface={} activation_status={}",
        format_rollup_map(&execution.summary.source_kind_distribution),
        format_rollup_map(&execution.summary.bridge_kind_distribution),
        format_rollup_map(&execution.summary.source_language_distribution),
        format_rollup_map(&execution.summary.setup_surface_distribution),
        format_rollup_map(&execution.summary.activation_status_distribution)
    ));
    for result in &execution.results {
        lines.extend(render_inventory_result_lines(result));
    }
    lines.join("\n")
}

fn render_inventory_result_lines(result: &PluginInventoryResult) -> Vec<String> {
    let activation_status = inventory_result_status_label(result);
    let setup_surface = inventory_result_setup_surface_label(result);
    let source_language = result.source_language.as_deref().unwrap_or("-");
    let manifest_path = display_text_or_dash(result.package_manifest_path.as_deref());
    let setup_mode = display_text_or_dash(result.setup_mode.as_deref());
    let host_api = result
        .compatibility
        .as_ref()
        .and_then(|compatibility| compatibility.host_api.as_deref());
    let host_version_req = result
        .compatibility
        .as_ref()
        .and_then(|compatibility| compatibility.host_version_req.as_deref());
    let required_env_vars = format_csv_or_dash(&result.setup_required_env_vars);
    let required_config_keys = format_csv_or_dash(&result.setup_required_config_keys);
    let runtime_health = result
        .runtime_health
        .as_ref()
        .map(|health| health.status.as_str());
    let attestation = result
        .activation_attestation
        .as_ref()
        .map(|attestation| attestation.integrity.as_str());

    let mut lines = vec![
        format!(
            "- plugin={} provider={} status={} loaded={} deferred={} bridge={} language={} setup_surface={}",
            result.plugin_id,
            result.provider_id,
            activation_status,
            result.loaded,
            result.deferred,
            result.bridge_kind,
            source_language,
            setup_surface
        ),
        format!(
            "  manifest={} setup_mode={} required_env={} required_config={} host_api={} host_version_req={}",
            manifest_path,
            setup_mode,
            required_env_vars,
            required_config_keys,
            display_text_or_dash(host_api),
            display_text_or_dash(host_version_req)
        ),
        format!(
            "  source={} bootstrap_hint={} runtime_health={} attestation={} summary={}",
            result.source_path,
            display_text_or_dash(result.bootstrap_hint.as_deref()),
            display_text_or_dash(runtime_health),
            display_text_or_dash(attestation),
            display_text_or_dash(result.summary.as_deref())
        ),
    ];
    if let Some(reason) = result.activation_reason.as_deref() {
        lines.push(format!("  activation_reason={reason}"));
    }
    lines
}

fn render_plugins_doctor_text(execution: &PluginsDoctorExecution) -> String {
    let preflight_summary = &execution.preflight_summary;
    let mut lines = vec![format!(
        "plugins doctor profile={} query={} roots={} matched_plugins={} returned_plugins={} passed={} warned={} blocked={}",
        execution.profile,
        display_text_or_dash(Some(execution.query.as_str())),
        execution.scan_roots.join(","),
        execution.summary.matched_plugins,
        execution.returned_results,
        execution.summary.passed_plugins,
        execution.summary.warned_plugins,
        execution.summary.blocked_plugins
    )];
    lines.push(format!(
        "policy source={} version={} checksum={} sha256={}",
        preflight_summary.policy_source,
        display_text_or_dash(preflight_summary.policy_version.as_deref()),
        preflight_summary.policy_checksum,
        preflight_summary.policy_sha256
    ));
    lines.push(format!(
        "bridge_support source={} sha256={}",
        display_text_or_dash(execution.bridge_support_source.as_deref()),
        display_text_or_dash(execution.bridge_support_sha256.as_deref())
    ));
    lines.push(format!(
        "bridge_support_delta source={} sha256={}",
        display_text_or_dash(execution.bridge_support_delta_source.as_deref()),
        display_text_or_dash(execution.bridge_support_delta_sha256.as_deref())
    ));
    lines.push(format!(
        "ecosystem bridge={} language={} setup_surface={} activation_status={}",
        format_rollup_map(&execution.summary.bridge_kind_distribution),
        format_rollup_map(&execution.summary.source_language_distribution),
        format_rollup_map(&execution.summary.setup_surface_distribution),
        format_rollup_map(&execution.summary.activation_status_distribution)
    ));
    lines.push(format!(
        "doctor_attention activation_ready={} setup_incomplete={} deferred={} loaded={} attention={} remediation_counts={}",
        execution.summary.activation_ready_plugins,
        execution.summary.setup_incomplete_plugins,
        execution.summary.deferred_plugins,
        execution.summary.loaded_plugins,
        execution.summary.packages_requiring_author_attention,
        format_rollup_map(&execution.summary.remediation_counts)
    ));
    lines.push(format!(
        "doctor_actions recommended={} operator_actions={} packages_with_operator_actions={} operator_plan_by_kind={}",
        execution.summary.total_recommended_actions,
        execution.summary.total_operator_actions,
        execution.summary.packages_with_operator_actions,
        format_rollup_map(&preflight_summary.operator_action_counts_by_kind)
    ));
    lines.extend(render_bridge_profile_fit_lines(preflight_summary));
    for result in &execution.results {
        lines.extend(render_plugin_doctor_result_lines(result));
    }
    lines.join("\n")
}

fn render_plugins_bridge_profiles_text(execution: &PluginsBridgeProfilesExecution) -> String {
    let mut lines = vec![format!(
        "plugins bridge-profiles returned_profiles={}",
        execution.profiles.len()
    )];
    for profile in &execution.profiles {
        lines.push(format!(
            "- profile={} version={} source={} checksum={} sha256={}",
            profile.profile_id,
            profile.policy_version.as_deref().unwrap_or("-"),
            profile.source,
            profile.checksum,
            profile.sha256
        ));
        lines.push(format!(
            "  bridges={} compatibility={} shims={} execute_process_stdio={} execute_http_json={} enforce_supported={} enforce_execution_success={}",
            format_csv_or_dash(&profile.supported_bridges),
            format_csv_or_dash(&profile.supported_compatibility_modes),
            format_csv_or_dash(&profile.supported_compatibility_shims),
            profile.execute_process_stdio,
            profile.execute_http_json,
            profile.enforce_supported,
            profile.enforce_execution_success
        ));
        for shim in &profile.shim_support_profiles {
            lines.push(format!(
                "  shim={} family={} version={} dialects={} bridges={} adapter_families={} languages={}",
                shim.shim_id,
                shim.shim_family,
                display_text_or_dash(shim.version.as_deref()),
                format_csv_or_dash(&shim.supported_dialects),
                format_csv_or_dash(&shim.supported_bridges),
                format_csv_or_dash(&shim.supported_adapter_families),
                format_csv_or_dash(&shim.supported_source_languages)
            ));
        }
    }
    lines.join("\n")
}

fn render_plugins_bridge_template_text(execution: &PluginsBridgeTemplateExecution) -> String {
    let mut lines = vec![format!(
        "plugins bridge-template profile={} query={} roots={} matched_plugins={} template_kind={}",
        execution.profile,
        display_text_or_dash(Some(execution.query.as_str())),
        execution.scan_roots.join(","),
        execution.summary.matched_plugins,
        execution.template_kind
    )];
    lines.push(format!(
        "bridge_support source={} sha256={}",
        display_text_or_dash(execution.bridge_support_source.as_deref()),
        display_text_or_dash(execution.bridge_support_sha256.as_deref())
    ));
    lines.push(format!(
        "bridge_support_delta source={} sha256={}",
        display_text_or_dash(execution.bridge_support_delta_source.as_deref()),
        display_text_or_dash(execution.bridge_support_delta_sha256.as_deref())
    ));
    lines.extend(render_bridge_profile_fit_lines(&execution.summary));
    lines.push(format!(
        "template profile={} source={} version={} checksum={} sha256={} output={}",
        execution.template_profile_id,
        execution.template_source,
        display_text_or_dash(execution.template_policy_version.as_deref()),
        execution.template_checksum,
        execution.template_sha256,
        display_text_or_dash(execution.output_path.as_deref())
    ));
    lines.push(format!(
        "template_delta base_profile={} base_source={} base_version={} checksum={} sha256={} output={}",
        execution.delta_artifact.base_profile_id,
        execution.delta_artifact.base_source,
        display_text_or_dash(execution.delta_artifact.base_policy_version.as_deref()),
        execution.delta_artifact.checksum,
        execution.delta_artifact.sha256,
        display_text_or_dash(execution.delta_output_path.as_deref())
    ));
    lines.push(format!(
        "template_delta_support bridges={} compatibility={} adapter_families={} shims={} shim_profiles={} unresolved={}",
        format_csv_or_dash(&execution.delta_artifact.delta.supported_bridges),
        format_csv_or_dash(&execution.delta_artifact.delta.supported_compatibility_modes),
        format_csv_or_dash(&execution.delta_artifact.delta.supported_adapter_families),
        format_csv_or_dash(&execution.delta_artifact.delta.supported_compatibility_shims),
        format_bridge_shim_profile_delta_artifact(&execution.delta_artifact.delta.shim_profile_additions),
        format_csv_or_dash(&execution.delta_artifact.delta.unresolved_blocking_reasons)
    ));
    lines.push(format!(
        "template_support bridges={} compatibility={} shims={} execute_process_stdio={} execute_http_json={} enforce_supported={} enforce_execution_success={}",
        execution
            .template
            .supported_bridges
            .iter()
            .map(|bridge| bridge.as_str().to_owned())
            .collect::<Vec<_>>()
            .join(","),
        execution
            .template
            .supported_compatibility_modes
            .iter()
            .map(|mode| mode.as_str().to_owned())
            .collect::<Vec<_>>()
            .join(","),
        execution
            .template
            .supported_compatibility_shims
            .iter()
            .map(|shim| format!("{}:{}", shim.shim_id, shim.family))
            .collect::<Vec<_>>()
            .join(","),
        execution.template.execute_process_stdio,
        execution.template.execute_http_json,
        execution.template.enforce_supported,
        execution.template.enforce_execution_success
    ));
    lines.join("\n")
}

fn render_plugins_preflight_text(execution: &PluginsPreflightExecution) -> String {
    let mut lines = vec![format!(
        "plugins preflight profile={} query={} roots={} matched_plugins={} returned_plugins={} passed={} warned={} blocked={}",
        execution.profile,
        display_text_or_dash(Some(execution.query.as_str())),
        execution.scan_roots.join(","),
        execution.summary.matched_plugins,
        execution.summary.returned_plugins,
        execution.summary.passed_plugins,
        execution.summary.warned_plugins,
        execution.summary.blocked_plugins
    )];
    lines.push(format!(
        "policy source={} version={} checksum={} sha256={}",
        execution.summary.policy_source,
        execution.summary.policy_version.as_deref().unwrap_or("-"),
        execution.summary.policy_checksum,
        execution.summary.policy_sha256
    ));
    lines.push(format!(
        "bridge_support source={} sha256={}",
        display_text_or_dash(execution.bridge_support_source.as_deref()),
        display_text_or_dash(execution.bridge_support_sha256.as_deref())
    ));
    lines.push(format!(
        "bridge_support_delta source={} sha256={}",
        display_text_or_dash(execution.bridge_support_delta_source.as_deref()),
        display_text_or_dash(execution.bridge_support_delta_sha256.as_deref())
    ));
    lines.push(format!(
        "ecosystem source_kind={} dialect={} compatibility={} language={} bridge={}",
        format_rollup_map(&execution.summary.source_kind_distribution),
        format_rollup_map(&execution.summary.dialect_distribution),
        format_rollup_map(&execution.summary.compatibility_mode_distribution),
        format_rollup_map(&execution.summary.source_language_distribution),
        format_rollup_map(&execution.summary.bridge_kind_distribution)
    ));
    lines.push(format!(
        "diagnostics total={} blocking={} error={} warning={} info={}",
        execution.summary.total_diagnostics,
        execution.summary.blocking_diagnostics,
        execution.summary.error_diagnostics,
        execution.summary.warning_diagnostics,
        execution.summary.info_diagnostics
    ));
    lines.push(format!(
        "operator_actions total={} by_surface={} by_kind={} reload={} no_reload={}",
        execution.summary.operator_action_plan.len(),
        format_rollup_map(&execution.summary.operator_action_counts_by_surface),
        format_rollup_map(&execution.summary.operator_action_counts_by_kind),
        execution.summary.operator_actions_requiring_reload,
        execution.summary.operator_actions_without_reload
    ));
    lines.extend(render_bridge_profile_fit_lines(&execution.summary));
    for result in &execution.results {
        let plugin = &result.plugin;
        let action_kinds =
            format_preflight_result_operator_action_kinds(&result.recommended_actions);
        lines.push(format!(
            "- plugin={} provider={} verdict={} baseline={} activation_ready={} loaded={} actions={}",
            plugin.plugin_id,
            plugin.provider_id,
            result.verdict,
            result.baseline_verdict,
            result.activation_ready,
            plugin.loaded,
            action_kinds
        ));
    }
    lines.join("\n")
}

fn render_plugin_doctor_result_lines(result: &PluginPreflightResult) -> Vec<String> {
    let plugin = &result.plugin;
    let activation_status = inventory_result_status_label(plugin);
    let setup_surface = inventory_result_setup_surface_label(plugin);
    let source_language = plugin.source_language.as_deref().unwrap_or("-");
    let manifest_path = display_text_or_dash(plugin.package_manifest_path.as_deref());
    let setup_mode = display_text_or_dash(plugin.setup_mode.as_deref());
    let required_env_vars = format_csv_or_dash(&plugin.setup_required_env_vars);
    let required_config_keys = format_csv_or_dash(&plugin.setup_required_config_keys);
    let setup_remediation = display_text_or_dash(plugin.setup_remediation.as_deref());
    let runtime_health = plugin
        .runtime_health
        .as_ref()
        .map(|health| health.status.as_str());
    let attestation = plugin
        .activation_attestation
        .as_ref()
        .map(|value| value.integrity.as_str());
    let effective_flags = format_csv_or_dash(&result.effective_policy_flags);
    let remediation_classes = format_preflight_remediation_classes(&result.remediation_classes);
    let operator_action_kinds =
        format_preflight_result_operator_action_kinds(&result.recommended_actions);
    let blocking_diagnostics = format_csv_or_dash(&result.blocking_diagnostic_codes);
    let advisory_diagnostics = format_csv_or_dash(&result.advisory_diagnostic_codes);
    let recommended_actions =
        format_preflight_result_recommended_actions(&result.recommended_actions);

    let mut lines = vec![format!(
        "- plugin={} provider={} verdict={} activation_status={} loaded={} deferred={} bridge={} language={} setup_surface={}",
        plugin.plugin_id,
        plugin.provider_id,
        result.verdict,
        activation_status,
        plugin.loaded,
        plugin.deferred,
        plugin.bridge_kind,
        source_language,
        setup_surface
    )];
    lines.push(format!(
        "  manifest={} setup_mode={} required_env={} required_config={} setup_remediation={}",
        manifest_path, setup_mode, required_env_vars, required_config_keys, setup_remediation
    ));
    lines.push(format!(
        "  source={} activation_ready={} runtime_health={} attestation={} summary={}",
        plugin.source_path,
        result.activation_ready,
        display_text_or_dash(runtime_health),
        display_text_or_dash(attestation),
        display_text_or_dash(plugin.summary.as_deref())
    ));
    lines.push(format!(
        "  policy_summary={} effective_flags={} remediation_classes={} operator_actions={}",
        result.policy_summary, effective_flags, remediation_classes, operator_action_kinds
    ));
    lines.push(format!(
        "  blocking_diagnostics={} advisory_diagnostics={}",
        blocking_diagnostics, advisory_diagnostics
    ));
    if let Some(reason) = plugin.activation_reason.as_deref() {
        lines.push(format!("  activation_reason={reason}"));
    }
    if recommended_actions != "-" {
        lines.push(format!("  recommended_actions={recommended_actions}"));
    }
    lines
}

fn render_plugins_actions_text(execution: &PluginsActionsExecution) -> String {
    let mut lines = vec![format!(
        "plugins actions profile={} query={} roots={} total_actions={} matched_actions={}",
        execution.profile,
        display_text_or_dash(Some(execution.query.as_str())),
        execution.scan_roots.join(","),
        execution.total_actions,
        execution.matched_actions
    )];
    lines.push(format!(
        "policy source={} version={} checksum={} sha256={}",
        execution.summary.policy_source,
        execution.summary.policy_version.as_deref().unwrap_or("-"),
        execution.summary.policy_checksum,
        execution.summary.policy_sha256
    ));
    lines.push(format!(
        "bridge_support source={} sha256={}",
        display_text_or_dash(execution.bridge_support_source.as_deref()),
        display_text_or_dash(execution.bridge_support_sha256.as_deref())
    ));
    lines.push(format!(
        "bridge_support_delta source={} sha256={}",
        display_text_or_dash(execution.bridge_support_delta_source.as_deref()),
        display_text_or_dash(execution.bridge_support_delta_sha256.as_deref())
    ));
    lines.push(format!(
        "ecosystem source_kind={} dialect={} compatibility={} language={} bridge={}",
        format_rollup_map(&execution.summary.source_kind_distribution),
        format_rollup_map(&execution.summary.dialect_distribution),
        format_rollup_map(&execution.summary.compatibility_mode_distribution),
        format_rollup_map(&execution.summary.source_language_distribution),
        format_rollup_map(&execution.summary.bridge_kind_distribution)
    ));
    lines.push(format!(
        "filters surface={} kind={} requires_reload={}",
        format_csv_or_dash(&execution.filters.surface),
        format_csv_or_dash(&execution.filters.kind),
        execution
            .filters
            .requires_reload
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned())
    ));
    lines.push(format!(
        "filtered_counts by_surface={} by_kind={} reload={} no_reload={}",
        format_rollup_map(&execution.filtered_action_counts_by_surface),
        format_rollup_map(&execution.filtered_action_counts_by_kind),
        execution.filtered_actions_requiring_reload,
        execution.filtered_actions_without_reload
    ));
    lines.extend(render_bridge_profile_fit_lines(&execution.summary));
    for item in &execution.actions {
        lines.extend(render_action_item_lines(item));
    }
    lines.join("\n")
}

fn render_action_item_lines(item: &PluginsActionPlanItemView) -> Vec<String> {
    let remediation_summary = item
        .supporting_remediations
        .iter()
        .map(|support| {
            let mut parts = vec![support.remediation_class.clone()];
            if let Some(code) = support.diagnostic_code.as_deref() {
                parts.push(format!("code={code}"));
            }
            if let Some(field_path) = support.field_path.as_deref() {
                parts.push(format!("field={field_path}"));
            }
            if support.blocking {
                parts.push("blocking=true".to_owned());
            }
            parts.join("|")
        })
        .collect::<Vec<_>>()
        .join("; ");
    vec![
        format!(
            "- action_id={} surface={} kind={} plugin={} provider={} reload={} follow_up={} supports={} blocked={} warned={} passed={}",
            item.action.action_id,
            item.action.surface,
            item.action.kind,
            item.action.target_plugin_id,
            display_text_or_dash(item.action.target_provider_id.as_deref()),
            item.action.requires_reload,
            display_text_or_dash(item.action.follow_up_profile.as_deref()),
            item.supporting_remediations.len(),
            item.blocked_results,
            item.warned_results,
            item.passed_results
        ),
        format!(
            "  source={} manifest={} supporting_results={} remediations={}",
            item.action.target_source_path,
            display_text_or_dash(item.action.target_manifest_path.as_deref()),
            item.supporting_results,
            if remediation_summary.is_empty() {
                "-".to_owned()
            } else {
                remediation_summary
            }
        ),
    ]
}

fn render_bridge_profile_fit_lines(summary: &PluginsPreflightSummaryView) -> Vec<String> {
    let mut lines = vec![format!(
        "bridge_profiles active={} recommended={} recommended_source={} active_matches={} active_support_fits_all={}",
        display_text_or_dash(summary.active_bridge_profile.as_deref()),
        display_text_or_dash(summary.recommended_bridge_profile.as_deref()),
        display_text_or_dash(summary.recommended_bridge_profile_source.as_deref()),
        summary
            .active_bridge_profile_matches_recommended
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned()),
        summary
            .active_bridge_support_fits_all_plugins
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned())
    )];

    for fit in &summary.bridge_profile_fits {
        lines.push(format!(
            "bridge_profile_fit profile={} version={} fits_all={} supported={} blocked={} reasons={} sample_blocked_plugins={}",
            fit.profile_id,
            display_text_or_dash(fit.policy_version.as_deref()),
            fit.fits_all_plugins,
            fit.supported_plugins,
            fit.blocked_plugins,
            format_rollup_map(&fit.blocking_reasons),
            format_csv_or_dash(&fit.sample_blocked_plugins)
        ));
    }

    if let Some(recommendation) = summary.bridge_profile_recommendation.as_ref() {
        lines.push(format!(
            "bridge_profile_recommendation kind={} target={} source={} version={} summary={}",
            recommendation.kind,
            recommendation.target_profile_id,
            recommendation.target_profile_source,
            display_text_or_dash(recommendation.target_policy_version.as_deref()),
            recommendation.summary
        ));
        if let Some(delta) = recommendation.delta.as_ref() {
            lines.push(format!(
                "bridge_profile_delta bridges={} compatibility={} adapter_families={} shims={} shim_profiles={} unresolved={}",
                format_csv_or_dash(&delta.supported_bridges),
                format_csv_or_dash(&delta.supported_compatibility_modes),
                format_csv_or_dash(&delta.supported_adapter_families),
                format_csv_or_dash(&delta.supported_compatibility_shims),
                format_shim_profile_deltas(&delta.shim_profile_additions),
                format_csv_or_dash(&delta.unresolved_blocking_reasons)
            ));
        }
    }

    lines
}

fn format_preflight_remediation_classes(
    values: &[crate::PluginPreflightRemediationClass],
) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }
    let mut classes = values
        .iter()
        .map(|value| value.as_str().to_owned())
        .collect::<Vec<_>>();
    classes.sort();
    classes.dedup();
    classes.join(",")
}

fn format_preflight_result_operator_action_kinds(
    values: &[crate::PluginPreflightRecommendedAction],
) -> String {
    let kinds = values
        .iter()
        .filter_map(|value| value.operator_action.as_ref())
        .map(|value| value.kind.as_str().to_owned())
        .collect::<BTreeSet<_>>();
    if kinds.is_empty() {
        "-".to_owned()
    } else {
        kinds.into_iter().collect::<Vec<_>>().join(",")
    }
}

fn format_preflight_result_recommended_actions(
    values: &[crate::PluginPreflightRecommendedAction],
) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }
    values
        .iter()
        .map(|value| {
            let mut parts = vec![
                value.remediation_class.as_str().to_owned(),
                value.summary.clone(),
            ];
            if let Some(action) = value.operator_action.as_ref() {
                parts.push(format!("action={}", action.kind.as_str()));
            }
            parts.join("|")
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn plugin_inventory_status_is_blocked(status: &str) -> bool {
    status != "ready" && status != "setup_incomplete"
}

fn increment_rollup_count(values: &mut BTreeMap<String, usize>, key: &str) {
    let entry = values.entry(key.to_owned()).or_default();
    *entry = entry.saturating_add(1);
}

fn count_preflight_result_operator_actions(result: &PluginPreflightResult) -> usize {
    result
        .recommended_actions
        .iter()
        .filter(|action| action.operator_action.is_some())
        .count()
}

fn format_shim_profile_deltas(values: &[PluginsBridgeShimProfileDeltaView]) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }
    values
        .iter()
        .map(|value| {
            format!(
                "{}:{}:dialects={}|bridges={}|adapter_families={}|languages={}",
                value.shim_id,
                value.shim_family,
                format_csv_or_dash(&value.supported_dialects),
                format_csv_or_dash(&value.supported_bridges),
                format_csv_or_dash(&value.supported_adapter_families),
                format_csv_or_dash(&value.supported_source_languages)
            )
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn format_bridge_shim_profile_delta_artifact(
    values: &[crate::PluginPreflightBridgeShimProfileDelta],
) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }
    values
        .iter()
        .map(|value| {
            format!(
                "{}:{}:dialects={}|bridges={}|adapter_families={}|languages={}",
                value.shim_id,
                value.shim_family,
                format_csv_or_dash(&value.supported_dialects),
                format_csv_or_dash(&value.supported_bridges),
                format_csv_or_dash(&value.supported_adapter_families),
                format_csv_or_dash(&value.supported_source_languages)
            )
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn inventory_result_status_label(result: &PluginInventoryResult) -> &str {
    let activation_status = result.activation_status.as_deref();
    if activation_status.is_some_and(|status| !status.is_empty()) {
        activation_status.unwrap_or("unknown")
    } else if result.deferred {
        "deferred"
    } else {
        "unknown"
    }
}

fn inventory_result_setup_surface_label(result: &PluginInventoryResult) -> &str {
    let setup_surface = result.setup_surface.as_deref();
    if setup_surface.is_some_and(|value| !value.is_empty()) {
        setup_surface.unwrap_or("none")
    } else if result
        .setup_mode
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        "unspecified"
    } else {
        "none"
    }
}

fn display_text_or_dash(value: Option<&str>) -> &str {
    match value {
        Some(value) if !value.is_empty() => value,
        _ => "-",
    }
}

fn format_csv_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_owned()
    } else {
        values.join(",")
    }
}

fn format_rollup_map(values: &BTreeMap<String, usize>) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }
    values
        .iter()
        .map(|(key, value)| format!("{key}:{value}"))
        .collect::<Vec<_>>()
        .join(",")
}
