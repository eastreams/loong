use std::collections::BTreeSet;
use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use dialoguer::console::{Term, user_attended};
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Error as DialoguerError, FuzzySelect, Input, Select};
use kernel::ToolCoreRequest;
use loongclaw_app as mvp;
use loongclaw_contracts::SecretRef;
use loongclaw_spec::CliResult;
use serde_json::json;

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
    explicit_web_search_provider_override,
    recommend_web_search_provider_from_available_credentials,
};
use crate::onboard_web_search::{
    configured_web_search_provider_credential_source_value,
    configured_web_search_provider_env_name, configured_web_search_provider_secret,
    current_web_search_provider, preferred_web_search_credential_env_default,
    resolve_effective_web_search_default_provider, resolve_web_search_provider_recommendation,
    summarize_web_search_provider_credential, web_search_provider_display_name,
    web_search_provider_has_inline_credential,
};
use crate::onboarding_model_policy;
use crate::provider_credential_policy;
use mvp::tui_surface::{
    TuiCalloutTone, TuiChoiceSpec, TuiHeaderStyle, TuiScreenSpec, TuiSectionSpec,
    render_onboard_screen_spec,
};
#[cfg(test)]
use std::fs;
#[cfg(test)]
use time::OffsetDateTime;

pub use crate::onboard_finalize::{
    OnboardingAction, OnboardingActionKind, OnboardingDomainOutcome, OnboardingSuccessSummary,
    backup_existing_config, build_onboarding_success_summary,
    render_onboarding_success_summary_with_width,
};
const ONBOARD_CLEAR_INPUT_TOKEN: &str = ":clear";
const ONBOARD_CUSTOM_MODEL_OPTION_SLUG: &str = "__custom_model__";
const ONBOARD_ESCAPE_CANCEL_HINT: &str = "- press Esc then Enter to cancel onboarding";
const ONBOARD_SINGLE_LINE_INPUT_HINT: &str = "- single-line input only";
const ONBOARD_PASTE_DRAIN_WINDOW_ENV: &str = "LOONGCLAW_ONBOARD_PASTE_DRAIN_WINDOW_MS";
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
        .name("loongclaw-onboard-stdin".to_owned())
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

fn render_select_option_item(option: &SelectOption) -> String {
    let mut rendered = option.label.clone();
    if !option.description.trim().is_empty() {
        rendered.push_str(" - ");
        rendered.push_str(option.description.trim());
    }
    if option.recommended {
        rendered.push_str(" (recommended)");
    }
    rendered
}

fn map_rich_prompt_error(action: &str, error: DialoguerError) -> String {
    let error: io::Error = error.into();
    if error.kind() == io::ErrorKind::Interrupted {
        return "onboarding cancelled: prompt aborted".to_owned();
    }
    format!("{action} failed: {error}")
}

fn prompt_with_default_rich(label: &str, default: &str) -> CliResult<String> {
    let term = rich_prompt_term();
    prompt_with_default_rich_on(&term, label, default)
}

fn prompt_with_default_rich_on(term: &Term, label: &str, default: &str) -> CliResult<String> {
    let theme = rich_prompt_theme();
    let value = Input::<String>::with_theme(&theme)
        .with_prompt(label)
        .default(default.to_owned())
        .report(false)
        .interact_text_on(term)
        .map_err(|error| map_rich_prompt_error("interactive prompt", error))?;
    let value = ensure_onboard_input_not_cancelled(value)?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(default.to_owned());
    }
    Ok(trimmed.to_owned())
}

fn prompt_required_rich(label: &str) -> CliResult<String> {
    let term = rich_prompt_term();
    prompt_required_rich_on(&term, label)
}

fn prompt_required_rich_on(term: &Term, label: &str) -> CliResult<String> {
    let theme = rich_prompt_theme();
    let value = Input::<String>::with_theme(&theme)
        .with_prompt(label)
        .report(false)
        .interact_text_on(term)
        .map_err(|error| map_rich_prompt_error("interactive prompt", error))?;
    let value = ensure_onboard_input_not_cancelled(value)?;
    Ok(value.trim().to_owned())
}

fn prompt_allow_empty_rich(label: &str) -> CliResult<String> {
    let term = rich_prompt_term();
    prompt_allow_empty_rich_on(&term, label)
}

fn prompt_allow_empty_rich_on(term: &Term, label: &str) -> CliResult<String> {
    let theme = rich_prompt_theme();
    let value = Input::<String>::with_theme(&theme)
        .with_prompt(label)
        .allow_empty(true)
        .report(false)
        .interact_text_on(term)
        .map_err(|error| map_rich_prompt_error("interactive prompt", error))?;
    let value = ensure_onboard_input_not_cancelled(value)?;
    Ok(value.trim().to_owned())
}

fn prompt_confirm_rich(message: &str, default: bool) -> CliResult<bool> {
    let term = rich_prompt_term();
    let theme = rich_prompt_theme();
    Confirm::with_theme(&theme)
        .with_prompt(message)
        .default(default)
        .report(false)
        .interact_on_opt(&term)
        .map_err(|error| map_rich_prompt_error("interactive confirmation", error))?
        .ok_or_else(|| "onboarding cancelled: prompt aborted".to_owned())
}

fn select_one_rich(
    label: &str,
    options: &[SelectOption],
    default: Option<usize>,
    interaction_mode: SelectInteractionMode,
) -> CliResult<usize> {
    let default = validate_select_one_state(options.len(), default)?;
    let items = options
        .iter()
        .map(render_select_option_item)
        .collect::<Vec<_>>();
    let term = rich_prompt_term();
    let theme = rich_prompt_theme();
    let selection = match interaction_mode {
        SelectInteractionMode::List => {
            let prompt = Select::with_theme(&theme)
                .with_prompt(label)
                .items(&items)
                .report(false);
            let prompt = if let Some(idx) = default {
                prompt.default(idx)
            } else {
                prompt
            };
            prompt
                .interact_on_opt(&term)
                .map_err(|error| map_rich_prompt_error("interactive selection", error))?
        }
        SelectInteractionMode::Search => {
            let prompt = FuzzySelect::with_theme(&theme)
                .with_prompt(label)
                .items(&items)
                .report(false);
            let prompt = if let Some(idx) = default {
                prompt.default(idx)
            } else {
                prompt
            };
            prompt
                .interact_on_opt(&term)
                .map_err(|error| map_rich_prompt_error("interactive model search", error))?
        }
    };
    selection.ok_or_else(|| "onboarding cancelled: prompt aborted".to_owned())
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
    ui: &mut impl OnboardUi,
    label: &str,
    options: &[OnboardScreenOption],
    default_key: Option<&str>,
) -> CliResult<usize> {
    let select_options = select_options_from_screen_options(options);
    let default_idx =
        default_key.and_then(|key| options.iter().position(|option| option.key == key));
    ui.select_one(
        label,
        &select_options,
        default_idx,
        SelectInteractionMode::List,
    )
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
        label: crate::onboard_presentation::start_fresh_option_label().to_owned(),
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
            detail_lines: vec![crate::onboard_presentation::shortcut_continue_detail().to_owned()],
            recommended: true,
        },
        OnboardScreenOption {
            key: "2".to_owned(),
            label: crate::onboard_presentation::adjust_settings_label().to_owned(),
            detail_lines: vec![crate::onboard_presentation::shortcut_adjust_detail().to_owned()],
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

fn resolve_select_one_eof(default: Option<usize>) -> CliResult<usize> {
    default.ok_or_else(|| {
        "onboarding cancelled: stdin closed while waiting for required selection".to_owned()
    })
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

fn prompt_optional(
    ui: &mut impl OnboardUi,
    label: &str,
    current: Option<&str>,
) -> CliResult<Option<String>> {
    let value = ui.prompt_allow_empty(label)?;
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

fn onboarding_default_external_skills_install_root(output_path: &Path) -> PathBuf {
    let base_dir = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    base_dir.join("external-skills-installed")
}

fn apply_selected_preinstalled_skills_to_config(
    config: &mut mvp::config::LoongClawConfig,
    output_path: &Path,
    selected_skill_ids: &[String],
) {
    if selected_skill_ids.is_empty() {
        return;
    }
    config.external_skills.enabled = true;
    config.external_skills.auto_expose_installed = true;
    if config.external_skills.install_root.is_none() {
        config.external_skills.install_root = Some(
            onboarding_default_external_skills_install_root(output_path)
                .display()
                .to_string(),
        );
    }
}

fn install_root_for_onboarded_skills(
    config: &mvp::config::LoongClawConfig,
    config_path: &Path,
) -> PathBuf {
    config
        .external_skills
        .resolved_install_root()
        .unwrap_or_else(|| onboarding_default_external_skills_install_root(config_path))
}

fn install_selected_preinstalled_skills(
    config_path: &Path,
    config: &mvp::config::LoongClawConfig,
    selected_skill_ids: &[String],
) -> CliResult<()> {
    if selected_skill_ids.is_empty() {
        return Ok(());
    }

    let install_root = install_root_for_onboarded_skills(config, config_path);
    let tool_runtime_config = mvp::tools::runtime_config::ToolRuntimeConfig::from_loongclaw_config(
        config,
        Some(config_path),
    );
    let mut installed_now = Vec::new();

    for skill_id in selected_skill_ids {
        if install_root.join(skill_id).join("SKILL.md").is_file() {
            continue;
        }
        let request = ToolCoreRequest {
            tool_name: "external_skills.install".to_owned(),
            payload: json!({
                "bundled_skill_id": skill_id,
                "replace": false,
            }),
        };
        if let Err(error) = mvp::tools::execute_tool_core_with_config(request, &tool_runtime_config)
        {
            for installed_skill_id in installed_now.iter().rev() {
                let _ = mvp::tools::execute_tool_core_with_config(
                    ToolCoreRequest {
                        tool_name: "external_skills.remove".to_owned(),
                        payload: json!({
                            "skill_id": installed_skill_id,
                        }),
                    },
                    &tool_runtime_config,
                );
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
            (GuidedPromptPath::NativePromptPack, GuidedOnboardStep::Review) => 8,
            (GuidedPromptPath::InlineOverride, GuidedOnboardStep::PromptCustomization) => 4,
            (GuidedPromptPath::InlineOverride, GuidedOnboardStep::MemoryProfile) => 5,
            (GuidedPromptPath::InlineOverride, GuidedOnboardStep::Review) => 7,
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
        );
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

    Ok(provider_config)
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
    config: &mvp::config::LoongClawConfig,
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
    config: &mvp::config::LoongClawConfig,
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

    let personalities = [
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
    ];
    let select_options: Vec<SelectOption> = personalities
        .iter()
        .map(|(p, label, desc)| SelectOption {
            label: label.to_string(),
            slug: prompt_personality_id(*p).to_owned(),
            description: desc.to_string(),
            recommended: *p == default_personality,
        })
        .collect();
    let default_idx = personalities
        .iter()
        .position(|(p, _, _)| *p == default_personality);

    print_lines(
        ui,
        render_personality_selection_header_lines(config, context.render_width),
    )?;
    let idx = ui.select_one(
        "Personality",
        &select_options,
        default_idx,
        SelectInteractionMode::List,
    )?;
    let (personality, _, _) = personalities
        .get(idx)
        .ok_or_else(|| format!("personality selection index {idx} out of range"))?;
    Ok(*personality)
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
        "Prompt addendum",
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
    let select_options: Vec<SelectOption> = MEMORY_PROFILE_CHOICES
        .iter()
        .map(|(p, label, desc)| SelectOption {
            label: label.to_string(),
            slug: memory_profile_id(*p).to_owned(),
            description: desc.to_string(),
            recommended: *p == default_profile,
        })
        .collect();
    let default_idx = MEMORY_PROFILE_CHOICES
        .iter()
        .position(|(p, _, _)| *p == default_profile);

    print_lines(
        ui,
        render_memory_profile_selection_header_lines(
            config,
            guided_prompt_path,
            context.render_width,
        ),
    )?;
    let idx = ui.select_one(
        "Memory profile",
        &select_options,
        default_idx,
        SelectInteractionMode::List,
    )?;
    let (profile, _, _) = MEMORY_PROFILE_CHOICES
        .get(idx)
        .ok_or_else(|| format!("memory profile selection index {idx} out of range"))?;
    Ok(*profile)
}

async fn resolve_web_search_provider_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
    guided_prompt_path: GuidedPromptPath,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<String> {
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
        "Web search provider",
        &select_options,
        default_idx,
        SelectInteractionMode::List,
    )?;
    let selected = select_options
        .get(idx)
        .ok_or_else(|| format!("web search provider selection index {idx} out of range"))?;
    Ok(selected.slug.clone())
}

fn resolve_web_search_credential_selection(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
    provider: &str,
    guided_prompt_path: GuidedPromptPath,
    non_interactive: bool,
    ui: &mut impl OnboardUi,
    context: &OnboardRuntimeContext,
) -> CliResult<WebSearchCredentialSelection> {
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

    let prompt_default = preferred_web_search_credential_env_default(config, provider);
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
        let value = ui.prompt_with_default("Web search credential env var name", initial_value)?;
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
                    format!(
                        "enter the environment variable name only, for example {example_env_name}, or type :clear to remove the configured web search credential"
                    ),
                )?;
            }
        }
    }
}

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

fn onboard_credential_env_name_is_safe(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }

    let mut config = mvp::config::LoongClawConfig::default();
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

    Err(format!(
        "web search credential source must be an environment variable name like {example_env_name}"
    ))
}

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

