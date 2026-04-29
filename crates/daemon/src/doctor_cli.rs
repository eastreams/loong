use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use clap::Subcommand;
use kernel::{probe_jsonl_audit_journal_runtime_ready, verify_jsonl_audit_journal};
use loong_app as mvp;
use loong_contracts::SecretRef;
use loong_spec::CliResult;
use serde_json::{Value, json};

use crate::plugin_bridge_account_summary::plugin_bridge_account_summary;
use crate::provider_credential_policy;
use crate::provider_model_probe_policy;

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum DoctorCommands {
    /// Report effective security exposure and config hygiene posture
    Security,
}

#[derive(Debug, Clone)]
pub struct DoctorCommandOptions {
    pub config: Option<String>,
    pub fix: bool,
    pub json: bool,
    pub skip_model_probe: bool,
    pub command: Option<DoctorCommands>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoctorCheckLevel {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone)]
pub struct DoctorCheck {
    pub name: String,
    pub level: DoctorCheckLevel,
    pub detail: String,
}

#[derive(Debug, Clone, Copy)]
struct DoctorSummary {
    pass: usize,
    warn: usize,
    fail: usize,
}

pub async fn run_doctor_cli(options: DoctorCommandOptions) -> CliResult<()> {
    if let Some(command) = options.command.clone() {
        return match command {
            DoctorCommands::Security => {
                crate::doctor_security_cli::run_doctor_security_cli(
                    crate::doctor_security_cli::DoctorSecurityCommandOptions {
                        config: options.config,
                        json: options.json,
                        fix: options.fix,
                        skip_model_probe: options.skip_model_probe,
                    },
                )
                .await
            }
        };
    }

    let (config_path, mut config) = mvp::config::load(options.config.as_deref())?;
    let mut checks = Vec::new();
    let mut fixes = Vec::new();
    let mut config_mutated = false;

    config_mutated |= maybe_apply_provider_env_fix(&mut config, options.fix, &mut fixes);
    config_mutated |= maybe_apply_channel_env_fix(&mut config, options.fix, &mut fixes);

    let has_provider_credentials = mvp::provider::provider_auth_ready(&config).await;
    let provider_requires_explicit_auth = config.provider.requires_explicit_auth_configuration();
    checks.push(provider_credentials_doctor_check(
        &config,
        has_provider_credentials,
    ));

    checks.push(provider_transport_doctor_check(&config.provider));
    if config.tools.web_search.enabled {
        checks.push(web_search_provider_doctor_check(&config));
    }

    if options.skip_model_probe {
        checks.push(DoctorCheck {
            name: "provider model probe".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "skipped by --skip-model-probe".to_owned(),
        });
    } else if !has_provider_credentials && provider_requires_explicit_auth {
        checks.push(DoctorCheck {
            name: "provider model probe".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "skipped because credentials are missing".to_owned(),
        });
    } else {
        match mvp::provider::fetch_available_models(&config).await {
            Ok(models) => checks.push(DoctorCheck {
                name: "provider model probe".to_owned(),
                level: DoctorCheckLevel::Pass,
                detail: format!("{} model(s) available", models.len()),
            }),
            Err(error) => {
                let probe_failure = provider_model_probe_policy::provider_model_probe_failure(
                    &config,
                    error.as_str(),
                );
                let should_collect_route_probe = matches!(
                    &probe_failure.kind,
                    provider_model_probe_policy::ProviderModelProbeFailureKind::TransportFailure
                );
                let check = doctor_check_from_provider_model_probe_failure(probe_failure);
                checks.push(check);
                if should_collect_route_probe
                    && let Some(route_probe) =
                        crate::provider_route_diagnostics::collect_provider_route_probe(
                            &config.provider,
                        )
                        .await
                {
                    checks.push(provider_route_probe_doctor_check(&route_probe));
                }
            }
        }
    }

    let sqlite_path = config.memory.resolved_sqlite_path();
    let sqlite_parent = sqlite_path.parent().unwrap_or(Path::new("."));
    checks.push(check_directory_ready(
        "memory path",
        sqlite_parent,
        options.fix,
        &mut fixes,
        "create memory directory",
    ));
    checks.push(audit_retention_doctor_check(&config.audit));
    checks.push(audit_integrity_doctor_check(&config.audit));
    if matches!(
        config.audit.mode,
        mvp::config::AuditMode::Jsonl | mvp::config::AuditMode::Fanout
    ) {
        let audit_path = config.audit.resolved_path();
        let audit_parent = audit_path.parent().unwrap_or(Path::new("."));
        checks.push(check_audit_journal_directory(
            audit_parent,
            options.fix,
            &mut fixes,
        ));
    }

    let mut file_root_resolution = config.tools.file_root_resolution();
    let uses_file_root_fallback = file_root_resolution.uses_current_working_directory_fallback();
    if uses_file_root_fallback {
        checks.push(DoctorCheck {
            name: "tool file root policy".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "tools.file_root is empty (falls back to current working directory)".to_owned(),
        });
        if options.fix {
            let suggested_root = mvp::config::default_loong_home()
                .join("workspace")
                .display()
                .to_string();
            config.tools.file_root = Some(suggested_root.clone());
            file_root_resolution = config.tools.file_root_resolution();
            config_mutated = true;
            fixes.push(format!("set tools.file_root={suggested_root}"));
        }
    } else {
        checks.push(DoctorCheck {
            name: "tool file root policy".to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: "tools.file_root is configured".to_owned(),
        });
    }
    let effective_tool_root = file_root_resolution.path().clone();
    checks.push(check_directory_ready(
        "tool file root",
        &effective_tool_root,
        options.fix,
        &mut fixes,
        "create tool file root",
    ));
    checks.extend(collect_browser_companion_doctor_checks(&config).await);
    let runtime_plugins_state = crate::collect_runtime_snapshot_runtime_plugins_state(&config);
    checks.extend(collect_runtime_plugins_doctor_checks_from_state(
        &runtime_plugins_state,
    ));
    let runtime_plugin_inventory =
        crate::plugins_cli::runtime_plugin_inventory_read_model(&config).await;

    checks.extend(check_feishu_integration(&config, options.fix, &mut fixes));
    let channel_inventory = mvp::channel::channel_inventory(&config);
    let channel_surface_checks = collect_channel_surface_checks(&channel_inventory);
    checks.extend(channel_surface_checks);
    let path_env = env::var_os("PATH");
    checks.extend(crate::browser_preview::browser_preview_check(
        &config,
        path_env.as_deref(),
    ));

    if options.fix && config_mutated {
        let path = config_path
            .to_str()
            .ok_or_else(|| format!("config path is not valid UTF-8: {}", config_path.display()))?;
        mvp::config::write(Some(path), &config, true)?;
    }

    let summary = summarize_checks(&checks);
    let mut next_steps = build_doctor_next_steps_with_channel_surfaces_and_path_env(
        &checks,
        &config_path,
        &config,
        &channel_inventory.channel_surfaces,
        options.fix,
        path_env.as_deref(),
    );
    next_steps.extend(
        crate::runtime_plugin_discovery::build_runtime_plugin_discovery_next_steps(
            runtime_plugin_inventory.discovery_guidance.as_ref(),
        ),
    );
    if options.json {
        let checks = doctor_checks_json_payload(&checks, &channel_inventory.channel_surfaces);
        let payload = json!({
            "ok": summary.fail == 0,
            "config": config_path.display().to_string(),
            "summary": {
                "ok": summary.pass,
                "warn": summary.warn,
                "fail": summary.fail
            },
            "checks": checks,
            "runtime_plugins": doctor_runtime_plugins_json_payload(&runtime_plugins_state),
            "runtime_plugin_inventory": serde_json::to_value(&runtime_plugin_inventory).unwrap_or(Value::Null),
            "fix_requested": options.fix,
            "applied_fixes": fixes,
            "next_steps": next_steps,
        });
        let encoded = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize doctor output failed: {error}"))?;
        println!("{encoded}");
        return Ok(());
    }

    println!(
        "{}",
        render_doctor_text(
            &checks,
            summary,
            &fixes,
            &next_steps,
            config_path.as_path(),
            options.fix,
        )
    );

    if summary.fail > 0 {
        return Err("doctor detected failing checks".to_owned());
    }
    Ok(())
}

fn check_directory_ready(
    name: &'static str,
    directory: &Path,
    fix: bool,
    fixes: &mut Vec<String>,
    fix_label: &'static str,
) -> DoctorCheck {
    if directory.exists() {
        if directory.is_dir() {
            return DoctorCheck {
                name: name.to_owned(),
                level: DoctorCheckLevel::Pass,
                detail: directory.display().to_string(),
            };
        }
        return DoctorCheck {
            name: name.to_owned(),
            level: DoctorCheckLevel::Fail,
            detail: format!("{} exists but is not a directory", directory.display()),
        };
    }

    if !fix {
        return DoctorCheck {
            name: name.to_owned(),
            level: DoctorCheckLevel::Fail,
            detail: format!(
                "{} is missing (rerun with --fix to create it)",
                directory.display()
            ),
        };
    }

    match fs::create_dir_all(directory) {
        Ok(()) => {
            fixes.push(format!("{fix_label}: {}", directory.display()));
            DoctorCheck {
                name: name.to_owned(),
                level: DoctorCheckLevel::Pass,
                detail: format!("created {}", directory.display()),
            }
        }
        Err(error) => DoctorCheck {
            name: name.to_owned(),
            level: DoctorCheckLevel::Fail,
            detail: format!("failed to create {}: {error}", directory.display()),
        },
    }
}

#[cfg(test)]
fn check_channel_surfaces(config: &mvp::config::LoongConfig) -> Vec<DoctorCheck> {
    let inventory = mvp::channel::channel_inventory(config);
    collect_channel_surface_checks(&inventory)
}

fn collect_channel_surface_checks(inventory: &mvp::channel::ChannelInventory) -> Vec<DoctorCheck> {
    let snapshot_checks = build_channel_surface_checks(&inventory.channels);
    let discovery_checks =
        build_channel_surface_managed_plugin_discovery_checks(&inventory.channel_surfaces);
    let mut checks = Vec::new();

    checks.extend(snapshot_checks);
    checks.extend(discovery_checks);

    checks
}

#[cfg(test)]
fn collect_runtime_plugins_doctor_checks(config: &mvp::config::LoongConfig) -> Vec<DoctorCheck> {
    let state = crate::collect_runtime_snapshot_runtime_plugins_state(config);
    collect_runtime_plugins_doctor_checks_from_state(&state)
}

fn collect_runtime_plugins_doctor_checks_from_state(
    state: &crate::RuntimeSnapshotRuntimePluginsState,
) -> Vec<DoctorCheck> {
    let runtime_level = if !state.enabled || state.scanned_root_count == 0 {
        DoctorCheckLevel::Warn
    } else {
        DoctorCheckLevel::Pass
    };
    let mut checks = vec![DoctorCheck {
        name: "runtime plugins runtime".to_owned(),
        level: runtime_level,
        detail: format!(
            "enabled={} supported_bridges={} supported_adapter_families={} roots={} scanned_roots={}",
            state.enabled,
            doctor_render_string_list(&state.supported_bridges),
            doctor_render_string_list(&state.supported_adapter_families),
            doctor_render_string_list(&state.roots),
            state.scanned_root_count,
        ),
    }];

    if !state.enabled {
        return checks;
    }

    let inventory_level = match state.inventory_status {
        crate::RuntimeSnapshotInventoryStatus::Error => DoctorCheckLevel::Fail,
        crate::RuntimeSnapshotInventoryStatus::Disabled => DoctorCheckLevel::Warn,
        crate::RuntimeSnapshotInventoryStatus::Ok => {
            let zero_roots_scanned = state.scanned_root_count == 0;
            let has_setup_warnings = state.setup_incomplete_plugin_count > 0;
            let has_blocked_plugins = state.blocked_plugin_count > 0;
            if zero_roots_scanned || has_setup_warnings || has_blocked_plugins {
                DoctorCheckLevel::Warn
            } else {
                DoctorCheckLevel::Pass
            }
        }
    };
    let inventory_detail = if let Some(error) = state.inventory_error.as_deref() {
        let rendered_error = crate::render_line_safe_text_value(error);

        format!(
            "inventory_status={} error={rendered_error}",
            state.inventory_status.as_str(),
        )
    } else {
        let blocked_ids = state
            .plugins
            .iter()
            .filter(|plugin| plugin.status.starts_with("blocked_"))
            .map(|plugin| plugin.plugin_id.as_str())
            .collect::<Vec<_>>();
        let setup_incomplete_ids = state
            .plugins
            .iter()
            .filter(|plugin| plugin.status == "setup_incomplete")
            .map(|plugin| plugin.plugin_id.as_str())
            .collect::<Vec<_>>();
        format!(
            "inventory_status={} readiness_evaluation={} discovered={} translated={} ready={} setup_incomplete={} blocked={} blocked_ids={} setup_incomplete_ids={}",
            state.inventory_status.as_str(),
            state.readiness_evaluation,
            state.discovered_plugin_count,
            state.translated_plugin_count,
            state.ready_plugin_count,
            state.setup_incomplete_plugin_count,
            state.blocked_plugin_count,
            doctor_render_string_list(
                &blocked_ids
                    .iter()
                    .map(|id| (*id).to_owned())
                    .collect::<Vec<_>>(),
            ),
            doctor_render_string_list(
                &setup_incomplete_ids
                    .iter()
                    .map(|id| (*id).to_owned())
                    .collect::<Vec<_>>(),
            ),
        )
    };
    checks.push(DoctorCheck {
        name: "runtime plugins inventory".to_owned(),
        level: inventory_level,
        detail: inventory_detail,
    });

    checks
}

fn doctor_runtime_plugins_json_payload(
    state: &crate::RuntimeSnapshotRuntimePluginsState,
) -> serde_json::Value {
    crate::runtime_snapshot_runtime_plugins_json(state)
}

fn audit_retention_doctor_check(audit: &mvp::config::AuditConfig) -> DoctorCheck {
    let path = audit.resolved_path();
    match audit.mode {
        mvp::config::AuditMode::InMemory => DoctorCheck {
            name: "audit retention".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "audit.mode=in_memory; security-critical audit evidence is lost on restart"
                .to_owned(),
        },
        mvp::config::AuditMode::Jsonl => durable_audit_retention_doctor_check(&path, "jsonl", None),
        mvp::config::AuditMode::Fanout => durable_audit_retention_doctor_check(
            &path,
            "fanout",
            if audit.retain_in_memory {
                Some("durable journal + live in-memory snapshot")
            } else {
                Some("durable journal only")
            },
        ),
    }
}

fn durable_audit_retention_doctor_check(
    path: &Path,
    mode: &'static str,
    suffix: Option<&'static str>,
) -> DoctorCheck {
    if let Some(issue) = durable_audit_target_issue(path) {
        return DoctorCheck {
            name: "audit retention".to_owned(),
            level: DoctorCheckLevel::Fail,
            detail: format!("audit.mode={mode} -> {issue}"),
        };
    }

    let mut detail = format!("audit.mode={mode} -> {}", path.display());
    if let Some(suffix) = suffix {
        detail.push_str(" (");
        detail.push_str(suffix);
        detail.push(')');
    }

    DoctorCheck {
        name: "audit retention".to_owned(),
        level: DoctorCheckLevel::Pass,
        detail,
    }
}

pub(crate) fn durable_audit_target_issue(path: &Path) -> Option<String> {
    durable_audit_target_issue_with_probe(path, durable_audit_runtime_probe)
}

fn durable_audit_target_issue_with_probe<F>(path: &Path, runtime_probe: F) -> Option<String>
where
    F: Fn(&Path) -> Result<(), String>,
{
    if let Some(issue) = durable_audit_metadata_issue(path) {
        return Some(issue);
    }

    runtime_probe(path).err()
}

fn durable_audit_metadata_issue(path: &Path) -> Option<String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            return Some(format!(
                "failed to inspect audit journal {}: {error}",
                path.display()
            ));
        }
    };

    if !metadata.is_file() {
        return Some(format!(
            "{} exists but is not a regular file",
            path.display()
        ));
    }

    if metadata.permissions().readonly() {
        return Some(format!("{} exists but is not writable", path.display()));
    }

    None
}

fn durable_audit_runtime_probe(path: &Path) -> Result<(), String> {
    let path_entry_existed = fs::symlink_metadata(path).is_ok();
    let created_directories = durable_audit_missing_parent_dirs(path);
    let probe_result = probe_jsonl_audit_journal_runtime_ready(path).map_err(|error| {
        format!(
            "runtime open + lock probe failed for {}: {error}",
            path.display()
        )
    });
    let cleanup_result =
        durable_audit_runtime_probe_cleanup(path, path_entry_existed, &created_directories);

    match (probe_result, cleanup_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn audit_integrity_doctor_check(audit: &mvp::config::AuditConfig) -> DoctorCheck {
    if matches!(audit.mode, mvp::config::AuditMode::InMemory) {
        return DoctorCheck {
            name: "audit integrity".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "audit integrity verification is unavailable while audit.mode=in_memory"
                .to_owned(),
        };
    }

    let journal_path = audit.resolved_path();
    if !journal_path.exists() {
        return DoctorCheck {
            name: "audit integrity".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: format!(
                "audit journal {} has not been created yet, so integrity verification is not available until the first durable write",
                journal_path.display()
            ),
        };
    }

    match verify_jsonl_audit_journal(&journal_path) {
        Ok(report) if report.valid => DoctorCheck {
            name: "audit integrity".to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: format!(
                "verified {} of {} audit events (last_entry_hash={})",
                report.verified_events,
                report.total_events,
                report.last_entry_hash.as_deref().unwrap_or("-")
            ),
        },
        Ok(report) => DoctorCheck {
            name: "audit integrity".to_owned(),
            level: DoctorCheckLevel::Fail,
            detail: format!(
                "audit journal integrity failed at line {} ({})",
                report
                    .first_invalid_line
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                report.reason.as_deref().unwrap_or("unknown reason")
            ),
        },
        Err(error) => DoctorCheck {
            name: "audit integrity".to_owned(),
            level: DoctorCheckLevel::Fail,
            detail: format!("audit integrity verification failed: {error}"),
        },
    }
}

fn durable_audit_missing_parent_dirs(path: &Path) -> Vec<PathBuf> {
    let mut missing = Vec::new();
    let Some(mut current) = path.parent() else {
        return missing;
    };

    while !current.as_os_str().is_empty() && !current.exists() {
        missing.push(current.to_path_buf());
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent;
    }

    missing.reverse();
    missing
}

fn durable_audit_runtime_probe_cleanup(
    path: &Path,
    path_entry_existed: bool,
    created_directories: &[PathBuf],
) -> Result<(), String> {
    if !path_entry_existed {
        match fs::metadata(path) {
            Ok(metadata) if metadata.len() == 0 => {
                fs::remove_file(path).map_err(|error| {
                    format!(
                        "runtime open + lock probe cleanup failed for {}: {error}",
                        path.display()
                    )
                })?;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "runtime open + lock probe cleanup failed for {}: {error}",
                    path.display()
                ));
            }
        }
    }

    for directory in created_directories.iter().rev() {
        match fs::remove_dir(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
            Err(error) => {
                return Err(format!(
                    "runtime open + lock probe cleanup failed for {}: failed to remove {}: {error}",
                    path.display(),
                    directory.display()
                ));
            }
        }
    }

    Ok(())
}

fn check_audit_journal_directory(
    directory: &Path,
    fix: bool,
    fixes: &mut Vec<String>,
) -> DoctorCheck {
    if directory.as_os_str().is_empty() {
        return DoctorCheck {
            name: "audit journal directory".to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: "current working directory (journal file is created on first audit write)"
                .to_owned(),
        };
    }

    if directory.exists() {
        if directory.is_dir() {
            return DoctorCheck {
                name: "audit journal directory".to_owned(),
                level: DoctorCheckLevel::Pass,
                detail: directory.display().to_string(),
            };
        }
        return DoctorCheck {
            name: "audit journal directory".to_owned(),
            level: DoctorCheckLevel::Fail,
            detail: format!("{} exists but is not a directory", directory.display()),
        };
    }

    if !fix {
        return DoctorCheck {
            name: "audit journal directory".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: format!(
                "{} is missing (rerun with --fix to create it, or let runtime create it on first audit write)",
                directory.display()
            ),
        };
    }

    match fs::create_dir_all(directory) {
        Ok(()) => {
            fixes.push(format!(
                "create audit journal directory: {}",
                directory.display()
            ));
            DoctorCheck {
                name: "audit journal directory".to_owned(),
                level: DoctorCheckLevel::Pass,
                detail: format!("created {}", directory.display()),
            }
        }
        Err(error) => DoctorCheck {
            name: "audit journal directory".to_owned(),
            level: DoctorCheckLevel::Fail,
            detail: format!("failed to create {}: {error}", directory.display()),
        },
    }
}

