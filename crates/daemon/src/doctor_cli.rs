use std::collections::BTreeMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use kernel::probe_jsonl_audit_journal_runtime_ready;
use loongclaw_app as mvp;
use loongclaw_contracts::SecretRef;
use loongclaw_spec::CliResult;
use serde_json::json;

use crate::provider_credential_policy;
use crate::provider_model_probe_policy;

#[derive(Debug, Clone)]
pub struct DoctorCommandOptions {
    pub config: Option<String>,
    pub fix: bool,
    pub json: bool,
    pub skip_model_probe: bool,
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
    let (config_path, mut config) = mvp::config::load(options.config.as_deref())?;
    let mut checks = Vec::new();
    let mut fixes = Vec::new();
    let mut config_mutated = false;

    config_mutated |= maybe_apply_provider_env_fix(&mut config, options.fix, &mut fixes);
    config_mutated |= maybe_apply_channel_env_fix(&mut config, options.fix, &mut fixes);

    let has_provider_credentials = mvp::provider::provider_auth_ready(&config).await;
    checks.push(provider_credentials_doctor_check(
        &config,
        has_provider_credentials,
    ));

    checks.push(provider_transport_doctor_check(&config.provider));

    if options.skip_model_probe {
        checks.push(DoctorCheck {
            name: "provider model probe".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "skipped by --skip-model-probe".to_owned(),
        });
    } else if !has_provider_credentials {
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

    if config
        .tools
        .file_root
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        checks.push(DoctorCheck {
            name: "tool file root policy".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "tools.file_root is empty (falls back to current working directory)".to_owned(),
        });
        if options.fix {
            let suggested_root = mvp::config::default_loongclaw_home()
                .join("workspace")
                .display()
                .to_string();
            config.tools.file_root = Some(suggested_root.clone());
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
    let effective_tool_root = config.tools.resolved_file_root();
    checks.push(check_directory_ready(
        "tool file root",
        &effective_tool_root,
        options.fix,
        &mut fixes,
        "create tool file root",
    ));
    checks.extend(collect_browser_companion_doctor_checks(&config).await);

    checks.extend(check_feishu_integration(&config, options.fix, &mut fixes));
    checks.extend(check_channel_surfaces(&config));
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
    let next_steps = build_doctor_next_steps(&checks, &config_path, &config, options.fix);
    if options.json {
        let payload = json!({
            "ok": summary.fail == 0,
            "config": config_path.display().to_string(),
            "summary": {
                "ok": summary.pass,
                "warn": summary.warn,
                "fail": summary.fail
            },
            "checks": checks.iter().map(|check| {
                json!({
                    "name": check.name,
                    "level": check_level_json(check.level),
                    "detail": check.detail
                })
            }).collect::<Vec<_>>(),
            "fix_requested": options.fix,
            "applied_fixes": fixes,
            "next_steps": next_steps,
        });
        let encoded = serde_json::to_string_pretty(&payload)
            .map_err(|error| format!("serialize doctor output failed: {error}"))?;
        println!("{encoded}");
        return Ok(());
    }

    print_doctor_checks(&checks);
    if options.fix {
        if fixes.is_empty() {
            println!("applied fixes: none");
        } else {
            println!("applied fixes:");
            for fix in &fixes {
                println!("- {fix}");
            }
        }
    }
    println!(
        "doctor summary: {} ok, {} warn, {} fail",
        summary.pass, summary.warn, summary.fail
    );
    if !next_steps.is_empty() {
        println!("next actions:");
        for step in &next_steps {
            println!("- {step}");
        }
    }

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

fn check_channel_surfaces(config: &mvp::config::LoongClawConfig) -> Vec<DoctorCheck> {
    let snapshots = mvp::channel::channel_status_snapshots(config);
    build_channel_surface_checks(&snapshots)
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

fn durable_audit_target_issue(path: &Path) -> Option<String> {
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
    config: &mvp::config::LoongClawConfig,
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

    let store = mvp::feishu::FeishuTokenStore::new(sqlite_path);
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
        let inventory =
            match mvp::feishu::inspect_grants_for_account(&store, resolved.account.id.as_str()) {
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
        let required_scopes = config.feishu_integration.trimmed_default_scopes();
        let Some(latest) = inventory.grants.first() else {
            continue;
        };
        let effective_grant = inventory.effective_grant();
        let effective_status =
            mvp::feishu::auth::summarize_grant_status(effective_grant, now_s, &required_scopes);

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
                    "configured_account={} account={} cannot determine effective scope coverage until a selected grant exists; run `{}`",
                    resolved.configured_account_id,
                    resolved.account.id,
                    crate::feishu_support::feishu_auth_select_command_hint(
                        resolved.configured_account_id.as_str(),
                    )
                )
            },
        });
        let doc_write_status = mvp::feishu::summarize_doc_write_scope_status(effective_grant);
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
                if doc_write_status.ready {
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
        let write_status = mvp::feishu::summarize_message_write_scope_status(effective_grant);
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
                if write_status.ready {
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
    config: &mut mvp::config::LoongClawConfig,
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
    config: &mut mvp::config::LoongClawConfig,
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
    config: &mvp::config::LoongClawConfig,
    has_provider_credentials: bool,
) -> DoctorCheck {
    let provider_label = crate::provider_presentation::active_provider_detail_label(config);
    if has_provider_credentials {
        return DoctorCheck {
            name: "provider credentials".to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: format!("{provider_label}: provider credentials are available"),
        };
    }

    let hints = provider_credential_policy::provider_credential_env_hints(&config.provider);
    let mut detail = if hints.is_empty() {
        "provider credentials are missing".to_owned()
    } else {
        format!(
            "provider credentials are missing (try env: {})",
            hints.join(", ")
        )
    };
    if let Some(hint) = config.provider.auth_guidance_hint() {
        detail.push(' ');
        detail.push_str(hint.as_str());
    }
    DoctorCheck {
        name: "provider credentials".to_owned(),
        level: DoctorCheckLevel::Warn,
        detail: format!("{provider_label}: {detail}"),
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
    config: &mvp::config::LoongClawConfig,
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
    config: &mvp::config::LoongClawConfig,
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
    config: &mvp::config::LoongClawConfig,
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

fn print_doctor_checks(checks: &[DoctorCheck]) {
    println!("doctor checks:");
    let width = checks
        .iter()
        .map(|check| check.name.len())
        .max()
        .unwrap_or(0);
    for check in checks {
        println!(
            "{} {:width$}  {}",
            check_level_marker(check.level),
            check.name,
            check.detail,
            width = width
        );
    }
}

fn check_level_marker(level: DoctorCheckLevel) -> &'static str {
    match level {
        DoctorCheckLevel::Pass => "[OK]",
        DoctorCheckLevel::Warn => "[WARN]",
        DoctorCheckLevel::Fail => "[FAIL]",
    }
}

fn check_level_json(level: DoctorCheckLevel) -> &'static str {
    match level {
        DoctorCheckLevel::Pass => "ok",
        DoctorCheckLevel::Warn => "warn",
        DoctorCheckLevel::Fail => "fail",
    }
}

fn build_doctor_next_steps(
    checks: &[DoctorCheck],
    config_path: &Path,
    config: &mvp::config::LoongClawConfig,
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

fn build_doctor_next_steps_with_path_env(
    checks: &[DoctorCheck],
    config_path: &Path,
    config: &mvp::config::LoongClawConfig,
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
        is_channel_catalog_action(action)
    });
    push_first_matching_action(&mut prioritized, &actions, |action| {
        is_general_browser_preview_action(action)
    });

    for action in actions {
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
    #[cfg(unix)]
    use std::ffi::OsString;
    use std::fs::Permissions;
    #[cfg(unix)]
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    #[cfg(unix)]
    use std::sync::MutexGuard;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static FEISHU_TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    use super::*;
    use crate::test_support::ScopedEnv;
    use mvp::channel::{
        ChannelOperationHealth, ChannelOperationRuntime, ChannelOperationStatus,
        ChannelStatusSnapshot,
    };

    fn browser_companion_temp_dir(label: &str) -> PathBuf {
        static NEXT_TEMP_DIR_SEED: AtomicU64 = AtomicU64::new(1);
        let seed = NEXT_TEMP_DIR_SEED.fetch_add(1, Ordering::Relaxed);
        let temp_dir = std::env::temp_dir().join(format!(
            "loongclaw-browser-companion-doctor-{label}-{}-{seed}",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp_dir).expect("create browser companion temp dir");
        temp_dir
    }

    #[cfg(unix)]
    fn write_browser_companion_script(script_path: &Path, body: &str) {
        let mut file = std::fs::File::create(script_path).expect("create browser companion script");
        file.write_all(body.as_bytes())
            .expect("write browser companion script");
        let mut permissions = file.metadata().expect("script metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(script_path, permissions).expect("chmod browser companion script");
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

    #[cfg(unix)]
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
            let key = "LOONGCLAW_BROWSER_COMPANION_READY";
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
            let key = "LOONGCLAW_BROWSER_COMPANION_READY";
            match self.saved_ready.take() {
                Some(value) => set_browser_companion_env_var(key, &value.to_string_lossy()),
                None => remove_browser_companion_env_var(key),
            }
        }
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
        let config = mvp::config::LoongClawConfig::default();
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
    fn channel_doctor_checks_report_enabled_channels_from_registry() {
        let mut config = mvp::config::LoongClawConfig::default();
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
        let mut config = mvp::config::LoongClawConfig::default();
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
        assert_eq!(
            config.feishu.verification_token_env.as_deref(),
            Some("FEISHU_VERIFICATION_TOKEN")
        );
        assert_eq!(
            config.feishu.encrypt_key_env.as_deref(),
            Some("FEISHU_ENCRYPT_KEY")
        );
        assert_eq!(
            config.matrix.access_token_env.as_deref(),
            Some("MATRIX_ACCESS_TOKEN")
        );
        assert_eq!(fixes.len(), 6);
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
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.api_key_env = None;
        config.provider.oauth_access_token_env = None;

        let mut fixes = Vec::new();
        let changed = maybe_apply_provider_env_fix(&mut config, true, &mut fixes);

        assert!(changed);
        assert_eq!(
            config.provider.oauth_access_token,
            Some(loongclaw_contracts::SecretRef::Env {
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
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.api_key = Some(loongclaw_contracts::SecretRef::Inline(
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
            Some(loongclaw_contracts::SecretRef::Inline(
                "inline-secret".to_owned(),
            ))
        );
        assert_eq!(config.provider.api_key_env, None);
        assert!(fixes.is_empty());
    }

    #[test]
    fn provider_env_fix_does_not_overwrite_file_backed_api_key() {
        let mut config = mvp::config::LoongClawConfig::default();
        let credential_path = PathBuf::from("/tmp/openai-api-key.txt");
        config.provider.api_key = Some(loongclaw_contracts::SecretRef::File {
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
            Some(loongclaw_contracts::SecretRef::File {
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
        let mut config = mvp::config::LoongClawConfig::default();
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
        let mut config = mvp::config::LoongClawConfig::default();
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
        let mut config = mvp::config::LoongClawConfig::default();
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
        let mut config = mvp::config::LoongClawConfig::default();
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
        let mut config = mvp::config::LoongClawConfig::default();
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
        let mut config = mvp::config::LoongClawConfig::default();
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
        let mut config = mvp::config::LoongClawConfig::default();
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
        let mut config = mvp::config::LoongClawConfig::default();
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
            Path::new("/tmp/loongclaw.toml"),
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
        let mut config = mvp::config::LoongClawConfig::default();
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
            Path::new("/tmp/loongclaw.toml"),
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
    fn build_doctor_next_steps_guides_durable_audit_when_in_memory() {
        let checks = vec![DoctorCheck {
            name: "audit retention".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "audit.mode=in_memory; security-critical audit evidence is lost on restart"
                .to_owned(),
        }];
        let config_path = PathBuf::from("/tmp/loongclaw.toml");
        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            &config_path,
            &mvp::config::LoongClawConfig::default(),
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
        let config_path = PathBuf::from("/tmp/loongclaw.toml");
        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            &config_path,
            &mvp::config::LoongClawConfig::default(),
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

    #[cfg(unix)]
    #[test]
    fn audit_retention_doctor_check_fails_when_parent_directory_is_not_writable() {
        let temp_dir = browser_companion_temp_dir("audit-target-parent-readonly");
        let readonly_dir = temp_dir.join("readonly-audit");
        std::fs::create_dir_all(&readonly_dir).expect("create readonly audit directory");
        let original_permissions = std::fs::metadata(&readonly_dir)
            .expect("readonly audit directory metadata")
            .permissions();
        let mut permissions = original_permissions.clone();
        permissions.set_mode(0o555);
        std::fs::set_permissions(&readonly_dir, permissions)
            .expect("mark audit directory readonly");
        let _permission_restore =
            PermissionRestore::new(readonly_dir.clone(), original_permissions);

        let journal_path = readonly_dir.join("events.jsonl");
        let check = audit_retention_doctor_check(&mvp::config::AuditConfig {
            mode: mvp::config::AuditMode::Fanout,
            path: journal_path.display().to_string(),
            retain_in_memory: true,
        });

        assert_eq!(check.name, "audit retention");
        assert_eq!(check.level, DoctorCheckLevel::Fail);
        assert!(check.detail.contains("runtime open + lock probe failed"));
    }

    #[cfg(unix)]
    #[test]
    fn audit_retention_doctor_check_fails_when_missing_parent_chain_is_not_creatable() {
        let temp_dir = browser_companion_temp_dir("audit-target-missing-parent-chain");
        let readonly_dir = temp_dir.join("readonly-audit");
        std::fs::create_dir_all(&readonly_dir).expect("create readonly audit directory");
        let original_permissions = std::fs::metadata(&readonly_dir)
            .expect("readonly audit directory metadata")
            .permissions();
        let mut permissions = original_permissions.clone();
        permissions.set_mode(0o555);
        std::fs::set_permissions(&readonly_dir, permissions)
            .expect("mark audit directory readonly");
        let _permission_restore =
            PermissionRestore::new(readonly_dir.clone(), original_permissions);

        let journal_path = readonly_dir.join("nested").join("events.jsonl");
        let check = audit_retention_doctor_check(&mvp::config::AuditConfig {
            mode: mvp::config::AuditMode::Fanout,
            path: journal_path.display().to_string(),
            retain_in_memory: true,
        });

        assert_eq!(check.name, "audit retention");
        assert_eq!(check.level, DoctorCheckLevel::Fail);
        assert!(check.detail.contains("runtime open + lock probe failed"));
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
                "loongclaw-doctor-feishu-{label}-{}-{nanos}-{sequence}.sqlite3",
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
        let mut config = mvp::config::LoongClawConfig::default();
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
        let mut config = mvp::config::LoongClawConfig::default();
        config.feishu.enabled = true;
        config.feishu.app_id = Some(SecretRef::Inline("cli_a1b2c3".to_owned()));
        config.feishu.app_secret = Some(SecretRef::Inline("app-secret".to_owned()));
        config.feishu_integration.sqlite_path = unique_temp_feishu_db("ready-grant");
        let resolved = config
            .feishu
            .resolve_account(None)
            .expect("resolve default feishu account");
        let store =
            mvp::feishu::FeishuTokenStore::new(config.feishu_integration.resolved_sqlite_path());
        store
            .save_grant(&mvp::feishu::FeishuGrant {
                principal: mvp::feishu::FeishuUserPrincipal {
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
                scopes: mvp::feishu::FeishuGrantScopeSet::from_scopes(
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
                detail: "/tmp/loongclaw-memory is missing".to_owned(),
            },
        ];
        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            Path::new("/tmp/loongclaw.toml"),
            &mvp::config::LoongClawConfig::default(),
            false,
            Some(std::ffi::OsStr::new("")),
        );

        assert_eq!(
            next_steps[0],
            "Apply safe local repairs: loongclaw doctor --config '/tmp/loongclaw.toml' --fix"
        );
        assert!(
            next_steps.iter().any(|step| {
                step == "Set provider credentials in env: OPENAI_CODEX_OAUTH_TOKEN or OPENAI_API_KEY"
            }),
            "doctor should turn missing provider auth into a concrete next step: {next_steps:#?}"
        );
        assert!(
            next_steps.iter().any(|step| step
                == "Re-run diagnostics: loongclaw doctor --config '/tmp/loongclaw.toml'"),
            "doctor should tell the operator how to confirm the repair path: {next_steps:#?}"
        );
    }

    #[test]
    fn provider_credentials_doctor_check_adds_volcengine_auth_guidance() {
        let mut config = mvp::config::LoongClawConfig::default();
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
    fn build_doctor_next_steps_shell_quotes_config_paths_with_single_quotes() {
        let checks = vec![DoctorCheck {
            name: "memory path".to_owned(),
            level: DoctorCheckLevel::Fail,
            detail: "/tmp/loongclaw-memory is missing".to_owned(),
        }];
        let next_steps = build_doctor_next_steps(
            &checks,
            Path::new("/tmp/loongclaw's config.toml"),
            &mvp::config::LoongClawConfig::default(),
            false,
        );

        assert!(
            next_steps.iter().any(|step| {
                step == "Apply safe local repairs: loongclaw doctor --config '/tmp/loongclaw'\"'\"'s config.toml' --fix"
            }),
            "doctor should shell-quote config paths with single quotes in fix commands: {next_steps:#?}"
        );
        assert!(
            next_steps.iter().any(|step| {
                step == "Re-run diagnostics: loongclaw doctor --config '/tmp/loongclaw'\"'\"'s config.toml'"
            }),
            "doctor should shell-quote config paths with single quotes in rerun commands: {next_steps:#?}"
        );
    }

    #[test]
    fn build_doctor_next_steps_guides_browser_companion_repair() {
        let checks = vec![DoctorCheck {
            name: "browser companion install".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail: "command `loongclaw-browser-companion` was not found on PATH".to_owned(),
        }];
        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            Path::new("/tmp/loongclaw.toml"),
            &mvp::config::LoongClawConfig::default(),
            false,
            Some(std::ffi::OsStr::new("")),
        );

        assert!(
            next_steps.iter().any(|step| {
                step == "Install or expose the browser companion command on PATH, then re-run: loongclaw doctor --config '/tmp/loongclaw.toml'"
            }),
            "doctor should turn browser companion warnings into a concrete repair path: {next_steps:#?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn browser_companion_doctor_checks_warn_when_command_is_missing() {
        let _env_guard = BrowserCompanionEnvGuard::runtime_gate_closed();
        let mut config = mvp::config::LoongClawConfig::default();
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
        let temp_dir = browser_companion_temp_dir("version-mismatch");
        let script_path = temp_dir.join("browser-companion");
        write_browser_companion_script(
            &script_path,
            "#!/bin/sh\necho 'loongclaw-browser-companion 1.4.0'\n",
        );

        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.command = Some(script_path.display().to_string());
        config.tools.browser_companion.expected_version = Some("1.5.0".to_owned());

        let checks = collect_browser_companion_doctor_checks(&config).await;

        assert!(
            checks.iter().any(|check| {
                check.name == "browser companion install"
                    && check.level == DoctorCheckLevel::Warn
                    && check.detail.contains("expected_version=1.5.0")
                    && check
                        .detail
                        .contains("observed_version=loongclaw-browser-companion 1.4.0")
            }),
            "doctor should surface version mismatches for the managed companion lane: {checks:#?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn browser_companion_doctor_checks_warn_when_runtime_gate_is_closed() {
        let _env_guard = BrowserCompanionEnvGuard::runtime_gate_closed();
        let temp_dir = browser_companion_temp_dir("runtime-gate");
        let script_path = temp_dir.join("browser-companion");
        write_browser_companion_script(
            &script_path,
            "#!/bin/sh\necho 'loongclaw-browser-companion 1.5.0'\n",
        );

        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.command = Some(script_path.display().to_string());
        config.tools.browser_companion.expected_version = Some("1.5.0".to_owned());

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
        let temp_dir = browser_companion_temp_dir("runtime-ready");
        let script_path = temp_dir.join("browser-companion");
        write_browser_companion_script(
            &script_path,
            "#!/bin/sh\necho 'loongclaw-browser-companion 1.5.0'\n",
        );

        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.command = Some(script_path.display().to_string());
        config.tools.browser_companion.expected_version = Some("1.5.0".to_owned());

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
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "auto".to_owned();

        let next_steps =
            build_doctor_next_steps(&checks, Path::new("/tmp/loongclaw.toml"), &config, false);

        assert!(
            next_steps.iter().any(|step| {
                step == "Rerun onboarding and accept reviewed model `deepseek-chat`: loongclaw onboard --config '/tmp/loongclaw.toml'"
            }),
            "doctor should point reviewed providers back to onboarding when auto-model recovery needs an explicit reviewed default: {next_steps:#?}"
        );
        assert!(
            next_steps.iter().any(|step| {
                step == "Or set `provider.model` / `preferred_models` explicitly, then re-run diagnostics: loongclaw doctor --config '/tmp/loongclaw.toml'"
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
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "deepseek-chat".to_owned();

        let next_steps =
            build_doctor_next_steps(&checks, Path::new("/tmp/loongclaw.toml"), &config, false);

        assert!(
            next_steps.iter().any(|step| {
                step == "Retry provider probe only after credentials are ready: loongclaw doctor --config '/tmp/loongclaw.toml'"
            }),
            "warn-level explicit model recovery should still tell operators how to retry diagnostics: {next_steps:#?}"
        );
        assert!(
            next_steps.iter().any(|step| {
                step == "If your provider blocks model listing during setup, retry with: loongclaw doctor --config '/tmp/loongclaw.toml' --skip-model-probe"
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
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "auto".to_owned();
        config.provider.preferred_models =
            vec!["deepseek-chat".to_owned(), "deepseek-reasoner".to_owned()];

        let next_steps =
            build_doctor_next_steps(&checks, Path::new("/tmp/loongclaw.toml"), &config, false);

        assert!(
            next_steps.iter().any(|step| {
                step == "Retry provider probe only after credentials are ready: loongclaw doctor --config '/tmp/loongclaw.toml'"
            }),
            "warn-level preferred-model recovery should still tell operators how to retry diagnostics: {next_steps:#?}"
        );
        assert!(
            next_steps.iter().any(|step| {
                step == "If your provider blocks model listing during setup, retry with: loongclaw doctor --config '/tmp/loongclaw.toml' --skip-model-probe"
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
            Path::new("/tmp/loongclaw.toml"),
            &mvp::config::LoongClawConfig::default(),
            false,
            Some(std::ffi::OsStr::new("")),
        );

        assert!(
            next_steps.iter().any(|step| {
                step.contains("provider route")
                    && step.contains("loongclaw doctor --config '/tmp/loongclaw.toml'")
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
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "deepseek-chat".to_owned();

        let next_steps =
            build_doctor_next_steps(&checks, Path::new("/tmp/loongclaw.toml"), &config, false);

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
            Path::new("/tmp/loongclaw.toml"),
            &mvp::config::LoongClawConfig::default(),
            false,
            Some(std::ffi::OsStr::new("")),
        );

        assert!(
            next_steps.iter().any(|step| {
                step == "Get a first answer: loongclaw ask --config '/tmp/loongclaw.toml' --message 'Summarize this repository and suggest the best next step.'"
            }),
            "green doctor runs should hand the user into ask immediately: {next_steps:#?}"
        );
        assert!(
            next_steps.iter().any(|step| {
                step == "Continue in chat: loongclaw chat --config '/tmp/loongclaw.toml'"
            }),
            "green doctor runs should still advertise chat as the follow-up path: {next_steps:#?}"
        );
        assert!(
            next_steps.iter().any(|step| {
                step == "Open a channel: loongclaw channels --config '/tmp/loongclaw.toml'"
            }),
            "green doctor runs should surface the channel catalog when no service channel is enabled yet: {next_steps:#?}"
        );
        assert!(
            !next_steps.iter().any(|step| {
                step == "Optional browser preview: loongclaw skills enable-browser-preview --config '/tmp/loongclaw.toml'"
            }),
            "green doctor runs should prioritize the channel catalog ahead of optional browser preview when no service channel is enabled yet: {next_steps:#?}"
        );
        assert!(
            !next_steps.iter().any(|step| {
                step == "Install browser preview runtime: npm install -g agent-browser && agent-browser install"
            }),
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
            "# Browser Companion Preview\n\nUse agent-browser through shell.exec.\n",
        )
        .expect("write managed preview skill");
        let checks = vec![DoctorCheck {
            name: "provider credentials".to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: "provider credentials are available".to_owned(),
        }];
        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.file_root = Some(root.display().to_string());
        config.tools.shell_allow.push("agent-browser".to_owned());
        config.external_skills.enabled = true;
        config.external_skills.auto_expose_installed = true;
        config.external_skills.install_root = Some(install_root.display().to_string());

        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            Path::new("/tmp/loongclaw.toml"),
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
                step == "Optional browser preview: loongclaw skills enable-browser-preview --config '/tmp/loongclaw.toml'"
            }),
            "doctor should not fall back to the optional enable step after preview has already been configured: {next_steps:#?}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn build_doctor_next_steps_keeps_browser_preview_visible_when_channels_are_enabled() {
        let checks = vec![DoctorCheck {
            name: "provider credentials".to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: "provider credentials are available".to_owned(),
        }];
        let mut config = mvp::config::LoongClawConfig::default();
        config.telegram.enabled = true;

        let next_steps = build_doctor_next_steps_with_path_env(
            &checks,
            Path::new("/tmp/loongclaw.toml"),
            &config,
            false,
            Some(std::ffi::OsStr::new("")),
        );

        assert!(
            next_steps.iter().any(|step| {
                step == "Optional browser preview: loongclaw skills enable-browser-preview --config '/tmp/loongclaw.toml'"
            }),
            "doctor should keep a browser preview action visible even when channel actions are available: {next_steps:#?}"
        );
    }
}
