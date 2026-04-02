use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

mod fullscreen;
#[path = "onboard_protocols.rs"]
mod onboard_protocols;
#[path = "onboard_workspace.rs"]
mod onboard_workspace;
pub mod presentation;
#[allow(dead_code)] // Contains functions accessed by integration tests via pub re-exports
mod screens;
pub(crate) use fullscreen::build_first_run_fullscreen_boot_flow;
pub use screens::*;

use loongclaw_app as mvp;
use loongclaw_contracts::SecretRef;
use loongclaw_spec::CliResult;

use crate::onboard_finalize::{
    ConfigWritePlan, build_onboarding_success_summary_with_outcome, prepare_output_path_for_write,
    render_onboarding_success_summary_lines, resolve_backup_path, rollback_onboard_write_failure,
};
use crate::onboard_flow::{OnboardFlowController, run_guided_onboard_flow};
pub use crate::onboard_preflight::{
    OnboardCheck, OnboardCheckLevel, OnboardNonInteractiveWarningPolicy,
    collect_channel_preflight_checks, directory_preflight_check, provider_credential_check,
    render_current_setup_preflight_summary_screen_lines,
    render_detected_setup_preflight_summary_screen_lines, render_preflight_summary_screen_lines,
};
use crate::onboard_preflight::{
    config_validation_failure_message,
    is_explicitly_accepted_non_interactive_warning as preflight_accepts_non_interactive_warning,
    non_interactive_preflight_failure_message, onboard_check_outcome,
    post_write_verification_failure_check, render_preflight_summary_screen_lines_with_progress,
    run_preflight_checks,
};
#[cfg(test)]
use crate::onboard_state::OnboardInteractionMode;
use crate::onboard_state::{OnboardDraft, OnboardOutcome, OnboardValueOrigin, OnboardWizardStep};
use crate::onboard_tui::LaunchDeckResult;
pub use crate::onboard_types::OnboardingCredentialSummary;
#[cfg(test)]
use crate::onboard_web_search::{
    current_web_search_provider, resolve_effective_web_search_default_provider,
};
use crate::onboard_web_search::{
    summarize_web_search_provider_credential, web_search_provider_display_name,
};
use crate::onboarding_model_policy;
use crate::provider_credential_policy;
use mvp::tui_surface::{
    TuiCalloutTone, TuiChoiceSpec, TuiHeaderStyle, TuiScreenSpec, TuiSectionSpec,
    render_onboard_screen_spec,
};

pub use crate::onboard_finalize::{
    OnboardingAction, OnboardingActionKind, OnboardingDomainOutcome, OnboardingSuccessSummary,
    backup_existing_config, build_onboarding_success_summary,
    render_onboarding_success_summary_with_width,
};
const ONBOARD_CLEAR_INPUT_TOKEN: &str = ":clear";
const ONBOARD_CUSTOM_MODEL_OPTION_SLUG: &str = "__custom_model__";
const ONBOARD_ESCAPE_CANCEL_HINT: &str = "- press Esc then Enter to cancel onboarding";
const ONBOARD_SINGLE_LINE_INPUT_HINT: &str = "- single-line input only";
const ONBOARD_BACK_NAVIGATION_SIGNAL: &str = "__loongclaw_onboard_back_navigation__";
const OPENAI_CODEX_OAUTH_PROVIDER_SELECTOR: &str = "openai-codex-oauth";

#[derive(Debug, Clone)]
pub struct OnboardCommandOptions {
    pub output: Option<String>,
    pub force: bool,
    pub non_interactive: bool,
    pub accept_risk: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub web_search_provider: Option<String>,
    pub web_search_api_key_env: Option<String>,
    pub personality: Option<String>,
    pub memory_profile: Option<String>,
    pub system_prompt: Option<String>,
    pub skip_model_probe: bool,
}

#[derive(Debug, Clone)]
pub struct SelectOption {
    pub label: String,
    pub slug: String,
    pub description: String,
    pub recommended: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectInteractionMode {
    List,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectAction {
    Selected(usize),
    Back,
}

#[derive(Debug, Clone)]
pub struct OnboardRuntimeContext {
    render_width: usize,
    workspace_root: Option<PathBuf>,
    codex_config_paths: Vec<PathBuf>,
    #[allow(dead_code)]
    attended_terminal: bool,
    #[allow(dead_code)]
    rich_prompt_ui_supported: bool,
}

impl OnboardRuntimeContext {
    fn capture() -> Self {
        Self {
            render_width: detect_render_width(),
            workspace_root: env::current_dir().ok(),
            codex_config_paths: default_codex_config_paths(),
            attended_terminal: atty_stdout(),
            rich_prompt_ui_supported: false,
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
            attended_terminal: true,
            rich_prompt_ui_supported: true,
        }
    }
}

#[cfg(test)]
fn resolve_onboard_interaction_mode_for_test(
    non_interactive: bool,
    attended_terminal: bool,
    rich_prompt_ui_supported: bool,
) -> OnboardInteractionMode {
    if non_interactive {
        return OnboardInteractionMode::NonInteractive;
    }
    if attended_terminal && rich_prompt_ui_supported {
        return OnboardInteractionMode::RichInteractive;
    }
    OnboardInteractionMode::PlainInteractive
}

fn is_explicitly_accepted_non_interactive_warning(
    check: &OnboardCheck,
    options: &OnboardCommandOptions,
) -> bool {
    preflight_accepts_non_interactive_warning(check, options.skip_model_probe)
}

#[cfg(test)]
fn provider_model_probe_failure_check(
    config: &mvp::config::LoongClawConfig,
    error: String,
) -> OnboardCheck {
    crate::onboard_preflight::provider_model_probe_failure_check(config, error)
}

const MEMORY_PROFILE_CHOICES: [(mvp::config::MemoryProfile, &str, &str); 3] = [
    (
        mvp::config::MemoryProfile::WindowOnly,
        "recent turns only",
        "only load the recent conversation turns",
    ),
    (
        mvp::config::MemoryProfile::WindowPlusSummary,
        "window plus summary",
        "load recent turns plus a short summary of earlier context",
    ),
    (
        mvp::config::MemoryProfile::ProfilePlusWindow,
        "profile plus window",
        "load recent turns plus durable profile notes",
    ),
];

// ---------------------------------------------------------------------------
// Simple stdin/stdout prompt helpers for pre/post-flow prompts.
// ---------------------------------------------------------------------------

fn read_stdin_line() -> CliResult<String> {
    let mut line = String::new();
    let bytes_read = io::stdin()
        .read_line(&mut line)
        .map_err(|e| format!("read stdin failed: {e}"))?;
    if bytes_read == 0 {
        return Err("onboarding cancelled: stdin closed".to_owned());
    }
    Ok(line)
}

#[allow(dead_code)] // retained for non-TUI fallback and future use
fn prompt_stdin_confirm(message: &str, default: bool) -> CliResult<bool> {
    let suffix = if default { "[Y/n]" } else { "[y/N]" };
    print!("{message} {suffix}: ");
    io::stdout()
        .flush()
        .map_err(|e| format!("flush stdout failed: {e}"))?;
    let line = read_stdin_line()?;
    let value = line.trim().to_ascii_lowercase();
    if value.is_empty() {
        Ok(default)
    } else {
        Ok(matches!(value.as_str(), "y" | "yes"))
    }
}

fn prompt_stdin_select(
    label: &str,
    options: &[SelectOption],
    default: Option<usize>,
) -> CliResult<SelectAction> {
    let default = validate_select_one_state(options.len(), default)?;
    loop {
        for (i, opt) in options.iter().enumerate() {
            let num = i + 1;
            let rec = if opt.recommended {
                " (recommended)"
            } else {
                ""
            };
            println!("  {num}) {}{rec}", opt.label);
            if !opt.description.is_empty() {
                println!("     {}", opt.description);
            }
        }
        println!();
        let prompt_text = match default {
            Some(idx) => format!("{label} (default {}):", idx + 1),
            None => format!("{label}: "),
        };
        print!("{prompt_text}");
        io::stdout()
            .flush()
            .map_err(|e| format!("flush stdout failed: {e}"))?;
        let line = read_stdin_line()?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if let Some(idx) = default {
                return Ok(SelectAction::Selected(idx));
            }
            println!("Please select an option.");
            continue;
        }
        if let Some(index) = parse_select_one_input(trimmed, options) {
            return Ok(SelectAction::Selected(index));
        }
        if trimmed.eq_ignore_ascii_case("back") {
            return Ok(SelectAction::Back);
        }
        println!("{}", render_select_one_invalid_input_message(options));
    }
}

/// Check if stdout is a tty.
fn atty_stdout() -> bool {
    crossterm::tty::IsTty::is_tty(&io::stdout())
}

fn summarize_select_option_description(detail_lines: &[String]) -> String {
    detail_lines
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ")
}

fn select_options_from_screen_options(options: &[OnboardScreenOption]) -> Vec<SelectOption> {
    options
        .iter()
        .map(|option| SelectOption {
            label: option.label.clone(),
            slug: option.key.clone(),
            description: summarize_select_option_description(&option.detail_lines),
            recommended: option.recommended,
        })
        .collect()
}

fn tui_choices_from_screen_options(options: &[OnboardScreenOption]) -> Vec<TuiChoiceSpec> {
    options
        .iter()
        .map(|option| TuiChoiceSpec {
            key: option.key.clone(),
            label: option.label.clone(),
            detail_lines: option.detail_lines.clone(),
            recommended: option.recommended,
        })
        .collect()
}

fn select_screen_option(
    label: &str,
    options: &[OnboardScreenOption],
    default_key: Option<&str>,
) -> CliResult<usize> {
    let select_options = select_options_from_screen_options(options);
    let default_idx =
        default_key.and_then(|key| options.iter().position(|option| option.key == key));
    select_one_selected_index(
        label,
        &select_options,
        default_idx,
        SelectInteractionMode::List,
    )
}

fn select_one_selected_index(
    label: &str,
    options: &[SelectOption],
    default: Option<usize>,
    _interaction_mode: SelectInteractionMode,
) -> CliResult<usize> {
    match prompt_stdin_select(label, options, default)? {
        SelectAction::Selected(index) => Ok(index),
        SelectAction::Back => Err(ONBOARD_BACK_NAVIGATION_SIGNAL.to_owned()),
    }
}

fn build_onboard_entry_screen_options(options: &[OnboardEntryOption]) -> Vec<OnboardScreenOption> {
    options
        .iter()
        .enumerate()
        .map(|(index, option)| OnboardScreenOption {
            key: (index + 1).to_string(),
            label: option.label.to_owned(),
            detail_lines: vec![option.detail.clone()],
            recommended: option.recommended,
        })
        .collect()
}

fn build_starting_point_selection_screen_options(
    sorted_candidates: &[ImportCandidate],
    width: usize,
) -> Vec<OnboardScreenOption> {
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
        label: presentation::start_fresh_option_label().to_owned(),
        detail_lines: start_fresh_starting_point_detail_lines(),
        recommended: false,
    });
    options
}

