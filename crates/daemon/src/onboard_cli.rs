use std::collections::BTreeSet;
use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use dialoguer::console::{Term, user_attended};
use dialoguer::theme::ColorfulTheme;
use loong_app as mvp;
use loong_contracts::SecretRef;
use loong_spec::CliResult;

use crate::copilot_onboarding::finalize_github_copilot_onboard_credentials;
use crate::onboard_finalize::{
    ConfigWritePlan, build_onboarding_success_summary_with_memory, prepare_output_path_for_write,
    render_onboarding_success_summary_lines, resolve_backup_path, rollback_onboard_write_failure,
};
#[cfg(test)]
use crate::onboard_finalize::{
    OnboardWriteRecovery, format_backup_timestamp_at, resolve_backup_path_at,
};
pub use crate::onboard_preflight::{
    OnboardCheck, OnboardCheckLevel, OnboardNonInteractiveWarningPolicy,
    collect_channel_preflight_checks, directory_preflight_check, provider_credential_check,
    render_current_setup_preflight_summary_screen_lines,
    render_detected_setup_preflight_summary_screen_lines, render_preflight_summary_screen_lines,
};
use crate::onboard_preflight::{
    config_validation_failure_message,
    is_explicitly_accepted_non_interactive_warning as preflight_accepts_non_interactive_warning,
    non_interactive_preflight_failure_message, render_preflight_summary_screen_lines_with_progress,
    run_preflight_checks,
};
pub use crate::onboard_types::OnboardingCredentialSummary;
#[cfg(test)]
use crate::onboard_web_search::{
    WebSearchProviderRecommendation, WebSearchProviderRecommendationSource,
    recommend_web_search_provider_from_available_credentials,
};
use crate::onboard_web_search::{
    current_web_search_provider, explicit_web_search_provider_override,
    resolve_effective_web_search_default_provider, resolve_web_search_provider_recommendation,
};
use crate::onboarding_model_policy;
use crate::provider_credential_policy;
use crate::query_search_guidance::{
    configured_query_search_credential_env_name, configured_query_search_credential_source_value,
    configured_query_search_secret, preferred_query_search_credential_env_default,
    query_search_has_inline_credential, query_search_provider_display_name,
    summarize_query_search_credential,
};
use mvp::tui_surface::{
    TuiCalloutTone, TuiChoiceSpec, TuiHeaderStyle, TuiScreenSpec, TuiSectionSpec,
    render_onboard_screen_spec,
};
#[cfg(test)]
use std::fs;
#[cfg(test)]
use time::OffsetDateTime;

#[path = "onboard_select.rs"]
mod select_support;

pub use crate::onboard_import::{
    ImportCandidate, ImportSurface, ImportSurfaceLevel, OnboardEntryChoice, OnboardEntryOption,
    build_onboard_entry_options,
};
use crate::onboard_import::{
    StartingConfigSelection, default_onboard_entry_choice, default_starting_config_selection,
    import_candidate_from_migration, migration_candidate_for_onboard_display,
    migration_candidate_from_onboard, onboard_starting_point_label, prepare_import_starting_state,
    select_non_interactive_starting_config_from_state, sort_starting_point_candidates,
};

use self::select_support::*;
#[path = "onboard_screen_specs.rs"]
mod screen_spec_support;

use self::screen_spec_support::*;
#[path = "onboard_cli_render.rs"]
mod onboard_cli_render;
pub use self::onboard_cli_render::{
    append_escape_cancel_hint,
    render_api_key_env_selection_screen_lines,
    render_api_key_env_selection_screen_lines_with_default,
    render_continue_current_setup_screen_lines,
    render_continue_detected_setup_screen_lines,
    collect_import_candidates_with_paths,
    render_current_setup_review_lines_with_guidance,
    render_current_setup_write_confirmation_screen_lines,
    render_detected_setup_review_lines_with_guidance,
    render_detected_setup_write_confirmation_screen_lines,
    render_existing_config_write_screen_lines,
    render_model_selection_screen_lines,
    render_model_selection_screen_lines_with_default,
    render_onboard_entry_screen_lines,
    render_onboard_review_lines_with_guidance,
    render_onboarding_risk_screen_lines,
    render_provider_selection_screen_lines,
    render_single_detected_setup_preview_screen_lines,
    render_starting_point_selection_screen_lines,
    render_system_prompt_selection_screen_lines,
    render_system_prompt_selection_screen_lines_with_default,
    render_default_choice_footer_line,
    render_write_confirmation_screen_lines,
    summarize_prompt_addendum,
    summarize_prompt_mode,
    summarize_provider_credential,
};
use self::onboard_cli_render::{
    build_onboard_review_candidate_with_selected_context,
    build_onboard_review_digest_display_lines,
    prompt_onboard_entry_choice,
    render_api_key_env_selection_screen_lines_with_style,
    render_existing_config_write_header_lines_with_style,
    render_model_selection_screen_lines_with_style,
    render_onboard_choice_screen,
    render_onboard_entry_interactive_screen_lines_with_style,
    render_onboard_review_lines_with_guidance_and_style,
    render_onboard_shortcut_header_lines_with_style,
    render_prompt_with_default_text,
    render_provider_selection_header_lines,
    render_system_prompt_selection_screen_lines_with_style,
    render_web_search_credential_selection_screen_lines_with_style,
    screen_subtitle,
    select_interactive_import_starting_config,
    start_fresh_starting_point_detail_lines,
    summarize_starting_point_detail_lines,
    tui_header_style,
};
#[cfg(test)]
use self::onboard_cli_render::{
    provider_matches_for_review, render_onboard_option_lines, render_onboard_option_prefix,
    render_onboard_shortcut_screen_lines_with_style,
    render_starting_point_selection_header_lines_with_style,
};
pub use crate::onboard_finalize::{
    OnboardingAction, OnboardingActionKind, OnboardingChannelSurfaceSummary,
    OnboardingDomainOutcome, OnboardingSuccessSummary, backup_existing_config,
    build_onboarding_success_summary, render_onboarding_success_summary_with_width,
};
const ONBOARD_CLEAR_INPUT_TOKEN: &str = ":clear";
const ONBOARD_CUSTOM_MODEL_OPTION_SLUG: &str = "__custom_model__";
const ONBOARD_ESCAPE_CANCEL_HINT: &str = "- press Esc then Enter to cancel onboarding";
const ONBOARD_SINGLE_LINE_INPUT_HINT: &str = "- single-line input only";
const ONBOARD_PASTE_DRAIN_WINDOW_ENV: &str = "LOONG_ONBOARD_PASTE_DRAIN_WINDOW_MS";
const DEFAULT_ONBOARD_PASTE_DRAIN_WINDOW: Duration = Duration::from_millis(75);
const ONBOARD_LINE_READER_BUFFER_SIZE: usize = 64;
const PREINSTALLED_SKILLS_PROMPT_LABEL: &str = "preinstalled skills";

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

