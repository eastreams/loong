use std::env;
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

pub(crate) async fn run_doctor_cli(options: DoctorCommandOptions) -> CliResult<()> {
    let (config_path, mut config) = mvp::config::load(options.config.as_deref())?;
    let mut checks = Vec::new();
    let mut fixes = Vec::new();
    let mut config_mutated = false;

    config_mutated |= maybe_apply_provider_env_fix(&mut config, options.fix, &mut fixes);
    config_mutated |= maybe_apply_channel_env_fix(&mut config, options.fix, &mut fixes);

    let has_provider_credentials = mvp::provider::provider_auth_ready(&config).await;
    if has_provider_credentials {
        checks.push(DoctorCheck {
            name: "provider credentials".to_owned(),
            level: DoctorCheckLevel::Pass,
            detail: "provider credentials are available".to_owned(),
        });
    } else {
        let hints = crate::onboard_cli::provider_credential_env_hints(&config.provider);
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

    checks.extend(collect_channel_doctor_checks(&config));

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
    let inventory = mvp::channel::channel_inventory(config);
    build_channel_surface_checks(&inventory.channel_surfaces)
}

fn build_channel_surface_checks(
    channel_surfaces: &[mvp::channel::ChannelSurface],
) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    for surface in channel_surfaces {
        let scoped = surface.configured_accounts.len() > 1;

        if scoped
            && let Some(default_snapshot) = surface.configured_accounts.iter().find(|snapshot| {
                snapshot.is_default_account
                    && snapshot.default_account_source
                        == mvp::config::ChannelDefaultAccountSelectionSource::Fallback
            })
        {
            checks.push(DoctorCheck {
                name: format!("{} default account policy", surface.catalog.id),
                level: DoctorCheckLevel::Warn,
                detail: format!(
                    "multiple configured accounts are using fallback default selection; omitting --account currently routes to `{}`. set default_account explicitly to avoid routing surprises",
                    default_snapshot.configured_account_label
                ),
            });
        }

        for snapshot in &surface.configured_accounts {
            for operation in &snapshot.operations {
                let Some(descriptor) = mvp::channel::resolve_channel_operation_descriptor(
                    surface.catalog.id,
                    operation.id,
                ) else {
                    continue;
                };
                let Some(spec) = descriptor.doctor else {
                    continue;
                };
                for check in spec.checks {
                    match check.trigger {
                        mvp::channel::ChannelDoctorCheckTrigger::OperationHealth => {
                            checks.push(DoctorCheck {
                                name: scoped_doctor_check_name(check.name, snapshot, scoped),
                                level: doctor_check_level_for_health(operation.health),
                                detail: operation.detail.clone(),
                            });
                        }
                        mvp::channel::ChannelDoctorCheckTrigger::ReadyRuntime => {
                            if operation.health == mvp::channel::ChannelOperationHealth::Ready {
                                checks.push(build_channel_runtime_check(
                                    scoped_doctor_check_name(check.name, snapshot, scoped).as_str(),
                                    operation,
                                ));
                            }
                        }
                    }
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
    let Some(binding) =
        crate::onboard_cli::preferred_provider_credential_env_binding(&config.provider)
    else {
        return false;
    };
    match binding.field {
        crate::onboard_cli::ProviderCredentialEnvField::ApiKey => ensure_env_binding(
            &mut config.provider.api_key_env,
            &binding.env_name,
            fixes,
            "set provider.api_key_env",
        ),
        crate::onboard_cli::ProviderCredentialEnvField::OAuthAccessToken => ensure_env_binding(
            &mut config.provider.oauth_access_token_env,
            &binding.env_name,
            fixes,
            "set provider.oauth_access_token_env",
        ),
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

fn collect_channel_doctor_checks(config: &mvp::config::LoongClawConfig) -> Vec<DoctorCheck> {
    check_channel_surfaces(config)
}

pub(crate) fn resolve_secret_value(inline: Option<&str>, env_key: Option<&str>) -> Option<String> {
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
        CHANNEL_OPERATION_SERVE_ID, ChannelOperationHealth, ChannelOperationRuntime,
        ChannelOperationStatus, ChannelStatusSnapshot, ChannelSurface,
    };

    fn runtime_channel_surface_from_catalog(
        id: &'static str,
        configured_accounts: Vec<ChannelStatusSnapshot>,
    ) -> ChannelSurface {
        let default_configured_account_id = configured_accounts
            .iter()
            .find(|snapshot| snapshot.is_default_account)
            .map(|snapshot| snapshot.configured_account_id.clone());
        let catalog = mvp::channel::resolve_channel_catalog_entry(id)
            .expect("channel catalog entry for test surface");
        ChannelSurface {
            catalog,
            configured_accounts,
            default_configured_account_id,
        }
    }

    fn runtime_operation_status(
        channel_id: &str,
        operation_id: &str,
        health: ChannelOperationHealth,
        detail: &str,
        runtime: Option<ChannelOperationRuntime>,
    ) -> ChannelOperationStatus {
        let descriptor =
            mvp::channel::resolve_channel_operation_descriptor(channel_id, operation_id)
                .expect("channel operation descriptor for doctor test");
        ChannelOperationStatus {
            id: descriptor.operation.id,
            label: descriptor.operation.label,
            command: descriptor.operation.command,
            health,
            detail: detail.to_owned(),
            issues: Vec::new(),
            runtime,
        }
    }

    #[test]
    fn runtime_channel_surface_from_catalog_uses_registry_metadata() {
        let surface = runtime_channel_surface_from_catalog("feishu", Vec::new());
        let family = mvp::channel::resolve_channel_catalog_command_family_descriptor("feishu")
            .expect("feishu catalog family");

        assert_eq!(surface.catalog.id, "feishu");
        assert_eq!(surface.catalog.label, "Feishu/Lark");
        assert_eq!(surface.catalog.aliases, vec!["lark"]);
        assert_eq!(surface.catalog.transport, "feishu_openapi_webhook");
        assert_eq!(
            surface
                .catalog
                .operations
                .iter()
                .map(|operation| operation.command)
                .collect::<Vec<_>>(),
            vec![family.send.command, family.serve.command]
        );
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
    fn channel_doctor_checks_omit_disabled_channels() {
        let checks = collect_channel_doctor_checks(&mvp::config::LoongClawConfig::default());
        assert!(
            checks.is_empty(),
            "disabled optional channels should not generate doctor warnings by default: {checks:#?}"
        );
    }

    #[test]
    fn channel_doctor_checks_report_enabled_channels_from_registry() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.telegram.enabled = true;
        config.telegram.bot_token = Some("123456:test-token".to_owned());
        config.feishu.enabled = true;
        config.feishu.app_id = Some("cli_a1b2c3".to_owned());
        config.feishu.app_secret = Some("feishu-secret".to_owned());

        let checks = collect_channel_doctor_checks(&config);
        let names = checks.iter().map(|check| check.name).collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "telegram channel",
                "feishu channel",
                "feishu webhook verification"
            ]
        );
        assert!(
            checks
                .iter()
                .any(|check| check.name == "telegram channel"
                    && check.level == DoctorCheckLevel::Pass),
            "telegram doctor check should come from the channel registry: {checks:#?}"
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
        assert_eq!(fixes.len(), 5);
    }

    #[test]
    fn provider_credential_env_hints_prioritize_oauth_defaults() {
        let hints = crate::onboard_cli::provider_credential_env_hints(
            &mvp::config::ProviderConfig::default(),
        );

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
            config.provider.oauth_access_token_env.as_deref(),
            Some("OPENAI_CODEX_OAUTH_TOKEN")
        );
        assert_eq!(config.provider.api_key_env, None);
        assert_eq!(
            fixes,
            vec!["set provider.oauth_access_token_env=OPENAI_CODEX_OAUTH_TOKEN".to_owned()]
        );
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
    fn build_channel_surface_checks_warns_when_ready_serve_operation_is_not_running() {
        let surfaces = vec![runtime_channel_surface_from_catalog(
            "telegram",
            vec![ChannelStatusSnapshot {
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
                operations: vec![runtime_operation_status(
                    "telegram",
                    CHANNEL_OPERATION_SERVE_ID,
                    ChannelOperationHealth::Ready,
                    "ready",
                    Some(ChannelOperationRuntime {
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
                )],
            }],
        )];

        let checks = build_channel_surface_checks(&surfaces);

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
        let surfaces = vec![runtime_channel_surface_from_catalog(
            "feishu",
            vec![ChannelStatusSnapshot {
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
                operations: vec![runtime_operation_status(
                    "feishu",
                    CHANNEL_OPERATION_SERVE_ID,
                    ChannelOperationHealth::Ready,
                    "ready",
                    Some(ChannelOperationRuntime {
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
                )],
            }],
        )];

        let checks = build_channel_surface_checks(&surfaces);

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
        let surfaces = vec![runtime_channel_surface_from_catalog(
            "telegram",
            vec![ChannelStatusSnapshot {
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
                operations: vec![runtime_operation_status(
                    "telegram",
                    CHANNEL_OPERATION_SERVE_ID,
                    ChannelOperationHealth::Ready,
                    "ready",
                    Some(ChannelOperationRuntime {
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
                )],
            }],
        )];

        let checks = build_channel_surface_checks(&surfaces);

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
    fn build_channel_surface_checks_scopes_names_for_multi_account_surfaces() {
        let surfaces = vec![runtime_channel_surface_from_catalog(
            "telegram",
            vec![
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
                    operations: vec![runtime_operation_status(
                        "telegram",
                        CHANNEL_OPERATION_SERVE_ID,
                        ChannelOperationHealth::Ready,
                        "ready",
                        Some(ChannelOperationRuntime {
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
                    )],
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
                    operations: vec![runtime_operation_status(
                        "telegram",
                        CHANNEL_OPERATION_SERVE_ID,
                        ChannelOperationHealth::Ready,
                        "ready",
                        Some(ChannelOperationRuntime {
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
                    )],
                },
            ],
        )];

        let checks = build_channel_surface_checks(&surfaces);

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
        let surfaces = vec![runtime_channel_surface_from_catalog(
            "telegram",
            vec![
                ChannelStatusSnapshot {
                    id: "telegram",
                    configured_account_id: "alerts".to_owned(),
                    configured_account_label: "alerts".to_owned(),
                    is_default_account: true,
                    default_account_source:
                        mvp::config::ChannelDefaultAccountSelectionSource::Fallback,
                    label: "Telegram",
                    aliases: Vec::new(),
                    transport: "telegram_bot_api_polling",
                    compiled: true,
                    enabled: true,
                    api_base_url: Some("https://api.telegram.org".to_owned()),
                    notes: vec!["default_account_source=fallback".to_owned()],
                    operations: vec![runtime_operation_status(
                        "telegram",
                        CHANNEL_OPERATION_SERVE_ID,
                        ChannelOperationHealth::Ready,
                        "ready",
                        None,
                    )],
                },
                ChannelStatusSnapshot {
                    id: "telegram",
                    configured_account_id: "work".to_owned(),
                    configured_account_label: "work".to_owned(),
                    is_default_account: false,
                    default_account_source:
                        mvp::config::ChannelDefaultAccountSelectionSource::Fallback,
                    label: "Telegram",
                    aliases: Vec::new(),
                    transport: "telegram_bot_api_polling",
                    compiled: true,
                    enabled: true,
                    api_base_url: Some("https://api.telegram.org".to_owned()),
                    notes: vec!["default_account_source=fallback".to_owned()],
                    operations: vec![runtime_operation_status(
                        "telegram",
                        CHANNEL_OPERATION_SERVE_ID,
                        ChannelOperationHealth::Ready,
                        "ready",
                        None,
                    )],
                },
            ],
        )];

        let checks = build_channel_surface_checks(&surfaces);

        assert!(checks.iter().any(|check| {
            check.name == "telegram default account policy"
                && check.level == DoctorCheckLevel::Warn
                && check.detail.contains("alerts")
                && check.detail.contains("default_account")
        }));
    }

    #[test]
    fn build_channel_surface_checks_ignores_stub_surfaces_without_accounts() {
        let surfaces = vec![ChannelSurface {
            catalog: mvp::channel::resolve_channel_catalog_entry("discord")
                .expect("discord catalog entry"),
            configured_accounts: Vec::new(),
            default_configured_account_id: None,
        }];

        let checks = build_channel_surface_checks(&surfaces);

        assert!(checks.is_empty());
    }
}
