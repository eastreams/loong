use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::Local;
use loongclaw_app as mvp;
use loongclaw_spec::CliResult;

#[derive(Debug, Clone)]
pub(crate) struct OnboardCommandOptions {
    pub output: Option<String>,
    pub force: bool,
    pub non_interactive: bool,
    pub accept_risk: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub personality: Option<String>,
    pub memory_profile: Option<String>,
    pub system_prompt: Option<String>,
    pub skip_model_probe: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnboardCheckLevel {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone)]
struct OnboardCheck {
    name: &'static str,
    level: OnboardCheckLevel,
    detail: String,
}

pub(crate) async fn run_onboard_cli(options: OnboardCommandOptions) -> CliResult<()> {
    validate_non_interactive_risk_gate(options.non_interactive, options.accept_risk)?;
    let using_prompt_override = options
        .system_prompt
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let total_steps = if using_prompt_override { 5 } else { 6 };

    if !options.non_interactive && !options.accept_risk {
        println!("Security warning:");
        println!("- LoongClaw can invoke tools and read local files when enabled.");
        println!("- Keep credentials in environment variables, not in prompts.");
        println!("- Prefer allowlist-style tool policy for shared environments.");
        if !prompt_confirm("Continue onboarding?", false)? {
            return Err("onboarding cancelled: risk acknowledgement declined".to_owned());
        }
    }

    let output_path = options
        .output
        .as_deref()
        .map(mvp::config::expand_path)
        .unwrap_or_else(mvp::config::default_config_path);
    let force_write = resolve_force_write(&output_path, &options)?;

    let mut config = mvp::config::LoongClawConfig::default();

    if !options.non_interactive {
        print_step_header(1, total_steps, "provider");
    }
    let selected_provider = resolve_provider_selection(&options, &config)?;
    config.provider.kind = selected_provider;
    let profile = config.provider.kind.profile();
    config.provider.base_url = profile.base_url.to_owned();
    config.provider.chat_completions_path = profile.chat_completions_path.to_owned();

    if !options.non_interactive {
        print_step_header(2, total_steps, "model");
    }
    let selected_model = resolve_model_selection(&options, &config)?;
    config.provider.model = selected_model;

    if !options.non_interactive {
        print_step_header(3, total_steps, "credential env");
    }
    let default_api_key_env = provider_default_api_key_env(config.provider.kind).to_owned();
    let selected_api_key_env = resolve_api_key_env_selection(&options, default_api_key_env)?;
    config.provider.api_key_env = if selected_api_key_env.trim().is_empty() {
        None
    } else {
        Some(selected_api_key_env)
    };

    if using_prompt_override {
        if !options.non_interactive {
            print_step_header(4, total_steps, "system prompt override");
        }
        if let Some(system_prompt) = resolve_system_prompt_selection(&options, &config)? {
            config.cli.prompt_pack_id = None;
            config.cli.personality = None;
            config.cli.system_prompt_addendum = None;
            config.cli.system_prompt = system_prompt;
        }
        if !options.non_interactive {
            print_step_header(5, total_steps, "memory profile");
        }
        config.memory.profile = resolve_memory_profile_selection(&options, &config)?;
    } else {
        if !options.non_interactive {
            print_step_header(4, total_steps, "personality");
        }
        let personality = resolve_personality_selection(&options, &config)?;
        config.cli.prompt_pack_id = Some(mvp::prompt::DEFAULT_PROMPT_PACK_ID.to_owned());
        config.cli.personality = Some(personality);

        if !options.non_interactive {
            print_step_header(5, total_steps, "prompt addendum");
        }
        config.cli.system_prompt_addendum = resolve_prompt_addendum_selection(&options, &config)?;
        config.cli.refresh_native_system_prompt();

        if !options.non_interactive {
            print_step_header(6, total_steps, "memory profile");
        }
        config.memory.profile = resolve_memory_profile_selection(&options, &config)?;
    }

    let checks = run_preflight_checks(&config, options.skip_model_probe).await;
    print_preflight_checks(&checks);

    let credential_ok = checks
        .iter()
        .find(|check| check.name == "provider credentials")
        .is_some_and(|check| check.level == OnboardCheckLevel::Pass);
    let has_failures = checks
        .iter()
        .any(|check| check.level == OnboardCheckLevel::Fail);
    let has_warnings = checks
        .iter()
        .any(|check| check.level == OnboardCheckLevel::Warn);

    if options.non_interactive {
        if !credential_ok {
            return Err(format!(
                "onboard preflight failed: provider credentials missing. set {} in env or pass --api-key-env with a populated variable",
                config
                    .provider
                    .api_key_env
                    .clone()
                    .unwrap_or_else(|| "OPENAI_API_KEY".to_owned())
            ));
        }
        if has_failures {
            return Err(
                "onboard preflight failed. rerun with --skip-model-probe if your provider blocks model listing during setup"
                    .to_owned(),
            );
        }
    } else if (has_failures || has_warnings)
        && !prompt_confirm("Some checks are not green. Continue anyway?", false)?
    {
        return Err("onboarding cancelled: unresolved preflight warnings".to_owned());
    }

    let path = mvp::config::write(options.output.as_deref(), &config, force_write)?;
    #[cfg(feature = "memory-sqlite")]
    let memory_path = {
        let mem_config =
            mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        mvp::memory::ensure_memory_db_ready(Some(config.memory.resolved_sqlite_path()), &mem_config)
            .map_err(|error| format!("failed to bootstrap sqlite memory: {error}"))?
    };

    println!("onboarding complete");
    println!("- config: {}", path.display());
    println!("- provider: {}", provider_kind_id(config.provider.kind));
    println!("- model: {}", config.provider.model);
    if let Some(pack_id) = config.cli.prompt_pack_id() {
        println!("- prompt pack: {pack_id}");
    } else {
        println!("- prompt mode: inline override");
    }
    if let Some(personality) = config.cli.personality {
        println!("- personality: {}", prompt_personality_id(personality));
    }
    println!(
        "- memory profile: {}",
        memory_profile_id(config.memory.profile)
    );
    if let Some(api_env) = config.provider.api_key_env.as_deref() {
        println!("- credential env: {api_env}");
    }
    #[cfg(feature = "memory-sqlite")]
    println!("- sqlite memory: {}", memory_path.display());
    println!("next step: loongclaw chat --config {}", path.display());
    Ok(())
}

