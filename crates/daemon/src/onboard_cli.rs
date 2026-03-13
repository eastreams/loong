use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use loongclaw_app as mvp;
use loongclaw_spec::CliResult;
use time::OffsetDateTime;
use time::format_description::FormatItem;
use time::macros::format_description;

const BACKUP_TIMESTAMP_FORMAT: &[FormatItem<'static>] =
    format_description!("[year][month][day]-[hour][minute][second]");

#[derive(Debug, Clone)]
pub(crate) struct OnboardCommandOptions {
    pub output: Option<String>,
    pub force: bool,
    pub non_interactive: bool,
    pub accept_risk: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key: Option<String>,
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

#[derive(Debug, Clone)]
struct OnboardImportDiscovery {
    report: mvp::migration::DiscoveryReport,
    summary: mvp::migration::DiscoveryPlanSummary,
    recommendation: Option<mvp::migration::PrimarySourceRecommendation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OnboardImportMode {
    Skip,
    RecommendedSingleSource { source_id: String },
    SelectedSingleSource { source_id: String },
    SafeProfileMerge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OnboardImportStrategy {
    pub mode: OnboardImportMode,
    pub recommended_source_id: Option<String>,
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

    let import_applied = maybe_apply_onboard_import(&options, &output_path)?;
    let mut config = if import_applied {
        load_or_default_onboard_config(&output_path)?
    } else {
        mvp::config::LoongClawConfig::default()
    };

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
        print_step_header(3, total_steps, "credential source");
    }
    let default_api_key_env = provider_default_api_key_env(config.provider.kind).to_owned();
    let selected_api_key = resolve_api_key_selection(&options, default_api_key_env)?;
    config.provider.api_key = normalize_provider_api_key_source(&selected_api_key);
    config.provider.api_key_env = None;

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
                "onboard preflight failed: provider credentials missing. set {} in env or pass --api-key with a populated value",
                provider_credential_env_hint(&config)
                    .unwrap_or_else(|| "OPENAI_API_KEY".to_owned())
            ));
        }
        if has_failures {
            return Err(
                "onboard preflight failed. rerun with --skip-model-probe if your provider blocks model listing during onboarding"
                    .to_owned(),
            );
        }
    } else if (has_failures || has_warnings)
        && !prompt_confirm("Some checks are not green. Continue anyway?", false)?
    {
        return Err("onboarding cancelled: unresolved preflight warnings".to_owned());
    }

    let path = mvp::config::write(
        options.output.as_deref(),
        &config,
        force_write || import_applied,
    )?;
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
    println!(
        "- credential source: {}",
        describe_provider_credential_source(&config)
    );
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

fn maybe_apply_onboard_import(
    options: &OnboardCommandOptions,
    output_path: &Path,
) -> CliResult<bool> {
    let Some(discovery) = discover_onboard_import_candidates()? else {
        return Ok(false);
    };

    let strategy = if options.non_interactive {
        let strategy = resolve_onboard_import_strategy(&discovery.summary, false)?;
        validate_non_interactive_import_strategy(&strategy, false)?;
        strategy
    } else {
        println!();
        println!("legacy claw migration:");
        println!(
            "{}",
            build_onboard_import_summary(&discovery.summary, discovery.recommendation.as_ref())
        );
        resolve_interactive_onboard_import_strategy(&discovery)?
    };

    let selection = match strategy.mode {
        OnboardImportMode::Skip => return Ok(false),
        OnboardImportMode::RecommendedSingleSource { source_id } => {
            mvp::migration::ImportSelectionMode::RecommendedSingleSource { source_id }
        }
        OnboardImportMode::SelectedSingleSource { source_id } => {
            mvp::migration::ImportSelectionMode::SelectedSingleSource { source_id }
        }
        OnboardImportMode::SafeProfileMerge => {
            let primary_source_id = strategy
                .recommended_source_id
                .clone()
                .ok_or_else(|| "safe profile merge requires a primary source".to_owned())?;
            mvp::migration::ImportSelectionMode::SafeProfileMerge { primary_source_id }
        }
    };

    let result = mvp::migration::apply_import_selection(&mvp::migration::ApplyImportSelection {
        discovery: discovery.report,
        output_path: output_path.to_path_buf(),
        mode: selection,
        apply_external_skills_plan: false,
        external_skills_input_path: None,
    })?;

    println!("imported legacy claw profile");
    println!("- primary source: {}", result.selected_primary_source_id);
    println!("- backup: {}", result.backup_path.display());
    println!("- manifest: {}", result.manifest_path.display());
    if result.merged_source_ids.len() > 1 {
        println!("- merged sources: {}", result.merged_source_ids.join(", "));
    }
    Ok(true)
}

