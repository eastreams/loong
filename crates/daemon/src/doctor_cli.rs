use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use loongclaw_app as mvp;
use loongclaw_spec::CliResult;
use serde_json::json;

#[derive(Debug, Clone)]
pub(crate) struct DoctorCommandOptions {
    pub config: Option<String>,
    pub fix: bool,
    pub json: bool,
    pub skip_model_probe: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DoctorCheckLevel {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone)]
struct DoctorCheck {
    name: String,
    level: DoctorCheckLevel,
    detail: String,
}

#[derive(Debug, Clone, Copy)]
struct DoctorSummary {
    pass: usize,
    warn: usize,
    fail: usize,
}

#[derive(Debug, Clone, Copy)]
struct DoctorChannelCheckSpec {
    config_name: &'static str,
    runtime_name: Option<&'static str>,
}

pub(crate) async fn run_doctor_cli(options: DoctorCommandOptions) -> CliResult<()> {
    let (config_path, mut config) = mvp::config::load(options.config.as_deref())?;
    let mut checks = Vec::new();
    let mut fixes = Vec::new();
    let mut config_mutated = false;

    config_mutated |= maybe_apply_provider_env_fix(&mut config, options.fix, &mut fixes);
    config_mutated |= maybe_apply_channel_env_fix(&mut config, options.fix, &mut fixes);

    let has_provider_credentials = config.provider.authorization_header().is_some();
    if has_provider_credentials {
        checks.push(DoctorCheck {
            name: "provider credentials".to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: "provider credentials are available".to_owned(),
        });
    } else {
        let mut hints = Vec::new();
        if let Some(key) = config.provider.api_key_env.as_deref().map(str::trim) {
            if !key.is_empty() {
                hints.push(key.to_owned());
            }
        }
        if let Some(default_key) = config.provider.kind.default_api_key_env() {
            if !hints.iter().any(|existing| existing == default_key) {
                hints.push(default_key.to_owned());
            }
        }
        let detail = if hints.is_empty() {
            "provider credentials are missing".to_owned()
        } else {
            format!(
                "provider credentials are missing (try env: {})",
                hints.join(", ")
            )
        };
        checks.push(DoctorCheck {
            name: "provider credentials".to_owned(),
            level: DoctorCheckLevel::Warn,
            detail,
        });
    }

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
            Err(error) => checks.push(DoctorCheck {
                name: "provider model probe".to_owned(),
                level: DoctorCheckLevel::Fail,
                detail: error,
            }),
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

    checks.extend(check_channel_surfaces(&config));

    if options.fix && config_mutated {
        let path = config_path
            .to_str()
            .ok_or_else(|| format!("config path is not valid UTF-8: {}", config_path.display()))?;
        mvp::config::write(Some(path), &config, true)?;
    }

    let summary = summarize_checks(&checks);
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
            let Some(spec) = doctor_check_spec(snapshot.id, operation.id) else {
                continue;
            };
            checks.push(DoctorCheck {
                name: scoped_doctor_check_name(spec.config_name, snapshot, scoped),
                level: doctor_check_level_for_health(operation.health),
                detail: operation.detail.clone(),
            });

            if let Some(runtime_name) = spec.runtime_name {
                if operation.health == mvp::channel::ChannelOperationHealth::Ready {
                    checks.push(build_channel_runtime_check(
                        scoped_doctor_check_name(runtime_name, snapshot, scoped).as_str(),
                        operation,
                    ));
                }
            }
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

fn doctor_check_spec(channel_id: &str, operation_id: &str) -> Option<DoctorChannelCheckSpec> {
    match (channel_id, operation_id) {
        ("telegram", "serve") => Some(DoctorChannelCheckSpec {
            config_name: "telegram channel",
            runtime_name: Some("telegram channel runtime"),
        }),
        ("feishu", "send") => Some(DoctorChannelCheckSpec {
            config_name: "feishu channel",
            runtime_name: None,
        }),
        ("feishu", "serve") => Some(DoctorChannelCheckSpec {
            config_name: "feishu webhook verification",
            runtime_name: Some("feishu webhook runtime"),
        }),
        _ => None,
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
        runtime
            .account_label
            .as_deref()
            .unwrap_or("-"),
        runtime
            .account_id
            .as_deref()
            .unwrap_or("-"),
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
    let mut changed = false;
    if config
        .provider
        .api_key_env
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        if let Some(default_key) = config.provider.kind.default_api_key_env() {
            if fix {
                config.provider.api_key_env = Some(default_key.to_owned());
                fixes.push(format!("set provider.api_key_env={default_key}"));
                changed = true;
            }
        }
    }
    changed
}

fn maybe_apply_channel_env_fix(
    config: &mut mvp::config::LoongClawConfig,
    fix: bool,
    fixes: &mut Vec<String>,
) -> bool {
    if !fix {
        return false;
    }
    let mut changed = false;

    changed |= ensure_env_binding(
        &mut config.telegram.bot_token_env,
        "TELEGRAM_BOT_TOKEN",
        fixes,
        "set telegram.bot_token_env",
    );
    changed |= ensure_env_binding(
        &mut config.feishu.app_id_env,
        "FEISHU_APP_ID",
        fixes,
        "set feishu.app_id_env",
    );
    changed |= ensure_env_binding(
        &mut config.feishu.app_secret_env,
        "FEISHU_APP_SECRET",
        fixes,
        "set feishu.app_secret_env",
    );
    changed |= ensure_env_binding(
        &mut config.feishu.verification_token_env,
        "FEISHU_VERIFICATION_TOKEN",
        fixes,
        "set feishu.verification_token_env",
    );
    changed |= ensure_env_binding(
        &mut config.feishu.encrypt_key_env,
        "FEISHU_ENCRYPT_KEY",
        fixes,
        "set feishu.encrypt_key_env",
    );

    changed
}

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

#[cfg(test)]
pub(crate) fn resolve_secret_value(inline: Option<&str>, env_key: Option<&str>) -> Option<String> {
    use std::env;

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

#[cfg(test)]
mod tests {
    use super::*;
    use mvp::channel::{
        ChannelOperationHealth, ChannelOperationRuntime, ChannelOperationStatus,
        ChannelStatusSnapshot,
    };

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
            transport: "feishu_openapi_webhook",
            compiled: true,
            enabled: true,
            api_base_url: Some("https://open.feishu.cn".to_owned()),
            notes: Vec::new(),
            operations: vec![ChannelOperationStatus {
                id: "serve",
                label: "webhook reply server",
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
                check.name == "feishu webhook runtime"
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

        assert!(checks
            .iter()
            .any(|check| check.name == "telegram channel [ops]"));
        assert!(checks
            .iter()
            .any(|check| check.name == "telegram channel runtime [personal]"));
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
}