fn resolve_provider_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
) -> CliResult<mvp::config::ProviderKind> {
    if options.non_interactive {
        if let Some(provider_raw) = options.provider.as_deref() {
            return parse_provider_kind(provider_raw).ok_or_else(|| {
                format!(
                    "unsupported --provider value \"{provider_raw}\". supported: {}",
                    supported_provider_list()
                )
            });
        }
        return Ok(config.provider.kind);
    }

    let default_provider = options
        .provider
        .as_deref()
        .and_then(parse_provider_kind)
        .unwrap_or(config.provider.kind);
    loop {
        println!("Provider options: {}", supported_provider_list());
        let input = prompt_with_default("Provider", provider_kind_id(default_provider))?;
        if let Some(kind) = parse_provider_kind(&input) {
            return Ok(kind);
        }
        println!("Invalid provider: {input}");
    }
}

fn resolve_model_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
) -> CliResult<String> {
    if options.non_interactive {
        if let Some(model) = options.model.as_deref() {
            let trimmed = model.trim();
            if trimmed.is_empty() {
                return Err("--model cannot be empty".to_owned());
            }
            return Ok(trimmed.to_owned());
        }
        return Ok(config.provider.model.clone());
    }

    let default_model = options
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(config.provider.model.as_str());
    let value = prompt_with_default("Model", default_model)?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("model cannot be empty".to_owned());
    }
    Ok(trimmed.to_owned())
}