fn build_onboard_shortcut_screen_options(
    shortcut_kind: OnboardShortcutKind,
) -> Vec<OnboardScreenOption> {
    vec![
        OnboardScreenOption {
            key: "1".to_owned(),
            label: shortcut_kind.primary_label().to_owned(),
            detail_lines: vec![presentation::shortcut_continue_detail().to_owned()],
            recommended: true,
        },
        OnboardScreenOption {
            key: "2".to_owned(),
            label: presentation::adjust_settings_label().to_owned(),
            detail_lines: vec![presentation::shortcut_adjust_detail().to_owned()],
            recommended: false,
        },
    ]
}

fn build_existing_config_write_screen_options() -> Vec<OnboardScreenOption> {
    vec![
        OnboardScreenOption {
            key: "o".to_owned(),
            label: "Replace existing config".to_owned(),
            detail_lines: vec!["overwrite the current file with this onboarding draft".to_owned()],
            recommended: false,
        },
        OnboardScreenOption {
            key: "b".to_owned(),
            label: "Create backup and replace".to_owned(),
            detail_lines: vec![
                "save a timestamped .bak copy first, then write the new config".to_owned(),
            ],
            recommended: true,
        },
        OnboardScreenOption {
            key: "c".to_owned(),
            label: "Cancel".to_owned(),
            detail_lines: vec!["leave the existing config untouched".to_owned()],
            recommended: false,
        },
    ]
}

fn validate_select_one_state(
    options_len: usize,
    default: Option<usize>,
) -> CliResult<Option<usize>> {
    if options_len == 0 {
        return Err("no selection options available".to_owned());
    }
    if let Some(idx) = default
        && idx >= options_len
    {
        return Err(format!(
            "default selection index {idx} out of range 0..{}",
            options_len - 1
        ));
    }
    Ok(default)
}

fn select_option_input_slug(option: &SelectOption) -> &str {
    if option.slug == ONBOARD_CUSTOM_MODEL_OPTION_SLUG {
        "custom"
    } else {
        option.slug.as_str()
    }
}

fn parse_select_one_input(trimmed: &str, options: &[SelectOption]) -> Option<usize> {
    if let Ok(selected) = trimmed.parse::<usize>()
        && (1..=options.len()).contains(&selected)
    {
        return Some(selected - 1);
    }
    options.iter().position(|option| {
        option.slug.eq_ignore_ascii_case(trimmed)
            || select_option_input_slug(option).eq_ignore_ascii_case(trimmed)
    })
}