pub fn check_feishu_integration(
    config: &mvp::config::LoongConfig,
    fix: bool,
    fixes: &mut Vec<String>,
) -> Vec<DoctorCheck> {
    if !feishu_integration_requested(&config.feishu) {
        return Vec::new();
    }

    let mut checks = Vec::new();
    let sqlite_path = config.feishu_integration.resolved_sqlite_path();
    let sqlite_parent = sqlite_path.parent().unwrap_or(Path::new("."));
    checks.push(check_directory_ready(
        "feishu integration store",
        sqlite_parent,
        fix,
        fixes,
        "create feishu integration store directory",
    ));

    let store = mvp::channel::feishu::api::FeishuTokenStore::new(sqlite_path);
    let configured_ids = config.feishu.configured_account_ids();
    let scoped = configured_ids.len() > 1;

    for configured_id in configured_ids {
        let resolved = match config.feishu.resolve_account(Some(configured_id.as_str())) {
            Ok(resolved) => resolved,
            Err(error) => {
                checks.push(DoctorCheck {
                    name: scoped_feishu_check_name(
                        "feishu integration account",
                        &configured_id,
                        scoped,
                    ),
                    level: DoctorCheckLevel::Fail,
                    detail: error,
                });
                continue;
            }
        };

        let credentials_name = scoped_feishu_check_name(
            "feishu integration credentials",
            &resolved.configured_account_id,
            scoped,
        );
        let has_app_id = resolved
            .app_id()
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some();
        let has_app_secret = resolved
            .app_secret()
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some();
        checks.push(DoctorCheck {
            name: credentials_name,
            level: if has_app_id && has_app_secret {
                DoctorCheckLevel::Pass
            } else {
                DoctorCheckLevel::Fail
            },
            detail: if has_app_id && has_app_secret {
                format!(
                    "configured_account={} account={} app credentials are available",
                    resolved.configured_account_id, resolved.account.id
                )
            } else {
                format!(
                    "configured_account={} account={} missing app credentials (need feishu.app_id/app_secret or account overrides)",
                    resolved.configured_account_id, resolved.account.id
                )
            },
        });

        let grant_name =
            scoped_feishu_check_name("feishu user grant", &resolved.configured_account_id, scoped);
        let inventory = match mvp::channel::feishu::api::inspect_grants_for_account(
            &store,
            resolved.account.id.as_str(),
        ) {
            Ok(inventory) => inventory,
            Err(error) => {
                checks.push(DoctorCheck {
                    name: grant_name,
                    level: DoctorCheckLevel::Fail,
                    detail: error,
                });
                continue;
            }
        };

        if inventory.grants.is_empty() {
            checks.push(DoctorCheck {
                name: grant_name,
                level: DoctorCheckLevel::Warn,
                detail: format!(
                    "configured_account={} account={} missing stored user grant; run `{}`",
                    resolved.configured_account_id,
                    resolved.account.id,
                    crate::feishu_support::feishu_auth_start_command_hint(
                        resolved.configured_account_id.as_str(),
                        false,
                        false,
                    )
                ),
            });
            continue;
        }

        let now_s = chrono::Utc::now().timestamp();
        let required_scopes = crate::feishu_support::resolve_required_feishu_scopes(
            &config.feishu_integration,
            &[],
            &[],
            false,
        );
        let Some(latest) = inventory.grants.first() else {
            continue;
        };
        let effective_grant = inventory.effective_grant();
        let effective_status = mvp::channel::feishu::api::auth::summarize_grant_status(
            effective_grant,
            now_s,
            &required_scopes,
        );

        checks.push(DoctorCheck {
            name: grant_name,
            level: DoctorCheckLevel::Pass,
            detail: format!(
                "configured_account={} account={} grants={} latest_open_id={} selected_open_id={} effective_open_id={}",
                resolved.configured_account_id,
                resolved.account.id,
                inventory.grants.len(),
                latest.principal.open_id,
                inventory.selected_open_id.as_deref().unwrap_or("-"),
                inventory.effective_open_id.as_deref().unwrap_or("-"),
            ),
        });
        checks.push(DoctorCheck {
            name: scoped_feishu_check_name(
                "feishu selected grant",
                &resolved.configured_account_id,
                scoped,
            ),
            level: if inventory.selected_open_id.is_some() {
                DoctorCheckLevel::Pass
            } else if inventory.stale_selected_open_id.is_some() || inventory.selection_required() {
                DoctorCheckLevel::Warn
            } else {
                DoctorCheckLevel::Pass
            },
            detail: if let Some(selected_open_id) = inventory.selected_open_id.as_deref() {
                if let Some(selected_grant) = inventory
                    .grants
                    .iter()
                    .find(|grant| grant.principal.open_id == selected_open_id)
                {
                    format!(
                        "configured_account={} account={} selected_open_id={} selected_name={}",
                        resolved.configured_account_id,
                        resolved.account.id,
                        selected_grant.principal.open_id,
                        selected_grant.principal.name.as_deref().unwrap_or("-")
                    )
                } else {
                    format!(
                        "configured_account={} account={} stale selected_open_id={} (grant not found); rerun `{}`",
                        resolved.configured_account_id,
                        resolved.account.id,
                        selected_open_id,
                        crate::feishu_support::feishu_auth_select_command_hint(
                            resolved.configured_account_id.as_str(),
                        )
                    )
                }
            } else if let Some(selected_open_id) = inventory
                .stale_selected_open_id
                .as_deref()
                .filter(|_| inventory.selection_required())
            {
                format!(
                    "configured_account={} account={} stale selected_open_id={} (grant not found); rerun `{}`",
                    resolved.configured_account_id,
                    resolved.account.id,
                    selected_open_id,
                    crate::feishu_support::feishu_auth_select_command_hint(
                        resolved.configured_account_id.as_str(),
                    )
                )
            } else if let Some(selected_open_id) = inventory.stale_selected_open_id.as_deref() {
                format!(
                    "configured_account={} account={} stale selected_open_id={} was cleared; single stored grant open_id={} now routes implicitly",
                    resolved.configured_account_id,
                    resolved.account.id,
                    selected_open_id,
                    latest.principal.open_id
                )
            } else if inventory.selection_required() {
                format!(
                    "configured_account={} account={} multiple stored grants without selected default; run `{}`",
                    resolved.configured_account_id,
                    resolved.account.id
                    ,
                    crate::feishu_support::feishu_auth_select_command_hint(
                        resolved.configured_account_id.as_str(),
                    )
                )
            } else {
                format!(
                    "configured_account={} account={} single stored grant open_id={} explicit selection not required",
                    resolved.configured_account_id,
                    resolved.account.id,
                    latest.principal.open_id
                )
            },
        });
        checks.push(DoctorCheck {
            name: scoped_feishu_check_name(
                "feishu token freshness",
                &resolved.configured_account_id,
                scoped,
            ),
            level: if effective_grant.is_none() {
                DoctorCheckLevel::Warn
            } else if effective_status.refresh_token_expired {
                DoctorCheckLevel::Fail
            } else if effective_status.access_token_expired {
                DoctorCheckLevel::Warn
            } else {
                DoctorCheckLevel::Pass
            },
            detail: if let Some(grant) = effective_grant {
                format!(
                    "configured_account={} account={} effective_open_id={} access_expired={} refresh_expired={}",
                    resolved.configured_account_id,
                    resolved.account.id,
                    grant.principal.open_id,
                    effective_status.access_token_expired,
                    effective_status.refresh_token_expired
                )
            } else {
                format!(
                    "configured_account={} account={} cannot determine effective token freshness until a selected grant exists; run `{}`",
                    resolved.configured_account_id,
                    resolved.account.id,
                    crate::feishu_support::feishu_auth_select_command_hint(
                        resolved.configured_account_id.as_str(),
                    )
                )
            },
        });
        checks.push(DoctorCheck {
            name: scoped_feishu_check_name(
                "feishu scope coverage",
                &resolved.configured_account_id,
                scoped,
            ),
            level: if effective_grant.is_none() {
                DoctorCheckLevel::Warn
            } else if effective_status.missing_scopes.is_empty() {
                DoctorCheckLevel::Pass
            } else {
                DoctorCheckLevel::Warn
            },
            detail: if let Some(grant) = effective_grant {
                if effective_status.missing_scopes.is_empty() {
                    format!(
                        "configured_account={} account={} effective_open_id={} required_scopes={} missing_scopes={}",
                        resolved.configured_account_id,
                        resolved.account.id,
                        grant.principal.open_id,
                        required_scopes.join(","),
                        effective_status.missing_scopes.join(",")
                    )
                } else {
                    format!(
                        "configured_account={} account={} effective_open_id={} required_scopes={} missing_scopes={}; rerun `{}`",
                        resolved.configured_account_id,
                        resolved.account.id,
                        grant.principal.open_id,
                        required_scopes.join(","),
                        effective_status.missing_scopes.join(","),
                        crate::feishu_support::feishu_auth_start_command_hint(
                            resolved.configured_account_id.as_str(),
                            false,
                            false,
                        )
                    )
                }
            } else {
                format!(
                    "configured_account={} account={} cannot determine effective scope coverage until a selected grant exists; run `{}`",
                    resolved.configured_account_id,
                    resolved.account.id,
                    crate::feishu_support::feishu_auth_select_command_hint(
                        resolved.configured_account_id.as_str(),
                    )
                )
            },
        });
        let doc_write_status = crate::feishu_support::summarize_required_doc_write_scope_status(
            effective_grant,
            &required_scopes,
        );
        checks.push(DoctorCheck {
            name: scoped_feishu_check_name(
                "feishu doc write readiness",
                &resolved.configured_account_id,
                scoped,
            ),
            level: if effective_grant.is_none() {
                DoctorCheckLevel::Warn
            } else if doc_write_status.ready {
                DoctorCheckLevel::Pass
            } else {
                DoctorCheckLevel::Warn
            },
            detail: if let Some(grant) = effective_grant {
                if doc_write_status.accepted_scopes.is_empty() {
                    format!(
                        "configured_account={} account={} open_id={} doc_write_ready={} not required by current config",
                        resolved.configured_account_id,
                        resolved.account.id,
                        grant.principal.open_id,
                        doc_write_status.ready,
                    )
                } else if doc_write_status.ready {
                    format!(
                        "configured_account={} account={} open_id={} doc_write_ready={} matched_scopes={} accepted_scopes={}",
                        resolved.configured_account_id,
                        resolved.account.id,
                        grant.principal.open_id,
                        doc_write_status.ready,
                        doc_write_status.matched_scopes.join(","),
                        doc_write_status.accepted_scopes.join(","),
                    )
                } else {
                    format!(
                        "configured_account={} account={} open_id={} doc_write_ready={} matched_scopes={} accepted_scopes={}; rerun `{}` to request document write scopes",
                        resolved.configured_account_id,
                        resolved.account.id,
                        grant.principal.open_id,
                        doc_write_status.ready,
                        doc_write_status.matched_scopes.join(","),
                        doc_write_status.accepted_scopes.join(","),
                        crate::feishu_support::feishu_auth_start_command_hint(
                            resolved.configured_account_id.as_str(),
                            false,
                            true,
                        )
                    )
                }
            } else {
                format!(
                    "configured_account={} account={} cannot determine active doc write readiness until a selected grant exists; select one with `{}`",
                    resolved.configured_account_id,
                    resolved.account.id,
                    crate::feishu_support::feishu_auth_select_command_hint(
                        resolved.configured_account_id.as_str(),
                    )
                )
            },
        });
        let write_status = crate::feishu_support::summarize_required_message_write_scope_status(
            effective_grant,
            &required_scopes,
        );
        checks.push(DoctorCheck {
            name: scoped_feishu_check_name(
                "feishu message write readiness",
                &resolved.configured_account_id,
                scoped,
            ),
            level: if effective_grant.is_none() {
                DoctorCheckLevel::Warn
            } else if write_status.ready {
                DoctorCheckLevel::Pass
            } else {
                DoctorCheckLevel::Warn
            },
            detail: if let Some(grant) = effective_grant {
                if write_status.accepted_scopes.is_empty() {
                    format!(
                        "configured_account={} account={} open_id={} write_ready={} not required by current config",
                        resolved.configured_account_id,
                        resolved.account.id,
                        grant.principal.open_id,
                        write_status.ready,
                    )
                } else if write_status.ready {
                    format!(
                        "configured_account={} account={} open_id={} write_ready={} matched_scopes={} accepted_scopes={}",
                        resolved.configured_account_id,
                        resolved.account.id,
                        grant.principal.open_id,
                        write_status.ready,
                        write_status.matched_scopes.join(","),
                        write_status.accepted_scopes.join(","),
                    )
                } else {
                    format!(
                        "configured_account={} account={} open_id={} write_ready={} matched_scopes={} accepted_scopes={}; rerun `{}` to request the recommended write scopes",
                        resolved.configured_account_id,
                        resolved.account.id,
                        grant.principal.open_id,
                        write_status.ready,
                        write_status.matched_scopes.join(","),
                        write_status.accepted_scopes.join(","),
                        crate::feishu_support::feishu_auth_start_command_hint(
                            resolved.configured_account_id.as_str(),
                            true,
                            false,
                        )
                    )
                }
            } else {
                format!(
                    "configured_account={} account={} cannot determine active write readiness until a selected grant exists; select one with `{}`",
                    resolved.configured_account_id,
                    resolved.account.id,
                    crate::feishu_support::feishu_auth_select_command_hint(
                        resolved.configured_account_id.as_str(),
                    )
                )
            },
        });
    }

    checks
}

fn feishu_integration_requested(config: &mvp::config::FeishuChannelConfig) -> bool {
    config.enabled
        || config
            .account_id
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        || config
            .default_account
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        || secret_ref_is_configured(config.app_id.as_ref())
        || secret_ref_is_configured(config.app_secret.as_ref())
        || !config.accounts.is_empty()
}

fn secret_ref_is_configured(secret_ref: Option<&SecretRef>) -> bool {
    let Some(secret_ref) = secret_ref else {
        return false;
    };

    secret_ref.is_configured()
}

fn scoped_feishu_check_name(base_name: &str, configured_account_id: &str, scoped: bool) -> String {
    if !scoped {
        return base_name.to_owned();
    }
    format!("{base_name} [{configured_account_id}]")
}

fn build_channel_surface_checks(
    snapshots: &[mvp::channel::ChannelStatusSnapshot],
) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    let mut counts = BTreeMap::new();
    for snapshot in snapshots {
        *counts.entry(snapshot.id).or_insert(0_usize) += 1;
    }

    for snapshot in snapshots {
        let scoped = counts.get(snapshot.id).copied().unwrap_or(0) > 1;
        if snapshot.is_default_account
            && scoped
            && snapshot.default_account_source
                == mvp::config::ChannelDefaultAccountSelectionSource::Fallback
        {
            checks.push(DoctorCheck {
                name: format!("{} default account policy", snapshot.id),
                level: DoctorCheckLevel::Warn,
                detail: format!(
                    "multiple configured accounts are using fallback default selection; omitting --account currently routes to `{}`. set default_account explicitly to avoid routing surprises",
                    snapshot.configured_account_label
                ),
            });
        }
        for operation in &snapshot.operations {
            let operation_checks =
                build_channel_operation_doctor_checks(snapshot, scoped, operation);
            checks.extend(operation_checks);
        }
        if let Some(check) = build_feishu_inbound_support_check(snapshot, scoped) {
            checks.push(check);
        }
    }

    checks
}

fn build_channel_surface_managed_plugin_discovery_checks(
    surfaces: &[mvp::channel::ChannelSurface],
) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();

    for surface in surfaces {
        let doctor_check = build_channel_surface_managed_plugin_discovery_check(surface);

        if let Some(doctor_check) = doctor_check {
            checks.push(doctor_check);
        }
    }

    checks
}

fn build_channel_surface_managed_plugin_discovery_check(
    surface: &mvp::channel::ChannelSurface,
) -> Option<DoctorCheck> {
    let has_plugin_bridge_contract = surface.catalog.plugin_bridge_contract.is_some();

    if !has_plugin_bridge_contract {
        return None;
    }

    let has_enabled_account = surface
        .configured_accounts
        .iter()
        .any(|snapshot| snapshot.enabled);

    if !has_enabled_account {
        return None;
    }

    let discovery = surface.plugin_bridge_discovery.as_ref()?;
    let check_name = format!("{} managed bridge discovery", surface.catalog.id);
    let check_level = managed_plugin_bridge_discovery_check_level(discovery);
    let check_detail = managed_plugin_bridge_discovery_check_detail(surface, discovery);

    Some(DoctorCheck {
        name: check_name,
        level: check_level,
        detail: check_detail,
    })
}

fn managed_plugin_bridge_discovery_check_level(
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
) -> DoctorCheckLevel {
    match discovery.status {
        mvp::channel::ChannelPluginBridgeDiscoveryStatus::NotConfigured => DoctorCheckLevel::Warn,
        mvp::channel::ChannelPluginBridgeDiscoveryStatus::ScanFailed => DoctorCheckLevel::Fail,
        mvp::channel::ChannelPluginBridgeDiscoveryStatus::NoMatches => DoctorCheckLevel::Warn,
        mvp::channel::ChannelPluginBridgeDiscoveryStatus::MatchesFound => {
            let has_ready_selection = managed_plugin_bridge_selection_is_ready(discovery);

            if has_ready_selection {
                return DoctorCheckLevel::Pass;
            }

            DoctorCheckLevel::Warn
        }
    }
}

fn managed_plugin_bridge_discovery_check_detail(
    surface: &mvp::channel::ChannelSurface,
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
) -> String {
    let managed_install_root =
        crate::render_line_safe_optional_text_value(discovery.managed_install_root.as_deref());
    let configured_plugin_id =
        crate::render_line_safe_optional_text_value(discovery.configured_plugin_id.as_deref());
    let selected_plugin_id =
        crate::render_line_safe_optional_text_value(discovery.selected_plugin_id.as_deref());
    let selection_status = discovery
        .selection_status
        .map(|status| status.as_str())
        .unwrap_or("-");

    match discovery.status {
        mvp::channel::ChannelPluginBridgeDiscoveryStatus::NotConfigured => {
            "managed bridge discovery is unavailable because external_skills.install_root is not configured".to_owned()
        }
        mvp::channel::ChannelPluginBridgeDiscoveryStatus::ScanFailed => {
            let scan_issue = discovery
                .scan_issue
                .as_deref()
                .map(crate::render_line_safe_text_value)
                .unwrap_or_else(|| "unknown scan failure".to_owned());
            let detail =
                format!("managed bridge discovery failed under {managed_install_root}: {scan_issue}");

            detail
        }
        mvp::channel::ChannelPluginBridgeDiscoveryStatus::NoMatches => {
            let has_configured_plugin_id = discovery.configured_plugin_id.is_some();

            if has_configured_plugin_id {
                return format!(
                    "managed bridge discovery found no matching bridge plugins under {managed_install_root}: configured_plugin_id={configured_plugin_id} selection_status={selection_status}"
                );
            }

            let detail = format!(
                "managed bridge discovery found no matching bridge plugins under {managed_install_root}"
            );

            detail
        }
        mvp::channel::ChannelPluginBridgeDiscoveryStatus::MatchesFound => {
            let compatible_plugins = discovery.compatible_plugins;
            let compatible_plugin_ids =
                render_managed_plugin_bridge_compatible_plugin_ids(&discovery.compatible_plugin_ids);
            let ambiguity_status = discovery
                .ambiguity_status
                .map(|status| status.as_str())
                .unwrap_or("-");
            let incomplete_plugins = discovery.incomplete_plugins;
            let incompatible_plugins = discovery.incompatible_plugins;
            let rendered_plugins = render_managed_plugin_bridge_discovery_plugins(&discovery.plugins);
            let mut detail = format!(
                "managed bridge discovery root={managed_install_root} configured_plugin_id={configured_plugin_id} selected_plugin_id={selected_plugin_id} selection_status={selection_status} compatible={compatible_plugins} compatible_plugin_ids={compatible_plugin_ids} ambiguity_status={ambiguity_status} incomplete={incomplete_plugins} incompatible={incompatible_plugins} plugins={rendered_plugins}"
            );

            let account_summary = plugin_bridge_account_summary(surface)
                .map(|summary| crate::render_line_safe_text_value(summary.as_str()));

            if let Some(account_summary) = account_summary {
                detail.push_str(" account_summary=");
                detail.push_str(account_summary.as_str());
            }

            detail
        }
    }
}

fn managed_plugin_bridge_selection_is_ready(
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
) -> bool {
    let selection_status = discovery.selection_status;
    let Some(selection_status) = selection_status else {
        return false;
    };

    selection_status.selects_ready_plugin()
}

fn render_managed_plugin_bridge_compatible_plugin_ids(compatible_plugin_ids: &[String]) -> String {
    crate::render_line_safe_text_values(compatible_plugin_ids.iter().map(String::as_str), ",")
}

fn render_managed_plugin_bridge_discovery_plugins(
    plugins: &[mvp::channel::ChannelDiscoveredPluginBridge],
) -> String {
    if plugins.is_empty() {
        return "-".to_owned();
    }

    let mut rendered_plugins = Vec::new();

    for plugin in plugins {
        let rendered_plugin = render_managed_plugin_bridge_discovery_plugin(plugin);
        rendered_plugins.push(rendered_plugin);
    }

    rendered_plugins.join("; ")
}

fn render_managed_plugin_bridge_discovery_plugin(
    plugin: &mvp::channel::ChannelDiscoveredPluginBridge,
) -> String {
    let mut segments = Vec::new();
    let plugin_id = crate::render_line_safe_text_value(&plugin.plugin_id);
    let bridge_kind = crate::render_line_safe_text_value(&plugin.bridge_kind);
    let adapter_family = crate::render_line_safe_text_value(&plugin.adapter_family);
    let source_path = crate::render_line_safe_text_value(&plugin.source_path);
    let package_root = crate::render_line_safe_text_value(&plugin.package_root);
    let package_manifest_path =
        crate::render_line_safe_optional_text_value(plugin.package_manifest_path.as_deref());

    segments.push(plugin_id);
    segments.push(format!("status={}", plugin.status.as_str()));
    segments.push(format!("bridge_kind={bridge_kind}"));
    segments.push(format!("adapter_family={adapter_family}"));

    if let Some(transport_family) = &plugin.transport_family {
        let rendered_transport_family = crate::render_line_safe_text_value(transport_family);
        segments.push(format!("transport_family={rendered_transport_family}"));
    }

    if let Some(target_contract) = &plugin.target_contract {
        let rendered_target_contract = crate::render_line_safe_text_value(target_contract);
        segments.push(format!("target_contract={rendered_target_contract}"));
    }

    if let Some(account_scope) = &plugin.account_scope {
        let rendered_account_scope = crate::render_line_safe_text_value(account_scope);
        segments.push(format!("account_scope={rendered_account_scope}"));
    }

    if let Some(runtime_contract) = &plugin.runtime_contract {
        let rendered_runtime_contract = crate::render_line_safe_text_value(runtime_contract);
        segments.push(format!("runtime_contract={rendered_runtime_contract}"));
    }

    if !plugin.runtime_operations.is_empty() {
        let rendered_runtime_operations = crate::render_line_safe_text_values(
            plugin.runtime_operations.iter().map(String::as_str),
            ",",
        );
        segments.push(format!("runtime_operations={rendered_runtime_operations}"));
    }

    segments.push(format!("source_path={source_path}"));
    segments.push(format!("package_root={package_root}"));
    segments.push(format!("package_manifest_path={package_manifest_path}"));

    if !plugin.missing_fields.is_empty() {
        let missing_fields = crate::render_line_safe_text_values(
            plugin.missing_fields.iter().map(String::as_str),
            ",",
        );
        segments.push(format!("missing_fields={missing_fields}"));
    }

    if !plugin.issues.is_empty() {
        let issues =
            crate::render_line_safe_text_values(plugin.issues.iter().map(String::as_str), "|");
        segments.push(format!("issues={issues}"));
    }

    if !plugin.required_env_vars.is_empty() {
        let required_env_vars = crate::render_line_safe_text_values(
            plugin.required_env_vars.iter().map(String::as_str),
            ",",
        );
        segments.push(format!("required_env_vars={required_env_vars}"));
    }

    if !plugin.recommended_env_vars.is_empty() {
        let recommended_env_vars = crate::render_line_safe_text_values(
            plugin.recommended_env_vars.iter().map(String::as_str),
            ",",
        );
        segments.push(format!("recommended_env_vars={recommended_env_vars}"));
    }

    if !plugin.required_config_keys.is_empty() {
        let required_config_keys = crate::render_line_safe_text_values(
            plugin.required_config_keys.iter().map(String::as_str),
            ",",
        );
        segments.push(format!("required_config_keys={required_config_keys}"));
    }

    if let Some(default_env_var) = &plugin.default_env_var {
        let rendered_default_env_var = crate::render_line_safe_text_value(default_env_var);
        segments.push(format!("default_env_var={rendered_default_env_var}"));
    }

    if !plugin.setup_docs_urls.is_empty() {
        let setup_docs_urls = crate::render_line_safe_text_values(
            plugin.setup_docs_urls.iter().map(String::as_str),
            ",",
        );
        segments.push(format!("setup_docs_urls={setup_docs_urls}"));
    }

    if let Some(setup_remediation) = &plugin.setup_remediation {
        let rendered_setup_remediation = crate::render_line_safe_text_value(setup_remediation);
        segments.push(format!("setup_remediation={rendered_setup_remediation}"));
    }

    segments.join(" ")
}

fn scoped_doctor_check_name(
    base_name: &str,
    snapshot: &mvp::channel::ChannelStatusSnapshot,
    scoped: bool,
) -> String {
    if !scoped {
        return base_name.to_owned();
    }
    format!("{base_name} [{}]", snapshot.configured_account_label)
}

fn build_feishu_inbound_support_check(
    snapshot: &mvp::channel::ChannelStatusSnapshot,
    scoped: bool,
) -> Option<DoctorCheck> {
    if !snapshot_matches_channel_id(snapshot, "feishu") {
        return None;
    }
    let serve = snapshot.operation("serve")?;
    if serve.health != mvp::channel::ChannelOperationHealth::Ready {
        return None;
    }

    let message_types = snapshot_note_value(snapshot, "webhook_inbound_message_types")?;
    let non_text_mode =
        snapshot_note_value(snapshot, "webhook_inbound_non_text_mode").unwrap_or("unknown");
    let binary_fetch =
        snapshot_note_value(snapshot, "webhook_inbound_binary_fetch").unwrap_or("unknown");
    let resource_download_tool =
        snapshot_note_value(snapshot, "webhook_resource_download_tool").unwrap_or("unknown");
    let resource_selection_mode =
        snapshot_note_value(snapshot, "webhook_resource_selection_mode").unwrap_or("unknown");
    let callback_event_types =
        snapshot_note_value(snapshot, "webhook_callback_event_types").unwrap_or("unknown");
    let callback_response_mode =
        snapshot_note_value(snapshot, "webhook_callback_response_mode").unwrap_or("unknown");

    Some(DoctorCheck {
        name: scoped_doctor_check_name("feishu webhook inbound support", snapshot, scoped),
        level: DoctorCheckLevel::Pass,
        detail: format!(
            "message_types={message_types} non_text_mode={non_text_mode} binary_fetch={binary_fetch} resource_download_tool={resource_download_tool} resource_selection_mode={resource_selection_mode} callback_event_types={callback_event_types} callback_response_mode={callback_response_mode}"
        ),
    })
}

fn snapshot_matches_channel_id(
    snapshot: &mvp::channel::ChannelStatusSnapshot,
    expected_channel_id: &str,
) -> bool {
    let normalized_channel_id = mvp::channel::normalize_channel_catalog_id(snapshot.id);
    normalized_channel_id == Some(expected_channel_id)
}

fn snapshot_note_value<'a>(
    snapshot: &'a mvp::channel::ChannelStatusSnapshot,
    key: &str,
) -> Option<&'a str> {
    let prefix = format!("{key}=");
    snapshot
        .notes
        .iter()
        .find_map(|note| note.strip_prefix(prefix.as_str()))
}

fn build_channel_operation_doctor_checks(
    snapshot: &mvp::channel::ChannelStatusSnapshot,
    scoped: bool,
    operation: &mvp::channel::ChannelOperationStatus,
) -> Vec<DoctorCheck> {
    let doctor_spec =
        mvp::channel::resolve_channel_doctor_operation_spec(snapshot.id, operation.id);
    let Some(doctor_spec) = doctor_spec else {
        return Vec::new();
    };

    let mut checks = Vec::new();
    for check_spec in doctor_spec.checks {
        let doctor_check =
            build_channel_operation_doctor_check(snapshot, scoped, operation, check_spec);
        if let Some(doctor_check) = doctor_check {
            checks.push(doctor_check);
        }
    }
    checks
}

fn build_channel_operation_doctor_check(
    snapshot: &mvp::channel::ChannelStatusSnapshot,
    scoped: bool,
    operation: &mvp::channel::ChannelOperationStatus,
    check_spec: &mvp::channel::ChannelDoctorCheckSpec,
) -> Option<DoctorCheck> {
    let check_name = scoped_doctor_check_name(check_spec.name, snapshot, scoped);
    match check_spec.trigger {
        mvp::channel::ChannelDoctorCheckTrigger::OperationHealth => {
            if operation.health == mvp::channel::ChannelOperationHealth::Disabled {
                return None;
            }

            Some(DoctorCheck {
                name: check_name,
                level: doctor_check_level_for_health(operation.health),
                detail: operation.detail.clone(),
            })
        }
        mvp::channel::ChannelDoctorCheckTrigger::ReadyRuntime => {
            if operation.health != mvp::channel::ChannelOperationHealth::Ready {
                return None;
            }
            let runtime_check = build_channel_runtime_check(check_name.as_str(), operation);
            Some(runtime_check)
        }
        mvp::channel::ChannelDoctorCheckTrigger::PluginBridgeHealth => {
            if operation.health == mvp::channel::ChannelOperationHealth::Disabled {
                return None;
            }
            let bridge_check =
                build_plugin_bridge_health_check(check_name.as_str(), snapshot, operation);
            Some(bridge_check)
        }
    }
}

fn doctor_check_level_for_health(health: mvp::channel::ChannelOperationHealth) -> DoctorCheckLevel {
    match health {
        mvp::channel::ChannelOperationHealth::Ready => DoctorCheckLevel::Pass,
        mvp::channel::ChannelOperationHealth::Disabled => DoctorCheckLevel::Warn,
        mvp::channel::ChannelOperationHealth::Unsupported
        | mvp::channel::ChannelOperationHealth::Misconfigured => DoctorCheckLevel::Fail,
    }
}

fn build_plugin_bridge_health_check(
    name: &str,
    snapshot: &mvp::channel::ChannelStatusSnapshot,
    operation: &mvp::channel::ChannelOperationStatus,
) -> DoctorCheck {
    let level = plugin_bridge_check_level(snapshot, operation);
    let detail = plugin_bridge_check_detail(snapshot, operation);

    DoctorCheck {
        name: name.to_owned(),
        level,
        detail,
    }
}

fn plugin_bridge_check_level(
    snapshot: &mvp::channel::ChannelStatusSnapshot,
    operation: &mvp::channel::ChannelOperationStatus,
) -> DoctorCheckLevel {
    match operation.health {
        mvp::channel::ChannelOperationHealth::Ready => DoctorCheckLevel::Pass,
        mvp::channel::ChannelOperationHealth::Disabled => DoctorCheckLevel::Warn,
        mvp::channel::ChannelOperationHealth::Misconfigured => DoctorCheckLevel::Fail,
        mvp::channel::ChannelOperationHealth::Unsupported => {
            let external_plugin_owner = snapshot_has_external_plugin_bridge_owner(snapshot);

            if snapshot.compiled && external_plugin_owner {
                return DoctorCheckLevel::Pass;
            }

            DoctorCheckLevel::Fail
        }
    }
}

fn plugin_bridge_check_detail(
    snapshot: &mvp::channel::ChannelStatusSnapshot,
    operation: &mvp::channel::ChannelOperationStatus,
) -> String {
    let external_plugin_owner = snapshot_has_external_plugin_bridge_owner(snapshot);
    let supported_external_bridge = snapshot.compiled && external_plugin_owner;
    let is_bridge_contract = operation.health == mvp::channel::ChannelOperationHealth::Unsupported;

    if supported_external_bridge && is_bridge_contract {
        let detail = operation.detail.as_str();
        return format!("configured for external bridge runtime ownership; {detail}");
    }

    operation.detail.clone()
}