fn resolve_api_key_env_selection(
    options: &OnboardCommandOptions,
    default_api_key_env: String,
) -> CliResult<String> {
    if options.non_interactive {
        return Ok(options
            .api_key_env
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(default_api_key_env.as_str())
            .to_owned());
    }
    let initial = options
        .api_key_env
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default_api_key_env.as_str());
    let value = prompt_with_default("API key env var", initial)?;
    Ok(value.trim().to_owned())
}

fn resolve_system_prompt_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
) -> CliResult<Option<String>> {
    if options.non_interactive {
        return Ok(options
            .system_prompt
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned));
    }
    let initial = options
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(config.cli.system_prompt.as_str());
    let value = prompt_with_default("CLI system prompt", initial)?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(trimmed.to_owned()))
}

fn resolve_personality_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
) -> CliResult<mvp::prompt::PromptPersonality> {
    if options.non_interactive {
        if let Some(personality_raw) = options.personality.as_deref() {
            return parse_prompt_personality(personality_raw).ok_or_else(|| {
                format!(
                    "unsupported --personality value \"{personality_raw}\". supported: {}",
                    supported_personality_list()
                )
            });
        }
        return Ok(config.cli.resolved_personality());
    }

    let default_personality = options
        .personality
        .as_deref()
        .and_then(parse_prompt_personality)
        .unwrap_or_else(|| config.cli.resolved_personality());
    loop {
        println!("Personality options: {}", supported_personality_list());
        let input = prompt_with_default("Personality", prompt_personality_id(default_personality))?;
        if let Some(personality) = parse_prompt_personality(&input) {
            return Ok(personality);
        }
        println!("Invalid personality: {input}");
    }
}

fn resolve_prompt_addendum_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
) -> CliResult<Option<String>> {
    if options.non_interactive {
        return Ok(config.cli.system_prompt_addendum.clone());
    }
    prompt_optional(
        "Prompt addendum (blank keeps current, '-' clears)",
        config.cli.system_prompt_addendum.as_deref(),
    )
}

fn resolve_memory_profile_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
) -> CliResult<mvp::config::MemoryProfile> {
    if options.non_interactive {
        if let Some(profile_raw) = options.memory_profile.as_deref() {
            return parse_memory_profile(profile_raw).ok_or_else(|| {
                format!(
                    "unsupported --memory-profile value \"{profile_raw}\". supported: {}",
                    supported_memory_profile_list()
                )
            });
        }
        return Ok(config.memory.profile);
    }

    let default_profile = options
        .memory_profile
        .as_deref()
        .and_then(parse_memory_profile)
        .unwrap_or(config.memory.profile);
    loop {
        println!(
            "Memory profile options: {}",
            supported_memory_profile_list()
        );
        let input = prompt_with_default("Memory profile", memory_profile_id(default_profile))?;
        if let Some(profile) = parse_memory_profile(&input) {
            return Ok(profile);
        }
        println!("Invalid memory profile: {input}");
    }
}