pub trait OnboardUi {
    fn print_line(&mut self, line: &str) -> CliResult<()>;
    fn prompt_with_default(&mut self, label: &str, default: &str) -> CliResult<String>;
    fn prompt_required(&mut self, label: &str) -> CliResult<String>;
    fn prompt_allow_empty(&mut self, label: &str) -> CliResult<String> {
        self.prompt_required(label)
    }
    fn prompt_confirm(&mut self, message: &str, default: bool) -> CliResult<bool>;
    fn select_one(
        &mut self,
        label: &str,
        options: &[SelectOption],
        default: Option<usize>,
        interaction_mode: SelectInteractionMode,
    ) -> CliResult<usize>;
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

fn is_explicitly_accepted_non_interactive_warning(
    check: &OnboardCheck,
    options: &OnboardCommandOptions,
) -> bool {
    preflight_accepts_non_interactive_warning(check, options.skip_model_probe)
}

#[cfg(test)]
fn provider_model_probe_failure_check(
    config: &mvp::config::LoongConfig,
    error: String,
) -> OnboardCheck {
    crate::onboard_preflight::provider_model_probe_failure_check(config, error)
}

trait OnboardPromptLineReader {
    fn read_blocking_line(&mut self) -> CliResult<OnboardPromptRead>;
    fn read_pending_line(&mut self) -> CliResult<Option<String>>;
}

#[derive(Debug, PartialEq, Eq)]
enum OnboardPromptRead {
    Line(String),
    Eof,
}

#[derive(Debug)]
enum StdioOnboardLineMessage {
    Line(String),
    Eof,
    Error(String),
}

type StdioOnboardLineSender = mpsc::SyncSender<StdioOnboardLineMessage>;

#[derive(Debug)]
enum StdioOnboardLineReader {
    Background {
        receiver: Receiver<StdioOnboardLineMessage>,
        paste_drain_window: Duration,
    },
    Direct {
        degraded_notice: Option<String>,
    },
}

fn onboard_line_channel() -> (StdioOnboardLineSender, Receiver<StdioOnboardLineMessage>) {
    onboard_line_channel_with_capacity(ONBOARD_LINE_READER_BUFFER_SIZE)
}

fn onboard_line_channel_with_capacity(
    buffer_size: usize,
) -> (StdioOnboardLineSender, Receiver<StdioOnboardLineMessage>) {
    assert!(
        buffer_size > 0,
        "onboard line reader buffer must be non-zero"
    );
    mpsc::sync_channel(buffer_size)
}

fn onboard_paste_drain_window() -> Duration {
    env::var(ONBOARD_PASTE_DRAIN_WINDOW_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_ONBOARD_PASTE_DRAIN_WINDOW)
}

fn spawn_onboard_stdin_reader(sender: StdioOnboardLineSender) -> io::Result<()> {
    thread::Builder::new()
        .name("loong-onboard-stdin".to_owned())
        .spawn(move || {
            loop {
                let mut line = String::new();
                match io::stdin().read_line(&mut line) {
                    Ok(0) => {
                        let _ = sender.send(StdioOnboardLineMessage::Eof);
                        break;
                    }
                    Ok(_) => {
                        if sender.send(StdioOnboardLineMessage::Line(line)).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(StdioOnboardLineMessage::Error(format!(
                            "read stdin failed: {error}"
                        )));
                        break;
                    }
                }
            }
        })
        .map(|_handle| ())
}

fn format_onboard_line_reader_spawn_notice(error: &io::Error) -> String {
    format!(
        "warning: failed to start onboarding stdin reader thread ({error}); single-line paste draining is disabled for this session"
    )
}

impl StdioOnboardLineReader {
    fn background_from_receiver(receiver: Receiver<StdioOnboardLineMessage>) -> Self {
        Self::Background {
            receiver,
            paste_drain_window: onboard_paste_drain_window(),
        }
    }

    fn try_spawn_background_receiver() -> io::Result<Receiver<StdioOnboardLineMessage>> {
        let (sender, receiver) = onboard_line_channel();
        spawn_onboard_stdin_reader(sender)?;
        Ok(receiver)
    }

    fn from_spawn_result(result: io::Result<Receiver<StdioOnboardLineMessage>>) -> Self {
        match result {
            Ok(receiver) => Self::background_from_receiver(receiver),
            Err(error) => Self::Direct {
                degraded_notice: Some(format_onboard_line_reader_spawn_notice(&error)),
            },
        }
    }

    fn take_degraded_notice(&mut self) -> Option<String> {
        match self {
            Self::Background { .. } => None,
            Self::Direct { degraded_notice } => degraded_notice.take(),
        }
    }
}

impl Default for StdioOnboardLineReader {
    fn default() -> Self {
        Self::from_spawn_result(Self::try_spawn_background_receiver())
    }
}

impl OnboardPromptLineReader for StdioOnboardLineReader {
    fn read_blocking_line(&mut self) -> CliResult<OnboardPromptRead> {
        if let Some(notice) = self.take_degraded_notice() {
            eprintln!("{notice}");
        }
        match self {
            Self::Background { receiver, .. } => match receiver.recv() {
                Ok(StdioOnboardLineMessage::Line(line)) => Ok(OnboardPromptRead::Line(line)),
                Ok(StdioOnboardLineMessage::Eof) => Ok(OnboardPromptRead::Eof),
                Ok(StdioOnboardLineMessage::Error(error)) => Err(error),
                Err(_) => Ok(OnboardPromptRead::Eof),
            },
            Self::Direct { .. } => {
                let mut line = String::new();
                let bytes_read = io::stdin()
                    .read_line(&mut line)
                    .map_err(|error| format!("read stdin failed: {error}"))?;
                if bytes_read == 0 {
                    return Ok(OnboardPromptRead::Eof);
                }
                Ok(OnboardPromptRead::Line(line))
            }
        }
    }