fn discover_onboard_import_candidates() -> CliResult<Option<OnboardImportDiscovery>> {
    let mut seen = std::collections::BTreeSet::new();
    let mut sources = Vec::new();
    for root in onboard_search_roots() {
        if !root.exists() {
            continue;
        }
        let report = mvp::migration::discover_import_sources(
            &root,
            mvp::migration::DiscoveryOptions::default(),
        )?;
        for source in report.sources {
            let canonical = source
                .path
                .canonicalize()
                .unwrap_or_else(|_| source.path.clone())
                .display()
                .to_string();
            if seen.insert(canonical) {
                sources.push(source);
            }
        }
    }

    if sources.is_empty() {
        return Ok(None);
    }

    sources.sort_by(|left, right| {
        right
            .confidence_score
            .cmp(&left.confidence_score)
            .then_with(|| left.path.cmp(&right.path))
    });
    let report = mvp::migration::DiscoveryReport { sources };
    let summary = mvp::migration::plan_import_sources(&report)?;
    let recommendation = mvp::migration::recommend_primary_source(&summary).ok();
    Ok(Some(OnboardImportDiscovery {
        report,
        summary,
        recommendation,
    }))
}

fn onboard_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let push_root =
        |roots: &mut Vec<PathBuf>, seen: &mut std::collections::BTreeSet<String>, path: PathBuf| {
            let canonical = path
                .canonicalize()
                .unwrap_or_else(|_| path.clone())
                .display()
                .to_string();
            if seen.insert(canonical) {
                roots.push(path);
            }
        };

    if let Ok(cwd) = std::env::current_dir() {
        push_root(&mut roots, &mut seen, cwd.clone());
        if let Some(parent) = cwd.parent() {
            push_root(&mut roots, &mut seen, parent.to_path_buf());
        }
    }
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        push_root(&mut roots, &mut seen, home.clone());
        push_root(&mut roots, &mut seen, home.join(".config"));
    }
    if let Some(config_parent) = mvp::config::default_loongclaw_home()
        .parent()
        .map(Path::to_path_buf)
    {
        push_root(&mut roots, &mut seen, config_parent);
    }

    roots
}