async fn run_preflight_checks(
    config: &mvp::config::LoongClawConfig,
    skip_model_probe: bool,
) -> Vec<OnboardCheck> {
    let mut checks = Vec::new();

    let api_key_env = config
        .provider
        .api_key_env
        .as_deref()
        .map(str::trim)
        .unwrap_or("");
    let has_credentials = if api_key_env.is_empty() {
        false
    } else {
        env::var(api_key_env)
            .ok()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    };

    if api_key_env.is_empty() {
        checks.push(OnboardCheck {
            name: "provider credentials",
            level: OnboardCheckLevel::Warn,
            detail: "provider.api_key_env is empty".to_owned(),
        });
    } else if has_credentials {
        checks.push(OnboardCheck {
            name: "provider credentials",
            level: OnboardCheckLevel::Pass,
            detail: format!("{api_key_env} is available"),
        });
    } else {
        checks.push(OnboardCheck {
            name: "provider credentials",
            level: OnboardCheckLevel::Warn,
            detail: format!("{api_key_env} is not set"),
        });
    }

    if skip_model_probe {
        checks.push(OnboardCheck {
            name: "provider model probe",
            level: OnboardCheckLevel::Warn,
            detail: "skipped by --skip-model-probe".to_owned(),
        });
    } else if !has_credentials {
        checks.push(OnboardCheck {
            name: "provider model probe",
            level: OnboardCheckLevel::Warn,
            detail: "skipped because credentials are missing".to_owned(),
        });
    } else {
        match mvp::provider::fetch_available_models(config).await {
            Ok(models) => checks.push(OnboardCheck {
                name: "provider model probe",
                level: OnboardCheckLevel::Pass,
                detail: format!("{} model(s) available", models.len()),
            }),
            Err(error) => checks.push(OnboardCheck {
                name: "provider model probe",
                level: OnboardCheckLevel::Fail,
                detail: error,
            }),
        }
    }

    let sqlite_path = config.memory.resolved_sqlite_path();
    let sqlite_parent = sqlite_path.parent().unwrap_or(Path::new("."));
    match fs::create_dir_all(sqlite_parent) {
        Ok(()) => checks.push(OnboardCheck {
            name: "memory path",
            level: OnboardCheckLevel::Pass,
            detail: format!("writable parent {}", sqlite_parent.display()),
        }),
        Err(error) => checks.push(OnboardCheck {
            name: "memory path",
            level: OnboardCheckLevel::Fail,
            detail: format!("failed to prepare {}: {error}", sqlite_parent.display()),
        }),
    }

    let file_root = config.tools.resolved_file_root();
    match fs::create_dir_all(&file_root) {
        Ok(()) => checks.push(OnboardCheck {
            name: "tool file root",
            level: OnboardCheckLevel::Pass,
            detail: file_root.display().to_string(),
        }),
        Err(error) => checks.push(OnboardCheck {
            name: "tool file root",
            level: OnboardCheckLevel::Fail,
            detail: format!("failed to create {}: {error}", file_root.display()),
        }),
    }

    checks
}

fn print_preflight_checks(checks: &[OnboardCheck]) {
    println!("preflight checks:");
    let name_width = checks
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
            width = name_width
        );
    }
}

fn print_step_header(step: usize, total: usize, title: &str) {
    println!();
    println!("[{step}/{total}] {title}");
}

fn check_level_marker(level: OnboardCheckLevel) -> &'static str {
    match level {
        OnboardCheckLevel::Pass => "[OK]",
        OnboardCheckLevel::Warn => "[WARN]",
        OnboardCheckLevel::Fail => "[FAIL]",
    }
}

fn prompt_with_default(label: &str, default: &str) -> CliResult<String> {
    print!("{label} [{default}]: ");
    io::stdout()
        .flush()
        .map_err(|error| format!("flush stdout failed: {error}"))?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|error| format!("read stdin failed: {error}"))?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(default.to_owned());
    }
    Ok(trimmed.to_owned())
}

fn prompt_confirm(message: &str, default: bool) -> CliResult<bool> {
    let suffix = if default { "[Y/n]" } else { "[y/N]" };
    print!("{message} {suffix}: ");
    io::stdout()
        .flush()
        .map_err(|error| format!("flush stdout failed: {error}"))?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|error| format!("read stdin failed: {error}"))?;
    let value = line.trim().to_ascii_lowercase();
    if value.is_empty() {
        return Ok(default);
    }
    Ok(matches!(value.as_str(), "y" | "yes"))
}

fn prompt_optional(label: &str, current: Option<&str>) -> CliResult<Option<String>> {
    let default = current.unwrap_or("none");
    print!("{label} [{default}]: ");
    io::stdout()
        .flush()
        .map_err(|error| format!("flush stdout failed: {error}"))?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|error| format!("read stdin failed: {error}"))?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(current.map(str::to_owned));
    }
    if trimmed == "-" {
        return Ok(None);
    }
    Ok(Some(trimmed.to_owned()))
}