    fn read_pending_line(&mut self) -> CliResult<Option<String>> {
        match self {
            Self::Background {
                receiver,
                paste_drain_window,
            } => match receiver.recv_timeout(*paste_drain_window) {
                Ok(StdioOnboardLineMessage::Line(line)) => Ok(Some(line)),
                Ok(StdioOnboardLineMessage::Eof) => Ok(None),
                Ok(StdioOnboardLineMessage::Error(error)) => Err(error),
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => Ok(None),
            },
            Self::Direct { .. } => Ok(None),
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct StdioOnboardUi {
    line_reader: Option<StdioOnboardLineReader>,
}

impl StdioOnboardUi {
    fn stdio_line_reader(&mut self) -> &mut StdioOnboardLineReader {
        self.line_reader
            .get_or_insert_with(StdioOnboardLineReader::default)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct OnboardPromptCapture {
    raw: String,
    dropped_line_count: usize,
    reached_eof: bool,
}

fn read_single_line_prompt_capture(
    reader: &mut impl OnboardPromptLineReader,
) -> CliResult<OnboardPromptCapture> {
    let read = reader.read_blocking_line()?;
    let mut dropped_line_count = 0;
    let (raw, reached_eof) = match read {
        OnboardPromptRead::Line(raw) => {
            while reader.read_pending_line()?.is_some() {
                dropped_line_count += 1;
            }
            (raw, false)
        }
        OnboardPromptRead::Eof => (String::new(), true),
    };
    Ok(OnboardPromptCapture {
        raw,
        dropped_line_count,
        reached_eof,
    })
}

fn print_dropped_paste_notice(label: &str, dropped_line_count: usize) {
    if dropped_line_count == 0 {
        return;
    }
    let noun = if dropped_line_count == 1 {
        "line"
    } else {
        "lines"
    };
    println!(
        "note: {label} accepts a single line; ignored {dropped_line_count} extra pasted {noun}"
    );
}

impl OnboardUi for StdioOnboardUi {
    fn print_line(&mut self, line: &str) -> CliResult<()> {
        println!("{line}");
        Ok(())
    }

    fn prompt_with_default(&mut self, label: &str, default: &str) -> CliResult<String> {
        if rich_prompt_ui_available() {
            return prompt_with_default_rich(label, default);
        }
        prompt_with_default_stdio(self.stdio_line_reader(), label, default)
    }

    fn prompt_required(&mut self, label: &str) -> CliResult<String> {
        if rich_prompt_ui_available() {
            return prompt_required_rich(label);
        }
        prompt_required_stdio(self.stdio_line_reader(), label)
    }

    fn prompt_allow_empty(&mut self, label: &str) -> CliResult<String> {
        if rich_prompt_ui_available() {
            return prompt_allow_empty_rich(label);
        }
        prompt_required_stdio(self.stdio_line_reader(), label)
    }

    fn prompt_confirm(&mut self, message: &str, default: bool) -> CliResult<bool> {
        if rich_prompt_ui_available() {
            return prompt_confirm_rich(message, default);
        }
        prompt_confirm_stdio(self.stdio_line_reader(), message, default)
    }

    fn select_one(
        &mut self,
        label: &str,
        options: &[SelectOption],
        default: Option<usize>,
        interaction_mode: SelectInteractionMode,
    ) -> CliResult<usize> {
        if rich_prompt_ui_available() {
            return select_one_rich(label, options, default, interaction_mode);
        }
        select_one_stdio(self.stdio_line_reader(), label, options, default)
    }
}

fn prompt_with_default_stdio(
    line_reader: &mut impl OnboardPromptLineReader,
    label: &str,
    default: &str,
) -> CliResult<String> {
    print!("{}", render_prompt_with_default_text(label, default));
    io::stdout()
        .flush()
        .map_err(|error| format!("flush stdout failed: {error}"))?;
    let capture = read_single_line_prompt_capture(line_reader)?;
    let line = ensure_onboard_input_not_cancelled(capture.raw)?;
    print_dropped_paste_notice(label, capture.dropped_line_count);
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(default.to_owned());
    }
    Ok(trimmed.to_owned())
}

fn prompt_required_stdio(
    line_reader: &mut impl OnboardPromptLineReader,
    label: &str,
) -> CliResult<String> {
    print!("{label}: ");
    io::stdout()
        .flush()
        .map_err(|error| format!("flush stdout failed: {error}"))?;
    let capture = read_single_line_prompt_capture(line_reader)?;
    let line = ensure_onboard_input_not_cancelled(capture.raw)?;
    print_dropped_paste_notice(label, capture.dropped_line_count);
    Ok(line.trim().to_owned())
}

fn prompt_confirm_stdio(
    line_reader: &mut impl OnboardPromptLineReader,
    message: &str,
    default: bool,
) -> CliResult<bool> {
    let suffix = if default { "[Y/n]" } else { "[y/N]" };
    print!("{message} {suffix}: ");
    io::stdout()
        .flush()
        .map_err(|error| format!("flush stdout failed: {error}"))?;
    let capture = read_single_line_prompt_capture(line_reader)?;
    let line = ensure_onboard_input_not_cancelled(capture.raw)?;
    print_dropped_paste_notice(message, capture.dropped_line_count);
    let value = line.trim().to_ascii_lowercase();
    if value.is_empty() {
        return Ok(default);
    }
    Ok(matches!(value.as_str(), "y" | "yes"))
}

fn select_one_stdio(
    line_reader: &mut impl OnboardPromptLineReader,
    label: &str,
    options: &[SelectOption],
    default: Option<usize>,
) -> CliResult<usize> {
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
            .map_err(|error| format!("flush stdout failed: {error}"))?;
        let capture = read_single_line_prompt_capture(line_reader)?;
        print_dropped_paste_notice(label, capture.dropped_line_count);
        if capture.reached_eof {
            return resolve_select_one_eof(default);
        }
        let input = ensure_onboard_input_not_cancelled(capture.raw)?;
        let trimmed = input.trim();
        if trimmed.is_empty() {
            if let Some(idx) = default {
                return Ok(idx);
            }
            println!("Please select an option.");
            continue;
        }
        if let Some(index) = parse_select_one_input(trimmed, options) {
            return Ok(index);
        }
        println!("{}", render_select_one_invalid_input_message(options));
    }
}

fn rich_prompt_ui_available() -> bool {
    user_attended()
}

fn rich_prompt_theme() -> ColorfulTheme {
    ColorfulTheme::default()
}

fn rich_prompt_term() -> Term {
    Term::stdout()
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

fn is_explicit_onboard_cancel_input(raw: &str) -> bool {
    matches!(raw.trim(), "\u{1b}")
}

fn ensure_onboard_input_not_cancelled(raw: String) -> CliResult<String> {
    if is_explicit_onboard_cancel_input(raw.as_str()) {
        return Err("onboarding cancelled: escape input received".to_owned());
    }
    Ok(raw)
}

fn render_preinstalled_skills_selection_screen_lines_with_style(
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let options = mvp::tools::bundled_preinstall_targets()
        .iter()
        .map(|target| OnboardScreenOption {
            key: target.install_id.to_owned(),
            label: target.display_name.to_owned(),
            detail_lines: vec![target.summary.to_owned()],
            recommended: target.recommended,
        })
        .collect();
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "optional add-ons",
        "preinstalled skills",
        None,
        vec![
            "- choose zero or more bundled skills to install into the managed runtime".to_owned(),
            "- type comma-separated ids, for example: find-skills,agent-browser".to_owned(),
        ],
        options,
        vec!["- press Enter to skip".to_owned()],
        true,
        color_enabled,
    )
}

fn parse_preinstalled_skill_selection(raw: &str) -> CliResult<Vec<String>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let mut selected = Vec::new();
    let mut seen = BTreeSet::new();
    for token in trimmed
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        let Some(choice) = mvp::tools::bundled_preinstall_targets()
            .iter()
            .find(|choice| choice.install_id.eq_ignore_ascii_case(token))
        else {
            let supported = mvp::tools::bundled_preinstall_targets()
                .iter()
                .map(|choice| choice.install_id)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "unsupported preinstalled skill selection `{token}`. choose from: {supported}"
            ));
        };
        for skill_id in choice.skill_ids {
            if seen.insert((*skill_id).to_owned()) {
                selected.push((*skill_id).to_owned());
            }
        }
    }
    Ok(selected)
}

