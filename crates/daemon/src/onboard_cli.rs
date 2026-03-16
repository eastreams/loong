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
const ONBOARD_CLEAR_INPUT_TOKEN: &str = ":clear";

#[derive(Debug, Clone)]
pub struct OnboardCommandOptions {
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

pub trait OnboardUi {
    fn print_line(&mut self, line: &str) -> CliResult<()>;
    fn prompt_with_default(&mut self, label: &str, default: &str) -> CliResult<String>;
    fn prompt_required(&mut self, label: &str) -> CliResult<String>;
    fn prompt_confirm(&mut self, message: &str, default: bool) -> CliResult<bool>;
}

#[derive(Debug, Clone)]
pub struct OnboardRuntimeContext {
    render_width: usize,
    workspace_root: Option<PathBuf>,
    codex_config_paths: Vec<PathBuf>,
}

impl OnboardRuntimeContext {
    fn capture() -> Self {
        Self {
            render_width: detect_render_width(),
            workspace_root: env::current_dir().ok(),
            codex_config_paths: default_codex_config_paths(),
        }
    }

    pub fn new_for_tests(
        render_width: usize,
        workspace_root: Option<PathBuf>,
        codex_config_paths: impl IntoIterator<Item = PathBuf>,
    ) -> Self {
        Self {
            render_width,
            workspace_root,
            codex_config_paths: codex_config_paths.into_iter().collect(),
        }
    }
}

#[derive(Debug, Default)]
struct StdioOnboardUi;

impl OnboardUi for StdioOnboardUi {
    fn print_line(&mut self, line: &str) -> CliResult<()> {
        println!("{line}");
        Ok(())
    }