fn resolve_interactive_onboard_import_strategy(
    discovery: &OnboardImportDiscovery,
) -> CliResult<OnboardImportStrategy> {
    let default_choice = if discovery.summary.plans.is_empty() {
        "s"
    } else {
        "r"
    };
    println!("Import options:");
    if let Some(recommendation) = discovery.recommendation.as_ref() {
        println!(
            "  [r] Recommended source ({} -> {} @ {})",
            recommendation.source_id,
            recommendation.source.as_id(),
            recommendation.input_path.display()
        );
    }
    for plan in &discovery.summary.plans {
        println!(
            "  [{}] Import only {} @ {}",
            plan.source_id,
            plan.source.as_id(),
            plan.input_path.display()
        );
    }
    if discovery.summary.plans.len() > 1 {
        println!("  [m] Safe profile merge");
    }
    println!("  [s] Skip import");

    loop {
        let choice = prompt_with_default("Import strategy", default_choice)?;
        let trimmed = choice.trim().to_ascii_lowercase();
        match trimmed.as_str() {
            "r" => return resolve_onboard_import_strategy(&discovery.summary, false),
            "s" | "skip" => {
                return Ok(OnboardImportStrategy {
                    mode: OnboardImportMode::Skip,
                    recommended_source_id: discovery
                        .recommendation
                        .as_ref()
                        .map(|recommendation| recommendation.source_id.clone()),
                });
            }
            "m" | "merge" if discovery.summary.plans.len() > 1 => {
                return resolve_onboard_import_strategy(&discovery.summary, true);
            }
            other
                if discovery
                    .summary
                    .plans
                    .iter()
                    .any(|plan| plan.source_id == other) =>
            {
                return Ok(OnboardImportStrategy {
                    mode: OnboardImportMode::SelectedSingleSource {
                        source_id: other.to_owned(),
                    },
                    recommended_source_id: discovery
                        .recommendation
                        .as_ref()
                        .map(|recommendation| recommendation.source_id.clone()),
                });
            }
            _ => {
                println!("Invalid import strategy: {trimmed}");
            }
        }
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

fn resolve_api_key_selection(
    options: &OnboardCommandOptions,
    default_api_key_env: String,
) -> CliResult<String> {
    if options.non_interactive {
        return Ok(options
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(default_api_key_env.as_str())
            .to_owned());
    }
    let initial = options
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default_api_key_env.as_str());
    let value = prompt_with_default("API key or env var", initial)?;
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

    let credential_source = provider_credential_source(config);
    let has_credentials = credential_source.is_available();
    checks.push(OnboardCheck {
        name: "provider credentials",
        level: if has_credentials {
            OnboardCheckLevel::Pass
        } else {
            OnboardCheckLevel::Warn
        },
        detail: credential_source.detail(),
    });

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

#[derive(Debug, Clone)]
enum ProviderCredentialSource {
    InlineLiteral,
    ApiKeyEnvRef(String),
    LegacyApiKeyEnv(String),
    DefaultApiKeyEnv(String),
    Missing,
}

impl ProviderCredentialSource {
    fn is_available(&self) -> bool {
        match self {
            Self::InlineLiteral => true,
            Self::ApiKeyEnvRef(key) | Self::LegacyApiKeyEnv(key) | Self::DefaultApiKeyEnv(key) => {
                env::var(key)
                    .ok()
                    .map(|value| !value.trim().is_empty())
                    .unwrap_or(false)
            }
            Self::Missing => false,
        }
    }

    fn detail(&self) -> String {
        match self {
            Self::InlineLiteral => "provider.api_key literal is configured".to_owned(),
            Self::ApiKeyEnvRef(key) => {
                if self.is_available() {
                    format!("env {key} is available")
                } else {
                    format!("env {key} is not set")
                }
            }
            Self::LegacyApiKeyEnv(key) => {
                if self.is_available() {
                    format!("legacy env {key} is available")
                } else {
                    format!("legacy env {key} is not set")
                }
            }
            Self::DefaultApiKeyEnv(key) => {
                if self.is_available() {
                    format!("default env {key} is available")
                } else {
                    format!("default env {key} is not set")
                }
            }
            Self::Missing => "provider.api_key is empty".to_owned(),
        }
    }

    fn summary(&self) -> String {
        match self {
            Self::InlineLiteral => "direct provider.api_key".to_owned(),
            Self::ApiKeyEnvRef(key) => format!("env {key}"),
            Self::LegacyApiKeyEnv(key) => format!("legacy env {key}"),
            Self::DefaultApiKeyEnv(key) => format!("default env {key}"),
            Self::Missing => "not configured".to_owned(),
        }
    }

    fn env_hint(&self) -> Option<String> {
        match self {
            Self::ApiKeyEnvRef(key) | Self::LegacyApiKeyEnv(key) | Self::DefaultApiKeyEnv(key) => {
                Some(key.clone())
            }
            Self::InlineLiteral | Self::Missing => None,
        }
    }
}

fn provider_credential_source(config: &mvp::config::LoongClawConfig) -> ProviderCredentialSource {
    if let Some(raw) = config
        .provider
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(env_name) = parse_explicit_api_key_env_name(raw) {
            return ProviderCredentialSource::ApiKeyEnvRef(env_name);
        }
        return ProviderCredentialSource::InlineLiteral;
    }
    if let Some(key) = config
        .provider
        .api_key_env
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return ProviderCredentialSource::LegacyApiKeyEnv(key.to_owned());
    }
    if let Some(default_key) = config.provider.kind.default_api_key_env() {
        return ProviderCredentialSource::DefaultApiKeyEnv(default_key.to_owned());
    }
    ProviderCredentialSource::Missing
}