fn resolve_preinstalled_skill_selection(
    options: &OnboardCommandOptions,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<Vec<String>> {
    if options.non_interactive {
        return Ok(Vec::new());
    }

    print_lines(
        ui,
        render_preinstalled_skills_selection_screen_lines_with_style(context.render_width, true),
    )?;
    let raw = ui.prompt_allow_empty(PREINSTALLED_SKILLS_PROMPT_LABEL)?;
    parse_preinstalled_skill_selection(raw.as_str())
}

fn onboarding_default_skills_install_root(output_path: &Path) -> PathBuf {
    let base_dir = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    base_dir.join(".loong/skills")
}

fn apply_selected_preinstalled_skills_to_config(
    config: &mut mvp::config::LoongConfig,
    output_path: &Path,
    selected_skill_ids: &[String],
) {
    if selected_skill_ids.is_empty() {
        return;
    }
    config.skills.enabled = true;
    config.skills.auto_expose_installed = true;
    if config.skills.install_root.is_none() {
        config.skills.install_root = Some(
            onboarding_default_skills_install_root(output_path)
                .display()
                .to_string(),
        );
    }
}

fn install_root_for_onboarded_skills(
    config: &mvp::config::LoongConfig,
    config_path: &Path,
) -> PathBuf {
    config
        .skills
        .resolved_install_root()
        .unwrap_or_else(|| onboarding_default_skills_install_root(config_path))
}

fn install_selected_preinstalled_skills(
    config_path: &Path,
    config: &mvp::config::LoongConfig,
    selected_skill_ids: &[String],
) -> CliResult<()> {
    if selected_skill_ids.is_empty() {
        return Ok(());
    }

    let install_root = install_root_for_onboarded_skills(config, config_path);
    let tool_runtime_config =
        mvp::tools::runtime_config::ToolRuntimeConfig::from_loong_config(config, Some(config_path));
    let mut installed_now: Vec<String> = Vec::new();

    for skill_id in selected_skill_ids {
        if install_root.join(skill_id).join("SKILL.md").is_file() {
            continue;
        }
        if let Err(error) = mvp::tools::skills_install_with_config(
            None,
            Some(skill_id.as_str()),
            None,
            None,
            false,
            false,
            &tool_runtime_config,
        ) {
            for installed_skill_id in installed_now.iter().rev() {
                let _ =
                    mvp::tools::skills_remove_with_config(installed_skill_id, &tool_runtime_config);
            }
            return Err(format!(
                "failed to install selected bundled skill `{skill_id}`: {error}"
            ));
        }
        installed_now.push(skill_id.clone());
    }

    Ok(())
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
            GuidedPromptPath::NativePromptPack => 7,
            GuidedPromptPath::InlineOverride => 6,
        }
    }

    const fn index(self, step: GuidedOnboardStep) -> usize {
        match (self, step) {
            (_, GuidedOnboardStep::Provider) => 1,
            (_, GuidedOnboardStep::Model) => 2,
            (_, GuidedOnboardStep::CredentialEnv) => 3,
            (GuidedPromptPath::NativePromptPack, GuidedOnboardStep::PromptCustomization) => 4,
            (_, GuidedOnboardStep::WebSearchProvider) => match self {
                GuidedPromptPath::NativePromptPack => 5,
                GuidedPromptPath::InlineOverride => 4,
            },
            (GuidedPromptPath::NativePromptPack, GuidedOnboardStep::Review) => 6,
            (GuidedPromptPath::InlineOverride, GuidedOnboardStep::PromptCustomization) => 4,
            (GuidedPromptPath::InlineOverride, GuidedOnboardStep::Review) => 5,
        }
    }

    const fn label(self, step: GuidedOnboardStep) -> &'static str {
        match step {
            GuidedOnboardStep::Provider => "provider",
            GuidedOnboardStep::Model => "model",
            GuidedOnboardStep::CredentialEnv => "credential source",
            GuidedOnboardStep::PromptCustomization => match self {
                GuidedPromptPath::NativePromptPack => "prompt addendum",
                GuidedPromptPath::InlineOverride => "system prompt",
            },
            GuidedOnboardStep::WebSearchProvider => "web search",
            GuidedOnboardStep::Review => "review",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuidedOnboardStep {
    Provider,
    Model,
    CredentialEnv,
    PromptCustomization,
    WebSearchProvider,
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
pub(crate) struct OnboardScreenOption {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) detail_lines: Vec<String>,
    pub(crate) recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
pub type ChannelImportReadiness = crate::migration::ChannelImportReadiness;

pub async fn run_onboard_cli(options: OnboardCommandOptions) -> CliResult<()> {
    let context = OnboardRuntimeContext::capture();
    let mut ui = StdioOnboardUi::default();
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
            render_onboard_shortcut_header_lines_with_style(
                shortcut_kind,
                &config,
                starting_selection.import_source.as_deref(),
                context.render_width,
                true,
            ),
        )?;
        matches!(
            prompt_onboard_shortcut_choice(ui, shortcut_kind)?,
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

        let available_models = load_onboarding_model_catalog(&options, &config).await;
        let selected_model = resolve_model_selection(
            &options,
            &config,
            guided_prompt_path,
            &available_models,
            ui,
            context,
        )?;
        config.provider.model = selected_model;

        if config.provider.kind == mvp::config::ProviderKind::GithubCopilot {
            finalize_github_copilot_onboard_credentials(
                &mut config.provider,
                &output_path,
                options.non_interactive,
            )
            .await?;
        } else {
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
        }

        match guided_prompt_path {
            GuidedPromptPath::NativePromptPack => {
                if options.non_interactive
                    && let Some(personality_raw) = options.personality.as_deref()
                {
                    let personality = parse_prompt_personality(personality_raw).ok_or_else(|| {
                        format!(
                            "unsupported --personality value \"{personality_raw}\". supported: {}",
                            supported_personality_list()
                        )
                    })?;
                    config.cli.prompt_pack_id =
                        Some(mvp::prompt::DEFAULT_PROMPT_PACK_ID.to_owned());
                    config.cli.personality = Some(personality);
                    config.cli.refresh_native_system_prompt();
                }
            }
            GuidedPromptPath::InlineOverride => {
                if options.non_interactive {
                    if let Some(system_prompt) = options.system_prompt.clone() {
                        let selected_system_prompt =
                            if is_explicit_onboard_clear_input(system_prompt.as_str()) {
                                Some(String::new())
                            } else {
                                Some(system_prompt)
                            };
                        apply_selected_system_prompt(&mut config, selected_system_prompt);
                    }
                } else {
                    let prompt_default = options
                        .system_prompt
                        .as_deref()
                        .filter(|value| !value.trim().is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            if config.cli.uses_native_prompt_pack() {
                                String::new()
                            } else {
                                config.cli.system_prompt.clone()
                            }
                        });
                    print_lines(
                        ui,
                        render_system_prompt_selection_screen_lines_with_style(
                            &config,
                            prompt_default.as_str(),
                            guided_prompt_path,
                            context.render_width,
                            true,
                        ),
                    )?;
                    let value = ui.prompt_with_default("System prompt", prompt_default.as_str())?;
                    let selected_system_prompt = if is_explicit_onboard_clear_input(&value) {
                        Some(String::new())
                    } else {
                        let trimmed = value.trim();
                        if trimmed.is_empty() {
                            None
                        } else {
                            Some(trimmed.to_owned())
                        }
                    };
                    apply_selected_system_prompt(&mut config, selected_system_prompt);
                }
            }
        }

        if let Some(profile_raw) = options.memory_profile.as_deref() {
            config.memory.profile = parse_memory_profile(profile_raw).ok_or_else(|| {
                format!(
                    "unsupported --memory-profile value \"{profile_raw}\". supported: {}",
                    supported_memory_profile_list()
                )
            })?;
        }

        let selected_web_search_provider = resolve_web_search_provider_selection(
            &options,
            &config,
            guided_prompt_path,
            ui,
            context,
        )
        .await?;
        config.tools.web_search.default_provider = selected_web_search_provider.clone();
        let web_search_credential_selection = resolve_web_search_credential_selection(
            &options,
            &config,
            selected_web_search_provider.as_str(),
            guided_prompt_path,
            options.non_interactive,
            ui,
            context,
        )?;
        apply_selected_web_search_credential(
            &mut config,
            selected_web_search_provider.as_str(),
            web_search_credential_selection,
        )?;
    }
    let selected_preinstalled_skill_ids =
        resolve_preinstalled_skill_selection(&options, ui, context)?;
    apply_selected_preinstalled_skills_to_config(
        &mut config,
        &output_path,
        &selected_preinstalled_skill_ids,
    );

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
    let config_validation_failure = config_validation_failure_message(&checks);

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
        if let Some(message) = config_validation_failure {
            return Err(message);
        }
        if !credential_ok {
            let credential_hint =
                provider_credential_policy::provider_credential_env_hint(&config.provider)
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
        if let Some(message) = config_validation_failure {
            return Err(message);
        }
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
        let write_recovery = prepare_output_path_for_write(&output_path, &write_plan)?;
        let backup_path = if write_recovery.keep_backup_on_success {
            write_recovery.backup_path.as_deref()
        } else {
            None
        };
        if let Some(backup_path) = backup_path {
            let backup_message = format!("Backed up existing config to: {}", backup_path.display());
            print_message(ui, backup_message)?;
        }
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

    if let Err(error) =
        install_selected_preinstalled_skills(&path, &config, &selected_preinstalled_skill_ids)
    {
        if let Some(write_recovery) = write_recovery.as_ref() {
            return Err(rollback_onboard_write_failure(
                &output_path,
                write_recovery,
                error,
            ));
        }
        return Err(error);
    }

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
    let success_summary_lines =
        render_onboarding_success_summary_lines(&success_summary, context.render_width, true);
    print_lines(ui, success_summary_lines)?;
    Ok(())
}

fn resolve_guided_prompt_path(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongConfig,
) -> GuidedPromptPath {
    if options.system_prompt.is_some() {
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
    config: &mvp::config::LoongConfig,
) -> &'static str {
    match resolve_guided_prompt_path(options, config) {
        GuidedPromptPath::NativePromptPack => "native",
        GuidedPromptPath::InlineOverride => "inline",
    }
}

pub fn build_channel_onboarding_follow_up_lines(config: &mvp::config::LoongConfig) -> Vec<String> {
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

fn resolve_provider_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongConfig,
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

    if !provider_selection.imported_choices.is_empty() {
        let select_options: Vec<SelectOption> = provider_selection
            .imported_choices
            .iter()
            .map(|choice| SelectOption {
                label: provider_kind_display_name(choice.kind).to_owned(),
                slug: choice.profile_id.clone(),
                description: format!("source: {}, summary: {}", choice.source, choice.summary),
                recommended: Some(choice.profile_id.as_str())
                    == provider_selection.default_profile_id.as_deref(),
            })
            .collect();
        let default_idx = if provider_selection.requires_explicit_choice {
            None
        } else {
            provider_selection
                .default_profile_id
                .as_deref()
                .and_then(|default_id| {
                    provider_selection
                        .imported_choices
                        .iter()
                        .position(|choice| choice.profile_id == default_id)
                })
        };
        print_lines(
            ui,
            render_provider_selection_header_lines(
                provider_selection,
                guided_prompt_path,
                context.render_width,
            ),
        )?;
        let idx = ui.select_one(
            "Provider",
            &select_options,
            default_idx,
            SelectInteractionMode::List,
        )?;
        let choice = provider_selection
            .imported_choices
            .get(idx)
            .ok_or_else(|| format!("provider selection index {idx} out of range"))?;
        return Ok(choice.config.clone());
    }

    // No imported choices — still use the numbered chooser so the provider
    // step stays aligned with the rest of onboarding.
    let default_provider_kind = options
        .provider
        .as_deref()
        .and_then(parse_provider_kind)
        .or(provider_selection.default_kind)
        .or_else(|| {
            provider_selection
                .default_profile_id
                .as_deref()
                .and_then(parse_provider_kind)
        })
        .unwrap_or(config.provider.kind);
    let provider_kinds = mvp::config::ProviderKind::all_sorted()
        .iter()
        .copied()
        .filter(|kind| {
            *kind != mvp::config::ProviderKind::Kimi
                && *kind != mvp::config::ProviderKind::KimiCoding
                && *kind != mvp::config::ProviderKind::Stepfun
                && *kind != mvp::config::ProviderKind::StepPlan
        })
        .collect::<Vec<_>>();
    let mut select_options: Vec<SelectOption> = provider_kinds
        .iter()
        .map(|kind| SelectOption {
            label: provider_kind_display_name(*kind).to_owned(),
            slug: provider_kind_id(*kind).to_owned(),
            description: String::new(),
            recommended: *kind == default_provider_kind,
        })
        .collect();
    select_options.push(SelectOption {
        label: "Kimi".to_owned(),
        slug: "kimi".to_owned(),
        description: "Kimi API or Kimi Coding".to_owned(),
        recommended: default_provider_kind == mvp::config::ProviderKind::Kimi
            || default_provider_kind == mvp::config::ProviderKind::KimiCoding,
    });
    select_options.push(SelectOption {
        label: "Stepfun".to_owned(),
        slug: "stepfun".to_owned(),
        description: "Stepfun API or Step Plan".to_owned(),
        recommended: default_provider_kind == mvp::config::ProviderKind::Stepfun
            || default_provider_kind == mvp::config::ProviderKind::StepPlan,
    });
    select_options.sort_by(|a, b| a.label.cmp(&b.label));
    let default_provider_slug = if matches!(
        default_provider_kind,
        mvp::config::ProviderKind::Kimi | mvp::config::ProviderKind::KimiCoding
    ) {
        "kimi"
    } else if matches!(
        default_provider_kind,
        mvp::config::ProviderKind::Stepfun | mvp::config::ProviderKind::StepPlan
    ) {
        "stepfun"
    } else {
        provider_kind_id(default_provider_kind)
    };
    let default_idx = if provider_selection.requires_explicit_choice {
        None
    } else {
        select_options
            .iter()
            .position(|option| option.slug == default_provider_slug)
    };
    print_lines(
        ui,
        render_provider_selection_header_lines(
            provider_selection,
            guided_prompt_path,
            context.render_width,
        ),
    )?;
    let idx = ui.select_one(
        "Provider",
        &select_options,
        default_idx,
        SelectInteractionMode::List,
    )?;
    let selected_slug = select_options
        .get(idx)
        .ok_or_else(|| format!("provider selection index {idx} out of range"))?
        .slug
        .clone();

    let kind: mvp::config::ProviderKind = if selected_slug == "kimi" {
        let kimi_options = vec![
            SelectOption {
                label: "Kimi API".to_owned(),
                slug: "kimi_api".to_owned(),
                description: "Standard Kimi chat completion API".to_owned(),
                recommended: true,
            },
            SelectOption {
                label: "Kimi Coding".to_owned(),
                slug: "kimi_coding".to_owned(),
                description: "Kimi for coding tasks".to_owned(),
                recommended: false,
            },
        ];
        print_lines(ui, vec!["Select the Kimi variant:".to_owned()])?;
        let kimi_default_idx = Some(usize::from(
            default_provider_kind == mvp::config::ProviderKind::KimiCoding,
        ));
        let sub_idx = ui.select_one(
            "Kimi variant",
            &kimi_options,
            kimi_default_idx,
            SelectInteractionMode::List,
        )?;
        let sub_slug = kimi_options
            .get(sub_idx)
            .ok_or_else(|| format!("kimi variant index {sub_idx} out of range"))?
            .slug
            .clone();
        if sub_slug == "kimi_coding" {
            mvp::config::ProviderKind::KimiCoding
        } else {
            mvp::config::ProviderKind::Kimi
        }
    } else if selected_slug == "stepfun" {
        let stepfun_options = vec![
            SelectOption {
                label: "Stepfun API".to_owned(),
                slug: "stepfun_api".to_owned(),
                description: "Standard Stepfun chat completion API".to_owned(),
                recommended: true,
            },
            SelectOption {
                label: "Step Plan".to_owned(),
                slug: "step_plan".to_owned(),
                description: "Step Plan for specialized tasks".to_owned(),
                recommended: false,
            },
        ];
        print_lines(ui, vec!["Select the Stepfun variant:".to_owned()])?;
        let stepfun_default_idx = Some(usize::from(
            default_provider_kind == mvp::config::ProviderKind::StepPlan,
        ));
        let sub_idx = ui.select_one(
            "Stepfun variant",
            &stepfun_options,
            stepfun_default_idx,
            SelectInteractionMode::List,
        )?;
        let sub_slug = stepfun_options
            .get(sub_idx)
            .ok_or_else(|| format!("stepfun variant index {sub_idx} out of range"))?
            .slug
            .clone();
        if sub_slug == "step_plan" {
            mvp::config::ProviderKind::StepPlan
        } else {
            mvp::config::ProviderKind::Stepfun
        }
    } else {
        provider_kinds
            .iter()
            .find(|kind| provider_kind_id(**kind) == selected_slug)
            .copied()
            .ok_or_else(|| format!("provider kind not found for slug {}", selected_slug))?
    };

    let mut provider_config =
        resolve_provider_config_from_selection(&config.provider, provider_selection, kind);

    if let Some(region_info) = kind.region_endpoint_info() {
        let configured_base_url = provider_config.base_url.as_str();
        let default_region_idx = region_info
            .variants
            .iter()
            .position(|variant| variant.base_url == configured_base_url)
            .unwrap_or(0);
        let region_options = region_info
            .variants
            .iter()
            .enumerate()
            .map(|(index, variant)| {
                let is_default_variant = index == 0;
                let label = if is_default_variant {
                    format!("{} (default)", variant.label)
                } else {
                    variant.label.to_owned()
                };
                let slug = variant.base_url.to_owned();
                let description = format!("endpoint: {}", variant.base_url);
                let recommended = index == default_region_idx;
                SelectOption {
                    label,
                    slug,
                    description,
                    recommended,
                }
            })
            .collect::<Vec<_>>();
        let region_prompt = format!("Select the {} region endpoint:", region_info.family_label);
        print_lines(ui, vec![region_prompt])?;
        let region_idx = ui.select_one(
            "Region",
            &region_options,
            Some(default_region_idx),
            SelectInteractionMode::List,
        )?;
        let selected_base_url = region_options
            .get(region_idx)
            .ok_or_else(|| format!("region selection index {region_idx} out of range"))?
            .slug
            .clone();
        provider_config.set_base_url(selected_base_url);
    }

    prompt_provider_base_url_if_needed(options, kind, &mut provider_config, ui)?;

    Ok(provider_config)
}

fn prompt_provider_base_url_if_needed(
    options: &OnboardCommandOptions,
    kind: mvp::config::ProviderKind,
    provider_config: &mut mvp::config::ProviderConfig,
    ui: &mut impl OnboardUi,
) -> CliResult<()> {
    let requires_custom_base_url = kind.requires_custom_base_url();
    if !requires_custom_base_url || options.non_interactive {
        return Ok(());
    }

    let configured_base_url = provider_config.base_url.trim().to_owned();
    let has_configured_base_url = !configured_base_url.is_empty();
    let has_unresolved_custom_base_url = provider_config.has_unresolved_custom_base_url();
    let prompt_lines = build_provider_base_url_prompt_lines(
        kind,
        configured_base_url.as_str(),
        has_unresolved_custom_base_url,
    );
    print_lines(ui, prompt_lines)?;

    let selected_base_url = if has_unresolved_custom_base_url || !has_configured_base_url {
        ui.prompt_required("Provider base URL")?
    } else {
        ui.prompt_with_default("Provider base URL", configured_base_url.as_str())?
    };
    let validated_base_url = validate_onboard_provider_base_url(selected_base_url.as_str())?;
    provider_config.set_base_url(validated_base_url);

    Ok(())
}

fn build_provider_base_url_prompt_lines(
    kind: mvp::config::ProviderKind,
    configured_base_url: &str,
    has_unresolved_custom_base_url: bool,
) -> Vec<String> {
    let mut lines = Vec::new();
    let provider_label = provider_kind_display_name(kind);
    let intro_line = format!("Set the {} API base URL:", provider_label);
    lines.push(intro_line);

    if let Some(configuration_hint) = kind.configuration_hint() {
        lines.push(configuration_hint.to_owned());
    }

    if has_unresolved_custom_base_url {
        let template_line = format!("Current template: {}", configured_base_url.trim());
        lines.push(template_line);
    }

    lines
}

fn validate_onboard_provider_base_url(raw: &str) -> CliResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("provider base URL cannot be empty".to_owned());
    }

    let parsed_url = reqwest::Url::parse(trimmed)
        .map_err(|error| format!("provider base URL is invalid: {error}"))?;
    let scheme = parsed_url.scheme();
    let valid_scheme = scheme == "http" || scheme == "https";
    if !valid_scheme {
        return Err("provider base URL must use http or https".to_owned());
    }

    let has_host = parsed_url.host_str().is_some();
    if !has_host {
        return Err("provider base URL must include a host".to_owned());
    }

    Ok(trimmed.to_owned())
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
    config: &mvp::config::LoongConfig,
    guided_prompt_path: GuidedPromptPath,
    available_models: &[String],
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<String> {
    let prompt_default = onboarding_model_policy::resolve_onboarding_model_prompt_default(
        &config.provider,
        options.model.as_deref(),
    )?;

    if options.non_interactive {
        return Ok(prompt_default);
    }

    print_lines(
        ui,
        render_model_selection_screen_lines_with_style(
            config,
            prompt_default.as_str(),
            guided_prompt_path,
            context.render_width,
            true,
            !available_models.is_empty(),
        ),
    )?;
    if !available_models.is_empty() {
        // When we render the model catalog choices from a static provider list,
        // we still compute `prompt_default` (often `auto`) for the prompt UI.
        // Hide `auto` from the selectable catalog to match operator expectations.
        let hide_prompt_default_from_catalog = prompt_default.trim().eq_ignore_ascii_case("auto")
            && is_volcengine_coding_plan_domestic_static_catalog(&config.provider);

        let effective_prompt_default = if hide_prompt_default_from_catalog {
            ""
        } else {
            prompt_default.as_str()
        };

        let catalog_choices = onboarding_model_policy::onboarding_model_catalog_choices(
            effective_prompt_default,
            available_models,
        );
        let (select_options, default_idx) = build_model_selection_options(&catalog_choices);
        let idx = ui.select_one(
            "Model",
            &select_options,
            default_idx,
            SelectInteractionMode::Search,
        )?;
        let selected = select_options
            .get(idx)
            .ok_or_else(|| format!("model selection index {idx} out of range"))?;
        if selected.slug != ONBOARD_CUSTOM_MODEL_OPTION_SLUG {
            return Ok(selected.slug.clone());
        }
        let custom_model = ui.prompt_with_default("Custom model id", effective_prompt_default)?;
        let trimmed = custom_model.trim();
        if trimmed.is_empty() {
            return Err("model cannot be empty".to_owned());
        }
        return Ok(trimmed.to_owned());
    }
    let value = ui.prompt_with_default("Model", prompt_default.as_str())?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("model cannot be empty".to_owned());
    }
    Ok(trimmed.to_owned())
}

