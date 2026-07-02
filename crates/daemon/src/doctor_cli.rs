use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use clap::Subcommand;
use kernel::{probe_jsonl_audit_journal_runtime_ready, verify_jsonl_audit_journal};
use loong_app as mvp;
use loong_contracts::SecretRef;
use loong_spec::CliResult;
use serde::Serialize;
use serde_json::json;

use crate::plugin_bridge_account_summary::plugin_bridge_account_summary;
use crate::provider::credential_policy as provider_credential_policy;
use crate::provider::model_probe_policy as provider_model_probe_policy;

mod next_steps;
mod presentation;
use self::next_steps::{
    ManagedBridgeRuntimeAttention, build_doctor_next_steps_with_channel_surfaces_and_path_env,
    doctor_checks_json_payload,
};
#[cfg(test)]
use self::next_steps::{build_doctor_next_steps, build_doctor_next_steps_with_path_env};

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

const DOCTOR_CLI_JSON_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize)]
struct DoctorCliJsonSchema {
    version: u32,
    surface: &'static str,
    purpose: &'static str,
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
    mvp::runtime_env::initialize_runtime_environment(&config, Some(config_path.as_path()));
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
                        crate::provider::route_diagnostics::collect_provider_route_probe(
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
    checks.extend(collect_runtime_plugins_doctor_checks(&config));

    checks.extend(check_feishu_integration(&config, options.fix, &mut fixes));
    let channel_inventory = mvp::channel::channel_inventory(&config);
    let channel_surface_checks = collect_channel_surface_checks(&channel_inventory);
    checks.extend(channel_surface_checks);
    let path_env = env::var_os("PATH");

    if options.fix && config_mutated {
        let path = config_path
            .to_str()
            .ok_or_else(|| format!("config path is not valid UTF-8: {}", config_path.display()))?;
        mvp::config::write(Some(path), &config, true)?;
    }

    let summary = presentation::summarize_checks(&checks);
    let next_steps = build_doctor_next_steps_with_channel_surfaces_and_path_env(
        &checks,
        &config_path,
        &config,
        &channel_inventory.channel_surfaces,
        options.fix,
        path_env.as_deref(),
    );
    if options.json {
        let checks = doctor_checks_json_payload(&checks, &channel_inventory.channel_surfaces);
        let payload = json!({
            "schema": doctor_cli_json_schema(),
            "ok": summary.fail == 0,
            "config": config_path.display().to_string(),
            "summary": {
                "ok": summary.pass,
                "warn": summary.warn,
                "fail": summary.fail
            },
            "checks": checks,
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
        presentation::render_doctor_text(
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

fn doctor_cli_json_schema() -> DoctorCliJsonSchema {
    DoctorCliJsonSchema {
        version: DOCTOR_CLI_JSON_SCHEMA_VERSION,
        surface: "doctor",
        purpose: "runtime_health_diagnostics",
    }
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

fn collect_runtime_plugins_doctor_checks(config: &mvp::config::LoongConfig) -> Vec<DoctorCheck> {
    let state = crate::collect_runtime_snapshot_runtime_plugins_state(config);
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
            "managed bridge discovery is unavailable because skills.install_root is not configured"
                .to_owned()
        }
        mvp::channel::ChannelPluginBridgeDiscoveryStatus::ScanFailed => {
            let scan_issue = discovery
                .scan_issue
                .as_deref()
                .map(crate::render_line_safe_text_value)
                .unwrap_or_else(|| "unknown scan failure".to_owned());
            let detail = format!(
                "managed bridge discovery failed under {managed_install_root}: {scan_issue}"
            );

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
            let compatible_plugin_ids = render_managed_plugin_bridge_compatible_plugin_ids(
                &discovery.compatible_plugin_ids,
            );
            let ambiguity_status = discovery
                .ambiguity_status
                .map(|status| status.as_str())
                .unwrap_or("-");
            let incomplete_plugins = discovery.incomplete_plugins;
            let incompatible_plugins = discovery.incompatible_plugins;
            let rendered_plugins =
                render_managed_plugin_bridge_discovery_plugins(&discovery.plugins);
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

mod render;

use render::{
    managed_bridge_duplicate_plugin_id_counts, managed_bridge_plugin_label,
    render_managed_bridge_compatible_plugin_labels, render_managed_bridge_configured_plugin_labels,
    render_managed_plugin_bridge_compatible_plugin_ids,
    render_managed_plugin_bridge_discovery_plugins, render_runtime_incident_summary,
    render_u32_list,
};

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

fn managed_bridge_runtime_serve_control_command(
    attention: &ManagedBridgeRuntimeAttention<'_>,
    config_path_display: &str,
    duplicate_cleanup: bool,
) -> Option<String> {
    let family =
        mvp::channel::resolve_channel_catalog_command_family_descriptor(attention.channel_id)?;
    let command = crate::cli_handoff::format_subcommand_with_config(
        family.serve.command,
        config_path_display,
    );
    let control_flag = if duplicate_cleanup {
        "--stop-duplicates"
    } else {
        "--stop"
    };
    let account_id = attention.account_ids.iter().next().cloned();
    let needs_explicit_account = attention.account_ids.len() == 1;

    if !needs_explicit_account {
        return Some(format!("{command} {control_flag}"));
    }

    let account_id = account_id?;
    Some(format!(
        "{command} {control_flag} --account {}",
        crate::cli_handoff::shell_quote_argument(&account_id)
    ))
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

    let recent_incidents = runtime
        .recent_incidents
        .iter()
        .map(|incident| {
            let kind = match incident.kind {
                mvp::channel::ChannelOperationRuntimeIncidentKind::Failure => "failure",
                mvp::channel::ChannelOperationRuntimeIncidentKind::Recovery => "recovery",
                mvp::channel::ChannelOperationRuntimeIncidentKind::DuplicateReclaim => {
                    "duplicate_reclaim"
                }
            };
            format!("{kind}@{}", incident.at_ms)
        })
        .collect::<Vec<_>>();
    let detail_tail = format!(
        "account={} account_id={} pid={} busy={} active_runs={} consecutive_failures={} instance_count={} running_instances={} stale_instances={} last_run_activity_at={} last_heartbeat_at={} last_failure_at={} last_recovery_at={} last_error={} duplicate_owner_pids={} last_duplicate_reclaim_at={} last_duplicate_reclaim_cleanup_owner_pids={} recent_incidents={}",
        runtime.account_label.as_deref().unwrap_or("-"),
        runtime.account_id.as_deref().unwrap_or("-"),
        runtime
            .pid
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned()),
        runtime.busy,
        runtime.active_runs,
        runtime.consecutive_failures,
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
        runtime
            .last_failure_at
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned()),
        runtime
            .last_recovery_at
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned()),
        runtime.last_error.as_deref().unwrap_or("-"),
        render_u32_list(&runtime.duplicate_owner_pids),
        runtime
            .last_duplicate_reclaim_at
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".to_owned()),
        render_u32_list(&runtime.last_duplicate_reclaim_cleanup_owner_pids),
        render_runtime_incident_summary(recent_incidents.as_slice()),
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

        if runtime.consecutive_failures > 0 {
            return DoctorCheck {
                name: name.to_owned(),
                level: DoctorCheckLevel::Warn,
                detail: format!("runtime is retrying after transient failures ({detail_tail})"),
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
    probe: &crate::provider::route_diagnostics::ProviderRouteProbe,
) -> DoctorCheck {
    DoctorCheck {
        name: crate::provider::route_diagnostics::PROVIDER_ROUTE_PROBE_CHECK_NAME.to_owned(),
        level: match probe.level {
            crate::provider::route_diagnostics::ProviderRouteProbeLevel::Pass => {
                DoctorCheckLevel::Pass
            }
            crate::provider::route_diagnostics::ProviderRouteProbeLevel::Warn => {
                DoctorCheckLevel::Warn
            }
            crate::provider::route_diagnostics::ProviderRouteProbeLevel::Fail => {
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
    let provider_label = crate::provider::presentation::active_provider_detail_label(config);
    let status = crate::provider::credentials_guidance::provider_credential_status(
        &config.provider,
        has_provider_credentials,
    );

    DoctorCheck {
        name: crate::provider::credentials_guidance::PROVIDER_CREDENTIALS_LABEL.to_owned(),
        level: if status.is_ready() {
            DoctorCheckLevel::Pass
        } else {
            DoctorCheckLevel::Warn
        },
        detail: format!("{provider_label}: {}", status.detail),
    }
}

fn web_search_provider_doctor_check(config: &mvp::config::LoongConfig) -> DoctorCheck {
    if !config.tools.web_search.enabled {
        return DoctorCheck {
            name: crate::access_terms::QUERY_SEARCH_PROVIDER_LABEL.to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: "tools.web_search.enabled=false".to_owned(),
        };
    }

    let provider_status = crate::query_search_surface::query_search_provider_status(config);

    if provider_status.credential_available {
        return DoctorCheck {
            name: crate::access_terms::QUERY_SEARCH_PROVIDER_LABEL.to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: provider_status.ready_detail(),
        };
    }

    DoctorCheck {
        name: crate::access_terms::QUERY_SEARCH_PROVIDER_LABEL.to_owned(),
        level: DoctorCheckLevel::Warn,
        detail: provider_status.blocked_detail(false),
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

fn check_level_json(level: DoctorCheckLevel) -> &'static str {
    match level {
        DoctorCheckLevel::Pass => "ok",
        DoctorCheckLevel::Warn => "warn",
        DoctorCheckLevel::Fail => "fail",
    }
}

fn doctor_render_string_list(values: &[String]) -> String {
    if values.is_empty() {
        return "-".to_owned();
    }

    crate::render_line_safe_text_values(values.iter().map(String::as_str), ",")
}

fn doctor_ready_for_first_turn(checks: &[DoctorCheck]) -> bool {
    checks
        .iter()
        .all(|check| check.level != DoctorCheckLevel::Fail)
        && checks.iter().any(|check| {
            check.name == "provider credentials" && check.level == DoctorCheckLevel::Pass
        })
}

#[cfg(test)]
mod tests;