fn render_select_one_invalid_input_message(options: &[SelectOption]) -> String {
    format!(
        "invalid selection. enter a number between 1 and {}, or one of: {}",
        options.len(),
        options
            .iter()
            .map(select_option_input_slug)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

#[cfg(test)]
fn resolve_select_one_eof(default: Option<usize>) -> CliResult<usize> {
    default.ok_or_else(|| {
        "onboarding cancelled: stdin closed while waiting for required selection".to_owned()
    })
}

fn print_stdout_lines(lines: impl IntoIterator<Item = String>) -> CliResult<()> {
    for line in lines {
        println!("{line}");
    }
    Ok(())
}

fn print_stdout_message(line: impl Into<String>) -> CliResult<()> {
    println!("{}", line.into());
    Ok(())
}

fn is_openai_codex_oauth_provider_selector(raw: &str) -> bool {
    raw.trim()
        .eq_ignore_ascii_case(OPENAI_CODEX_OAUTH_PROVIDER_SELECTOR)
}

fn apply_onboard_provider_selector_defaults(
    provider: &mut mvp::config::ProviderConfig,
    selector: &str,
) -> bool {
    if !is_openai_codex_oauth_provider_selector(selector) {
        return false;
    }

    let profile = mvp::config::ProviderKind::Openai.profile();
    provider.kind = mvp::config::ProviderKind::Openai;
    provider.base_url = profile.base_url.to_owned();
    provider.chat_completions_path = profile.chat_completions_path.to_owned();
    provider.api_key = None;
    provider.clear_api_key_env_binding();
    provider.oauth_access_token = None;

    let oauth_env_name = mvp::config::ProviderKind::Openai
        .default_oauth_access_token_env()
        .unwrap_or("OPENAI_CODEX_OAUTH_TOKEN");
    provider.set_oauth_access_token_env_binding(Some(oauth_env_name.to_owned()));
    true
}

fn launch_chat_config_path(
    summary: &OnboardingSuccessSummary,
    launch_result: Option<LaunchDeckResult>,
) -> Option<String> {
    let launch_result = launch_result?;
    if !launch_result.open_chat {
        return None;
    }
    if summary.outcome == OnboardOutcome::Blocked {
        return None;
    }

    let config_path = summary.config_path.trim();
    if config_path.is_empty() {
        return None;
    }

    Some(config_path.to_owned())
}

fn is_explicit_onboard_clear_input(raw: &str) -> bool {
    raw.trim().eq_ignore_ascii_case(ONBOARD_CLEAR_INPUT_TOKEN)
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
            GuidedPromptPath::NativePromptPack => 8,
            GuidedPromptPath::InlineOverride => 7,
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
            (_, GuidedOnboardStep::WebSearchProvider) => match self {
                GuidedPromptPath::NativePromptPack => 7,
                GuidedPromptPath::InlineOverride => 6,
            },
            (GuidedPromptPath::InlineOverride, GuidedOnboardStep::PromptCustomization) => 4,
            (GuidedPromptPath::InlineOverride, GuidedOnboardStep::MemoryProfile) => 5,
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
            GuidedOnboardStep::WebSearchProvider => "web search",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // WebSearchProvider variant used only in test-gated render functions
enum GuidedOnboardStep {
    Provider,
    Model,
    CredentialEnv,
    Personality,
    PromptCustomization,
    MemoryProfile,
    WebSearchProvider,
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
    const fn review_kind(self) -> presentation::ReviewFlowKind {
        match self {
            ReviewFlowStyle::Guided(_) => presentation::ReviewFlowKind::Guided,
            ReviewFlowStyle::QuickCurrentSetup => presentation::ReviewFlowKind::QuickCurrentSetup,
            ReviewFlowStyle::QuickDetectedSetup => presentation::ReviewFlowKind::QuickDetectedSetup,
        }
    }

    fn progress_line(self) -> String {
        match self {
            ReviewFlowStyle::Guided(_) => {
                guided_step_progress_line(OnboardWizardStep::ReviewAndWrite)
            }
            ReviewFlowStyle::QuickCurrentSetup | ReviewFlowStyle::QuickDetectedSetup => {
                presentation::review_flow_copy(self.review_kind())
                    .progress_line
                    .to_owned()
            }
        }
    }

    const fn header_subtitle(self) -> &'static str {
        presentation::review_flow_copy(self.review_kind()).header_subtitle
    }
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
enum SystemPromptSelection {
    KeepCurrent,
    RestoreBuiltIn,
    Set(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OnboardScreenOption {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) detail_lines: Vec<String>,
    pub(crate) recommended: bool,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
enum WebSearchCredentialSelection {
    KeepCurrent,
    ClearConfigured,
    UseEnv(String),
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnboardShortcutKind {
    CurrentSetup,
    DetectedSetup,
}

impl OnboardShortcutKind {
    const fn presentation_kind(self) -> presentation::ShortcutKind {
        match self {
            OnboardShortcutKind::CurrentSetup => presentation::ShortcutKind::CurrentSetup,
            OnboardShortcutKind::DetectedSetup => presentation::ShortcutKind::DetectedSetup,
        }
    }

    const fn review_flow_style(self) -> ReviewFlowStyle {
        match self {
            OnboardShortcutKind::CurrentSetup => ReviewFlowStyle::QuickCurrentSetup,
            OnboardShortcutKind::DetectedSetup => ReviewFlowStyle::QuickDetectedSetup,
        }
    }

    const fn subtitle(self) -> &'static str {
        presentation::shortcut_copy(self.presentation_kind()).subtitle
    }

    const fn title(self) -> &'static str {
        presentation::shortcut_copy(self.presentation_kind()).title
    }

    const fn summary_line(self) -> &'static str {
        presentation::shortcut_copy(self.presentation_kind()).summary_line
    }

    const fn primary_label(self) -> &'static str {
        presentation::shortcut_copy(self.presentation_kind()).primary_label
    }

    const fn default_choice_description(self) -> &'static str {
        presentation::shortcut_copy(self.presentation_kind()).default_choice_description
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnboardShortcutChoice {
    UseShortcut,
    AdjustSettings,
}
pub type ChannelImportReadiness = crate::migration::ChannelImportReadiness;

pub async fn run_onboard_cli(options: OnboardCommandOptions) -> CliResult<()> {
    let context = OnboardRuntimeContext::capture();
    run_onboard_cli_inner(options, &context).await
}

/// Test-only entrypoint that accepts an explicit runtime context, bypassing
/// terminal detection and real filesystem defaults.
#[doc(hidden)]
pub async fn run_onboard_cli_with_context(
    options: OnboardCommandOptions,
    context: &OnboardRuntimeContext,
) -> CliResult<()> {
    run_onboard_cli_inner(options, context).await
}

/// Apply CLI flag overrides to the draft config for non-interactive mode.
///
/// In non-interactive mode the TUI guided flow does not run, so CLI flags
/// (`--provider`, `--model`, `--api-key-env`, `--personality`,
/// `--memory-profile`, `--system-prompt`, `--web-search-provider`,
/// `--web-search-api-key-env`) must be applied directly to the draft.
fn apply_non_interactive_overrides(
    options: &OnboardCommandOptions,
    draft: &mut OnboardDraft,
    context: &OnboardRuntimeContext,
) -> CliResult<()> {
    // --- provider ---
    if let Some(provider_raw) = options.provider.as_deref() {
        let mut provider = draft.config.provider.clone();
        let selector_applied =
            apply_onboard_provider_selector_defaults(&mut provider, provider_raw);
        if !selector_applied {
            let kind = parse_provider_kind(provider_raw).ok_or_else(|| {
                format!(
                    "unsupported provider value \"{provider_raw}\". accepted: {}",
                    supported_provider_selector_list()
                )
            })?;
            let profile = kind.profile();
            provider.kind = kind;
            provider.base_url = profile.base_url.to_owned();
            provider.chat_completions_path = profile.chat_completions_path.to_owned();
        }
        draft.set_provider_config(provider);
    }

    // --- model ---
    let resolved_model = onboarding_model_policy::resolve_onboarding_model_prompt_default(
        &draft.config.provider,
        options.model.as_deref(),
    )?;
    draft.set_provider_model(resolved_model);

    // --- credential env ---
    if let Some(api_key_env) = options.api_key_env.as_deref() {
        if is_explicit_onboard_clear_input(api_key_env) {
            // :clear removes env bindings but preserves inline credentials
            draft.set_provider_credential_env(String::new());
        } else {
            let trimmed = api_key_env.trim();
            if !trimmed.is_empty() {
                draft.set_provider_credential_env(trimmed.to_owned());
            }
        }
    } else {
        // No explicit --api-key-env flag: apply default credential routing.
        // `preferred_api_key_env_default` respects already-configured env
        // bindings and inline credentials — it returns an empty string when
        // an inline literal is present, avoiding accidental clearing.
        let default_env = preferred_api_key_env_default(&draft.config);
        draft.set_provider_credential_env(default_env);
    }

    // --- personality ---
    if let Some(personality_raw) = options.personality.as_deref() {
        let personality = parse_prompt_personality(personality_raw).ok_or_else(|| {
            format!(
                "unsupported --personality value \"{personality_raw}\". supported: {}",
                supported_personality_list()
            )
        })?;
        draft.use_native_prompt_pack(personality, draft.config.cli.system_prompt_addendum.clone());
    }

    // --- memory profile ---
    if let Some(profile_raw) = options.memory_profile.as_deref() {
        let profile = parse_memory_profile(profile_raw).ok_or_else(|| {
            format!(
                "unsupported --memory-profile value \"{profile_raw}\". supported: {}",
                supported_memory_profile_list()
            )
        })?;
        draft.set_memory_profile(profile);
    }

    // --- system prompt ---
    if let Some(system_prompt) = options.system_prompt.as_deref() {
        if is_explicit_onboard_clear_input(system_prompt) {
            draft.restore_built_in_prompt();
        } else {
            let trimmed = system_prompt.trim();
            if !trimmed.is_empty() {
                draft.set_inline_system_prompt(trimmed.to_owned());
            }
        }
    }

    // --- web search provider ---
    if let Some(web_search_provider) = options.web_search_provider.as_deref() {
        let trimmed = web_search_provider.trim();
        if !trimmed.is_empty() {
            draft.set_web_search_default_provider(trimmed.to_owned());
        }
    }

    // --- web search credential env ---
    if let Some(env_name) = options.web_search_api_key_env.as_deref() {
        let provider = draft.config.tools.web_search.default_provider.clone();
        if is_explicit_onboard_clear_input(env_name) {
            draft.clear_web_search_credential(&provider);
        } else {
            let trimmed = env_name.trim();
            if !trimmed.is_empty() {
                draft.set_web_search_credential_env(&provider, trimmed.to_owned());
            }
        }
    }

    // --- workspace defaults ---
    // Derive workspace step values (sqlite path, file_root) from the runtime
    // context and apply them to the draft, matching the guided-flow workspace step.
    let workspace_values = onboard_workspace::derive_workspace_step_values(draft, context);
    onboard_workspace::apply_workspace_step_values(draft, &workspace_values);

    Ok(())
}

async fn run_onboard_cli_inner(
    options: OnboardCommandOptions,
    context: &OnboardRuntimeContext,
) -> CliResult<()> {
    validate_non_interactive_risk_gate(options.non_interactive, options.accept_risk)?;

    // Create the TUI runner at the start for the entire interactive session.
    let mut tui_runner = if !options.non_interactive {
        match crate::onboard_tui::RatatuiOnboardRunner::new() {
            Ok(runner) => Some(runner),
            Err(e) => {
                eprintln!("warning: TUI unavailable ({e}), falling back to basic prompts");
                None
            }
        }
    } else {
        None
    };

    let output_path = options
        .output
        .as_deref()
        .map(mvp::config::expand_path)
        .unwrap_or_else(mvp::config::default_config_path);

    if let Some(runner) = &mut tui_runner {
        runner.run_opening_screen()?;
    }

    // --- Pre-flow: entry choice + import selection ---
    let starting_selection =
        load_import_starting_config_with_tui(&output_path, &options, context, &mut tui_runner)?;

    let mut flow = OnboardFlowController::new(OnboardDraft::from_config(
        starting_selection.config.clone(),
        output_path.clone(),
        initial_draft_origin(starting_selection.entry_choice),
    ));
    if let Some(runner) = &mut tui_runner {
        runner.set_provider_selection_plan(starting_selection.provider_selection.clone());
    }

    // --- Pre-flow: shortcut choice ---
    let shortcut_kind = resolve_onboard_shortcut_kind(&options, &starting_selection);
    let skip_detailed_setup = if let Some(shortcut_kind) = shortcut_kind {
        if let Some(runner) = &mut tui_runner {
            let snapshot_lines = build_onboard_review_digest_display_lines(&flow.draft().config);
            runner.run_shortcut_choice_screen(shortcut_kind.primary_label(), &snapshot_lines)?
        } else {
            // non-interactive fallback (should not happen, but defensive)
            false
        }
    } else {
        false
    };

    let review_flow_style = if skip_detailed_setup {
        shortcut_kind
            .map(OnboardShortcutKind::review_flow_style)
            .unwrap_or(ReviewFlowStyle::Guided(GuidedPromptPath::NativePromptPack))
    } else {
        ReviewFlowStyle::Guided(resolve_guided_prompt_path(&options, &flow.draft().config))
    };

    // --- Guided wizard flow ---
    if !skip_detailed_setup && !options.non_interactive {
        if let Some(runner) = &mut tui_runner {
            runner.skip_next_guided_welcome_once();
            flow = run_guided_onboard_flow(flow, runner).await?;
        }
    } else if !skip_detailed_setup && options.non_interactive {
        apply_non_interactive_overrides(&options, flow.draft_mut(), context)?;
    }
    let show_guided_environment_step = !options.non_interactive && !skip_detailed_setup;

    let workspace_guidance = context
        .workspace_root
        .as_deref()
        .map(crate::migration::detect_workspace_guidance)
        .unwrap_or_default();
    let review_candidate = build_onboard_review_candidate_with_selected_context(
        &flow.draft().config,
        &workspace_guidance,
        starting_selection.review_candidate.as_ref(),
    );

    let checks = run_preflight_checks(&flow.draft().config, options.skip_model_probe).await;
    let config_validation_failure = config_validation_failure_message(&checks);
    let final_outcome = onboard_check_outcome(&checks, None);

    let credential_ok = checks
        .iter()
        .find(|check| check.name == "provider credentials")
        .is_some_and(|check| check.level == OnboardCheckLevel::Pass);
    let has_failures = checks
        .iter()
        .any(|check| check.level == OnboardCheckLevel::Fail);
    let existing_output_config = load_existing_output_config(&output_path);
    let skip_config_write =
        should_skip_config_write(existing_output_config.as_ref(), &flow.draft().config);
    let has_blocking_non_interactive_warnings = !skip_config_write
        && checks.iter().any(|check| {
            check.level == OnboardCheckLevel::Warn
                && !is_explicitly_accepted_non_interactive_warning(check, &options)
        });

    if options.non_interactive {
        if let Some(message) = config_validation_failure {
            return Err(message);
        }
        if !credential_ok {
            let credential_hint = provider_credential_policy::provider_credential_env_hint(
                &flow.draft().config.provider,
            )
            .unwrap_or_else(|| "PROVIDER_API_KEY".to_owned());
            return Err(format!(
                "onboard preflight failed: provider credentials missing. configure inline credentials or set {} in env",
                credential_hint
            ));
        }
        if has_failures {
            return Err(non_interactive_preflight_failure_message(&checks));
        }
        if has_blocking_non_interactive_warnings {
            let warning_message = non_interactive_preflight_warning_message(&checks, &options);
            return Err(warning_message);
        }
    } else if tui_runner.is_some() {
        // --- Post-flow: hard-stop verification failures (TUI) ---
        // For hard failures, drop the TUI runner first so raw mode is
        // restored before the error string is printed to stderr.
        if let Some(message) = config_validation_failure {
            drop(tui_runner);
            return Err(message);
        }
        if has_failures {
            let message = non_interactive_preflight_failure_message(&checks);
            drop(tui_runner);
            return Err(message);
        }
    }

    // --- Post-flow: verify + write screen ---
    if let Some(runner) = &mut tui_runner
        && !skip_config_write
    {
        if show_guided_environment_step
            && flow.current_step() == OnboardWizardStep::EnvironmentCheck
        {
            flow.advance();
        }
        let review_lines = render_onboard_review_lines_for_draft_with_guidance_and_style(
            flow.draft(),
            starting_selection.import_source.as_deref(),
            &workspace_guidance,
            starting_selection.review_candidate.as_ref(),
            context.render_width,
            review_flow_style,
            false,
        );
        if !runner.run_verify_and_write_screen(
            &checks,
            &review_lines,
            &output_path.display().to_string(),
        )? {
            return Err("onboarding cancelled: review declined before write".to_owned());
        }
    }

    let mut deferred_backup_message: Option<String> = None;
    let (path, config_status, write_recovery) = if skip_config_write {
        (
            output_path.clone(),
            Some("existing config kept; no changes were needed".to_owned()),
            None,
        )
    } else {
        let write_plan = resolve_write_plan(&output_path, &options, context)?;
        let write_recovery = prepare_output_path_for_write(&output_path, &write_plan)?;
        let backup_path = if write_recovery.keep_backup_on_success {
            write_recovery.backup_path.as_deref()
        } else {
            None
        };
        if let Some(backup_path) = backup_path {
            deferred_backup_message = Some(format!(
                "Backed up existing config to: {}",
                backup_path.display()
            ));
        }
        let path = match mvp::config::write(
            options.output.as_deref(),
            &flow.draft().config,
            write_plan.force,
        ) {
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
        let mem_config = mvp::memory::runtime_config::MemoryRuntimeConfig::from_memory_config(
            &flow.draft().config.memory,
        );
        match mvp::memory::ensure_memory_db_ready(
            Some(flow.draft().config.memory.resolved_sqlite_path()),
            &mem_config,
        ) {
            Ok(path) => path,
            Err(error) => {
                let verification_detail = format!("failed to bootstrap sqlite memory: {error}");
                if let Some(write_recovery) = write_recovery.as_ref() {
                    let rollback_result = write_recovery.rollback(&output_path);
                    let restored_summary_candidate =
                        existing_output_config.as_ref().map(|config| {
                            (
                                config,
                                build_onboard_review_candidate_with_selected_context(
                                    config,
                                    &workspace_guidance,
                                    starting_selection.review_candidate.as_ref(),
                                ),
                            )
                        });
                    let (summary_config, summary_review_candidate, config_status) =
                        match &rollback_result {
                            Ok(()) => match restored_summary_candidate.as_ref() {
                                Some((config, review_candidate)) => (
                                    *config,
                                    Some(review_candidate),
                                    Some(
                                        "previous config restored after verification failed"
                                            .to_owned(),
                                    ),
                                ),
                                None => (
                                    &flow.draft().config,
                                    Some(&review_candidate),
                                    Some(
                                        "verification failed after write; rollback removed the \
                                         partial config and no prior config was available"
                                            .to_owned(),
                                    ),
                                ),
                            },
                            Err(rollback_error) => (
                                &flow.draft().config,
                                Some(&review_candidate),
                                Some(format!(
                                    "verification failed and rollback also failed: \
                                     {rollback_error}"
                                )),
                            ),
                        };
                    let failure = match &rollback_result {
                        Ok(()) => verification_detail.clone(),
                        Err(rollback_error) => format!(
                            "{verification_detail}; additionally failed to restore original config: {rollback_error}"
                        ),
                    };
                    let verification_check =
                        post_write_verification_failure_check(verification_detail.as_str());
                    let blocked_outcome = onboard_check_outcome(&checks, Some(&verification_check));
                    let blocked_summary = build_onboarding_success_summary_with_outcome(
                        &output_path,
                        summary_config,
                        starting_selection.import_source.as_deref(),
                        summary_review_candidate,
                        None,
                        config_status.as_deref(),
                        blocked_outcome,
                        Some(verification_check.detail.as_str()),
                    );
                    let mut blocked_launch_result: Option<LaunchDeckResult> = None;
                    if let Some(runner) = &mut tui_runner {
                        blocked_launch_result = runner.run_launch_screen(&blocked_summary).ok();
                    } else {
                        print_guided_step_boundary(OnboardWizardStep::Ready)?;
                        let styled_blocked_lines = render_onboarding_success_summary_lines(
                            &blocked_summary,
                            context.render_width,
                            true,
                        );
                        print_stdout_lines(styled_blocked_lines)?;
                    }
                    drop(tui_runner);
                    if let Some(msg) = deferred_backup_message.take() {
                        print_stdout_message(msg)?;
                    }
                    let blocked_chat_config_path =
                        launch_chat_config_path(&blocked_summary, blocked_launch_result);
                    if let Some(config_path) = blocked_chat_config_path.as_deref() {
                        crate::run_chat_cli(
                            Some(config_path),
                            None,
                            crate::CliChatUiModeArg::Tui,
                            false,
                            false,
                            &[],
                            None,
                        )
                        .await?;
                    }
                    return Err(failure);
                }
                return Err(verification_detail);
            }
        }
    };

    let memory_path_display = Some(memory_path.display().to_string());
    #[cfg(not(feature = "memory-sqlite"))]
    let memory_path_display: Option<String> = None;

    if let Some(write_recovery) = write_recovery.as_ref() {
        write_recovery.finish_success();
    }
    let success_summary = build_onboarding_success_summary_with_outcome(
        &path,
        &flow.draft().config,
        starting_selection.import_source.as_deref(),
        Some(&review_candidate),
        memory_path_display.as_deref(),
        config_status.as_deref(),
        final_outcome,
        Some(match final_outcome {
            OnboardOutcome::Success => "passed",
            OnboardOutcome::SuccessWithWarnings => "passed with warnings kept by choice",
            OnboardOutcome::Blocked => "blocked after verification",
        }),
    );
    let mut launch_result: Option<LaunchDeckResult> = None;
    if let Some(runner) = &mut tui_runner {
        let result = runner.run_launch_screen(&success_summary)?;
        launch_result = Some(result);
    } else {
        print_guided_step_boundary(OnboardWizardStep::Ready)?;
        let styled_lines =
            render_onboarding_success_summary_lines(&success_summary, context.render_width, true);
        print_stdout_lines(styled_lines)?;
    }

    // Drop the TUI runner to restore the terminal before returning.
    drop(tui_runner);

    // Print deferred messages now that raw mode is restored.
    if let Some(msg) = deferred_backup_message {
        print_stdout_message(msg)?;
    }
    let chat_config_path = launch_chat_config_path(&success_summary, launch_result);
    if let Some(config_path) = chat_config_path.as_deref() {
        crate::tui_cli::run_tui_cli_with_system_message(
            Some(config_path),
            None,
            Some("Setup complete. Entering chat.".to_owned()),
        )
        .await?;
    }

    Ok(())
}

fn initial_draft_origin(entry_choice: OnboardEntryChoice) -> Option<OnboardValueOrigin> {
    match entry_choice {
        OnboardEntryChoice::ContinueCurrentSetup => Some(OnboardValueOrigin::CurrentSetup),
        OnboardEntryChoice::ImportDetectedSetup => Some(OnboardValueOrigin::DetectedStartingPoint),
        OnboardEntryChoice::StartFresh => None,
    }
}

fn print_guided_step_boundary(step: OnboardWizardStep) -> CliResult<()> {
    println!("{}", guided_step_progress_line(step));
    Ok(())
}

fn guided_step_progress_line(step: OnboardWizardStep) -> String {
    let (index, label) = match step {
        OnboardWizardStep::Welcome => (1, "welcome"),
        OnboardWizardStep::Authentication => (2, "authentication"),
        OnboardWizardStep::RuntimeDefaults => (3, "runtime defaults"),
        OnboardWizardStep::Workspace => (4, "workspace"),
        OnboardWizardStep::Protocols => (5, "protocols"),
        OnboardWizardStep::EnvironmentCheck => (6, "environment check"),
        OnboardWizardStep::ReviewAndWrite => (7, "review and write"),
        OnboardWizardStep::Ready => (8, "ready"),
    };
    format!("step {index} of 8 · {label}")
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
            "- {} [{}] selection_order={} selection_label=\"{}\" strategy={} aliases={} status_command=\"{}\" repair_command={} setup_hint=\"{}\" blurb=\"{}\"",
            surface.catalog.label,
            surface.catalog.id,
            surface.catalog.selection_order,
            surface.catalog.selection_label,
            surface.catalog.onboarding.strategy.as_str(),
            aliases,
            surface.catalog.onboarding.status_command,
            repair_command,
            surface.catalog.onboarding.setup_hint,
            surface.catalog.blurb,
        ));
    }

    lines
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

    if is_openai_codex_oauth_provider_selector(selector) {
        let mut provider = crate::migration::resolve_provider_config_from_selection(
            current_provider,
            provider_selection,
            mvp::config::ProviderKind::Openai,
        );
        let _ = apply_onboard_provider_selector_defaults(&mut provider, selector);
        return Ok(provider);
    }
    let kind = parse_provider_kind(selector).ok_or_else(|| {
        if provider_selection.imported_choices.is_empty() {
            return format!(
                "unsupported provider value \"{selector}\". accepted selectors: {}. {}",
                supported_provider_selector_list(),
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

#[cfg(test)]
fn apply_selected_api_key_env(
    provider: &mut mvp::config::ProviderConfig,
    selected_api_key_env: String,
) {
    let selected_api_key_env = selected_api_key_env.trim();
    if selected_api_key_env.is_empty() {
        provider.clear_api_key_env_binding();
        provider.clear_oauth_access_token_env_binding();
        return;
    }

    provider.api_key = None;
    provider.oauth_access_token = None;
    match provider_credential_policy::selected_provider_credential_env_field(
        provider,
        selected_api_key_env,
    ) {
        provider_credential_policy::ProviderCredentialEnvField::ApiKey => {
            provider.clear_oauth_access_token_env_binding();
            provider.set_api_key_env_binding(Some(selected_api_key_env.to_owned()));
        }
        provider_credential_policy::ProviderCredentialEnvField::OAuthAccessToken => {
            provider.clear_api_key_env_binding();
            provider.set_oauth_access_token_env_binding(Some(selected_api_key_env.to_owned()));
        }
    }
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

#[cfg(test)]
fn build_web_search_provider_screen_options(
    config: &mvp::config::LoongClawConfig,
    recommended_provider: &str,
) -> Vec<OnboardScreenOption> {
    mvp::config::web_search_provider_descriptors()
        .iter()
        .map(|descriptor| {
            let mut detail_lines = vec![descriptor.description.to_owned()];
            if let Some(credential) =
                summarize_web_search_provider_credential(config, descriptor.id)
            {
                detail_lines.push(format!("{}: {}", credential.label, credential.value));
            }
            OnboardScreenOption {
                key: descriptor.id.to_owned(),
                label: descriptor.display_name.to_owned(),
                detail_lines,
                recommended: descriptor.id == recommended_provider,
            }
        })
        .collect()
}

#[cfg(test)]
fn render_web_search_provider_selection_screen_lines_with_style(
    config: &mvp::config::LoongClawConfig,
    recommended_provider: &str,
    default_provider: &str,
    recommendation_reason: &str,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let current_provider = current_web_search_provider(config);
    let current_provider_label = web_search_provider_display_name(current_provider);
    let recommended_provider_label = web_search_provider_display_name(recommended_provider);
    let default_provider_label = web_search_provider_display_name(default_provider);
    let options = build_web_search_provider_screen_options(config, recommended_provider);
    let default_footer_description = if default_provider == current_provider {
        format!("keep {current_provider_label}")
    } else {
        format!("use {default_provider_label}")
    };

    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "choose web search",
        "choose web search provider",
        Some((GuidedOnboardStep::WebSearchProvider, guided_prompt_path)),
        vec![
            format!("- current provider: {current_provider_label}"),
            format!("- recommended provider: {recommended_provider_label}"),
            format!("- why this is recommended: {recommendation_reason}"),
        ],
        options,
        vec![render_default_choice_footer_line(
            "Enter",
            default_footer_description.as_str(),
        )],
        true,
        color_enabled,
    )
}

#[cfg(test)]
fn apply_selected_web_search_credential(
    config: &mut mvp::config::LoongClawConfig,
    provider: &str,
    selection: WebSearchCredentialSelection,
) {
    let next_value = match selection {
        WebSearchCredentialSelection::KeepCurrent => return,
        WebSearchCredentialSelection::ClearConfigured => None,
        WebSearchCredentialSelection::UseEnv(env_name) => Some(format!("${{{}}}", env_name.trim())),
    };

    match provider {
        mvp::config::WEB_SEARCH_PROVIDER_BRAVE => {
            config.tools.web_search.brave_api_key = next_value;
        }
        mvp::config::WEB_SEARCH_PROVIDER_TAVILY => {
            config.tools.web_search.tavily_api_key = next_value;
        }
        mvp::config::WEB_SEARCH_PROVIDER_PERPLEXITY => {
            config.tools.web_search.perplexity_api_key = next_value;
        }
        mvp::config::WEB_SEARCH_PROVIDER_EXA => {
            config.tools.web_search.exa_api_key = next_value;
        }
        mvp::config::WEB_SEARCH_PROVIDER_JINA => {
            config.tools.web_search.jina_api_key = next_value;
        }
        _ => {}
    }
}

fn non_interactive_preflight_warning_message(
    checks: &[OnboardCheck],
    options: &OnboardCommandOptions,
) -> String {
    let blocking_warning = checks.iter().find(|check| {
        let is_warning = check.level == OnboardCheckLevel::Warn;
        let is_accepted = is_explicitly_accepted_non_interactive_warning(check, options);

        is_warning && !is_accepted
    });

    let detail = blocking_warning
        .map(|check| format!("{}: {}", check.name, check.detail))
        .unwrap_or_else(|| "unresolved warnings require interactive review".to_owned());

    format!(
        "onboard preflight failed: {detail}; rerun without --non-interactive to inspect and confirm them"
    )
}
fn render_configured_provider_credential_source_value(
    provider: &mvp::config::ProviderConfig,
) -> Option<String> {
    let configured_oauth = provider.configured_oauth_access_token_env_override();
    let rendered_oauth = provider_credential_policy::render_provider_credential_source_value(
        configured_oauth.as_deref(),
    );
    if rendered_oauth.is_some() {
        return rendered_oauth;
    }

    let configured_api_key = provider.configured_api_key_env_override();
    provider_credential_policy::render_provider_credential_source_value(
        configured_api_key.as_deref(),
    )
}

pub fn preferred_api_key_env_default(config: &mvp::config::LoongClawConfig) -> String {
    let provider = &config.provider;
    if let Some(binding) =
        provider_credential_policy::configured_provider_credential_env_binding(provider)
    {
        return binding.env_name;
    }
    if provider_credential_policy::provider_has_inline_credential(provider) {
        return String::new();
    }
    if provider.kind == mvp::config::ProviderKind::Openai {
        let api_key_env = provider.kind.default_api_key_env().unwrap_or_default();
        return api_key_env.to_owned();
    }
    provider_credential_policy::preferred_provider_credential_env_binding(provider)
        .map(|binding| binding.env_name)
        .unwrap_or_default()
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

fn load_import_starting_config(
    output_path: &Path,
    options: &OnboardCommandOptions,

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
        current_setup_state,
        current_candidate.as_ref(),
        &import_candidates,
        &entry_options,
        context,
    )?;
    match prompt_onboard_entry_choice(&entry_options)? {
        OnboardEntryChoice::ContinueCurrentSetup => Ok(current_candidate
            .map(|candidate| {
                starting_config_selection_from_current_candidate(candidate, current_setup_state)
            })
            .unwrap_or_else(default_starting_config_selection)),
        OnboardEntryChoice::ImportDetectedSetup => select_interactive_import_starting_config(
            context,
            current_setup_state,
            import_candidates,
            &all_candidates,
        ),
        OnboardEntryChoice::StartFresh => Ok(default_starting_config_selection()),
    }
}

/// TUI-aware version of `load_import_starting_config`.
///
/// When `tui_runner` is `Some`, the entry choice and import candidate selection
/// are rendered via ratatui screens; otherwise falls back to plain stdin.
fn load_import_starting_config_with_tui(
    output_path: &Path,
    options: &OnboardCommandOptions,
    context: &OnboardRuntimeContext,
    tui_runner: &mut Option<crate::onboard_tui::RatatuiOnboardRunner>,
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

    // TUI path: use the runner for the entry choice if available.
    if let Some(runner) = tui_runner {
        let tui_options: Vec<(String, String)> = entry_options
            .iter()
            .map(|opt| (opt.label.to_owned(), opt.detail.clone()))
            .collect();
        let snapshot_lines = build_starting_point_snapshot_lines(
            current_setup_state,
            current_candidate.as_ref(),
            &import_candidates,
            context.workspace_root.as_deref(),
        );
        let default_idx = entry_options
            .iter()
            .position(|opt| opt.recommended)
            .unwrap_or(0);
        let idx = runner.run_entry_choice_screen(&tui_options, default_idx, &snapshot_lines)?;
        let choice = entry_options
            .get(idx)
            .map(|opt| opt.choice)
            .ok_or_else(|| format!("entry selection index {idx} out of range"))?;
        return match choice {
            OnboardEntryChoice::ContinueCurrentSetup => Ok(current_candidate
                .map(|candidate| {
                    starting_config_selection_from_current_candidate(candidate, current_setup_state)
                })
                .unwrap_or_else(default_starting_config_selection)),
            OnboardEntryChoice::ImportDetectedSetup => {
                select_interactive_import_starting_config_with_tui(
                    runner,
                    current_setup_state,
                    import_candidates,
                    &all_candidates,
                )
            }
            OnboardEntryChoice::StartFresh => Ok(default_starting_config_selection()),
        };
    }

    // Fallback to plain stdin (should not reach here for interactive mode
    // since `tui_runner` is always `Some`, but kept for completeness).
    load_import_starting_config(output_path, options, context)
}

fn select_interactive_import_starting_config_with_tui(
    runner: &mut crate::onboard_tui::RatatuiOnboardRunner,
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
            return Ok(starting_config_selection_from_import_candidate(
                candidate.clone(),
                all_candidates,
                current_setup_state,
            ));
        }
        return Ok(default_starting_config_selection());
    }

    let tui_candidates: Vec<(String, String)> = import_candidates
        .iter()
        .map(|c| {
            let label = onboard_starting_point_label(Some(c.source_kind), &c.source);
            let detail = if c.surfaces.is_empty() {
                String::new()
            } else {
                c.surfaces
                    .iter()
                    .map(|s| s.detail.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            (label, detail)
        })
        .collect();
    let summary_lines = import_candidates
        .first()
        .map(build_detected_candidate_summary_lines)
        .unwrap_or_else(|| {
            vec!["Choose a detected source to inspect how much setup it carries.".to_owned()]
        });

    match runner.run_import_candidate_screen(&tui_candidates, 0, &summary_lines)? {
        Some(idx) => {
            if let Some(candidate) = import_candidates.get(idx) {
                Ok(starting_config_selection_from_import_candidate(
                    candidate.clone(),
                    all_candidates,
                    current_setup_state,
                ))
            } else {
                Ok(default_starting_config_selection())
            }
        }
        None => Ok(default_starting_config_selection()),
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
            label: presentation::current_setup_option_label(),
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
            label: presentation::detected_setup_option_label(),
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
        label: presentation::start_fresh_option_label(),
        detail: presentation::start_fresh_option_detail().to_owned(),
        recommended: !options.iter().any(|option| option.recommended),
    });

    options
}

fn describe_current_setup_option(
    current_setup_state: crate::migration::CurrentSetupState,
) -> String {
    presentation::current_setup_option_detail(current_setup_state).to_owned()
}

fn describe_import_option(
    has_current_setup: bool,
    recommended_plan_available: bool,
    detected_source_count: usize,
) -> String {
    presentation::import_option_detail(
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

fn build_starting_point_snapshot_lines(
    current_setup_state: crate::migration::CurrentSetupState,
    current_candidate: Option<&ImportCandidate>,
    import_candidates: &[ImportCandidate],
    workspace_root: Option<&Path>,
) -> Vec<String> {
    let recommended_plan_available = import_candidates.iter().any(|candidate| {
        candidate.source_kind == crate::migration::ImportSourceKind::RecommendedPlan
    });
    let mut lines = render_detected_settings_digest_lines(
        current_setup_state,
        current_candidate,
        import_candidates,
        workspace_root,
        recommended_plan_available,
    );
    if lines.is_empty() {
        lines.push("No reusable local setup was detected.".to_owned());
    }
    lines
}

fn build_detected_candidate_summary_lines(candidate: &ImportCandidate) -> Vec<String> {
    let mut lines = vec![format!(
        "Source: {}",
        onboard_starting_point_label(Some(candidate.source_kind), &candidate.source)
    )];
    if let Some(reason_line) =
        format_starting_point_reason(&collect_starting_point_fit_hints(candidate))
    {
        lines.push(reason_line);
    }
    let preview_candidate = migration_candidate_for_onboard_display(candidate);
    lines.extend(crate::migration::render::candidate_preview_display_lines(
        &preview_candidate,
    ));
    lines
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
    current_setup_state: crate::migration::CurrentSetupState,
    current_candidate: Option<&ImportCandidate>,
    import_candidates: &[ImportCandidate],
    options: &[OnboardEntryOption],
    context: &OnboardRuntimeContext,
) -> CliResult<()> {
    print_stdout_lines(render_onboard_entry_interactive_screen_lines_with_style(
        current_setup_state,
        current_candidate,
        import_candidates,
        options,
        context.workspace_root.as_deref(),
        context.render_width,
        true,
    ))
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
    let spec = build_onboard_entry_screen_spec(
        current_setup_state,
        current_candidate,
        import_candidates,
        options,
        workspace_root,
        false,
    );

    render_onboard_screen_spec(&spec, width, color_enabled)
}

fn render_onboard_entry_interactive_screen_lines_with_style(
    current_setup_state: crate::migration::CurrentSetupState,
    current_candidate: Option<&ImportCandidate>,
    import_candidates: &[ImportCandidate],
    options: &[OnboardEntryOption],
    workspace_root: Option<&Path>,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_onboard_entry_screen_spec(
        current_setup_state,
        current_candidate,
        import_candidates,
        options,
        workspace_root,
        true,
    );

    render_onboard_screen_spec(&spec, width, color_enabled)
}

fn build_onboard_entry_screen_spec(
    current_setup_state: crate::migration::CurrentSetupState,
    current_candidate: Option<&ImportCandidate>,
    import_candidates: &[ImportCandidate],
    options: &[OnboardEntryOption],
    workspace_root: Option<&Path>,
    interactive: bool,
) -> TuiScreenSpec {
    let recommended_plan_available = import_candidates.iter().any(|candidate| {
        candidate.source_kind == crate::migration::ImportSourceKind::RecommendedPlan
    });
    let detected_settings_lines = render_detected_settings_digest_lines(
        current_setup_state,
        current_candidate,
        import_candidates,
        workspace_root,
        recommended_plan_available,
    );
    let detected_settings_section = TuiSectionSpec::Narrative {
        title: Some(presentation::detected_settings_section_heading().to_owned()),
        lines: detected_settings_lines,
    };

    let mut sections = vec![detected_settings_section];

    if !options.is_empty() {
        let entry_choice_section = TuiSectionSpec::Narrative {
            title: Some(presentation::entry_choice_section_heading().to_owned()),
            lines: Vec::new(),
        };

        sections.push(entry_choice_section);
    }

    let choices = if interactive {
        Vec::new()
    } else {
        let screen_options = build_onboard_entry_screen_options(options);
        tui_choices_from_screen_options(&screen_options)
    };

    let footer_lines = if interactive {
        append_escape_cancel_hint(Vec::<String>::new())
    } else {
        let default_footer_lines = render_onboard_entry_default_choice_footer_line(options)
            .into_iter()
            .collect::<Vec<_>>();

        append_escape_cancel_hint(default_footer_lines)
    };

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some("guided setup for provider, channels, and workspace guidance".to_owned()),
        title: None,
        progress_line: None,
        intro_lines: Vec::new(),
        sections,
        choices,
        footer_lines,
    }
}

fn render_onboard_entry_default_choice_footer_line(
    options: &[OnboardEntryOption],
) -> Option<String> {
    let default_choice = default_onboard_entry_choice(options);
    let default_index = options
        .iter()
        .position(|option| option.choice == default_choice)
        .map(|index| index + 1)?;
    let description =
        presentation::entry_default_choice_description(onboard_entry_choice_kind(default_choice));
    Some(render_default_choice_footer_line(
        &default_index.to_string(),
        description,
    ))
}

const fn onboard_entry_choice_kind(choice: OnboardEntryChoice) -> presentation::EntryChoiceKind {
    match choice {
        OnboardEntryChoice::ContinueCurrentSetup => presentation::EntryChoiceKind::CurrentSetup,
        OnboardEntryChoice::ImportDetectedSetup => presentation::EntryChoiceKind::DetectedSetup,
        OnboardEntryChoice::StartFresh => presentation::EntryChoiceKind::StartFresh,
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
        presentation::current_setup_state_label(current_setup_state)
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
        let prefix = presentation::detected_coverage_prefix(recommended_plan_available);
        lines.push(format!("{prefix}{coverage}"));
    } else if recommended_plan_available {
        lines.push(presentation::suggested_starting_point_ready_line().to_owned());
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
fn prompt_onboard_entry_choice(options: &[OnboardEntryOption]) -> CliResult<OnboardEntryChoice> {
    let screen_options = build_onboard_entry_screen_options(options);
    let default_key = screen_options
        .iter()
        .find(|option| option.recommended)
        .map(|option| option.key.as_str())
        .or_else(|| screen_options.first().map(|option| option.key.as_str()));
    let idx = select_screen_option("Setup path", &screen_options, default_key)?;
    options
        .get(idx)
        .map(|option| option.choice)
        .ok_or_else(|| format!("entry selection index {idx} out of range"))
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
            print_import_candidate_preview(candidate, all_candidates, context)?;
            return Ok(starting_config_selection_from_import_candidate(
                candidate.clone(),
                all_candidates,
                current_setup_state,
            ));
        }
        return Ok(default_starting_config_selection());
    }

    print_import_candidates(&import_candidates, context)?;
    let Some(index) = prompt_import_candidate_choice(&import_candidates, context.render_width)?
    else {
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
    candidate: &ImportCandidate,
    all_candidates: &[ImportCandidate],
    context: &OnboardRuntimeContext,
) -> CliResult<()> {
    print_stdout_lines(
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
    let preview_candidate = migration_candidate_for_onboard_display(candidate);
    let preview_lines =
        crate::migration::render::candidate_preview_display_lines(&preview_candidate);
    intro_lines.extend(preview_lines);

    let provider_selection_lines =
        crate::migration::render::provider_selection_display_lines(&provider_selection);
    intro_lines.extend(provider_selection_lines);

    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        presentation::single_detected_starting_point_preview_subtitle(),
        presentation::single_detected_starting_point_preview_title(),
        None,
        intro_lines,
        Vec::new(),
        vec![presentation::single_detected_starting_point_preview_footer().to_owned()],
        false,
        color_enabled,
    )
}

fn print_import_candidates(
    candidates: &[ImportCandidate],
    context: &OnboardRuntimeContext,
) -> CliResult<()> {
    print_stdout_lines(render_starting_point_selection_header_lines_with_style(
        candidates,
        context.render_width,
        true,
    ))
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
    left.api_key_env_explicit = false;
    left.oauth_access_token = None;
    left.oauth_access_token_env = None;
    left.oauth_access_token_env_explicit = false;

    right.api_key = None;
    right.api_key_env = None;
    right.api_key_env_explicit = false;
    right.oauth_access_token = None;
    right.oauth_access_token_env = None;
    right.oauth_access_token_env_explicit = false;

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
    let spec = build_onboard_review_screen_spec(
        config,
        import_source,
        workspace_guidance,
        selected_candidate,
        flow_style,
    );

    render_onboard_screen_spec(&spec, width, color_enabled)
}

fn render_onboard_review_lines_for_draft_with_guidance_and_style(
    draft: &OnboardDraft,
    import_source: Option<&str>,
    workspace_guidance: &[crate::migration::WorkspaceGuidanceCandidate],
    selected_candidate: Option<&ImportCandidate>,
    width: usize,
    flow_style: ReviewFlowStyle,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_onboard_review_screen_spec_for_draft(
        draft,
        import_source,
        workspace_guidance,
        selected_candidate,
        flow_style,
    );

    render_onboard_screen_spec(&spec, width, color_enabled)
}

fn build_onboard_review_screen_spec(
    config: &mvp::config::LoongClawConfig,
    import_source: Option<&str>,
    workspace_guidance: &[crate::migration::WorkspaceGuidanceCandidate],
    selected_candidate: Option<&ImportCandidate>,
    flow_style: ReviewFlowStyle,
) -> TuiScreenSpec {
    let mut sections = Vec::new();

    if let Some(source) = import_source {
        let starting_point_label = onboard_starting_point_label(None, source);
        let starting_point_lines = vec![onboard_display_line(
            "- starting point: ",
            &starting_point_label,
        )];
        let starting_point_section = TuiSectionSpec::Narrative {
            title: Some("starting point".to_owned()),
            lines: starting_point_lines,
        };

        sections.push(starting_point_section);
    }

    let configuration_lines = build_onboard_review_digest_display_lines(config);
    let configuration_section = TuiSectionSpec::Narrative {
        title: Some("configuration".to_owned()),
        lines: configuration_lines,
    };

    sections.push(configuration_section);

    let review_candidate = build_onboard_review_candidate_with_selected_context(
        config,
        workspace_guidance,
        selected_candidate,
    );
    let draft_source_lines =
        crate::migration::render::candidate_preview_display_lines(&review_candidate);
    let draft_source_section = TuiSectionSpec::Narrative {
        title: Some("draft source".to_owned()),
        lines: draft_source_lines,
    };

    sections.push(draft_source_section);

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some(flow_style.header_subtitle().to_owned()),
        title: Some("review setup".to_owned()),
        progress_line: Some(flow_style.progress_line()),
        intro_lines: Vec::new(),
        sections,
        choices: Vec::new(),
        footer_lines: Vec::new(),
    }
}

fn build_onboard_review_screen_spec_for_draft(
    draft: &OnboardDraft,
    import_source: Option<&str>,
    workspace_guidance: &[crate::migration::WorkspaceGuidanceCandidate],
    selected_candidate: Option<&ImportCandidate>,
    flow_style: ReviewFlowStyle,
) -> TuiScreenSpec {
    let mut sections = Vec::new();

    if let Some(source) = import_source {
        let starting_point_label = onboard_starting_point_label(None, source);
        let starting_point_lines = vec![onboard_display_line(
            "- starting point: ",
            &starting_point_label,
        )];
        let starting_point_section = TuiSectionSpec::Narrative {
            title: Some("starting point".to_owned()),
            lines: starting_point_lines,
        };

        sections.push(starting_point_section);
    }

    let configuration_lines = build_onboard_review_digest_display_lines_for_draft(draft);
    let configuration_section = TuiSectionSpec::Narrative {
        title: Some("configuration".to_owned()),
        lines: configuration_lines,
    };

    sections.push(configuration_section);

    let review_candidate = build_onboard_review_candidate_with_selected_context(
        &draft.config,
        workspace_guidance,
        selected_candidate,
    );
    let draft_source_lines =
        crate::migration::render::candidate_preview_display_lines(&review_candidate);
    let draft_source_section = TuiSectionSpec::Narrative {
        title: Some("draft source".to_owned()),
        lines: draft_source_lines,
    };

    sections.push(draft_source_section);

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some(flow_style.header_subtitle().to_owned()),
        title: Some("review setup".to_owned()),
        progress_line: Some(flow_style.progress_line()),
        intro_lines: Vec::new(),
        sections,
        choices: Vec::new(),
        footer_lines: Vec::new(),
    }
}

#[cfg(test)]
#[cfg(test)]
mod tests;