async fn load_onboarding_model_catalog(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongConfig,
) -> Vec<String> {
    // Volcano Engine "Coding Plan" domestic endpoint has a stable, operator-provided model list.
    // Using it avoids an interactive onboarding dependency on `GET /models`.
    if is_volcengine_coding_plan_domestic_static_catalog(&config.provider) {
        return vec![
            // Keep the historical default model id as an explicit choice.
            "ark-code-latest".to_owned(),
            "doubao-seed-2.0-code".to_owned(),
            "doubao-seed-2.0-pro".to_owned(),
            "doubao-seed-2.0-lite".to_owned(),
            "doubao-seed-code".to_owned(),
            "minimax-m2.5".to_owned(),
            "glm-4.7".to_owned(),
            "deepseek-v3.2".to_owned(),
            "kimi-k2.5".to_owned(),
        ];
    }

    if options.non_interactive || options.skip_model_probe {
        return Vec::new();
    }
    let has_provider_credentials = mvp::provider::provider_auth_ready(config).await;
    let provider_requires_explicit_auth = config.provider.requires_explicit_auth_configuration();
    if !has_provider_credentials && provider_requires_explicit_auth {
        return Vec::new();
    }
    mvp::provider::fetch_available_models(config)
        .await
        .unwrap_or_default()
}