    fn prompt_with_default(&mut self, label: &str, default: &str) -> CliResult<String> {
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

    fn prompt_required(&mut self, label: &str) -> CliResult<String> {
        print!("{label}: ");
        io::stdout()
            .flush()
            .map_err(|error| format!("flush stdout failed: {error}"))?;
        let mut line = String::new();
        io::stdin()
            .read_line(&mut line)
            .map_err(|error| format!("read stdin failed: {error}"))?;
        Ok(line.trim().to_owned())
    }

    fn prompt_confirm(&mut self, message: &str, default: bool) -> CliResult<bool> {
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
}

fn print_lines(ui: &mut impl OnboardUi, lines: impl IntoIterator<Item = String>) -> CliResult<()> {
    for line in lines {
        ui.print_line(&line)?;
    }
    Ok(())
}

fn print_message(ui: &mut impl OnboardUi, line: impl Into<String>) -> CliResult<()> {
    ui.print_line(&line.into())
}

fn is_explicit_onboard_clear_input(raw: &str) -> bool {
    raw.trim().eq_ignore_ascii_case(ONBOARD_CLEAR_INPUT_TOKEN)
}

fn prompt_optional(
    ui: &mut impl OnboardUi,
    label: &str,
    current: Option<&str>,
) -> CliResult<Option<String>> {
    let value = ui.prompt_required(label)?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(current
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned));
    }
    if trimmed == "-" {
        return Ok(None);
    }
    Ok(Some(trimmed.to_owned()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardCheckLevel {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnboardNonInteractiveWarningPolicy {
    #[default]
    Block,
    AcceptedBySkipModelProbe,
    AcceptedByExplicitModel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct OnboardCheckCounts {
    pass: usize,
    warn: usize,
    fail: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardCheck {
    pub name: &'static str,
    pub level: OnboardCheckLevel,
    pub detail: String,
    pub non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportSurfaceLevel {
    Ready,
    Review,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSurface {
    pub name: &'static str,
    pub domain: crate::migration::SetupDomainKind,
    pub level: ImportSurfaceLevel,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct ImportCandidate {
    pub source_kind: crate::migration::ImportSourceKind,
    pub source: String,
    pub config: mvp::config::LoongClawConfig,
    pub surfaces: Vec<ImportSurface>,
    pub domains: Vec<crate::migration::DomainPreview>,
    pub channel_candidates: Vec<crate::migration::ChannelCandidate>,
    pub workspace_guidance: Vec<crate::migration::WorkspaceGuidanceCandidate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardEntryChoice {
    ContinueCurrentSetup,
    ImportDetectedSetup,
    StartFresh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardEntryOption {
    pub choice: OnboardEntryChoice,
    pub label: &'static str,
    pub detail: String,
    pub recommended: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnboardHeaderStyle {
    Brand,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuidedPromptPath {
    NativePromptPack,
    InlineOverride,
}

impl GuidedPromptPath {
    const fn total_steps(self) -> usize {
        match self {
            GuidedPromptPath::NativePromptPack => 7,
            GuidedPromptPath::InlineOverride => 6,
        }
    }

    const fn index(self, step: GuidedOnboardStep) -> usize {
        match (self, step) {
            (_, GuidedOnboardStep::Provider) => 1,
            (_, GuidedOnboardStep::Model) => 2,
            (_, GuidedOnboardStep::CredentialEnv) => 3,
            (GuidedPromptPath::NativePromptPack, GuidedOnboardStep::Personality) => 4,
            (GuidedPromptPath::NativePromptPack, GuidedOnboardStep::PromptCustomization) => 5,
            (GuidedPromptPath::NativePromptPack, GuidedOnboardStep::MemoryProfile) => 6,
            (GuidedPromptPath::NativePromptPack, GuidedOnboardStep::Review) => 7,
            (GuidedPromptPath::InlineOverride, GuidedOnboardStep::PromptCustomization) => 4,
            (GuidedPromptPath::InlineOverride, GuidedOnboardStep::MemoryProfile) => 5,
            (GuidedPromptPath::InlineOverride, GuidedOnboardStep::Review) => 6,
            (GuidedPromptPath::InlineOverride, GuidedOnboardStep::Personality) => 4,
        }
    }

    const fn label(self, step: GuidedOnboardStep) -> &'static str {
        match step {
            GuidedOnboardStep::Provider => "provider",
            GuidedOnboardStep::Model => "model",
            GuidedOnboardStep::CredentialEnv => "credential source",
            GuidedOnboardStep::Personality => "personality",
            GuidedOnboardStep::PromptCustomization => match self {
                GuidedPromptPath::NativePromptPack => "prompt addendum",
                GuidedPromptPath::InlineOverride => "system prompt",
            },
            GuidedOnboardStep::MemoryProfile => "memory profile",
            GuidedOnboardStep::Review => "review",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuidedOnboardStep {
    Provider,
    Model,
    CredentialEnv,
    Personality,
    PromptCustomization,
    MemoryProfile,
    Review,
}

impl GuidedOnboardStep {
    fn progress_line(self, path: GuidedPromptPath) -> String {
        format!(
            "step {} of {} · {}",
            path.index(self),
            path.total_steps(),
            path.label(self)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReviewFlowStyle {
    Guided(GuidedPromptPath),
    QuickCurrentSetup,
    QuickDetectedSetup,
}

impl ReviewFlowStyle {
    const fn review_kind(self) -> crate::onboard_presentation::ReviewFlowKind {
        match self {
            ReviewFlowStyle::Guided(_) => crate::onboard_presentation::ReviewFlowKind::Guided,
            ReviewFlowStyle::QuickCurrentSetup => {
                crate::onboard_presentation::ReviewFlowKind::QuickCurrentSetup
            }
            ReviewFlowStyle::QuickDetectedSetup => {
                crate::onboard_presentation::ReviewFlowKind::QuickDetectedSetup
            }
        }
    }

    fn progress_line(self) -> String {
        match self {
            ReviewFlowStyle::Guided(prompt_path) => {
                GuidedOnboardStep::Review.progress_line(prompt_path)
            }
            ReviewFlowStyle::QuickCurrentSetup | ReviewFlowStyle::QuickDetectedSetup => {
                crate::onboard_presentation::review_flow_copy(self.review_kind())
                    .progress_line
                    .to_owned()
            }
        }
    }

    const fn header_subtitle(self) -> &'static str {
        crate::onboard_presentation::review_flow_copy(self.review_kind()).header_subtitle
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SystemPromptSelection {
    KeepCurrent,
    RestoreBuiltIn,
    Set(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OnboardScreenOption {
    key: String,
    label: String,
    detail_lines: Vec<String>,
    recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StartingPointFitHint {
    key: &'static str,
    detail: String,
    domain: Option<crate::migration::SetupDomainKind>,
}

#[derive(Debug, Clone)]
struct StartingConfigSelection {
    config: mvp::config::LoongClawConfig,
    import_source: Option<String>,
    provider_selection: crate::migration::ProviderSelectionPlan,
    entry_choice: OnboardEntryChoice,
    current_setup_state: crate::migration::CurrentSetupState,
    review_candidate: Option<ImportCandidate>,
}

#[derive(Debug, Clone)]
struct ConfigWritePlan {
    force: bool,
    backup_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct OnboardWriteRecovery {
    output_preexisted: bool,
    backup_path: Option<PathBuf>,
    keep_backup_on_success: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnboardShortcutKind {
    CurrentSetup,
    DetectedSetup,
}

impl OnboardShortcutKind {
    const fn presentation_kind(self) -> crate::onboard_presentation::ShortcutKind {
        match self {
            OnboardShortcutKind::CurrentSetup => {
                crate::onboard_presentation::ShortcutKind::CurrentSetup
            }
            OnboardShortcutKind::DetectedSetup => {
                crate::onboard_presentation::ShortcutKind::DetectedSetup
            }
        }
    }

    const fn review_flow_style(self) -> ReviewFlowStyle {
        match self {
            OnboardShortcutKind::CurrentSetup => ReviewFlowStyle::QuickCurrentSetup,
            OnboardShortcutKind::DetectedSetup => ReviewFlowStyle::QuickDetectedSetup,
        }
    }

    const fn subtitle(self) -> &'static str {
        crate::onboard_presentation::shortcut_copy(self.presentation_kind()).subtitle
    }

    const fn title(self) -> &'static str {
        crate::onboard_presentation::shortcut_copy(self.presentation_kind()).title
    }

    const fn summary_line(self) -> &'static str {
        crate::onboard_presentation::shortcut_copy(self.presentation_kind()).summary_line
    }

    const fn primary_label(self) -> &'static str {
        crate::onboard_presentation::shortcut_copy(self.presentation_kind()).primary_label
    }

    const fn default_choice_description(self) -> &'static str {
        crate::onboard_presentation::shortcut_copy(self.presentation_kind())
            .default_choice_description
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnboardShortcutChoice {
    UseShortcut,
    AdjustSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingSuccessSummary {
    pub import_source: Option<String>,
    pub config_path: String,
    pub config_status: Option<String>,
    pub provider: String,
    pub saved_provider_profiles: Vec<String>,
    pub model: String,
    pub transport: String,
    pub credential: Option<OnboardingCredentialSummary>,
    pub prompt_mode: String,
    pub personality: Option<String>,
    pub prompt_addendum: Option<String>,
    pub memory_profile: String,
    pub memory_path: Option<String>,
    pub channels: Vec<String>,
    pub domain_outcomes: Vec<OnboardingDomainOutcome>,
    pub next_actions: Vec<OnboardingAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingCredentialSummary {
    pub label: &'static str,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingDomainOutcome {
    pub kind: crate::migration::SetupDomainKind,
    pub decision: crate::migration::types::PreviewDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingActionKind {
    Ask,
    Chat,
    Channel,
    BrowserPreview,
    Doctor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingAction {
    pub kind: OnboardingActionKind,
    pub label: String,
    pub command: String,
}

pub type ChannelImportReadiness = crate::migration::ChannelImportReadiness;

pub async fn run_onboard_cli(options: OnboardCommandOptions) -> CliResult<()> {
    let context = OnboardRuntimeContext::capture();
    let mut ui = StdioOnboardUi;
    run_onboard_cli_with_ui(options, &mut ui, &context).await
}

pub async fn run_onboard_cli_with_ui(
    options: OnboardCommandOptions,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<()> {
    validate_non_interactive_risk_gate(options.non_interactive, options.accept_risk)?;

    if !options.non_interactive && !options.accept_risk {
        print_lines(
            ui,
            render_onboarding_risk_screen_lines_with_style(context.render_width, true),
        )?;
        if !ui.prompt_confirm(
            crate::onboard_presentation::risk_screen_copy().confirm_prompt,
            false,
        )? {
            return Err("onboarding cancelled: risk acknowledgement declined".to_owned());
        }
    }

    let output_path = options
        .output
        .as_deref()
        .map(mvp::config::expand_path)
        .unwrap_or_else(mvp::config::default_config_path);
    let starting_selection = load_import_starting_config(&output_path, &options, ui, context)?;
    let shortcut_kind = resolve_onboard_shortcut_kind(&options, &starting_selection);
    let mut config = starting_selection.config.clone();
    let skip_detailed_setup = if let Some(shortcut_kind) = shortcut_kind {
        print_lines(
            ui,
            render_onboard_shortcut_screen_lines_with_style(
                shortcut_kind,
                &config,
                starting_selection.import_source.as_deref(),
                context.render_width,
                true,
            ),
        )?;
        matches!(
            prompt_onboard_shortcut_choice(ui)?,
            OnboardShortcutChoice::UseShortcut
        )
    } else {
        false
    };
    let review_flow_style = if skip_detailed_setup {
        shortcut_kind
            .map(OnboardShortcutKind::review_flow_style)
            .unwrap_or(ReviewFlowStyle::Guided(GuidedPromptPath::NativePromptPack))
    } else {
        ReviewFlowStyle::Guided(resolve_guided_prompt_path(&options, &config))
    };

    if !skip_detailed_setup {
        let guided_prompt_path = resolve_guided_prompt_path(&options, &config);
        let selected_provider = resolve_provider_selection(
            &options,
            &config,
            &starting_selection.provider_selection,
            guided_prompt_path,
            ui,
            context,
        )?;
        config.provider = selected_provider;

        let selected_model =
            resolve_model_selection(&options, &config, guided_prompt_path, ui, context)?;
        config.provider.model = selected_model;

        let default_api_key_env = preferred_api_key_env_default(&config);
        let selected_api_key_env = resolve_api_key_env_selection(
            &options,
            &config,
            default_api_key_env,
            guided_prompt_path,
            ui,
            context,
        )?;
        apply_selected_api_key_env(&mut config.provider, selected_api_key_env);

        match guided_prompt_path {
            GuidedPromptPath::NativePromptPack => {
                let personality = resolve_personality_selection(&options, &config, ui, context)?;
                config.cli.prompt_pack_id = Some(mvp::prompt::DEFAULT_PROMPT_PACK_ID.to_owned());
                config.cli.personality = Some(personality);
                config.cli.system_prompt_addendum =
                    resolve_prompt_addendum_selection(&options, &config, ui, context)?;
                config.cli.refresh_native_system_prompt();
            }
            GuidedPromptPath::InlineOverride => {
                let system_prompt_selection =
                    resolve_system_prompt_selection(&options, &config, ui, context)?;
                match system_prompt_selection {
                    SystemPromptSelection::KeepCurrent => {}
                    SystemPromptSelection::RestoreBuiltIn => {
                        config.cli.prompt_pack_id =
                            Some(mvp::prompt::DEFAULT_PROMPT_PACK_ID.to_owned());
                        config.cli.personality = Some(mvp::prompt::PromptPersonality::default());
                        config.cli.system_prompt_addendum = None;
                        config.cli.refresh_native_system_prompt();
                    }
                    SystemPromptSelection::Set(system_prompt) => {
                        config.cli.prompt_pack_id = Some(String::new());
                        config.cli.personality = None;
                        config.cli.system_prompt_addendum = None;
                        config.cli.system_prompt = system_prompt;
                    }
                }
            }
        }

        config.memory.profile =
            resolve_memory_profile_selection(&options, &config, guided_prompt_path, ui, context)?;
    }

    let workspace_guidance = context
        .workspace_root
        .as_deref()
        .map(crate::migration::detect_workspace_guidance)
        .unwrap_or_default();
    let review_candidate = build_onboard_review_candidate_with_selected_context(
        &config,
        &workspace_guidance,
        starting_selection.review_candidate.as_ref(),
    );
    if !options.non_interactive {
        print_lines(
            ui,
            render_onboard_review_lines_with_guidance_and_style(
                &config,
                starting_selection.import_source.as_deref(),
                &workspace_guidance,
                starting_selection.review_candidate.as_ref(),
                context.render_width,
                review_flow_style,
                true,
            ),
        )?;
    }

    let checks = run_preflight_checks(&config, options.skip_model_probe).await;

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
    let existing_output_config = load_existing_output_config(&output_path);
    let skip_config_write = should_skip_config_write(existing_output_config.as_ref(), &config);
    let has_blocking_non_interactive_warnings = !skip_config_write
        && checks.iter().any(|check| {
            check.level == OnboardCheckLevel::Warn
                && !is_explicitly_accepted_non_interactive_warning(check, &options)
        });

    if options.non_interactive {
        if !credential_ok {
            let credential_hint = provider_credential_env_hint(&config.provider)
                .unwrap_or_else(|| "PROVIDER_API_KEY".to_owned());
            return Err(format!(
                "onboard preflight failed: provider credentials missing. configure inline credentials or set {} in env",
                credential_hint
            ));
        }
        if has_failures {
            return Err(
                "onboard preflight failed. rerun with --skip-model-probe if your provider blocks model listing during setup"
                    .to_owned(),
            );
        }
        if has_blocking_non_interactive_warnings {
            return Err(
                "onboard preflight failed: unresolved warnings require interactive review; rerun without --non-interactive to inspect and confirm them"
                    .to_owned(),
            );
        }
    } else {
        print_lines(
            ui,
            render_preflight_summary_screen_lines_with_style(
                &checks,
                context.render_width,
                review_flow_style,
                true,
            ),
        )?;
        if (has_failures || has_warnings)
            && !ui.prompt_confirm(
                crate::onboard_presentation::preflight_confirm_prompt(),
                false,
            )?
        {
            return Err("onboarding cancelled: unresolved preflight warnings".to_owned());
        }
    }
    if !options.non_interactive && !skip_config_write {
        print_lines(
            ui,
            render_write_confirmation_screen_lines_with_style(
                &output_path.display().to_string(),
                has_failures || has_warnings,
                context.render_width,
                review_flow_style,
                true,
            ),
        )?;
        if !ui.prompt_confirm(
            crate::onboard_presentation::write_confirmation_prompt(),
            true,
        )? {
            return Err("onboarding cancelled: review declined before write".to_owned());
        }
    }

    let (path, config_status, write_recovery) = if skip_config_write {
        (
            output_path.clone(),
            Some("existing config kept; no changes were needed".to_owned()),
            None,
        )
    } else {
        let write_plan = resolve_write_plan(&output_path, &options, ui, context)?;
        let write_recovery = prepare_output_path_for_write(&output_path, &write_plan, ui)?;
        let path = match mvp::config::write(options.output.as_deref(), &config, write_plan.force) {
            Ok(path) => path,
            Err(error) => {
                return Err(rollback_onboard_write_failure(
                    &output_path,
                    &write_recovery,
                    error,
                ));
            }
        };
        (path, None, Some(write_recovery))
    };
    #[cfg(feature = "memory-sqlite")]
    let memory_path = {
        let mem_config =
            mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(&config.memory);
        match mvp::memory::ensure_memory_db_ready(
            Some(config.memory.resolved_sqlite_path()),
            &mem_config,
        ) {
            Ok(path) => path,
            Err(error) => {
                let failure = format!("failed to bootstrap sqlite memory: {error}");
                if let Some(write_recovery) = write_recovery.as_ref() {
                    return Err(rollback_onboard_write_failure(
                        &output_path,
                        write_recovery,
                        failure,
                    ));
                }
                return Err(failure);
            }
        }
    };

    let memory_path_display = Some(memory_path.display().to_string());
    #[cfg(not(feature = "memory-sqlite"))]
    let memory_path_display: Option<String> = None;

    if let Some(write_recovery) = write_recovery.as_ref() {
        write_recovery.finish_success();
    }

    let success_summary = build_onboarding_success_summary_with_memory(
        &path,
        &config,
        starting_selection.import_source.as_deref(),
        Some(&review_candidate),
        memory_path_display.as_deref(),
        config_status.as_deref(),
    );
    print_lines(ui, render_onboarding_success_summary(&success_summary))?;
    Ok(())
}

fn resolve_guided_prompt_path(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
) -> GuidedPromptPath {
    if options
        .system_prompt
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return GuidedPromptPath::InlineOverride;
    }
    if options
        .personality
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return GuidedPromptPath::NativePromptPack;
    }
    if options.non_interactive {
        if config.cli.uses_native_prompt_pack() {
            return GuidedPromptPath::NativePromptPack;
        }
        if !config.cli.system_prompt.trim().is_empty() {
            return GuidedPromptPath::InlineOverride;
        }
    }
    GuidedPromptPath::NativePromptPack
}

pub fn resolve_guided_prompt_path_label_for_test(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
) -> &'static str {
    match resolve_guided_prompt_path(options, config) {
        GuidedPromptPath::NativePromptPack => "native",
        GuidedPromptPath::InlineOverride => "inline",
    }
}

pub fn build_channel_onboarding_follow_up_lines(
    config: &mvp::config::LoongClawConfig,
) -> Vec<String> {
    let inventory = mvp::channel::channel_inventory(config);
    let mut lines = Vec::with_capacity(inventory.channel_surfaces.len() + 1);
    lines.push("channel next steps:".to_owned());

    for surface in inventory.channel_surfaces {
        let aliases = if surface.catalog.aliases.is_empty() {
            "-".to_owned()
        } else {
            surface.catalog.aliases.join(",")
        };
        let repair_command = surface
            .catalog
            .onboarding
            .repair_command
            .map(|command| format!("\"{command}\""))
            .unwrap_or_else(|| "-".to_owned());
        lines.push(format!(
            "- {} [{}] strategy={} aliases={} status_command=\"{}\" repair_command={} setup_hint=\"{}\"",
            surface.catalog.label,
            surface.catalog.id,
            surface.catalog.onboarding.strategy.as_str(),
            aliases,
            surface.catalog.onboarding.status_command,
            repair_command,
            surface.catalog.onboarding.setup_hint,
        ));
    }

    lines
}

fn resolve_provider_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
    provider_selection: &crate::migration::ProviderSelectionPlan,
    guided_prompt_path: GuidedPromptPath,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<mvp::config::ProviderConfig> {
    if options.non_interactive {
        if let Some(provider_raw) = options.provider.as_deref() {
            return resolve_provider_config_from_selector(
                &config.provider,
                provider_selection,
                provider_raw,
            );
        }
        if provider_selection.requires_explicit_choice {
            let detected = provider_selection
                .imported_choices
                .iter()
                .map(|choice| choice.profile_id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "multiple detected provider choices found ({detected}); rerun with --provider {} to choose the active provider",
                crate::migration::provider_selection::PROVIDER_SELECTOR_PLACEHOLDER,
            ));
        }
        if let Some(default_profile_id) = provider_selection.default_profile_id.as_deref() {
            return resolve_provider_config_from_selector(
                &config.provider,
                provider_selection,
                default_profile_id,
            );
        }
        return Ok(crate::migration::resolve_provider_config_from_selection(
            &config.provider,
            provider_selection,
            provider_selection
                .default_kind
                .unwrap_or(config.provider.kind),
        ));
    }

    print_lines(
        ui,
        render_provider_selection_screen_lines_with_style(
            provider_selection,
            guided_prompt_path,
            context.render_width,
            true,
        ),
    )?;
    let default_provider = options
        .provider
        .clone()
        .or_else(|| provider_selection.default_profile_id.clone())
        .or_else(|| {
            provider_selection
                .default_kind
                .map(|kind| provider_kind_id(kind).to_owned())
        })
        .unwrap_or_else(|| provider_kind_id(config.provider.kind).to_owned());
    loop {
        let input = if provider_selection.requires_explicit_choice {
            ui.prompt_required("Provider")?
        } else {
            ui.prompt_with_default("Provider", &default_provider)?
        };
        match resolve_provider_config_from_selector(&config.provider, provider_selection, &input) {
            Ok(provider) => return Ok(provider),
            Err(error) => print_message(ui, error)?,
        }
    }
}

pub fn resolve_provider_config_from_selector(
    current_provider: &mvp::config::ProviderConfig,
    provider_selection: &crate::migration::ProviderSelectionPlan,
    selector: &str,
) -> CliResult<mvp::config::ProviderConfig> {
    match crate::migration::resolve_choice_by_selector_resolution(provider_selection, selector) {
        crate::migration::ImportedChoiceSelectorResolution::Match(profile_id) => {
            let Some(choice) = provider_selection
                .imported_choices
                .iter()
                .find(|choice| choice.profile_id == profile_id)
            else {
                return Err(format!(
                    "provider selection plan is inconsistent: resolved profile `{profile_id}` is missing"
                ));
            };
            return Ok(choice.config.clone());
        }
        crate::migration::ImportedChoiceSelectorResolution::Ambiguous(profile_ids) => {
            return Err(crate::migration::format_ambiguous_selector_error(
                provider_selection,
                selector,
                &profile_ids,
            ));
        }
        crate::migration::ImportedChoiceSelectorResolution::NoMatch => {}
    }

    let kind = parse_provider_kind(selector).ok_or_else(|| {
        if provider_selection.imported_choices.is_empty() {
            return format!(
                "unsupported provider value \"{selector}\". accepted selectors: {}. {}",
                supported_provider_list(),
                crate::migration::provider_selection::PROVIDER_SELECTOR_NOTE,
            );
        }
        crate::migration::format_unknown_selector_error(
            provider_selection,
            format!("unsupported provider value \"{selector}\"").as_str(),
        )
    })?;
    let matching_choices = provider_selection
        .imported_choices
        .iter()
        .filter(|choice| choice.kind == kind)
        .collect::<Vec<_>>();
    if matching_choices.len() > 1 {
        let profile_ids = matching_choices
            .iter()
            .map(|choice| choice.profile_id.clone())
            .collect::<Vec<_>>();
        return Err(crate::migration::format_ambiguous_selector_error(
            provider_selection,
            selector,
            &profile_ids,
        ));
    }
    if let Some(choice) = matching_choices.first() {
        return Ok(choice.config.clone());
    }
    Ok(crate::migration::resolve_provider_config_from_selection(
        current_provider,
        provider_selection,
        kind,
    ))
}

pub fn build_provider_selection_plan_for_candidate(
    selected_candidate: &ImportCandidate,
    all_candidates: &[ImportCandidate],
) -> crate::migration::ProviderSelectionPlan {
    let migration_selected = migration_candidate_from_onboard(selected_candidate);
    let migration_candidates = all_candidates
        .iter()
        .map(migration_candidate_from_onboard)
        .collect::<Vec<_>>();
    crate::migration::build_provider_selection_plan_for_candidate(
        &migration_selected,
        &migration_candidates,
    )
}

pub fn resolve_provider_config_from_selection(
    current_provider: &mvp::config::ProviderConfig,
    plan: &crate::migration::ProviderSelectionPlan,
    selected_kind: mvp::config::ProviderKind,
) -> mvp::config::ProviderConfig {
    crate::migration::resolve_provider_config_from_selection(current_provider, plan, selected_kind)
}

fn resolve_model_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
    guided_prompt_path: GuidedPromptPath,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
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
    print_lines(
        ui,
        render_model_selection_screen_lines_with_style(
            config,
            default_model,
            guided_prompt_path,
            context.render_width,
            true,
        ),
    )?;
    let value = ui.prompt_with_default("Model", default_model)?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("model cannot be empty".to_owned());
    }
    Ok(trimmed.to_owned())
}

fn resolve_api_key_env_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
    default_api_key_env: String,
    guided_prompt_path: GuidedPromptPath,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<String> {
    if options.non_interactive {
        if let Some(api_key_env) = options.api_key_env.as_deref() {
            if is_explicit_onboard_clear_input(api_key_env) {
                return Ok(String::new());
            }
            let trimmed = api_key_env.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_owned());
            }
        }
        return Ok(default_api_key_env);
    }
    let initial = options
        .api_key_env
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default_api_key_env.as_str());
    print_lines(
        ui,
        render_api_key_env_selection_screen_lines_with_style(
            config,
            default_api_key_env.as_str(),
            initial,
            guided_prompt_path,
            context.render_width,
            true,
        ),
    )?;
    let value = ui.prompt_with_default("API key env var", initial)?;
    if is_explicit_onboard_clear_input(&value) {
        return Ok(String::new());
    }
    Ok(value.trim().to_owned())
}

fn apply_selected_api_key_env(
    provider: &mut mvp::config::ProviderConfig,
    selected_api_key_env: String,
) {
    let selected_api_key_env = selected_api_key_env.trim();
    if selected_api_key_env.is_empty() {
        provider.set_api_key_env(None);
        return;
    }

    provider.api_key = None;
    provider.oauth_access_token = None;
    provider.set_oauth_access_token_env(None);
    provider.set_api_key_env(Some(selected_api_key_env.to_owned()));
}

#[cfg(test)]
fn apply_selected_system_prompt(
    config: &mut mvp::config::LoongClawConfig,
    selection: SystemPromptSelection,
) {
    match selection {
        SystemPromptSelection::KeepCurrent => {}
        SystemPromptSelection::RestoreBuiltIn => {
            config.cli.system_prompt = if config.cli.uses_native_prompt_pack() {
                config.cli.rendered_native_system_prompt()
            } else {
                mvp::config::CliChannelConfig::default().system_prompt
            };
        }
        SystemPromptSelection::Set(system_prompt) => {
            config.cli.system_prompt = system_prompt;
        }
    }
}

fn resolve_personality_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
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
    print_lines(
        ui,
        render_personality_selection_screen_lines_with_style(
            config,
            default_personality,
            context.render_width,
            true,
        ),
    )?;
    loop {
        let input =
            ui.prompt_with_default("Personality", prompt_personality_id(default_personality))?;
        if let Some(personality) = parse_prompt_personality(&input) {
            return Ok(personality);
        }
        print_message(
            ui,
            format!(
                "Unsupported personality: {input}. Use one of: {}",
                supported_personality_list()
            ),
        )?;
    }
}

fn resolve_prompt_addendum_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<Option<String>> {
    if options.non_interactive {
        return Ok(config.cli.system_prompt_addendum.clone());
    }
    print_lines(
        ui,
        render_prompt_addendum_selection_screen_lines_with_style(
            config,
            context.render_width,
            true,
        ),
    )?;
    prompt_optional(
        ui,
        "Prompt addendum (blank keeps current, '-' clears)",
        config.cli.system_prompt_addendum.as_deref(),
    )
}

fn resolve_system_prompt_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<SystemPromptSelection> {
    if options.non_interactive {
        if let Some(system_prompt) = options.system_prompt.as_deref() {
            if is_explicit_onboard_clear_input(system_prompt) {
                return Ok(SystemPromptSelection::RestoreBuiltIn);
            }
            let trimmed = system_prompt.trim();
            if !trimmed.is_empty() {
                return Ok(SystemPromptSelection::Set(trimmed.to_owned()));
            }
        }
        return Ok(SystemPromptSelection::KeepCurrent);
    }
    let initial = options
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(config.cli.system_prompt.as_str());
    print_lines(
        ui,
        render_system_prompt_selection_screen_lines_with_style(
            config,
            initial,
            GuidedPromptPath::InlineOverride,
            context.render_width,
            true,
        ),
    )?;
    let value = ui.prompt_with_default("CLI system prompt", initial)?;
    if is_explicit_onboard_clear_input(&value) {
        return Ok(SystemPromptSelection::RestoreBuiltIn);
    }
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == config.cli.system_prompt.trim() {
        return Ok(SystemPromptSelection::KeepCurrent);
    }
    Ok(SystemPromptSelection::Set(trimmed.to_owned()))
}

fn resolve_memory_profile_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
    guided_prompt_path: GuidedPromptPath,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
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
    print_lines(
        ui,
        render_memory_profile_selection_screen_lines_with_style(
            config,
            default_profile,
            guided_prompt_path,
            context.render_width,
            true,
        ),
    )?;
    loop {
        let input = ui.prompt_with_default("Memory profile", memory_profile_id(default_profile))?;
        if let Some(profile) = parse_memory_profile(&input) {
            return Ok(profile);
        }
        print_message(
            ui,
            format!(
                "Unsupported memory profile: {input}. Use one of: {}",
                supported_memory_profile_list()
            ),
        )?;
    }
}

async fn run_preflight_checks(
    config: &mvp::config::LoongClawConfig,
    skip_model_probe: bool,
) -> Vec<OnboardCheck> {
    let mut checks = Vec::new();
    let credential_check = provider_credential_check(config);
    let has_credentials = credential_check.level == OnboardCheckLevel::Pass;
    checks.push(credential_check);
    checks.push(provider_transport_check(config));

    if skip_model_probe {
        checks.push(OnboardCheck {
            name: "provider model probe",
            level: OnboardCheckLevel::Warn,
            detail: "skipped by --skip-model-probe".to_owned(),
            non_interactive_warning_policy:
                OnboardNonInteractiveWarningPolicy::AcceptedBySkipModelProbe,
        });
    } else if !has_credentials {
        checks.push(OnboardCheck {
            name: "provider model probe",
            level: OnboardCheckLevel::Warn,
            detail: "skipped because credentials are missing".to_owned(),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        });
    } else {
        match mvp::provider::fetch_available_models(config).await {
            Ok(models) => checks.push(OnboardCheck {
                name: "provider model probe",
                level: OnboardCheckLevel::Pass,
                detail: format!("{} model(s) available", models.len()),
                non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
            }),
            Err(error) => checks.push(provider_model_probe_failure_check(config, error)),
        }
    }

    let sqlite_path = config.memory.resolved_sqlite_path();
    let sqlite_parent = sqlite_path.parent().unwrap_or(Path::new("."));
    checks.push(directory_preflight_check("memory path", sqlite_parent));

    let file_root = config.tools.resolved_file_root();
    checks.push(directory_preflight_check("tool file root", &file_root));

    checks.extend(collect_channel_preflight_checks(config));

    checks
}

fn provider_check_detail_prefix(config: &mvp::config::LoongClawConfig) -> String {
    crate::provider_presentation::active_provider_detail_label(config)
}

fn provider_model_probe_failure_check(
    config: &mvp::config::LoongClawConfig,
    error: String,
) -> OnboardCheck {
    if let Some(model) = config.provider.explicit_model() {
        return OnboardCheck {
            name: "provider model probe",
            level: OnboardCheckLevel::Warn,
            detail: format!(
                "{}: model catalog probe failed ({error}); chat may still work because model `{model}` is explicitly configured",
                provider_check_detail_prefix(config)
            ),
            non_interactive_warning_policy:
                OnboardNonInteractiveWarningPolicy::AcceptedByExplicitModel,
        };
    }

    OnboardCheck {
        name: "provider model probe",
        level: OnboardCheckLevel::Fail,
        detail: format!("{}: {error}", provider_check_detail_prefix(config)),
        non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
    }
}

pub fn provider_credential_check(config: &mvp::config::LoongClawConfig) -> OnboardCheck {
    let provider = &config.provider;
    let provider_prefix = provider_check_detail_prefix(config);
    let inline_oauth = provider
        .oauth_access_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if inline_oauth {
        return OnboardCheck {
            name: "provider credentials",
            level: OnboardCheckLevel::Pass,
            detail: format!("{provider_prefix}: inline oauth access token configured"),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        };
    }

    let inline_api_key = provider
        .api_key
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if inline_api_key {
        return OnboardCheck {
            name: "provider credentials",
            level: OnboardCheckLevel::Pass,
            detail: format!("{provider_prefix}: inline api key configured"),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        };
    }

    if provider.authorization_header().is_some() {
        let detail = provider_credential_env_hint(provider)
            .map(|env_name| format!("{env_name} is available"))
            .unwrap_or_else(|| "provider credentials are available".to_owned());
        return OnboardCheck {
            name: "provider credentials",
            level: OnboardCheckLevel::Pass,
            detail: format!("{provider_prefix}: {detail}"),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        };
    }

    let detail = provider_credential_env_hint(provider)
        .map(|env_name| format!("{env_name} is not set"))
        .unwrap_or_else(|| "provider credentials are not configured".to_owned());
    OnboardCheck {
        name: "provider credentials",
        level: OnboardCheckLevel::Warn,
        detail: format!("{provider_prefix}: {detail}"),
        non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
    }
}

fn provider_transport_check(config: &mvp::config::LoongClawConfig) -> OnboardCheck {
    let readiness = config.provider.transport_readiness();
    OnboardCheck {
        name: "provider transport",
        level: match readiness.level {
            mvp::config::ProviderTransportReadinessLevel::Ready => OnboardCheckLevel::Pass,
            mvp::config::ProviderTransportReadinessLevel::Review => OnboardCheckLevel::Warn,
            mvp::config::ProviderTransportReadinessLevel::Unsupported => OnboardCheckLevel::Fail,
        },
        detail: readiness.detail,
        non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
    }
}

fn is_explicitly_accepted_non_interactive_warning(
    check: &OnboardCheck,
    options: &OnboardCommandOptions,
) -> bool {
    (options.skip_model_probe
        && matches!(
            check.non_interactive_warning_policy,
            OnboardNonInteractiveWarningPolicy::AcceptedBySkipModelProbe
        ))
        || matches!(
            check.non_interactive_warning_policy,
            OnboardNonInteractiveWarningPolicy::AcceptedByExplicitModel
        )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderCredentialEnvField {
    ApiKey,
    OAuthAccessToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCredentialEnvBinding {
    pub field: ProviderCredentialEnvField,
    pub env_name: String,
}

pub fn provider_credential_env_hints(provider: &mvp::config::ProviderConfig) -> Vec<String> {
    let mut hints = Vec::new();
    push_provider_credential_env_hint(&mut hints, provider.oauth_access_token_env.as_deref());
    push_provider_credential_env_hint(&mut hints, provider.api_key_env.as_deref());
    push_provider_credential_env_hint(&mut hints, provider.kind.default_oauth_access_token_env());
    push_provider_credential_env_hint(&mut hints, provider.kind.default_api_key_env());
    hints
}

pub fn provider_credential_env_hint(provider: &mvp::config::ProviderConfig) -> Option<String> {
    provider_credential_env_hints(provider).into_iter().next()
}

pub fn preferred_provider_credential_env_binding(
    provider: &mvp::config::ProviderConfig,
) -> Option<ProviderCredentialEnvBinding> {
    provider
        .oauth_access_token_env
        .as_deref()
        .and_then(normalize_provider_credential_env_name)
        .map(|env_name| ProviderCredentialEnvBinding {
            field: ProviderCredentialEnvField::OAuthAccessToken,
            env_name,
        })
        .or_else(|| {
            provider
                .api_key_env
                .as_deref()
                .and_then(normalize_provider_credential_env_name)
                .map(|env_name| ProviderCredentialEnvBinding {
                    field: ProviderCredentialEnvField::ApiKey,
                    env_name,
                })
        })
        .or_else(|| {
            provider
                .kind
                .default_oauth_access_token_env()
                .and_then(normalize_provider_credential_env_name)
                .map(|env_name| ProviderCredentialEnvBinding {
                    field: ProviderCredentialEnvField::OAuthAccessToken,
                    env_name,
                })
        })
        .or_else(|| {
            provider
                .kind
                .default_api_key_env()
                .and_then(normalize_provider_credential_env_name)
                .map(|env_name| ProviderCredentialEnvBinding {
                    field: ProviderCredentialEnvField::ApiKey,
                    env_name,
                })
        })
}

fn push_provider_credential_env_hint(hints: &mut Vec<String>, maybe_env_name: Option<&str>) {
    let Some(env_name) = maybe_env_name.and_then(normalize_provider_credential_env_name) else {
        return;
    };
    if !hints.iter().any(|existing| existing == &env_name) {
        hints.push(env_name);
    }
}

fn normalize_provider_credential_env_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

fn render_provider_credential_source_value(raw: Option<&str>) -> Option<String> {
    normalize_provider_credential_env_name(raw?).map(|env_name| format!("${{{env_name}}}"))
}

pub fn preferred_api_key_env_default(config: &mvp::config::LoongClawConfig) -> String {
    let provider = &config.provider;
    if let Some(api_key_env) = provider
        .api_key_env
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return api_key_env.to_owned();
    }
    let inline_or_oauth_auth = provider
        .api_key
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        || provider
            .oauth_access_token
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        || provider
            .oauth_access_token_env
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
    if inline_or_oauth_auth {
        return String::new();
    }
    if provider.api_key().is_some() {
        return provider_default_api_key_env(provider.kind)
            .unwrap_or_default()
            .to_owned();
    }
    provider_default_api_key_env(provider.kind)
        .unwrap_or_default()
        .to_owned()
}

pub fn directory_preflight_check(name: &'static str, target: &Path) -> OnboardCheck {
    if target.exists() {
        return match fs::metadata(target) {
            Ok(metadata) if metadata.is_dir() => OnboardCheck {
                name,
                level: OnboardCheckLevel::Pass,
                detail: target.display().to_string(),
                non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
            },
            Ok(_) => OnboardCheck {
                name,
                level: OnboardCheckLevel::Fail,
                detail: format!("{} exists but is not a directory", target.display()),
                non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
            },
            Err(error) => OnboardCheck {
                name,
                level: OnboardCheckLevel::Fail,
                detail: format!("failed to inspect {}: {error}", target.display()),
                non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
            },
        };
    }

    let mut ancestor = target;
    while !ancestor.exists() {
        let Some(parent) = ancestor.parent() else {
            return OnboardCheck {
                name,
                level: OnboardCheckLevel::Fail,
                detail: format!("no existing parent found for {}", target.display()),
                non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
            };
        };
        ancestor = parent;
    }

    match fs::metadata(ancestor) {
        Ok(metadata) if metadata.is_dir() => OnboardCheck {
            name,
            level: OnboardCheckLevel::Pass,
            detail: format!("would create under {}", ancestor.display()),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        },
        Ok(_) => OnboardCheck {
            name,
            level: OnboardCheckLevel::Fail,
            detail: format!("{} exists but is not a directory", ancestor.display()),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        },
        Err(error) => OnboardCheck {
            name,
            level: OnboardCheckLevel::Fail,
            detail: format!("failed to inspect {}: {error}", ancestor.display()),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        },
    }
}

pub fn collect_channel_preflight_checks(
    config: &mvp::config::LoongClawConfig,
) -> Vec<OnboardCheck> {
    crate::migration::channels::collect_channel_preflight_checks(config)
        .into_iter()
        .map(|check| OnboardCheck {
            name: check.name,
            level: match check.level {
                crate::migration::channels::ChannelCheckLevel::Pass => OnboardCheckLevel::Pass,
                crate::migration::channels::ChannelCheckLevel::Warn => OnboardCheckLevel::Warn,
                crate::migration::channels::ChannelCheckLevel::Fail => OnboardCheckLevel::Fail,
            },
            detail: check.detail,
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        })
        .collect()
}

pub fn collect_import_surfaces(config: &mvp::config::LoongClawConfig) -> Vec<ImportSurface> {
    crate::migration::collect_import_surfaces(config)
        .into_iter()
        .map(import_surface_from_migration)
        .collect()
}

pub fn collect_import_surfaces_with_channel_readiness(
    config: &mvp::config::LoongClawConfig,
    readiness: ChannelImportReadiness,
) -> Vec<ImportSurface> {
    crate::migration::collect_import_surfaces_with_channel_readiness(
        config,
        &to_migration_readiness(readiness),
    )
    .into_iter()
    .map(import_surface_from_migration)
    .collect()
}

fn summarize_onboard_checks(checks: &[OnboardCheck]) -> OnboardCheckCounts {
    let mut counts = OnboardCheckCounts::default();
    for check in checks {
        match check.level {
            OnboardCheckLevel::Pass => counts.pass += 1,
            OnboardCheckLevel::Warn => counts.warn += 1,
            OnboardCheckLevel::Fail => counts.fail += 1,
        }
    }
    counts
}

fn render_preflight_check_rows(checks: &[OnboardCheck], width: usize) -> Vec<String> {
    let render_stacked_rows = |checks: &[OnboardCheck], width: usize| {
        let mut lines = Vec::new();
        for check in checks {
            lines.push(format!(
                "{} {}",
                check_level_marker(check.level),
                check.name
            ));
            lines.extend(mvp::presentation::render_wrapped_text_line(
                "  ",
                &check.detail,
                width,
            ));
        }
        lines
    };

    if width < 68 {
        return render_stacked_rows(checks, width);
    }

    let name_width = checks
        .iter()
        .map(|check| check.name.len())
        .max()
        .unwrap_or(0);
    let rows = checks
        .iter()
        .map(|check| {
            format!(
                "{} {:width$}  {}",
                check_level_marker(check.level),
                check.name,
                check.detail,
                width = name_width
            )
        })
        .collect::<Vec<_>>();
    if rows.iter().any(|row| row.len() > width) {
        return render_stacked_rows(checks, width);
    }
    rows
}

fn check_level_marker(level: OnboardCheckLevel) -> &'static str {
    match level {
        OnboardCheckLevel::Pass => "[OK]",
        OnboardCheckLevel::Warn => "[WARN]",
        OnboardCheckLevel::Fail => "[FAIL]",
    }
}

fn load_import_starting_config(
    output_path: &Path,
    options: &OnboardCommandOptions,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<StartingConfigSelection> {
    let default_config = mvp::config::LoongClawConfig::default();
    let readiness = resolve_channel_import_readiness(&default_config);
    let current_setup_state = crate::migration::classify_current_setup(output_path);
    let candidates = collect_import_candidates_with_context(output_path, context, readiness)?;
    let all_candidates = candidates.clone();
    let entry_options = build_onboard_entry_options(current_setup_state, &candidates);
    let (current_candidate, import_candidates) = split_onboard_candidates(candidates);

    if current_candidate.is_none() && import_candidates.is_empty() {
        return Ok(default_starting_config_selection());
    }

    if options.non_interactive {
        return Ok(select_non_interactive_starting_config(
            current_setup_state,
            &entry_options,
            current_candidate,
            import_candidates,
            &all_candidates,
        ));
    }

    if entry_options
        .first()
        .is_some_and(|option| option.choice == OnboardEntryChoice::StartFresh)
    {
        return Ok(default_starting_config_selection());
    }

    print_onboard_entry_options(
        ui,
        current_setup_state,
        current_candidate.as_ref(),
        &import_candidates,
        &entry_options,
        context,
    )?;
    match prompt_onboard_entry_choice(ui, &entry_options)? {
        OnboardEntryChoice::ContinueCurrentSetup => Ok(current_candidate
            .map(|candidate| {
                starting_config_selection_from_current_candidate(candidate, current_setup_state)
            })
            .unwrap_or_else(default_starting_config_selection)),
        OnboardEntryChoice::ImportDetectedSetup => select_interactive_import_starting_config(
            ui,
            context,
            current_setup_state,
            import_candidates,
            &all_candidates,
        ),
        OnboardEntryChoice::StartFresh => Ok(default_starting_config_selection()),
    }
}

pub fn build_onboard_entry_options(
    current_setup_state: crate::migration::CurrentSetupState,
    candidates: &[ImportCandidate],
) -> Vec<OnboardEntryOption> {
    let has_current_setup = candidates.iter().any(|candidate| {
        candidate.source_kind == crate::migration::ImportSourceKind::ExistingLoongClawConfig
    });
    let recommended_plan_available = candidates.iter().any(|candidate| {
        candidate.source_kind == crate::migration::ImportSourceKind::RecommendedPlan
    });
    let detected_source_count = detected_reusable_source_count_for_entry(
        candidates.iter().find(|candidate| {
            candidate.source_kind == crate::migration::ImportSourceKind::ExistingLoongClawConfig
        }),
        candidates,
    );
    let mut options = Vec::new();

    if has_current_setup {
        options.push(OnboardEntryOption {
            choice: OnboardEntryChoice::ContinueCurrentSetup,
            label: crate::onboard_presentation::current_setup_option_label(),
            detail: describe_current_setup_option(current_setup_state),
            recommended: matches!(
                current_setup_state,
                crate::migration::CurrentSetupState::Healthy
            ) || matches!(
                current_setup_state,
                crate::migration::CurrentSetupState::Repairable
            ) && detected_source_count == 0,
        });
    }

    if detected_source_count > 0 || recommended_plan_available {
        options.push(OnboardEntryOption {
            choice: OnboardEntryChoice::ImportDetectedSetup,
            label: crate::onboard_presentation::detected_setup_option_label(),
            detail: describe_import_option(
                has_current_setup,
                recommended_plan_available,
                detected_source_count,
            ),
            recommended: matches!(
                current_setup_state,
                crate::migration::CurrentSetupState::Absent
                    | crate::migration::CurrentSetupState::LegacyOrIncomplete
                    | crate::migration::CurrentSetupState::Repairable
            ),
        });
    }

    options.push(OnboardEntryOption {
        choice: OnboardEntryChoice::StartFresh,
        label: crate::onboard_presentation::start_fresh_option_label(),
        detail: crate::onboard_presentation::start_fresh_option_detail().to_owned(),
        recommended: !options.iter().any(|option| option.recommended),
    });

    options
}

fn describe_current_setup_option(
    current_setup_state: crate::migration::CurrentSetupState,
) -> String {
    crate::onboard_presentation::current_setup_option_detail(current_setup_state).to_owned()
}

fn describe_import_option(
    has_current_setup: bool,
    recommended_plan_available: bool,
    detected_source_count: usize,
) -> String {
    crate::onboard_presentation::import_option_detail(
        has_current_setup,
        recommended_plan_available,
        detected_source_count,
    )
}

fn split_onboard_candidates(
    candidates: Vec<ImportCandidate>,
) -> (Option<ImportCandidate>, Vec<ImportCandidate>) {
    let mut current_candidate = None;
    let mut import_candidates = Vec::new();

    for candidate in candidates {
        if candidate.source_kind == crate::migration::ImportSourceKind::ExistingLoongClawConfig
            && current_candidate.is_none()
        {
            current_candidate = Some(candidate);
        } else {
            import_candidates.push(candidate);
        }
    }

    (current_candidate, import_candidates)
}

fn select_non_interactive_starting_config(
    current_setup_state: crate::migration::CurrentSetupState,
    entry_options: &[OnboardEntryOption],
    current_candidate: Option<ImportCandidate>,
    import_candidates: Vec<ImportCandidate>,
    all_candidates: &[ImportCandidate],
) -> StartingConfigSelection {
    match default_onboard_entry_choice(entry_options) {
        OnboardEntryChoice::ContinueCurrentSetup => current_candidate
            .map(|candidate| {
                starting_config_selection_from_current_candidate(candidate, current_setup_state)
            })
            .unwrap_or_else(default_starting_config_selection),
        OnboardEntryChoice::ImportDetectedSetup => {
            sort_starting_point_candidates(import_candidates)
                .into_iter()
                .map(|candidate| {
                    starting_config_selection_from_import_candidate(
                        candidate,
                        all_candidates,
                        current_setup_state,
                    )
                })
                .next()
                .unwrap_or_else(default_starting_config_selection)
        }
        OnboardEntryChoice::StartFresh => default_starting_config_selection(),
    }
}

fn print_onboard_entry_options(
    ui: &mut impl OnboardUi,
    current_setup_state: crate::migration::CurrentSetupState,
    current_candidate: Option<&ImportCandidate>,
    import_candidates: &[ImportCandidate],
    options: &[OnboardEntryOption],
    context: &OnboardRuntimeContext,
) -> CliResult<()> {
    print_lines(
        ui,
        render_onboard_entry_screen_lines_with_style(
            current_setup_state,
            current_candidate,
            import_candidates,
            options,
            context.workspace_root.as_deref(),
            context.render_width,
            true,
        ),
    )
}

pub fn render_onboard_entry_screen_lines(
    current_setup_state: crate::migration::CurrentSetupState,
    current_candidate: Option<&ImportCandidate>,
    import_candidates: &[ImportCandidate],
    options: &[OnboardEntryOption],
    workspace_root: Option<&Path>,
    width: usize,
) -> Vec<String> {
    render_onboard_entry_screen_lines_with_style(
        current_setup_state,
        current_candidate,
        import_candidates,
        options,
        workspace_root,
        width,
        false,
    )
}

fn render_onboard_entry_screen_lines_with_style(
    current_setup_state: crate::migration::CurrentSetupState,
    current_candidate: Option<&ImportCandidate>,
    import_candidates: &[ImportCandidate],
    options: &[OnboardEntryOption],
    workspace_root: Option<&Path>,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let recommended_plan_available = import_candidates.iter().any(|candidate| {
        candidate.source_kind == crate::migration::ImportSourceKind::RecommendedPlan
    });
    let mut lines = mvp::presentation::style_brand_lines(
        &mvp::presentation::render_brand_header(
            width,
            &mvp::presentation::BuildVersionInfo::current(),
            Some("guided setup for provider, channels, and workspace guidance"),
        ),
        color_enabled,
    );
    lines.push(String::new());
    lines.push(crate::onboard_presentation::detected_settings_section_heading().to_owned());
    lines.extend(render_onboard_wrapped_display_lines(
        render_detected_settings_digest_lines(
            current_setup_state,
            current_candidate,
            import_candidates,
            workspace_root,
            recommended_plan_available,
        ),
        width,
    ));
    lines.push(String::new());
    lines.push(crate::onboard_presentation::entry_choice_section_heading().to_owned());
    let screen_options = options
        .iter()
        .enumerate()
        .map(|(index, option)| OnboardScreenOption {
            key: (index + 1).to_string(),
            label: option.label.to_owned(),
            detail_lines: vec![option.detail.clone()],
            recommended: option.recommended,
        })
        .collect::<Vec<_>>();
    lines.extend(render_onboard_option_lines(&screen_options, width));
    if let Some(default_choice_line) = render_onboard_entry_default_choice_footer_line(options) {
        lines.push(String::new());
        lines.extend(render_onboard_wrapped_display_lines(
            [default_choice_line],
            width,
        ));
    }
    lines
}

fn render_onboard_entry_default_choice_footer_line(
    options: &[OnboardEntryOption],
) -> Option<String> {
    let default_choice = default_onboard_entry_choice(options);
    let default_index = options
        .iter()
        .position(|option| option.choice == default_choice)
        .map(|index| index + 1)?;
    let description = crate::onboard_presentation::entry_default_choice_description(
        onboard_entry_choice_kind(default_choice),
    );
    Some(render_default_choice_footer_line(
        &default_index.to_string(),
        description,
    ))
}

const fn onboard_entry_choice_kind(
    choice: OnboardEntryChoice,
) -> crate::onboard_presentation::EntryChoiceKind {
    match choice {
        OnboardEntryChoice::ContinueCurrentSetup => {
            crate::onboard_presentation::EntryChoiceKind::CurrentSetup
        }
        OnboardEntryChoice::ImportDetectedSetup => {
            crate::onboard_presentation::EntryChoiceKind::DetectedSetup
        }
        OnboardEntryChoice::StartFresh => crate::onboard_presentation::EntryChoiceKind::StartFresh,
    }
}

fn collect_detected_workspace_guidance_files<'a>(
    current_candidate: impl Iterator<Item = &'a ImportCandidate>,
    import_candidates: &'a [ImportCandidate],
) -> Vec<String> {
    let mut files = std::collections::BTreeSet::new();
    for candidate in current_candidate.chain(import_candidates.iter()) {
        for guidance in &candidate.workspace_guidance {
            if let Some(name) = Path::new(&guidance.path).file_name() {
                files.insert(name.to_string_lossy().to_string());
            }
        }
    }
    files.into_iter().collect()
}

fn recommended_starting_point_candidate(
    import_candidates: &[ImportCandidate],
) -> Option<&ImportCandidate> {
    import_candidates.iter().find(|candidate| {
        candidate.source_kind == crate::migration::ImportSourceKind::RecommendedPlan
    })
}

fn collect_detected_coverage_kinds(
    candidates: impl IntoIterator<Item = impl std::borrow::Borrow<ImportCandidate>>,
) -> std::collections::BTreeSet<crate::migration::SetupDomainKind> {
    let mut kinds = std::collections::BTreeSet::new();
    for candidate in candidates {
        let candidate = candidate.borrow();
        for domain in &candidate.domains {
            if domain.status != crate::migration::PreviewStatus::Unavailable {
                kinds.insert(domain.kind);
            }
        }
        if candidate
            .channel_candidates
            .iter()
            .any(|channel| channel.status != crate::migration::PreviewStatus::Unavailable)
        {
            kinds.insert(crate::migration::SetupDomainKind::Channels);
        }
        if !candidate.workspace_guidance.is_empty() {
            kinds.insert(crate::migration::SetupDomainKind::WorkspaceGuidance);
        }
    }
    kinds
}

fn collect_detected_channel_labels(import_candidates: &[ImportCandidate]) -> Vec<String> {
    let mut labels = std::collections::BTreeSet::new();
    for candidate in import_candidates {
        for channel in &candidate.channel_candidates {
            if channel.status != crate::migration::PreviewStatus::Unavailable {
                labels.insert(channel.label.to_owned());
            }
        }
    }
    labels.into_iter().collect()
}

fn detected_reusable_source_count_for_entry(
    current_candidate: Option<&ImportCandidate>,
    import_candidates: &[ImportCandidate],
) -> usize {
    if let Some(recommended_candidate) = recommended_starting_point_candidate(import_candidates) {
        let mut labels = crate::migration::render::candidate_source_rollup_labels(
            &migration_candidate_from_onboard(recommended_candidate),
        );
        if let Some(current_candidate) = current_candidate {
            labels.retain(|label| label != &current_candidate.source);
        }
        return labels.len();
    }

    import_candidates
        .iter()
        .filter(|candidate| {
            !matches!(
                candidate.source_kind,
                crate::migration::ImportSourceKind::ExistingLoongClawConfig
                    | crate::migration::ImportSourceKind::RecommendedPlan
            )
        })
        .count()
}

fn render_detected_settings_digest_lines(
    current_setup_state: crate::migration::CurrentSetupState,
    current_candidate: Option<&ImportCandidate>,
    import_candidates: &[ImportCandidate],
    workspace_root: Option<&Path>,
    recommended_plan_available: bool,
) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(workspace_root) = workspace_root {
        lines.push(format!("- workspace: {}", workspace_root.display()));
    }
    lines.push(format!(
        "- current setup: {}",
        crate::onboard_presentation::current_setup_state_label(current_setup_state)
    ));
    if let Some(candidate) = current_candidate {
        lines.push(format!("- current config: {}", candidate.source));
    }

    let coverage_kinds = recommended_starting_point_candidate(import_candidates)
        .map(|candidate| collect_detected_coverage_kinds([candidate]))
        .filter(|kinds| !kinds.is_empty())
        .or_else(|| {
            let kinds = collect_detected_coverage_kinds(import_candidates.iter());
            (!kinds.is_empty()).then_some(kinds)
        });
    if let Some(coverage_kinds) = coverage_kinds {
        let coverage = coverage_kinds
            .into_iter()
            .map(|kind| kind.label())
            .collect::<Vec<_>>()
            .join(", ");
        let prefix =
            crate::onboard_presentation::detected_coverage_prefix(recommended_plan_available);
        lines.push(format!("{prefix}{coverage}"));
    } else if recommended_plan_available {
        lines.push(crate::onboard_presentation::suggested_starting_point_ready_line().to_owned());
    }

    let channel_labels = collect_detected_channel_labels(import_candidates);
    if !channel_labels.is_empty() {
        lines.push(format!(
            "- channels detected: {}",
            channel_labels.join(", ")
        ));
    }

    let guidance_files =
        collect_detected_workspace_guidance_files(current_candidate.into_iter(), import_candidates);
    if !guidance_files.is_empty() {
        lines.push(format!(
            "- workspace guidance: {}",
            guidance_files.join(", ")
        ));
    }

    let reusable_source_count =
        detected_reusable_source_count_for_entry(current_candidate, import_candidates);
    if reusable_source_count > 0 {
        lines.push(format!("- reusable sources: {reusable_source_count}"));
    }

    lines
}
fn prompt_onboard_entry_choice(
    ui: &mut impl OnboardUi,
    options: &[OnboardEntryOption],
) -> CliResult<OnboardEntryChoice> {
    let default_choice = options
        .iter()
        .position(|option| option.recommended)
        .map(|index| (index + 1).to_string())
        .unwrap_or_else(|| "1".to_owned());
    loop {
        let choice = ui.prompt_with_default("Setup path", &default_choice)?;
        let trimmed = choice.trim();
        let Ok(selected) = trimmed.parse::<usize>() else {
            print_message(ui, format!("Invalid choice: {trimmed}"))?;
            continue;
        };
        if let Some(option) = options.get(selected - 1) {
            return Ok(option.choice);
        }
        print_message(ui, format!("Invalid choice: {trimmed}"))?;
    }
}

fn default_onboard_entry_choice(options: &[OnboardEntryOption]) -> OnboardEntryChoice {
    options
        .iter()
        .find(|option| option.recommended)
        .map(|option| option.choice)
        .unwrap_or(OnboardEntryChoice::StartFresh)
}

fn starting_point_candidate_coverage_breadth(candidate: &ImportCandidate) -> usize {
    collect_detected_coverage_kinds([candidate]).len()
}

fn direct_starting_point_source_rank(source_kind: crate::migration::ImportSourceKind) -> usize {
    source_kind.direct_starting_point_rank()
}

fn sort_starting_point_candidates(mut candidates: Vec<ImportCandidate>) -> Vec<ImportCandidate> {
    candidates.sort_by_key(|candidate| {
        (
            usize::from(
                candidate.source_kind != crate::migration::ImportSourceKind::RecommendedPlan,
            ),
            std::cmp::Reverse(starting_point_candidate_coverage_breadth(candidate)),
            direct_starting_point_source_rank(candidate.source_kind),
            candidate.source.to_ascii_lowercase(),
        )
    });
    candidates
}

fn select_interactive_import_starting_config(
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
    current_setup_state: crate::migration::CurrentSetupState,
    import_candidates: Vec<ImportCandidate>,
    all_candidates: &[ImportCandidate],
) -> CliResult<StartingConfigSelection> {
    let import_candidates = sort_starting_point_candidates(import_candidates);
    if import_candidates.is_empty() {
        return Ok(default_starting_config_selection());
    }
    if import_candidates.len() == 1 {
        if let Some(candidate) = import_candidates.first() {
            print_import_candidate_preview(ui, candidate, all_candidates, context)?;
            return Ok(starting_config_selection_from_import_candidate(
                candidate.clone(),
                all_candidates,
                current_setup_state,
            ));
        }
        return Ok(default_starting_config_selection());
    }

    print_import_candidates(ui, &import_candidates, context)?;
    let Some(index) = prompt_import_candidate_choice(ui, import_candidates.len())? else {
        return Ok(default_starting_config_selection());
    };
    if let Some(candidate) = import_candidates.get(index) {
        return Ok(starting_config_selection_from_import_candidate(
            candidate.clone(),
            all_candidates,
            current_setup_state,
        ));
    }
    Ok(default_starting_config_selection())
}

pub fn collect_import_candidates_with_paths(
    output_path: &Path,
    codex_config_path: Option<&Path>,
    readiness: ChannelImportReadiness,
) -> CliResult<Vec<ImportCandidate>> {
    let workspace_root = env::current_dir().ok();
    crate::migration::collect_import_candidates_with_paths_and_readiness(
        output_path,
        codex_config_path,
        workspace_root.as_deref(),
        to_migration_readiness(readiness),
    )
    .map(crate::migration::prepend_recommended_import_candidate)
    .map(|candidates| {
        candidates
            .into_iter()
            .map(import_candidate_from_migration)
            .collect()
    })
}

fn collect_import_candidates_with_context(
    output_path: &Path,
    context: &OnboardRuntimeContext,
    readiness: ChannelImportReadiness,
) -> CliResult<Vec<ImportCandidate>> {
    crate::migration::discovery::collect_import_candidates_with_path_list_and_readiness(
        output_path,
        &context.codex_config_paths,
        context.workspace_root.as_deref(),
        to_migration_readiness(readiness),
    )
    .map(crate::migration::prepend_recommended_import_candidate)
    .map(|candidates| {
        candidates
            .into_iter()
            .map(import_candidate_from_migration)
            .collect()
    })
}

fn default_starting_config_selection() -> StartingConfigSelection {
    StartingConfigSelection {
        config: mvp::config::LoongClawConfig::default(),
        import_source: None,
        provider_selection: crate::migration::ProviderSelectionPlan::default(),
        entry_choice: OnboardEntryChoice::StartFresh,
        current_setup_state: crate::migration::CurrentSetupState::Absent,
        review_candidate: None,
    }
}

fn starting_config_selection_from_current_candidate(
    candidate: ImportCandidate,
    current_setup_state: crate::migration::CurrentSetupState,
) -> StartingConfigSelection {
    StartingConfigSelection {
        config: candidate.config.clone(),
        import_source: Some(onboard_starting_point_label(
            Some(candidate.source_kind),
            &candidate.source,
        )),
        provider_selection: crate::migration::ProviderSelectionPlan::default(),
        entry_choice: OnboardEntryChoice::ContinueCurrentSetup,
        current_setup_state,
        review_candidate: Some(candidate),
    }
}

fn starting_config_selection_from_import_candidate(
    candidate: ImportCandidate,
    all_candidates: &[ImportCandidate],
    current_setup_state: crate::migration::CurrentSetupState,
) -> StartingConfigSelection {
    let provider_selection =
        build_provider_selection_plan_for_candidate(&candidate, all_candidates);
    StartingConfigSelection {
        config: candidate.config.clone(),
        import_source: Some(onboard_starting_point_label(
            Some(candidate.source_kind),
            &candidate.source,
        )),
        provider_selection,
        entry_choice: OnboardEntryChoice::ImportDetectedSetup,
        current_setup_state,
        review_candidate: Some(candidate),
    }
}

fn print_import_candidate_preview(
    ui: &mut impl OnboardUi,
    candidate: &ImportCandidate,
    all_candidates: &[ImportCandidate],
    context: &OnboardRuntimeContext,
) -> CliResult<()> {
    print_lines(
        ui,
        render_single_detected_setup_preview_screen_lines_with_style(
            candidate,
            all_candidates,
            context.render_width,
            true,
        ),
    )
}

pub fn render_single_detected_setup_preview_screen_lines(
    candidate: &ImportCandidate,
    all_candidates: &[ImportCandidate],
    width: usize,
) -> Vec<String> {
    render_single_detected_setup_preview_screen_lines_with_style(
        candidate,
        all_candidates,
        width,
        false,
    )
}

fn render_single_detected_setup_preview_screen_lines_with_style(
    candidate: &ImportCandidate,
    all_candidates: &[ImportCandidate],
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let migration_candidate = migration_candidate_from_onboard(candidate);
    let migration_candidates = all_candidates
        .iter()
        .map(migration_candidate_from_onboard)
        .collect::<Vec<_>>();
    let provider_selection = crate::migration::build_provider_selection_plan_for_candidate(
        &migration_candidate,
        &migration_candidates,
    );
    let mut intro_lines = Vec::new();
    if let Some(reason_line) =
        format_starting_point_reason(&collect_starting_point_fit_hints(candidate))
    {
        intro_lines.push(reason_line);
    }
    intro_lines.extend(crate::migration::render::render_candidate_preview_lines(
        &migration_candidate_for_onboard_display(candidate),
        width,
    ));
    intro_lines.extend(crate::migration::render::render_provider_selection_lines(
        &provider_selection,
        width,
    ));

    render_onboard_choice_screen(
        OnboardHeaderStyle::Brand,
        width,
        crate::onboard_presentation::single_detected_starting_point_preview_subtitle(),
        crate::onboard_presentation::single_detected_starting_point_preview_title(),
        None,
        intro_lines,
        Vec::new(),
        vec![
            crate::onboard_presentation::single_detected_starting_point_preview_footer().to_owned(),
        ],
        color_enabled,
    )
}

fn print_import_candidates(
    ui: &mut impl OnboardUi,
    candidates: &[ImportCandidate],
    context: &OnboardRuntimeContext,
) -> CliResult<()> {
    print_lines(
        ui,
        render_starting_point_selection_screen_lines_with_style(
            candidates,
            context.render_width,
            true,
        ),
    )
}

fn build_onboard_review_candidate_with_guidance(
    config: &mvp::config::LoongClawConfig,
    workspace_guidance: &[crate::migration::WorkspaceGuidanceCandidate],
) -> crate::migration::ImportCandidate {
    crate::migration::build_import_candidate(
        crate::migration::ImportSourceKind::CurrentSetup,
        crate::source_presentation::current_onboarding_draft_source_label().to_owned(),
        config.clone(),
        crate::migration::resolve_channel_import_readiness_from_config,
        workspace_guidance.to_vec(),
    )
    .unwrap_or_else(|| crate::migration::ImportCandidate {
        source_kind: crate::migration::ImportSourceKind::CurrentSetup,
        source: crate::source_presentation::current_onboarding_draft_source_label().to_owned(),
        config: config.clone(),
        surfaces: Vec::new(),
        domains: Vec::new(),
        channel_candidates: Vec::new(),
        workspace_guidance: workspace_guidance.to_vec(),
    })
}

pub fn render_onboard_review_lines_with_guidance(
    config: &mvp::config::LoongClawConfig,
    import_source: Option<&str>,
    workspace_guidance: &[crate::migration::WorkspaceGuidanceCandidate],
    width: usize,
) -> Vec<String> {
    render_onboard_review_lines_with_guidance_and_style(
        config,
        import_source,
        workspace_guidance,
        None,
        width,
        ReviewFlowStyle::Guided(GuidedPromptPath::NativePromptPack),
        false,
    )
}

pub fn render_current_setup_review_lines_with_guidance(
    config: &mvp::config::LoongClawConfig,
    import_source: Option<&str>,
    workspace_guidance: &[crate::migration::WorkspaceGuidanceCandidate],
    width: usize,
) -> Vec<String> {
    render_onboard_review_lines_with_guidance_and_style(
        config,
        import_source,
        workspace_guidance,
        None,
        width,
        ReviewFlowStyle::QuickCurrentSetup,
        false,
    )
}

pub fn render_detected_setup_review_lines_with_guidance(
    config: &mvp::config::LoongClawConfig,
    import_source: Option<&str>,
    workspace_guidance: &[crate::migration::WorkspaceGuidanceCandidate],
    width: usize,
) -> Vec<String> {
    render_onboard_review_lines_with_guidance_and_style(
        config,
        import_source,
        workspace_guidance,
        None,
        width,
        ReviewFlowStyle::QuickDetectedSetup,
        false,
    )
}

fn channel_candidates_match(
    left: &[crate::migration::ChannelCandidate],
    right: &[crate::migration::ChannelCandidate],
) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.id == right.id
                && left.label == right.label
                && left.status == right.status
                && left.summary == right.summary
        })
}

fn should_preserve_review_domain(
    kind: crate::migration::SetupDomainKind,
    config: &mvp::config::LoongClawConfig,
    workspace_guidance: &[crate::migration::WorkspaceGuidanceCandidate],
    selected_candidate: &ImportCandidate,
    channels_unchanged: bool,
) -> bool {
    match kind {
        crate::migration::SetupDomainKind::Provider => {
            provider_matches_for_review(&selected_candidate.config.provider, &config.provider)
        }
        crate::migration::SetupDomainKind::Channels => channels_unchanged,
        crate::migration::SetupDomainKind::Cli => selected_candidate.config.cli == config.cli,
        crate::migration::SetupDomainKind::Memory => {
            selected_candidate.config.memory == config.memory
        }
        crate::migration::SetupDomainKind::Tools => selected_candidate.config.tools == config.tools,
        crate::migration::SetupDomainKind::WorkspaceGuidance => {
            selected_candidate.workspace_guidance.as_slice() == workspace_guidance
        }
    }
}

fn provider_matches_for_review(
    left: &mvp::config::ProviderConfig,
    right: &mvp::config::ProviderConfig,
) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();

    left.api_key = None;
    left.api_key_env = None;
    left.oauth_access_token = None;
    left.oauth_access_token_env = None;

    right.api_key = None;
    right.api_key_env = None;
    right.oauth_access_token = None;
    right.oauth_access_token_env = None;

    left == right
}

fn build_onboard_review_candidate_with_selected_context(
    config: &mvp::config::LoongClawConfig,
    workspace_guidance: &[crate::migration::WorkspaceGuidanceCandidate],
    selected_candidate: Option<&ImportCandidate>,
) -> crate::migration::ImportCandidate {
    let draft_candidate = build_onboard_review_candidate_with_guidance(config, workspace_guidance);
    let Some(selected_candidate) = selected_candidate else {
        return draft_candidate;
    };
    if selected_candidate.config == *config
        && selected_candidate.workspace_guidance.as_slice() == workspace_guidance
    {
        return migration_candidate_for_onboard_display(selected_candidate);
    }

    let channels_unchanged = channel_candidates_match(
        &draft_candidate.channel_candidates,
        &selected_candidate.channel_candidates,
    );
    let mut review_candidate = draft_candidate;

    if channels_unchanged {
        review_candidate.channel_candidates = selected_candidate.channel_candidates.clone();
    }
    if selected_candidate.workspace_guidance.as_slice() == workspace_guidance {
        review_candidate.workspace_guidance = selected_candidate.workspace_guidance.clone();
    }

    for domain in &mut review_candidate.domains {
        if should_preserve_review_domain(
            domain.kind,
            config,
            workspace_guidance,
            selected_candidate,
            channels_unchanged,
        ) {
            if let Some(selected_domain) = selected_candidate
                .domains
                .iter()
                .find(|selected_domain| selected_domain.kind == domain.kind)
            {
                *domain = selected_domain.clone();
            }
        } else {
            domain.decision = Some(crate::migration::types::PreviewDecision::AdjustedInSession);
        }
    }

    review_candidate
}

fn render_onboard_review_lines_with_guidance_and_style(
    config: &mvp::config::LoongClawConfig,
    import_source: Option<&str>,
    workspace_guidance: &[crate::migration::WorkspaceGuidanceCandidate],
    selected_candidate: Option<&ImportCandidate>,
    width: usize,
    flow_style: ReviewFlowStyle,
    color_enabled: bool,
) -> Vec<String> {
    let mut lines = render_onboard_brand_header(width, flow_style.header_subtitle(), color_enabled);
    lines.push(String::new());
    lines.push("review setup".to_owned());
    lines.push(flow_style.progress_line());
    if let Some(source) = import_source {
        lines.extend(mvp::presentation::render_wrapped_text_line(
            "- starting point: ",
            &onboard_starting_point_label(None, source),
            width,
        ));
    }
    lines.extend(render_onboard_review_digest_lines(config, width));
    let review_candidate = build_onboard_review_candidate_with_selected_context(
        config,
        workspace_guidance,
        selected_candidate,
    );
    lines.extend(crate::migration::render::render_candidate_preview_lines(
        &review_candidate,
        width,
    ));
    lines
}

pub fn build_onboarding_success_summary(
    path: &Path,
    config: &mvp::config::LoongClawConfig,
    import_source: Option<&str>,
) -> OnboardingSuccessSummary {
    build_onboarding_success_summary_with_memory(path, config, import_source, None, None, None)
}

fn collect_onboarding_domain_outcomes(
    review_candidate: Option<&crate::migration::ImportCandidate>,
) -> Vec<OnboardingDomainOutcome> {
    review_candidate
        .into_iter()
        .flat_map(|candidate| candidate.domains.iter())
        .filter_map(|domain| {
            domain.decision.map(|decision| OnboardingDomainOutcome {
                kind: domain.kind,
                decision,
            })
        })
        .collect()
}

fn build_onboarding_success_summary_with_memory(
    path: &Path,
    config: &mvp::config::LoongClawConfig,
    import_source: Option<&str>,
    review_candidate: Option<&crate::migration::ImportCandidate>,
    memory_path: Option<&str>,
    config_status: Option<&str>,
) -> OnboardingSuccessSummary {
    let config_path = path.display().to_string();
    let next_actions = crate::next_actions::collect_setup_next_actions(config, &config_path)
        .into_iter()
        .map(|action| OnboardingAction {
            kind: match action.kind {
                crate::next_actions::SetupNextActionKind::Ask => OnboardingActionKind::Ask,
                crate::next_actions::SetupNextActionKind::Chat => OnboardingActionKind::Chat,
                crate::next_actions::SetupNextActionKind::Channel => OnboardingActionKind::Channel,
                crate::next_actions::SetupNextActionKind::BrowserPreview => {
                    OnboardingActionKind::BrowserPreview
                }
                crate::next_actions::SetupNextActionKind::Doctor => OnboardingActionKind::Doctor,
            },
            label: action.label,
            command: action.command,
        })
        .collect();

    OnboardingSuccessSummary {
        import_source: import_source.map(str::to_owned),
        config_path,
        config_status: config_status.map(str::to_owned),
        provider: crate::provider_presentation::active_provider_label(config),
        saved_provider_profiles: crate::provider_presentation::saved_provider_profile_ids(config),
        model: config.provider.model.clone(),
        transport: config.provider.transport_readiness().summary,
        credential: summarize_provider_credential(&config.provider),
        prompt_mode: summarize_prompt_mode(config),
        personality: config
            .cli
            .uses_native_prompt_pack()
            .then(|| prompt_personality_id(config.cli.resolved_personality()).to_owned()),
        prompt_addendum: summarize_prompt_addendum(config),
        memory_profile: memory_profile_id(config.memory.profile).to_owned(),
        memory_path: memory_path.map(str::to_owned),
        channels: enabled_channel_ids(config),
        domain_outcomes: collect_onboarding_domain_outcomes(review_candidate),
        next_actions,
    }
}

fn summarize_prompt_mode(config: &mvp::config::LoongClawConfig) -> String {
    if config.cli.uses_native_prompt_pack() {
        "native prompt pack".to_owned()
    } else {
        "inline system prompt override".to_owned()
    }
}

fn summarize_prompt_addendum(config: &mvp::config::LoongClawConfig) -> Option<String> {
    config
        .cli
        .system_prompt_addendum
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn render_onboarding_domain_outcome_lines(
    outcomes: &[OnboardingDomainOutcome],
    width: usize,
) -> Vec<String> {
    let mut grouped: Vec<(crate::migration::types::PreviewDecision, Vec<&'static str>)> =
        Vec::new();
    let mut sorted = outcomes.to_vec();
    sorted.sort_by_key(|outcome| (outcome.decision.outcome_rank(), outcome.kind));
    for outcome in sorted {
        if let Some((_, labels)) = grouped
            .iter_mut()
            .find(|(decision, _)| *decision == outcome.decision)
        {
            labels.push(outcome.kind.label());
        } else {
            grouped.push((outcome.decision, vec![outcome.kind.label()]));
        }
    }
    grouped
        .into_iter()
        .flat_map(|(decision, labels)| {
            mvp::presentation::render_wrapped_csv_line(
                &format!("- {}: ", decision.outcome_label()),
                &labels,
                width,
            )
        })
        .collect()
}

fn render_onboarding_success_summary(summary: &OnboardingSuccessSummary) -> Vec<String> {
    render_onboarding_success_summary_with_width_and_style(summary, detect_render_width(), true)
}

pub fn render_onboarding_success_summary_with_width(
    summary: &OnboardingSuccessSummary,
    width: usize,
) -> Vec<String> {
    render_onboarding_success_summary_with_width_and_style(summary, width, false)
}

fn render_onboarding_success_summary_with_width_and_style(
    summary: &OnboardingSuccessSummary,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let mut lines = render_onboard_brand_header(width, "setup complete", color_enabled);
    lines.push(String::new());
    lines.push("onboarding complete".to_owned());
    lines.extend(mvp::presentation::render_wrapped_text_line(
        "- config: ",
        &summary.config_path,
        width,
    ));
    if let Some(config_status) = summary.config_status.as_deref() {
        lines.extend(mvp::presentation::render_wrapped_text_line(
            "- config status: ",
            config_status,
            width,
        ));
    }
    if let Some(source) = summary.import_source.as_deref() {
        lines.extend(mvp::presentation::render_wrapped_text_line(
            "- starting point: ",
            &onboard_starting_point_label(None, source),
            width,
        ));
    }
    lines.extend(
        crate::provider_presentation::render_provider_profile_state_lines_from_parts(
            &summary.provider,
            &summary.saved_provider_profiles,
            width,
            Some("- provider: "),
        ),
    );
    lines.extend(mvp::presentation::render_wrapped_text_line(
        "- model: ",
        &summary.model,
        width,
    ));
    lines.extend(mvp::presentation::render_wrapped_text_line(
        "- transport: ",
        &summary.transport,
        width,
    ));
    if let Some(credential) = summary.credential.as_ref() {
        lines.extend(mvp::presentation::render_wrapped_text_line(
            &format!("- {}: ", credential.label),
            &credential.value,
            width,
        ));
    }
    lines.extend(mvp::presentation::render_wrapped_text_line(
        "- prompt mode: ",
        &summary.prompt_mode,
        width,
    ));
    if let Some(personality) = summary.personality.as_deref() {
        lines.extend(mvp::presentation::render_wrapped_text_line(
            "- personality: ",
            personality,
            width,
        ));
    }
    if let Some(prompt_addendum) = summary.prompt_addendum.as_deref() {
        lines.extend(mvp::presentation::render_wrapped_text_line(
            "- prompt addendum: ",
            prompt_addendum,
            width,
        ));
    }
    lines.extend(mvp::presentation::render_wrapped_text_line(
        "- memory profile: ",
        &summary.memory_profile,
        width,
    ));
    if let Some(memory_path) = summary.memory_path.as_deref() {
        lines.extend(mvp::presentation::render_wrapped_text_line(
            "- sqlite memory: ",
            memory_path,
            width,
        ));
    }
    if !summary.channels.is_empty() {
        let channels = summary
            .channels
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        lines.extend(mvp::presentation::render_wrapped_csv_line(
            "- channels: ",
            &channels,
            width,
        ));
    }
    if !summary.domain_outcomes.is_empty() {
        lines.push("setup outcome".to_owned());
        lines.extend(render_onboarding_domain_outcome_lines(
            &summary.domain_outcomes,
            width,
        ));
    }
    if summary.next_actions.is_empty() {
        return lines;
    }

    let mut actions = summary.next_actions.iter();
    if let Some(primary) = actions.next() {
        if width < 56 {
            lines.push("start here".to_owned());
            lines.extend(mvp::presentation::render_wrapped_text_line(
                &format!("- {}: ", primary.label),
                &primary.command,
                width,
            ));
        } else {
            lines.extend(mvp::presentation::render_wrapped_text_line(
                "start here: ",
                &primary.command,
                width,
            ));
        }
    }

    let secondary_actions = actions.collect::<Vec<_>>();
    if secondary_actions.is_empty() {
        return lines;
    }

    lines.push("also available".to_owned());
    lines.extend(secondary_actions.into_iter().flat_map(|action| {
        mvp::presentation::render_wrapped_text_line(
            &format!("- {}: ", action.label),
            &action.command,
            width,
        )
    }));
    lines
}

fn render_onboard_brand_header(width: usize, subtitle: &str, color_enabled: bool) -> Vec<String> {
    mvp::presentation::style_brand_lines_with_palette(
        &mvp::presentation::render_brand_header(
            width,
            &mvp::presentation::BuildVersionInfo::current(),
            Some(subtitle),
        ),
        color_enabled,
        mvp::presentation::ONBOARD_BRAND_PALETTE,
    )
}

fn render_onboard_compact_header(width: usize, subtitle: &str, color_enabled: bool) -> Vec<String> {
    mvp::presentation::style_brand_lines_with_palette(
        &mvp::presentation::render_compact_brand_header(
            width,
            &mvp::presentation::BuildVersionInfo::current(),
            Some(subtitle),
        ),
        color_enabled,
        mvp::presentation::ONBOARD_BRAND_PALETTE,
    )
}

fn render_onboard_header(
    style: OnboardHeaderStyle,
    width: usize,
    subtitle: &str,
    color_enabled: bool,
) -> Vec<String> {
    match style {
        OnboardHeaderStyle::Brand => render_onboard_brand_header(width, subtitle, color_enabled),
        OnboardHeaderStyle::Compact => {
            render_onboard_compact_header(width, subtitle, color_enabled)
        }
    }
}

fn render_onboard_wrapped_display_lines<I, S>(display_lines: I, width: usize) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    display_lines
        .into_iter()
        .flat_map(|line| mvp::presentation::render_wrapped_display_line(line.as_ref(), width))
        .collect()
}

fn render_onboard_option_lines(options: &[OnboardScreenOption], width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for option in options {
        let suffix = if option.recommended {
            " (recommended)"
        } else {
            ""
        };
        lines.extend(
            mvp::presentation::render_wrapped_text_line_with_continuation(
                &format!("[{}] ", option.key),
                "    ",
                &format!("{}{}", option.label, suffix),
                width,
            ),
        );
        lines.extend(render_onboard_wrapped_display_lines(
            option
                .detail_lines
                .iter()
                .map(|detail| format!("    {detail}"))
                .collect::<Vec<_>>(),
            width,
        ));
    }
    lines
}

fn render_default_choice_footer_line(key: &str, description: &str) -> String {
    format!("press Enter to use [{key}], {description}")
}

fn render_default_input_hint_line(description: impl AsRef<str>) -> String {
    format!("- press Enter to {}", description.as_ref())
}

fn render_clear_input_hint_line(description: impl AsRef<str>) -> String {
    format!(
        "- type {ONBOARD_CLEAR_INPUT_TOKEN} to {}",
        description.as_ref()
    )
}

fn render_model_selection_default_hint_line(
    config: &mvp::config::LoongClawConfig,
    prompt_default: &str,
) -> String {
    let prompt_default = prompt_default.trim();
    let current_model = config.provider.model.trim();
    if prompt_default == current_model {
        render_default_input_hint_line("keep current model")
    } else if prompt_default.is_empty() {
        render_default_input_hint_line("leave the model blank")
    } else {
        render_default_input_hint_line(format!("use prefilled model: {prompt_default}"))
    }
}

fn render_api_key_env_selection_default_hint_line(
    config: &mvp::config::LoongClawConfig,
    suggested_env: &str,
    prompt_default: &str,
) -> String {
    let prompt_default = render_provider_credential_source_value(Some(prompt_default))
        .unwrap_or_else(|| prompt_default.trim().to_owned());
    let suggested_env = render_provider_credential_source_value(Some(suggested_env))
        .unwrap_or_else(|| suggested_env.trim().to_owned());
    let current_env = config
        .provider
        .api_key_env
        .as_deref()
        .and_then(|value| render_provider_credential_source_value(Some(value)));

    if prompt_default.is_empty() {
        return render_default_input_hint_line("leave this blank");
    }

    if current_env
        .as_deref()
        .is_some_and(|current_env| current_env == prompt_default)
    {
        return render_default_input_hint_line("keep current source");
    }

    if !suggested_env.is_empty() && prompt_default == suggested_env {
        return render_default_input_hint_line(format!("use suggested source: {prompt_default}"));
    }

    render_default_input_hint_line(format!("use prefilled source: {prompt_default}"))
}

fn render_system_prompt_selection_default_hint_line(
    config: &mvp::config::LoongClawConfig,
    prompt_default: &str,
) -> String {
    let prompt_default = prompt_default.trim();
    let current_prompt = config.cli.system_prompt.trim();

    if prompt_default == current_prompt {
        if current_prompt.is_empty() {
            render_default_input_hint_line("keep the built-in default")
        } else {
            render_default_input_hint_line("keep current prompt")
        }
    } else if prompt_default.is_empty() {
        render_default_input_hint_line("keep the built-in default")
    } else {
        render_default_input_hint_line(format!("use prefilled prompt: {prompt_default}"))
    }
}

fn with_default_choice_footer(
    mut footer_lines: Vec<String>,
    default_choice_line: Option<String>,
) -> Vec<String> {
    if let Some(default_choice_line) = default_choice_line {
        footer_lines.insert(0, default_choice_line);
    }
    footer_lines
}

fn render_onboard_choice_screen(
    header_style: OnboardHeaderStyle,
    width: usize,
    subtitle: &str,
    title: &str,
    step: Option<(GuidedOnboardStep, GuidedPromptPath)>,
    intro_lines: Vec<String>,
    options: Vec<OnboardScreenOption>,
    footer_lines: Vec<String>,
    color_enabled: bool,
) -> Vec<String> {
    let mut lines = render_onboard_header(header_style, width, subtitle, color_enabled);
    lines.push(String::new());
    lines.extend(render_onboard_wrapped_display_lines([title], width));
    if let Some((step, guided_prompt_path)) = step {
        lines.extend(render_onboard_wrapped_display_lines(
            [step.progress_line(guided_prompt_path)],
            width,
        ));
    }
    lines.extend(render_onboard_wrapped_display_lines(intro_lines, width));
    if !options.is_empty() {
        lines.push(String::new());
        lines.extend(render_onboard_option_lines(&options, width));
    }
    if !footer_lines.is_empty() {
        lines.push(String::new());
        lines.extend(render_onboard_wrapped_display_lines(footer_lines, width));
    }
    lines
}

fn render_onboard_input_screen(
    width: usize,
    title: &str,
    step: GuidedOnboardStep,
    guided_prompt_path: GuidedPromptPath,
    context_lines: Vec<String>,
    hint_lines: Vec<String>,
    color_enabled: bool,
) -> Vec<String> {
    let mut lines = render_onboard_header(OnboardHeaderStyle::Compact, width, "", color_enabled);
    lines.push(String::new());
    lines.extend(render_onboard_wrapped_display_lines([title], width));
    lines.extend(render_onboard_wrapped_display_lines(
        [step.progress_line(guided_prompt_path)],
        width,
    ));
    lines.extend(render_onboard_wrapped_display_lines(context_lines, width));
    if !hint_lines.is_empty() {
        lines.push(String::new());
        lines.extend(render_onboard_wrapped_display_lines(hint_lines, width));
    }
    lines
}

pub fn render_continue_current_setup_screen_lines(
    config: &mvp::config::LoongClawConfig,
    width: usize,
) -> Vec<String> {
    render_onboard_shortcut_screen_lines_with_style(
        OnboardShortcutKind::CurrentSetup,
        config,
        None,
        width,
        false,
    )
}

pub fn render_continue_detected_setup_screen_lines(
    config: &mvp::config::LoongClawConfig,
    import_source: &str,
    width: usize,
) -> Vec<String> {
    render_onboard_shortcut_screen_lines_with_style(
        OnboardShortcutKind::DetectedSetup,
        config,
        Some(import_source),
        width,
        false,
    )
}

fn render_onboard_shortcut_screen_lines_with_style(
    shortcut_kind: OnboardShortcutKind,
    config: &mvp::config::LoongClawConfig,
    import_source: Option<&str>,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let mut context_lines = Vec::new();
    if let Some(source) = import_source {
        context_lines.push(format!(
            "- starting point: {}",
            onboard_starting_point_label(None, source)
        ));
    }
    context_lines.extend(render_onboard_review_digest_lines(config, width));
    context_lines.push(shortcut_kind.summary_line().to_owned());

    render_onboard_choice_screen(
        OnboardHeaderStyle::Brand,
        width,
        shortcut_kind.subtitle(),
        shortcut_kind.title(),
        None,
        context_lines,
        vec![
            OnboardScreenOption {
                key: "1".to_owned(),
                label: shortcut_kind.primary_label().to_owned(),
                detail_lines: vec![
                    crate::onboard_presentation::shortcut_continue_detail().to_owned(),
                ],
                recommended: true,
            },
            OnboardScreenOption {
                key: "2".to_owned(),
                label: crate::onboard_presentation::adjust_settings_label().to_owned(),
                detail_lines: vec![
                    crate::onboard_presentation::shortcut_adjust_detail().to_owned(),
                ],
                recommended: false,
            },
        ],
        vec![render_shortcut_default_choice_footer_line(shortcut_kind)],
        color_enabled,
    )
}

fn render_shortcut_default_choice_footer_line(shortcut_kind: OnboardShortcutKind) -> String {
    render_default_choice_footer_line("1", shortcut_kind.default_choice_description())
}

pub fn render_onboarding_risk_screen_lines(width: usize) -> Vec<String> {
    render_onboarding_risk_screen_lines_with_style(width, false)
}

fn render_onboarding_risk_screen_lines_with_style(
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let copy = crate::onboard_presentation::risk_screen_copy();
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        copy.subtitle,
        copy.title,
        None,
        vec![
            "- LoongClaw can invoke tools and read local files when enabled.".to_owned(),
            "- Keep credentials in environment variables, not in prompts.".to_owned(),
            "- Prefer allowlist-style tool policy for shared environments.".to_owned(),
        ],
        vec![
            OnboardScreenOption {
                key: "y".to_owned(),
                label: copy.continue_label.to_owned(),
                detail_lines: vec![copy.continue_detail.to_owned()],
                recommended: false,
            },
            OnboardScreenOption {
                key: "n".to_owned(),
                label: copy.cancel_label.to_owned(),
                detail_lines: vec![copy.cancel_detail.to_owned()],
                recommended: false,
            },
        ],
        vec![render_default_choice_footer_line(
            "n",
            copy.default_choice_description,
        )],
        color_enabled,
    )
}

pub fn render_preflight_summary_screen_lines(checks: &[OnboardCheck], width: usize) -> Vec<String> {
    render_preflight_summary_screen_lines_with_style(
        checks,
        width,
        ReviewFlowStyle::Guided(GuidedPromptPath::NativePromptPack),
        false,
    )
}

pub fn render_current_setup_preflight_summary_screen_lines(
    checks: &[OnboardCheck],
    width: usize,
) -> Vec<String> {
    render_preflight_summary_screen_lines_with_style(
        checks,
        width,
        ReviewFlowStyle::QuickCurrentSetup,
        false,
    )
}

pub fn render_detected_setup_preflight_summary_screen_lines(
    checks: &[OnboardCheck],
    width: usize,
) -> Vec<String> {
    render_preflight_summary_screen_lines_with_style(
        checks,
        width,
        ReviewFlowStyle::QuickDetectedSetup,
        false,
    )
}

fn render_preflight_summary_screen_lines_with_style(
    checks: &[OnboardCheck],
    width: usize,
    flow_style: ReviewFlowStyle,
    color_enabled: bool,
) -> Vec<String> {
    let counts = summarize_onboard_checks(checks);
    let has_attention = counts.warn > 0 || counts.fail > 0;
    let mut lines = render_onboard_compact_header(
        width,
        crate::onboard_presentation::preflight_header_title(),
        color_enabled,
    );
    let mut summary_lines = vec![format!(
        "- status: {} pass · {} warn · {} fail",
        counts.pass, counts.warn, counts.fail
    )];
    if has_attention {
        summary_lines
            .push(crate::onboard_presentation::preflight_attention_summary_line().to_owned());
        if counts.fail > 0 {
            summary_lines
                .push(crate::onboard_presentation::preflight_probe_rerun_hint().to_owned());
        }
    } else {
        summary_lines.push(crate::onboard_presentation::preflight_green_summary_line().to_owned());
    }
    lines.push(String::new());
    lines.extend(render_onboard_wrapped_display_lines(
        [crate::onboard_presentation::preflight_section_title()],
        width,
    ));
    lines.extend(render_onboard_wrapped_display_lines(
        [flow_style.progress_line()],
        width,
    ));
    lines.extend(render_onboard_wrapped_display_lines(summary_lines, width));
    if !checks.is_empty() {
        lines.push(String::new());
        lines.extend(render_preflight_check_rows(checks, width));
    }
    if has_attention {
        let options = vec![
            OnboardScreenOption {
                key: "y".to_owned(),
                label: crate::onboard_presentation::preflight_continue_label().to_owned(),
                detail_lines: vec![
                    crate::onboard_presentation::preflight_continue_detail().to_owned(),
                ],
                recommended: false,
            },
            OnboardScreenOption {
                key: "n".to_owned(),
                label: crate::onboard_presentation::preflight_cancel_label().to_owned(),
                detail_lines: vec![
                    crate::onboard_presentation::preflight_cancel_detail().to_owned(),
                ],
                recommended: false,
            },
        ];
        lines.push(String::new());
        lines.extend(render_onboard_option_lines(&options, width));
        lines.push(String::new());
        lines.extend(render_onboard_wrapped_display_lines(
            [render_default_choice_footer_line(
                "n",
                crate::onboard_presentation::preflight_default_choice_description(),
            )],
            width,
        ));
    }
    lines
}

pub fn render_write_confirmation_screen_lines(
    config_path: &str,
    warnings_kept: bool,
    width: usize,
) -> Vec<String> {
    render_write_confirmation_screen_lines_with_style(
        config_path,
        warnings_kept,
        width,
        ReviewFlowStyle::Guided(GuidedPromptPath::NativePromptPack),
        false,
    )
}

pub fn render_current_setup_write_confirmation_screen_lines(
    config_path: &str,
    warnings_kept: bool,
    width: usize,
) -> Vec<String> {
    render_write_confirmation_screen_lines_with_style(
        config_path,
        warnings_kept,
        width,
        ReviewFlowStyle::QuickCurrentSetup,
        false,
    )
}

pub fn render_detected_setup_write_confirmation_screen_lines(
    config_path: &str,
    warnings_kept: bool,
    width: usize,
) -> Vec<String> {
    render_write_confirmation_screen_lines_with_style(
        config_path,
        warnings_kept,
        width,
        ReviewFlowStyle::QuickDetectedSetup,
        false,
    )
}

fn render_write_confirmation_screen_lines_with_style(
    config_path: &str,
    warnings_kept: bool,
    width: usize,
    flow_style: ReviewFlowStyle,
    color_enabled: bool,
) -> Vec<String> {
    let mut context_lines = vec![format!("- config: {config_path}")];
    context_lines.push(
        crate::onboard_presentation::write_confirmation_status_line(warnings_kept).to_owned(),
    );
    let options = vec![
        OnboardScreenOption {
            key: "y".to_owned(),
            label: crate::onboard_presentation::write_confirmation_label().to_owned(),
            detail_lines: vec![crate::onboard_presentation::write_confirmation_detail().to_owned()],
            recommended: false,
        },
        OnboardScreenOption {
            key: "n".to_owned(),
            label: crate::onboard_presentation::write_confirmation_cancel_label().to_owned(),
            detail_lines: vec![
                crate::onboard_presentation::write_confirmation_cancel_detail().to_owned(),
            ],
            recommended: false,
        },
    ];
    let mut lines = render_onboard_header(OnboardHeaderStyle::Compact, width, "", color_enabled);
    lines.push(String::new());
    lines.extend(render_onboard_wrapped_display_lines(
        [crate::onboard_presentation::write_confirmation_title()],
        width,
    ));
    lines.extend(render_onboard_wrapped_display_lines(
        [flow_style.progress_line()],
        width,
    ));
    lines.extend(render_onboard_wrapped_display_lines(context_lines, width));
    lines.push(String::new());
    lines.extend(render_onboard_option_lines(&options, width));
    lines.push(String::new());
    lines.extend(render_onboard_wrapped_display_lines(
        [render_default_choice_footer_line(
            "y",
            crate::onboard_presentation::write_confirmation_default_choice_description(),
        )],
        width,
    ));
    lines
}

fn push_starting_point_fit_hint(
    hints: &mut Vec<StartingPointFitHint>,
    seen: &mut std::collections::BTreeSet<&'static str>,
    key: &'static str,
    detail: impl Into<String>,
    domain: Option<crate::migration::SetupDomainKind>,
) {
    if seen.insert(key) {
        hints.push(StartingPointFitHint {
            key,
            detail: detail.into(),
            domain,
        });
    }
}

fn summarize_direct_starting_point_source_reason(
    candidate: &ImportCandidate,
) -> Option<&'static str> {
    candidate.source_kind.direct_starting_point_reason()
}

fn collect_starting_point_fit_hints(candidate: &ImportCandidate) -> Vec<StartingPointFitHint> {
    let mut hints = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    if let Some(reason) = summarize_direct_starting_point_source_reason(candidate) {
        push_starting_point_fit_hint(&mut hints, &mut seen, "direct_source", reason, None);
    } else if let Some(provider_domain) = candidate
        .domains
        .iter()
        .find(|domain| domain.kind == crate::migration::SetupDomainKind::Provider)
        && let Some(decision) = provider_domain.decision
        && let Some(reason) = provider_domain.kind.starting_point_reason(decision)
    {
        let key = match decision {
            crate::migration::types::PreviewDecision::KeepCurrent => "provider_keep",
            crate::migration::types::PreviewDecision::UseDetected => "provider_detected",
            crate::migration::types::PreviewDecision::Supplement
            | crate::migration::types::PreviewDecision::ReviewConflict
            | crate::migration::types::PreviewDecision::AdjustedInSession => "provider",
        };
        push_starting_point_fit_hint(
            &mut hints,
            &mut seen,
            key,
            reason,
            Some(crate::migration::SetupDomainKind::Provider),
        );
    }

    if let Some(channels_domain) = candidate
        .domains
        .iter()
        .find(|domain| domain.kind == crate::migration::SetupDomainKind::Channels)
        && let Some(decision) = channels_domain.decision
        && let Some(reason) = channels_domain.kind.starting_point_reason(decision)
    {
        let key = match decision {
            crate::migration::types::PreviewDecision::Supplement => "channels_add",
            crate::migration::types::PreviewDecision::UseDetected => "channels_detected",
            crate::migration::types::PreviewDecision::KeepCurrent
            | crate::migration::types::PreviewDecision::ReviewConflict
            | crate::migration::types::PreviewDecision::AdjustedInSession => "channels",
        };
        push_starting_point_fit_hint(
            &mut hints,
            &mut seen,
            key,
            reason,
            Some(crate::migration::SetupDomainKind::Channels),
        );
    } else if !candidate.channel_candidates.is_empty()
        && let Some(reason) = crate::migration::SetupDomainKind::Channels
            .starting_point_reason(crate::migration::types::PreviewDecision::Supplement)
    {
        push_starting_point_fit_hint(
            &mut hints,
            &mut seen,
            "channels_add",
            reason,
            Some(crate::migration::SetupDomainKind::Channels),
        );
    }

    if (!candidate.workspace_guidance.is_empty()
        || candidate.domains.iter().any(|domain| {
            domain.kind == crate::migration::SetupDomainKind::WorkspaceGuidance
                && matches!(
                    domain.decision,
                    Some(crate::migration::types::PreviewDecision::UseDetected)
                        | Some(crate::migration::types::PreviewDecision::Supplement)
                )
        }))
        && let Some(reason) = crate::migration::SetupDomainKind::WorkspaceGuidance
            .starting_point_reason(crate::migration::types::PreviewDecision::UseDetected)
    {
        push_starting_point_fit_hint(
            &mut hints,
            &mut seen,
            "workspace_guidance",
            reason,
            Some(crate::migration::SetupDomainKind::WorkspaceGuidance),
        );
    }

    for (kind, key) in [
        (crate::migration::SetupDomainKind::Cli, "cli"),
        (crate::migration::SetupDomainKind::Memory, "memory"),
        (crate::migration::SetupDomainKind::Tools, "tools"),
    ] {
        if hints.len() >= 3 {
            break;
        }
        if candidate.domains.iter().any(|domain| {
            domain.kind == kind
                && matches!(
                    domain.decision,
                    Some(crate::migration::types::PreviewDecision::UseDetected)
                        | Some(crate::migration::types::PreviewDecision::Supplement)
                )
        }) && let Some(reason) =
            kind.starting_point_reason(crate::migration::types::PreviewDecision::UseDetected)
        {
            push_starting_point_fit_hint(&mut hints, &mut seen, key, reason, Some(kind));
        }
    }

    if hints.is_empty() {
        let source_count = crate::migration::render::candidate_source_rollup_labels(
            &migration_candidate_from_onboard(candidate),
        )
        .len();
        if source_count > 1 {
            push_starting_point_fit_hint(
                &mut hints,
                &mut seen,
                "combined_sources",
                format!("combine {source_count} reusable sources"),
                None,
            );
        }
    }

    hints
}

fn format_starting_point_reason(hints: &[StartingPointFitHint]) -> Option<String> {
    if hints.is_empty() {
        return None;
    }

    Some(format!(
        "good fit: {}",
        hints
            .iter()
            .take(3)
            .map(|hint| hint.detail.as_str())
            .collect::<Vec<_>>()
            .join(" + ")
    ))
}

fn should_include_starting_point_domain_decision(candidate: &ImportCandidate) -> bool {
    candidate.source_kind == crate::migration::ImportSourceKind::RecommendedPlan
}

fn format_starting_point_domain_detail(
    candidate: &ImportCandidate,
    domain: &crate::migration::DomainPreview,
) -> String {
    let mut detail = format!("{}: ", domain.kind.label());
    if should_include_starting_point_domain_decision(candidate)
        && let Some(decision) = domain.decision
    {
        detail.push_str(decision.label());
        detail.push_str(" · ");
    }
    detail.push_str(&domain.summary);
    detail
}

fn summarize_starting_point_detail_lines(candidate: &ImportCandidate, width: usize) -> Vec<String> {
    let mut details = Vec::new();
    let max_lines = if width < 68 { 4 } else { 5 };
    let mut detail_lines_used = 0usize;
    let has_channel_details = !candidate.channel_candidates.is_empty();
    let has_workspace_guidance_details = !candidate.workspace_guidance.is_empty();
    let migration_candidate = migration_candidate_from_onboard(candidate);
    let fit_hints = collect_starting_point_fit_hints(candidate);
    let emphasized_domains = if width < 68 {
        fit_hints
            .iter()
            .filter_map(|hint| hint.domain)
            .collect::<std::collections::BTreeSet<_>>()
    } else {
        std::collections::BTreeSet::new()
    };

    if let Some(reason_line) = format_starting_point_reason(&fit_hints) {
        details.push(reason_line);
    }

    let mut source_labels =
        crate::migration::render::candidate_source_rollup_labels(&migration_candidate);
    if has_workspace_guidance_details {
        source_labels.retain(|label| label != "workspace guidance");
    }
    let should_render_source_summary =
        if candidate.source_kind == crate::migration::ImportSourceKind::RecommendedPlan {
            !source_labels.is_empty()
        } else {
            source_labels.len() > 1
        };
    if should_render_source_summary {
        details.push(format!("sources: {}", source_labels.join(" + ")));
        detail_lines_used += 1;
    }

    for domain in &candidate.domains {
        if has_channel_details && domain.kind == crate::migration::SetupDomainKind::Channels {
            continue;
        }
        if has_workspace_guidance_details
            && domain.kind == crate::migration::SetupDomainKind::WorkspaceGuidance
        {
            continue;
        }
        if emphasized_domains.contains(&domain.kind) {
            continue;
        }
        details.push(format_starting_point_domain_detail(candidate, domain));
        detail_lines_used += 1;
        if detail_lines_used >= max_lines {
            return details;
        }
    }

    for channel in &candidate.channel_candidates {
        details.push(format!(
            "{}: {}",
            channel.label.to_ascii_lowercase(),
            channel.summary
        ));
        detail_lines_used += 1;
        if detail_lines_used >= max_lines {
            return details;
        }
    }

    if details.len() < max_lines && !candidate.workspace_guidance.is_empty() {
        let files = candidate
            .workspace_guidance
            .iter()
            .filter_map(|guidance| Path::new(&guidance.path).file_name())
            .map(|name| name.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        if !files.is_empty() {
            details.push(format!("workspace guidance: {}", files.join(", ")));
        }
    }

    if details.is_empty() {
        details.push("ready to use as a starting point".to_owned());
    }

    details
}

fn start_fresh_starting_point_detail_lines() -> Vec<String> {
    vec![
        crate::onboard_presentation::start_fresh_starting_point_fit_line().to_owned(),
        crate::onboard_presentation::start_fresh_starting_point_detail_line().to_owned(),
    ]
}

fn render_starting_point_selection_footer_lines(
    sorted_candidates: &[ImportCandidate],
) -> Vec<String> {
    let Some(first_candidate) = sorted_candidates.first() else {
        return Vec::new();
    };

    let first_hint = render_default_choice_footer_line(
        "1",
        crate::onboard_presentation::starting_point_footer_description(first_candidate.source_kind),
    );

    vec![first_hint]
}

pub fn render_starting_point_selection_screen_lines(
    candidates: &[ImportCandidate],
    width: usize,
) -> Vec<String> {
    render_starting_point_selection_screen_lines_with_style(candidates, width, false)
}

fn render_starting_point_selection_screen_lines_with_style(
    candidates: &[ImportCandidate],
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let sorted_candidates = sort_starting_point_candidates(candidates.to_vec());
    let mut options = sorted_candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| OnboardScreenOption {
            key: (index + 1).to_string(),
            label: onboard_starting_point_label(Some(candidate.source_kind), &candidate.source),
            detail_lines: summarize_starting_point_detail_lines(candidate, width),
            recommended: matches!(
                candidate.source_kind,
                crate::migration::ImportSourceKind::RecommendedPlan
            ),
        })
        .collect::<Vec<_>>();
    options.push(OnboardScreenOption {
        key: "0".to_owned(),
        label: crate::onboard_presentation::start_fresh_option_label().to_owned(),
        detail_lines: start_fresh_starting_point_detail_lines(),
        recommended: false,
    });
    let footer_lines = render_starting_point_selection_footer_lines(&sorted_candidates);

    render_onboard_choice_screen(
        OnboardHeaderStyle::Brand,
        width,
        crate::onboard_presentation::starting_point_selection_subtitle(),
        crate::onboard_presentation::starting_point_selection_title(),
        None,
        vec![crate::onboard_presentation::starting_point_selection_hint().to_owned()],
        options,
        footer_lines,
        color_enabled,
    )
}

pub fn render_provider_selection_screen_lines(
    plan: &crate::migration::ProviderSelectionPlan,
    width: usize,
) -> Vec<String> {
    render_provider_selection_screen_lines_with_style(
        plan,
        GuidedPromptPath::NativePromptPack,
        width,
        false,
    )
}

fn render_provider_selection_screen_lines_with_style(
    plan: &crate::migration::ProviderSelectionPlan,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let intro = if plan.imported_choices.is_empty() {
        vec!["pick the provider that should back this setup".to_owned()]
    } else if plan.requires_explicit_choice {
        vec!["other detected settings stay merged".to_owned()]
    } else {
        vec!["review the detected provider choices for this setup".to_owned()]
    };
    let options = plan
        .imported_choices
        .iter()
        .map(|choice| OnboardScreenOption {
            key: choice.profile_id.clone(),
            label: provider_kind_display_name(choice.kind).to_owned(),
            detail_lines: {
                let mut detail_lines = vec![
                    format!("source: {}", choice.source),
                    format!("summary: {}", choice.summary),
                ];
                if let Some(selector_detail) =
                    crate::migration::provider_selection::selector_detail_line(
                        plan,
                        &choice.profile_id,
                        width,
                    )
                {
                    detail_lines.push(selector_detail);
                }
                if let Some(transport_summary) = choice.config.preview_transport_summary() {
                    detail_lines.push(format!("transport: {transport_summary}"));
                }
                detail_lines
            },
            recommended: Some(choice.profile_id.as_str()) == plan.default_profile_id.as_deref(),
        })
        .collect::<Vec<_>>();
    render_onboard_choice_screen(
        OnboardHeaderStyle::Brand,
        width,
        "choose the current provider",
        "choose active provider",
        Some((GuidedOnboardStep::Provider, guided_prompt_path)),
        intro,
        options,
        with_default_choice_footer(
            crate::migration::guidance_lines(plan, width),
            render_provider_selection_default_choice_footer_line(plan),
        ),
        color_enabled,
    )
}

fn render_provider_selection_default_choice_footer_line(
    plan: &crate::migration::ProviderSelectionPlan,
) -> Option<String> {
    if plan.requires_explicit_choice {
        return None;
    }
    let default_profile_id = plan.default_profile_id.as_deref()?;
    let default_kind = plan
        .imported_choices
        .iter()
        .find(|choice| choice.profile_id == default_profile_id)
        .map(|choice| choice.kind)
        .or(plan.default_kind)?;
    Some(render_default_choice_footer_line(
        default_profile_id,
        &format!("the {} provider", provider_kind_display_name(default_kind)),
    ))
}

pub fn render_model_selection_screen_lines(
    config: &mvp::config::LoongClawConfig,
    width: usize,
) -> Vec<String> {
    render_model_selection_screen_lines_with_style(
        config,
        config.provider.model.as_str(),
        GuidedPromptPath::NativePromptPack,
        width,
        false,
    )
}

pub fn render_model_selection_screen_lines_with_default(
    config: &mvp::config::LoongClawConfig,
    prompt_default: &str,
    width: usize,
) -> Vec<String> {
    render_model_selection_screen_lines_with_style(
        config,
        prompt_default,
        GuidedPromptPath::NativePromptPack,
        width,
        false,
    )
}

fn render_model_selection_screen_lines_with_style(
    config: &mvp::config::LoongClawConfig,
    prompt_default: &str,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let mut context_lines = vec![
        format!(
            "- provider: {}",
            crate::provider_presentation::guided_provider_label(config.provider.kind)
        ),
        format!("- current model: {}", config.provider.model),
    ];
    if let Some(default_model) = config
        .provider
        .kind
        .default_model()
        .filter(|default_model| *default_model != config.provider.model)
    {
        context_lines.push(format!("- provider default: {default_model}"));
    }

    render_onboard_input_screen(
        width,
        "choose model",
        GuidedOnboardStep::Model,
        guided_prompt_path,
        context_lines,
        vec![
            render_model_selection_default_hint_line(config, prompt_default),
            "- type any provider model id to override it".to_owned(),
        ],
        color_enabled,
    )
}

pub fn render_api_key_env_selection_screen_lines(
    config: &mvp::config::LoongClawConfig,
    default_api_key_env: &str,
    width: usize,
) -> Vec<String> {
    render_api_key_env_selection_screen_lines_with_style(
        config,
        default_api_key_env,
        default_api_key_env,
        GuidedPromptPath::NativePromptPack,
        width,
        false,
    )
}

pub fn render_api_key_env_selection_screen_lines_with_default(
    config: &mvp::config::LoongClawConfig,
    default_api_key_env: &str,
    prompt_default: &str,
    width: usize,
) -> Vec<String> {
    render_api_key_env_selection_screen_lines_with_style(
        config,
        default_api_key_env,
        prompt_default,
        GuidedPromptPath::NativePromptPack,
        width,
        false,
    )
}

fn render_api_key_env_selection_screen_lines_with_style(
    config: &mvp::config::LoongClawConfig,
    default_api_key_env: &str,
    prompt_default: &str,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let mut context_lines = vec![format!(
        "- provider: {}",
        crate::provider_presentation::guided_provider_label(config.provider.kind)
    )];
    if let Some(current_env) = config
        .provider
        .api_key_env
        .as_deref()
        .and_then(|value| render_provider_credential_source_value(Some(value)))
    {
        context_lines.push(format!("- current source: {current_env}"));
    }
    if let Some(suggested_source) =
        render_provider_credential_source_value(Some(default_api_key_env))
    {
        context_lines.push(format!("- suggested source: {suggested_source}"));
    }

    let mut hint_lines = vec![render_api_key_env_selection_default_hint_line(
        config,
        default_api_key_env,
        prompt_default,
    )];
    if provider_supports_blank_api_key_env(config) {
        if prompt_default.trim().is_empty() {
            hint_lines.push("- leave this blank to keep inline or oauth credentials".to_owned());
        } else {
            hint_lines.push(render_clear_input_hint_line(
                "keep inline or oauth credentials",
            ));
        }
    }

    render_onboard_input_screen(
        width,
        "choose credential source",
        GuidedOnboardStep::CredentialEnv,
        guided_prompt_path,
        context_lines,
        hint_lines,
        color_enabled,
    )
}

pub fn render_system_prompt_selection_screen_lines(
    config: &mvp::config::LoongClawConfig,
    width: usize,
) -> Vec<String> {
    render_system_prompt_selection_screen_lines_with_style(
        config,
        config.cli.system_prompt.as_str(),
        GuidedPromptPath::InlineOverride,
        width,
        false,
    )
}

pub fn render_system_prompt_selection_screen_lines_with_default(
    config: &mvp::config::LoongClawConfig,
    prompt_default: &str,
    width: usize,
) -> Vec<String> {
    render_system_prompt_selection_screen_lines_with_style(
        config,
        prompt_default,
        GuidedPromptPath::InlineOverride,
        width,
        false,
    )
}

fn render_system_prompt_selection_screen_lines_with_style(
    config: &mvp::config::LoongClawConfig,
    prompt_default: &str,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let current_prompt = config.cli.system_prompt.trim();
    let current_prompt_display = if current_prompt.is_empty() {
        "built-in default".to_owned()
    } else {
        current_prompt.to_owned()
    };

    render_onboard_input_screen(
        width,
        "adjust cli behavior",
        GuidedOnboardStep::PromptCustomization,
        guided_prompt_path,
        vec![format!("- current prompt: {current_prompt_display}")],
        vec![
            render_system_prompt_selection_default_hint_line(config, prompt_default),
            if prompt_default.trim().is_empty() {
                "- leave this blank to use the built-in behavior".to_owned()
            } else {
                render_clear_input_hint_line("use the built-in behavior")
            },
        ],
        color_enabled,
    )
}

pub fn render_personality_selection_screen_lines(
    config: &mvp::config::LoongClawConfig,
    width: usize,
) -> Vec<String> {
    render_personality_selection_screen_lines_with_style(
        config,
        config.cli.resolved_personality(),
        width,
        false,
    )
}

fn render_personality_selection_screen_lines_with_style(
    config: &mvp::config::LoongClawConfig,
    default_personality: mvp::prompt::PromptPersonality,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let options = [
        (
            mvp::prompt::PromptPersonality::CalmEngineering,
            "calm engineering",
            "rigorous, direct, and technically grounded",
        ),
        (
            mvp::prompt::PromptPersonality::FriendlyCollab,
            "friendly collab",
            "warm, cooperative, and explanatory when helpful",
        ),
        (
            mvp::prompt::PromptPersonality::AutonomousExecutor,
            "autonomous executor",
            "decisive, high-initiative, and execution-oriented",
        ),
    ]
    .into_iter()
    .map(|(personality, label, detail)| OnboardScreenOption {
        key: prompt_personality_id(personality).to_owned(),
        label: label.to_owned(),
        detail_lines: vec![detail.to_owned()],
        recommended: personality == default_personality,
    })
    .collect::<Vec<_>>();

    render_onboard_choice_screen(
        OnboardHeaderStyle::Brand,
        width,
        "choose how LoongClaw should speak and take initiative",
        "choose personality",
        Some((
            GuidedOnboardStep::Personality,
            GuidedPromptPath::NativePromptPack,
        )),
        vec![format!(
            "- current personality: {}",
            prompt_personality_id(config.cli.resolved_personality())
        )],
        options,
        vec![render_default_choice_footer_line(
            prompt_personality_id(default_personality),
            "the current personality",
        )],
        color_enabled,
    )
}

pub fn render_prompt_addendum_selection_screen_lines(
    config: &mvp::config::LoongClawConfig,
    width: usize,
) -> Vec<String> {
    render_prompt_addendum_selection_screen_lines_with_style(config, width, false)
}

fn render_prompt_addendum_selection_screen_lines_with_style(
    config: &mvp::config::LoongClawConfig,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let current_addendum = config
        .cli
        .system_prompt_addendum
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("none");

    render_onboard_input_screen(
        width,
        "adjust prompt addendum",
        GuidedOnboardStep::PromptCustomization,
        GuidedPromptPath::NativePromptPack,
        vec![
            format!(
                "- personality: {}",
                prompt_personality_id(config.cli.resolved_personality())
            ),
            format!("- current addendum: {current_addendum}"),
        ],
        vec![
            "- blank keeps the current addendum".to_owned(),
            "- type '-' to clear it".to_owned(),
        ],
        color_enabled,
    )
}

pub fn render_memory_profile_selection_screen_lines(
    config: &mvp::config::LoongClawConfig,
    width: usize,
) -> Vec<String> {
    render_memory_profile_selection_screen_lines_with_style(
        config,
        config.memory.profile,
        GuidedPromptPath::NativePromptPack,
        width,
        false,
    )
}

fn render_memory_profile_selection_screen_lines_with_style(
    config: &mvp::config::LoongClawConfig,
    default_profile: mvp::config::MemoryProfile,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let options = [
        (
            mvp::config::MemoryProfile::WindowOnly,
            "recent turns only",
            "load only the active sliding window",
        ),
        (
            mvp::config::MemoryProfile::WindowPlusSummary,
            "window plus summary",
            "add a summary block before the recent window",
        ),
        (
            mvp::config::MemoryProfile::ProfilePlusWindow,
            "profile plus window",
            "inject durable profile notes before the recent window",
        ),
    ]
    .into_iter()
    .map(|(profile, label, detail)| OnboardScreenOption {
        key: memory_profile_id(profile).to_owned(),
        label: label.to_owned(),
        detail_lines: vec![detail.to_owned()],
        recommended: profile == default_profile,
    })
    .collect::<Vec<_>>();

    render_onboard_choice_screen(
        OnboardHeaderStyle::Brand,
        width,
        "choose how much memory context LoongClaw should inject",
        "choose memory profile",
        Some((GuidedOnboardStep::MemoryProfile, guided_prompt_path)),
        vec![format!(
            "- current profile: {}",
            memory_profile_id(config.memory.profile)
        )],
        options,
        vec![render_default_choice_footer_line(
            memory_profile_id(default_profile),
            "the current memory profile",
        )],
        color_enabled,
    )
}

pub fn render_existing_config_write_screen_lines(config_path: &str, width: usize) -> Vec<String> {
    render_existing_config_write_screen_lines_with_style(config_path, width, false)
}

fn render_existing_config_write_screen_lines_with_style(
    config_path: &str,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "decide how to write the config",
        "existing config found",
        None,
        vec![
            format!("- config: {config_path}"),
            "- choose whether to replace it, keep a backup, or cancel".to_owned(),
        ],
        vec![
            OnboardScreenOption {
                key: "o".to_owned(),
                label: "Replace existing config".to_owned(),
                detail_lines: vec![
                    "overwrite the current file with this onboarding draft".to_owned(),
                ],
                recommended: false,
            },
            OnboardScreenOption {
                key: "b".to_owned(),
                label: "Create backup and replace".to_owned(),
                detail_lines: vec![
                    "save a timestamped .bak copy first, then write the new config".to_owned(),
                ],
                recommended: false,
            },
            OnboardScreenOption {
                key: "c".to_owned(),
                label: "Cancel".to_owned(),
                detail_lines: vec!["leave the existing config untouched".to_owned()],
                recommended: false,
            },
        ],
        vec![render_default_choice_footer_line("c", "cancel")],
        color_enabled,
    )
}

fn render_onboard_review_digest_lines(
    config: &mvp::config::LoongClawConfig,
    width: usize,
) -> Vec<String> {
    let mut lines = crate::provider_presentation::render_provider_profile_state_lines(
        config,
        width,
        Some("- provider: "),
    );
    lines.extend(mvp::presentation::render_wrapped_text_line(
        "- model: ",
        &config.provider.model,
        width,
    ));
    lines.extend(mvp::presentation::render_wrapped_text_line(
        "- transport: ",
        &config.provider.transport_readiness().summary,
        width,
    ));

    if let Some(credential_line) = render_onboard_review_credential_line(&config.provider) {
        lines.push(credential_line);
    }
    lines.extend(mvp::presentation::render_wrapped_text_line(
        "- prompt mode: ",
        &summarize_prompt_mode(config),
        width,
    ));
    if config.cli.uses_native_prompt_pack() {
        lines.extend(mvp::presentation::render_wrapped_text_line(
            "- personality: ",
            prompt_personality_id(config.cli.resolved_personality()),
            width,
        ));
        if let Some(prompt_addendum) = summarize_prompt_addendum(config) {
            lines.extend(mvp::presentation::render_wrapped_text_line(
                "- prompt addendum: ",
                &prompt_addendum,
                width,
            ));
        }
    }
    lines.extend(mvp::presentation::render_wrapped_text_line(
        "- memory profile: ",
        memory_profile_id(config.memory.profile),
        width,
    ));

    let enabled_channels = enabled_channel_ids(config)
        .into_iter()
        .filter(|channel| channel != "cli")
        .collect::<Vec<_>>();
    if !enabled_channels.is_empty() {
        let channels = enabled_channels
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        lines.extend(mvp::presentation::render_wrapped_csv_line(
            "- channels: ",
            &channels,
            width,
        ));
    }

    lines
}

fn render_onboard_review_credential_line(provider: &mvp::config::ProviderConfig) -> Option<String> {
    summarize_provider_credential(provider)
        .map(|credential| format!("- {}: {}", credential.label, credential.value))
}

fn summarize_provider_credential(
    provider: &mvp::config::ProviderConfig,
) -> Option<OnboardingCredentialSummary> {
    if provider
        .oauth_access_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return Some(OnboardingCredentialSummary {
            label: "credential",
            value: "inline oauth token".to_owned(),
        });
    }
    if let Some(oauth_env) = provider
        .oauth_access_token_env
        .as_deref()
        .and_then(|value| render_provider_credential_source_value(Some(value)))
    {
        return Some(OnboardingCredentialSummary {
            label: "credential source",
            value: oauth_env,
        });
    }
    if provider
        .api_key
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return Some(OnboardingCredentialSummary {
            label: "credential",
            value: "inline api key".to_owned(),
        });
    }
    provider
        .api_key_env
        .as_deref()
        .and_then(|value| render_provider_credential_source_value(Some(value)))
        .or_else(|| {
            provider
                .kind
                .default_api_key_env()
                .and_then(|value| render_provider_credential_source_value(Some(value)))
        })
        .map(|api_key_env| OnboardingCredentialSummary {
            label: "credential source",
            value: api_key_env,
        })
}

fn provider_supports_blank_api_key_env(config: &mvp::config::LoongClawConfig) -> bool {
    config
        .provider
        .api_key
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        || config
            .provider
            .oauth_access_token
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
        || config
            .provider
            .oauth_access_token_env
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
}

fn prompt_import_candidate_choice(
    ui: &mut impl OnboardUi,
    count: usize,
) -> CliResult<Option<usize>> {
    loop {
        let choice = ui.prompt_with_default("Starting point", "1")?;
        let trimmed = choice.trim();
        if trimmed == "0" {
            return Ok(None);
        }
        let Ok(selected) = trimmed.parse::<usize>() else {
            print_message(ui, format!("Invalid choice: {trimmed}"))?;
            continue;
        };
        if (1..=count).contains(&selected) {
            return Ok(Some(selected - 1));
        }
        print_message(ui, format!("Invalid choice: {trimmed}"))?;
    }
}

fn prompt_onboard_shortcut_choice(ui: &mut impl OnboardUi) -> CliResult<OnboardShortcutChoice> {
    loop {
        let choice = ui.prompt_with_default("Your choice", "1")?;
        match choice.trim() {
            "1" => return Ok(OnboardShortcutChoice::UseShortcut),
            "2" => return Ok(OnboardShortcutChoice::AdjustSettings),
            trimmed => print_message(ui, format!("Invalid choice: {trimmed}"))?,
        }
    }
}

pub fn detect_import_starting_config_with_channel_readiness(
    readiness: ChannelImportReadiness,
) -> mvp::config::LoongClawConfig {
    crate::migration::detect_import_starting_config_with_channel_readiness(to_migration_readiness(
        readiness,
    ))
}

fn resolve_channel_import_readiness(
    config: &mvp::config::LoongClawConfig,
) -> ChannelImportReadiness {
    crate::migration::resolve_channel_import_readiness_from_config(config)
}

fn default_codex_config_paths() -> Vec<PathBuf> {
    crate::migration::discovery::default_detected_codex_config_paths()
}

fn to_migration_readiness(
    readiness: ChannelImportReadiness,
) -> crate::migration::ChannelImportReadiness {
    readiness
}

fn import_surface_from_migration(surface: crate::migration::ImportSurface) -> ImportSurface {
    ImportSurface {
        name: surface.name,
        domain: surface.domain,
        level: match surface.level {
            crate::migration::ImportSurfaceLevel::Ready => ImportSurfaceLevel::Ready,
            crate::migration::ImportSurfaceLevel::Review => ImportSurfaceLevel::Review,
            crate::migration::ImportSurfaceLevel::Blocked => ImportSurfaceLevel::Blocked,
        },
        detail: surface.detail,
    }
}

fn import_surface_to_migration(surface: &ImportSurface) -> crate::migration::ImportSurface {
    crate::migration::ImportSurface {
        name: surface.name,
        domain: surface.domain,
        level: match surface.level {
            ImportSurfaceLevel::Ready => crate::migration::ImportSurfaceLevel::Ready,
            ImportSurfaceLevel::Review => crate::migration::ImportSurfaceLevel::Review,
            ImportSurfaceLevel::Blocked => crate::migration::ImportSurfaceLevel::Blocked,
        },
        detail: surface.detail.clone(),
    }
}

fn import_candidate_from_migration(
    candidate: crate::migration::ImportCandidate,
) -> ImportCandidate {
    ImportCandidate {
        source_kind: candidate.source_kind,
        source: candidate.source,
        config: candidate.config,
        surfaces: candidate
            .surfaces
            .into_iter()
            .map(import_surface_from_migration)
            .collect(),
        domains: candidate.domains,
        channel_candidates: candidate.channel_candidates,
        workspace_guidance: candidate.workspace_guidance,
    }
}

fn migration_candidate_from_onboard(
    candidate: &ImportCandidate,
) -> crate::migration::ImportCandidate {
    crate::migration::ImportCandidate {
        source_kind: candidate.source_kind,
        source: candidate.source.clone(),
        config: candidate.config.clone(),
        surfaces: candidate
            .surfaces
            .iter()
            .map(import_surface_to_migration)
            .collect(),
        domains: candidate.domains.clone(),
        channel_candidates: candidate.channel_candidates.clone(),
        workspace_guidance: candidate.workspace_guidance.clone(),
    }
}

fn migration_candidate_for_onboard_display(
    candidate: &ImportCandidate,
) -> crate::migration::ImportCandidate {
    let mut migration_candidate = migration_candidate_from_onboard(candidate);
    migration_candidate.source =
        onboard_starting_point_label(Some(candidate.source_kind), &candidate.source);
    migration_candidate
}

fn onboard_starting_point_label(
    source_kind: Option<crate::migration::ImportSourceKind>,
    source: &str,
) -> String {
    crate::migration::ImportSourceKind::onboarding_label(source_kind, source)
}

fn detect_render_width() -> usize {
    env::var("COLUMNS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|width| *width > 0)
        .unwrap_or(80)
}

fn enabled_channel_ids(config: &mvp::config::LoongClawConfig) -> Vec<String> {
    config.enabled_channel_ids()
}

pub fn validate_non_interactive_risk_gate(
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

pub fn should_offer_current_setup_shortcut(
    options: &OnboardCommandOptions,
    current_setup_state: crate::migration::CurrentSetupState,
    entry_choice: OnboardEntryChoice,
) -> bool {
    !options.non_interactive
        && entry_choice == OnboardEntryChoice::ContinueCurrentSetup
        && current_setup_state == crate::migration::CurrentSetupState::Healthy
        && !onboard_has_explicit_overrides(options)
}

pub fn should_offer_detected_setup_shortcut(
    options: &OnboardCommandOptions,
    entry_choice: OnboardEntryChoice,
    provider_selection: &crate::migration::ProviderSelectionPlan,
) -> bool {
    !options.non_interactive
        && entry_choice == OnboardEntryChoice::ImportDetectedSetup
        && !provider_selection.requires_explicit_choice
        && !onboard_has_explicit_overrides(options)
}

fn resolve_onboard_shortcut_kind(
    options: &OnboardCommandOptions,
    starting_selection: &StartingConfigSelection,
) -> Option<OnboardShortcutKind> {
    if should_offer_current_setup_shortcut(
        options,
        starting_selection.current_setup_state,
        starting_selection.entry_choice,
    ) {
        return Some(OnboardShortcutKind::CurrentSetup);
    }
    if should_offer_detected_setup_shortcut(
        options,
        starting_selection.entry_choice,
        &starting_selection.provider_selection,
    ) {
        return Some(OnboardShortcutKind::DetectedSetup);
    }
    None
}

fn onboard_has_explicit_overrides(options: &OnboardCommandOptions) -> bool {
    options
        .provider
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || options
            .model
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || options
            .api_key_env
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || options
            .personality
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || options
            .memory_profile
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || options
            .system_prompt
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

fn load_existing_output_config(output_path: &Path) -> Option<mvp::config::LoongClawConfig> {
    let path_str = output_path.to_str()?;
    mvp::config::load(Some(path_str))
        .ok()
        .map(|(_, config)| config)
}

pub fn should_skip_config_write(
    existing_config: Option<&mvp::config::LoongClawConfig>,
    draft: &mvp::config::LoongClawConfig,
) -> bool {
    existing_config.is_some_and(|existing| existing == draft)
}

pub fn parse_provider_kind(raw: &str) -> Option<mvp::config::ProviderKind> {
    mvp::config::ProviderKind::parse(raw)
}

pub fn parse_prompt_personality(raw: &str) -> Option<mvp::prompt::PromptPersonality> {
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

pub fn parse_memory_profile(raw: &str) -> Option<mvp::config::MemoryProfile> {
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

pub fn provider_default_api_key_env(kind: mvp::config::ProviderKind) -> Option<&'static str> {
    kind.default_api_key_env()
}

pub fn provider_kind_id(kind: mvp::config::ProviderKind) -> &'static str {
    kind.as_str()
}

pub fn provider_kind_display_name(kind: mvp::config::ProviderKind) -> &'static str {
    kind.display_name()
}

pub fn prompt_personality_id(personality: mvp::prompt::PromptPersonality) -> &'static str {
    match personality {
        mvp::prompt::PromptPersonality::CalmEngineering => "calm_engineering",
        mvp::prompt::PromptPersonality::FriendlyCollab => "friendly_collab",
        mvp::prompt::PromptPersonality::AutonomousExecutor => "autonomous_executor",
    }
}

pub fn memory_profile_id(profile: mvp::config::MemoryProfile) -> &'static str {
    match profile {
        mvp::config::MemoryProfile::WindowOnly => "window_only",
        mvp::config::MemoryProfile::WindowPlusSummary => "window_plus_summary",
        mvp::config::MemoryProfile::ProfilePlusWindow => "profile_plus_window",
    }
}

pub fn supported_provider_list() -> String {
    mvp::config::ProviderKind::all_sorted()
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn supported_personality_list() -> &'static str {
    "calm_engineering, friendly_collab, autonomous_executor"
}

pub fn supported_memory_profile_list() -> &'static str {
    "window_only, window_plus_summary, profile_plus_window"
}

fn resolve_write_plan(
    output_path: &Path,
    options: &OnboardCommandOptions,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<ConfigWritePlan> {
    if !output_path.exists() {
        return Ok(ConfigWritePlan {
            force: false,
            backup_path: None,
        });
    }
    if options.force {
        return Ok(ConfigWritePlan {
            force: true,
            backup_path: None,
        });
    }

    if options.non_interactive {
        return Err(format!(
            "config {} already exists (use --force to overwrite)",
            output_path.display()
        ));
    }

    let existing_path = output_path.display().to_string();
    print_lines(
        ui,
        render_existing_config_write_screen_lines_with_style(
            &existing_path,
            context.render_width,
            true,
        ),
    )?;
    loop {
        let choice = ui.prompt_with_default("Your choice", "c")?;
        match choice.trim().to_ascii_lowercase().as_str() {
            "o" | "overwrite" => {
                return Ok(ConfigWritePlan {
                    force: true,
                    backup_path: None,
                });
            }
            "b" | "backup" => {
                return Ok(ConfigWritePlan {
                    force: true,
                    backup_path: Some(resolve_backup_path(output_path)?),
                });
            }
            "c" | "cancel" => {
                return Err("onboarding cancelled: config file already exists".to_owned());
            }
            _ => {
                print_message(
                    ui,
                    "Invalid choice. Please enter 'o' (overwrite), 'b' (backup), or 'c' (cancel)",
                )?;
            }
        }
    }
}

fn prepare_output_path_for_write(
    output_path: &Path,
    plan: &ConfigWritePlan,
    ui: &mut impl OnboardUi,
) -> CliResult<OnboardWriteRecovery> {
    let output_preexisted = output_path.exists();
    let keep_backup_on_success = plan.backup_path.is_some();
    let backup_path = if output_preexisted {
        Some(
            plan.backup_path
                .clone()
                .unwrap_or(resolve_rollback_backup_path(output_path)?),
        )
    } else {
        None
    };

    if let Some(backup_path) = backup_path.as_deref() {
        backup_existing_config(output_path, backup_path)?;
    }
    if let Some(backup_path) = plan.backup_path.as_deref() {
        print_message(
            ui,
            format!("Backed up existing config to: {}", backup_path.display()),
        )?;
    }
    Ok(OnboardWriteRecovery {
        output_preexisted,
        backup_path,
        keep_backup_on_success,
    })
}

pub fn backup_existing_config(output_path: &Path, backup_path: &Path) -> CliResult<()> {
    fs::copy(output_path, backup_path)
        .map_err(|error| format!("failed to backup config: {error}"))?;
    Ok(())
}

impl OnboardWriteRecovery {
    fn rollback(&self, output_path: &Path) -> CliResult<()> {
        if self.output_preexisted {
            let backup_path = self
                .backup_path
                .as_deref()
                .ok_or_else(|| "missing rollback backup for existing config".to_owned())?;
            fs::copy(backup_path, output_path).map_err(|error| {
                format!(
                    "failed to restore original config {} from backup {}: {error}",
                    output_path.display(),
                    backup_path.display(),
                )
            })?;
            self.finish_success();
            return Ok(());
        }

        if output_path.exists() {
            fs::remove_file(output_path).map_err(|error| {
                format!(
                    "failed to remove partial config {} after onboarding failure: {error}",
                    output_path.display()
                )
            })?;
        }
        self.finish_success();
        Ok(())
    }

    fn finish_success(&self) {
        if self.keep_backup_on_success {
            return;
        }
        if let Some(backup_path) = self.backup_path.as_deref() {
            let _ = fs::remove_file(backup_path);
        }
    }
}

fn rollback_onboard_write_failure(
    output_path: &Path,
    write_recovery: &OnboardWriteRecovery,
    failure: impl Into<String>,
) -> String {
    let failure = failure.into();
    match write_recovery.rollback(output_path) {
        Ok(()) => failure,
        Err(rollback_error) => {
            format!("{failure}; additionally failed to restore original config: {rollback_error}")
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

fn resolve_rollback_backup_path(original: &Path) -> CliResult<PathBuf> {
    let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    resolve_rollback_backup_path_at(original, now)
}

fn resolve_rollback_backup_path_at(
    original: &Path,
    timestamp: OffsetDateTime,
) -> CliResult<PathBuf> {
    let parent = original.parent().unwrap_or(Path::new("."));
    let file_name = original
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "config.toml".to_owned());

    let formatted_timestamp = format_backup_timestamp_at(timestamp)?;
    Ok(parent.join(format!(
        ".{file_name}.onboard-rollback-{formatted_timestamp}"
    )))
}

fn format_backup_timestamp_at(timestamp: OffsetDateTime) -> CliResult<String> {
    timestamp
        .format(BACKUP_TIMESTAMP_FORMAT)
        .map_err(|error| format!("format backup timestamp failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct TestOnboardUi {
        inputs: VecDeque<String>,
    }

    impl TestOnboardUi {
        fn with_inputs(inputs: impl IntoIterator<Item = impl Into<String>>) -> Self {
            Self {
                inputs: inputs.into_iter().map(Into::into).collect(),
            }
        }
    }

    impl OnboardUi for TestOnboardUi {
        fn print_line(&mut self, _line: &str) -> CliResult<()> {
            Ok(())
        }

        fn prompt_with_default(&mut self, _label: &str, default: &str) -> CliResult<String> {
            Ok(self
                .inputs
                .pop_front()
                .unwrap_or_else(|| default.to_owned()))
        }

        fn prompt_required(&mut self, _label: &str) -> CliResult<String> {
            self.inputs
                .pop_front()
                .ok_or_else(|| "missing required test input".to_owned())
        }

        fn prompt_confirm(&mut self, _message: &str, default: bool) -> CliResult<bool> {
            Ok(self
                .inputs
                .pop_front()
                .map(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
                .unwrap_or(default))
        }
    }

    fn import_candidate_with_domain_status(
        source_kind: crate::migration::ImportSourceKind,
        source: &str,
        domains: impl IntoIterator<
            Item = (
                crate::migration::SetupDomainKind,
                crate::migration::PreviewStatus,
            ),
        >,
    ) -> ImportCandidate {
        ImportCandidate {
            source_kind,
            source: source.to_owned(),
            config: mvp::config::LoongClawConfig::default(),
            surfaces: Vec::new(),
            domains: domains
                .into_iter()
                .map(|(kind, status)| crate::migration::DomainPreview {
                    kind,
                    status,
                    decision: Some(crate::migration::types::PreviewDecision::UseDetected),
                    source: source.to_owned(),
                    summary: format!("{} {}", kind.label(), status.label()),
                })
                .collect(),
            channel_candidates: Vec::new(),
            workspace_guidance: Vec::new(),
        }
    }

    fn recommended_import_entry_options() -> Vec<OnboardEntryOption> {
        vec![
            OnboardEntryOption {
                choice: OnboardEntryChoice::ImportDetectedSetup,
                label: "Use detected starting point",
                detail: "detected setup is recommended".to_owned(),
                recommended: true,
            },
            OnboardEntryOption {
                choice: OnboardEntryChoice::StartFresh,
                label: "Start fresh",
                detail: "configure from scratch".to_owned(),
                recommended: false,
            },
        ]
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_preflight_checks_includes_provider_transport_review_for_responses_compatibility_mode()
     {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "deepseek-chat".to_owned();
        config.provider.wire_api = mvp::config::ProviderWireApi::Responses;

        let checks = run_preflight_checks(&config, true).await;

        assert!(
            checks.iter().any(|check| {
                check.name == "provider transport"
                    && check.level == OnboardCheckLevel::Warn
                    && check
                        .detail
                        .contains("retry chat_completions automatically")
            }),
            "preflight should surface transport review before writing a Responses-compatible config: {checks:#?}"
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
        assert_eq!(check.level, OnboardCheckLevel::Warn);
        assert!(
            check.detail.contains("explicitly configured"),
            "explicit-model probe failures should explain that catalog discovery is advisory: {check:#?}"
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
        assert_eq!(check.level, OnboardCheckLevel::Fail);
        assert!(
            check.detail.contains("OpenAI [openai]"),
            "onboard failures should still identify the active provider context: {check:#?}"
        );
    }

    #[test]
    fn explicit_model_probe_warning_is_accepted_non_interactively() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.model = "openai/gpt-5.1-codex".to_owned();
        let check = provider_model_probe_failure_check(
            &config,
            "provider rejected the model list".to_owned(),
        );
        let options = OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: true,
            accept_risk: true,
            provider: None,
            model: None,
            api_key_env: None,
            personality: None,
            memory_profile: None,
            system_prompt: None,
            skip_model_probe: false,
        };

        assert!(
            is_explicitly_accepted_non_interactive_warning(&check, &options),
            "explicit-model probe warnings should not block non-interactive onboarding because model discovery is advisory: {check:#?}"
        );
    }

    #[test]
    fn resolve_api_key_env_selection_accepts_explicit_clear_token_in_interactive_mode() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Openai;
        config.provider.api_key = Some("inline-secret".to_owned());
        let mut ui = TestOnboardUi::with_inputs([":clear"]);
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

        let selected = resolve_api_key_env_selection(
            &OnboardCommandOptions {
                output: None,
                force: false,
                non_interactive: false,
                accept_risk: true,
                provider: None,
                model: None,
                api_key_env: None,
                personality: None,
                memory_profile: None,
                system_prompt: None,
                skip_model_probe: false,
            },
            &config,
            "OPENAI_API_KEY".to_owned(),
            GuidedPromptPath::NativePromptPack,
            &mut ui,
            &context,
        )
        .expect("resolve api key env selection");

        assert!(
            selected.is_empty(),
            "typing :clear should explicitly clear the api-key env selection instead of persisting the literal token: {selected:?}"
        );
    }

    #[test]
    fn resolve_system_prompt_selection_accepts_explicit_clear_token_in_interactive_mode() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.cli.system_prompt = "be terse and code-focused".to_owned();
        let mut ui = TestOnboardUi::with_inputs([":clear"]);
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

        let selected = resolve_system_prompt_selection(
            &OnboardCommandOptions {
                output: None,
                force: false,
                non_interactive: false,
                accept_risk: true,
                provider: None,
                model: None,
                api_key_env: None,
                personality: None,
                memory_profile: None,
                system_prompt: None,
                skip_model_probe: false,
            },
            &config,
            &mut ui,
            &context,
        )
        .expect("resolve system prompt selection");

        assert_eq!(
            selected,
            SystemPromptSelection::RestoreBuiltIn,
            "typing :clear should restore the built-in system prompt instead of keeping the literal token"
        );
    }

    #[test]
    fn resolve_system_prompt_selection_keeps_current_prompt_when_interactive_default_is_used() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.cli.system_prompt = "be terse and code-focused".to_owned();
        let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

        let selected = resolve_system_prompt_selection(
            &OnboardCommandOptions {
                output: None,
                force: false,
                non_interactive: false,
                accept_risk: true,
                provider: None,
                model: None,
                api_key_env: None,
                personality: None,
                memory_profile: None,
                system_prompt: None,
                skip_model_probe: false,
            },
            &config,
            &mut ui,
            &context,
        )
        .expect("resolve system prompt selection");

        assert_eq!(
            selected,
            SystemPromptSelection::KeepCurrent,
            "using the prompt default should keep the current system prompt when no override is prefilled"
        );
    }

    #[test]
    fn resolve_system_prompt_selection_keeps_prefilled_override_when_interactive_default_is_used() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.cli.system_prompt = "be terse and code-focused".to_owned();
        let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

        let selected = resolve_system_prompt_selection(
            &OnboardCommandOptions {
                output: None,
                force: false,
                non_interactive: false,
                accept_risk: true,
                provider: None,
                model: None,
                api_key_env: None,
                personality: None,
                memory_profile: None,
                system_prompt: Some("prefer concise code reviews".to_owned()),
                skip_model_probe: false,
            },
            &config,
            &mut ui,
            &context,
        )
        .expect("resolve system prompt selection");

        assert_eq!(
            selected,
            SystemPromptSelection::Set("prefer concise code reviews".to_owned()),
            "using the prompt default should still apply a prefilled system prompt override"
        );
    }

    #[test]
    fn apply_selected_system_prompt_restore_uses_rendered_native_prompt() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.cli.system_prompt = "custom review prompt".to_owned();
        config.cli.system_prompt_addendum = Some("Prefer concrete remediation steps.".to_owned());
        let expected = config.cli.rendered_native_system_prompt();

        apply_selected_system_prompt(&mut config, SystemPromptSelection::RestoreBuiltIn);

        assert_eq!(
            config.cli.system_prompt, expected,
            "restoring the built-in prompt should respect the active native prompt rendering inputs"
        );
    }

    #[test]
    fn accepted_non_interactive_warnings_do_not_depend_on_display_text() {
        let check = OnboardCheck {
            name: "provider model probe",
            level: OnboardCheckLevel::Warn,
            detail: "display text changed".to_owned(),
            non_interactive_warning_policy:
                OnboardNonInteractiveWarningPolicy::AcceptedBySkipModelProbe,
        };
        let options = OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: true,
            accept_risk: true,
            provider: None,
            model: None,
            api_key_env: None,
            personality: None,
            memory_profile: None,
            system_prompt: None,
            skip_model_probe: true,
        };

        assert!(
            is_explicitly_accepted_non_interactive_warning(&check, &options),
            "non-interactive warning acceptance should follow structured policy rather than fragile display strings"
        );
    }

    #[test]
    fn select_non_interactive_starting_config_uses_sorted_detected_candidate_priority() {
        let codex_candidate = import_candidate_with_domain_status(
            crate::migration::ImportSourceKind::CodexConfig,
            "Codex config at ~/.codex/config.toml",
            [(
                crate::migration::SetupDomainKind::Provider,
                crate::migration::PreviewStatus::Ready,
            )],
        );
        let environment_candidate = import_candidate_with_domain_status(
            crate::migration::ImportSourceKind::Environment,
            "your current environment",
            [
                (
                    crate::migration::SetupDomainKind::Provider,
                    crate::migration::PreviewStatus::Ready,
                ),
                (
                    crate::migration::SetupDomainKind::Channels,
                    crate::migration::PreviewStatus::Ready,
                ),
                (
                    crate::migration::SetupDomainKind::WorkspaceGuidance,
                    crate::migration::PreviewStatus::Ready,
                ),
            ],
        );
        let all_candidates = vec![codex_candidate, environment_candidate];

        let selection = select_non_interactive_starting_config(
            crate::migration::CurrentSetupState::Absent,
            &recommended_import_entry_options(),
            None,
            all_candidates.clone(),
            &all_candidates,
        );

        assert_eq!(
            selection
                .review_candidate
                .as_ref()
                .map(|candidate| candidate.source_kind),
            Some(crate::migration::ImportSourceKind::Environment),
            "non-interactive onboarding should reuse the same sorted detected-candidate priority as the interactive chooser: {selection:#?}"
        );
    }

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

    #[test]
    fn rollback_removes_partial_first_write_config() {
        let output_path = std::env::temp_dir().join(format!(
            "loongclaw-first-write-rollback-{}.toml",
            std::process::id()
        ));
        fs::write(&output_path, "partial = true\n").expect("write partial config");

        let recovery = OnboardWriteRecovery {
            output_preexisted: false,
            backup_path: None,
            keep_backup_on_success: false,
        };

        recovery
            .rollback(&output_path)
            .expect("first-write rollback should succeed");

        assert!(
            !output_path.exists(),
            "first-write rollback should remove the partially written config"
        );
    }
}