pub(crate) fn validate_non_interactive_risk_gate(
    non_interactive: bool,
    accept_risk: bool,
) -> CliResult<()> {
    if non_interactive && !accept_risk {
        return Err(
            "non-interactive onboarding requires --accept-risk (explicit acknowledgement)"
                .to_owned(),
        );
    }
    Ok(())
}

pub(crate) fn parse_provider_kind(raw: &str) -> Option<mvp::config::ProviderKind> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "anthropic" | "anthropic_compatible" => Some(mvp::config::ProviderKind::Anthropic),
        "deepseek" | "deepseek_compatible" => Some(mvp::config::ProviderKind::Deepseek),
        "kimi" | "kimi_compatible" => Some(mvp::config::ProviderKind::Kimi),
        "kimi_coding" | "kimi_coding_compatible" => Some(mvp::config::ProviderKind::KimiCoding),
        "minimax" | "minimax_compatible" => Some(mvp::config::ProviderKind::Minimax),
        "ollama" | "ollama_compatible" => Some(mvp::config::ProviderKind::Ollama),
        "openai" | "openai_compatible" => Some(mvp::config::ProviderKind::Openai),
        "openrouter" | "openrouter_compatible" => Some(mvp::config::ProviderKind::Openrouter),
        "volcengine" | "volcengine_custom" | "volcengine_compatible" => {
            Some(mvp::config::ProviderKind::Volcengine)
        }
        "xai" | "xai_compatible" => Some(mvp::config::ProviderKind::Xai),
        "zai" | "zai_compatible" => Some(mvp::config::ProviderKind::Zai),
        "zhipu" | "zhipu_compatible" => Some(mvp::config::ProviderKind::Zhipu),
        _ => None,
    }
}

pub(crate) fn parse_prompt_personality(raw: &str) -> Option<mvp::prompt::PromptPersonality> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "calm_engineering" | "engineering" | "calm" => {
            Some(mvp::prompt::PromptPersonality::CalmEngineering)
        }
        "friendly_collab" | "friendly" | "collab" => {
            Some(mvp::prompt::PromptPersonality::FriendlyCollab)
        }
        "autonomous_executor" | "autonomous" | "executor" => {
            Some(mvp::prompt::PromptPersonality::AutonomousExecutor)
        }
        _ => None,
    }
}

pub(crate) fn parse_memory_profile(raw: &str) -> Option<mvp::config::MemoryProfile> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "window_only" | "window" => Some(mvp::config::MemoryProfile::WindowOnly),
        "window_plus_summary" | "summary" | "summary_window" => {
            Some(mvp::config::MemoryProfile::WindowPlusSummary)
        }
        "profile_plus_window" | "profile" | "profile_window" => {
            Some(mvp::config::MemoryProfile::ProfilePlusWindow)
        }
        _ => None,
    }
}

pub(crate) fn provider_default_api_key_env(kind: mvp::config::ProviderKind) -> &'static str {
    match kind {
        mvp::config::ProviderKind::Anthropic => "ANTHROPIC_API_KEY",
        mvp::config::ProviderKind::Deepseek => "DEEPSEEK_API_KEY",
        mvp::config::ProviderKind::Kimi => "MOONSHOT_API_KEY",
        mvp::config::ProviderKind::KimiCoding => "KIMI_CODING_API_KEY",
        mvp::config::ProviderKind::Minimax => "MINIMAX_API_KEY",
        mvp::config::ProviderKind::Ollama => "OLLAMA_API_KEY",
        mvp::config::ProviderKind::Openai => "OPENAI_API_KEY",
        mvp::config::ProviderKind::Openrouter => "OPENROUTER_API_KEY",
        mvp::config::ProviderKind::Volcengine => "ARK_API_KEY",
        mvp::config::ProviderKind::Xai => "XAI_API_KEY",
        mvp::config::ProviderKind::Zai => "ZAI_API_KEY",
        mvp::config::ProviderKind::Zhipu => "ZHIPU_API_KEY",
    }
}