fn snapshot_has_external_plugin_bridge_owner(
    snapshot: &mvp::channel::ChannelStatusSnapshot,
) -> bool {
    let bridge_runtime_owner = snapshot_note_value(snapshot, "bridge_runtime_owner");
    bridge_runtime_owner == Some("external_plugin")
}

fn build_channel_runtime_check(
    name: &str,
    operation: &mvp::channel::ChannelOperationStatus,
) -> DoctorCheck {
    let Some(runtime) = operation.runtime.as_ref() else {
        return DoctorCheck {
            name: name.to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "ready but runtime tracking is unavailable".to_owned(),
        };
    };

    let detail_tail = format!(
        "account={} account_id={} pid={} busy={} active_runs={} instance_count={} running_instances={} stale_instances={} last_run_activity_at={} last_heartbeat_at={}",
        runtime.account_label.as_deref().unwrap_or("-"),
        runtime.account_id.as_deref().unwrap_or("-"),
        runtime
            .pid
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned()),
        runtime.busy,
        runtime.active_runs,
        runtime.instance_count,
        runtime.running_instances,
        runtime.stale_instances,
        runtime
            .last_run_activity_at
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned()),
        runtime
            .last_heartbeat_at
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned()),
    );

    if runtime.stale {
        return DoctorCheck {
            name: name.to_owned(),
            level: DoctorCheckLevel::Fail,
            detail: format!("stale runtime detected ({detail_tail})"),
        };
    }

    if runtime.running {
        if runtime.running_instances > 1 {
            return DoctorCheck {
                name: name.to_owned(),
                level: DoctorCheckLevel::Warn,
                detail: format!("multiple runtime instances detected ({detail_tail})"),
            };
        }

        return DoctorCheck {
            name: name.to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: format!("running ({detail_tail})"),
        };
    }

    DoctorCheck {
        name: name.to_owned(),
        level: DoctorCheckLevel::Warn,
        detail: format!("ready but not currently running ({detail_tail})"),
    }
}

fn maybe_apply_provider_env_fix(
    config: &mut mvp::config::LoongConfig,
    fix: bool,
    fixes: &mut Vec<String>,
) -> bool {
    if !fix {
        return false;
    }
    let binding =
        provider_credential_policy::preferred_provider_credential_env_binding(&config.provider);
    let Some(binding) = binding else {
        return false;
    };
    match binding.field {
        provider_credential_policy::ProviderCredentialEnvField::ApiKey => {
            ensure_provider_env_binding(
                &mut config.provider,
                provider_credential_policy::ProviderCredentialEnvField::ApiKey,
                &binding.env_name,
                fixes,
                "set provider.api_key.env",
            )
        }
        provider_credential_policy::ProviderCredentialEnvField::OAuthAccessToken => {
            ensure_provider_env_binding(
                &mut config.provider,
                provider_credential_policy::ProviderCredentialEnvField::OAuthAccessToken,
                &binding.env_name,
                fixes,
                "set provider.oauth_access_token.env",
            )
        }
    }
}

fn maybe_apply_channel_env_fix(
    config: &mut mvp::config::LoongConfig,
    fix: bool,
    fixes: &mut Vec<String>,
) -> bool {
    if !fix {
        return false;
    }
    let channel_fixes = crate::migration::channels::apply_default_channel_env_bindings(config);
    let changed = !channel_fixes.is_empty();
    fixes.extend(channel_fixes);
    changed
}

#[cfg(test)]
fn ensure_env_binding(
    slot: &mut Option<String>,
    default_key: &str,
    fixes: &mut Vec<String>,
    label: &'static str,
) -> bool {
    if slot
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        return false;
    }
    *slot = Some(default_key.to_owned());
    fixes.push(format!("{label}={default_key}"));
    true
}

fn ensure_provider_env_binding(
    provider: &mut mvp::config::ProviderConfig,
    field: provider_credential_policy::ProviderCredentialEnvField,
    default_key: &str,
    fixes: &mut Vec<String>,
    label: &'static str,
) -> bool {
    let configured = match field {
        provider_credential_policy::ProviderCredentialEnvField::ApiKey => {
            provider.configured_api_key_env_override()
        }
        provider_credential_policy::ProviderCredentialEnvField::OAuthAccessToken => {
            provider.configured_oauth_access_token_env_override()
        }
    };
    if configured.is_some() {
        return false;
    }
    if provider_has_non_env_credential(provider) {
        return false;
    }

    match field {
        provider_credential_policy::ProviderCredentialEnvField::ApiKey => {
            provider.set_api_key_env_binding(Some(default_key.to_owned()));
        }
        provider_credential_policy::ProviderCredentialEnvField::OAuthAccessToken => {
            provider.set_oauth_access_token_env_binding(Some(default_key.to_owned()));
        }
    }

    fixes.push(format!("{label}={default_key}"));
    true
}

fn provider_has_non_env_credential(provider: &mvp::config::ProviderConfig) -> bool {
    provider_secret_ref_is_non_env_credential(provider.api_key.as_ref())
        || provider_secret_ref_is_non_env_credential(provider.oauth_access_token.as_ref())
}

fn provider_secret_ref_is_non_env_credential(secret_ref: Option<&SecretRef>) -> bool {
    let Some(secret_ref) = secret_ref else {
        return false;
    };

    secret_ref.is_configured() && secret_ref.explicit_env_name().is_none()
}

fn provider_transport_doctor_check(provider: &mvp::config::ProviderConfig) -> DoctorCheck {
    let readiness = provider.transport_readiness();
    DoctorCheck {
        name: "provider transport".to_owned(),
        level: match readiness.level {
            mvp::config::ProviderTransportReadinessLevel::Ready => DoctorCheckLevel::Pass,
            mvp::config::ProviderTransportReadinessLevel::Review => DoctorCheckLevel::Warn,
            mvp::config::ProviderTransportReadinessLevel::Unsupported => DoctorCheckLevel::Fail,
        },
        detail: readiness.detail,
    }
}

fn provider_route_probe_doctor_check(
    probe: &crate::provider_route_diagnostics::ProviderRouteProbe,
) -> DoctorCheck {
    DoctorCheck {
        name: crate::provider_route_diagnostics::PROVIDER_ROUTE_PROBE_CHECK_NAME.to_owned(),
        level: match probe.level {
            crate::provider_route_diagnostics::ProviderRouteProbeLevel::Pass => {
                DoctorCheckLevel::Pass
            }
            crate::provider_route_diagnostics::ProviderRouteProbeLevel::Warn => {
                DoctorCheckLevel::Warn
            }
            crate::provider_route_diagnostics::ProviderRouteProbeLevel::Fail => {
                DoctorCheckLevel::Fail
            }
        },
        detail: probe.detail.clone(),
    }
}

fn provider_credentials_doctor_check(
    config: &mvp::config::LoongConfig,
    has_provider_credentials: bool,
) -> DoctorCheck {
    let provider_label = crate::provider_presentation::active_provider_detail_label(config);
    let support_facts = config.provider.support_facts();
    let auth_support = support_facts.auth;
    if has_provider_credentials {
        return DoctorCheck {
            name: "provider credentials".to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: format!("{provider_label}: provider credentials are available"),
        };
    }

    if !auth_support.requires_explicit_configuration {
        return DoctorCheck {
            name: "provider credentials".to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: format!(
                "{provider_label}: provider credentials are optional for this provider"
            ),
        };
    }

    let detail = auth_support.missing_configuration_message;
    DoctorCheck {
        name: "provider credentials".to_owned(),
        level: DoctorCheckLevel::Warn,
        detail: format!("{provider_label}: {detail}"),
    }
}

fn web_search_provider_doctor_check(config: &mvp::config::LoongConfig) -> DoctorCheck {
    if !config.tools.web_search.enabled {
        return DoctorCheck {
            name: "web search provider".to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: "tools.web_search.enabled=false".to_owned(),
        };
    }

    let configured_provider = config.tools.web_search.default_provider.as_str();
    let normalized_provider = mvp::config::normalize_web_search_provider(configured_provider);
    let provider = normalized_provider.unwrap_or(mvp::config::DEFAULT_WEB_SEARCH_PROVIDER);
    let provider_label = crate::onboard_web_search::web_search_provider_display_name(provider);
    let credential_summary =
        crate::onboard_web_search::summarize_web_search_provider_credential(config, provider);
    let credential_available =
        crate::onboard_web_search::web_search_provider_has_available_credential(config, provider);

    if credential_available {
        let detail = credential_summary
            .map(|summary| format!("{provider_label}: {}", summary.value))
            .unwrap_or_else(|| provider_label.clone());

        return DoctorCheck {
            name: "web search provider".to_owned(),
            level: DoctorCheckLevel::Pass,
            detail,
        };
    }

    let detail = credential_summary
        .map(|summary| {
            format!(
                "{provider_label}: {}. web.search will stay unavailable until the provider credential is supplied",
                summary.value
            )
        })
        .unwrap_or_else(|| provider_label.clone());

    DoctorCheck {
        name: "web search provider".to_owned(),
        level: DoctorCheckLevel::Warn,
        detail,
    }
}

fn doctor_check_from_provider_model_probe_failure(
    probe_failure: provider_model_probe_policy::ProviderModelProbeFailure,
) -> DoctorCheck {
    let level = match probe_failure.level {
        provider_model_probe_policy::ProviderModelProbeFailureLevel::Warn => DoctorCheckLevel::Warn,
        provider_model_probe_policy::ProviderModelProbeFailureLevel::Fail => DoctorCheckLevel::Fail,
    };

    DoctorCheck {
        name: "provider model probe".to_owned(),
        level,
        detail: probe_failure.detail,
    }
}

#[cfg(test)]
fn provider_model_probe_failure_check(
    config: &mvp::config::LoongConfig,
    error: String,
) -> DoctorCheck {
    let probe_failure =
        provider_model_probe_policy::provider_model_probe_failure(config, error.as_str());
    doctor_check_from_provider_model_probe_failure(probe_failure)
}

fn is_provider_model_probe_failure_check(check: &DoctorCheck) -> bool {
    let is_provider_model_probe = check.name == "provider model probe";
    let is_failure = check.level != DoctorCheckLevel::Pass;
    let matches_probe_failure_detail =
        provider_model_probe_policy::provider_model_probe_failed_detail(check.detail.as_str());

    is_provider_model_probe && is_failure && matches_probe_failure_detail
}

fn provider_model_probe_recovery_advice_for_checks(
    checks: &[DoctorCheck],
    config: &mvp::config::LoongConfig,
) -> Option<provider_model_probe_policy::ProviderModelProbeRecoveryAdvice> {
    let probe_failure_check = checks
        .iter()
        .find(|check| is_provider_model_probe_failure_check(check))?;
    let recovery_advice = provider_model_probe_policy::provider_model_probe_recovery_advice(
        config,
        probe_failure_check.detail.as_str(),
    )?;
    Some(recovery_advice)
}

async fn collect_browser_companion_doctor_checks(
    config: &mvp::config::LoongConfig,
) -> Vec<DoctorCheck> {
    let Some(diagnostics) =
        crate::browser_companion_diagnostics::collect_browser_companion_diagnostics(config).await
    else {
        return Vec::new();
    };

    let install_level = if diagnostics.install_ready() {
        DoctorCheckLevel::Pass
    } else {
        DoctorCheckLevel::Warn
    };
    let mut checks = vec![DoctorCheck {
        name: crate::browser_companion_diagnostics::BROWSER_COMPANION_INSTALL_CHECK_NAME.to_owned(),
        level: install_level,
        detail: diagnostics.install_detail(),
    }];

    if let Some(detail) = diagnostics.runtime_gate_detail() {
        checks.push(DoctorCheck {
            name: crate::browser_companion_diagnostics::BROWSER_COMPANION_RUNTIME_GATE_CHECK_NAME
                .to_owned(),
            level: if diagnostics.runtime_ready {
                DoctorCheckLevel::Pass
            } else {
                DoctorCheckLevel::Warn
            },
            detail,
        });
    }

    checks
}

pub fn resolve_secret_value(inline: Option<&str>, env_key: Option<&str>) -> Option<String> {
    if let Some(value) = inline.map(str::trim).filter(|value| !value.is_empty()) {
        return Some(value.to_owned());
    }
    let key = env_key.map(str::trim).filter(|value| !value.is_empty())?;
    let value = env::var(key).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

fn summarize_checks(checks: &[DoctorCheck]) -> DoctorSummary {
    let mut pass = 0_usize;
    let mut warn = 0_usize;
    let mut fail = 0_usize;
    for check in checks {
        match check.level {
            DoctorCheckLevel::Pass => pass += 1,
            DoctorCheckLevel::Warn => warn += 1,
            DoctorCheckLevel::Fail => fail += 1,
        }
    }
    DoctorSummary { pass, warn, fail }
}

fn render_doctor_text(
    checks: &[DoctorCheck],
    summary: DoctorSummary,
    fixes: &[String],
    next_steps: &[String],
    config_path: &Path,
    fix_requested: bool,
) -> String {
    let mut sections = Vec::new();
    sections.push(mvp::tui_surface::TuiSectionSpec::Callout {
        tone: if summary.fail == 0 {
            mvp::tui_surface::TuiCalloutTone::Success
        } else {
            mvp::tui_surface::TuiCalloutTone::Warning
        },
        title: Some("summary".to_owned()),
        lines: vec![format!(
            "{} ok · {} warn · {} fail",
            summary.pass, summary.warn, summary.fail
        )],
    });
    sections.push(mvp::tui_surface::TuiSectionSpec::Checklist {
        title: Some("checks".to_owned()),
        items: checks
            .iter()
            .map(|check| mvp::tui_surface::TuiChecklistItemSpec {
                status: match check.level {
                    DoctorCheckLevel::Pass => mvp::tui_surface::TuiChecklistStatus::Pass,
                    DoctorCheckLevel::Warn => mvp::tui_surface::TuiChecklistStatus::Warn,
                    DoctorCheckLevel::Fail => mvp::tui_surface::TuiChecklistStatus::Fail,
                },
                label: check.name.clone(),
                detail: check.detail.clone(),
            })
            .collect(),
    });

    let action_items = next_steps
        .iter()
        .filter_map(|step| {
            let (label, command) = step.split_once(": ")?;
            Some(mvp::tui_surface::TuiActionSpec {
                label: label.to_owned(),
                command: command.to_owned(),
            })
        })
        .take(3)
        .collect::<Vec<_>>();
    if !action_items.is_empty() {
        sections.push(mvp::tui_surface::TuiSectionSpec::ActionGroup {
            title: Some("start here".to_owned()),
            inline_title_when_wide: false,
            items: action_items,
        });
    }
    if fix_requested {
        let fix_lines = if fixes.is_empty() {
            vec!["applied fixes: none".to_owned()]
        } else {
            fixes.iter().map(|fix| format!("- {fix}")).collect()
        };
        sections.push(mvp::tui_surface::TuiSectionSpec::Narrative {
            title: Some("applied fixes".to_owned()),
            lines: fix_lines,
        });
    }
    if !next_steps.is_empty() {
        sections.push(mvp::tui_surface::TuiSectionSpec::Narrative {
            title: Some("next actions".to_owned()),
            lines: next_steps.iter().map(|step| format!("- {step}")).collect(),
        });
    }

    let screen = mvp::tui_surface::TuiScreenSpec {
        header_style: mvp::tui_surface::TuiHeaderStyle::Compact,
        subtitle: Some("runtime health".to_owned()),
        title: Some("doctor".to_owned()),
        progress_line: None,
        intro_lines: vec![format!("config={}", config_path.display())],
        sections,
        choices: Vec::new(),
        footer_lines: vec![
            "Use `loong doctor --json` for machine-readable diagnostics.".to_owned(),
        ],
    };

    mvp::tui_surface::render_tui_screen_spec_ratatui(
        &screen,
        mvp::presentation::detect_render_width(),
        false,
    )
    .join("\n")
}

fn check_level_json(level: DoctorCheckLevel) -> &'static str {
    match level {
        DoctorCheckLevel::Pass => "ok",
        DoctorCheckLevel::Warn => "warn",
        DoctorCheckLevel::Fail => "fail",
    }
}

fn doctor_checks_json_payload(
    checks: &[DoctorCheck],
    channel_surfaces: &[mvp::channel::ChannelSurface],
) -> Vec<serde_json::Value> {
    let account_summaries = doctor_plugin_bridge_account_summaries(channel_surfaces);
    let mut payload = Vec::with_capacity(checks.len());

    for check in checks {
        let mut object = serde_json::Map::new();
        let level = check_level_json(check.level).to_owned();
        let account_summary = account_summaries.get(check.name.as_str());

        object.insert(
            "name".to_owned(),
            serde_json::Value::String(check.name.clone()),
        );
        object.insert("level".to_owned(), serde_json::Value::String(level));
        object.insert(
            "detail".to_owned(),
            serde_json::Value::String(check.detail.clone()),
        );

        if let Some(account_summary) = account_summary {
            object.insert(
                "plugin_bridge_account_summary".to_owned(),
                serde_json::Value::String(account_summary.clone()),
            );
        }

        payload.push(serde_json::Value::Object(object));
    }

    payload
}

fn doctor_plugin_bridge_account_summaries(
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

fn doctor_render_string_list(values: &[String]) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }

    crate::render_line_safe_text_values(values.iter().map(String::as_str), ",")
}