fn validate_selected_provider_credential_env(
    config: &mvp::config::LoongClawConfig,
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
        title: Some(crate::onboard_presentation::detected_settings_section_heading().to_owned()),
        lines: detected_settings_lines,
    };

    let mut sections = vec![detected_settings_section];

    if !options.is_empty() {
        let entry_choice_section = TuiSectionSpec::Narrative {
            title: Some(crate::onboard_presentation::entry_choice_section_heading().to_owned()),
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
    let screen_options = build_onboard_entry_screen_options(options);
    let default_key = screen_options
        .iter()
        .find(|option| option.recommended)
        .map(|option| option.key.as_str())
        .or_else(|| screen_options.first().map(|option| option.key.as_str()));
    let idx = select_screen_option(ui, "Setup path", &screen_options, default_key)?;
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
    let Some(index) = prompt_import_candidate_choice(ui, &import_candidates, context.render_width)?
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
        crate::onboard_presentation::single_detected_starting_point_preview_subtitle(),
        crate::onboard_presentation::single_detected_starting_point_preview_title(),
        None,
        intro_lines,
        Vec::new(),
        vec![
            crate::onboard_presentation::single_detected_starting_point_preview_footer().to_owned(),
        ],
        false,
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
        render_starting_point_selection_header_lines_with_style(
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

#[cfg(test)]
pub(crate) fn render_onboard_wrapped_display_lines<I, S>(
    display_lines: I,
    width: usize,
) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    display_lines
        .into_iter()
        .flat_map(|line| mvp::presentation::render_wrapped_display_line(line.as_ref(), width))
        .collect()
}

#[cfg(test)]
pub(crate) fn render_onboard_option_lines(
    options: &[OnboardScreenOption],
    width: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    for option in options {
        let suffix = if option.recommended {
            " (recommended)"
        } else {
            ""
        };
        let prefix = render_onboard_option_prefix(&option.key);
        let continuation = " ".repeat(prefix.chars().count());
        lines.extend(
            mvp::presentation::render_wrapped_text_line_with_continuation(
                &prefix,
                &continuation,
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

pub(crate) fn render_default_choice_footer_line(key: &str, description: &str) -> String {
    format!("press Enter to use default {key}, {description}")
}

fn render_prompt_with_default_text(label: &str, default: &str) -> String {
    format!("{label} (default: {default}): ")
}

#[cfg(test)]
fn render_onboard_option_prefix(key: &str) -> String {
    format!("{key}) ")
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
    let prompt_default =
        provider_credential_policy::render_provider_credential_source_value(Some(prompt_default))
            .unwrap_or_default();
    let suggested_env =
        provider_credential_policy::render_provider_credential_source_value(Some(suggested_env))
            .unwrap_or_default();
    let current_env =
        provider_credential_policy::configured_provider_credential_env_binding(&config.provider)
            .and_then(|binding| {
                provider_credential_policy::render_provider_credential_source_value(Some(
                    binding.env_name.as_str(),
                ))
            });

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

fn render_web_search_credential_selection_default_hint_line(
    config: &mvp::config::LoongClawConfig,
    provider: &str,
    prompt_default: &str,
) -> String {
    let prompt_default =
        provider_credential_policy::render_provider_credential_source_value(Some(prompt_default))
            .unwrap_or_default();
    let suggested_env = mvp::config::web_search_provider_descriptor(provider)
        .and_then(|descriptor| descriptor.default_api_key_env)
        .and_then(|env_name| {
            provider_credential_policy::render_provider_credential_source_value(Some(env_name))
        })
        .unwrap_or_default();
    let current_env =
        configured_web_search_provider_env_name(config, provider).and_then(|env_name| {
            provider_credential_policy::render_provider_credential_source_value(Some(
                env_name.as_str(),
            ))
        });

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

pub(crate) fn append_escape_cancel_hint(mut lines: Vec<String>) -> Vec<String> {
    if !lines.iter().any(|line| {
        let lower = line.to_ascii_lowercase();
        lower.contains("esc") && lower.contains("cancel")
    }) {
        lines.push(ONBOARD_ESCAPE_CANCEL_HINT.to_owned());
    }
    lines
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
    show_escape_cancel_hint: bool,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_onboard_choice_screen_spec(
        header_style,
        subtitle,
        title,
        step,
        intro_lines,
        options,
        footer_lines,
        show_escape_cancel_hint,
    );

    render_onboard_screen_spec(&spec, width, color_enabled)
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
    let spec =
        build_onboard_input_screen_spec(title, step, guided_prompt_path, context_lines, hint_lines);

    render_onboard_screen_spec(&spec, width, color_enabled)
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
    let spec = build_onboard_shortcut_screen_spec(shortcut_kind, config, import_source, true);
    render_onboard_screen_spec(&spec, width, color_enabled)
}

fn render_onboard_shortcut_header_lines_with_style(
    shortcut_kind: OnboardShortcutKind,
    config: &mvp::config::LoongClawConfig,
    import_source: Option<&str>,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let spec = build_onboard_shortcut_screen_spec(shortcut_kind, config, import_source, false);
    render_onboard_screen_spec(&spec, width, color_enabled)
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
    let footer_lines = append_escape_cancel_hint(vec![render_default_choice_footer_line(
        "n",
        copy.default_choice_description,
    )]);
    let spec = TuiScreenSpec {
        header_style: TuiHeaderStyle::Brand,
        subtitle: Some(copy.subtitle.to_owned()),
        title: Some(copy.title.to_owned()),
        progress_line: None,
        intro_lines: vec!["review the trust boundary before writing any config".to_owned()],
        sections: vec![
            TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Warning,
                title: Some("what onboarding can do".to_owned()),
                lines: vec![
                    "LoongClaw can invoke tools and read local files when enabled.".to_owned(),
                    "Keep credentials in environment variables, not in prompts.".to_owned(),
                    "Prefer allowlist-style tool policy for shared environments.".to_owned(),
                ],
            },
            TuiSectionSpec::Narrative {
                title: Some("recommended baseline".to_owned()),
                lines: vec![
                    "start with the narrowest tool scope that still lets you verify first success"
                        .to_owned(),
                    "you can widen channels, models, and local automation after doctor and review"
                        .to_owned(),
                ],
            },
        ],
        choices: vec![
            TuiChoiceSpec {
                key: "y".to_owned(),
                label: copy.continue_label.to_owned(),
                detail_lines: vec![copy.continue_detail.to_owned()],
                recommended: false,
            },
            TuiChoiceSpec {
                key: "n".to_owned(),
                label: copy.cancel_label.to_owned(),
                detail_lines: vec![copy.cancel_detail.to_owned()],
                recommended: false,
            },
        ],
        footer_lines,
    };

    render_onboard_screen_spec(&spec, width, color_enabled)
}

fn build_onboard_shortcut_screen_spec(
    shortcut_kind: OnboardShortcutKind,
    config: &mvp::config::LoongClawConfig,
    import_source: Option<&str>,
    include_choices: bool,
) -> TuiScreenSpec {
    let mut snapshot_lines = Vec::new();
    if let Some(source) = import_source {
        let starting_point_label = onboard_starting_point_label(None, source);
        snapshot_lines.push(onboard_display_line(
            "- starting point: ",
            &starting_point_label,
        ));
    }
    snapshot_lines.extend(build_onboard_review_digest_display_lines(config));
    let snapshot_title = if import_source.is_some() {
        "detected starting point snapshot"
    } else {
        "current setup snapshot"
    };

    let choices = if include_choices {
        tui_choices_from_screen_options(&build_onboard_shortcut_screen_options(shortcut_kind))
    } else {
        Vec::new()
    };
    let default_choice_footer_line = render_shortcut_default_choice_footer_line(shortcut_kind);
    let footer_lines = append_escape_cancel_hint(vec![default_choice_footer_line]);

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: Some(shortcut_kind.subtitle().to_owned()),
        title: Some(shortcut_kind.title().to_owned()),
        progress_line: None,
        intro_lines: Vec::new(),
        sections: vec![
            TuiSectionSpec::Narrative {
                title: Some(snapshot_title.to_owned()),
                lines: snapshot_lines,
            },
            TuiSectionSpec::Callout {
                tone: TuiCalloutTone::Success,
                title: Some("fast lane".to_owned()),
                lines: vec![shortcut_kind.summary_line().to_owned()],
            },
        ],
        choices,
        footer_lines,
    }
}

fn render_preflight_summary_screen_lines_with_style(
    checks: &[OnboardCheck],
    width: usize,
    flow_style: ReviewFlowStyle,
    color_enabled: bool,
) -> Vec<String> {
    let progress_line = flow_style.progress_line();

    render_preflight_summary_screen_lines_with_progress(
        checks,
        width,
        progress_line.as_str(),
        color_enabled,
    )
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
    let spec = build_write_confirmation_screen_spec(config_path, warnings_kept, flow_style);

    render_onboard_screen_spec(&spec, width, color_enabled)
}

fn build_onboard_choice_screen_spec(
    header_style: OnboardHeaderStyle,
    subtitle: &str,
    title: &str,
    step: Option<(GuidedOnboardStep, GuidedPromptPath)>,
    intro_lines: Vec<String>,
    options: Vec<OnboardScreenOption>,
    footer_lines: Vec<String>,
    show_escape_cancel_hint: bool,
) -> TuiScreenSpec {
    let resolved_subtitle = screen_subtitle(subtitle);
    let resolved_progress_line =
        step.map(|(step, guided_prompt_path)| step.progress_line(guided_prompt_path));
    let resolved_footer_lines = if show_escape_cancel_hint {
        append_escape_cancel_hint(footer_lines)
    } else {
        footer_lines
    };
    let resolved_choices = tui_choices_from_screen_options(&options);

    TuiScreenSpec {
        header_style: tui_header_style(header_style),
        subtitle: resolved_subtitle,
        title: Some(title.to_owned()),
        progress_line: resolved_progress_line,
        intro_lines,
        sections: Vec::new(),
        choices: resolved_choices,
        footer_lines: resolved_footer_lines,
    }
}

fn build_onboard_input_screen_spec(
    title: &str,
    step: GuidedOnboardStep,
    guided_prompt_path: GuidedPromptPath,
    context_lines: Vec<String>,
    hint_lines: Vec<String>,
) -> TuiScreenSpec {
    let resolved_footer_lines = append_escape_cancel_hint(hint_lines);
    let progress_line = step.progress_line(guided_prompt_path);

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: None,
        title: Some(title.to_owned()),
        progress_line: Some(progress_line),
        intro_lines: context_lines,
        sections: Vec::new(),
        choices: Vec::new(),
        footer_lines: resolved_footer_lines,
    }
}

fn build_write_confirmation_screen_spec(
    config_path: &str,
    warnings_kept: bool,
    flow_style: ReviewFlowStyle,
) -> TuiScreenSpec {
    let mut intro_lines = Vec::new();
    let config_line = format!("- config: {config_path}");
    let status_line =
        crate::onboard_presentation::write_confirmation_status_line(warnings_kept).to_owned();

    intro_lines.push(config_line);
    intro_lines.push(status_line);

    let choices = vec![
        TuiChoiceSpec {
            key: "y".to_owned(),
            label: crate::onboard_presentation::write_confirmation_label().to_owned(),
            detail_lines: vec![crate::onboard_presentation::write_confirmation_detail().to_owned()],
            recommended: false,
        },
        TuiChoiceSpec {
            key: "n".to_owned(),
            label: crate::onboard_presentation::write_confirmation_cancel_label().to_owned(),
            detail_lines: vec![
                crate::onboard_presentation::write_confirmation_cancel_detail().to_owned(),
            ],
            recommended: false,
        },
    ];

    let default_choice_line = render_default_choice_footer_line(
        "y",
        crate::onboard_presentation::write_confirmation_default_choice_description(),
    );
    let footer_lines = append_escape_cancel_hint(vec![default_choice_line]);

    TuiScreenSpec {
        header_style: TuiHeaderStyle::Compact,
        subtitle: None,
        title: Some(crate::onboard_presentation::write_confirmation_title().to_owned()),
        progress_line: Some(flow_style.progress_line()),
        intro_lines,
        sections: Vec::new(),
        choices,
        footer_lines,
    }
}

fn tui_header_style(style: OnboardHeaderStyle) -> TuiHeaderStyle {
    match style {
        OnboardHeaderStyle::Compact => TuiHeaderStyle::Compact,
    }
}

fn screen_subtitle(subtitle: &str) -> Option<String> {
    let trimmed_subtitle = subtitle.trim();

    if trimmed_subtitle.is_empty() {
        return None;
    }

    Some(trimmed_subtitle.to_owned())
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
        OnboardHeaderStyle::Compact,
        width,
        crate::onboard_presentation::starting_point_selection_subtitle(),
        crate::onboard_presentation::starting_point_selection_title(),
        None,
        vec![crate::onboard_presentation::starting_point_selection_hint().to_owned()],
        options,
        footer_lines,
        true,
        color_enabled,
    )
}

fn render_starting_point_selection_header_lines_with_style(
    _candidates: &[ImportCandidate],
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        crate::onboard_presentation::starting_point_selection_subtitle(),
        crate::onboard_presentation::starting_point_selection_title(),
        None,
        vec![crate::onboard_presentation::starting_point_selection_hint().to_owned()],
        Vec::new(),
        Vec::new(),
        true,
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
    let intro = provider_selection_intro_lines(plan);
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
        OnboardHeaderStyle::Compact,
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
        true,
        color_enabled,
    )
}

fn render_provider_selection_header_lines(
    plan: &crate::migration::ProviderSelectionPlan,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
) -> Vec<String> {
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "choose the current provider",
        "choose active provider",
        Some((GuidedOnboardStep::Provider, guided_prompt_path)),
        provider_selection_intro_lines(plan),
        vec![],
        vec![],
        true,
        true,
    )
}

fn provider_selection_intro_lines(plan: &crate::migration::ProviderSelectionPlan) -> Vec<String> {
    if plan.imported_choices.is_empty() {
        vec!["pick the provider that should back this setup".to_owned()]
    } else if plan.requires_explicit_choice {
        vec!["other detected settings stay merged".to_owned()]
    } else {
        vec!["review the detected provider choices for this setup".to_owned()]
    }
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
        false,
    )
}

fn render_model_selection_screen_lines_with_style(
    config: &mvp::config::LoongClawConfig,
    prompt_default: &str,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
    catalog_models_available: bool,
) -> Vec<String> {
    let selection_context =
        onboarding_model_policy::onboarding_model_selection_context(&config.provider);
    let current_model = selection_context.current_model;
    let recommended_model = selection_context.recommended_model;
    let preferred_fallback_models = selection_context.preferred_fallback_models;
    let allows_auto_fallback_hint = selection_context.allows_auto_fallback_hint;
    let mut context_lines = vec![
        format!(
            "- provider: {}",
            crate::provider_presentation::guided_provider_label(config.provider.kind)
        ),
        format!("- current model: {current_model}"),
    ];
    if let Some(recommended_model) = recommended_model {
        context_lines.push(format!("- recommended model: {recommended_model}"));
    }
    if !preferred_fallback_models.is_empty() {
        let preferred_fallback_summary = preferred_fallback_models.join(", ");
        context_lines.push(format!(
            "- configured preferred fallback: {preferred_fallback_summary}",
        ));
    }

    let mut hint_lines = vec![render_model_selection_default_hint_line(
        config,
        prompt_default,
    )];
    if catalog_models_available {
        hint_lines.push(
            "- use arrow keys to browse or type to filter available provider models".to_owned(),
        );
        hint_lines.push(
            "- choose `enter custom model id` if you want to type an override manually".to_owned(),
        );
    } else {
        hint_lines.push("- type any provider model id to override it".to_owned());
    }
    if allows_auto_fallback_hint {
        let preferred_fallback_summary = preferred_fallback_models.join(", ");
        hint_lines.push(format!(
            "- type `auto` to let runtime try configured preferred fallbacks first: {preferred_fallback_summary}",
        ));
    }

    render_onboard_input_screen(
        width,
        "choose model",
        GuidedOnboardStep::Model,
        guided_prompt_path,
        context_lines,
        hint_lines,
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
    if let Some(current_env) = render_configured_provider_credential_source_value(&config.provider)
    {
        context_lines.push(format!("- current source: {current_env}"));
    }
    if let Some(suggested_source) =
        provider_credential_policy::render_provider_credential_source_value(Some(
            default_api_key_env,
        ))
    {
        context_lines.push(format!("- suggested source: {suggested_source}"));
    }

    let example_env_name =
        provider_credential_policy::provider_credential_env_hint(&config.provider)
            .unwrap_or_else(|| "PROVIDER_API_KEY".to_owned());
    let mut hint_lines = vec![render_api_key_env_selection_default_hint_line(
        config,
        default_api_key_env,
        prompt_default,
    )];
    hint_lines.push("- enter an env var name, not the secret value itself".to_owned());
    hint_lines.push(format!("- example: {example_env_name}"));
    if prompt_default.trim().is_empty() {
        if provider_credential_policy::provider_has_inline_credential(&config.provider) {
            hint_lines.push("- leave this blank to keep inline credentials".to_owned());
        }
    } else if provider_supports_blank_api_key_env(config) {
        hint_lines.push(render_clear_input_hint_line(
            "clear the configured credential env",
        ));
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

fn render_web_search_credential_selection_screen_lines_with_style(
    config: &mvp::config::LoongClawConfig,
    provider: &str,
    prompt_default: &str,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
    color_enabled: bool,
) -> Vec<String> {
    let provider_label = web_search_provider_display_name(provider);
    let mut context_lines = vec![format!("- provider: {provider_label}")];
    if let Some(current_value) =
        configured_web_search_provider_credential_source_value(config, provider)
    {
        let label = if current_value == "inline api key" {
            "- current credential: "
        } else {
            "- current source: "
        };
        context_lines.extend(mvp::presentation::render_wrapped_text_line(
            label,
            &current_value,
            width,
        ));
    }
    if let Some(suggested_env) = mvp::config::web_search_provider_descriptor(provider)
        .and_then(|descriptor| descriptor.default_api_key_env)
        .and_then(|env_name| {
            provider_credential_policy::render_provider_credential_source_value(Some(env_name))
        })
    {
        context_lines.extend(mvp::presentation::render_wrapped_text_line(
            "- suggested source: ",
            &suggested_env,
            width,
        ));
    }

    let mut hint_lines = vec![render_web_search_credential_selection_default_hint_line(
        config,
        provider,
        prompt_default,
    )];
    hint_lines.push("- enter an env var name, not the secret value itself".to_owned());
    let example_env_name = mvp::config::web_search_provider_descriptor(provider)
        .and_then(|descriptor| {
            descriptor
                .default_api_key_env
                .or_else(|| descriptor.api_key_env_names.first().copied())
        })
        .unwrap_or("WEB_SEARCH_API_KEY");
    hint_lines.push(format!("- example: {example_env_name}"));
    if prompt_default.trim().is_empty()
        && web_search_provider_has_inline_credential(config, provider)
    {
        hint_lines.push("- leave this blank to keep inline credentials".to_owned());
    }
    if configured_web_search_provider_secret(config, provider)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        hint_lines.push(render_clear_input_hint_line(
            "clear the configured web search credential",
        ));
    }

    render_onboard_input_screen(
        width,
        "choose web search credential",
        GuidedOnboardStep::WebSearchProvider,
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
            ONBOARD_SINGLE_LINE_INPUT_HINT.to_owned(),
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
        OnboardHeaderStyle::Compact,
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
        true,
        color_enabled,
    )
}

fn render_personality_selection_header_lines(
    config: &mvp::config::LoongClawConfig,
    width: usize,
) -> Vec<String> {
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
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
        vec![],
        vec![],
        true,
        true,
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
            "- press Enter to keep current addendum".to_owned(),
            "- type '-' to clear it".to_owned(),
            ONBOARD_SINGLE_LINE_INPUT_HINT.to_owned(),
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
    let options = MEMORY_PROFILE_CHOICES
        .into_iter()
        .map(|(profile, label, detail)| OnboardScreenOption {
            key: memory_profile_id(profile).to_owned(),
            label: label.to_owned(),
            detail_lines: vec![detail.to_owned()],
            recommended: profile == default_profile,
        })
        .collect::<Vec<_>>();

    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
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
        true,
        color_enabled,
    )
}

fn render_memory_profile_selection_header_lines(
    config: &mvp::config::LoongClawConfig,
    guided_prompt_path: GuidedPromptPath,
    width: usize,
) -> Vec<String> {
    render_onboard_choice_screen(
        OnboardHeaderStyle::Compact,
        width,
        "choose how much memory context LoongClaw should inject",
        "choose memory profile",
        Some((GuidedOnboardStep::MemoryProfile, guided_prompt_path)),
        vec![format!(
            "- current profile: {}",
            memory_profile_id(config.memory.profile)
        )],
        vec![],
        vec![],
        true,
        true,
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
        build_existing_config_write_screen_options(),
        vec![render_default_choice_footer_line(
            "b",
            "create backup and replace",
        )],
        true,
        color_enabled,
    )
}

fn render_existing_config_write_header_lines_with_style(
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
        Vec::new(),
        Vec::new(),
        true,
        color_enabled,
    )
}

fn onboard_display_line(prefix: &str, value: &str) -> String {
    format!("{prefix}{value}")
}

fn build_onboard_review_digest_display_lines(config: &mvp::config::LoongClawConfig) -> Vec<String> {
    let mut lines = crate::provider_presentation::provider_profile_state_display_lines(
        config,
        Some("- provider: "),
    );
    lines.push(onboard_display_line("- model: ", &config.provider.model));
    lines.push(onboard_display_line(
        "- transport: ",
        &config.provider.transport_readiness().summary,
    ));

    if let Some(provider_endpoint) = config.provider.region_endpoint_note() {
        lines.push(onboard_display_line(
            "- provider endpoint: ",
            &provider_endpoint,
        ));
    }

    if let Some(credential_line) = render_onboard_review_credential_line(&config.provider) {
        lines.push(credential_line);
    }

    let prompt_mode = summarize_prompt_mode(config);
    lines.push(onboard_display_line("- prompt mode: ", &prompt_mode));

    if config.cli.uses_native_prompt_pack() {
        lines.push(onboard_display_line(
            "- personality: ",
            prompt_personality_id(config.cli.resolved_personality()),
        ));

        if let Some(prompt_addendum) = summarize_prompt_addendum(config) {
            lines.push(onboard_display_line(
                "- prompt addendum: ",
                &prompt_addendum,
            ));
        }
    }

    lines.push(onboard_display_line(
        "- memory profile: ",
        memory_profile_id(config.memory.profile),
    ));

    let web_search_provider =
        web_search_provider_display_name(config.tools.web_search.default_provider.as_str());
    lines.push(onboard_display_line("- web search: ", &web_search_provider));

    if let Some(web_search_credential) = summarize_web_search_provider_credential(
        config,
        config.tools.web_search.default_provider.as_str(),
    ) {
        let credential_prefix = format!("- {}: ", web_search_credential.label);
        lines.push(onboard_display_line(
            &credential_prefix,
            &web_search_credential.value,
        ));
    }

    let enabled_channels = enabled_channel_ids(config)
        .into_iter()
        .filter(|channel| channel != "cli")
        .collect::<Vec<_>>();
    if !enabled_channels.is_empty() {
        lines.push(onboard_display_line(
            "- channels: ",
            &enabled_channels.join(", "),
        ));
    }

    lines
}

fn render_onboard_review_credential_line(provider: &mvp::config::ProviderConfig) -> Option<String> {
    summarize_provider_credential(provider)
        .map(|credential| format!("- {}: {}", credential.label, credential.value))
}

pub(crate) fn summarize_prompt_mode(config: &mvp::config::LoongClawConfig) -> String {
    if config.cli.uses_native_prompt_pack() {
        return "native prompt pack".to_owned();
    }

    "inline system prompt override".to_owned()
}

pub(crate) fn summarize_prompt_addendum(config: &mvp::config::LoongClawConfig) -> Option<String> {
    config
        .cli
        .system_prompt_addendum
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub(crate) fn summarize_provider_credential(
    provider: &mvp::config::ProviderConfig,
) -> Option<OnboardingCredentialSummary> {
    if secret_ref_has_inline_literal(provider.oauth_access_token.as_ref()) {
        return Some(OnboardingCredentialSummary {
            label: "credential",
            value: "inline oauth token".to_owned(),
        });
    }
    if let Some(configured_env) = render_configured_provider_credential_source_value(provider) {
        return Some(OnboardingCredentialSummary {
            label: "credential source",
            value: configured_env,
        });
    }
    if secret_ref_has_inline_literal(provider.api_key.as_ref()) {
        return Some(OnboardingCredentialSummary {
            label: "credential",
            value: "inline api key".to_owned(),
        });
    }
    provider_credential_policy::preferred_provider_credential_env_binding(provider)
        .and_then(|binding| {
            provider_credential_policy::render_provider_credential_source_value(Some(
                binding.env_name.as_str(),
            ))
        })
        .map(|credential_env| OnboardingCredentialSummary {
            label: "credential source",
            value: credential_env,
        })
}

fn provider_supports_blank_api_key_env(config: &mvp::config::LoongClawConfig) -> bool {
    provider_credential_policy::provider_has_inline_credential(&config.provider)
        || provider_credential_policy::provider_has_configured_credential_env(&config.provider)
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
    mvp::presentation::detect_render_width()
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
        || option_has_non_empty_value(env::var("LOONGCLAW_WEB_SEARCH_PROVIDER").ok().as_deref())
}

fn option_has_non_empty_value(raw: Option<&str>) -> bool {
    raw.is_some_and(|value| !value.trim().is_empty())
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
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::ffi::OsString;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, MutexGuard};

    use crate::test_support::ScopedEnv;

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

    struct SelectOnlyTestUi {
        inputs: VecDeque<String>,
    }

    impl SelectOnlyTestUi {
        fn with_inputs(inputs: impl IntoIterator<Item = impl Into<String>>) -> Self {
            Self {
                inputs: inputs.into_iter().map(Into::into).collect(),
            }
        }
    }

    struct AllowEmptyOnlyTestUi {
        inputs: VecDeque<String>,
    }

    impl AllowEmptyOnlyTestUi {
        fn with_inputs(inputs: impl IntoIterator<Item = impl Into<String>>) -> Self {
            Self {
                inputs: inputs.into_iter().map(Into::into).collect(),
            }
        }
    }

    fn interactive_onboard_options() -> OnboardCommandOptions {
        OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
            accept_risk: true,
            provider: None,
            model: None,
            api_key_env: None,
            web_search_provider: None,
            web_search_api_key_env: None,
            personality: None,
            memory_profile: None,
            system_prompt: None,
            skip_model_probe: false,
        }
    }

    fn onboard_test_context() -> OnboardRuntimeContext {
        OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>())
    }

    #[allow(dead_code)]
    fn browser_companion_temp_dir(label: &str) -> PathBuf {
        static NEXT_TEMP_DIR_SEED: AtomicU64 = AtomicU64::new(1);
        let seed = NEXT_TEMP_DIR_SEED.fetch_add(1, Ordering::Relaxed);
        let temp_dir = std::env::temp_dir().join(format!(
            "loongclaw-browser-companion-onboard-{label}-{}-{seed}",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp_dir).expect("create browser companion onboard temp dir");
        temp_dir
    }

    fn uuid_shaped_secret_fixture() -> String {
        let first = "9f479837";
        let second = "0a12";
        let third = "4b56";
        let fourth = "89ab";
        let fifth = "cdef01234567";
        format!("{first}-{second}-{third}-{fourth}-{fifth}")
    }

    fn browser_companion_script_path(temp_dir: &Path) -> PathBuf {
        #[cfg(windows)]
        {
            temp_dir.join("browser-companion.cmd")
        }
        #[cfg(not(windows))]
        {
            temp_dir.join("browser-companion")
        }
    }

    #[allow(dead_code)]
    fn write_browser_companion_version_script(temp_dir: &Path, version: &str) -> PathBuf {
        let script_path = browser_companion_script_path(temp_dir);

        #[cfg(windows)]
        {
            let script_body = format!(
                "@echo off\r\nif \"%~1\"==\"--version\" (\r\n  echo loongclaw-browser-companion {version}\r\n  exit /b 0\r\n)\r\necho unexpected arguments 1>&2\r\nexit /b 1\r\n"
            );
            std::fs::write(&script_path, script_body).expect("write browser companion script");
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let script_body = format!(
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  echo 'loongclaw-browser-companion {version}'\n  exit 0\nfi\necho 'unexpected arguments' >&2\nexit 1\n"
            );
            let mut file =
                std::fs::File::create(&script_path).expect("create browser companion script");
            file.write_all(script_body.as_bytes())
                .expect("write browser companion script");
            file.sync_all()
                .expect("sync browser companion script to disk");
            drop(file);

            let metadata = std::fs::metadata(&script_path).expect("script metadata");
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&script_path, permissions)
                .expect("chmod browser companion script");
        }

        script_path
    }

    impl OnboardUi for TestOnboardUi {
        fn print_line(&mut self, _line: &str) -> CliResult<()> {
            Ok(())
        }

        fn prompt_with_default(&mut self, _label: &str, default: &str) -> CliResult<String> {
            let value =
                ensure_onboard_input_not_cancelled(self.inputs.pop_front().unwrap_or_default())?;
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return Ok(default.to_owned());
            }
            Ok(trimmed.to_owned())
        }

        fn prompt_required(&mut self, _label: &str) -> CliResult<String> {
            let value = self
                .inputs
                .pop_front()
                .ok_or_else(|| "missing required test input".to_owned())?;
            Ok(ensure_onboard_input_not_cancelled(value)?.trim().to_owned())
        }

        fn prompt_allow_empty(&mut self, label: &str) -> CliResult<String> {
            match self.inputs.front() {
                Some(value)
                    if label == PREINSTALLED_SKILLS_PROMPT_LABEL
                        && parse_preinstalled_skill_selection(value.as_str()).is_err() =>
                {
                    Ok(String::new())
                }
                Some(_) => {
                    let value = self
                        .inputs
                        .pop_front()
                        .ok_or_else(|| "missing allow-empty test input".to_owned())?;
                    Ok(ensure_onboard_input_not_cancelled(value)?.trim().to_owned())
                }
                None if label == PREINSTALLED_SKILLS_PROMPT_LABEL => Ok(String::new()),
                None => Err("missing allow-empty test input".to_owned()),
            }
        }

        fn prompt_confirm(&mut self, _message: &str, default: bool) -> CliResult<bool> {
            let Some(value) = self.inputs.pop_front() else {
                return Ok(default);
            };
            let value = ensure_onboard_input_not_cancelled(value)?;
            let value = value.trim().to_ascii_lowercase();
            if value.is_empty() {
                return Ok(default);
            }
            Ok(matches!(value.as_str(), "y" | "yes"))
        }

        fn select_one(
            &mut self,
            _label: &str,
            options: &[SelectOption],
            default: Option<usize>,
            _interaction_mode: SelectInteractionMode,
        ) -> CliResult<usize> {
            let default = validate_select_one_state(options.len(), default)?;
            match self.inputs.pop_front() {
                Some(value) => {
                    let value = ensure_onboard_input_not_cancelled(value)?;
                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        return default
                            .ok_or_else(|| "no default for required selection".to_owned());
                    }
                    if let Ok(n) = trimmed.parse::<usize>() {
                        if n >= 1 && n <= options.len() {
                            return Ok(n - 1);
                        }
                        return Err(format!(
                            "test selection {n} out of range 1..={}",
                            options.len()
                        ));
                    }
                    parse_select_one_input(trimmed, options)
                        .ok_or_else(|| format!("invalid test selection input: {trimmed}"))
                }
                None => {
                    default.ok_or_else(|| "missing test input for required selection".to_owned())
                }
            }
        }
    }

    impl OnboardUi for SelectOnlyTestUi {
        fn print_line(&mut self, _line: &str) -> CliResult<()> {
            Ok(())
        }

        fn prompt_with_default(&mut self, _label: &str, _default: &str) -> CliResult<String> {
            Err("test expected interactive select widget instead of prompt_with_default".to_owned())
        }

        fn prompt_required(&mut self, _label: &str) -> CliResult<String> {
            Err("test expected interactive select widget instead of prompt_required".to_owned())
        }

        fn prompt_allow_empty(&mut self, label: &str) -> CliResult<String> {
            if label == PREINSTALLED_SKILLS_PROMPT_LABEL {
                return Ok(String::new());
            }
            Err("test expected interactive select widget instead of prompt_allow_empty".to_owned())
        }

        fn prompt_confirm(&mut self, _message: &str, _default: bool) -> CliResult<bool> {
            Err("test expected interactive select widget instead of prompt_confirm".to_owned())
        }

        fn select_one(
            &mut self,
            _label: &str,
            options: &[SelectOption],
            default: Option<usize>,
            _interaction_mode: SelectInteractionMode,
        ) -> CliResult<usize> {
            let default = validate_select_one_state(options.len(), default)?;
            match self.inputs.pop_front() {
                Some(value) => {
                    let value = ensure_onboard_input_not_cancelled(value)?;
                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        return default
                            .ok_or_else(|| "no default for required selection".to_owned());
                    }
                    if let Ok(n) = trimmed.parse::<usize>() {
                        if n >= 1 && n <= options.len() {
                            return Ok(n - 1);
                        }
                        return Err(format!(
                            "test selection {n} out of range 1..={}",
                            options.len()
                        ));
                    }
                    parse_select_one_input(trimmed, options)
                        .ok_or_else(|| format!("invalid test selection input: {trimmed}"))
                }
                None => {
                    default.ok_or_else(|| "missing test input for required selection".to_owned())
                }
            }
        }
    }

    impl OnboardUi for AllowEmptyOnlyTestUi {
        fn print_line(&mut self, _line: &str) -> CliResult<()> {
            Ok(())
        }

        fn prompt_with_default(&mut self, _label: &str, _default: &str) -> CliResult<String> {
            Err("test expected prompt_allow_empty instead of prompt_with_default".to_owned())
        }

        fn prompt_required(&mut self, _label: &str) -> CliResult<String> {
            Err("test expected prompt_allow_empty instead of prompt_required".to_owned())
        }

        fn prompt_allow_empty(&mut self, _label: &str) -> CliResult<String> {
            let value = self
                .inputs
                .pop_front()
                .ok_or_else(|| "missing allow-empty test input".to_owned())?;
            Ok(ensure_onboard_input_not_cancelled(value)?.trim().to_owned())
        }

        fn prompt_confirm(&mut self, _message: &str, _default: bool) -> CliResult<bool> {
            Err("test expected prompt_allow_empty instead of prompt_confirm".to_owned())
        }

        fn select_one(
            &mut self,
            _label: &str,
            _options: &[SelectOption],
            _default: Option<usize>,
            _interaction_mode: SelectInteractionMode,
        ) -> CliResult<usize> {
            Err("test expected prompt_allow_empty instead of select_one".to_owned())
        }
    }

    struct TestPromptLineReader {
        blocking_reads: VecDeque<OnboardPromptRead>,
        pending_lines: VecDeque<String>,
    }

    impl TestPromptLineReader {
        fn new(
            blocking_reads: impl IntoIterator<Item = OnboardPromptRead>,
            pending_lines: impl IntoIterator<Item = impl Into<String>>,
        ) -> Self {
            Self {
                blocking_reads: blocking_reads.into_iter().collect(),
                pending_lines: pending_lines.into_iter().map(Into::into).collect(),
            }
        }
    }

    impl OnboardPromptLineReader for TestPromptLineReader {
        fn read_blocking_line(&mut self) -> CliResult<OnboardPromptRead> {
            Ok(self
                .blocking_reads
                .pop_front()
                .unwrap_or(OnboardPromptRead::Eof))
        }

        fn read_pending_line(&mut self) -> CliResult<Option<String>> {
            Ok(self.pending_lines.pop_front())
        }
    }

    struct BrowserCompanionEnvGuard {
        _lock: MutexGuard<'static, ()>,
        saved_ready: Option<OsString>,
    }

    fn set_browser_companion_env_var(key: &str, value: &str) {
        // SAFETY: daemon tests serialize process env mutations behind
        // `lock_daemon_test_environment`, so no concurrent env readers/writers
        // observe racy updates while these tests run.
        #[allow(unsafe_code, clippy::disallowed_methods)]
        unsafe {
            std::env::set_var(key, value);
        }
    }

    fn remove_browser_companion_env_var(key: &str) {
        // SAFETY: daemon tests serialize process env mutations behind
        // `lock_daemon_test_environment`, so removing the variable here is
        // coordinated with all other env-mutating daemon tests.
        #[allow(unsafe_code, clippy::disallowed_methods)]
        unsafe {
            std::env::remove_var(key);
        }
    }

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

    struct PasteDrainWindowEnvGuard {
        _lock: MutexGuard<'static, ()>,
        saved_value: Option<OsString>,
    }

    impl PasteDrainWindowEnvGuard {
        fn set(value: Option<&str>) -> Self {
            let lock = crate::test_support::lock_daemon_test_environment();
            let saved_value = std::env::var_os(ONBOARD_PASTE_DRAIN_WINDOW_ENV);
            match value {
                Some(value) => set_browser_companion_env_var(ONBOARD_PASTE_DRAIN_WINDOW_ENV, value),
                None => remove_browser_companion_env_var(ONBOARD_PASTE_DRAIN_WINDOW_ENV),
            }
            Self {
                _lock: lock,
                saved_value,
            }
        }
    }

    impl Drop for PasteDrainWindowEnvGuard {
        fn drop(&mut self) {
            match &self.saved_value {
                Some(value) => {
                    set_browser_companion_env_var(
                        ONBOARD_PASTE_DRAIN_WINDOW_ENV,
                        &value.to_string_lossy(),
                    );
                }
                None => remove_browser_companion_env_var(ONBOARD_PASTE_DRAIN_WINDOW_ENV),
            }
        }
    }

    impl Drop for BrowserCompanionEnvGuard {
        fn drop(&mut self) {
            let key = "LOONGCLAW_BROWSER_COMPANION_READY";
            match self.saved_ready.take() {
                Some(value) => set_browser_companion_env_var(key, &value.to_string_lossy()),
                None => remove_browser_companion_env_var(key),
            }
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

    #[tokio::test(flavor = "current_thread")]
    async fn browser_companion_onboard_preflight_warns_when_enabled_without_command() {
        let _env_guard = BrowserCompanionEnvGuard::runtime_gate_closed();
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.api_key = Some(SecretRef::Inline("inline-openai-key".to_owned()));
        config.tools.browser_companion.enabled = true;

        let checks = run_preflight_checks(&config, true).await;

        assert!(
            checks.iter().any(|check| {
                check.name == "browser companion install"
                    && check.level == OnboardCheckLevel::Warn
                    && check.detail.contains("no command is configured")
            }),
            "onboard preflight should flag companion configs that cannot be executed yet: {checks:#?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_preflight_checks_fail_for_invalid_provider_credential_env_value() {
        let secret = "sk-live-direct-secret-value";
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Openai;
        config.provider.api_key_env = Some(secret.to_owned());
        config.provider.api_key = None;

        let checks = run_preflight_checks(&config, true).await;

        assert!(
            checks.iter().any(|check| {
                check.name == "config validation"
                    && check.level == OnboardCheckLevel::Fail
                    && check.detail.contains("provider.api_key_env")
                    && !check.detail.contains(secret)
            }),
            "preflight should fail fast on invalid provider credential env values without echoing the secret: {checks:#?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn browser_companion_onboard_preflight_warns_when_runtime_gate_is_closed() {
        let _env_guard = BrowserCompanionEnvGuard::runtime_gate_closed();

        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.api_key = Some(SecretRef::Inline("inline-openai-key".to_owned()));
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.command = Some(
            crate::browser_companion_diagnostics::fake_browser_companion_version_command("1.5.0"),
        );
        config.tools.browser_companion.expected_version = Some("1.5.0".to_owned());

        let checks = run_preflight_checks(&config, true).await;

        assert!(
            checks.iter().any(|check| {
                check.name == "browser companion install"
                    && check.level == OnboardCheckLevel::Warn
                    && check.detail.contains("runtime gate is still closed")
            }),
            "onboard preflight should surface that a healthy install still is not runtime-ready: {checks:#?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn browser_companion_onboard_preflight_passes_when_runtime_gate_is_open() {
        let _env_guard = BrowserCompanionEnvGuard::runtime_gate_open();

        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.api_key = Some(SecretRef::Inline("inline-openai-key".to_owned()));
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.command = Some(
            crate::browser_companion_diagnostics::fake_browser_companion_version_command("1.5.0"),
        );
        config.tools.browser_companion.expected_version = Some("1.5.0".to_owned());

        let checks = run_preflight_checks(&config, true).await;

        assert!(
            checks.iter().any(|check| {
                check.name == "browser companion install"
                    && check.level == OnboardCheckLevel::Pass
                    && check.detail.contains("runtime is ready")
            }),
            "onboard preflight should mark the companion lane healthy when the runtime gate is open: {checks:#?}"
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
    fn provider_model_probe_transport_failure_prioritizes_route_guidance() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.model = "custom-explicit-model".to_owned();

        let check = provider_model_probe_failure_check(
            &config,
            "provider model-list request failed on attempt 3/3: operation timed out".to_owned(),
        );

        assert_eq!(check.name, "provider model probe");
        assert_eq!(check.level, OnboardCheckLevel::Fail);
        assert!(
            check
                .detail
                .contains(crate::provider_route_diagnostics::MODEL_CATALOG_TRANSPORT_FAILED_MARKER),
            "transport probe failures should use the route-focused marker during onboarding: {check:#?}"
        );
        assert!(
            !check.detail.contains("provider.model"),
            "transport probe failures should not suggest model-selection repair when the route is the real blocker: {check:#?}"
        );
        assert!(
            !check.detail.contains("below"),
            "transport probe failures should not promise a later probe section that may not exist in non-interactive output: {check:#?}"
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
        assert!(
            check.detail.contains("model = auto"),
            "auto-model probe failures should explain why onboarding cannot continue with an unresolved automatic model: {check:#?}"
        );
        assert!(
            check.detail.contains("provider.model"),
            "auto-model probe failures should point users to an explicit provider.model remediation path: {check:#?}"
        );
        assert!(
            check.detail.contains("preferred_models"),
            "auto-model probe failures should point users to preferred_models when catalog probing is unavailable: {check:#?}"
        );
    }

    #[test]
    fn provider_model_probe_failure_warns_for_preferred_model_fallbacks() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Minimax;
        config.provider.model = "auto".to_owned();
        config.provider.preferred_models = vec![
            "MiniMax-M2.5".to_owned(),
            "MiniMax-M2.5".to_owned(),
            "MiniMax-M2.7-highspeed".to_owned(),
        ];

        let check = provider_model_probe_failure_check(
            &config,
            "provider rejected the model list".to_owned(),
        );

        assert_eq!(check.name, "provider model probe");
        assert_eq!(check.level, OnboardCheckLevel::Warn);
        assert!(
            check.detail.contains("configured preferred"),
            "onboarding should only advertise fallback continuation for explicitly configured preferred models: {check:#?}"
        );
        assert!(
            check.detail.contains("MiniMax-M2.5"),
            "onboard warning should surface the first fallback model to keep the first-run path actionable: {check:#?}"
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
        assert_eq!(check.level, OnboardCheckLevel::Fail);
        assert_eq!(
            check.non_interactive_warning_policy,
            OnboardNonInteractiveWarningPolicy::RequiresExplicitModel
        );
        assert!(
            check.detail.contains("deepseek-chat"),
            "reviewed providers should point users to the reviewed onboarding default when catalog probing is unavailable: {check:#?}"
        );
        assert!(
            check.detail.contains("rerun onboarding"),
            "reviewed providers should suggest rerunning onboarding to accept the reviewed model instead of leaving recovery implicit: {check:#?}"
        );
    }

    #[test]
    fn provider_model_probe_failure_includes_region_hint_for_minimax() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Minimax;
        config.provider.model = "auto".to_owned();

        let check =
            provider_model_probe_failure_check(&config, "provider returned status 401".to_owned());

        assert_eq!(check.name, "provider model probe");
        assert_eq!(check.level, OnboardCheckLevel::Fail);
        assert!(
            check.detail.contains("https://api.minimax.io"),
            "onboard probe failures for region-sensitive providers should surface the alternate endpoint: {check:#?}"
        );
        assert!(
            check.detail.contains("provider.base_url"),
            "onboard probe failures should explain the concrete config knob to change: {check:#?}"
        );
    }

    #[test]
    fn provider_model_probe_failure_skips_region_hint_for_non_auth_errors() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Minimax;
        config.provider.model = "auto".to_owned();

        let check =
            provider_model_probe_failure_check(&config, "provider returned status 503".to_owned());

        assert_eq!(check.name, "provider model probe");
        assert_eq!(check.level, OnboardCheckLevel::Fail);
        assert!(
            !check.detail.contains("provider.base_url"),
            "non-auth probe failures should not steer operators toward region endpoint changes: {check:#?}"
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
            web_search_provider: None,
            web_search_api_key_env: None,
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
    fn configured_preferred_model_probe_warning_is_accepted_non_interactively() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Minimax;
        config.provider.model = "auto".to_owned();
        config.provider.preferred_models = vec!["MiniMax-M2.5".to_owned()];
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
            web_search_provider: None,
            web_search_api_key_env: None,
            personality: None,
            memory_profile: None,
            system_prompt: None,
            skip_model_probe: false,
        };

        assert!(
            is_explicitly_accepted_non_interactive_warning(&check, &options),
            "configured preferred-model fallback warnings should not block non-interactive onboarding because runtime can still try the operator-configured models: {check:#?}"
        );
    }

    #[test]
    fn non_interactive_preflight_failure_message_uses_first_failing_check_detail() {
        let checks = vec![
            OnboardCheck {
                name: "provider credentials",
                level: OnboardCheckLevel::Pass,
                detail: "credentials ok".to_owned(),
                non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
            },
            OnboardCheck {
                name: "provider model probe",
                level: OnboardCheckLevel::Fail,
                detail: "DeepSeek [deepseek]: model catalog probe failed (401 Unauthorized); current config still uses `model = auto`; rerun onboarding and accept reviewed model `deepseek-chat`, or set `provider.model` / `preferred_models` explicitly".to_owned(),
                non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
            },
        ];

        let message = non_interactive_preflight_failure_message(&checks);

        assert!(
            message.contains("onboard preflight failed: DeepSeek [deepseek]"),
            "non-interactive onboarding should return the actionable failing-check detail instead of a generic probe hint: {message}"
        );
        assert!(
            message.contains("provider.model"),
            "non-interactive onboarding should preserve the explicit remediation from the failing check: {message}"
        );
    }

    #[test]
    fn non_interactive_preflight_failure_message_appends_provider_route_probe_detail_for_transport_failures()
     {
        let checks = vec![
            OnboardCheck {
                name: "provider model probe",
                level: OnboardCheckLevel::Fail,
                detail:
                    "OpenAI [openai]: model catalog transport failed (provider model-list request failed on attempt 3/3: operation timed out); runtime could not verify the provider route. inspect provider route diagnostics and retry once dns / proxy / TUN routing is stable"
                        .to_owned(),
                non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
            },
            OnboardCheck {
                name: "provider route probe",
                level: OnboardCheckLevel::Warn,
                detail:
                    "request/models host api.openai.com:443: dns resolved to 198.18.0.2 (fake-ip-style); tcp connect ok via 198.18.0.2. the route currently depends on local fake-ip/TUN interception, so intermittent long-request failures usually point to proxy health or direct/bypass rules."
                        .to_owned(),
                non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
            },
        ];

        let message = non_interactive_preflight_failure_message(&checks);

        assert!(
            message.contains("provider route probe"),
            "non-interactive onboarding should mention the collected provider route probe when transport diagnostics are available: {message}"
        );
        assert!(
            message.contains("fake-ip-style"),
            "non-interactive onboarding should include the route-probe detail instead of dropping it behind the first failing check: {message}"
        );
    }

    #[test]
    fn non_interactive_preflight_warning_message_uses_first_blocking_warning_detail() {
        let checks = vec![
            OnboardCheck {
                name: "web search provider",
                level: OnboardCheckLevel::Warn,
                detail: "Tavily: TAVILY_API_KEY (expected). web.search will stay unavailable until the provider credential is supplied".to_owned(),
                non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
            },
        ];
        let options = OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: true,
            accept_risk: true,
            provider: None,
            model: None,
            api_key_env: None,
            web_search_provider: Some("tavily".to_owned()),
            web_search_api_key_env: None,
            personality: None,
            memory_profile: None,
            system_prompt: None,
            skip_model_probe: false,
        };

        let message = non_interactive_preflight_warning_message(&checks, &options);

        assert!(
            message.contains("web search provider: Tavily"),
            "non-interactive warning failures should surface the first blocking warning detail instead of collapsing to a generic message: {message}"
        );
        assert!(
            message.contains("rerun without --non-interactive"),
            "non-interactive warning failures should still tell the user how to continue interactively: {message}"
        );
    }

    #[test]
    fn config_validation_failure_message_only_matches_config_validation_failures() {
        let checks = vec![
            OnboardCheck {
                name: "provider credentials",
                level: OnboardCheckLevel::Fail,
                detail: "credentials missing".to_owned(),
                non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
            },
            OnboardCheck {
                name: "config validation",
                level: OnboardCheckLevel::Fail,
                detail: "provider.api_key_env must be an environment variable name".to_owned(),
                non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
            },
        ];

        assert_eq!(
            config_validation_failure_message(&checks),
            Some("onboard preflight failed: provider.api_key_env must be an environment variable name".to_owned()),
            "config validation failures should be surfaced as terminal preflight errors"
        );
    }

    #[test]
    fn provider_credential_check_adds_volcengine_auth_guidance_when_missing() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::VolcengineCoding;
        config.provider.api_key = None;
        config.provider.api_key_env = None;
        config.provider.oauth_access_token = None;
        config.provider.oauth_access_token_env = None;
        let auth_env_names = config.provider.auth_hint_env_names();
        let mut env = ScopedEnv::new();
        for env_name in auth_env_names {
            env.remove(env_name);
        }

        let check = provider_credential_check(&config);

        assert_eq!(check.name, "provider credentials");
        assert_eq!(check.level, OnboardCheckLevel::Warn);
        assert!(check.detail.contains("ARK_API_KEY"));
        assert!(check.detail.contains("Authorization: Bearer <ARK_API_KEY>"));
    }

    #[test]
    fn provider_credential_check_accepts_x_api_key_provider_env_credentials() {
        let mut env = ScopedEnv::new();
        env.set("ANTHROPIC_API_KEY", "test-anthropic-key");
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Anthropic;
        config.provider.api_key = None;
        config.provider.api_key_env = None;
        config.provider.oauth_access_token = None;
        config.provider.oauth_access_token_env = None;

        let check = provider_credential_check(&config);

        assert_eq!(check.name, "provider credentials");
        assert_eq!(check.level, OnboardCheckLevel::Pass);
        assert!(check.detail.contains("ANTHROPIC_API_KEY is available"));
    }

    #[test]
    fn provider_credential_check_passes_for_auth_optional_provider() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Ollama;
        config.provider.api_key = None;
        config.provider.api_key_env = None;
        config.provider.oauth_access_token = None;
        config.provider.oauth_access_token_env = None;

        let check = provider_credential_check(&config);

        assert_eq!(check.name, "provider credentials");
        assert_eq!(check.level, OnboardCheckLevel::Pass);
        assert!(check.detail.contains("optional for this provider"));
    }

    #[test]
    fn preferred_api_key_env_default_ignores_invalid_configured_secret_literal() {
        let secret = "sk-live-direct-secret-value";
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Openai;
        config.provider.api_key_env = Some(secret.to_owned());

        let default_env = preferred_api_key_env_default(&config);

        assert_eq!(
            default_env, "OPENAI_CODEX_OAUTH_TOKEN",
            "invalid configured credential env values should fall back to the provider's safe onboarding default instead of being reused as the interactive prompt default"
        );
        assert!(
            !default_env.contains(secret),
            "prompt defaults must never echo the rejected secret-like value"
        );
    }

    #[test]
    fn build_onboarding_success_summary_does_not_echo_invalid_credential_env_value() {
        let secret = "sk-live-direct-secret-value";
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Openai;
        config.provider.api_key_env = Some(secret.to_owned());

        let summary =
            build_onboarding_success_summary(Path::new("/tmp/loongclaw.toml"), &config, None);
        let credential = summary
            .credential
            .expect("summary should still describe the configured credential lane");

        assert_eq!(
            credential.value, "environment variable",
            "success summary should redact invalid configured env pointers instead of inventing a provider default binding"
        );
        assert!(
            !credential.value.contains(secret),
            "success summary must never echo invalid secret-like env input: {credential:#?}"
        );
    }

    #[test]
    fn resolve_api_key_env_selection_accepts_explicit_clear_token_in_interactive_mode() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Openai;
        config.provider.api_key = Some(SecretRef::Inline("inline-secret".to_owned()));
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
                web_search_provider: None,
                web_search_api_key_env: None,
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
    fn resolve_api_key_env_selection_reprompts_after_secret_literal_interactively() {
        let secret = "sk-live-direct-secret-value";
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Openai;
        let mut ui = TestOnboardUi::with_inputs([secret, "OPENAI_API_KEY"]);
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
                web_search_provider: None,
                web_search_api_key_env: None,
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
        .expect("interactive credential selection should reprompt on invalid secret-like input");

        assert_eq!(
            selected, "OPENAI_API_KEY",
            "interactive onboarding should reject secret-like input and keep asking for an env var name"
        );
    }

    #[test]
    fn resolve_api_key_env_selection_rejects_secret_literal_non_interactively() {
        let secret = "sk-live-direct-secret-value";
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Openai;
        let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

        let error = resolve_api_key_env_selection(
            &OnboardCommandOptions {
                output: None,
                force: false,
                non_interactive: true,
                accept_risk: true,
                provider: None,
                model: None,
                api_key_env: Some(secret.to_owned()),
                web_search_provider: None,
                web_search_api_key_env: None,
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
        .expect_err("non-interactive onboarding should reject secret-like env selections");

        assert!(
            error.contains("provider.api_key.env"),
            "the validation error should identify the bad field: {error}"
        );
        assert!(
            !error.contains(secret),
            "non-interactive validation must not echo the secret-like input: {error}"
        );
    }

    #[test]
    fn resolve_api_key_env_selection_reprompts_after_uuid_secret_literal_interactively() {
        let secret = uuid_shaped_secret_fixture();
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::VolcengineCoding;
        let mut ui = TestOnboardUi::with_inputs([secret.as_str(), "ARK_API_KEY"]);
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
                web_search_provider: None,
                web_search_api_key_env: None,
                personality: None,
                memory_profile: None,
                system_prompt: None,
                skip_model_probe: false,
            },
            &config,
            "ARK_API_KEY".to_owned(),
            GuidedPromptPath::NativePromptPack,
            &mut ui,
            &context,
        )
        .expect("uuid-shaped credential input should be rejected and reprompted");

        assert_eq!(selected, "ARK_API_KEY");
    }

    #[test]
    fn resolve_api_key_env_selection_rejects_uuid_secret_literal_non_interactively() {
        let secret = uuid_shaped_secret_fixture();
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::VolcengineCoding;
        let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

        let error = resolve_api_key_env_selection(
            &OnboardCommandOptions {
                output: None,
                force: false,
                non_interactive: true,
                accept_risk: true,
                provider: None,
                model: None,
                api_key_env: Some(secret.clone()),
                web_search_provider: None,
                web_search_api_key_env: None,
                personality: None,
                memory_profile: None,
                system_prompt: None,
                skip_model_probe: false,
            },
            &config,
            "ARK_API_KEY".to_owned(),
            GuidedPromptPath::NativePromptPack,
            &mut ui,
            &context,
        )
        .expect_err("uuid-shaped env selections should be rejected non-interactively");

        assert!(error.contains("provider.api_key.env"));
        assert!(!error.contains(secret.as_str()));
    }

    #[test]
    fn resolve_web_search_credential_selection_accepts_clear_token_interactively() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.web_search.default_provider =
            mvp::config::WEB_SEARCH_PROVIDER_TAVILY.to_owned();
        config.tools.web_search.tavily_api_key = Some("${TEAM_TAVILY_KEY}".to_owned());
        let mut ui = TestOnboardUi::with_inputs([":clear"]);
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());
        let options = OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
            accept_risk: true,
            provider: None,
            model: None,
            api_key_env: None,
            web_search_provider: None,
            web_search_api_key_env: None,
            personality: None,
            memory_profile: None,
            system_prompt: None,
            skip_model_probe: false,
        };

        let selected = resolve_web_search_credential_selection(
            &options,
            &config,
            mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
            GuidedPromptPath::NativePromptPack,
            false,
            &mut ui,
            &context,
        )
        .expect("resolve web search credential selection");

        assert_eq!(selected, WebSearchCredentialSelection::ClearConfigured);
    }

    #[test]
    fn resolve_web_search_credential_selection_reprompts_after_secret_literal_interactively() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.web_search.default_provider =
            mvp::config::WEB_SEARCH_PROVIDER_TAVILY.to_owned();
        let mut ui = TestOnboardUi::with_inputs(["sk-live-direct-secret-value", "TEAM_TAVILY_KEY"]);
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());
        let options = OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
            accept_risk: true,
            provider: None,
            model: None,
            api_key_env: None,
            web_search_provider: None,
            web_search_api_key_env: None,
            personality: None,
            memory_profile: None,
            system_prompt: None,
            skip_model_probe: false,
        };

        let selected = resolve_web_search_credential_selection(
            &options,
            &config,
            mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
            GuidedPromptPath::NativePromptPack,
            false,
            &mut ui,
            &context,
        )
        .expect("interactive web search credential selection should reprompt");

        assert_eq!(
            selected,
            WebSearchCredentialSelection::UseEnv("TEAM_TAVILY_KEY".to_owned())
        );
    }

    #[test]
    fn resolve_web_search_credential_selection_keeps_inline_secret_on_blank_input() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.web_search.default_provider =
            mvp::config::WEB_SEARCH_PROVIDER_TAVILY.to_owned();
        config.tools.web_search.tavily_api_key = Some("inline-web-secret".to_owned());
        let mut ui = TestOnboardUi::with_inputs([""]);
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());
        let options = OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
            accept_risk: true,
            provider: None,
            model: None,
            api_key_env: None,
            web_search_provider: None,
            web_search_api_key_env: None,
            personality: None,
            memory_profile: None,
            system_prompt: None,
            skip_model_probe: false,
        };

        let selected = resolve_web_search_credential_selection(
            &options,
            &config,
            mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
            GuidedPromptPath::NativePromptPack,
            false,
            &mut ui,
            &context,
        )
        .expect("blank input should keep current inline web search credential");

        assert_eq!(selected, WebSearchCredentialSelection::KeepCurrent);
    }

    #[test]
    fn apply_selected_web_search_credential_formats_env_reference() {
        let mut config = mvp::config::LoongClawConfig::default();

        apply_selected_web_search_credential(
            &mut config,
            mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
            WebSearchCredentialSelection::UseEnv("TEAM_TAVILY_KEY".to_owned()),
        );

        assert_eq!(
            config.tools.web_search.tavily_api_key.as_deref(),
            Some("${TEAM_TAVILY_KEY}")
        );
    }

    #[test]
    fn recommend_web_search_provider_from_available_credentials_prefers_unique_ready_provider() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.web_search.perplexity_api_key = Some("${PERPLEXITY_API_KEY}".to_owned());

        let mut env = ScopedEnv::new();
        env.set("PERPLEXITY_API_KEY", "perplexity-test-token");

        let recommendation = recommend_web_search_provider_from_available_credentials(&config)
            .expect("a unique ready provider should be recommended");

        assert_eq!(
            recommendation.provider,
            mvp::config::WEB_SEARCH_PROVIDER_PERPLEXITY
        );
        assert_eq!(
            recommendation.source,
            WebSearchProviderRecommendationSource::DetectedCredential
        );
        assert!(
            recommendation.reason.contains("Perplexity Search"),
            "recommendation reason should identify the provider that already has a ready credential: {recommendation:?}"
        );
    }

    #[test]
    fn recommend_web_search_provider_from_available_credentials_returns_none_when_multiple_ready() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.web_search.tavily_api_key = Some("${TAVILY_API_KEY}".to_owned());
        config.tools.web_search.perplexity_api_key = Some("${PERPLEXITY_API_KEY}".to_owned());

        let mut env = ScopedEnv::new();
        env.set("TAVILY_API_KEY", "tavily-test-token");
        env.set("PERPLEXITY_API_KEY", "perplexity-test-token");

        let recommendation = recommend_web_search_provider_from_available_credentials(&config);

        assert_eq!(
            recommendation, None,
            "multiple ready providers should fall back to the environment heuristic instead of relying on an arbitrary hidden priority"
        );
    }

    #[test]
    fn explicit_web_search_provider_override_prefers_cli_option_over_env() {
        let options = OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: false,
            accept_risk: true,
            provider: None,
            model: None,
            api_key_env: None,
            web_search_provider: Some("exa".to_owned()),
            web_search_api_key_env: None,
            personality: None,
            memory_profile: None,
            system_prompt: None,
            skip_model_probe: false,
        };
        let mut env = ScopedEnv::new();
        env.set("LOONGCLAW_WEB_SEARCH_PROVIDER", "tavily");

        let recommendation = explicit_web_search_provider_override(&options)
            .expect("cli override should parse")
            .expect("cli override should win");

        assert_eq!(
            recommendation.provider,
            mvp::config::WEB_SEARCH_PROVIDER_EXA
        );
        assert_eq!(
            recommendation.source,
            WebSearchProviderRecommendationSource::ExplicitCli
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resolve_web_search_provider_selection_keeps_current_provider_on_blank_interactive_input_when_recommendation_differs()
     {
        let options = interactive_onboard_options();
        let mut config = mvp::config::LoongClawConfig::default();
        config.tools.web_search.tavily_api_key = Some("${TAVILY_API_KEY}".to_owned());

        let mut env = ScopedEnv::new();
        env.set("TAVILY_API_KEY", "tavily-test-token");

        let mut ui = TestOnboardUi::with_inputs([""]);
        let context = onboard_test_context();
        let selected = resolve_web_search_provider_selection(
            &options,
            &config,
            GuidedPromptPath::NativePromptPack,
            &mut ui,
            &context,
        )
        .await
        .expect("blank interactive input should keep the current web search provider");

        assert_eq!(
            selected,
            mvp::config::WEB_SEARCH_PROVIDER_DUCKDUCKGO,
            "interactive enter should preserve the current provider even when another provider is recommended"
        );
    }

    #[test]
    fn render_web_search_provider_selection_screen_uses_actual_default_provider_in_footer() {
        let config = mvp::config::LoongClawConfig::default();
        let current_provider = mvp::config::WEB_SEARCH_PROVIDER_DUCKDUCKGO;
        let recommended_provider = mvp::config::WEB_SEARCH_PROVIDER_TAVILY;
        let current_provider_label = web_search_provider_display_name(current_provider);
        let recommended_provider_label = web_search_provider_display_name(recommended_provider);
        let footer_description = format!("keep {current_provider_label}");
        let expected_footer =
            render_default_choice_footer_line("Enter", footer_description.as_str());
        let lines = render_web_search_provider_selection_screen_lines_with_style(
            &config,
            recommended_provider,
            current_provider,
            "found a ready credential",
            GuidedPromptPath::NativePromptPack,
            80,
            false,
        );

        assert!(
            lines
                .iter()
                .any(|line| line == &format!("- current provider: {current_provider_label}")),
            "web search provider screen should show the current provider separately: {lines:#?}"
        );
        assert!(
            lines.iter().any(
                |line| line == &format!("- recommended provider: {recommended_provider_label}")
            ),
            "web search provider screen should show the recommendation separately: {lines:#?}"
        );
        assert!(
            lines.iter().any(|line| line == &expected_footer),
            "web search provider footer should describe the real Enter default instead of the recommendation: {lines:#?}"
        );
    }

    #[test]
    fn resolve_effective_web_search_default_provider_keeps_explicit_non_interactive_provider() {
        let options = OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: true,
            accept_risk: true,
            provider: None,
            model: None,
            api_key_env: None,
            web_search_provider: Some("tavily".to_owned()),
            web_search_api_key_env: None,
            personality: None,
            memory_profile: None,
            system_prompt: None,
            skip_model_probe: false,
        };
        let config = mvp::config::LoongClawConfig::default();
        let recommendation = WebSearchProviderRecommendation {
            provider: mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
            reason: "set by --web-search-provider".to_owned(),
            source: WebSearchProviderRecommendationSource::ExplicitCli,
        };

        let selected =
            resolve_effective_web_search_default_provider(&options, &config, &recommendation);

        assert_eq!(
            selected,
            mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
            "non-interactive onboarding should keep an explicit web-search provider choice instead of silently falling back"
        );
    }

    #[test]
    fn resolve_effective_web_search_default_provider_falls_back_for_detected_tavily_without_credential()
     {
        let options = OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: true,
            accept_risk: true,
            provider: None,
            model: None,
            api_key_env: None,
            web_search_provider: None,
            web_search_api_key_env: None,
            personality: None,
            memory_profile: None,
            system_prompt: None,
            skip_model_probe: false,
        };
        let config = mvp::config::LoongClawConfig::default();
        let recommendation = WebSearchProviderRecommendation {
            provider: mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
            reason: "domestic locale or timezone was detected".to_owned(),
            source: WebSearchProviderRecommendationSource::DetectedSignals,
        };

        let selected =
            resolve_effective_web_search_default_provider(&options, &config, &recommendation);

        assert_eq!(
            selected,
            mvp::config::WEB_SEARCH_PROVIDER_DUCKDUCKGO,
            "detected Tavily recommendations should still fall back to the key-free provider in non-interactive mode when no Tavily credential is ready"
        );
    }

    #[test]
    fn resolve_web_search_credential_selection_uses_explicit_option_non_interactively() {
        let options = OnboardCommandOptions {
            output: None,
            force: false,
            non_interactive: true,
            accept_risk: true,
            provider: None,
            model: None,
            api_key_env: None,
            web_search_provider: Some("tavily".to_owned()),
            web_search_api_key_env: Some("TEAM_TAVILY_KEY".to_owned()),
            personality: None,
            memory_profile: None,
            system_prompt: None,
            skip_model_probe: false,
        };
        let config = mvp::config::LoongClawConfig::default();
        let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

        let selected = resolve_web_search_credential_selection(
            &options,
            &config,
            mvp::config::WEB_SEARCH_PROVIDER_TAVILY,
            GuidedPromptPath::NativePromptPack,
            true,
            &mut ui,
            &context,
        )
        .expect("non-interactive explicit web-search credential env should be accepted");

        assert_eq!(
            selected,
            WebSearchCredentialSelection::UseEnv("TEAM_TAVILY_KEY".to_owned())
        );
    }

    #[test]
    fn apply_selected_api_key_env_routes_openai_oauth_env_to_oauth_binding() {
        let mut provider = mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::Openai,
            api_key: Some(SecretRef::Env {
                env: "OPENAI_API_KEY".to_owned(),
            }),
            ..mvp::config::ProviderConfig::default()
        };

        apply_selected_api_key_env(&mut provider, "OPENAI_CODEX_OAUTH_TOKEN".to_owned());

        assert_eq!(
            provider.oauth_access_token,
            Some(SecretRef::Env {
                env: "OPENAI_CODEX_OAUTH_TOKEN".to_owned(),
            })
        );
        assert_eq!(
            provider.api_key_env, None,
            "switching to the OpenAI oauth env should clear the stale api-key env binding"
        );
        assert_eq!(provider.api_key, None);
    }

    #[test]
    fn apply_selected_api_key_env_routes_unknown_openai_env_to_api_key_binding() {
        let mut provider = mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::Openai,
            oauth_access_token: Some(SecretRef::Env {
                env: "OPENAI_CODEX_OAUTH_TOKEN".to_owned(),
            }),
            ..mvp::config::ProviderConfig::default()
        };

        apply_selected_api_key_env(&mut provider, "OPENAI_ALT_BEARER".to_owned());

        assert_eq!(
            provider.api_key,
            Some(SecretRef::Env {
                env: "OPENAI_ALT_BEARER".to_owned(),
            }),
            "unknown env names should stay on the explicit api-key field instead of being silently rebound to oauth"
        );
        assert_eq!(
            provider.oauth_access_token_env, None,
            "switching to a custom env name should clear the stale oauth binding"
        );
        assert_eq!(provider.oauth_access_token, None);
    }

    #[test]
    fn provider_matches_for_review_ignores_credential_field_explicitness() {
        let current = mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::Openai,
            model: "gpt-4.1".to_owned(),
            api_key: Some(SecretRef::Inline("inline-secret".to_owned())),
            ..mvp::config::ProviderConfig::default()
        };

        let mut api_key_env_update = current.clone();
        apply_selected_api_key_env(&mut api_key_env_update, "OPENAI_API_KEY".to_owned());
        assert_eq!(
            api_key_env_update.api_key,
            Some(SecretRef::Env {
                env: "OPENAI_API_KEY".to_owned(),
            })
        );
        assert!(!api_key_env_update.api_key_env_explicit);
        assert!(
            provider_matches_for_review(&current, &api_key_env_update),
            "review matching should ignore credential binding rewrites when the provider identity is otherwise unchanged"
        );

        let mut oauth_env_update = current.clone();
        apply_selected_api_key_env(&mut oauth_env_update, "OPENAI_CODEX_OAUTH_TOKEN".to_owned());
        assert_eq!(
            oauth_env_update.oauth_access_token,
            Some(SecretRef::Env {
                env: "OPENAI_CODEX_OAUTH_TOKEN".to_owned(),
            })
        );
        assert!(!oauth_env_update.oauth_access_token_env_explicit);
        assert!(
            provider_matches_for_review(&current, &oauth_env_update),
            "review matching should ignore credential binding rewrites when the provider identity is otherwise unchanged"
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
                web_search_provider: None,
                web_search_api_key_env: None,
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
                web_search_provider: None,
                web_search_api_key_env: None,
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
                web_search_provider: None,
                web_search_api_key_env: None,
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
    fn resolve_prompt_addendum_selection_keeps_current_addendum_when_blank_input_is_used() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.cli.system_prompt_addendum = Some("Keep answers direct.".to_owned());
        let mut ui = TestOnboardUi::with_inputs([""]);
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

        let selected = resolve_prompt_addendum_selection(
            &OnboardCommandOptions {
                output: None,
                force: false,
                non_interactive: false,
                accept_risk: true,
                provider: None,
                model: None,
                api_key_env: None,
                web_search_provider: None,
                web_search_api_key_env: None,
                personality: None,
                memory_profile: None,
                system_prompt: None,
                skip_model_probe: false,
            },
            &config,
            &mut ui,
            &context,
        )
        .expect("resolve prompt addendum selection");

        assert_eq!(
            selected.as_deref(),
            Some("Keep answers direct."),
            "blank optional input should keep the current addendum"
        );
    }

    #[test]
    fn resolve_prompt_addendum_selection_uses_allow_empty_prompt_path_for_blank_first_run_input() {
        let config = mvp::config::LoongClawConfig::default();
        let mut ui = AllowEmptyOnlyTestUi::with_inputs([""]);
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

        let selected = resolve_prompt_addendum_selection(
            &OnboardCommandOptions {
                output: None,
                force: false,
                non_interactive: false,
                accept_risk: true,
                provider: None,
                model: None,
                api_key_env: None,
                web_search_provider: None,
                web_search_api_key_env: None,
                personality: None,
                memory_profile: None,
                system_prompt: None,
                skip_model_probe: false,
            },
            &config,
            &mut ui,
            &context,
        )
        .expect("resolve prompt addendum selection");

        assert_eq!(
            selected, None,
            "blank first-run optional input should preserve the absence of an addendum"
        );
    }

    #[test]
    fn resolve_prompt_addendum_selection_uses_allow_empty_prompt_path_for_clear_input() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.cli.system_prompt_addendum = Some("Keep answers direct.".to_owned());
        let mut ui = AllowEmptyOnlyTestUi::with_inputs(["-"]);
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

        let selected = resolve_prompt_addendum_selection(
            &OnboardCommandOptions {
                output: None,
                force: false,
                non_interactive: false,
                accept_risk: true,
                provider: None,
                model: None,
                api_key_env: None,
                web_search_provider: None,
                web_search_api_key_env: None,
                personality: None,
                memory_profile: None,
                system_prompt: None,
                skip_model_probe: false,
            },
            &config,
            &mut ui,
            &context,
        )
        .expect("resolve prompt addendum selection");

        assert_eq!(
            selected, None,
            "allow-empty prompt handling should still respect the explicit clear token"
        );
    }

    #[test]
    fn resolve_prompt_addendum_selection_clears_current_addendum_when_dash_input_is_used() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.cli.system_prompt_addendum = Some("Keep answers direct.".to_owned());
        let mut ui = TestOnboardUi::with_inputs(["-"]);
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

        let selected = resolve_prompt_addendum_selection(
            &OnboardCommandOptions {
                output: None,
                force: false,
                non_interactive: false,
                accept_risk: true,
                provider: None,
                model: None,
                api_key_env: None,
                web_search_provider: None,
                web_search_api_key_env: None,
                personality: None,
                memory_profile: None,
                system_prompt: None,
                skip_model_probe: false,
            },
            &config,
            &mut ui,
            &context,
        )
        .expect("resolve prompt addendum selection");

        assert_eq!(
            selected, None,
            "typing '-' should still clear the current addendum"
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
            web_search_provider: None,
            web_search_api_key_env: None,
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
    fn resolve_provider_selection_keeps_zai_available_in_interactive_list() {
        let config = mvp::config::LoongClawConfig::default();
        let options = interactive_onboard_options();
        let provider_selection = crate::migration::ProviderSelectionPlan::default();
        let context = onboard_test_context();
        let mut ui = TestOnboardUi::with_inputs(["zai"]);

        let selected = resolve_provider_selection(
            &options,
            &config,
            &provider_selection,
            GuidedPromptPath::NativePromptPack,
            &mut ui,
            &context,
        )
        .expect("z.ai should stay selectable in the interactive provider list");

        assert_eq!(selected.kind, mvp::config::ProviderKind::Zai);
        assert_eq!(selected.base_url, "https://api.z.ai");
    }

    #[test]
    fn resolve_provider_selection_preserves_kimi_coding_default_variant() {
        let mut config = mvp::config::LoongClawConfig::default();
        let options = interactive_onboard_options();
        let provider_selection = crate::migration::ProviderSelectionPlan::default();
        let context = onboard_test_context();
        let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
        config.provider =
            mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::KimiCoding);

        let selected = resolve_provider_selection(
            &options,
            &config,
            &provider_selection,
            GuidedPromptPath::NativePromptPack,
            &mut ui,
            &context,
        )
        .expect("default kimi coding selection should stay stable");

        assert_eq!(selected.kind, mvp::config::ProviderKind::KimiCoding);
    }

    #[test]
    fn resolve_provider_selection_preserves_step_plan_default_variant() {
        let mut config = mvp::config::LoongClawConfig::default();
        let options = interactive_onboard_options();
        let provider_selection = crate::migration::ProviderSelectionPlan::default();
        let context = onboard_test_context();
        let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
        config.provider =
            mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::StepPlan);

        let selected = resolve_provider_selection(
            &options,
            &config,
            &provider_selection,
            GuidedPromptPath::NativePromptPack,
            &mut ui,
            &context,
        )
        .expect("default step plan selection should stay stable");

        assert_eq!(selected.kind, mvp::config::ProviderKind::StepPlan);
    }

    #[test]
    fn resolve_provider_selection_preserves_existing_region_endpoint_default() {
        let mut config = mvp::config::LoongClawConfig::default();
        let options = interactive_onboard_options();
        let provider_selection = crate::migration::ProviderSelectionPlan::default();
        let context = onboard_test_context();
        let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
        let global_minimax_base_url = "https://api.minimax.io".to_owned();
        config.provider =
            mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::Minimax);
        config.provider.base_url = global_minimax_base_url.clone();

        let selected = resolve_provider_selection(
            &options,
            &config,
            &provider_selection,
            GuidedPromptPath::NativePromptPack,
            &mut ui,
            &context,
        )
        .expect("region selection should preserve the current endpoint when accepting defaults");

        assert_eq!(selected.kind, mvp::config::ProviderKind::Minimax);
        assert_eq!(selected.base_url, global_minimax_base_url);
    }

    #[test]
    fn resolve_provider_selection_allows_switching_step_plan_region_endpoint() {
        let mut config = mvp::config::LoongClawConfig::default();
        let options = interactive_onboard_options();
        let provider_selection = crate::migration::ProviderSelectionPlan::default();
        let context = onboard_test_context();
        let mut ui = TestOnboardUi::with_inputs(["", "", "2"]);

        config.provider =
            mvp::config::ProviderConfig::fresh_for_kind(mvp::config::ProviderKind::StepPlan);

        let selected = resolve_provider_selection(
            &options,
            &config,
            &provider_selection,
            GuidedPromptPath::NativePromptPack,
            &mut ui,
            &context,
        )
        .expect("step plan region selection should accept the global endpoint");

        assert_eq!(selected.kind, mvp::config::ProviderKind::StepPlan);
        assert_eq!(selected.base_url, "https://api.stepfun.ai");
    }

    #[test]
    fn preinstalled_skills_screen_only_surfaces_the_onboarding_subset() {
        let lines = render_preinstalled_skills_selection_screen_lines_with_style(100, false);
        let joined = lines.join("\n");

        for expected in [
            "systematic-debugging",
            "plan",
            "github-issues",
            "Anthropic Office pack",
            "Minimax Office pack",
        ] {
            assert!(
                joined.contains(expected),
                "expected onboarding preinstall screen to advertise `{expected}`: {joined}"
            );
        }

        for hidden in [
            "native-mcp)",
            "mcporter)",
            "docx)",
            "pdf)",
            "pptx)",
            "xlsx)",
        ] {
            assert!(
                !joined.contains(hidden),
                "did not expect onboarding preinstall screen to advertise `{hidden}`: {joined}"
            );
        }
    }

    #[test]
    fn onboarding_preinstall_targets_are_derived_from_app_registry() {
        let anthropic = mvp::tools::bundled_preinstall_targets()
            .iter()
            .find(|target| target.install_id == "anthropic-office")
            .expect("anthropic office pack should be exposed by app registry");
        assert_eq!(anthropic.skill_ids, &["docx", "pdf", "pptx", "xlsx"]);
    }

    #[test]
    fn resolve_model_selection_prefills_minimax_recommended_model_interactively() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Minimax;
        config.provider.model = "auto".to_owned();
        let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

        let selected = resolve_model_selection(
            &OnboardCommandOptions {
                output: None,
                force: false,
                non_interactive: false,
                accept_risk: true,
                provider: None,
                model: None,
                api_key_env: None,
                web_search_provider: None,
                web_search_api_key_env: None,
                personality: None,
                memory_profile: None,
                system_prompt: None,
                skip_model_probe: false,
            },
            &config,
            GuidedPromptPath::NativePromptPack,
            &[],
            &mut ui,
            &context,
        )
        .expect("resolve model selection");

        assert!(
            selected == "MiniMax-M2.7",
            "interactive onboarding should prefill the provider-recommended explicit model for MiniMax instead of leaving the operator on hidden runtime fallbacks: {selected:?}"
        );
    }

    #[test]
    fn resolve_model_selection_applies_minimax_recommended_model_non_interactively() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Minimax;
        config.provider.model = "auto".to_owned();
        let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

        let selected = resolve_model_selection(
            &OnboardCommandOptions {
                output: None,
                force: false,
                non_interactive: true,
                accept_risk: true,
                provider: None,
                model: None,
                api_key_env: None,
                web_search_provider: None,
                web_search_api_key_env: None,
                personality: None,
                memory_profile: None,
                system_prompt: None,
                skip_model_probe: false,
            },
            &config,
            GuidedPromptPath::NativePromptPack,
            &[],
            &mut ui,
            &context,
        )
        .expect("resolve model selection");

        assert!(
            selected == "MiniMax-M2.7",
            "non-interactive onboarding should use the reviewed provider default for MiniMax instead of carrying auto into preflight: {selected:?}"
        );
    }

    #[test]
    fn resolve_model_selection_prefills_deepseek_recommended_model_interactively() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "auto".to_owned();
        let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

        let selected = resolve_model_selection(
            &OnboardCommandOptions {
                output: None,
                force: false,
                non_interactive: false,
                accept_risk: true,
                provider: None,
                model: None,
                api_key_env: None,
                web_search_provider: None,
                web_search_api_key_env: None,
                personality: None,
                memory_profile: None,
                system_prompt: None,
                skip_model_probe: false,
            },
            &config,
            GuidedPromptPath::NativePromptPack,
            &[],
            &mut ui,
            &context,
        )
        .expect("resolve model selection");

        assert!(
            selected == "deepseek-chat",
            "interactive onboarding should prefill the provider-recommended explicit model for DeepSeek instead of leaving the operator on auto: {selected:?}"
        );
    }

    #[test]
    fn resolve_model_selection_applies_deepseek_recommended_model_non_interactively() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "auto".to_owned();
        let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

        let selected = resolve_model_selection(
            &OnboardCommandOptions {
                output: None,
                force: false,
                non_interactive: true,
                accept_risk: true,
                provider: None,
                model: None,
                api_key_env: None,
                web_search_provider: None,
                web_search_api_key_env: None,
                personality: None,
                memory_profile: None,
                system_prompt: None,
                skip_model_probe: false,
            },
            &config,
            GuidedPromptPath::NativePromptPack,
            &[],
            &mut ui,
            &context,
        )
        .expect("resolve model selection");

        assert!(
            selected == "deepseek-chat",
            "non-interactive onboarding should use the reviewed provider default for DeepSeek instead of carrying auto into preflight: {selected:?}"
        );
    }

    #[test]
    fn resolve_model_selection_prefills_reviewed_model_for_mixed_case_auto_interactively() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "  AUTO  ".to_owned();
        let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

        let selected = resolve_model_selection(
            &OnboardCommandOptions {
                output: None,
                force: false,
                non_interactive: false,
                accept_risk: true,
                provider: None,
                model: None,
                api_key_env: None,
                web_search_provider: None,
                web_search_api_key_env: None,
                personality: None,
                memory_profile: None,
                system_prompt: None,
                skip_model_probe: false,
            },
            &config,
            GuidedPromptPath::NativePromptPack,
            &[],
            &mut ui,
            &context,
        )
        .expect("resolve model selection");

        assert_eq!(
            selected, "deepseek-chat",
            "interactive onboarding should treat mixed-case auto the same as auto when choosing a reviewed provider default"
        );
    }

    #[test]
    fn resolve_model_selection_rejects_blank_explicit_model_non_interactively() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "auto".to_owned();
        let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

        let error = resolve_model_selection(
            &OnboardCommandOptions {
                output: None,
                force: false,
                non_interactive: true,
                accept_risk: true,
                provider: None,
                model: Some("   ".to_owned()),
                api_key_env: None,
                web_search_provider: None,
                web_search_api_key_env: None,
                personality: None,
                memory_profile: None,
                system_prompt: None,
                skip_model_probe: false,
            },
            &config,
            GuidedPromptPath::NativePromptPack,
            &[],
            &mut ui,
            &context,
        )
        .expect_err(
            "blank explicit --model should fail instead of falling back to a recommended model",
        );

        assert_eq!(error, "model cannot be empty");
    }

    #[test]
    fn resolve_model_selection_uses_catalog_choices_when_available_interactively() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "auto".to_owned();
        let mut ui = TestOnboardUi::with_inputs(["2"]);
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());
        let available_models = vec!["deepseek-chat".to_owned(), "deepseek-reasoner".to_owned()];

        let selected = resolve_model_selection(
            &OnboardCommandOptions {
                output: None,
                force: false,
                non_interactive: false,
                accept_risk: true,
                provider: None,
                model: None,
                api_key_env: None,
                web_search_provider: None,
                web_search_api_key_env: None,
                personality: None,
                memory_profile: None,
                system_prompt: None,
                skip_model_probe: false,
            },
            &config,
            GuidedPromptPath::NativePromptPack,
            &available_models,
            &mut ui,
            &context,
        )
        .expect("resolve model selection");

        assert_eq!(
            selected, "deepseek-reasoner",
            "interactive onboarding should use the probed model catalog instead of treating numeric selection input as a literal model id"
        );
    }

    #[test]
    fn resolve_model_selection_keeps_auto_visible_for_noncanonical_volcengine_catalog() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider = mvp::config::ProviderConfig::fresh_for_kind(
            mvp::config::ProviderKind::VolcengineCoding,
        );
        config.provider.base_url =
            "https://proxy.example.com/forward/ark.cn-beijing.volces.com/api/coding/v3".to_owned();
        config.provider.model = "auto".to_owned();
        let mut ui = TestOnboardUi::with_inputs(["1"]);
        let context = onboard_test_context();
        let available_models = vec![
            "doubao-seed-2.0-code".to_owned(),
            "doubao-seed-2.0-pro".to_owned(),
        ];

        let selected = resolve_model_selection(
            &interactive_onboard_options(),
            &config,
            GuidedPromptPath::NativePromptPack,
            &available_models,
            &mut ui,
            &context,
        )
        .expect("resolve model selection");

        assert_eq!(
            selected, "auto",
            "noncanonical Volcengine endpoints should not hide the `auto` choice just because the returned models contain a static-catalog model id"
        );
    }

    #[test]
    fn resolve_model_selection_rejects_blank_custom_override_when_auto_is_hidden() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider = mvp::config::ProviderConfig::fresh_for_kind(
            mvp::config::ProviderKind::VolcengineCoding,
        );
        config.provider.model = "auto".to_owned();
        let mut ui = TestOnboardUi::with_inputs(["3", ""]);
        let context = onboard_test_context();
        let available_models = vec![
            "ark-code-latest".to_owned(),
            "doubao-seed-2.0-code".to_owned(),
        ];

        let error = resolve_model_selection(
            &interactive_onboard_options(),
            &config,
            GuidedPromptPath::NativePromptPack,
            &available_models,
            &mut ui,
            &context,
        )
        .expect_err("blank custom entry should not round-trip hidden auto");

        assert_eq!(error, "model cannot be empty");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn load_onboarding_model_catalog_returns_static_list_for_canonical_volcengine_endpoint() {
        let mut options = interactive_onboard_options();
        options.skip_model_probe = true;
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider = mvp::config::ProviderConfig::fresh_for_kind(
            mvp::config::ProviderKind::VolcengineCoding,
        );

        let models = load_onboarding_model_catalog(&options, &config).await;

        assert_eq!(
            models,
            vec![
                "ark-code-latest".to_owned(),
                "doubao-seed-2.0-code".to_owned(),
                "doubao-seed-2.0-pro".to_owned(),
                "doubao-seed-2.0-lite".to_owned(),
                "doubao-seed-code".to_owned(),
                "minimax-m2.5".to_owned(),
                "glm-4.7".to_owned(),
                "deepseek-v3.2".to_owned(),
                "kimi-k2.5".to_owned(),
            ],
            "the canonical Volcengine Coding endpoint should still use the static onboarding catalog"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn load_onboarding_model_catalog_skips_static_list_for_noncanonical_volcengine_endpoint()
    {
        let mut options = interactive_onboard_options();
        options.skip_model_probe = true;
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider = mvp::config::ProviderConfig::fresh_for_kind(
            mvp::config::ProviderKind::VolcengineCoding,
        );
        config.provider.base_url =
            "https://proxy.example.com/forward/ark.cn-beijing.volces.com/api/coding/v3".to_owned();

        let models = load_onboarding_model_catalog(&options, &config).await;

        assert!(
            models.is_empty(),
            "noncanonical Volcengine endpoints should follow normal probe-skip behavior instead of forcing the hardcoded static catalog: {models:?}"
        );
    }

    #[test]
    fn resolve_model_selection_allows_custom_override_when_catalog_is_available() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Openai;
        config.provider.model = "openai/gpt-5.1-codex".to_owned();
        let mut ui = TestOnboardUi::with_inputs(["2", "openai/gpt-5.2"]);
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());
        let available_models = vec!["openai/gpt-5.1-codex".to_owned()];

        let selected = resolve_model_selection(
            &OnboardCommandOptions {
                output: None,
                force: false,
                non_interactive: false,
                accept_risk: true,
                provider: None,
                model: None,
                api_key_env: None,
                web_search_provider: None,
                web_search_api_key_env: None,
                personality: None,
                memory_profile: None,
                system_prompt: None,
                skip_model_probe: false,
            },
            &config,
            GuidedPromptPath::NativePromptPack,
            &available_models,
            &mut ui,
            &context,
        )
        .expect("resolve model selection");

        assert_eq!(
            selected, "openai/gpt-5.2",
            "interactive onboarding should keep a manual override path even when a searchable model catalog is available"
        );
    }

    #[test]
    fn prompt_onboard_entry_choice_uses_select_widget() {
        let options = vec![
            OnboardEntryOption {
                choice: OnboardEntryChoice::ContinueCurrentSetup,
                label: "continue current setup",
                detail: "reuse current draft".to_owned(),
                recommended: true,
            },
            OnboardEntryOption {
                choice: OnboardEntryChoice::StartFresh,
                label: "start fresh",
                detail: "ignore detected setup".to_owned(),
                recommended: false,
            },
        ];
        let mut ui = SelectOnlyTestUi::with_inputs(["2"]);

        let choice = prompt_onboard_entry_choice(&mut ui, &options)
            .expect("entry choice should route through select_one");

        assert_eq!(choice, OnboardEntryChoice::StartFresh);
    }

    #[test]
    fn prompt_import_candidate_choice_uses_select_widget() {
        let mut ui = SelectOnlyTestUi::with_inputs(["3"]);
        let candidates = vec![
            ImportCandidate {
                source_kind: crate::migration::ImportSourceKind::RecommendedPlan,
                source: "recommended plan".to_owned(),
                config: mvp::config::LoongClawConfig::default(),
                surfaces: Vec::new(),
                domains: Vec::new(),
                channel_candidates: Vec::new(),
                workspace_guidance: Vec::new(),
            },
            ImportCandidate {
                source_kind: crate::migration::ImportSourceKind::CodexConfig,
                source: "codex config".to_owned(),
                config: mvp::config::LoongClawConfig::default(),
                surfaces: Vec::new(),
                domains: Vec::new(),
                channel_candidates: Vec::new(),
                workspace_guidance: Vec::new(),
            },
        ];

        let choice = prompt_import_candidate_choice(&mut ui, &candidates, 80)
            .expect("starting-point choice should route through select_one");

        assert_eq!(choice, None);
    }

    #[test]
    fn prompt_onboard_shortcut_choice_uses_select_widget() {
        let mut ui = SelectOnlyTestUi::with_inputs(["2"]);

        let choice = prompt_onboard_shortcut_choice(&mut ui, OnboardShortcutKind::CurrentSetup)
            .expect("shortcut choice should route through select_one");

        assert_eq!(choice, OnboardShortcutChoice::AdjustSettings);
    }

    #[test]
    fn resolve_write_plan_uses_select_widget_for_existing_config() {
        let temp_dir = std::env::temp_dir().join(format!(
            "loongclaw-onboard-write-plan-{}",
            OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let output_path = temp_dir.join("loongclaw.toml");
        fs::write(&output_path, "provider = 'openai'\n").expect("seed existing config");
        let mut ui = SelectOnlyTestUi::with_inputs(["2"]);
        let context = OnboardRuntimeContext::new_for_tests(80, None, std::iter::empty::<PathBuf>());

        let plan = resolve_write_plan(
            &output_path,
            &OnboardCommandOptions {
                output: None,
                force: false,
                non_interactive: false,
                accept_risk: true,
                provider: None,
                model: None,
                api_key_env: None,
                web_search_provider: None,
                web_search_api_key_env: None,
                personality: None,
                memory_profile: None,
                system_prompt: None,
                skip_model_probe: false,
            },
            &mut ui,
            &context,
        )
        .expect("existing-config confirmation should route through select_one");

        assert!(plan.force);
        assert!(
            plan.backup_path.is_some(),
            "backup selection should preserve the safer write path"
        );
        fs::remove_dir_all(&temp_dir).expect("cleanup temp dir");
    }

    #[test]
    fn prompt_onboard_shortcut_choice_cancels_on_escape_input() {
        let mut ui = TestOnboardUi::with_inputs(["\u{1b}"]);

        let error = prompt_onboard_shortcut_choice(&mut ui, OnboardShortcutKind::CurrentSetup)
            .expect_err("escape input should cancel instead of silently falling through");

        assert!(
            error.contains("cancelled"),
            "escape cancellation should produce a user-facing cancel error: {error}"
        );
    }

    #[test]
    fn test_onboard_ui_prompt_with_default_only_checks_user_input_for_cancel() {
        let mut ui = TestOnboardUi::with_inputs(std::iter::empty::<&str>());

        let value = ui
            .prompt_with_default("Provider", "\u{1b}")
            .expect("missing input should keep the configured default");

        assert_eq!(value, "\u{1b}");
    }

    #[test]
    fn explicit_onboard_cancel_input_requires_escape_byte() {
        assert!(is_explicit_onboard_cancel_input("\u{1b}"));
        assert!(
            !is_explicit_onboard_cancel_input("esc"),
            "literal text should remain valid operator input instead of being treated as an escape keystroke"
        );
        assert!(
            !is_explicit_onboard_cancel_input("ESC"),
            "case variants of plain text should not trigger onboarding cancellation"
        );
    }

    #[test]
    fn literal_esc_text_is_not_treated_as_cancel_input() {
        let value = ensure_onboard_input_not_cancelled("esc".to_owned())
            .expect("literal esc text should remain valid input");

        assert_eq!(value, "esc");
    }

    #[test]
    fn test_onboard_ui_prompt_required_trims_input_like_stdio() {
        let mut ui = TestOnboardUi::with_inputs(["  minimax  "]);

        let value = ui
            .prompt_required("Provider")
            .expect("required prompt should preserve stdio trimming semantics");

        assert_eq!(value, "minimax");
    }

    #[test]
    fn single_line_prompt_capture_drains_follow_up_paste_before_next_prompt() {
        let mut reader = TestPromptLineReader::new(
            [
                OnboardPromptRead::Line("You are helpful.\n".to_owned()),
                OnboardPromptRead::Line("window-plus-summary\n".to_owned()),
            ],
            ["Always be concise.\n"],
        );

        let first = read_single_line_prompt_capture(&mut reader)
            .expect("first prompt capture should succeed");
        let second = read_single_line_prompt_capture(&mut reader)
            .expect("second prompt capture should consume the next real prompt line");

        assert_eq!(first.raw, "You are helpful.\n");
        assert_eq!(first.dropped_line_count, 1);
        assert!(!first.reached_eof);
        assert_eq!(second.raw, "window-plus-summary\n");
        assert_eq!(second.dropped_line_count, 0);
        assert!(!second.reached_eof);
    }

    #[test]
    fn onboard_paste_drain_window_prefers_valid_env_override() {
        let _guard = PasteDrainWindowEnvGuard::set(Some("125"));

        assert_eq!(onboard_paste_drain_window(), Duration::from_millis(125));
    }

    #[test]
    fn onboard_paste_drain_window_falls_back_for_invalid_env_values() {
        let _guard = PasteDrainWindowEnvGuard::set(Some("not-a-number"));

        assert_eq!(
            onboard_paste_drain_window(),
            DEFAULT_ONBOARD_PASTE_DRAIN_WINDOW
        );
    }

    #[test]
    fn onboard_paste_drain_window_rejects_zero_millisecond_override() {
        let _guard = PasteDrainWindowEnvGuard::set(Some("0"));

        assert_eq!(
            onboard_paste_drain_window(),
            DEFAULT_ONBOARD_PASTE_DRAIN_WINDOW
        );
    }

    #[test]
    fn onboard_line_channel_applies_backpressure_after_buffer_limit() {
        let (sender, receiver) = onboard_line_channel_with_capacity(1);
        let second_send_completed = Arc::new(AtomicBool::new(false));
        let completed_flag = Arc::clone(&second_send_completed);
        let producer = thread::spawn(move || {
            sender
                .send(StdioOnboardLineMessage::Line("system prompt\n".to_owned()))
                .expect("send first line");
            sender
                .send(StdioOnboardLineMessage::Line(
                    "follow-up paste\n".to_owned(),
                ))
                .expect("send second line after receiver drains");
            completed_flag.store(true, Ordering::SeqCst);
        });

        for _ in 0..1_000 {
            if second_send_completed.load(Ordering::SeqCst) {
                break;
            }
            thread::yield_now();
        }
        assert!(
            !second_send_completed.load(Ordering::SeqCst),
            "bounded onboarding queue should apply backpressure once the first buffered line is occupied"
        );

        let mut reader = StdioOnboardLineReader::background_from_receiver(receiver);
        let capture = read_single_line_prompt_capture(&mut reader)
            .expect("capture should drain the queued follow-up line");
        producer.join().expect("producer join");

        assert_eq!(capture.raw, "system prompt\n");
        assert_eq!(capture.dropped_line_count, 1);
        assert!(!capture.reached_eof);
        assert!(
            second_send_completed.load(Ordering::SeqCst),
            "receiver drain should unblock the producer once capacity is freed"
        );
    }

    #[test]
    fn stdio_onboard_line_reader_warns_once_when_background_spawn_fails() {
        let mut reader = StdioOnboardLineReader::from_spawn_result(Err(io::Error::other(
            "thread quota exhausted",
        )));

        assert!(
            matches!(reader, StdioOnboardLineReader::Direct { .. }),
            "spawn failure should fall back to direct reads instead of constructing a broken background reader"
        );

        let first_notice = reader
            .take_degraded_notice()
            .expect("spawn failure should surface a degraded-mode notice");
        assert!(
            first_notice.contains("single-line paste draining is disabled"),
            "spawn failure notice should explain the lost hardening: {first_notice}"
        );
        assert_eq!(
            reader.take_degraded_notice(),
            None,
            "degraded-mode notice should only be emitted once per session"
        );
    }

    #[test]
    fn prompt_addendum_screen_mentions_single_line_terminal_input() {
        let lines = render_prompt_addendum_selection_screen_lines(
            &mvp::config::LoongClawConfig::default(),
            80,
        );

        assert!(
            lines.iter().any(|line| line == "- single-line input only"),
            "prompt addendum screen should keep the terminal input note concise: {lines:#?}"
        );
    }

    #[test]
    fn system_prompt_screen_mentions_single_line_terminal_input() {
        let lines = render_system_prompt_selection_screen_lines(
            &mvp::config::LoongClawConfig::default(),
            80,
        );

        assert!(
            lines.iter().any(|line| line == "- single-line input only"),
            "system prompt screen should keep the terminal input note concise: {lines:#?}"
        );
    }

    #[test]
    fn test_onboard_ui_select_one_cancels_on_escape_input() {
        let mut ui = TestOnboardUi::with_inputs(["\u{1b}"]);
        let options = vec![SelectOption {
            label: "OpenAI".to_owned(),
            slug: "openai".to_owned(),
            description: String::new(),
            recommended: true,
        }];

        let error = ui
            .select_one("Provider", &options, Some(0), SelectInteractionMode::List)
            .expect_err("escape input should cancel selection instead of surfacing a parse error");

        assert!(
            error.contains("cancelled"),
            "escape cancellation should stay user-facing for selection prompts: {error}"
        );
    }

    #[test]
    fn validate_select_one_state_rejects_empty_options() {
        let error = validate_select_one_state(0, None)
            .expect_err("select_one should reject empty option lists before prompting");

        assert!(
            error.contains("no selection options"),
            "empty option lists should return a clear error: {error}"
        );
    }

    #[test]
    fn validate_select_one_state_rejects_out_of_bounds_default() {
        let error = validate_select_one_state(2, Some(2))
            .expect_err("select_one should reject a default index that is outside the option list");

        assert!(
            error.contains("default selection index"),
            "invalid default index should be reported clearly: {error}"
        );
    }

    #[test]
    fn default_choice_footer_avoids_bracket_default_syntax() {
        assert_eq!(
            render_default_choice_footer_line("1", "keep current setup"),
            "press Enter to use default 1, keep current setup"
        );
    }

    #[test]
    fn prompt_with_default_text_avoids_bracket_default_syntax() {
        assert_eq!(
            render_prompt_with_default_text("Setup path", "1"),
            "Setup path (default: 1): "
        );
    }

    #[test]
    fn render_onboard_option_lines_avoid_bracketed_choice_tokens() {
        let lines = render_onboard_option_lines(
            &[OnboardScreenOption {
                key: "1".to_owned(),
                label: "Keep current setup".to_owned(),
                detail_lines: vec!["reuse the detected setup".to_owned()],
                recommended: true,
            }],
            80,
        );

        assert!(
            lines
                .iter()
                .any(|line| line.contains("1) Keep current setup (recommended)")),
            "choice rows should present plain option markers instead of bracket wrappers: {lines:#?}"
        );
        assert!(
            lines.iter().all(|line| !line.contains("[1]")),
            "choice rows should not imply that brackets are part of the expected input syntax: {lines:#?}"
        );
    }

    #[test]
    fn render_onboard_option_lines_align_wrapped_labels_with_option_prefix() {
        let lines = render_onboard_option_lines(
            &[OnboardScreenOption {
                key: "friendly_collab".to_owned(),
                label: "friendly collab keeps longer wrapped labels aligned".to_owned(),
                detail_lines: Vec::new(),
                recommended: false,
            }],
            28,
        );
        let continuation = lines
            .iter()
            .find(|line| line.starts_with(' ') && !line.trim().is_empty())
            .expect("wrapped option labels should emit a continuation line");

        assert!(
            continuation.starts_with(
                &" ".repeat(
                    render_onboard_option_prefix("friendly_collab")
                        .chars()
                        .count()
                )
            ),
            "wrapped option labels should continue under the label text instead of snapping back to a fixed indent: {lines:#?}"
        );
    }

    #[test]
    fn interactive_entry_screen_omits_static_options_when_selection_widget_handles_choices() {
        let options = recommended_import_entry_options();
        let lines = render_onboard_entry_interactive_screen_lines_with_style(
            crate::migration::CurrentSetupState::Absent,
            None,
            &[],
            &options,
            None,
            80,
            false,
        );

        assert!(
            lines
                .iter()
                .any(|line| line == crate::onboard_presentation::entry_choice_section_heading()),
            "interactive entry screen should keep the section heading even when the chooser renders options separately: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .all(|line| !line.contains("Continue current setup")),
            "interactive entry screen should not duplicate option labels before the selection widget renders them: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .all(|line| !line.contains("press Enter to use default")),
            "interactive entry screen should omit the redundant static default footer: {lines:#?}"
        );
    }

    #[test]
    fn interactive_starting_point_screen_omits_static_options_when_selection_widget_handles_choices()
     {
        let candidate = ImportCandidate {
            source_kind: crate::migration::ImportSourceKind::CodexConfig,
            source: "Codex config at ~/.codex/config.toml".to_owned(),
            config: mvp::config::LoongClawConfig::default(),
            surfaces: Vec::new(),
            domains: Vec::new(),
            channel_candidates: Vec::new(),
            workspace_guidance: Vec::new(),
        };
        let lines =
            render_starting_point_selection_header_lines_with_style(&[candidate], 80, false);

        assert!(
            lines
                .iter()
                .any(|line| line == crate::onboard_presentation::starting_point_selection_title()),
            "interactive starting-point screen should keep the title even when choices render separately: {lines:#?}"
        );
        assert!(
            lines.iter().all(|line| !line.contains("(recommended)")),
            "interactive starting-point screen should not duplicate static choice rows before the selection widget renders them: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .all(|line| !line.contains("press Enter to use default")),
            "interactive starting-point screen should omit the redundant static default footer: {lines:#?}"
        );
    }

    #[test]
    fn interactive_existing_config_write_screen_omits_static_options_when_selection_widget_handles_choices()
     {
        let lines = render_existing_config_write_header_lines_with_style(
            "/tmp/loongclaw-config.toml",
            80,
            false,
        );

        assert!(
            lines.iter().any(|line| line == "existing config found"),
            "interactive existing-config screen should keep its heading: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .all(|line| !line.contains("Replace existing config")),
            "interactive existing-config screen should let the selection widget own the actual options: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .all(|line| !line.contains("press Enter to use default")),
            "interactive existing-config screen should omit the redundant static default footer: {lines:#?}"
        );
    }

    #[test]
    fn stdio_onboard_ui_starts_without_initializing_line_reader() {
        let ui = StdioOnboardUi::default();

        assert!(
            ui.line_reader.is_none(),
            "stdio ui should not create a stdin reader until the stdio fallback path is actually used"
        );
    }

    #[test]
    fn parse_select_one_input_accepts_custom_alias_for_custom_model_option() {
        let options = vec![
            SelectOption {
                label: "gpt-5.2".to_owned(),
                slug: "openai/gpt-5.2".to_owned(),
                description: String::new(),
                recommended: true,
            },
            SelectOption {
                label: "enter custom model id".to_owned(),
                slug: ONBOARD_CUSTOM_MODEL_OPTION_SLUG.to_owned(),
                description: String::new(),
                recommended: false,
            },
        ];

        assert_eq!(parse_select_one_input("custom", &options), Some(1));
        assert_eq!(
            parse_select_one_input(ONBOARD_CUSTOM_MODEL_OPTION_SLUG, &options),
            Some(1),
            "the internal sentinel may still appear in older scripted flows and should stay backward compatible"
        );
    }

    #[test]
    fn render_select_one_invalid_input_message_hides_internal_custom_model_slug() {
        let options = vec![
            SelectOption {
                label: "gpt-5.2".to_owned(),
                slug: "openai/gpt-5.2".to_owned(),
                description: String::new(),
                recommended: true,
            },
            SelectOption {
                label: "enter custom model id".to_owned(),
                slug: ONBOARD_CUSTOM_MODEL_OPTION_SLUG.to_owned(),
                description: String::new(),
                recommended: false,
            },
        ];

        let message = render_select_one_invalid_input_message(&options);
        assert!(
            message.contains("custom"),
            "invalid-input help should surface a friendly custom alias: {message}"
        );
        assert!(
            !message.contains(ONBOARD_CUSTOM_MODEL_OPTION_SLUG),
            "invalid-input help must not leak the internal custom sentinel: {message}"
        );
    }

    #[test]
    fn test_onboard_ui_select_one_accepts_slug_input() {
        let mut ui = TestOnboardUi::with_inputs(["friendly_collab"]);
        let options = vec![
            SelectOption {
                label: "calm engineering".to_owned(),
                slug: "calm_engineering".to_owned(),
                description: String::new(),
                recommended: true,
            },
            SelectOption {
                label: "friendly collab".to_owned(),
                slug: "friendly_collab".to_owned(),
                description: String::new(),
                recommended: false,
            },
        ];

        let index = ui
            .select_one(
                "Personality",
                &options,
                Some(0),
                SelectInteractionMode::List,
            )
            .expect("test ui should stay aligned with shared slug-selection behavior");

        assert_eq!(index, 1);
    }

    #[test]
    fn resolve_select_one_eof_returns_default_when_available() {
        let idx = resolve_select_one_eof(Some(1)).expect("EOF should fall back to the default");
        assert_eq!(idx, 1);
    }

    #[test]
    fn resolve_select_one_eof_errors_when_selection_is_required() {
        let error = resolve_select_one_eof(None)
            .expect_err("EOF without a default should terminate instead of looping forever");

        assert!(
            error.contains("stdin closed"),
            "required selections should surface EOF as a terminal error: {error}"
        );
    }

    #[test]
    fn shortcut_screen_footer_mentions_escape_cancel() {
        let lines = render_continue_current_setup_screen_lines(
            &mvp::config::LoongClawConfig::default(),
            80,
        );

        assert!(
            lines
                .iter()
                .any(|line| line.contains("Esc") && line.contains("cancel")),
            "choice screens should teach the exit gesture explicitly: {lines:#?}"
        );
    }

    #[test]
    fn shortcut_header_footer_mentions_escape_cancel() {
        let lines = render_onboard_shortcut_header_lines_with_style(
            OnboardShortcutKind::CurrentSetup,
            &mvp::config::LoongClawConfig::default(),
            None,
            80,
            false,
        );

        assert!(
            lines
                .iter()
                .any(|line| line.contains("Esc") && line.contains("cancel")),
            "header-only shortcut screens should keep the exit gesture visible before the chooser opens: {lines:#?}"
        );
    }

    #[test]
    fn detected_shortcut_snapshot_wraps_starting_point_like_review_rows() {
        let config = mvp::config::LoongClawConfig::default();
        let import_source =
            "Codex config at /very/long/path/to/a/workspace/with/a/deeply/nested/config.toml";
        let expected_label = onboard_starting_point_label(None, import_source);
        let expected_lines =
            mvp::presentation::render_wrapped_text_line("- starting point: ", &expected_label, 48);
        let lines = render_onboard_shortcut_screen_lines_with_style(
            OnboardShortcutKind::DetectedSetup,
            &config,
            Some(import_source),
            48,
            false,
        );

        for expected_line in expected_lines {
            assert!(
                lines.iter().any(|line| line == &expected_line),
                "detected shortcut snapshots should wrap the starting-point row with the same helper used by the review digest: {lines:#?}"
            );
        }
    }

    #[test]
    fn preflight_summary_screen_footer_mentions_escape_cancel() {
        let checks = vec![OnboardCheck {
            name: "provider model probe",
            level: OnboardCheckLevel::Warn,
            detail: "catalog probe failed".to_owned(),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        }];

        let lines = render_preflight_summary_screen_lines(&checks, 80);

        assert!(
            lines
                .iter()
                .any(|line| line.contains("Esc") && line.contains("cancel")),
            "interactive preflight review should teach the exit gesture explicitly: {lines:#?}"
        );
    }

    #[test]
    fn preflight_summary_uses_explicit_model_guidance_for_reviewed_auto_failures() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Deepseek;
        config.provider.model = "auto".to_owned();

        let check = provider_model_probe_failure_check(
            &config,
            "provider rejected the model list".to_owned(),
        );
        let lines = render_preflight_summary_screen_lines(&[check], 80);

        assert!(
            lines.iter().any(|line| {
                line.contains("rerun onboarding to choose a reviewed model")
                    || line.contains("set provider.model / preferred_models explicitly")
            }),
            "reviewed auto-model failures should keep the explicit-model remediation visible in the summary: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .all(|line| !line.contains("--skip-model-probe")),
            "reviewed auto-model failures should not suggest --skip-model-probe because that contradicts the explicit-model recovery path: {lines:#?}"
        );
    }

    #[test]
    fn preflight_summary_uses_explicit_model_only_guidance_without_reviewed_default() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Custom;
        config.provider.model = "auto".to_owned();

        let check = provider_model_probe_failure_check(
            &config,
            "provider rejected the model list".to_owned(),
        );
        let lines = render_preflight_summary_screen_lines(&[check], 80);

        assert!(
            lines.iter().any(|line| {
                line == crate::onboard_presentation::preflight_explicit_model_only_rerun_hint()
            }),
            "providers without a reviewed model should keep the summary hint aligned with the explicit-model-only recovery path: {lines:#?}"
        );
        assert!(
            lines
                .iter()
                .all(|line| !line.contains("choose a reviewed model")),
            "providers without a reviewed model should not advertise a reviewed-model recovery path that does not exist: {lines:#?}"
        );
    }

    #[test]
    fn preflight_summary_omits_skip_model_probe_rerun_hint_after_probe_is_already_skipped() {
        let lines = render_preflight_summary_screen_lines(
            &[OnboardCheck {
                name: "provider model probe",
                level: OnboardCheckLevel::Warn,
                detail: "skipped by --skip-model-probe".to_owned(),
                non_interactive_warning_policy:
                    OnboardNonInteractiveWarningPolicy::AcceptedBySkipModelProbe,
            }],
            80,
        );

        assert!(
            lines.iter().all(|line| {
                line.as_str() != crate::onboard_presentation::preflight_probe_rerun_hint()
            }),
            "preflight should not suggest rerunning with --skip-model-probe after the current run already skipped the probe: {lines:#?}"
        );
    }

    #[test]
    fn entry_screen_footer_mentions_escape_cancel() {
        let options = build_onboard_entry_options(crate::migration::CurrentSetupState::Absent, &[]);
        let lines = render_onboard_entry_screen_lines(
            crate::migration::CurrentSetupState::Absent,
            None,
            &[],
            &options,
            None,
            80,
        );

        assert!(
            lines
                .iter()
                .any(|line| line.contains("Esc") && line.contains("cancel")),
            "interactive entry selection should teach the exit gesture explicitly: {lines:#?}"
        );
    }

    #[test]
    fn write_confirmation_screen_footer_mentions_escape_cancel() {
        let lines = render_write_confirmation_screen_lines("/tmp/loongclaw.toml", false, 80);

        assert!(
            lines
                .iter()
                .any(|line| line.contains("Esc") && line.contains("cancel")),
            "write confirmation should teach the exit gesture explicitly: {lines:#?}"
        );
    }

    #[test]
    fn append_escape_cancel_hint_dedupes_case_insensitively() {
        let footer_lines = append_escape_cancel_hint(vec![
            "- press esc then enter to cancel onboarding".to_owned(),
        ]);

        assert_eq!(
            footer_lines,
            vec!["- press esc then enter to cancel onboarding".to_owned()],
            "case-only changes should not duplicate the escape cancel footer: {footer_lines:#?}"
        );
    }

    #[test]
    fn model_selection_screen_tells_users_to_type_auto_for_fallbacks() {
        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.kind = mvp::config::ProviderKind::Minimax;
        config.provider.model = "auto".to_owned();
        config.provider.preferred_models = vec!["MiniMax-M2.5".to_owned()];

        let lines = render_model_selection_screen_lines_with_default(&config, "MiniMax-M2.7", 80);
        let rendered = lines.join("\n");

        assert!(
            rendered.contains("type `auto`")
                && rendered.contains("configured preferred fallbacks first")
                && rendered.contains("MiniMax-M2.5"),
            "explicit prefill flows should tell users to type `auto` when they want configured fallback behavior: {lines:#?}"
        );
        assert!(
            !rendered.contains("leave `auto`"),
            "explicit prefill flows should not imply Enter keeps `auto`: {lines:#?}"
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