fn is_volcengine_coding_plan_domestic_static_catalog(
    provider: &mvp::config::ProviderConfig,
) -> bool {
    if provider.kind != mvp::config::ProviderKind::VolcengineCoding {
        return false;
    }

    let Ok(actual_url) = reqwest::Url::parse(provider.resolved_base_url().trim()) else {
        return false;
    };
    let Ok(canonical_url) = reqwest::Url::parse(
        mvp::config::ProviderKind::VolcengineCoding
            .profile()
            .base_url,
    ) else {
        return false;
    };

    actual_url.scheme() == canonical_url.scheme()
        && actual_url.host_str() == canonical_url.host_str()
        && actual_url.port_or_known_default() == canonical_url.port_or_known_default()
        && actual_url.path().trim_end_matches('/') == canonical_url.path().trim_end_matches('/')
}

#[cfg(test)]
mod volcengine_coding_plan_catalog_tests {
    use super::*;

    #[test]
    fn volcengine_coding_plan_domestic_static_catalog_detects_cn_beijing_coding_v3() {
        let provider = mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::VolcengineCoding,
            base_url: "https://ark.cn-beijing.volces.com/api/coding/v3".to_owned(),
            ..mvp::config::ProviderConfig::default()
        };

        assert!(is_volcengine_coding_plan_domestic_static_catalog(&provider));
    }

    #[test]
    fn volcengine_coding_plan_domestic_static_catalog_rejects_non_coding_plan_endpoints() {
        let provider = mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::VolcengineCoding,
            base_url: "https://ark.cn-beijing.volces.com/api/v3".to_owned(),
            ..mvp::config::ProviderConfig::default()
        };

        assert!(!is_volcengine_coding_plan_domestic_static_catalog(
            &provider
        ));
    }

    #[test]
    fn volcengine_coding_plan_domestic_static_catalog_rejects_proxy_path() {
        let provider = mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::VolcengineCoding,
            base_url: "https://proxy.example.com/api/coding/v3".to_owned(),
            ..mvp::config::ProviderConfig::default()
        };

        assert!(!is_volcengine_coding_plan_domestic_static_catalog(
            &provider
        ));
    }
}

fn build_model_selection_options(
    catalog_choices: &onboarding_model_policy::OnboardingModelCatalogChoices,
) -> (Vec<SelectOption>, Option<usize>) {
    let default_idx = catalog_choices.default_index;
    let mut options = Vec::new();

    for (index, model) in catalog_choices.ordered_models.iter().enumerate() {
        let is_default_model = default_idx == Some(index);
        let description = if is_default_model {
            "current or suggested default".to_owned()
        } else {
            String::new()
        };

        let option = SelectOption {
            label: model.clone(),
            slug: model.clone(),
            description,
            recommended: is_default_model,
        };
        options.push(option);
    }

    options.push(SelectOption {
        label: "enter custom model id".to_owned(),
        slug: ONBOARD_CUSTOM_MODEL_OPTION_SLUG.to_owned(),
        description: "manually type any provider model id".to_owned(),
        recommended: false,
    });

    (options, default_idx)
}