pub(crate) fn provider_kind_id(kind: mvp::config::ProviderKind) -> &'static str {
    match kind {
        mvp::config::ProviderKind::Anthropic => "anthropic",
        mvp::config::ProviderKind::Deepseek => "deepseek",
        mvp::config::ProviderKind::Kimi => "kimi",
        mvp::config::ProviderKind::KimiCoding => "kimi_coding",
        mvp::config::ProviderKind::Minimax => "minimax",
        mvp::config::ProviderKind::Ollama => "ollama",
        mvp::config::ProviderKind::Openai => "openai",
        mvp::config::ProviderKind::Openrouter => "openrouter",
        mvp::config::ProviderKind::Volcengine => "volcengine",
        mvp::config::ProviderKind::Xai => "xai",
        mvp::config::ProviderKind::Zai => "zai",
        mvp::config::ProviderKind::Zhipu => "zhipu",
    }
}

pub(crate) fn prompt_personality_id(personality: mvp::prompt::PromptPersonality) -> &'static str {
    match personality {
        mvp::prompt::PromptPersonality::CalmEngineering => "calm_engineering",
        mvp::prompt::PromptPersonality::FriendlyCollab => "friendly_collab",
        mvp::prompt::PromptPersonality::AutonomousExecutor => "autonomous_executor",
    }
}

pub(crate) fn memory_profile_id(profile: mvp::config::MemoryProfile) -> &'static str {
    match profile {
        mvp::config::MemoryProfile::WindowOnly => "window_only",
        mvp::config::MemoryProfile::WindowPlusSummary => "window_plus_summary",
        mvp::config::MemoryProfile::ProfilePlusWindow => "profile_plus_window",
    }
}

fn supported_provider_list() -> &'static str {
    "openai, anthropic, openrouter, kimi, kimi_coding, minimax, ollama, volcengine, xai, zai, zhipu, deepseek"
}

fn supported_personality_list() -> &'static str {
    "calm_engineering, friendly_collab, autonomous_executor"
}

fn supported_memory_profile_list() -> &'static str {
    "window_only, window_plus_summary, profile_plus_window"
}

fn resolve_force_write(output_path: &Path, options: &OnboardCommandOptions) -> CliResult<bool> {
    if !output_path.exists() || options.force {
        return Ok(options.force);
    }

    if options.non_interactive {
        return Err(format!(
            "config {} already exists (use --force to overwrite)",
            output_path.display()
        ));
    }

    let existing_path = output_path.display().to_string();
    println!("Config file already exists: {}", existing_path);
    println!("Options:");
    println!("  [o] Overwrite (replace existing)");
    println!("  [b] Backup (rename existing to .bak-YYYYMMDD-HHMMSS)");
    println!("  [c] Cancel");
    loop {
        let choice = prompt_with_default("Your choice", "c")?;
        match choice.trim().to_ascii_lowercase().as_str() {
            "o" | "overwrite" => {
                return Ok(true);
            }
            "b" | "backup" => {
                let backup_path = resolve_backup_path(output_path)?;
                fs::rename(output_path, &backup_path)
                    .map_err(|e| format!("failed to backup config: {e}"))?;
                println!("Backed up existing config to: {}", backup_path.display());
                return Ok(false);
            }
            "c" | "cancel" => {
                return Err("onboarding cancelled: config file already exists".to_owned());
            }
            _ => {
                println!(
                    "Invalid choice. Please enter 'o' (overwrite), 'b' (backup), or 'c' (cancel)"
                );
            }
        }
    }
}

fn resolve_backup_path(original: &Path) -> CliResult<PathBuf> {
    let parent = original.parent().unwrap_or(Path::new("."));
    let file_stem = original
        .file_stem()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "config".to_owned());

    let timestamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
    Ok(parent.join(format!("{}.toml.bak-{}", file_stem, timestamp)))
}