fn provider_credential_env_hint(config: &mvp::config::LoongClawConfig) -> Option<String> {
    provider_credential_source(config).env_hint()
}

fn describe_provider_credential_source(config: &mvp::config::LoongClawConfig) -> String {
    provider_credential_source(config).summary()
}

fn normalize_provider_api_key_source(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(env_name) = normalize_onboard_api_key_env_name(trimmed) {
        return Some(format!("${{{env_name}}}"));
    }
    Some(trimmed.to_owned())
}

fn normalize_onboard_api_key_env_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(env_name) = parse_explicit_api_key_env_name(trimmed) {
        return Some(env_name);
    }
    if looks_like_bare_onboard_env_name(trimmed) {
        return Some(trimmed.to_owned());
    }
    None
}

fn parse_explicit_api_key_env_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(env_name) = parse_dollar_env_name(trimmed) {
        return Some(env_name.to_owned());
    }
    if trimmed.len() >= 4 && trimmed[..4].eq_ignore_ascii_case("env:") {
        let env_name = trimmed[4..].trim();
        if looks_like_env_name(env_name) {
            return Some(env_name.to_owned());
        }
    }
    if let Some(env_name) = trimmed
        .strip_prefix('%')
        .and_then(|rest| rest.strip_suffix('%'))
        .map(str::trim)
        .filter(|value| looks_like_env_name(value))
    {
        return Some(env_name.to_owned());
    }
    None
}

fn looks_like_bare_onboard_env_name(raw: &str) -> bool {
    let mut chars = raw.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_uppercase() || first == '_') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
}

fn parse_dollar_env_name(raw: &str) -> Option<&str> {
    let stripped = raw.strip_prefix('$')?.trim();
    if stripped.is_empty() {
        return None;
    }
    let candidate = stripped
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
        .map(str::trim)
        .unwrap_or(stripped);
    looks_like_env_name(candidate).then_some(candidate)
}

fn looks_like_env_name(raw: &str) -> bool {
    let mut chars = raw.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphanumeric() || first == '_') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.')
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

pub(crate) fn resolve_onboard_import_strategy(
    summary: &mvp::migration::DiscoveryPlanSummary,
    prefer_safe_profile_merge: bool,
) -> CliResult<OnboardImportStrategy> {
    let recommended_source_id = match summary.plans.len() {
        0 => None,
        1 => summary.plans.first().map(|plan| plan.source_id.clone()),
        _ => Some(mvp::migration::recommend_primary_source(summary)?.source_id),
    };

    let mode = match (summary.plans.len(), prefer_safe_profile_merge) {
        (0, _) => OnboardImportMode::Skip,
        (_, true) if summary.plans.len() > 1 => OnboardImportMode::SafeProfileMerge,
        _ => OnboardImportMode::RecommendedSingleSource {
            source_id: recommended_source_id
                .clone()
                .ok_or_else(|| "missing recommended import source".to_owned())?,
        },
    };

    Ok(OnboardImportStrategy {
        mode,
        recommended_source_id,
    })
}