fn resolve_api_key_env_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongConfig,
    default_api_key_env: String,
    guided_prompt_path: GuidedPromptPath,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<String> {
    let explicit_selection = if let Some(api_key_env) = options.api_key_env.as_deref() {
        if is_explicit_onboard_clear_input(api_key_env) {
            return Ok(String::new());
        }
        let trimmed = api_key_env.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(validate_selected_provider_credential_env(config, trimmed)?)
        }
    } else {
        None
    };

    if options.non_interactive {
        return Ok(explicit_selection.unwrap_or(default_api_key_env));
    }
    let initial = explicit_selection
        .as_deref()
        .unwrap_or(default_api_key_env.as_str());
    let example_env_name =
        provider_credential_policy::provider_credential_env_hint(&config.provider)
            .unwrap_or_else(|| "PROVIDER_API_KEY".to_owned());
    loop {
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
        let value = ui.prompt_with_default("Credential env var name", initial)?;
        if is_explicit_onboard_clear_input(&value) {
            return Ok(String::new());
        }
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Ok(String::new());
        }
        match validate_selected_provider_credential_env(config, trimmed) {
            Ok(validated) => return Ok(validated),
            Err(error) => {
                print_message(ui, error)?;
                print_message(
                    ui,
                    format!(
                        "enter the environment variable name only, for example {example_env_name}, or type :clear to remove the env binding"
                    ),
                )?;
            }
        }
    }
}

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

fn apply_selected_system_prompt(
    config: &mut mvp::config::LoongConfig,
    system_prompt: Option<String>,
) {
    match system_prompt.as_deref().map(str::trim) {
        Some(value) if !value.is_empty() => {
            config.cli.prompt_pack_id = Some(String::new());
            config.cli.personality = None;
            config.cli.system_prompt_addendum = None;
            config.cli.system_prompt = value.to_owned();
        }
        _ => {
            config.cli.prompt_pack_id = Some(mvp::prompt::DEFAULT_PROMPT_PACK_ID.to_owned());
            config.cli.personality = Some(mvp::prompt::PromptPersonality::default());
            config.cli.refresh_native_system_prompt();
        }
    }
}

async fn resolve_web_search_provider_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongConfig,
    guided_prompt_path: GuidedPromptPath,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<String> {
    let explicit_override = explicit_web_search_provider_override(options)?;
    if mvp::provider::native_query_search_active(config) && explicit_override.is_none() {
        return Ok(current_web_search_provider(config).to_owned());
    }

    let recommendation = resolve_web_search_provider_recommendation(options, config).await?;
    let recommended_provider = recommendation.provider;
    let default_provider =
        resolve_effective_web_search_default_provider(options, config, &recommendation);

    if options.non_interactive {
        return Ok(default_provider.to_owned());
    }

    let screen_options = build_web_search_provider_screen_options(config, recommended_provider);
    let select_options = select_options_from_screen_options(&screen_options);
    let default_idx = screen_options
        .iter()
        .position(|option| option.key == default_provider);

    print_lines(
        ui,
        render_web_search_provider_selection_screen_lines_with_style(
            config,
            recommended_provider,
            default_provider,
            recommendation.reason.as_str(),
            guided_prompt_path,
            context.render_width,
            true,
        ),
    )?;
    let idx = ui.select_one(
        crate::access_terms::QUERY_SEARCH_PROVIDER_LABEL,
        &select_options,
        default_idx,
        SelectInteractionMode::List,
    )?;
    let selected = select_options
        .get(idx)
        .ok_or_else(|| crate::access_terms::query_search_provider_selection_index_error(idx))?;
    Ok(selected.slug.clone())
}

fn resolve_web_search_credential_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongConfig,
    provider: &str,
    guided_prompt_path: GuidedPromptPath,
    non_interactive: bool,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<WebSearchCredentialSelection> {
    let explicit_override = explicit_web_search_provider_override(options)?;
    if mvp::provider::native_query_search_active(config) && explicit_override.is_none() {
        return Ok(WebSearchCredentialSelection::KeepCurrent);
    }

    let Some(descriptor) = mvp::config::web_search_provider_descriptor(provider) else {
        return Ok(WebSearchCredentialSelection::KeepCurrent);
    };
    if !descriptor.requires_api_key {
        return Ok(WebSearchCredentialSelection::KeepCurrent);
    }

    let explicit_selection = if let Some(raw_env_name) = options.web_search_api_key_env.as_deref() {
        if is_explicit_onboard_clear_input(raw_env_name) {
            return Ok(WebSearchCredentialSelection::ClearConfigured);
        }

        let trimmed_env_name = raw_env_name.trim();
        if trimmed_env_name.is_empty() {
            None
        } else {
            let validated_env_name =
                validate_selected_web_search_credential_env(provider, trimmed_env_name)?;
            Some(validated_env_name)
        }
    } else {
        None
    };

    let prompt_default = preferred_query_search_credential_env_default(config, provider);
    if non_interactive {
        if let Some(explicit_env_name) = explicit_selection {
            return Ok(WebSearchCredentialSelection::UseEnv(explicit_env_name));
        }

        return Ok(if prompt_default.trim().is_empty() {
            WebSearchCredentialSelection::KeepCurrent
        } else {
            WebSearchCredentialSelection::UseEnv(prompt_default)
        });
    }

    let initial_value = explicit_selection
        .as_deref()
        .unwrap_or(prompt_default.as_str());
    let example_env_name = descriptor
        .default_api_key_env
        .or_else(|| descriptor.api_key_env_names.first().copied())
        .unwrap_or("WEB_SEARCH_API_KEY")
        .to_owned();
    loop {
        print_lines(
            ui,
            render_web_search_credential_selection_screen_lines_with_style(
                config,
                provider,
                initial_value,
                guided_prompt_path,
                context.render_width,
                true,
            ),
        )?;
        let value = ui.prompt_with_default(
            crate::access_terms::query_search_credential_prompt_label(),
            initial_value,
        )?;
        if is_explicit_onboard_clear_input(&value) {
            return Ok(WebSearchCredentialSelection::ClearConfigured);
        }
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Ok(WebSearchCredentialSelection::KeepCurrent);
        }
        match validate_selected_web_search_credential_env(provider, trimmed) {
            Ok(validated) => return Ok(WebSearchCredentialSelection::UseEnv(validated)),
            Err(error) => {
                print_message(ui, error)?;
                print_message(
                    ui,
                    crate::access_terms::query_search_credential_input_hint(
                        example_env_name.as_str(),
                    ),
                )?;
            }
        }
    }
}