#[cfg(test)]
fn build_doctor_next_steps(
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
fn build_doctor_next_steps_with_path_env(
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

fn build_doctor_next_steps_with_channel_surfaces_and_path_env(
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

    if checks
        .iter()
        .any(|check| check.name == "web search provider" && check.level != DoctorCheckLevel::Pass)
    {
        let configured_provider = config.tools.web_search.default_provider.as_str();
        let normalized_provider = mvp::config::normalize_web_search_provider(configured_provider);
        let provider = normalized_provider.unwrap_or(mvp::config::DEFAULT_WEB_SEARCH_PROVIDER);
        let descriptor = mvp::config::web_search_provider_descriptor(provider);
        let default_env_name = descriptor.and_then(|value| value.default_api_key_env);

        if let Some(default_env_name) = default_env_name {
            push_unique_step(
                &mut steps,
                format!("Set web search credential in env: {default_env_name}"),
            );
        }

        push_unique_step(
            &mut steps,
            format!(
                "Or rerun onboarding to review the web search provider choice: {rerun_onboard_command}"
            ),
        );
    }

    let provider_model_probe_recovery =
        provider_model_probe_recovery_advice_for_checks(checks, config);
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
                check.name == crate::provider_route_diagnostics::PROVIDER_ROUTE_PROBE_CHECK_NAME
                    && check.level != DoctorCheckLevel::Pass
            }) {
                push_unique_step(
                    &mut steps,
                    format!(
                        "Fix the active provider route (DNS / proxy / TUN), then re-run diagnostics: {rerun_command}"
                    ),
                );
                if checks.iter().any(|check| {
                    check.name == crate::provider_route_diagnostics::PROVIDER_ROUTE_PROBE_CHECK_NAME
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

    if checks.iter().any(|check| {
        check.name == crate::browser_companion_diagnostics::BROWSER_COMPANION_INSTALL_CHECK_NAME
            && check.level != DoctorCheckLevel::Pass
    }) {
        push_unique_step(
            &mut steps,
            format!(
                "Install or expose the browser companion command on PATH, then re-run: {rerun_command}"
            ),
        );
        if checks.iter().any(|check| {
            check.name == crate::browser_companion_diagnostics::BROWSER_COMPANION_INSTALL_CHECK_NAME
                && check.detail.contains("expected_version=")
        }) {
            push_unique_step(
                &mut steps,
                "Align `tools.browser_companion.expected_version` with the installed companion build before retrying."
                    .to_owned(),
            );
        }
    }

    if checks.iter().any(|check| {
        check.name
            == crate::browser_companion_diagnostics::BROWSER_COMPANION_RUNTIME_GATE_CHECK_NAME
            && check.level != DoctorCheckLevel::Pass
    }) {
        push_unique_step(
            &mut steps,
            format!(
                "Keep using the built-in browser lane, or disable `tools.browser_companion.enabled` until the managed companion runtime is ready, then re-run: {rerun_command}"
            ),
        );
    }

    let runtime_snapshot_json_command = format!(
        "{} runtime-snapshot --json --config {}",
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
        let mut browser_preview_needs_runtime_verify = false;
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
                crate::next_actions::SetupNextActionKind::BrowserPreview => {
                    match action.browser_preview_phase {
                        Some(crate::next_actions::BrowserPreviewActionPhase::Enable) => {
                            "Optional browser preview"
                        }
                        Some(crate::next_actions::BrowserPreviewActionPhase::Unblock) => {
                            "Unblock browser preview"
                        }
                        Some(crate::next_actions::BrowserPreviewActionPhase::InstallRuntime) => {
                            "Install browser preview runtime"
                        }
                        Some(crate::next_actions::BrowserPreviewActionPhase::Ready) | None => {
                            "Try browser companion preview"
                        }
                    }
                }
                crate::next_actions::SetupNextActionKind::Doctor => "Run diagnostics",
            };
            if action.kind == crate::next_actions::SetupNextActionKind::BrowserPreview
                && action.browser_preview_phase
                    == Some(crate::next_actions::BrowserPreviewActionPhase::InstallRuntime)
            {
                browser_preview_needs_runtime_verify = true;
            }
            push_unique_step(&mut steps, format!("{prefix}: {}", action.command));
        }
        if browser_preview_needs_runtime_verify {
            push_unique_step(
                &mut steps,
                crate::browser_preview::browser_preview_verify_step(),
            );
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

fn managed_bridge_incomplete_setup_step(
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

fn render_managed_bridge_compatible_plugin_labels(
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
) -> String {
    let duplicate_plugin_id_counts = managed_bridge_duplicate_plugin_id_counts(&discovery.plugins);
    let mut compatible_plugin_labels = Vec::new();

    for plugin in &discovery.plugins {
        let is_compatible =
            plugin.status == mvp::channel::ChannelDiscoveredPluginBridgeStatus::CompatibleReady;

        if !is_compatible {
            continue;
        }

        let plugin_label = managed_bridge_plugin_label(plugin, &duplicate_plugin_id_counts);
        compatible_plugin_labels.push(plugin_label);
    }

    crate::render_line_safe_text_values(compatible_plugin_labels.iter().map(String::as_str), ",")
}

fn render_managed_bridge_configured_plugin_labels(
    discovery: &mvp::channel::ChannelPluginBridgeDiscovery,
) -> String {
    let configured_plugin_id = discovery.configured_plugin_id.as_deref();
    let Some(configured_plugin_id) = configured_plugin_id else {
        return "-".to_owned();
    };

    let duplicate_plugin_id_counts = managed_bridge_duplicate_plugin_id_counts(&discovery.plugins);
    let mut matching_plugin_labels = Vec::new();

    for plugin in &discovery.plugins {
        let matches_configured_plugin_id = plugin.plugin_id == configured_plugin_id;

        if !matches_configured_plugin_id {
            continue;
        }

        let plugin_label = managed_bridge_plugin_label(plugin, &duplicate_plugin_id_counts);
        matching_plugin_labels.push(plugin_label);
    }

    crate::render_line_safe_text_values(matching_plugin_labels.iter().map(String::as_str), ",")
}

fn managed_bridge_duplicate_plugin_id_counts(
    plugins: &[mvp::channel::ChannelDiscoveredPluginBridge],
) -> BTreeMap<String, usize> {
    let mut duplicate_plugin_id_counts = BTreeMap::new();

    for plugin in plugins {
        let count = duplicate_plugin_id_counts
            .entry(plugin.plugin_id.clone())
            .or_insert(0);
        *count += 1;
    }

    duplicate_plugin_id_counts
}

fn managed_bridge_plugin_label(
    plugin: &mvp::channel::ChannelDiscoveredPluginBridge,
    duplicate_plugin_id_counts: &BTreeMap<String, usize>,
) -> String {
    let duplicate_count = duplicate_plugin_id_counts
        .get(&plugin.plugin_id)
        .copied()
        .unwrap_or(0);
    let has_duplicate_plugin_id = duplicate_count > 1;

    if !has_duplicate_plugin_id {
        return plugin.plugin_id.clone();
    }

    format!("{}@{}", plugin.plugin_id, plugin.package_root)
}

fn doctor_ready_for_first_turn(checks: &[DoctorCheck]) -> bool {
    checks
        .iter()
        .all(|check| check.level != DoctorCheckLevel::Fail)
        && checks.iter().any(|check| {
            check.name == "provider credentials" && check.level == DoctorCheckLevel::Pass
        })
}

fn select_doctor_first_turn_actions(
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
        is_repair_priority_browser_preview_action(action)
    });
    push_first_matching_action(&mut prioritized, &actions, |action| {
        action.kind == crate::next_actions::SetupNextActionKind::Personalize
    });
    push_first_matching_action(&mut prioritized, &actions, |action| {
        is_channel_catalog_action(action)
    });
    push_first_matching_action(&mut prioritized, &actions, |action| {
        is_general_browser_preview_action(action)
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

fn is_channel_catalog_action(action: &crate::next_actions::SetupNextAction) -> bool {
    let kind = &action.kind;
    let channel_action_id = action.channel_action_id;
    *kind == crate::next_actions::SetupNextActionKind::Channel
        && channel_action_id == Some(crate::migration::channels::CHANNEL_CATALOG_ACTION_ID)
}

fn is_repair_priority_browser_preview_action(
    action: &crate::next_actions::SetupNextAction,
) -> bool {
    let kind = &action.kind;
    let phase = action.browser_preview_phase;
    *kind == crate::next_actions::SetupNextActionKind::BrowserPreview
        && matches!(
            phase,
            Some(crate::next_actions::BrowserPreviewActionPhase::Unblock)
                | Some(crate::next_actions::BrowserPreviewActionPhase::InstallRuntime)
        )
}

fn is_general_browser_preview_action(action: &crate::next_actions::SetupNextAction) -> bool {
    let kind = &action.kind;
    let phase = action.browser_preview_phase;
    let is_browser_preview = *kind == crate::next_actions::SetupNextActionKind::BrowserPreview;
    let is_general_phase = matches!(
        phase,
        Some(crate::next_actions::BrowserPreviewActionPhase::Ready)
            | Some(crate::next_actions::BrowserPreviewActionPhase::Enable)
    );
    is_browser_preview && is_general_phase
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    #[cfg(unix)]
    use std::ffi::OsString;
    use std::fs::Permissions;
    #[cfg(unix)]
    use std::path::{Path, PathBuf};
    #[cfg(unix)]
    use std::sync::MutexGuard;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static FEISHU_TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    use super::*;
    use crate::test_support::ScopedEnv;
    use kernel::AuditSink;
    use mvp::channel::{
        ChannelOperationHealth, ChannelOperationRuntime, ChannelOperationStatus,
        ChannelStatusSnapshot,
    };

    fn browser_companion_temp_dir(label: &str) -> PathBuf {
        static NEXT_TEMP_DIR_SEED: AtomicU64 = AtomicU64::new(1);
        let seed = NEXT_TEMP_DIR_SEED.fetch_add(1, Ordering::Relaxed);
        let temp_dir = std::env::temp_dir().join(format!(
            "loong-browser-companion-doctor-{label}-{}-{seed}",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp_dir).expect("create browser companion temp dir");
        temp_dir
    }

    struct DoctorCliCurrentDirGuard {
        original: PathBuf,
    }

    impl DoctorCliCurrentDirGuard {
        fn set(path: &Path) -> Self {
            let original = std::env::current_dir().expect("read current dir");
            std::env::set_current_dir(path).expect("set current dir");
            Self { original }
        }
    }

    impl Drop for DoctorCliCurrentDirGuard {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.original).expect("restore current dir");
        }
    }

    fn runtime_plugins_test_config(root: &Path, enabled: bool) -> mvp::config::LoongConfig {
        let mut config = mvp::config::LoongConfig::default();
        config.tools.file_root = Some(root.display().to_string());
        config.runtime_plugins.enabled = enabled;
        config.runtime_plugins.roots = vec![root.join("runtime-plugins").display().to_string()];
        config
    }

    fn sample_audit_event(
        event_id: &str,
        timestamp_epoch_s: u64,
        agent_id: Option<&str>,
        kind: kernel::AuditEventKind,
    ) -> kernel::AuditEvent {
        kernel::AuditEvent {
            event_id: event_id.to_owned(),
            timestamp_epoch_s,
            agent_id: agent_id.map(str::to_owned),
            kind,
        }
    }

    fn managed_bridge_manifest(
        channel_id: &str,
        setup_surface: Option<&str>,
        metadata: BTreeMap<String, String>,
    ) -> kernel::PluginManifest {
        let mut metadata = metadata;
        metadata
            .entry("channel_runtime_contract".to_owned())
            .or_insert_with(|| "loongclaw_channel_bridge_v1".to_owned());
        metadata
            .entry("channel_runtime_operations_json".to_owned())
            .or_insert_with(|| {
                serde_json::to_string(&[
                    "send_message",
                    "receive_batch",
                    "ack_inbound",
                    "complete_batch",
                ])
                .expect("serialize default runtime operations")
            });
        let setup = setup_surface.map(|surface| kernel::PluginSetup {
            mode: kernel::PluginSetupMode::MetadataOnly,
            surface: Some(surface.to_owned()),
            required_env_vars: Vec::new(),
            recommended_env_vars: Vec::new(),
            required_config_keys: Vec::new(),
            default_env_var: None,
            docs_urls: Vec::new(),
            remediation: None,
        });
        managed_bridge_manifest_with_setup(channel_id, metadata, setup)
    }

    fn managed_bridge_manifest_with_setup(
        channel_id: &str,
        metadata: BTreeMap<String, String>,
        setup: Option<kernel::PluginSetup>,
    ) -> kernel::PluginManifest {
        let plugin_id = format!("{channel_id}-managed-bridge");

        managed_bridge_manifest_with_plugin_id(&plugin_id, channel_id, metadata, setup)
    }

    fn managed_bridge_manifest_with_plugin_id(
        plugin_id: &str,
        channel_id: &str,
        metadata: BTreeMap<String, String>,
        setup: Option<kernel::PluginSetup>,
    ) -> kernel::PluginManifest {
        kernel::PluginManifest {
            api_version: Some("v1alpha1".to_owned()),
            version: Some("1.0.0".to_owned()),
            plugin_id: plugin_id.to_owned(),
            provider_id: format!("{channel_id}-provider"),
            connector_name: format!("{channel_id}-connector"),
            channel_id: Some(channel_id.to_owned()),
            endpoint: Some("http://127.0.0.1:9999/invoke".to_owned()),
            capabilities: BTreeSet::new(),
            trust_tier: kernel::PluginTrustTier::Unverified,
            metadata,
            summary: None,
            tags: Vec::new(),
            input_examples: Vec::new(),
            output_examples: Vec::new(),
            defer_loading: false,
            setup,
            slot_claims: Vec::new(),
            compatibility: None,
        }
    }

    #[test]
    fn select_doctor_first_turn_actions_skips_doctor_self_recursion() {
        let actions = vec![
            crate::next_actions::SetupNextAction {
                kind: crate::next_actions::SetupNextActionKind::Doctor,
                channel_action_id: None,
                browser_preview_phase: None,
                label: "verify managed bridges".to_owned(),
                command: "loong doctor --config '/tmp/loong-config.toml'".to_owned(),
            },
            crate::next_actions::SetupNextAction {
                kind: crate::next_actions::SetupNextActionKind::Channel,
                channel_action_id: Some(crate::migration::channels::CHANNEL_CATALOG_ACTION_ID),
                browser_preview_phase: None,
                label: "channels".to_owned(),
                command: "loong channels --config '/tmp/loong-config.toml'".to_owned(),
            },
        ];

        let selected = select_doctor_first_turn_actions(actions);

        assert_eq!(selected.len(), 1);
        assert_eq!(
            selected[0].kind,
            crate::next_actions::SetupNextActionKind::Channel
        );
        assert_eq!(selected[0].label, "channels");
        assert!(
            selected
                .iter()
                .all(|action| { action.kind != crate::next_actions::SetupNextActionKind::Doctor }),
            "doctor success follow-ups should not suggest running doctor again: {selected:#?}"
        );
    }

    fn managed_bridge_setup_with_guidance(
        surface: &str,
        required_env_vars: Vec<&str>,
        required_config_keys: Vec<&str>,
        docs_urls: Vec<&str>,
        remediation: Option<&str>,
    ) -> kernel::PluginSetup {
        let normalized_required_env_vars =
            required_env_vars.into_iter().map(str::to_owned).collect();
        let normalized_required_config_keys = required_config_keys
            .into_iter()
            .map(str::to_owned)
            .collect();
        let normalized_docs_urls = docs_urls.into_iter().map(str::to_owned).collect();
        let normalized_remediation = remediation.map(str::to_owned);

        kernel::PluginSetup {
            mode: kernel::PluginSetupMode::MetadataOnly,
            surface: Some(surface.to_owned()),
            required_env_vars: normalized_required_env_vars,
            recommended_env_vars: Vec::new(),
            required_config_keys: normalized_required_config_keys,
            default_env_var: None,
            docs_urls: normalized_docs_urls,
            remediation: normalized_remediation,
        }
    }

    fn compatible_managed_bridge_metadata(
        transport_family: &str,
        target_contract: &str,
    ) -> BTreeMap<String, String> {
        let mut metadata = BTreeMap::new();
        let runtime_operations = serde_json::to_string(&[
            "send_message",
            "receive_batch",
            "ack_inbound",
            "complete_batch",
        ])
        .expect("serialize runtime operations");

        metadata.insert("adapter_family".to_owned(), "channel-bridge".to_owned());
        metadata.insert("transport_family".to_owned(), transport_family.to_owned());
        metadata.insert("target_contract".to_owned(), target_contract.to_owned());
        metadata.insert(
            "channel_runtime_contract".to_owned(),
            mvp::channel::CHANNEL_PLUGIN_BRIDGE_RUNTIME_CONTRACT_V1.to_owned(),
        );
        metadata.insert(
            "channel_runtime_operations_json".to_owned(),
            runtime_operations,
        );

        metadata
    }

    fn write_managed_bridge_manifest(
        install_root: &Path,
        directory_name: &str,
        manifest: &kernel::PluginManifest,
    ) {
        let plugin_directory = install_root.join(directory_name);
        let manifest_path = plugin_directory.join("loong.plugin.json");
        let encoded_manifest =
            serde_json::to_string_pretty(manifest).expect("serialize managed bridge manifest");

        std::fs::create_dir_all(&plugin_directory).expect("create managed bridge plugin directory");
        std::fs::write(&manifest_path, encoded_manifest)
            .expect("write managed bridge plugin manifest");
    }

    #[cfg(unix)]
    struct BrowserCompanionEnvGuard {
        _lock: MutexGuard<'static, ()>,
        saved_ready: Option<OsString>,
    }

    struct PermissionRestore {
        path: PathBuf,
        permissions: Permissions,
    }

    impl PermissionRestore {
        fn new(path: PathBuf, permissions: Permissions) -> Self {
            Self { path, permissions }
        }
    }

    impl Drop for PermissionRestore {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.path, self.permissions.clone());
        }
    }

    fn set_browser_companion_env_var(key: &str, value: &str) {
        // SAFETY: daemon tests serialize process env mutations behind
        // `lock_daemon_test_environment`, so no concurrent env readers/writers
        // observe racy updates while these tests run.
        #[allow(unsafe_code, clippy::disallowed_methods)]
        unsafe {
            std::env::set_var(key, value);
        }
    }

    #[cfg(unix)]
    fn remove_browser_companion_env_var(key: &str) {
        // SAFETY: daemon tests serialize process env mutations behind
        // `lock_daemon_test_environment`, so removing the variable here is
        // coordinated with all other env-mutating daemon tests.
        #[allow(unsafe_code, clippy::disallowed_methods)]
        unsafe {
            std::env::remove_var(key);
        }
    }

    #[cfg(unix)]
    impl BrowserCompanionEnvGuard {
        fn runtime_gate_closed() -> Self {
            Self::set_ready(None)
        }

        fn runtime_gate_open() -> Self {
            Self::set_ready(Some("true"))
        }

        fn set_ready(value: Option<&str>) -> Self {
            let lock = crate::test_support::lock_daemon_test_environment();
            let key = "LOONG_BROWSER_COMPANION_READY";
            let saved_ready = std::env::var_os(key);
            match value {
                Some(value) => set_browser_companion_env_var(key, value),
                None => remove_browser_companion_env_var(key),
            }
            Self {
                _lock: lock,
                saved_ready,
            }
        }
    }

    #[cfg(unix)]
    impl Drop for BrowserCompanionEnvGuard {
        fn drop(&mut self) {
            let key = "LOONG_BROWSER_COMPANION_READY";
            match self.saved_ready.take() {
                Some(value) => set_browser_companion_env_var(key, &value.to_string_lossy()),
                None => remove_browser_companion_env_var(key),
            }
        }
    }

    #[cfg(unix)]
    fn rustc_version_probe() -> (String, String, String, String) {
        let output = std::process::Command::new("rustc")
            .arg("--version")
            .output()
            .expect("run rustc --version");
        let observed_version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let exact_version = observed_version
            .split_whitespace()
            .nth(1)
            .expect("rustc --version should include a semantic version")
            .to_owned();
        let version_components = exact_version.split('.').collect::<Vec<_>>();
        let partial_version =
            version_components[..version_components.len().saturating_sub(1)].join(".");

        (
            "rustc".to_owned(),
            observed_version,
            exact_version,
            partial_version,
        )
    }

    #[test]
    fn resolve_secret_prefers_inline_value() {
        let resolved = resolve_secret_value(Some(" inline-key "), Some("SHOULD_NOT_BE_USED"));
        assert_eq!(resolved.as_deref(), Some("inline-key"));
    }

    #[test]
    fn resolve_secret_reads_env_value() {
        let resolved = resolve_secret_value(None, Some("PATH"));
        assert!(resolved.is_some());
    }

    #[test]
    fn ensure_env_binding_fills_empty_slot() {
        let mut slot = None;
        let mut fixes = Vec::new();
        let changed = ensure_env_binding(&mut slot, "OPENAI_API_KEY", &mut fixes, "set provider");
        assert!(changed);
        assert_eq!(slot.as_deref(), Some("OPENAI_API_KEY"));
        assert_eq!(fixes.len(), 1);
    }

    #[test]
    fn check_channel_surfaces_omit_disabled_channels() {
        let config = mvp::config::LoongConfig::default();
        let checks = check_channel_surfaces(&config);
        assert!(
            checks.is_empty(),
            "disabled optional channels should not generate doctor warnings by default: {checks:#?}"
        );
    }

    #[test]
    fn build_channel_surface_checks_omit_disabled_registry_operations() {
        let snapshots = vec![ChannelStatusSnapshot {
            id: "telegram",
            configured_account_id: "ops".to_owned(),
            configured_account_label: "ops".to_owned(),
            is_default_account: true,
            default_account_source:
                mvp::config::ChannelDefaultAccountSelectionSource::ExplicitDefault,
            label: "Telegram",
            aliases: Vec::new(),
            transport: "telegram_bot_api",
            compiled: true,
            enabled: false,
            api_base_url: Some("https://api.telegram.org".to_owned()),
            notes: Vec::new(),
            reserved_runtime_fields: Vec::new(),
            operations: vec![ChannelOperationStatus {
                id: "serve",
                label: "event listener",
                command: "telegram-serve",
                health: ChannelOperationHealth::Disabled,
                detail: "disabled by telegram account configuration".to_owned(),
                issues: Vec::new(),
                runtime: None,
            }],
        }];

        let checks = build_channel_surface_checks(&snapshots);

        assert!(
            checks.is_empty(),
            "disabled registry-backed operations should not emit live doctor checks: {checks:#?}"
        );
    }

    #[test]
    fn build_channel_surface_checks_reports_plugin_bridge_contract_status_for_configured_surface() {
        let config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
            "weixin": {
                "enabled": true,
                "bridge_url": "https://bridge.example.test/weixin",
                "bridge_access_token": "weixin-token",
                "allowed_contact_ids": ["wxid_alice"]
            },
            "qqbot": {
                "enabled": true,
                "app_id": "10001",
                "client_secret": "qqbot-secret",
                "allowed_peer_ids": ["openid-alice"]
            },
            "onebot": {
                "enabled": true,
                "websocket_url": "ws://127.0.0.1:5700",
                "access_token": "onebot-token",
                "allowed_group_ids": ["123456"]
            }
        }))
        .expect("deserialize bridge-backed config");

        let checks = check_channel_surfaces(&config);

        assert!(checks.iter().any(|check| {
            check.name == "weixin bridge send contract" && check.level == DoctorCheckLevel::Pass
        }));
        assert!(checks.iter().any(|check| {
            check.name == "weixin bridge serve contract" && check.level == DoctorCheckLevel::Pass
        }));
        assert!(checks.iter().any(|check| {
            check.name == "qqbot bridge send contract" && check.level == DoctorCheckLevel::Pass
        }));
        assert!(checks.iter().any(|check| {
            check.name == "qqbot bridge serve contract" && check.level == DoctorCheckLevel::Pass
        }));
        assert!(checks.iter().any(|check| {
            check.name == "onebot bridge send contract" && check.level == DoctorCheckLevel::Pass
        }));
        assert!(checks.iter().any(|check| {
            check.name == "onebot bridge serve contract" && check.level == DoctorCheckLevel::Pass
        }));
    }

    #[test]
    fn check_channel_surfaces_reports_managed_bridge_discovery_for_compatible_plugins() {
        let install_root = browser_companion_temp_dir("managed-bridge-compatible");
        let manifest = managed_bridge_manifest(
            "weixin",
            Some("channel"),
            compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        );
        let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
            "weixin": {
                "enabled": true,
                "bridge_url": "https://bridge.example.test/weixin",
                "bridge_access_token": "weixin-token",
                "allowed_contact_ids": ["wxid_alice"]
            }
        }))
        .expect("deserialize weixin config");

        write_managed_bridge_manifest(install_root.as_path(), "weixin-managed-bridge", &manifest);

        config.external_skills.install_root = Some(install_root.display().to_string());

        let checks = check_channel_surfaces(&config);

        assert!(checks.iter().any(|check| {
            check.name == "weixin bridge send contract" && check.level == DoctorCheckLevel::Pass
        }));
        assert!(checks.iter().any(|check| {
            check.name == "weixin managed bridge discovery"
                && check.level == DoctorCheckLevel::Pass
                && check.detail.contains("compatible=1")
                && check.detail.contains("weixin-managed-bridge")
        }));
    }

    #[test]
    fn check_channel_surfaces_warns_when_managed_bridge_discovery_is_ambiguous() {
        let install_root = browser_companion_temp_dir("managed-bridge-ambiguous");
        let first_plugin_directory = "weixin-bridge-a";
        let second_plugin_directory = "weixin-bridge-b";
        let mut first_manifest = managed_bridge_manifest(
            "weixin",
            Some("channel"),
            compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        );
        let mut second_manifest = managed_bridge_manifest(
            "weixin",
            Some("channel"),
            compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        );
        let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
            "weixin": {
                "enabled": true,
                "bridge_url": "https://bridge.example.test/weixin",
                "bridge_access_token": "weixin-token",
                "allowed_contact_ids": ["wxid_alice"]
            }
        }))
        .expect("deserialize weixin config");

        first_manifest.plugin_id = "weixin-bridge-shared".to_owned();
        second_manifest.plugin_id = "weixin-bridge-shared".to_owned();

        write_managed_bridge_manifest(
            install_root.as_path(),
            first_plugin_directory,
            &first_manifest,
        );
        write_managed_bridge_manifest(
            install_root.as_path(),
            second_plugin_directory,
            &second_manifest,
        );

        config.external_skills.install_root = Some(install_root.display().to_string());

        let checks = check_channel_surfaces(&config);

        assert!(checks.iter().any(|check| {
            check.name == "weixin managed bridge discovery"
                && check.level == DoctorCheckLevel::Warn
                && check
                    .detail
                    .contains("ambiguity_status=duplicate_compatible_plugin_ids")
                && check
                    .detail
                    .contains("compatible_plugin_ids=weixin-bridge-shared,weixin-bridge-shared")
                && check.detail.contains("package_root=")
                && check.detail.contains(first_plugin_directory)
                && check.detail.contains(second_plugin_directory)
        }));
    }

    #[test]
    fn check_channel_surfaces_warns_when_configured_managed_bridge_plugin_id_is_duplicated() {
        let install_root = browser_companion_temp_dir("managed-bridge-selection-duplicated");
        let mut first_manifest = managed_bridge_manifest(
            "weixin",
            Some("channel"),
            compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        );
        let mut second_manifest = managed_bridge_manifest(
            "weixin",
            Some("channel"),
            compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        );
        let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
            "weixin": {
                "enabled": true,
                "managed_bridge_plugin_id": "weixin-bridge-shared",
                "bridge_url": "https://bridge.example.test/weixin",
                "bridge_access_token": "weixin-token",
                "allowed_contact_ids": ["wxid_alice"]
            }
        }))
        .expect("deserialize weixin config");

        first_manifest.plugin_id = "weixin-bridge-shared".to_owned();
        second_manifest.plugin_id = "weixin-bridge-shared".to_owned();

        write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-a", &first_manifest);
        write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-b", &second_manifest);
        config.external_skills.install_root = Some(install_root.display().to_string());

        let checks = check_channel_surfaces(&config);

        assert!(checks.iter().any(|check| {
            check.name == "weixin managed bridge discovery"
                && check.level == DoctorCheckLevel::Warn
                && check
                    .detail
                    .contains("configured_plugin_id=weixin-bridge-shared")
                && check
                    .detail
                    .contains("selection_status=configured_plugin_id_duplicated")
        }));
    }

    #[test]
    fn check_channel_surfaces_warns_when_managed_bridge_discovery_only_finds_incomplete_plugins() {
        let install_root = browser_companion_temp_dir("managed-bridge-incomplete");
        let mut metadata = compatible_managed_bridge_metadata(
            "qq_official_bot_gateway_or_plugin_bridge",
            "qqbot_reply_loop",
        );
        let removed_transport_family = metadata.remove("transport_family");
        let manifest = managed_bridge_manifest("qqbot", Some("channel"), metadata);
        let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
            "qqbot": {
                "enabled": true,
                "app_id": "10001",
                "client_secret": "qqbot-secret",
                "allowed_peer_ids": ["openid-alice"]
            }
        }))
        .expect("deserialize qqbot config");

        assert_eq!(
            removed_transport_family.as_deref(),
            Some("qq_official_bot_gateway_or_plugin_bridge")
        );

        write_managed_bridge_manifest(install_root.as_path(), "qqbot-incomplete-bridge", &manifest);

        config.external_skills.install_root = Some(install_root.display().to_string());

        let checks = check_channel_surfaces(&config);

        assert!(checks.iter().any(|check| {
            check.name == "qqbot bridge serve contract" && check.level == DoctorCheckLevel::Pass
        }));
        assert!(checks.iter().any(|check| {
            check.name == "qqbot managed bridge discovery"
                && check.level == DoctorCheckLevel::Warn
                && check.detail.contains("incomplete=1")
                && check
                    .detail
                    .contains("missing_fields=metadata.transport_family")
        }));
    }

    #[test]
    fn check_channel_surfaces_detail_includes_managed_bridge_setup_guidance() {
        let install_root = browser_companion_temp_dir("managed-bridge-setup-guidance");
        let mut metadata = compatible_managed_bridge_metadata(
            "qq_official_bot_gateway_or_plugin_bridge",
            "qqbot_reply_loop",
        );
        let removed_transport_family = metadata.remove("transport_family");
        let setup = managed_bridge_setup_with_guidance(
            "channel",
            vec!["QQBOT_BRIDGE_URL"],
            vec!["qqbot.bridge_url"],
            vec!["https://example.test/docs/qqbot-bridge"],
            Some(
                "Run the QQ bridge setup flow before enabling this bridge.\nThen confirm exactly one managed bridge remains.",
            ),
        );
        let mut manifest = managed_bridge_manifest_with_setup("qqbot", metadata, Some(setup));
        let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
            "qqbot": {
                "enabled": true,
                "app_id": "10001",
                "client_secret": "qqbot-secret",
                "allowed_peer_ids": ["openid-alice"]
            }
        }))
        .expect("deserialize qqbot config");

        manifest.plugin_id = "qqbot-bridge-guided".to_owned();
        assert_eq!(
            removed_transport_family.as_deref(),
            Some("qq_official_bot_gateway_or_plugin_bridge")
        );

        write_managed_bridge_manifest(install_root.as_path(), "qqbot-bridge-guided", &manifest);
        config.external_skills.install_root = Some(install_root.display().to_string());

        let checks = check_channel_surfaces(&config);

        assert!(checks.iter().any(|check| {
            check.name == "qqbot managed bridge discovery"
                && check.level == DoctorCheckLevel::Warn
                && check.detail.contains("required_env_vars=QQBOT_BRIDGE_URL")
                && check
                    .detail
                    .contains("required_config_keys=qqbot.bridge_url")
                && check
                    .detail
                    .contains("setup_docs_urls=https://example.test/docs/qqbot-bridge")
                && check.detail.contains(
                    "setup_remediation=\"Run the QQ bridge setup flow before enabling this bridge.\\nThen confirm exactly one managed bridge remains.\"",
                )
        }));
    }

    #[test]
    fn managed_plugin_bridge_discovery_detail_escapes_untrusted_values() {
        let discovery = mvp::channel::ChannelPluginBridgeDiscovery {
            managed_install_root: Some("/tmp/managed bridge".to_owned()),
            status: mvp::channel::ChannelPluginBridgeDiscoveryStatus::MatchesFound,
            scan_issue: Some("scan failed\nplease inspect".to_owned()),
            configured_plugin_id: Some("bridge\none".to_owned()),
            selected_plugin_id: Some("bridge\none".to_owned()),
            selection_status: Some(
                mvp::channel::ChannelPluginBridgeSelectionStatus::SelectedCompatible,
            ),
            ambiguity_status: Some(
                mvp::channel::ChannelPluginBridgeDiscoveryAmbiguityStatus::MultipleCompatiblePlugins,
            ),
            compatible_plugins: 1,
            compatible_plugin_ids: vec!["bridge\none".to_owned()],
            incomplete_plugins: 1,
            incompatible_plugins: 0,
            plugins: vec![mvp::channel::ChannelDiscoveredPluginBridge {
                plugin_id: "qqbot bridge".to_owned(),
                source_path: "/tmp/plugin root/bridge\nplugin.json".to_owned(),
                package_root: "/tmp/plugin root".to_owned(),
                package_manifest_path: Some("/tmp/plugin root/manifest\tbridge.json".to_owned()),
                bridge_kind: "managed connector".to_owned(),
                adapter_family: "channel bridge".to_owned(),
                transport_family: Some("qq official".to_owned()),
                target_contract: Some("qqbot\nreply".to_owned()),
                account_scope: Some("shared scope".to_owned()),
                runtime_contract: Some("loong_channel_bridge_v1".to_owned()),
                runtime_operations: vec![
                    "send_message".to_owned(),
                    "receive_batch".to_owned(),
                ],
                status: mvp::channel::ChannelDiscoveredPluginBridgeStatus::CompatibleIncompleteContract,
                issues: vec!["missing\nfield".to_owned()],
                missing_fields: vec!["metadata.transport family".to_owned()],
                required_env_vars: vec!["QQBOT BRIDGE URL".to_owned()],
                recommended_env_vars: vec!["QQBOT BRIDGE TOKEN".to_owned()],
                required_config_keys: vec!["qqbot.bridge url".to_owned()],
                default_env_var: Some("QQBOT DEFAULT ENV".to_owned()),
                setup_docs_urls: vec!["https://example.test/docs bridge".to_owned()],
                setup_remediation: Some("fix bridge\nthen retry".to_owned()),
            }],
        };

        let surface = mvp::channel::ChannelSurface {
            catalog: mvp::channel::ChannelCatalogEntry {
                id: "qqbot",
                label: "QQBot",
                selection_order: 0,
                selection_label: "QQBot",
                blurb: "plugin bridge",
                aliases: Vec::new(),
                transport: "plugin_bridge",
                implementation_status:
                    mvp::channel::ChannelCatalogImplementationStatus::PluginBacked,
                capabilities: Vec::new(),
                operations: Vec::new(),
                onboarding: mvp::channel::ChannelOnboardingDescriptor {
                    strategy: mvp::channel::ChannelOnboardingStrategy::PluginBridge,
                    setup_hint: "plugin bridge",
                    status_command: "loong doctor",
                    repair_command: None,
                },
                supported_target_kinds: Vec::new(),
                plugin_bridge_contract: Some(mvp::channel::ChannelPluginBridgeContract {
                    manifest_channel_id: "qqbot",
                    required_setup_surface: "channel",
                    runtime_owner: "external_plugin",
                    supported_operations: Vec::new(),
                    recommended_metadata_keys: Vec::new(),
                    stable_targets: Vec::new(),
                    account_scope_note: None,
                }),
            },
            configured_accounts: Vec::new(),
            default_configured_account_id: None,
            plugin_bridge_discovery: Some(discovery.clone()),
        };
        let detail = managed_plugin_bridge_discovery_check_detail(&surface, &discovery);

        assert!(detail.contains("root=\"/tmp/managed bridge\""));
        assert!(detail.contains("compatible_plugin_ids=\"bridge\\none\""));
        assert!(detail.contains("\"qqbot bridge\""));
        assert!(detail.contains("target_contract=\"qqbot\\nreply\""));
        assert!(detail.contains("setup_docs_urls=\"https://example.test/docs bridge\""));
        assert!(detail.contains("setup_remediation=\"fix bridge\\nthen retry\""));
    }

    #[test]
    fn managed_bridge_incomplete_setup_step_escapes_untrusted_values() {
        let config = mvp::config::LoongConfig::default();
        let inventory = mvp::channel::channel_inventory(&config);
        let surface = inventory
            .channel_surfaces
            .iter()
            .find(|surface| surface.catalog.id == "weixin")
            .expect("weixin surface");
        let plugin = mvp::channel::ChannelDiscoveredPluginBridge {
            plugin_id: "weixin bridge".to_owned(),
            source_path: "/tmp/plugin root/bridge\nplugin.json".to_owned(),
            package_root: "/tmp/plugin root".to_owned(),
            package_manifest_path: Some("/tmp/plugin root/manifest bridge.json".to_owned()),
            bridge_kind: "managed connector".to_owned(),
            adapter_family: "channel bridge".to_owned(),
            transport_family: Some("wechat clawbot".to_owned()),
            target_contract: Some("weixin reply".to_owned()),
            account_scope: Some("shared scope".to_owned()),
            runtime_contract: Some("loong_channel_bridge_v1".to_owned()),
            runtime_operations: vec!["send_message".to_owned(), "receive_batch".to_owned()],
            status: mvp::channel::ChannelDiscoveredPluginBridgeStatus::CompatibleIncompleteContract,
            issues: vec!["missing\nfield".to_owned()],
            missing_fields: vec!["metadata.transport family".to_owned()],
            required_env_vars: vec!["WEIXIN BRIDGE URL".to_owned()],
            recommended_env_vars: vec!["WEIXIN BRIDGE TOKEN".to_owned()],
            required_config_keys: vec!["weixin.bridge url".to_owned()],
            default_env_var: Some("WEIXIN DEFAULT ENV".to_owned()),
            setup_docs_urls: vec!["https://example.test/docs bridge".to_owned()],
            setup_remediation: Some("fix bridge\nthen retry".to_owned()),
        };
        let duplicate_plugin_id_counts =
            managed_bridge_duplicate_plugin_id_counts(std::slice::from_ref(&plugin));
        let step =
            managed_bridge_incomplete_setup_step(surface, &plugin, &duplicate_plugin_id_counts);

        assert!(step.contains("plugin \"weixin bridge\""));
        assert!(step.contains("required env: \"WEIXIN BRIDGE URL\""));
        assert!(step.contains("required config keys: \"weixin.bridge url\""));
        assert!(step.contains("docs: \"https://example.test/docs bridge\""));
        assert!(step.contains("remediation: \"fix bridge\\nthen retry\""));
    }

    #[test]
    fn build_channel_surface_checks_fails_plugin_bridge_contract_when_serve_requirements_are_missing()
     {
        let config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
            "qqbot": {
                "enabled": true,
                "app_id": "10001",
                "client_secret": "qqbot-secret"
            }
        }))
        .expect("deserialize qqbot config");

        let checks = check_channel_surfaces(&config);

        assert!(checks.iter().any(|check| {
            check.name == "qqbot bridge send contract" && check.level == DoctorCheckLevel::Pass
        }));
        assert!(checks.iter().any(|check| {
            check.name == "qqbot bridge serve contract"
                && check.level == DoctorCheckLevel::Fail
                && check.detail.contains("allowed_peer_ids is empty")
        }));
    }

    #[test]
    fn build_channel_surface_checks_fails_plugin_bridge_contract_when_surface_is_uncompiled() {
        let snapshots = vec![ChannelStatusSnapshot {
            id: "weixin",
            configured_account_id: "default".to_owned(),
            configured_account_label: "default".to_owned(),
            is_default_account: true,
            default_account_source:
                mvp::config::ChannelDefaultAccountSelectionSource::ExplicitDefault,
            label: "Weixin",
            aliases: vec!["wechat", "wx"],
            transport: "wechat_clawbot_ilink_bridge",
            compiled: false,
            enabled: true,
            api_base_url: None,
            notes: vec!["bridge_runtime_owner=external_plugin".to_owned()],
            reserved_runtime_fields: Vec::new(),
            operations: vec![ChannelOperationStatus {
                id: "send",
                label: "bridge send",
                command: "weixin-send",
                health: ChannelOperationHealth::Unsupported,
                detail: "weixin bridge surface is unavailable in this build".to_owned(),
                issues: vec!["weixin bridge surface is unavailable in this build".to_owned()],
                runtime: None,
            }],
        }];

        let checks = build_channel_surface_checks(&snapshots);

        assert!(checks.iter().any(|check| {
            check.name == "weixin bridge send contract"
                && check.level == DoctorCheckLevel::Fail
                && check.detail.contains("unavailable in this build")
        }));
    }

    #[test]
    fn channel_doctor_checks_report_enabled_channels_from_registry() {
        let mut config = mvp::config::LoongConfig::default();
        config.telegram.enabled = true;
        config.telegram.bot_token = Some(SecretRef::Inline("123456:test-token".to_owned()));
        config.telegram.allowed_chat_ids = vec![123_i64];
        config.feishu.enabled = true;
        config.feishu.app_id = Some(SecretRef::Inline("cli_a1b2c3".to_owned()));
        config.feishu.app_secret = Some(SecretRef::Inline("feishu-secret".to_owned()));
        config.matrix.enabled = true;
        config.matrix.access_token = Some(SecretRef::Inline("matrix-token".to_owned()));
        config.matrix.base_url = Some("https://matrix.example.org".to_owned());
        config.matrix.allowed_room_ids = vec!["!ops:example.org".to_owned()];
        config.matrix.user_id = Some("@ops-bot:example.org".to_owned());

        let checks = check_channel_surfaces(&config);
        let names = checks
            .iter()
            .map(|check| check.name.as_str())
            .collect::<Vec<_>>();

        assert!(
            names.contains(&"telegram channel"),
            "telegram send/serve surfaces should appear in live doctor output: {checks:#?}"
        );
        assert!(
            names.contains(&"telegram channel runtime"),
            "ready telegram serve surfaces should emit runtime checks in live doctor output: {checks:#?}"
        );
        assert!(
            names.contains(&"matrix channel") && names.contains(&"matrix room sync"),
            "matrix send/serve surfaces should appear in live doctor output: {checks:#?}"
        );
        assert!(
            names.contains(&"matrix channel runtime"),
            "ready matrix serve surfaces should emit runtime checks in live doctor output: {checks:#?}"
        );
        assert!(
            names.contains(&"feishu channel") && names.contains(&"feishu inbound transport"),
            "feishu send/serve surfaces should appear in live doctor output: {checks:#?}"
        );
        assert!(
            checks
                .iter()
                .any(|check| check.name == "matrix room sync"
                    && check.level == DoctorCheckLevel::Pass),
            "matrix serve configuration should stay healthy through the live doctor path: {checks:#?}"
        );
    }

    #[test]
    fn channel_env_fix_uses_registered_channel_defaults() {
        let mut config = mvp::config::LoongConfig::default();
        config.telegram.bot_token_env = None;
        config.feishu.app_id_env = None;
        config.feishu.app_secret_env = None;
        config.feishu.verification_token_env = None;
        config.feishu.encrypt_key_env = None;
        config.matrix.access_token_env = None;

        let mut fixes = Vec::new();
        let changed = maybe_apply_channel_env_fix(&mut config, true, &mut fixes);

        assert!(changed);
        assert_eq!(
            config.telegram.bot_token_env.as_deref(),
            Some("TELEGRAM_BOT_TOKEN")
        );
        assert_eq!(config.feishu.app_id_env.as_deref(), Some("FEISHU_APP_ID"));
        assert_eq!(
            config.feishu.app_secret_env.as_deref(),
            Some("FEISHU_APP_SECRET")
        );
        assert!(
            config.feishu.verification_token_env.is_none(),
            "default feishu mode is websocket; doctor env fix must not set webhook verification_token_env"
        );
        assert!(
            config.feishu.encrypt_key_env.is_none(),
            "default feishu mode is websocket; doctor env fix must not set webhook encrypt_key_env"
        );
        assert_eq!(
            config.matrix.access_token_env.as_deref(),
            Some("MATRIX_ACCESS_TOKEN")
        );
        assert_eq!(fixes.len(), 4);
    }

    #[test]
    fn provider_credential_env_hints_prioritize_oauth_defaults() {
        let provider = mvp::config::ProviderConfig::default();
        let hints = provider_credential_policy::provider_credential_env_hints(&provider);

        assert!(
            hints.contains(&"OPENAI_CODEX_OAUTH_TOKEN".to_owned()),
            "doctor hints should include the provider's oauth default when available: {hints:?}"
        );
        assert!(
            hints.contains(&"OPENAI_API_KEY".to_owned()),
            "doctor hints should still include the api key fallback for providers that support both auth paths: {hints:?}"
        );
    }

    #[test]
    fn provider_env_fix_prefers_oauth_default_when_available() {
        let mut config = mvp::config::LoongConfig::default();
        config.provider.api_key_env = None;
        config.provider.oauth_access_token_env = None;

        let mut fixes = Vec::new();
        let changed = maybe_apply_provider_env_fix(&mut config, true, &mut fixes);

        assert!(changed);
        assert_eq!(
            config.provider.oauth_access_token,
            Some(loong_contracts::SecretRef::Env {
                env: "OPENAI_CODEX_OAUTH_TOKEN".to_owned(),
            })
        );
        assert_eq!(config.provider.api_key_env, None);
        assert_eq!(
            fixes,
            vec!["set provider.oauth_access_token.env=OPENAI_CODEX_OAUTH_TOKEN".to_owned()]
        );
    }

    #[test]
    fn provider_env_fix_does_not_overwrite_inline_api_key() {
        let mut config = mvp::config::LoongConfig::default();
        config.provider.api_key = Some(loong_contracts::SecretRef::Inline(
            "inline-secret".to_owned(),
        ));
        config.provider.api_key_env = None;
        config.provider.oauth_access_token = None;
        config.provider.oauth_access_token_env = None;

        let mut fixes = Vec::new();
        let changed = maybe_apply_provider_env_fix(&mut config, true, &mut fixes);

        assert!(!changed);
        assert_eq!(
            config.provider.api_key,
            Some(loong_contracts::SecretRef::Inline(
                "inline-secret".to_owned(),
            ))
        );
        assert_eq!(config.provider.api_key_env, None);
        assert!(fixes.is_empty());
    }

    #[test]
    fn provider_env_fix_does_not_overwrite_file_backed_api_key() {
        let mut config = mvp::config::LoongConfig::default();
        let credential_path = PathBuf::from("/tmp/openai-api-key.txt");
        config.provider.api_key = Some(loong_contracts::SecretRef::File {
            file: credential_path.clone(),
        });
        config.provider.api_key_env = None;
        config.provider.oauth_access_token = None;
        config.provider.oauth_access_token_env = None;

        let mut fixes = Vec::new();
        let changed = maybe_apply_provider_env_fix(&mut config, true, &mut fixes);

        assert!(!changed);
        assert_eq!(
            config.provider.api_key,
            Some(loong_contracts::SecretRef::File {
                file: credential_path,
            })
        );
        assert_eq!(config.provider.oauth_access_token, None);
        assert_eq!(config.provider.api_key_env, None);
        assert_eq!(config.provider.oauth_access_token_env, None);
        assert!(fixes.is_empty());
    }

    #[test]
    fn provider_transport_doctor_check_warns_for_responses_compatibility_mode() {
        let provider = mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::Deepseek,
            model: "deepseek-chat".to_owned(),
            wire_api: mvp::config::ProviderWireApi::Responses,
            ..mvp::config::ProviderConfig::default()
        };

        let check = provider_transport_doctor_check(&provider);

        assert_eq!(check.name, "provider transport");
        assert_eq!(check.level, DoctorCheckLevel::Warn);
        assert!(
            check
                .detail
                .contains("retry chat_completions automatically"),
            "doctor should surface the automatic transport fallback in review mode: {check:#?}"
        );
    }

    #[test]
    fn provider_model_probe_failure_warns_for_explicit_model() {
        let mut config = mvp::config::LoongConfig::default();
        config.provider.model = "openai/gpt-5.1-codex".to_owned();

        let check = provider_model_probe_failure_check(
            &config,
            "provider rejected the model list".to_owned(),
        );

        assert_eq!(check.name, "provider model probe");
        assert_eq!(check.level, DoctorCheckLevel::Warn);
        assert!(
            check.detail.contains("explicitly configured"),
            "doctor should explain that explicit-model runtime may still work when catalog probing fails: {check:#?}"
        );
    }

    #[test]
    fn provider_model_probe_transport_failure_prioritizes_route_guidance() {
        let mut config = mvp::config::LoongConfig::default();
        config.provider.model = "custom-explicit-model".to_owned();

        let check = provider_model_probe_failure_check(
            &config,
            "provider model-list request failed on attempt 3/3: operation timed out".to_owned(),
        );

        assert_eq!(check.name, "provider model probe");
        assert_eq!(check.level, DoctorCheckLevel::Fail);
        assert!(
            check
                .detail
                .contains(crate::provider_route_diagnostics::MODEL_CATALOG_TRANSPORT_FAILED_MARKER),
            "transport probe failures should use the route-focused marker: {check:#?}"
        );
        assert!(
            !check.detail.contains("provider.model"),
            "transport probe failures should not suggest model-selection repair when the route is the real blocker: {check:#?}"
        );
        assert!(
            !check.detail.contains("below"),
            "doctor should not promise a later route-probe section that may not exist when collection is unavailable: {check:#?}"
        );
    }

    #[test]
    fn provider_model_probe_failure_fails_for_auto_model() {
        let mut config = mvp::config::LoongConfig::default();
        config.provider.model = "auto".to_owned();

        let check = provider_model_probe_failure_check(
            &config,
            "provider rejected the model list".to_owned(),
        );

        assert_eq!(check.name, "provider model probe");
        assert_eq!(check.level, DoctorCheckLevel::Fail);
        assert!(
            check.detail.contains("OpenAI [openai]"),
            "doctor failures should still identify the active provider context: {check:#?}"
        );
        assert!(
            check.detail.contains("model = auto"),
            "doctor failures should explain why runtime cannot rely on an unresolved automatic model: {check:#?}"
        );
        assert!(
            check.detail.contains("provider.model"),
            "doctor failures should point users to an explicit provider.model remediation path: {check:#?}"
        );
        assert!(
            check.detail.contains("preferred_models"),
            "doctor failures should point users to preferred_models when catalog probing is unavailable: {check:#?}"
        );
    }

    #[test]
    fn provider_model_probe_failure_warns_for_preferred_model_fallbacks() {
        let mut config = mvp::config::LoongConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Minimax;
        config.provider.model = "auto".to_owned();
        config.provider.preferred_models = vec!["MiniMax-M2.5".to_owned()];

        let check = provider_model_probe_failure_check(
            &config,
            "provider rejected the model list".to_owned(),
        );

        assert_eq!(check.name, "provider model probe");
        assert_eq!(check.level, DoctorCheckLevel::Warn);
        assert!(
            check.detail.contains("configured preferred"),
            "doctor should only advertise fallback continuation for explicitly configured preferred models: {check:#?}"
        );
        assert!(
            check.detail.contains("MiniMax-M2.5"),
            "doctor warning should surface the fallback candidate to keep remediation concrete: {check:#?}"
        );
    }

    #[test]
    fn provider_model_probe_failure_guides_reviewed_default_for_auto_model() {
        let mut config = mvp::config::LoongConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "auto".to_owned();

        let check = provider_model_probe_failure_check(
            &config,
            "provider rejected the model list".to_owned(),
        );

        assert_eq!(check.name, "provider model probe");
        assert_eq!(check.level, DoctorCheckLevel::Fail);
        assert!(
            check.detail.contains("deepseek-chat"),
            "reviewed providers should point users to the reviewed onboarding default when doctor cannot list models: {check:#?}"
        );
        assert!(
            check.detail.contains("rerun onboarding"),
            "doctor should suggest rerunning onboarding to accept the reviewed model instead of leaving recovery implicit: {check:#?}"
        );
    }

    #[test]
    fn provider_model_probe_failure_includes_region_hint_for_zhipu() {
        let mut config = mvp::config::LoongConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Zhipu;
        config.provider.model = "auto".to_owned();

        let check =
            provider_model_probe_failure_check(&config, "provider returned status 401".to_owned());

        assert_eq!(check.name, "provider model probe");
        assert_eq!(check.level, DoctorCheckLevel::Fail);
        assert!(
            check.detail.contains("https://api.z.ai"),
            "doctor probe failures should surface the alternate regional endpoint when auth can be region-bound: {check:#?}"
        );
    }

    #[test]
    fn provider_model_probe_failure_skips_region_hint_for_non_auth_errors() {
        let mut config = mvp::config::LoongConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Zhipu;
        config.provider.model = "auto".to_owned();

        let check =
            provider_model_probe_failure_check(&config, "provider returned status 503".to_owned());

        assert_eq!(check.name, "provider model probe");
        assert_eq!(check.level, DoctorCheckLevel::Fail);
        assert!(
            !check.detail.contains("provider.base_url"),
            "non-auth doctor probe failures should not steer operators toward region endpoint changes: {check:#?}"
        );
    }

    #[test]
    fn build_doctor_next_steps_includes_region_endpoint_step_for_minimax_probe_failures() {
        let mut config = mvp::config::LoongConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Minimax;
        let checks = vec![
            DoctorCheck {
                name: "provider credentials".to_owned(),
                level: DoctorCheckLevel::Pass,
                detail: "provider credentials are available".to_owned(),
            },
            DoctorCheck {
                name: "provider model probe".to_owned(),
                level: DoctorCheckLevel::Fail,
                detail:
                    "MiniMax [minimax]: model catalog probe failed (provider returned status 401)"
                        .to_owned(),
            },
        ];

        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            Path::new("/tmp/loong.toml"),
            &config,
            false,
            Some(std::ffi::OsStr::new("")),
        );

        assert!(
            next_steps.iter().any(|step| {
                step.contains("provider.base_url")
                    && step.contains("https://api.minimax.io")
                    && step.contains("https://api.minimaxi.com")
            }),
            "doctor next steps should include a concrete region endpoint adjustment for MiniMax auth/probe failures: {next_steps:#?}"
        );
    }

    #[test]
    fn build_doctor_next_steps_skips_region_endpoint_step_for_non_auth_probe_failures() {
        let mut config = mvp::config::LoongConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Minimax;
        let checks = vec![
            DoctorCheck {
                name: "provider credentials".to_owned(),
                level: DoctorCheckLevel::Pass,
                detail: "provider credentials are available".to_owned(),
            },
            DoctorCheck {
                name: "provider model probe".to_owned(),
                level: DoctorCheckLevel::Fail,
                detail:
                    "MiniMax [minimax]: model catalog probe failed (provider returned status 503)"
                        .to_owned(),
            },
        ];

        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            Path::new("/tmp/loong.toml"),
            &config,
            false,
            Some(std::ffi::OsStr::new("")),
        );

        assert!(
            !next_steps
                .iter()
                .any(|step| step.contains("provider.base_url")),
            "doctor next steps should not include a region endpoint adjustment for non-auth probe failures: {next_steps:#?}"
        );
    }

    #[test]
    fn audit_retention_doctor_check_warns_for_in_memory_mode() {
        let check = audit_retention_doctor_check(&mvp::config::AuditConfig {
            mode: mvp::config::AuditMode::InMemory,
            ..mvp::config::AuditConfig::default()
        });

        assert_eq!(check.name, "audit retention");
        assert_eq!(check.level, DoctorCheckLevel::Warn);
        assert!(check.detail.contains("lost on restart"));
    }

    #[test]
    fn audit_integrity_doctor_check_warns_for_in_memory_mode() {
        let check = audit_integrity_doctor_check(&mvp::config::AuditConfig {
            mode: mvp::config::AuditMode::InMemory,
            ..mvp::config::AuditConfig::default()
        });

        assert_eq!(check.name, "audit integrity");
        assert_eq!(check.level, DoctorCheckLevel::Warn);
        assert!(
            check
                .detail
                .contains("unavailable while audit.mode=in_memory")
        );
    }

    #[test]
    fn audit_integrity_doctor_check_passes_for_valid_chain() {
        let temp_dir = browser_companion_temp_dir("audit-integrity-valid");
        let journal_path = temp_dir.join("events.jsonl");
        let sink = kernel::JsonlAuditSink::new(journal_path.clone()).expect("create jsonl sink");

        sink.record(sample_audit_event(
            "evt-integrity-1",
            1_700_010_400,
            Some("agent-a"),
            kernel::AuditEventKind::TokenRevoked {
                token_id: "token-a".to_owned(),
            },
        ))
        .expect("record event");

        let check = audit_integrity_doctor_check(&mvp::config::AuditConfig {
            mode: mvp::config::AuditMode::Jsonl,
            path: journal_path.display().to_string(),
            retain_in_memory: false,
        });

        assert_eq!(check.name, "audit integrity");
        assert_eq!(check.level, DoctorCheckLevel::Pass);
        assert!(check.detail.contains("verified 1 of 1 audit events"));
    }

    #[test]
    fn audit_integrity_doctor_check_fails_for_tampered_chain() {
        let temp_dir = browser_companion_temp_dir("audit-integrity-tampered");
        let journal_path = temp_dir.join("events.jsonl");
        let sink = kernel::JsonlAuditSink::new(journal_path.clone()).expect("create jsonl sink");

        sink.record(sample_audit_event(
            "evt-integrity-1",
            1_700_010_410,
            Some("agent-a"),
            kernel::AuditEventKind::TokenRevoked {
                token_id: "token-a".to_owned(),
            },
        ))
        .expect("record event");

        sink.record(sample_audit_event(
            "evt-integrity-2",
            1_700_010_411,
            Some("agent-b"),
            kernel::AuditEventKind::TokenRevoked {
                token_id: "token-b".to_owned(),
            },
        ))
        .expect("record event");

        let contents = std::fs::read_to_string(&journal_path).expect("read audit journal");
        let tampered = contents.replacen("token-b", "token-x", 1);
        std::fs::write(&journal_path, tampered).expect("rewrite tampered journal");

        let check = audit_integrity_doctor_check(&mvp::config::AuditConfig {
            mode: mvp::config::AuditMode::Jsonl,
            path: journal_path.display().to_string(),
            retain_in_memory: false,
        });

        assert_eq!(check.name, "audit integrity");
        assert_eq!(check.level, DoctorCheckLevel::Fail);
        assert!(check.detail.contains("failed at line 2"));
    }

    #[test]
    fn build_doctor_next_steps_guides_durable_audit_when_in_memory() {
        let checks = vec![DoctorCheck {
            name: "audit retention".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "audit.mode=in_memory; security-critical audit evidence is lost on restart"
                .to_owned(),
        }];
        let config_path = PathBuf::from("/tmp/loong.toml");
        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            &config_path,
            &mvp::config::LoongConfig::default(),
            false,
            None,
        );

        assert!(
            next_steps
                .iter()
                .any(|step| step
                    == "Switch to durable audit retention: set [audit].mode = \"fanout\""),
            "doctor should recommend durable audit retention when audit remains in-memory: {next_steps:#?}"
        );
    }

    #[test]
    fn build_doctor_next_steps_guides_fix_when_audit_path_is_invalid() {
        let checks = vec![DoctorCheck {
            name: "audit retention".to_owned(),
            level: DoctorCheckLevel::Fail,
            detail: "audit.mode=fanout -> /tmp/audit exists but is not a regular file".to_owned(),
        }];
        let config_path = PathBuf::from("/tmp/loong.toml");
        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            &config_path,
            &mvp::config::LoongConfig::default(),
            false,
            None,
        );

        assert!(
            next_steps
                .iter()
                .any(|step| step.contains("Point [audit].path at a writable journal file path")),
            "doctor should guide operators toward a writable audit journal target when durable audit retention is misconfigured: {next_steps:#?}"
        );
    }

    #[test]
    fn audit_journal_directory_check_accepts_bare_relative_filename() {
        let mut fixes = Vec::new();
        let audit_path = PathBuf::from("events.jsonl");
        let directory = audit_path.parent().unwrap_or(Path::new("."));
        let check = check_audit_journal_directory(directory, false, &mut fixes);

        assert_eq!(check.name, "audit journal directory");
        assert_eq!(check.level, DoctorCheckLevel::Pass);
        assert!(check.detail.contains("current working directory"));
        assert!(fixes.is_empty());
    }

    #[test]
    fn audit_retention_doctor_check_fails_when_durable_path_is_directory() {
        let temp_dir = browser_companion_temp_dir("audit-target-directory");
        let check = audit_retention_doctor_check(&mvp::config::AuditConfig {
            mode: mvp::config::AuditMode::Fanout,
            path: temp_dir.display().to_string(),
            retain_in_memory: true,
        });

        assert_eq!(check.name, "audit retention");
        assert_eq!(check.level, DoctorCheckLevel::Fail);
        assert!(check.detail.contains("not a regular file"));
    }

    #[test]
    fn audit_retention_doctor_check_fails_when_durable_path_is_readonly_file() {
        let temp_dir = browser_companion_temp_dir("audit-target-readonly");
        let journal_path = temp_dir.join("events.jsonl");
        std::fs::write(&journal_path, b"{}\n").expect("write audit journal fixture");
        let original_permissions = std::fs::metadata(&journal_path)
            .expect("audit journal metadata")
            .permissions();
        let mut permissions = original_permissions.clone();
        permissions.set_readonly(true);
        std::fs::set_permissions(&journal_path, permissions)
            .expect("mark audit journal fixture readonly");
        let _permission_restore =
            PermissionRestore::new(journal_path.clone(), original_permissions);

        let check = audit_retention_doctor_check(&mvp::config::AuditConfig {
            mode: mvp::config::AuditMode::Jsonl,
            path: journal_path.display().to_string(),
            retain_in_memory: false,
        });

        assert_eq!(check.name, "audit retention");
        assert_eq!(check.level, DoctorCheckLevel::Fail);
        assert!(check.detail.contains("not writable"));
    }

    #[test]
    fn audit_retention_doctor_check_fails_when_parent_path_is_not_a_directory() {
        let temp_dir = browser_companion_temp_dir("audit-target-parent-not-directory");
        let blocked_parent = temp_dir.join("readonly-audit");
        std::fs::write(&blocked_parent, b"not a directory").expect("create blocking parent file");

        let journal_path = blocked_parent.join("events.jsonl");
        let check = audit_retention_doctor_check(&mvp::config::AuditConfig {
            mode: mvp::config::AuditMode::Fanout,
            path: journal_path.display().to_string(),
            retain_in_memory: true,
        });

        assert_eq!(check.name, "audit retention");
        assert_eq!(check.level, DoctorCheckLevel::Fail);
        assert!(
            check
                .detail
                .contains(journal_path.display().to_string().as_str()),
            "expected failing detail to mention the blocked journal path, got: {}",
            check.detail
        );
    }

    #[test]
    fn audit_retention_doctor_check_fails_when_missing_parent_chain_runs_into_file_boundary() {
        let temp_dir = browser_companion_temp_dir("audit-target-missing-parent-chain");
        let blocked_parent = temp_dir.join("readonly-audit");
        std::fs::write(&blocked_parent, b"not a directory").expect("create blocking parent file");

        let journal_path = blocked_parent.join("nested").join("events.jsonl");
        let check = audit_retention_doctor_check(&mvp::config::AuditConfig {
            mode: mvp::config::AuditMode::Fanout,
            path: journal_path.display().to_string(),
            retain_in_memory: true,
        });

        assert_eq!(check.name, "audit retention");
        assert_eq!(check.level, DoctorCheckLevel::Fail);
        assert!(
            check
                .detail
                .contains(journal_path.display().to_string().as_str()),
            "expected failing detail to mention the blocked journal path, got: {}",
            check.detail
        );
    }

    #[test]
    fn audit_retention_doctor_check_cleans_up_probe_artifacts_for_creatable_missing_path() {
        let temp_dir = browser_companion_temp_dir("audit-target-cleanup");
        let journal_path = temp_dir.join("nested").join("events.jsonl");

        let check = audit_retention_doctor_check(&mvp::config::AuditConfig {
            mode: mvp::config::AuditMode::Fanout,
            path: journal_path.display().to_string(),
            retain_in_memory: true,
        });

        assert_eq!(check.name, "audit retention");
        assert_eq!(check.level, DoctorCheckLevel::Pass);
        assert!(!journal_path.exists());
        assert!(!journal_path.parent().expect("nested parent").exists());
    }

    fn unique_temp_feishu_db(label: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before epoch")
            .as_nanos();
        let sequence = FEISHU_TEST_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir()
            .join(format!(
                "loong-doctor-feishu-{label}-{}-{nanos}-{sequence}.sqlite3",
                std::process::id()
            ))
            .display()
            .to_string()
    }

    #[test]
    fn feishu_integration_requested_is_false_for_default_config() {
        let config = mvp::config::FeishuChannelConfig::default();
        assert!(!feishu_integration_requested(&config));
    }

    #[test]
    fn check_feishu_integration_warns_when_user_grants_are_missing() {
        let mut config = mvp::config::LoongConfig::default();
        config.feishu.enabled = true;
        config.feishu.app_id = Some(SecretRef::Inline("cli_a1b2c3".to_owned()));
        config.feishu.app_secret = Some(SecretRef::Inline("app-secret".to_owned()));
        config.feishu_integration.sqlite_path = unique_temp_feishu_db("missing-grant");
        let mut fixes = Vec::new();

        let checks = check_feishu_integration(&config, false, &mut fixes);

        assert!(
            checks.iter().any(|check| {
                check.name == "feishu integration credentials"
                    && check.level == DoctorCheckLevel::Pass
            }),
            "configured Feishu account should report available credentials"
        );
        assert!(
            checks.iter().any(|check| {
                check.level == DoctorCheckLevel::Warn
                    && check.name.contains("feishu user grant")
                    && check.detail.contains("missing stored user grant")
            }),
            "missing grants should warn rather than fail hard"
        );
    }

    #[test]
    fn check_feishu_integration_passes_when_ready_grant_exists() {
        let mut config = mvp::config::LoongConfig::default();
        config.feishu.enabled = true;
        config.feishu.app_id = Some(SecretRef::Inline("cli_a1b2c3".to_owned()));
        config.feishu.app_secret = Some(SecretRef::Inline("app-secret".to_owned()));
        config.feishu_integration.sqlite_path = unique_temp_feishu_db("ready-grant");
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve default feishu account");
        let store = mvp::channel::feishu::api::FeishuTokenStore::new(
            config.feishu_integration.resolved_sqlite_path(),
        );
        store
            .save_grant(&mvp::channel::feishu::api::FeishuGrant {
                principal: mvp::channel::feishu::api::FeishuUserPrincipal {
                    account_id: resolved.account.id,
                    open_id: "ou_123".to_owned(),
                    union_id: Some("on_456".to_owned()),
                    user_id: Some("u_789".to_owned()),
                    name: Some("Alice".to_owned()),
                    tenant_key: Some("tenant_x".to_owned()),
                    avatar_url: None,
                    email: Some("alice@example.com".to_owned()),
                    enterprise_email: None,
                },
                access_token: "u-token".to_owned(),
                refresh_token: "r-token".to_owned(),
                scopes: mvp::channel::feishu::api::FeishuGrantScopeSet::from_scopes(
                    config.feishu_integration.trimmed_default_scopes(),
                ),
                access_expires_at_s: chrono::Utc::now().timestamp() + 3600,
                refresh_expires_at_s: chrono::Utc::now().timestamp() + 86_400,
                refreshed_at_s: chrono::Utc::now().timestamp(),
            })
            .expect("save feishu grant");
        let mut fixes = Vec::new();

        let checks = check_feishu_integration(&config, false, &mut fixes);

        assert!(
            checks.iter().any(|check| {
                check.name.contains("feishu user grant")
                    && check.level == DoctorCheckLevel::Pass
                    && check.detail.contains("latest_open_id=ou_123")
            }),
            "stored grants should be visible to doctor"
        );
        assert!(
            checks.iter().any(|check| {
                check.name.contains("feishu token freshness")
                    && check.level == DoctorCheckLevel::Pass
            }),
            "ready grants should upgrade Feishu integration health to pass"
        );
    }

    #[test]
    fn build_channel_surface_checks_warns_when_ready_serve_operation_is_not_running() {
        let snapshots = vec![ChannelStatusSnapshot {
            id: "telegram",
            configured_account_id: "bot_123456".to_owned(),
            configured_account_label: "bot_123456".to_owned(),
            is_default_account: true,
            default_account_source:
                mvp::config::ChannelDefaultAccountSelectionSource::RuntimeIdentity,
            label: "Telegram",
            aliases: Vec::new(),
            transport: "telegram_bot_api_polling",
            compiled: true,
            enabled: true,
            api_base_url: Some("https://api.telegram.org".to_owned()),
            notes: Vec::new(),
            reserved_runtime_fields: Vec::new(),
            operations: vec![ChannelOperationStatus {
                id: "serve",
                label: "reply loop",
                command: "telegram-serve",
                health: ChannelOperationHealth::Ready,
                detail: "ready".to_owned(),
                issues: Vec::new(),
                runtime: Some(ChannelOperationRuntime {
                    running: false,
                    stale: false,
                    busy: false,
                    active_runs: 0,
                    last_run_activity_at: None,
                    last_heartbeat_at: None,
                    pid: None,
                    account_id: Some("bot_123456".to_owned()),
                    account_label: Some("bot:123456".to_owned()),
                    instance_count: 1,
                    running_instances: 0,
                    stale_instances: 0,
                }),
            }],
        }];

        let checks = build_channel_surface_checks(&snapshots);

        assert!(
            checks.iter().any(|check| {
                check.name == "telegram channel runtime"
                    && check.level == DoctorCheckLevel::Warn
                    && check.detail.contains("not currently running")
                    && check.detail.contains("account=bot:123456")
            }),
            "ready telegram serve operation should emit runtime warning when not running"
        );
    }

    #[test]
    fn build_channel_surface_checks_fails_when_ready_serve_operation_is_stale() {
        let snapshots = vec![ChannelStatusSnapshot {
            id: "feishu",
            configured_account_id: "feishu_cli_a1b2c3".to_owned(),
            configured_account_label: "feishu_cli_a1b2c3".to_owned(),
            is_default_account: true,
            default_account_source:
                mvp::config::ChannelDefaultAccountSelectionSource::RuntimeIdentity,
            label: "Feishu/Lark",
            aliases: vec!["lark"],
            transport: "feishu_openapi_webhook_or_websocket",
            compiled: true,
            enabled: true,
            api_base_url: Some("https://open.feishu.cn".to_owned()),
            notes: Vec::new(),
            reserved_runtime_fields: Vec::new(),
            operations: vec![ChannelOperationStatus {
                id: "serve",
                label: "inbound reply service",
                command: "feishu-serve",
                health: ChannelOperationHealth::Ready,
                detail: "ready".to_owned(),
                issues: Vec::new(),
                runtime: Some(ChannelOperationRuntime {
                    running: false,
                    stale: true,
                    busy: true,
                    active_runs: 1,
                    last_run_activity_at: Some(1_700_000_000_000),
                    last_heartbeat_at: Some(1_700_000_005_000),
                    pid: Some(4242),
                    account_id: Some("feishu_cli_a1b2c3".to_owned()),
                    account_label: Some("feishu:cli_a1b2c3".to_owned()),
                    instance_count: 1,
                    running_instances: 0,
                    stale_instances: 1,
                }),
            }],
        }];

        let checks = build_channel_surface_checks(&snapshots);

        assert!(
            checks.iter().any(|check| {
                check.name == "feishu serve runtime"
                    && check.level == DoctorCheckLevel::Fail
                    && check.detail.contains("stale")
                    && check.detail.contains("pid=4242")
                    && check.detail.contains("account=feishu:cli_a1b2c3")
            }),
            "stale feishu serve runtime should fail doctor checks"
        );
    }

    #[test]
    fn build_channel_surface_checks_warns_when_multiple_runtime_instances_are_running() {
        let snapshots = vec![ChannelStatusSnapshot {
            id: "telegram",
            configured_account_id: "bot_123456".to_owned(),
            configured_account_label: "bot_123456".to_owned(),
            is_default_account: true,
            default_account_source:
                mvp::config::ChannelDefaultAccountSelectionSource::RuntimeIdentity,
            label: "Telegram",
            aliases: Vec::new(),
            transport: "telegram_bot_api_polling",
            compiled: true,
            enabled: true,
            api_base_url: Some("https://api.telegram.org".to_owned()),
            notes: Vec::new(),
            reserved_runtime_fields: Vec::new(),
            operations: vec![ChannelOperationStatus {
                id: "serve",
                label: "reply loop",
                command: "telegram-serve",
                health: ChannelOperationHealth::Ready,
                detail: "ready".to_owned(),
                issues: Vec::new(),
                runtime: Some(ChannelOperationRuntime {
                    running: true,
                    stale: false,
                    busy: true,
                    active_runs: 1,
                    last_run_activity_at: Some(1_700_000_000_000),
                    last_heartbeat_at: Some(1_700_000_005_000),
                    pid: Some(3003),
                    account_id: Some("bot_123456".to_owned()),
                    account_label: Some("bot:123456".to_owned()),
                    instance_count: 2,
                    running_instances: 2,
                    stale_instances: 0,
                }),
            }],
        }];

        let checks = build_channel_surface_checks(&snapshots);

        assert!(
            checks.iter().any(|check| {
                check.name == "telegram channel runtime"
                    && check.level == DoctorCheckLevel::Warn
                    && check.detail.contains("multiple runtime instances")
                    && check.detail.contains("running_instances=2")
            }),
            "duplicate running telegram runtimes should emit runtime warning"
        );
    }

    #[test]
    fn build_channel_surface_checks_resolves_alias_metadata_from_channel_registry() {
        let snapshots = vec![ChannelStatusSnapshot {
            id: "lark",
            configured_account_id: "feishu_main".to_owned(),
            configured_account_label: "feishu_main".to_owned(),
            is_default_account: true,
            default_account_source:
                mvp::config::ChannelDefaultAccountSelectionSource::ExplicitDefault,
            label: "Feishu/Lark",
            aliases: vec!["feishu"],
            transport: "feishu_openapi_webhook_or_websocket",
            compiled: true,
            enabled: true,
            api_base_url: Some("https://open.feishu.cn".to_owned()),
            notes: vec![
                "webhook_inbound_message_types=text,image,file".to_owned(),
                "webhook_inbound_non_text_mode=structured_text_summary".to_owned(),
                "webhook_inbound_binary_fetch=disabled".to_owned(),
                "webhook_resource_download_tool=feishu.messages.resource.get".to_owned(),
                "webhook_resource_selection_mode=single_resource_default_or_unique_partial_inference_or_resource_inventory".to_owned(),
                "webhook_callback_event_types=card.action.trigger".to_owned(),
                "webhook_callback_response_mode=noop_json".to_owned(),
            ],
            reserved_runtime_fields: Vec::new(),
            operations: vec![ChannelOperationStatus {
                id: "serve",
                label: "inbound reply service",
                command: "feishu-serve",
                health: ChannelOperationHealth::Ready,
                detail: "ready".to_owned(),
                issues: Vec::new(),
                runtime: Some(ChannelOperationRuntime {
                    running: true,
                    stale: false,
                    busy: false,
                    active_runs: 1,
                    last_run_activity_at: Some(1_700_000_000_000),
                    last_heartbeat_at: Some(1_700_000_005_000),
                    pid: Some(4242),
                    account_id: Some("feishu_main".to_owned()),
                    account_label: Some("feishu:main".to_owned()),
                    instance_count: 1,
                    running_instances: 1,
                    stale_instances: 0,
                }),
            }],
        }];

        let checks = build_channel_surface_checks(&snapshots);

        assert!(
            checks
                .iter()
                .any(|check| check.name == "feishu inbound transport"),
            "alias channel ids should reuse registry-backed operation-health metadata: {checks:#?}"
        );
        assert!(
            checks
                .iter()
                .any(|check| check.name == "feishu serve runtime"),
            "alias channel ids should reuse registry-backed runtime metadata: {checks:#?}"
        );
        assert!(
            checks
                .iter()
                .any(|check| check.name == "feishu webhook inbound support"),
            "alias channel ids should preserve feishu inbound support checks: {checks:#?}"
        );
    }

    #[test]
    fn build_channel_surface_checks_reports_feishu_inbound_support_matrix() {
        let snapshots = vec![ChannelStatusSnapshot {
            id: "feishu",
            configured_account_id: "feishu_main".to_owned(),
            configured_account_label: "feishu_main".to_owned(),
            is_default_account: true,
            default_account_source:
                mvp::config::ChannelDefaultAccountSelectionSource::ExplicitDefault,
            label: "Feishu/Lark",
            aliases: vec!["lark"],
            transport: "feishu_openapi_webhook_or_websocket",
            compiled: true,
            enabled: true,
            api_base_url: Some("https://open.feishu.cn".to_owned()),
            notes: vec![
                "webhook_inbound_message_types=text,image,file,post,audio,media,folder,sticker,interactive,share_chat,share_user,system,location,video_chat,todo,vote,merge_forward,share_calendar_event,calendar,general_calendar".to_owned(),
                "webhook_inbound_non_text_mode=structured_text_summary".to_owned(),
                "webhook_inbound_binary_fetch=disabled".to_owned(),
                "webhook_resource_download_tool=feishu.messages.resource.get".to_owned(),
                "webhook_resource_selection_mode=single_resource_default_or_unique_partial_inference_or_resource_inventory".to_owned(),
                "webhook_callback_event_types=card.action.trigger,card.action.trigger_v1".to_owned(),
                "webhook_callback_response_mode=noop_json".to_owned(),
            ],
            reserved_runtime_fields: Vec::new(),
            operations: vec![ChannelOperationStatus {
                id: "serve",
                label: "inbound reply service",
                command: "feishu-serve",
                health: ChannelOperationHealth::Ready,
                detail: "ready".to_owned(),
                issues: Vec::new(),
                runtime: None,
            }],
        }];

        let checks = build_channel_surface_checks(&snapshots);

        assert!(checks.iter().any(|check| {
            check.name == "feishu webhook inbound support"
                && check.level == DoctorCheckLevel::Pass
                && check
                    .detail
                    .contains("text,image,file,post,audio,media,folder,sticker,interactive,share_chat,share_user,system,location,video_chat,todo,vote,merge_forward,share_calendar_event,calendar,general_calendar")
                && check.detail.contains("structured_text_summary")
                && check.detail.contains("binary_fetch=disabled")
                && check
                    .detail
                    .contains("resource_download_tool=feishu.messages.resource.get")
                && check.detail.contains(
                    "resource_selection_mode=single_resource_default_or_unique_partial_inference_or_resource_inventory"
                )
                && check
                    .detail
                    .contains("callback_event_types=card.action.trigger,card.action.trigger_v1")
                && check.detail.contains("callback_response_mode=noop_json")
        }));
    }

    #[test]
    fn build_channel_surface_checks_scopes_names_for_multi_account_snapshots() {
        let snapshots = vec![
            ChannelStatusSnapshot {
                id: "telegram",
                configured_account_id: "ops".to_owned(),
                configured_account_label: "ops".to_owned(),
                is_default_account: true,
                default_account_source:
                    mvp::config::ChannelDefaultAccountSelectionSource::ExplicitDefault,
                label: "Telegram",
                aliases: Vec::new(),
                transport: "telegram_bot_api_polling",
                compiled: true,
                enabled: true,
                api_base_url: Some("https://api.telegram.org".to_owned()),
                notes: vec!["configured_account_id=ops".to_owned()],
                reserved_runtime_fields: Vec::new(),
                operations: vec![ChannelOperationStatus {
                    id: "serve",
                    label: "reply loop",
                    command: "telegram-serve",
                    health: ChannelOperationHealth::Ready,
                    detail: "ready".to_owned(),
                    issues: Vec::new(),
                    runtime: Some(ChannelOperationRuntime {
                        running: true,
                        stale: false,
                        busy: false,
                        active_runs: 0,
                        last_run_activity_at: None,
                        last_heartbeat_at: None,
                        pid: Some(2001),
                        account_id: Some("bot_123456".to_owned()),
                        account_label: Some("bot:123456".to_owned()),
                        instance_count: 1,
                        running_instances: 1,
                        stale_instances: 0,
                    }),
                }],
            },
            ChannelStatusSnapshot {
                id: "telegram",
                configured_account_id: "personal".to_owned(),
                configured_account_label: "personal".to_owned(),
                is_default_account: false,
                default_account_source:
                    mvp::config::ChannelDefaultAccountSelectionSource::ExplicitDefault,
                label: "Telegram",
                aliases: Vec::new(),
                transport: "telegram_bot_api_polling",
                compiled: true,
                enabled: true,
                api_base_url: Some("https://api.telegram.org".to_owned()),
                notes: vec!["configured_account_id=personal".to_owned()],
                reserved_runtime_fields: Vec::new(),
                operations: vec![ChannelOperationStatus {
                    id: "serve",
                    label: "reply loop",
                    command: "telegram-serve",
                    health: ChannelOperationHealth::Ready,
                    detail: "ready".to_owned(),
                    issues: Vec::new(),
                    runtime: Some(ChannelOperationRuntime {
                        running: false,
                        stale: false,
                        busy: false,
                        active_runs: 0,
                        last_run_activity_at: None,
                        last_heartbeat_at: None,
                        pid: None,
                        account_id: Some("bot_654321".to_owned()),
                        account_label: Some("bot:654321".to_owned()),
                        instance_count: 0,
                        running_instances: 0,
                        stale_instances: 0,
                    }),
                }],
            },
        ];

        let checks = build_channel_surface_checks(&snapshots);

        assert!(
            checks
                .iter()
                .any(|check| check.name == "telegram channel [ops]")
        );
        assert!(
            checks
                .iter()
                .any(|check| check.name == "telegram channel runtime [personal]")
        );
    }

    #[test]
    fn build_channel_surface_checks_warns_when_multi_account_default_uses_fallback() {
        let snapshots = vec![
            ChannelStatusSnapshot {
                id: "telegram",
                configured_account_id: "alerts".to_owned(),
                configured_account_label: "alerts".to_owned(),
                is_default_account: true,
                default_account_source: mvp::config::ChannelDefaultAccountSelectionSource::Fallback,
                label: "Telegram",
                aliases: Vec::new(),
                transport: "telegram_bot_api_polling",
                compiled: true,
                enabled: true,
                api_base_url: Some("https://api.telegram.org".to_owned()),
                notes: vec!["default_account_source=fallback".to_owned()],
                reserved_runtime_fields: Vec::new(),
                operations: vec![ChannelOperationStatus {
                    id: "serve",
                    label: "reply loop",
                    command: "telegram-serve",
                    health: ChannelOperationHealth::Ready,
                    detail: "ready".to_owned(),
                    issues: Vec::new(),
                    runtime: None,
                }],
            },
            ChannelStatusSnapshot {
                id: "telegram",
                configured_account_id: "work".to_owned(),
                configured_account_label: "work".to_owned(),
                is_default_account: false,
                default_account_source: mvp::config::ChannelDefaultAccountSelectionSource::Fallback,
                label: "Telegram",
                aliases: Vec::new(),
                transport: "telegram_bot_api_polling",
                compiled: true,
                enabled: true,
                api_base_url: Some("https://api.telegram.org".to_owned()),
                notes: vec!["default_account_source=fallback".to_owned()],
                reserved_runtime_fields: Vec::new(),
                operations: vec![ChannelOperationStatus {
                    id: "serve",
                    label: "reply loop",
                    command: "telegram-serve",
                    health: ChannelOperationHealth::Ready,
                    detail: "ready".to_owned(),
                    issues: Vec::new(),
                    runtime: None,
                }],
            },
        ];

        let checks = build_channel_surface_checks(&snapshots);

        assert!(checks.iter().any(|check| {
            check.name == "telegram default account policy"
                && check.level == DoctorCheckLevel::Warn
                && check.detail.contains("alerts")
                && check.detail.contains("default_account")
        }));
    }

    #[test]
    fn build_channel_surface_checks_ignores_stub_surfaces_without_accounts() {
        let snapshots: Vec<mvp::channel::ChannelStatusSnapshot> = Vec::new();

        let checks = build_channel_surface_checks(&snapshots);

        assert!(checks.is_empty());
    }

    #[test]
    fn build_doctor_next_steps_guides_fix_and_provider_credentials() {
        let checks = vec![
            DoctorCheck {
                name: "provider credentials".to_owned(),
                level: DoctorCheckLevel::Warn,
                detail: "provider credentials are missing (try env: OPENAI_CODEX_OAUTH_TOKEN, OPENAI_API_KEY)"
                    .to_owned(),
            },
            DoctorCheck {
                name: "memory path".to_owned(),
                level: DoctorCheckLevel::Fail,
                detail: "/tmp/loong-memory is missing".to_owned(),
            },
        ];
        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            Path::new("/tmp/loong.toml"),
            &mvp::config::LoongConfig::default(),
            false,
            Some(std::ffi::OsStr::new("")),
        );

        assert_eq!(
            next_steps[0],
            "Apply safe local repairs: loong doctor --config '/tmp/loong.toml' --fix"
        );
        assert!(
            next_steps.iter().any(|step| {
                step
                    == "Set provider credentials in env: OPENAI_CODEX_OAUTH_TOKEN or OPENAI_OAUTH_ACCESS_TOKEN or OPENAI_API_KEY"
            }),
            "doctor should turn missing provider auth into a concrete next step: {next_steps:#?}"
        );
        assert!(
            next_steps
                .iter()
                .any(|step| step == "Re-run diagnostics: loong doctor --config '/tmp/loong.toml'"),
            "doctor should tell the operator how to confirm the repair path: {next_steps:#?}"
        );
    }

    #[test]
    fn build_doctor_next_steps_guides_managed_bridge_incomplete_setup() {
        let install_root = browser_companion_temp_dir("managed-bridge-next-steps-incomplete");
        let mut metadata = compatible_managed_bridge_metadata(
            "qq_official_bot_gateway_or_plugin_bridge",
            "qqbot_reply_loop",
        );
        let removed_transport_family = metadata.remove("transport_family");
        let setup = managed_bridge_setup_with_guidance(
            "channel",
            vec!["QQBOT_BRIDGE_URL"],
            vec!["qqbot.bridge_url"],
            vec!["https://example.test/docs/qqbot-bridge"],
            Some(
                "Run the QQ bridge setup flow before enabling this bridge.\nThen confirm exactly one managed bridge remains.",
            ),
        );
        let mut manifest = managed_bridge_manifest_with_setup("qqbot", metadata, Some(setup));
        let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
            "qqbot": {
                "enabled": true,
                "app_id": "10001",
                "client_secret": "qqbot-secret",
                "allowed_peer_ids": ["openid-alice"]
            }
        }))
        .expect("deserialize qqbot config");

        manifest.plugin_id = "qqbot-bridge-guided".to_owned();
        assert_eq!(
            removed_transport_family.as_deref(),
            Some("qq_official_bot_gateway_or_plugin_bridge")
        );

        write_managed_bridge_manifest(install_root.as_path(), "qqbot-bridge-guided", &manifest);
        config.external_skills.install_root = Some(install_root.display().to_string());

        let checks = check_channel_surfaces(&config);
        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            Path::new("/tmp/loong.toml"),
            &config,
            false,
            Some(std::ffi::OsStr::new("")),
        );

        assert!(
            next_steps.iter().any(|step| {
                step.contains("Complete managed bridge setup for qqbot plugin qqbot-bridge-guided")
                    && step.contains("required env: QQBOT_BRIDGE_URL")
                    && step.contains("required config keys: qqbot.bridge_url")
                    && step.contains("docs: https://example.test/docs/qqbot-bridge")
                    && step.contains(
                        "remediation: \"Run the QQ bridge setup flow before enabling this bridge.\\nThen confirm exactly one managed bridge remains.\""
                    )
            }),
            "doctor should translate incomplete managed bridge metadata into concrete remediation next steps: {next_steps:#?}"
        );
    }

    #[test]
    fn build_doctor_next_steps_guides_managed_bridge_ambiguity_resolution() {
        let install_root = browser_companion_temp_dir("managed-bridge-next-steps-ambiguity");
        let mut first_manifest = managed_bridge_manifest(
            "weixin",
            Some("channel"),
            compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        );
        let mut second_manifest = managed_bridge_manifest(
            "weixin",
            Some("channel"),
            compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        );
        let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
            "weixin": {
                "enabled": true,
                "bridge_url": "https://bridge.example.test/weixin",
                "bridge_access_token": "weixin-token",
                "allowed_contact_ids": ["wxid_alice"]
            }
        }))
        .expect("deserialize weixin config");

        first_manifest.plugin_id = "weixin-bridge-shared".to_owned();
        second_manifest.plugin_id = "weixin-bridge-shared".to_owned();

        write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-a", &first_manifest);
        write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-b", &second_manifest);
        config.external_skills.install_root = Some(install_root.display().to_string());

        let checks = check_channel_surfaces(&config);
        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            Path::new("/tmp/loong.toml"),
            &config,
            false,
            Some(std::ffi::OsStr::new("")),
        );

        assert!(
            next_steps.iter().any(|step| {
                step.contains("Resolve managed bridge ambiguity for weixin")
                    && step.contains("weixin-bridge-shared@")
                    && step.contains("weixin-bridge-a")
                    && step.contains("weixin-bridge-b")
            }),
            "doctor should add a deterministic de-ambiguation step when multiple compatible managed bridges are discovered: {next_steps:#?}"
        );
    }

    #[test]
    fn check_channel_surfaces_warns_when_configured_managed_bridge_plugin_id_is_missing() {
        let install_root = browser_companion_temp_dir("managed-bridge-selection-missing");
        let mut first_manifest = managed_bridge_manifest(
            "weixin",
            Some("channel"),
            compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        );
        let mut second_manifest = managed_bridge_manifest(
            "weixin",
            Some("channel"),
            compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        );
        let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
            "weixin": {
                "enabled": true,
                "managed_bridge_plugin_id": "missing-bridge",
                "bridge_url": "https://bridge.example.test/weixin",
                "bridge_access_token": "weixin-token",
                "allowed_contact_ids": ["wxid_alice"]
            }
        }))
        .expect("deserialize weixin config");

        first_manifest.plugin_id = "weixin-bridge-a".to_owned();
        second_manifest.plugin_id = "weixin-bridge-b".to_owned();

        write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-a", &first_manifest);
        write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-b", &second_manifest);
        config.external_skills.install_root = Some(install_root.display().to_string());

        let checks = check_channel_surfaces(&config);

        assert!(checks.iter().any(|check| {
            check.name == "weixin managed bridge discovery"
                && check.level == DoctorCheckLevel::Warn
                && check.detail.contains("configured_plugin_id=missing-bridge")
                && check
                    .detail
                    .contains("selection_status=configured_plugin_not_found")
        }));
    }

    #[test]
    fn check_channel_surfaces_summarizes_multi_account_bridge_state_in_discovery_detail() {
        let install_root = browser_companion_temp_dir("managed-bridge-discovery-multi-account");
        let manifest = managed_bridge_manifest(
            "weixin",
            Some("channel"),
            compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        );
        let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
            "weixin": {
                "enabled": true,
                "default_account": "ops",
                "accounts": {
                    "ops": {
                        "enabled": true,
                        "bridge_url": "https://bridge.example.test/ops",
                        "bridge_access_token": "ops-token",
                        "allowed_contact_ids": ["wxid_ops"]
                    },
                    "backup": {
                        "enabled": true,
                        "bridge_access_token": "backup-token",
                        "allowed_contact_ids": ["wxid_backup"]
                    }
                }
            }
        }))
        .expect("deserialize weixin config");

        write_managed_bridge_manifest(install_root.as_path(), "weixin-managed-bridge", &manifest);
        config.external_skills.install_root = Some(install_root.display().to_string());

        let checks = check_channel_surfaces(&config);

        assert!(checks.iter().any(|check| {
            check.name == "weixin managed bridge discovery"
                && check.level == DoctorCheckLevel::Pass
                && check
                    .detail
                    .contains("selected_plugin_id=weixin-managed-bridge")
                && check.detail.contains("configured_account=ops")
                && check.detail.contains("(default): ready")
                && check.detail.contains("configured_account=backup")
                && check.detail.contains("bridge_url is missing")
        }));
    }

    #[test]
    fn doctor_json_checks_include_plugin_bridge_account_summary_for_mixed_multi_account_surface() {
        let install_root = browser_companion_temp_dir("managed-bridge-json-multi-account");
        let manifest = managed_bridge_manifest(
            "weixin",
            Some("channel"),
            compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        );
        let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
            "weixin": {
                "enabled": true,
                "default_account": "ops",
                "accounts": {
                    "ops": {
                        "enabled": true,
                        "bridge_url": "https://bridge.example.test/ops",
                        "bridge_access_token": "ops-token",
                        "allowed_contact_ids": ["wxid_ops"]
                    },
                    "backup": {
                        "enabled": true,
                        "bridge_access_token": "backup-token",
                        "allowed_contact_ids": ["wxid_backup"]
                    }
                }
            }
        }))
        .expect("deserialize weixin config");

        write_managed_bridge_manifest(install_root.as_path(), "weixin-managed-bridge", &manifest);
        config.external_skills.install_root = Some(install_root.display().to_string());

        let inventory = mvp::channel::channel_inventory(&config);
        let checks = collect_channel_surface_checks(&inventory);
        let payload = doctor_checks_json_payload(&checks, &inventory.channel_surfaces);
        let discovery_check = payload
            .iter()
            .find(|value| value["name"].as_str() == Some("weixin managed bridge discovery"))
            .expect("weixin discovery payload");

        assert_eq!(
            discovery_check["plugin_bridge_account_summary"]
                .as_str()
                .expect("plugin bridge account summary string"),
            "configured_account=ops (default): ready; configured_account=backup: bridge_url is missing"
        );
    }

    #[test]
    fn build_doctor_next_steps_guides_missing_managed_bridge_selection_resolution() {
        let install_root =
            browser_companion_temp_dir("managed-bridge-next-steps-selection-missing");
        let mut first_manifest = managed_bridge_manifest(
            "weixin",
            Some("channel"),
            compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        );
        let mut second_manifest = managed_bridge_manifest(
            "weixin",
            Some("channel"),
            compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        );
        let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
            "weixin": {
                "enabled": true,
                "managed_bridge_plugin_id": "missing-bridge",
                "bridge_url": "https://bridge.example.test/weixin",
                "bridge_access_token": "weixin-token",
                "allowed_contact_ids": ["wxid_alice"]
            }
        }))
        .expect("deserialize weixin config");

        first_manifest.plugin_id = "weixin-bridge-a".to_owned();
        second_manifest.plugin_id = "weixin-bridge-b".to_owned();

        write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-a", &first_manifest);
        write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-b", &second_manifest);
        config.external_skills.install_root = Some(install_root.display().to_string());

        let checks = check_channel_surfaces(&config);
        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            Path::new("/tmp/loong.toml"),
            &config,
            false,
            Some(std::ffi::OsStr::new("")),
        );

        assert!(
            next_steps.iter().any(|step| {
                step.contains("Fix managed bridge selection for weixin")
                    && step.contains("managed_bridge_plugin_id=missing-bridge")
                    && step.contains("weixin-bridge-a,weixin-bridge-b")
            }),
            "doctor should guide users toward a valid configured managed bridge selection: {next_steps:#?}"
        );
    }

    #[test]
    fn build_doctor_next_steps_guides_duplicate_managed_bridge_selection_resolution() {
        let install_root =
            browser_companion_temp_dir("managed-bridge-next-steps-selection-duplicated");
        let mut first_manifest = managed_bridge_manifest(
            "weixin",
            Some("channel"),
            compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        );
        let mut second_manifest = managed_bridge_manifest(
            "weixin",
            Some("channel"),
            compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        );
        let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
            "weixin": {
                "enabled": true,
                "managed_bridge_plugin_id": "weixin-bridge-shared",
                "bridge_url": "https://bridge.example.test/weixin",
                "bridge_access_token": "weixin-token",
                "allowed_contact_ids": ["wxid_alice"]
            }
        }))
        .expect("deserialize weixin config");

        first_manifest.plugin_id = "weixin-bridge-shared".to_owned();
        second_manifest.plugin_id = "weixin-bridge-shared".to_owned();

        write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-a", &first_manifest);
        write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-b", &second_manifest);
        config.external_skills.install_root = Some(install_root.display().to_string());

        let checks = check_channel_surfaces(&config);
        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            Path::new("/tmp/loong.toml"),
            &config,
            false,
            Some(std::ffi::OsStr::new("")),
        );

        assert!(
            next_steps.iter().any(|step| {
                step.contains("Fix managed bridge selection for weixin")
                    && step.contains("managed_bridge_plugin_id=weixin-bridge-shared")
                    && step.contains("weixin-bridge-shared@")
                    && step.contains("weixin-bridge-a")
                    && step.contains("weixin-bridge-b")
            }),
            "doctor should guide operators to remove or rename duplicate managed bridge packages when configured selection is not unique: {next_steps:#?}"
        );
    }

    #[test]
    fn build_doctor_next_steps_with_channel_surfaces_keeps_managed_bridge_snapshot_stable() {
        let install_root = browser_companion_temp_dir("managed-bridge-next-steps-snapshot");
        let mut first_manifest = managed_bridge_manifest(
            "weixin",
            Some("channel"),
            compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        );
        let mut second_manifest = managed_bridge_manifest(
            "weixin",
            Some("channel"),
            compatible_managed_bridge_metadata("wechat_clawbot_ilink_bridge", "weixin_reply_loop"),
        );
        let mut config: mvp::config::LoongConfig = serde_json::from_value(serde_json::json!({
            "weixin": {
                "enabled": true,
                "bridge_url": "https://bridge.example.test/weixin",
                "bridge_access_token": "weixin-token",
                "allowed_contact_ids": ["wxid_alice"]
            }
        }))
        .expect("deserialize weixin config");

        first_manifest.plugin_id = "weixin-bridge-a".to_owned();
        second_manifest.plugin_id = "weixin-bridge-b".to_owned();

        write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-a", &first_manifest);
        write_managed_bridge_manifest(install_root.as_path(), "weixin-bridge-b", &second_manifest);
        config.external_skills.install_root = Some(install_root.display().to_string());

        let checks = check_channel_surfaces(&config);
        let inventory = mvp::channel::channel_inventory(&config);
        let removed_plugin_directory = install_root.as_path().join("weixin-bridge-b");

        std::fs::remove_dir_all(&removed_plugin_directory)
            .expect("remove second managed bridge after checks");

        let next_steps = build_doctor_next_steps_with_channel_surfaces_and_path_env(
            &checks,
            Path::new("/tmp/loong.toml"),
            &config,
            &inventory.channel_surfaces,
            false,
            Some(std::ffi::OsStr::new("")),
        );

        assert!(
            next_steps.iter().any(|step| {
                step.contains("Resolve managed bridge ambiguity for weixin")
                    && step.contains("weixin-bridge-a,weixin-bridge-b")
            }),
            "doctor next steps should stay anchored to the same discovery snapshot as the checks even if the managed install root changes afterward: {next_steps:#?}"
        );
    }

    #[test]
    fn provider_credentials_doctor_check_adds_volcengine_auth_guidance() {
        let mut config = mvp::config::LoongConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Volcengine;
        config.provider.api_key = None;
        config.provider.api_key_env = None;
        config.provider.oauth_access_token = None;
        config.provider.oauth_access_token_env = None;
        let auth_env_names = config.provider.auth_hint_env_names();
        let mut env = ScopedEnv::new();
        for env_name in auth_env_names {
            env.remove(env_name);
        }

        let check = provider_credentials_doctor_check(&config, false);

        assert_eq!(check.name, "provider credentials");
        assert_eq!(check.level, DoctorCheckLevel::Warn);
        assert!(check.detail.contains("ARK_API_KEY"));
        assert!(check.detail.contains("Authorization: Bearer <ARK_API_KEY>"));
    }

    #[test]
    fn provider_credentials_doctor_check_passes_for_auth_optional_provider() {
        let mut config = mvp::config::LoongConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Ollama;
        config.provider.api_key = None;
        config.provider.api_key_env = None;
        config.provider.oauth_access_token = None;
        config.provider.oauth_access_token_env = None;

        let check = provider_credentials_doctor_check(&config, false);

        assert_eq!(check.name, "provider credentials");
        assert_eq!(check.level, DoctorCheckLevel::Pass);
        assert!(check.detail.contains("optional for this provider"));
    }

    #[test]
    fn web_search_provider_doctor_check_warns_when_firecrawl_credential_is_missing() {
        let mut env = ScopedEnv::new();
        let mut config = mvp::config::LoongConfig::default();
        let provider_id = mvp::config::WEB_SEARCH_PROVIDER_FIRECRAWL.to_owned();
        let configured_secret = "${FIRECRAWL_API_KEY}".to_owned();

        env.remove("FIRECRAWL_API_KEY");
        config.tools.web_search.default_provider = provider_id;
        config.tools.web_search.firecrawl_api_key = Some(configured_secret);

        let check = web_search_provider_doctor_check(&config);

        assert_eq!(check.name, "web search provider");
        assert_eq!(check.level, DoctorCheckLevel::Warn);
        assert!(check.detail.contains("Firecrawl Search"));
        assert!(check.detail.contains("FIRECRAWL_API_KEY"));
        assert!(check.detail.contains("web.search will stay unavailable"));
    }

    #[test]
    fn web_search_provider_doctor_check_passes_when_firecrawl_credential_is_available() {
        let mut config = mvp::config::LoongConfig::default();
        let provider_id = mvp::config::WEB_SEARCH_PROVIDER_FIRECRAWL.to_owned();
        let configured_secret = "${FIRECRAWL_API_KEY}".to_owned();
        let mut env = ScopedEnv::new();

        env.set("FIRECRAWL_API_KEY", "firecrawl-test-token");
        config.tools.web_search.default_provider = provider_id;
        config.tools.web_search.firecrawl_api_key = Some(configured_secret);

        let check = web_search_provider_doctor_check(&config);

        assert_eq!(check.name, "web search provider");
        assert_eq!(check.level, DoctorCheckLevel::Pass);
        assert!(check.detail.contains("Firecrawl Search"));
        assert!(check.detail.contains("FIRECRAWL_API_KEY"));
    }

    #[test]
    fn web_search_provider_doctor_check_passes_when_tool_is_disabled() {
        let mut config = mvp::config::LoongConfig::default();

        config.tools.web_search.enabled = false;
        config.tools.web_search.default_provider =
            mvp::config::WEB_SEARCH_PROVIDER_FIRECRAWL.to_owned();

        let check = web_search_provider_doctor_check(&config);

        assert_eq!(check.name, "web search provider");
        assert_eq!(check.level, DoctorCheckLevel::Pass);
        assert_eq!(check.detail, "tools.web_search.enabled=false");
    }

    #[test]
    fn build_doctor_next_steps_shell_quotes_config_paths_with_single_quotes() {
        let checks = vec![DoctorCheck {
            name: "memory path".to_owned(),
            level: DoctorCheckLevel::Fail,
            detail: "/tmp/loong-memory is missing".to_owned(),
        }];
        let next_steps = build_doctor_next_steps(
            &checks,
            Path::new("/tmp/loong's config.toml"),
            &mvp::config::LoongConfig::default(),
            false,
        );

        assert!(
            next_steps.iter().any(|step| {
                step == "Apply safe local repairs: loong doctor --config '/tmp/loong'\"'\"'s config.toml' --fix"
            }),
            "doctor should shell-quote config paths with single quotes in fix commands: {next_steps:#?}"
        );
        assert!(
            next_steps.iter().any(|step| {
                step == "Re-run diagnostics: loong doctor --config '/tmp/loong'\"'\"'s config.toml'"
            }),
            "doctor should shell-quote config paths with single quotes in rerun commands: {next_steps:#?}"
        );
    }

    #[test]
    fn build_doctor_next_steps_guides_browser_companion_repair() {
        let checks = vec![DoctorCheck {
            name: "browser companion install".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "command `loong-browser-companion` was not found on PATH".to_owned(),
        }];
        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            Path::new("/tmp/loong.toml"),
            &mvp::config::LoongConfig::default(),
            false,
            Some(std::ffi::OsStr::new("")),
        );

        assert!(
            next_steps.iter().any(|step| {
                step == "Install or expose the browser companion command on PATH, then re-run: loong doctor --config '/tmp/loong.toml'"
            }),
            "doctor should turn browser companion warnings into a concrete repair path: {next_steps:#?}"
        );
    }

    #[test]
    fn build_doctor_next_steps_guides_browser_companion_version_alignment() {
        let checks = vec![DoctorCheck {
            name: "browser companion install".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "command `browser-companion` responded, but expected_version=1.5.0 observed_version=loong-browser-companion 1.4.0".to_owned(),
        }];
        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            Path::new("/tmp/loong.toml"),
            &mvp::config::LoongConfig::default(),
            false,
            Some(std::ffi::OsStr::new("")),
        );

        assert!(
            next_steps.iter().any(|step| {
                step == "Align `tools.browser_companion.expected_version` with the installed companion build before retrying."
            }),
            "doctor should guide expected_version alignment when the companion install check reports a mismatch: {next_steps:#?}"
        );
    }

    #[test]
    fn build_doctor_next_steps_guides_missing_web_search_credentials() {
        let checks = vec![DoctorCheck {
            name: "web search provider".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "Firecrawl Search: FIRECRAWL_API_KEY (expected). web.search will stay unavailable until the provider credential is supplied".to_owned(),
        }];
        let mut config = mvp::config::LoongConfig::default();
        let config_path = Path::new("/tmp/loong.toml");

        config.tools.web_search.default_provider =
            mvp::config::WEB_SEARCH_PROVIDER_FIRECRAWL.to_owned();

        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            config_path,
            &config,
            false,
            Some(std::ffi::OsStr::new("")),
        );
        let rerun_onboard_command =
            crate::cli_handoff::format_subcommand_with_config("onboard", "/tmp/loong.toml");
        let expected_onboard_step = format!(
            "Or rerun onboarding to review the web search provider choice: {rerun_onboard_command}"
        );

        assert!(
            next_steps
                .iter()
                .any(|step| step == "Set web search credential in env: FIRECRAWL_API_KEY"),
            "doctor should surface the missing Firecrawl env binding as a concrete next step: {next_steps:#?}"
        );
        assert!(
            next_steps.iter().any(|step| step == &expected_onboard_step),
            "doctor should keep the onboarding recovery path explicit for web search credentials: {next_steps:#?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn browser_companion_doctor_checks_warn_when_command_is_missing() {
        let _env_guard = BrowserCompanionEnvGuard::runtime_gate_closed();
        let mut config = mvp::config::LoongConfig::default();
        config.tools.browser_companion.enabled = true;

        let checks = collect_browser_companion_doctor_checks(&config).await;

        assert!(
            checks.iter().any(|check| {
                check.name == "browser companion install"
                    && check.level == DoctorCheckLevel::Warn
                    && check.detail.contains("no command is configured")
            }),
            "doctor should warn when browser companion is enabled without a command: {checks:#?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn browser_companion_doctor_checks_warn_when_expected_version_mismatches() {
        let _env_guard = BrowserCompanionEnvGuard::runtime_gate_closed();
        let (command, observed_version, _exact_version, partial_version) = rustc_version_probe();

        let mut config = mvp::config::LoongConfig::default();
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.command = Some(command);
        config.tools.browser_companion.expected_version = Some(partial_version.clone());

        let checks = collect_browser_companion_doctor_checks(&config).await;

        assert!(
            checks.iter().any(|check| {
                check.name == "browser companion install"
                    && check.level == DoctorCheckLevel::Warn
                    && check
                        .detail
                        .contains(format!("expected_version={partial_version}").as_str())
                    && check
                        .detail
                        .contains(format!("observed_version={observed_version}").as_str())
            }),
            "doctor should surface version mismatches for the managed companion lane: {checks:#?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn browser_companion_doctor_checks_warn_when_runtime_gate_is_closed() {
        let _env_guard = BrowserCompanionEnvGuard::runtime_gate_closed();
        let (command, _observed_version, exact_version, _partial_version) = rustc_version_probe();

        let mut config = mvp::config::LoongConfig::default();
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.command = Some(command);
        config.tools.browser_companion.expected_version = Some(exact_version);

        let checks = collect_browser_companion_doctor_checks(&config).await;

        assert!(
            checks.iter().any(|check| {
                check.name == "browser companion runtime gate"
                    && check.level == DoctorCheckLevel::Warn
                    && check.detail.contains("install looks healthy")
            }),
            "doctor should distinguish healthy companion installs from a still-closed runtime gate: {checks:#?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn browser_companion_doctor_checks_pass_when_runtime_gate_is_open() {
        let _env_guard = BrowserCompanionEnvGuard::runtime_gate_open();
        let (command, _observed_version, exact_version, _partial_version) = rustc_version_probe();

        let mut config = mvp::config::LoongConfig::default();
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.command = Some(command);
        config.tools.browser_companion.expected_version = Some(exact_version);

        let checks = collect_browser_companion_doctor_checks(&config).await;

        assert!(
            checks.iter().any(|check| {
                check.name == "browser companion install"
                    && check.level == DoctorCheckLevel::Pass
                    && check.detail.contains("responded with")
            }),
            "doctor should mark the companion install healthy when the version probe matches: {checks:#?}"
        );
        assert!(
            checks.iter().any(|check| {
                check.name == "browser companion runtime gate"
                    && check.level == DoctorCheckLevel::Pass
                    && check.detail.contains("runtime is ready")
            }),
            "doctor should mark the runtime gate healthy when the companion lane is opened: {checks:#?}"
        );
    }

    #[test]
    fn build_doctor_next_steps_guides_reviewed_onboarding_default_for_auto_model_probe_failures() {
        let checks = vec![DoctorCheck {
            name: "provider model probe".to_owned(),
            level: DoctorCheckLevel::Fail,
            detail: "DeepSeek [deepseek]: model catalog probe failed (401 Unauthorized); current config still uses `model = auto`; rerun onboarding and accept reviewed model `deepseek-chat`, or set `provider.model` / `preferred_models` explicitly".to_owned(),
        }];
        let mut config = mvp::config::LoongConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "auto".to_owned();

        let next_steps =
            build_doctor_next_steps(&checks, Path::new("/tmp/loong.toml"), &config, false);

        assert!(
            next_steps.iter().any(|step| {
                step == "Rerun onboarding and accept reviewed model `deepseek-chat`: loong onboard --config '/tmp/loong.toml'"
            }),
            "doctor should point reviewed providers back to onboarding when auto-model recovery needs an explicit reviewed default: {next_steps:#?}"
        );
        assert!(
            next_steps.iter().any(|step| {
                step == "Or set `provider.model` / `preferred_models` explicitly, then re-run diagnostics: loong doctor --config '/tmp/loong.toml'"
            }),
            "doctor should also keep the manual remediation path explicit for operators who do not want to rerun onboarding: {next_steps:#?}"
        );
        assert!(
            next_steps
                .iter()
                .all(|step| !step.contains("--skip-model-probe")),
            "doctor should not suggest --skip-model-probe when the real blocker is still `model = auto` without explicit recovery candidates: {next_steps:#?}"
        );
    }

    #[test]
    fn build_doctor_next_steps_guides_warn_level_explicit_model_probe_recovery() {
        let checks = vec![DoctorCheck {
            name: "provider model probe".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "DeepSeek [deepseek]: model catalog probe failed (401 Unauthorized); chat may still work because model `deepseek-chat` is explicitly configured".to_owned(),
        }];
        let mut config = mvp::config::LoongConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "deepseek-chat".to_owned();

        let next_steps =
            build_doctor_next_steps(&checks, Path::new("/tmp/loong.toml"), &config, false);

        assert!(
            next_steps.iter().any(|step| {
                step == "Retry provider probe only after credentials are ready: loong doctor --config '/tmp/loong.toml'"
            }),
            "warn-level explicit model recovery should still tell operators how to retry diagnostics: {next_steps:#?}"
        );
        assert!(
            next_steps.iter().any(|step| {
                step == "If your provider blocks model listing during setup, retry with: loong doctor --config '/tmp/loong.toml' --skip-model-probe"
            }),
            "warn-level explicit model recovery should still keep the skip-model-probe escape hatch visible: {next_steps:#?}"
        );
    }

    #[test]
    fn build_doctor_next_steps_guides_warn_level_preferred_model_probe_recovery() {
        let checks = vec![DoctorCheck {
            name: "provider model probe".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "DeepSeek [deepseek]: model catalog probe failed (401 Unauthorized); runtime will try configured preferred model fallback(s): `deepseek-chat`, `deepseek-reasoner`".to_owned(),
        }];
        let mut config = mvp::config::LoongConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "auto".to_owned();
        config.provider.preferred_models =
            vec!["deepseek-chat".to_owned(), "deepseek-reasoner".to_owned()];

        let next_steps =
            build_doctor_next_steps(&checks, Path::new("/tmp/loong.toml"), &config, false);

        assert!(
            next_steps.iter().any(|step| {
                step == "Retry provider probe only after credentials are ready: loong doctor --config '/tmp/loong.toml'"
            }),
            "warn-level preferred-model recovery should still tell operators how to retry diagnostics: {next_steps:#?}"
        );
        assert!(
            next_steps.iter().any(|step| {
                step == "If your provider blocks model listing during setup, retry with: loong doctor --config '/tmp/loong.toml' --skip-model-probe"
            }),
            "warn-level preferred-model recovery should still keep the skip-model-probe escape hatch visible: {next_steps:#?}"
        );
    }

    #[test]
    fn build_doctor_next_steps_guides_provider_route_probe_repairs() {
        let checks = vec![
            DoctorCheck {
                name: "provider model probe".to_owned(),
                level: DoctorCheckLevel::Fail,
                detail:
                    "OpenAI [openai]: model catalog transport failed (provider model-list request failed on attempt 3/3: operation timed out)"
                        .to_owned(),
            },
            DoctorCheck {
                name: "provider route probe".to_owned(),
                level: DoctorCheckLevel::Warn,
                detail:
                    "request/models host api.openai.com:443: dns resolved to 198.18.0.2 (fake-ip-style); tcp connect ok. the route currently depends on local fake-ip/TUN interception."
                        .to_owned(),
            },
        ];

        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            Path::new("/tmp/loong.toml"),
            &mvp::config::LoongConfig::default(),
            false,
            Some(std::ffi::OsStr::new("")),
        );

        assert!(
            next_steps.iter().any(|step| {
                step.contains("provider route")
                    && step.contains("loong doctor --config '/tmp/loong.toml'")
            }),
            "route-probe findings should produce a concrete diagnostics rerun step: {next_steps:#?}"
        );
        assert!(
            next_steps.iter().any(|step| {
                step.contains("fake-ip") || step.contains("direct/bypass") || step.contains("proxy")
            }),
            "route-probe findings should explain how to repair proxy/fake-ip routing instead of leaving recovery implicit: {next_steps:#?}"
        );
    }

    #[test]
    fn build_doctor_next_steps_ignores_non_failure_model_probe_warnings() {
        let checks = vec![DoctorCheck {
            name: "provider model probe".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "skipped because credentials are missing".to_owned(),
        }];
        let mut config = mvp::config::LoongConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "deepseek-chat".to_owned();

        let next_steps =
            build_doctor_next_steps(&checks, Path::new("/tmp/loong.toml"), &config, false);

        assert!(
            next_steps
                .iter()
                .all(|step| !step.contains("Retry provider probe only after credentials are ready")),
            "skipped probe warnings should not look like real model catalog failures: {next_steps:#?}"
        );
        assert!(
            next_steps
                .iter()
                .all(|step| !step.contains("--skip-model-probe")),
            "skipped probe warnings should not advertise the skip-model-probe recovery branch: {next_steps:#?}"
        );
    }

    #[test]
    fn build_doctor_next_steps_promotes_ask_and_chat_when_green() {
        let checks = vec![
            DoctorCheck {
                name: "provider credentials".to_owned(),
                level: DoctorCheckLevel::Pass,
                detail: "provider credentials are available".to_owned(),
            },
            DoctorCheck {
                name: "provider transport".to_owned(),
                level: DoctorCheckLevel::Pass,
                detail: "responses api".to_owned(),
            },
        ];
        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            Path::new("/tmp/loong.toml"),
            &mvp::config::LoongConfig::default(),
            false,
            Some(std::ffi::OsStr::new("")),
        );

        assert!(
            next_steps.iter().any(|step| {
                step == "Get a first answer: loong ask --config '/tmp/loong.toml' --message 'Summarize this repository and suggest the best next step.'"
            }),
            "green doctor runs should hand the user into ask immediately: {next_steps:#?}"
        );
        assert!(
            next_steps
                .iter()
                .any(|step| { step == "Continue in chat: loong chat --config '/tmp/loong.toml'" }),
            "green doctor runs should still advertise chat as the follow-up path: {next_steps:#?}"
        );
        assert!(
            next_steps.iter().any(|step| {
                step == "Set your working preferences: loong personalize --config '/tmp/loong.toml'"
            }),
            "green doctor runs should surface personalization as the third healthy-path suggestion: {next_steps:#?}"
        );
        assert!(
            !next_steps.iter().any(|step| {
                step == "Open a channel: loong channels --config '/tmp/loong.toml'"
            }),
            "green doctor runs should cap the healthy-path list before lower-priority channel setup suggestions: {next_steps:#?}"
        );
        assert!(
            !next_steps.iter().any(|step| {
                step == "Optional browser preview: loong skills enable-browser-preview --config '/tmp/loong.toml'"
            }),
            "green doctor runs should keep generic browser-preview nudges behind personalization: {next_steps:#?}"
        );
        assert!(
            !next_steps.iter().any(
                |step| {
                    step == "Install browser preview runtime: npm install -g agent-browser && agent-browser install"
                }
            ),
            "green doctor runs should not push runtime install steps before preview has been enabled: {next_steps:#?}"
        );
        assert!(
            !next_steps.iter().any(
                |step| step == "Verify browser preview runtime: agent-browser open example.com"
            ),
            "green doctor runs should not ask for runtime verification before preview has been enabled: {next_steps:#?}"
        );
    }

    #[test]
    fn build_doctor_next_steps_guides_browser_companion_preview_setup() {
        let root = browser_companion_temp_dir("preview-runtime-missing");
        let install_root = root.join("managed-skills");
        std::fs::create_dir_all(install_root.join("browser-companion-preview"))
            .expect("create managed skill directory");
        std::fs::write(
            install_root
                .join("browser-companion-preview")
                .join("SKILL.md"),
            "# Browser Companion Preview\n\nUse agent-browser through exec.\n",
        )
        .expect("write managed preview skill");
        let checks = vec![DoctorCheck {
            name: "provider credentials".to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: "provider credentials are available".to_owned(),
        }];
        let mut config = mvp::config::LoongConfig::default();
        config.tools.file_root = Some(root.display().to_string());
        config.tools.shell_allow.push("agent-browser".to_owned());
        config.external_skills.enabled = true;
        config.external_skills.auto_expose_installed = true;
        config.external_skills.install_root = Some(install_root.display().to_string());

        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            Path::new("/tmp/loong.toml"),
            &config,
            false,
            Some(std::ffi::OsStr::new("")),
        );

        assert!(
            next_steps.iter().any(|step| {
                step == "Install browser preview runtime: npm install -g agent-browser && agent-browser install"
            }),
            "doctor should point preview-enabled operators at a concrete runtime install action when agent-browser is missing: {next_steps:#?}"
        );
        assert!(
            next_steps.iter().any(|step| {
                step == "Verify browser preview runtime: agent-browser open example.com"
            }),
            "doctor should still surface a verification step after the runtime install hint: {next_steps:#?}"
        );
        assert!(
            !next_steps.iter().any(|step| {
                step == "Optional browser preview: loong skills enable-browser-preview --config '/tmp/loong.toml'"
            }),
            "doctor should not fall back to the optional enable step after preview has already been configured: {next_steps:#?}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn build_doctor_next_steps_prioritizes_personalization_when_channels_are_enabled() {
        let checks = vec![DoctorCheck {
            name: "provider credentials".to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: "provider credentials are available".to_owned(),
        }];
        let mut config = mvp::config::LoongConfig::default();
        config.telegram.enabled = true;

        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            Path::new("/tmp/loong.toml"),
            &config,
            false,
            Some(std::ffi::OsStr::new("")),
        );

        assert!(
            next_steps.iter().any(|step| {
                step == "Set your working preferences: loong personalize --config '/tmp/loong.toml'"
            }),
            "doctor should prioritize personalization ahead of generic browser preview when the healthy-path list is capped: {next_steps:#?}"
        );
        assert!(
            !next_steps.iter().any(|step| {
                step == "Optional browser preview: loong skills enable-browser-preview --config '/tmp/loong.toml'"
            }),
            "doctor should keep generic browser preview behind personalization when only three healthy-path actions are shown: {next_steps:#?}"
        );
    }

    #[test]
    fn collect_runtime_plugins_doctor_checks_warns_when_runtime_is_disabled() {
        let root = browser_companion_temp_dir("runtime-plugins-disabled");
        let config = runtime_plugins_test_config(&root, false);

        let checks = collect_runtime_plugins_doctor_checks(&config);

        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].name, "runtime plugins runtime");
        assert_eq!(checks[0].level, DoctorCheckLevel::Warn);
        assert!(checks[0].detail.contains("enabled=false"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn collect_runtime_plugins_doctor_checks_escape_runtime_values() {
        let root = browser_companion_temp_dir("runtime-plugins-escaped");
        let mut config = runtime_plugins_test_config(&root, true);
        let escaped_root = root.join("runtime\nplugins");

        config.runtime_plugins.roots = vec![escaped_root.display().to_string()];
        config.runtime_plugins.supported_adapter_families = vec!["web\nsearch".to_owned()];

        let checks = collect_runtime_plugins_doctor_checks(&config);
        let runtime_check = checks
            .iter()
            .find(|check| check.name == "runtime plugins runtime")
            .expect("runtime plugins runtime check should exist");
        let inventory_check = checks
            .iter()
            .find(|check| check.name == "runtime plugins inventory")
            .expect("runtime plugins inventory check should exist");

        assert!(
            runtime_check
                .detail
                .contains("supported_adapter_families=\"web\\nsearch\"")
        );
        assert!(runtime_check.detail.contains("roots=\""));
        assert!(runtime_check.detail.contains("\\nplugins\""));
        assert!(
            inventory_check
                .detail
                .contains("error=\"runtime plugin scan failed for ")
        );
        assert!(inventory_check.detail.contains("\\nplugins"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn collect_runtime_plugins_doctor_checks_warns_when_no_runtime_roots_are_scanned() {
        let root = browser_companion_temp_dir("runtime-plugins-zero-roots");
        let mut config = runtime_plugins_test_config(&root, true);
        config.runtime_plugins.roots = vec!["   ".to_owned()];

        let checks = collect_runtime_plugins_doctor_checks(&config);

        assert!(
            checks.iter().any(|check| {
                check.name == "runtime plugins runtime"
                    && check.level == DoctorCheckLevel::Warn
                    && check.detail.contains("enabled=true")
                    && check.detail.contains("scanned_roots=0")
            }),
            "runtime plugins runtime should warn when no usable roots can be scanned: {checks:#?}"
        );
        assert!(
            checks.iter().any(|check| {
                check.name == "runtime plugins inventory"
                    && check.level == DoctorCheckLevel::Fail
                    && check.detail.contains("inventory_status=error")
            }),
            "runtime plugins inventory should fail when roots resolve to nothing: {checks:#?}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn doctor_runtime_plugins_json_payload_exposes_structured_runtime_plugin_truth() {
        let root = browser_companion_temp_dir("runtime-plugins-json-payload");
        let config = runtime_plugins_test_config(&root, true);
        let runtime_root = root.join("runtime-plugins");
        std::fs::create_dir_all(runtime_root.join("search")).expect("create runtime root");
        std::fs::write(
            runtime_root.join("search").join("loong.plugin.json"),
            r#"{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "demo-extension-plugin",
  "provider_id": "demo-extension",
  "connector_name": "demo-extension-stdio",
  "capabilities": ["InvokeConnector"],
  "summary": "Demo native extension search adapter",
  "tags": ["search", "demo"],
  "metadata": {
    "bridge_kind": "process_stdio",
    "adapter_family": "python-stdio-adapter",
    "command": "python3",
    "entrypoint": "index.py",
    "loong_extension_contract": "process_stdio_json_line_v1",
    "loong_extension_facets_json": "[\"tooling\",\"events\"]",
    "loong_extension_methods_json": "[\"extension/tool\",\"extension/event\"]",
    "loong_extension_events_json": "[\"session_start\",\"tool_result\"]",
    "loong_extension_host_actions_json": "[\"append_entry\",\"notify\"]"
  }
}"#,
        )
        .expect("write runtime plugin manifest");

        let state = crate::collect_runtime_snapshot_runtime_plugins_state(&config);
        let payload = doctor_runtime_plugins_json_payload(&state);
        let plugin = payload["plugins"]
            .as_array()
            .and_then(|plugins| {
                plugins
                    .iter()
                    .find(|plugin| plugin["plugin_id"].as_str() == Some("demo-extension-plugin"))
            })
            .expect("demo extension plugin should be present");

        assert_eq!(payload["enabled"], json!(true));
        assert_eq!(plugin["manifest_api_version"], json!("v1alpha1"));
        assert_eq!(plugin["plugin_version"], json!("1.0.0"));
        assert_eq!(
            plugin["summary"],
            json!("Demo native extension search adapter")
        );
        assert_eq!(plugin["tags"], json!(["search", "demo"]));
        assert_eq!(plugin["dialect"], json!("loong_package_manifest"));
        assert_eq!(plugin["compatibility_mode"], json!("native"));
        assert_eq!(plugin["compatibility_shim"], serde_json::Value::Null);
        assert_eq!(plugin["compatibility_shim_supported_dialects"], json!([]));
        assert_eq!(plugin["compatibility_shim_mismatch_reasons"], json!([]));
        assert_eq!(plugin["source_language"], json!("manifest"));
        assert_eq!(plugin["entrypoint_hint"], json!("index.py"));
        assert_eq!(
            plugin["extension_contract"],
            json!("process_stdio_json_line_v1")
        );
        assert_eq!(plugin["extension_facets"], json!(["tooling", "events"]));
        assert_eq!(
            plugin["extension_methods"],
            json!(["extension/tool", "extension/event"])
        );
        assert_eq!(
            plugin["extension_events"],
            json!(["session_start", "tool_result"])
        );
        assert_eq!(
            plugin["extension_host_actions"],
            json!(["append_entry", "notify"])
        );
        assert_eq!(plugin["diagnostic_codes"], json!([]));
        assert_eq!(plugin["extension_metadata_issues"], json!([]));

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn doctor_runtime_plugin_inventory_json_payload_surfaces_runtime_plugin_results() {
        let root = browser_companion_temp_dir("runtime-plugins-inventory-json-payload");
        let config = runtime_plugins_test_config(&root, true);
        let runtime_root = root.join("runtime-plugins");
        std::fs::create_dir_all(runtime_root.join("search")).expect("create runtime root");
        std::fs::write(
            runtime_root.join("search").join("loong.plugin.json"),
            r#"{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "demo-extension-plugin",
  "provider_id": "demo-extension",
  "connector_name": "demo-extension-stdio",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "process_stdio",
    "adapter_family": "python-stdio-adapter",
    "command": "python3",
    "entrypoint": "index.py",
    "loong_extension_family": "governed_native_runtime_extension",
    "loong_extension_trust_lane": "governed_sidecar"
  }
}"#,
        )
        .expect("write runtime plugin manifest");

        let payload = serde_json::to_value(
            crate::plugins_cli::runtime_plugin_inventory_read_model(&config).await,
        )
        .expect("serialize runtime plugin inventory payload");
        let plugin = payload["results"]
            .as_array()
            .and_then(|plugins| {
                plugins
                    .iter()
                    .find(|plugin| plugin["plugin_id"].as_str() == Some("demo-extension-plugin"))
            })
            .expect("demo extension plugin inventory result should be present");

        assert_eq!(payload["available"], json!(true));
        assert_eq!(payload["returned_results"], json!(1));
        assert!(payload["summary"].is_object());
        assert!(payload["native_extension_authoring_summary"].is_null());
        assert_eq!(plugin["plugin_id"], json!("demo-extension-plugin"));
        assert_eq!(
            plugin["extension_family"],
            json!("governed_native_runtime_extension")
        );
        assert_eq!(plugin["extension_trust_lane"], json!("governed_sidecar"));
        assert!(plugin.get("activation_attestation").is_some());
        assert!(plugin.get("runtime_health").is_some());

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn doctor_runtime_plugin_inventory_json_payload_rolls_up_native_extension_authoring_semantics()
     {
        let root = browser_companion_temp_dir("runtime-plugins-inventory-json-authoring-rollup");
        let config = runtime_plugins_test_config(&root, true);
        let runtime_root = root.join("runtime-plugins");
        std::fs::create_dir_all(runtime_root.join("search")).expect("create runtime root");
        std::fs::write(
            runtime_root.join("search").join("loong.plugin.json"),
            r#"{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "broken-extension",
  "provider_id": "broken-extension",
  "connector_name": "broken-extension",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "process_stdio",
    "adapter_family": "javascript-stdio-adapter",
    "entrypoint": "stdin/stdout::invoke",
    "source_language": "javascript",
    "command": "node",
    "args_json": "[\"index.js\"]",
    "process_timeout_ms": "15000",
    "loong_extension_contract": "process_stdio_json_line_v1",
    "loong_extension_facets_json": "[\"events\",\"commands\",\"resources\"]",
    "loong_extension_methods_json": "not-json",
    "loong_extension_events_json": "[\"session_start\"]",
    "loong_extension_host_actions_json": "[]"
  }
}"#,
        )
        .expect("write runtime plugin manifest");

        let payload = serde_json::to_value(
            crate::plugins_cli::runtime_plugin_inventory_read_model(&config).await,
        )
        .expect("serialize runtime plugin inventory payload");
        let summary = &payload["native_extension_authoring_summary"];
        assert_eq!(summary["guided_plugins"], json!(1));
        assert_eq!(summary["plugins_with_metadata_issues"], json!(1));
        assert_eq!(summary["total_remediation_actions"], json!(4));
        assert_eq!(summary["runnable_action_count"], json!(3));
        assert_eq!(summary["allow_command_gated_action_count"], json!(1));
        assert_eq!(summary["action_roles"]["author"], json!(1));
        assert_eq!(summary["action_roles"]["verification"], json!(3));
        assert_eq!(summary["action_execution_kinds"]["manual_edit"], json!(1));
        assert_eq!(summary["action_execution_kinds"]["read_only_cli"], json!(2));
        assert_eq!(
            summary["action_execution_kinds"]["governed_smoke_probe"],
            json!(1)
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn doctor_runtime_plugin_inventory_json_payload_auto_discovers_project_local_loong_extensions()
     {
        let root = browser_companion_temp_dir("runtime-plugins-auto-discovery");
        let _cwd = DoctorCliCurrentDirGuard::set(&root);
        let mut config = runtime_plugins_test_config(&root, true);
        config.runtime_plugins.roots = vec!["   ".to_owned()];
        let runtime_root = root.join(".loong/extensions");
        std::fs::create_dir_all(runtime_root.join("search")).expect("create runtime root");
        std::fs::write(
            runtime_root.join("search").join("loong.plugin.json"),
            r#"{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "auto-discovered-extension",
  "provider_id": "auto-discovered-extension",
  "connector_name": "auto-discovered-extension",
  "capabilities": ["InvokeConnector"],
  "metadata": {
    "bridge_kind": "process_stdio",
    "adapter_family": "python-stdio-adapter",
    "entrypoint": "stdin/stdout::invoke",
    "source_language": "python",
    "command": "python3",
    "args_json": "[\"index.py\"]",
    "process_timeout_ms": "5000"
  }
}"#,
        )
        .expect("write runtime plugin manifest");

        let payload = serde_json::to_value(
            crate::plugins_cli::runtime_plugin_inventory_read_model(&config).await,
        )
        .expect("serialize runtime plugin inventory payload");

        assert_eq!(payload["available"], json!(true));
        assert_eq!(payload["returned_results"], json!(1));
        let plugin = payload["results"]
            .as_array()
            .and_then(|plugins| {
                plugins.iter().find(|plugin| {
                    plugin["plugin_id"].as_str() == Some("auto-discovered-extension")
                })
            })
            .expect("auto-discovered extension should be present");
        assert_eq!(plugin["plugin_id"], json!("auto-discovered-extension"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn doctor_runtime_plugin_inventory_json_payload_prefers_project_local_loong_extensions_over_global_duplicates()
     {
        let root = browser_companion_temp_dir("runtime-plugins-auto-discovery-precedence");
        let home = browser_companion_temp_dir("runtime-plugins-auto-discovery-home");
        let mut env = ScopedEnv::new();
        env.set("HOME", &home);
        let _cwd = DoctorCliCurrentDirGuard::set(&root);
        let mut config = runtime_plugins_test_config(&root, true);
        config.runtime_plugins.roots = vec!["   ".to_owned()];

        let project_root = root.join(".loong/extensions/search");
        let global_root = home.join(".loong/agent/extensions/search");
        std::fs::create_dir_all(&project_root).expect("create project runtime root");
        std::fs::create_dir_all(&global_root).expect("create global runtime root");

        std::fs::write(
            project_root.join("loong.plugin.json"),
            r#"{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "shared-extension",
  "provider_id": "project-extension",
  "connector_name": "project-extension",
  "capabilities": ["InvokeConnector"],
  "summary": "Project-local extension",
  "metadata": {
    "bridge_kind": "process_stdio",
    "adapter_family": "python-stdio-adapter",
    "entrypoint": "stdin/stdout::invoke",
    "source_language": "python",
    "command": "python3",
    "args_json": "[\"index.py\"]",
    "process_timeout_ms": "5000"
  }
}"#,
        )
        .expect("write project-local runtime plugin manifest");
        std::fs::write(
            global_root.join("loong.plugin.json"),
            r#"{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "shared-extension",
  "provider_id": "global-extension",
  "connector_name": "global-extension",
  "capabilities": ["InvokeConnector"],
  "summary": "Global extension",
  "metadata": {
    "bridge_kind": "process_stdio",
    "adapter_family": "python-stdio-adapter",
    "entrypoint": "stdin/stdout::invoke",
    "source_language": "python",
    "command": "python3",
    "args_json": "[\"index.py\"]",
    "process_timeout_ms": "5000"
  }
}"#,
        )
        .expect("write global runtime plugin manifest");

        let payload = serde_json::to_value(
            crate::plugins_cli::runtime_plugin_inventory_read_model(&config).await,
        )
        .expect("serialize runtime plugin inventory payload");

        assert_eq!(payload["available"], json!(true));
        assert_eq!(payload["roots_source"], json!("auto_discovered"));
        assert_eq!(payload["returned_results"], json!(1));
        assert_eq!(payload["shadowed_plugin_ids"], json!(["shared-extension"]));
        assert_eq!(
            payload["discovery_guidance"]["precedence_rule"],
            json!("project_local_over_global")
        );
        assert_eq!(
            payload["discovery_guidance"]["recommended_action"],
            json!("review_global_duplicate")
        );
        assert_eq!(
            payload["discovery_guidance"]["shadowed_conflicts"][0]["plugin_id"],
            json!("shared-extension")
        );
        assert_eq!(
            payload["discovery_guidance"]["discovery_actions"][0]["kind"],
            json!("inspect_effective_package")
        );
        assert_eq!(
            payload["discovery_guidance"]["discovery_actions"][0]["role"],
            json!("operator")
        );
        assert!(
            payload["discovery_guidance"]["discovery_actions"][0]["command"]
                .as_str()
                .is_some_and(|command| command.contains("loong plugins doctor --root"))
        );
        assert!(
            payload["discovery_guidance"]["discovery_actions"]
                .as_array()
                .is_some_and(|actions| actions.iter().any(|action| {
                    action["kind"] == json!("compare_shadowed_manifests")
                        && action["command"]
                            .as_str()
                            .is_some_and(|command| command.contains("git diff --no-index"))
                }))
        );
        assert!(
            payload["discovery_guidance"]["shadowed_conflicts"][0]["effective_source_path"]
                .as_str()
                .is_some_and(|path| path.ends_with(".loong/extensions/search/loong.plugin.json"))
        );
        assert!(
            payload["discovery_guidance"]["shadowed_conflicts"][0]["shadowed_source_paths"][0]
                .as_str()
                .is_some_and(
                    |path| path.ends_with(".loong/agent/extensions/search/loong.plugin.json")
                )
        );
        let plugin = payload["results"]
            .as_array()
            .and_then(|plugins| {
                plugins
                    .iter()
                    .find(|plugin| plugin["plugin_id"].as_str() == Some("shared-extension"))
            })
            .expect("shared extension should be present");
        assert!(
            plugin["source_path"]
                .as_str()
                .is_some_and(|path| path.ends_with(".loong/extensions/search/loong.plugin.json")),
            "project-local descriptor should win the precedence race: {plugin:#?}"
        );

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&home).ok();
    }

    #[tokio::test]
    async fn doctor_runtime_plugin_inventory_discovery_guidance_yields_review_next_steps() {
        let root = browser_companion_temp_dir("runtime-plugins-auto-discovery-next-steps");
        let home = browser_companion_temp_dir("runtime-plugins-auto-discovery-next-steps-home");
        let mut env = ScopedEnv::new();
        env.set("HOME", &home);
        let _cwd = DoctorCliCurrentDirGuard::set(&root);
        let mut config = runtime_plugins_test_config(&root, true);
        config.runtime_plugins.roots = vec!["   ".to_owned()];

        let project_root = root.join(".loong/extensions/search");
        let global_root = home.join(".loong/agent/extensions/search");
        std::fs::create_dir_all(&project_root).expect("create project runtime root");
        std::fs::create_dir_all(&global_root).expect("create global runtime root");

        std::fs::write(
            project_root.join("loong.plugin.json"),
            r#"{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "shared-extension",
  "provider_id": "project-extension",
  "connector_name": "project-extension",
  "capabilities": ["InvokeConnector"],
  "summary": "Project-local extension",
  "metadata": {
    "bridge_kind": "process_stdio",
    "adapter_family": "python-stdio-adapter",
    "entrypoint": "stdin/stdout::invoke",
    "source_language": "python",
    "command": "python3",
    "args_json": "[\"index.py\"]",
    "process_timeout_ms": "5000"
  }
}"#,
        )
        .expect("write project-local runtime plugin manifest");
        std::fs::write(
            global_root.join("loong.plugin.json"),
            r#"{
  "api_version": "v1alpha1",
  "version": "1.0.0",
  "plugin_id": "shared-extension",
  "provider_id": "global-extension",
  "connector_name": "global-extension",
  "capabilities": ["InvokeConnector"],
  "summary": "Global extension",
  "metadata": {
    "bridge_kind": "process_stdio",
    "adapter_family": "python-stdio-adapter",
    "entrypoint": "stdin/stdout::invoke",
    "source_language": "python",
    "command": "python3",
    "args_json": "[\"index.py\"]",
    "process_timeout_ms": "5000"
  }
}"#,
        )
        .expect("write global runtime plugin manifest");

        let inventory = crate::plugins_cli::runtime_plugin_inventory_read_model(&config).await;
        let next_steps = crate::runtime_plugin_discovery::build_runtime_plugin_discovery_next_steps(
            inventory.discovery_guidance.as_ref(),
        );

        assert!(next_steps.iter().any(|step| {
            step.contains("Inspect the effective project-local package for shared-extension")
                && step.contains(".loong/extensions/search")
        }));
        assert!(next_steps.iter().any(|step| {
            step.contains("Inspect the shadowed package for shared-extension")
                && step.contains(".loong/agent/extensions/search")
        }));

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn build_doctor_next_steps_guides_runtime_plugin_enablement_when_disabled() {
        let checks = vec![DoctorCheck {
            name: "runtime plugins runtime".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "enabled=false supported_bridges=- supported_adapter_families=- roots=/tmp/runtime-plugins scanned_roots=0".to_owned(),
        }];
        let config = mvp::config::LoongConfig::default();

        let next_steps =
            build_doctor_next_steps(&checks, Path::new("/tmp/loong.toml"), &config, false);

        assert!(
            next_steps.iter().any(|step| {
                step == "Enable runtime plugins by setting [runtime_plugins].enabled = true, then re-run diagnostics: loong doctor --config '/tmp/loong.toml'"
            }),
            "doctor should surface an explicit runtime-plugin enablement step: {next_steps:#?}"
        );
        assert!(
            next_steps
                .iter()
                .all(|step| { !step.starts_with("Inspect runtime plugin inventory:") }),
            "disabled runtime plugins should not suggest inventory inspection before enablement: {next_steps:#?}"
        );
    }
}