pub(crate) fn build_onboard_import_summary(
    summary: &mvp::migration::DiscoveryPlanSummary,
    recommendation: Option<&mvp::migration::PrimarySourceRecommendation>,
) -> String {
    if summary.plans.is_empty() {
        return "No legacy claw import sources detected.".to_owned();
    }

    let mut lines = Vec::new();
    lines.push(format!(
        "Detected {} legacy claw source(s).",
        summary.plans.len()
    ));

    for plan in &summary.plans {
        let prompt_state = if plan.prompt_addendum_present {
            "prompt overlay"
        } else {
            "no prompt overlay"
        };
        let profile_state = if plan.profile_note_present {
            "profile overlay"
        } else {
            "no profile overlay"
        };
        let warning_state = if plan.warning_count == 0 {
            "no warnings".to_owned()
        } else {
            format!("{} warning(s)", plan.warning_count)
        };
        lines.push(format!(
            "- {}: {} @ {}, score {}, {}, {}, {}",
            plan.source_id,
            plan.source.as_id(),
            plan.input_path.display(),
            plan.confidence_score,
            prompt_state,
            profile_state,
            warning_state
        ));
    }

    if let Some(recommendation) = recommendation {
        lines.push(format!(
            "Recommended import source: {} ({} @ {})",
            recommendation.source_id,
            recommendation.source.as_id(),
            recommendation.input_path.display()
        ));
    }

    if summary.plans.len() > 1 {
        lines.push(
            "Secondary option: safe profile merge keeps LoongClaw native prompts and merges only profile-lane traits."
                .to_owned(),
        );
    }

    lines.join("\n")
}

pub(crate) fn validate_non_interactive_import_strategy(
    strategy: &OnboardImportStrategy,
    allow_multi_source_merge: bool,
) -> CliResult<()> {
    if matches!(strategy.mode, OnboardImportMode::SafeProfileMerge) && !allow_multi_source_merge {
        return Err(
            "non-interactive onboarding blocks multi-source merge without explicit opt-in"
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
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    resolve_backup_path_at(original, now)
}

fn resolve_backup_path_at(original: &Path, timestamp: OffsetDateTime) -> CliResult<PathBuf> {
    let parent = original.parent().unwrap_or(Path::new("."));
    let file_stem = original
        .file_stem()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "config".to_owned());

    let formatted_timestamp = format_backup_timestamp_at(timestamp)?;
    Ok(parent.join(format!("{}.toml.bak-{}", file_stem, formatted_timestamp)))
}

fn format_backup_timestamp_at(timestamp: OffsetDateTime) -> CliResult<String> {
    timestamp
        .format(BACKUP_TIMESTAMP_FORMAT)
        .map_err(|error| format!("format backup timestamp failed: {error}"))
}

fn load_or_default_onboard_config(path: &Path) -> CliResult<mvp::config::LoongClawConfig> {
    if !path.exists() {
        return Ok(mvp::config::LoongClawConfig::default());
    }
    let path_string = path.display().to_string();
    let (_, config) = mvp::config::load(Some(&path_string))?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_backup_timestamp_at_matches_existing_filename_shape() {
        let timestamp = time::macros::datetime!(2026-03-14 01:23:45 +08:00);

        let formatted = match format_backup_timestamp_at(timestamp) {
            Ok(value) => value,
            Err(error) => panic!("formatting should succeed: {error}"),
        };

        assert_eq!(formatted, "20260314-012345");
    }

    #[test]
    fn resolve_backup_path_at_uses_formatted_timestamp() {
        let original = Path::new("/tmp/loongclaw.toml");
        let timestamp = time::macros::datetime!(2026-03-14 01:23:45 +08:00);

        let path = match resolve_backup_path_at(original, timestamp) {
            Ok(value) => value,
            Err(error) => panic!("backup path should resolve: {error}"),
        };

        assert_eq!(
            path,
            PathBuf::from("/tmp/loongclaw.toml.bak-20260314-012345")
        );
    }
}
