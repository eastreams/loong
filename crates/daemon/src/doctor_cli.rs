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
    name: &'static str,
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

    let has_provider_credentials = config.provider.authorization_header().is_some();
    if has_provider_credentials {
        checks.push(DoctorCheck {
            name: "provider credentials",
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
            name: "provider credentials",
            level: DoctorCheckLevel::Warn,
            detail,
        });
    }

    if options.skip_model_probe {
        checks.push(DoctorCheck {
            name: "provider model probe",
            level: DoctorCheckLevel::Warn,
            detail: "skipped by --skip-model-probe".to_owned(),
        });
    } else if !has_provider_credentials {
        checks.push(DoctorCheck {
            name: "provider model probe",
            level: DoctorCheckLevel::Warn,
            detail: "skipped because credentials are missing".to_owned(),
        });
    } else {
        match mvp::provider::fetch_available_models(&config).await {
            Ok(models) => checks.push(DoctorCheck {
                name: "provider model probe",
                level: DoctorCheckLevel::Pass,
                detail: format!("{} model(s) available", models.len()),
            }),
            Err(error) => checks.push(DoctorCheck {
                name: "provider model probe",
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
            name: "tool file root policy",
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
            name: "tool file root policy",
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

    checks.push(check_telegram_channel(&config));
    let feishu_checks = check_feishu_channel(&config);
    checks.extend(feishu_checks);

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
                name,
                level: DoctorCheckLevel::Pass,
                detail: directory.display().to_string(),
            };
        }
        return DoctorCheck {
            name,
            level: DoctorCheckLevel::Fail,
            detail: format!("{} exists but is not a directory", directory.display()),
        };
    }

    if !fix {
        return DoctorCheck {
            name,
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
                name,
                level: DoctorCheckLevel::Pass,
                detail: format!("created {}", directory.display()),
            }
        }
        Err(error) => DoctorCheck {
            name,
            level: DoctorCheckLevel::Fail,
            detail: format!("failed to create {}: {error}", directory.display()),
        },
    }
}

fn check_telegram_channel(config: &mvp::config::LoongClawConfig) -> DoctorCheck {
    if !config.telegram.enabled {
        return DoctorCheck {
            name: "telegram channel",
            level: DoctorCheckLevel::Warn,
            detail: "disabled".to_owned(),
        };
    }
    let token = resolve_secret_value(
        config.telegram.bot_token.as_deref(),
        config.telegram.bot_token_env.as_deref(),
    );
    if token.is_some() {
        DoctorCheck {
            name: "telegram channel",
            level: DoctorCheckLevel::Pass,
            detail: "bot token resolved".to_owned(),
        }
    } else {
        DoctorCheck {
            name: "telegram channel",
            level: DoctorCheckLevel::Fail,
            detail: "enabled but bot token is missing (telegram.bot_token or env)".to_owned(),
        }
    }
}

fn check_feishu_channel(config: &mvp::config::LoongClawConfig) -> Vec<DoctorCheck> {
    if !config.feishu.enabled {
        return vec![DoctorCheck {
            name: "feishu channel",
            level: DoctorCheckLevel::Warn,
            detail: "disabled".to_owned(),
        }];
    }

    let app_id = resolve_secret_value(
        config.feishu.app_id.as_deref(),
        config.feishu.app_id_env.as_deref(),
    );
    let app_secret = resolve_secret_value(
        config.feishu.app_secret.as_deref(),
        config.feishu.app_secret_env.as_deref(),
    );
    let mut checks = Vec::new();
    if app_id.is_some() && app_secret.is_some() {
        checks.push(DoctorCheck {
            name: "feishu channel",
            level: DoctorCheckLevel::Pass,
            detail: "app credentials resolved".to_owned(),
        });
    } else {
        let mut missing = Vec::new();
        if app_id.is_none() {
            missing.push("app_id");
        }
        if app_secret.is_none() {
            missing.push("app_secret");
        }
        checks.push(DoctorCheck {
            name: "feishu channel",
            level: DoctorCheckLevel::Fail,
            detail: format!("enabled but missing {}", missing.join(", ")),
        });
    }

    let verification_token = resolve_secret_value(
        config.feishu.verification_token.as_deref(),
        config.feishu.verification_token_env.as_deref(),
    );
    let encrypt_key = resolve_secret_value(
        config.feishu.encrypt_key.as_deref(),
        config.feishu.encrypt_key_env.as_deref(),
    );
    if verification_token.is_none() && encrypt_key.is_none() {
        checks.push(DoctorCheck {
            name: "feishu webhook verification",
            level: DoctorCheckLevel::Warn,
            detail: "verification token and encrypt key are both missing".to_owned(),
        });
    } else {
        checks.push(DoctorCheck {
            name: "feishu webhook verification",
            level: DoctorCheckLevel::Pass,
            detail: "verification token or encrypt key is configured".to_owned(),
        });
    }
    checks
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
}