fn build_web_search_provider_screen_options(
    config: &mvp::config::LoongConfig,
    recommended_provider: &str,
) -> Vec<OnboardScreenOption> {
    mvp::config::web_search_provider_descriptors()
        .iter()
        .map(|descriptor| {
            let mut detail_lines = vec![descriptor.description.to_owned()];
            if let Some(credential) = summarize_query_search_credential(config, descriptor.id) {
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

fn render_web_search_provider_selection_screen_lines_with_style(
    config: &mvp::config::LoongConfig,
    recommended_provider: &str,
    default_provider: &str,
    recommendation_reason: &str,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let current_provider = current_web_search_provider(config);
    let current_provider_label = query_search_provider_display_name(current_provider);
    let recommended_provider_label = query_search_provider_display_name(recommended_provider);
    let default_provider_label = query_search_provider_display_name(default_provider);
    let options = build_web_search_provider_screen_options(config, recommended_provider);
    let default_footer_description = if default_provider == current_provider {
        format!("keep {current_provider_label}")
    } else {
        format!("use {default_provider_label}")
    };

    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        crate::access_terms::CHOOSE_QUERY_SEARCH_TITLE,
        crate::access_terms::CHOOSE_QUERY_SEARCH_PROVIDER_TITLE,
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

fn onboard_credential_env_name_is_safe(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }

    let mut config = mvp::config::LoongConfig::default();
    config.provider.api_key = Some(SecretRef::Env {
        env: trimmed.to_owned(),
    });
    config.provider.api_key_env = None;

    config.validate().is_ok()
}

fn normalize_onboard_credential_env_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let is_empty = trimmed.is_empty();
    if is_empty {
        return None;
    }

    let is_safe = onboard_credential_env_name_is_safe(trimmed);
    if !is_safe {
        return None;
    }

    Some(trimmed.to_owned())
}

fn validate_selected_web_search_credential_env(
    provider: &str,
    selected_env_name: &str,
) -> CliResult<String> {
    let trimmed = selected_env_name.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }

    if let Some(normalized) = normalize_onboard_credential_env_name(trimmed) {
        return Ok(normalized);
    }

    let example_env_name = mvp::config::web_search_provider_descriptor(provider)
        .and_then(|descriptor| {
            descriptor
                .default_api_key_env
                .or_else(|| descriptor.api_key_env_names.first().copied())
        })
        .unwrap_or("WEB_SEARCH_API_KEY");

    Err(crate::access_terms::query_search_credential_source_validation_error(example_env_name))
}

fn apply_selected_web_search_credential(
    config: &mut mvp::config::LoongConfig,
    provider: &str,
    selection: WebSearchCredentialSelection,
) -> CliResult<()> {
    let next_value = match selection {
        WebSearchCredentialSelection::KeepCurrent => return Ok(()),
        WebSearchCredentialSelection::ClearConfigured => None,
        WebSearchCredentialSelection::UseEnv(env_name) => Some(format!("${{{}}}", env_name.trim())),
    };

    let updated = config
        .tools
        .web_search
        .set_configured_api_key_for_provider(provider, next_value);

    if !updated {
        let message =
            format!("unsupported web.search provider `{provider}`; credential update was skipped");
        return Err(message);
    }

    Ok(())
}

fn validate_selected_provider_credential_env(
    config: &mvp::config::LoongConfig,
    selected_env_name: &str,
) -> CliResult<String> {
    let trimmed = selected_env_name.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }

    let mut candidate = config.clone();
    apply_selected_api_key_env(&mut candidate.provider, trimmed.to_owned());
    candidate.validate().map(|_| trimmed.to_owned())
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
pub fn preferred_api_key_env_default(config: &mvp::config::LoongConfig) -> String {
    provider_credential_policy::preferred_provider_credential_env_name(config)
}

pub fn collect_import_surfaces(config: &mvp::config::LoongConfig) -> Vec<ImportSurface> {
    crate::migration::collect_import_surfaces(config)
        .into_iter()
        .map(import_surface_from_migration)
        .collect()
}

pub fn collect_import_surfaces_with_channel_readiness(
    config: &mvp::config::LoongConfig,
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
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<StartingConfigSelection> {
    let state = prepare_import_starting_state(
        output_path,
        &context.codex_config_paths,
        context.workspace_root.as_deref(),
    )?;

    if state.current_candidate.is_none() && state.import_candidates.is_empty() {
        return Ok(default_starting_config_selection());
    }

    if options.non_interactive {
        return Ok(select_non_interactive_starting_config_from_state(&state));
    }

    if state
        .entry_options
        .first()
        .is_some_and(|option| option.choice == OnboardEntryChoice::StartFresh)
    {
        return Ok(default_starting_config_selection());
    }

    print_onboard_entry_options(
        ui,
        state.current_setup_state,
        state.current_candidate.as_ref(),
        &state.import_candidates,
        &state.entry_options,
        context,
    )?;

    match prompt_onboard_entry_choice(ui, &state.entry_options)? {
        OnboardEntryChoice::ContinueCurrentSetup => Ok(state
            .current_candidate
            .clone()
            .map(|candidate| {
                crate::onboard_import::starting_config_selection_from_current_candidate(
                    candidate,
                    state.current_setup_state,
                )
            })
            .unwrap_or_else(default_starting_config_selection)),
        OnboardEntryChoice::ImportDetectedSetup => select_interactive_import_starting_config(
            ui,
            context,
            state.current_setup_state,
            state.import_candidates.clone(),
            &state.all_candidates,
        ),
        OnboardEntryChoice::StartFresh => Ok(default_starting_config_selection()),
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
        render_onboard_entry_interactive_screen_lines_with_style(
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


fn prompt_import_candidate_choice(
    ui: &mut impl OnboardUi,
    candidates: &[ImportCandidate],
    width: usize,
) -> CliResult<Option<usize>> {
    let screen_options = build_starting_point_selection_screen_options(candidates, width);
    let idx = select_screen_option(ui, "Starting point", &screen_options, Some("1"))?;
    let selected = screen_options
        .get(idx)
        .ok_or_else(|| format!("starting point selection index {idx} out of range"))?;
    if selected.key == "0" {
        return Ok(None);
    }
    selected
        .key
        .parse::<usize>()
        .map(|value| Some(value - 1))
        .map_err(|error| {
            format!(
                "invalid starting point selection key {}: {error}",
                selected.key
            )
        })
}

fn prompt_onboard_shortcut_choice(
    ui: &mut impl OnboardUi,
    shortcut_kind: OnboardShortcutKind,
) -> CliResult<OnboardShortcutChoice> {
    let options = build_onboard_shortcut_screen_options(shortcut_kind);
    match select_screen_option(ui, "Your choice", &options, Some("1"))? {
        0 => Ok(OnboardShortcutChoice::UseShortcut),
        1 => Ok(OnboardShortcutChoice::AdjustSettings),
        idx => Err(format!("shortcut selection index {idx} out of range")),
    }
}

pub fn detect_import_starting_config_with_channel_readiness(
    readiness: ChannelImportReadiness,
) -> mvp::config::LoongConfig {
    crate::migration::detect_import_starting_config_with_channel_readiness(to_migration_readiness(
        readiness,
    ))
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

fn detect_render_width() -> usize {
    mvp::presentation::detect_render_width()
}

fn enabled_channel_ids(config: &mvp::config::LoongConfig) -> Vec<String> {
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

fn secret_ref_has_inline_literal(secret_ref: Option<&SecretRef>) -> bool {
    let Some(secret_ref) = secret_ref else {
        return false;
    };

    secret_ref.inline_literal_value().is_some()
}

fn onboard_has_explicit_overrides(options: &OnboardCommandOptions) -> bool {
    option_has_non_empty_value(options.provider.as_deref())
        || option_has_non_empty_value(options.model.as_deref())
        || option_has_non_empty_value(options.api_key_env.as_deref())
        || option_has_non_empty_value(options.web_search_provider.as_deref())
        || option_has_non_empty_value(options.web_search_api_key_env.as_deref())
        || option_has_non_empty_value(options.personality.as_deref())
        || option_has_non_empty_value(options.memory_profile.as_deref())
        || option_has_non_empty_value(options.system_prompt.as_deref())
        || option_has_non_empty_value(env::var("LOONG_WEB_SEARCH_PROVIDER").ok().as_deref())
}

fn option_has_non_empty_value(raw: Option<&str>) -> bool {
    raw.is_some_and(|value| !value.trim().is_empty())
}

fn load_existing_output_config(output_path: &Path) -> Option<mvp::config::LoongConfig> {
    let path_str = output_path.to_str()?;
    mvp::config::load(Some(path_str))
        .ok()
        .map(|(_, config)| config)
}

pub fn should_skip_config_write(
    existing_config: Option<&mvp::config::LoongConfig>,
    draft: &mvp::config::LoongConfig,
) -> bool {
    existing_config.is_some_and(|existing| existing == draft)
}

pub fn parse_provider_kind(raw: &str) -> Option<mvp::config::ProviderKind> {
    mvp::config::ProviderKind::parse(raw)
}

pub fn parse_prompt_personality(raw: &str) -> Option<mvp::prompt::PromptPersonality> {
    mvp::prompt::parse_prompt_personality(raw)
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
    personality.id()
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

pub fn supported_personality_list() -> String {
    mvp::prompt::supported_prompt_personality_list()
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
        render_existing_config_write_header_lines_with_style(
            &existing_path,
            context.render_width,
            true,
        ),
    )?;
    let options = build_existing_config_write_screen_options();
    let selected = options
        .get(select_screen_option(
            ui,
            "Your choice",
            &options,
            Some("b"),
        )?)
        .ok_or_else(|| "existing-config write selection out of range".to_owned())?;
    match selected.key.as_str() {
        "o" => Ok(ConfigWritePlan {
            force: true,
            backup_path: None,
        }),
        "b" => Ok(ConfigWritePlan {
            force: true,
            backup_path: Some(resolve_backup_path(output_path)?),
        }),
        "c" => Err("onboarding cancelled: config file already exists".to_owned()),
        key => Err(format!(
            "unexpected existing-config write selection key: {key}"
        )),
    }
}

#[cfg(test)]
#[path = "onboard_cli_tests.rs"]
mod tests;
