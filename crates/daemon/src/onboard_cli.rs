use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use dialoguer::console::{Term, user_attended};
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Error as DialoguerError, FuzzySelect, Input, Select};
use loongclaw_app as mvp;
use loongclaw_spec::CliResult;
use time::OffsetDateTime;
use time::format_description::FormatItem;
use time::macros::format_description;

const BACKUP_TIMESTAMP_FORMAT: &[FormatItem<'static>] =
    format_description!("[year][month][day]-[hour][minute][second]");
const ONBOARD_CLEAR_INPUT_TOKEN: &str = ":clear";
const ONBOARD_CUSTOM_MODEL_OPTION_SLUG: &str = "__custom_model__";
const ONBOARD_ESCAPE_CANCEL_HINT: &str = "- press Esc then Enter to cancel onboarding";
const ONBOARD_SINGLE_LINE_INPUT_HINT: &str = "- single-line input only";
const ONBOARD_PASTE_DRAIN_WINDOW_ENV: &str = "LOONGCLAW_ONBOARD_PASTE_DRAIN_WINDOW_MS";
const DEFAULT_ONBOARD_PASTE_DRAIN_WINDOW: Duration = Duration::from_millis(75);
const ONBOARD_LINE_READER_BUFFER_SIZE: usize = 64;

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
struct StdioOnboardUi {
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
    AcceptedByPreferredModels,
    RequiresExplicitModel,
    RequiresExplicitModelWithoutReviewedDefault,
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
    pub provider_endpoint: Option<String>,
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
            let credential_hint = provider_credential_env_hint(&config.provider)
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
    let provider_kinds = mvp::config::ProviderKind::all_sorted();
    let select_options: Vec<SelectOption> = provider_kinds
        .iter()
        .map(|kind| SelectOption {
            label: provider_kind_display_name(*kind).to_owned(),
            slug: provider_kind_id(*kind).to_owned(),
            description: String::new(),
            recommended: *kind == default_provider_kind,
        })
        .collect();
    let default_idx = if provider_selection.requires_explicit_choice {
        None
    } else {
        provider_kinds
            .iter()
            .position(|kind| *kind == default_provider_kind)
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
    let kind = *provider_kinds
        .get(idx)
        .ok_or_else(|| format!("provider selection index {idx} out of range"))?;
    Ok(resolve_provider_config_from_selection(
        &config.provider,
        provider_selection,
        kind,
    ))
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
    if let Some(model) = options.model.as_deref()
        && model.trim().is_empty()
    {
        return Err("model cannot be empty".to_owned());
    }

    if options.non_interactive {
        if let Some(model) = options.model.as_deref() {
            return Ok(model.trim().to_owned());
        }
        return Ok(config.provider.configured_model_value());
    }

    let default_model = resolve_onboarding_model_prompt_default(options, config);
    print_lines(
        ui,
        render_model_selection_screen_lines_with_style(
            config,
            default_model.as_str(),
            guided_prompt_path,
            context.render_width,
            true,
            !available_models.is_empty(),
        ),
    )?;
    if !available_models.is_empty() {
        let (select_options, default_idx) =
            build_model_selection_options(default_model.as_str(), available_models);
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
        let custom_model = ui.prompt_with_default("Custom model id", default_model.as_str())?;
        let trimmed = custom_model.trim();
        if trimmed.is_empty() {
            return Err("model cannot be empty".to_owned());
        }
        return Ok(trimmed.to_owned());
    }
    let value = ui.prompt_with_default("Model", default_model.as_str())?;
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
    if options.non_interactive || options.skip_model_probe {
        return Vec::new();
    }
    if !mvp::provider::provider_auth_ready(config).await {
        return Vec::new();
    }
    mvp::provider::fetch_available_models(config)
        .await
        .unwrap_or_default()
}

fn build_model_selection_options(
    default_model: &str,
    available_models: &[String],
) -> (Vec<SelectOption>, Option<usize>) {
    let default_model = default_model.trim();
    let mut models = Vec::new();
    if !default_model.is_empty() {
        models.push(default_model.to_owned());
    }
    for model in available_models {
        let trimmed = model.trim();
        if trimmed.is_empty() || models.iter().any(|existing| existing == trimmed) {
            continue;
        }
        models.push(trimmed.to_owned());
    }

    let mut options = models
        .iter()
        .map(|model| SelectOption {
            label: model.clone(),
            slug: model.clone(),
            description: if model == default_model {
                "current or suggested default".to_owned()
            } else {
                String::new()
            },
            recommended: model == default_model,
        })
        .collect::<Vec<_>>();
    let default_idx = options
        .iter()
        .position(|option| option.slug == default_model);
    options.push(SelectOption {
        label: "enter custom model id".to_owned(),
        slug: ONBOARD_CUSTOM_MODEL_OPTION_SLUG.to_owned(),
        description: "manually type any provider model id".to_owned(),
        recommended: false,
    });

    (options, default_idx)
}

fn resolve_onboarding_model_prompt_default(
    options: &OnboardCommandOptions,
    config: &mvp::config::LoongClawConfig,
) -> String {
    if let Some(model) = options.model.as_deref() {
        return model.trim().to_owned();
    }

    if let Some(model) = config.provider.explicit_model() {
        return model;
    }

    let configured_model = config.provider.configured_model_value();
    if configured_model.eq_ignore_ascii_case("auto")
        && let Some(model) = config.provider.kind.recommended_onboarding_model()
    {
        return model.to_owned();
    }

    configured_model
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
    let example_env_name = provider_credential_env_hint(&config.provider)
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
        provider.set_api_key_env(None);
        provider.set_oauth_access_token_env(None);
        return;
    }

    provider.api_key = None;
    provider.oauth_access_token = None;
    match selected_provider_credential_env_field(provider, selected_api_key_env) {
        ProviderCredentialEnvField::ApiKey => {
            provider.set_oauth_access_token_env(None);
            provider.set_api_key_env(Some(selected_api_key_env.to_owned()));
        }
        ProviderCredentialEnvField::OAuthAccessToken => {
            provider.set_api_key_env(None);
            provider.set_oauth_access_token_env(Some(selected_api_key_env.to_owned()));
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

async fn run_preflight_checks(
    config: &mvp::config::LoongClawConfig,
    skip_model_probe: bool,
) -> Vec<OnboardCheck> {
    let mut checks = Vec::new();
    if let Some(validation_check) = config_validation_check(config) {
        checks.push(validation_check);
    }
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
            Err(error) => {
                let transport_style_failure =
                    crate::provider_route_diagnostics::is_transport_style_model_probe_failure(
                        error.as_str(),
                    );
                checks.push(provider_model_probe_failure_check(config, error));
                if transport_style_failure
                    && let Some(route_probe) =
                        crate::provider_route_diagnostics::collect_provider_route_probe(
                            &config.provider,
                        )
                        .await
                {
                    checks.push(provider_route_probe_preflight_check(&route_probe));
                }
            }
        }
    }

    let sqlite_path = config.memory.resolved_sqlite_path();
    let sqlite_parent = sqlite_path.parent().unwrap_or(Path::new("."));
    checks.push(directory_preflight_check("memory path", sqlite_parent));

    let file_root = config.tools.resolved_file_root();
    checks.push(directory_preflight_check("tool file root", &file_root));

    checks.extend(collect_browser_companion_preflight_checks(config).await);
    checks.extend(collect_channel_preflight_checks(config));

    checks
}

fn config_validation_check(config: &mvp::config::LoongClawConfig) -> Option<OnboardCheck> {
    config.validate().err().map(|detail| OnboardCheck {
        name: "config validation",
        level: OnboardCheckLevel::Fail,
        detail,
        non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
    })
}

fn provider_check_detail_prefix(config: &mvp::config::LoongClawConfig) -> String {
    crate::provider_presentation::active_provider_detail_label(config)
}

fn render_onboard_model_candidate_list(models: &[String]) -> String {
    models
        .iter()
        .map(|model| format!("`{model}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn provider_model_probe_failure_check(
    config: &mvp::config::LoongClawConfig,
    error: String,
) -> OnboardCheck {
    let provider_prefix = provider_check_detail_prefix(config);
    if crate::provider_route_diagnostics::is_transport_style_model_probe_failure(error.as_str()) {
        return OnboardCheck {
            name: "provider model probe",
            level: OnboardCheckLevel::Fail,
            detail: format!(
                "{provider_prefix}: {} ({error}); runtime could not verify the provider route. inspect provider route diagnostics and retry once dns / proxy / TUN routing is stable",
                crate::provider_route_diagnostics::MODEL_CATALOG_TRANSPORT_FAILED_MARKER
            ),
            non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
        };
    }
    let auth_style_failure = mvp::provider::is_auth_style_failure_message(error.as_str());
    let append_region_hint = |mut detail: String| {
        if auth_style_failure && let Some(hint) = config.provider.region_endpoint_failure_hint() {
            detail.push(' ');
            detail.push_str(hint.as_str());
        }
        detail
    };
    let (level, detail, non_interactive_warning_policy) = match config
        .provider
        .model_catalog_probe_recovery()
    {
        mvp::config::ModelCatalogProbeRecovery::ExplicitModel(model) => (
            OnboardCheckLevel::Warn,
            append_region_hint(format!(
                "{provider_prefix}: model catalog probe failed ({error}); chat may still work because model `{model}` is explicitly configured"
            )),
            OnboardNonInteractiveWarningPolicy::AcceptedByExplicitModel,
        ),
        mvp::config::ModelCatalogProbeRecovery::ConfiguredPreferredModels(fallback_models) => (
            OnboardCheckLevel::Warn,
            append_region_hint(format!(
                "{provider_prefix}: model catalog probe failed ({error}); runtime will try configured preferred model fallback(s): {}",
                render_onboard_model_candidate_list(&fallback_models)
            )),
            OnboardNonInteractiveWarningPolicy::AcceptedByPreferredModels,
        ),
        mvp::config::ModelCatalogProbeRecovery::RequiresExplicitModel {
            recommended_onboarding_model,
        } => (
            OnboardCheckLevel::Fail,
            append_region_hint(provider_model_probe_requires_explicit_model_detail(
                provider_prefix.as_str(),
                error.as_str(),
                recommended_onboarding_model,
            )),
            if recommended_onboarding_model.is_some() {
                OnboardNonInteractiveWarningPolicy::RequiresExplicitModel
            } else {
                OnboardNonInteractiveWarningPolicy::RequiresExplicitModelWithoutReviewedDefault
            },
        ),
    };

    OnboardCheck {
        name: "provider model probe",
        level,
        detail,
        non_interactive_warning_policy,
    }
}

async fn collect_browser_companion_preflight_checks(
    config: &mvp::config::LoongClawConfig,
) -> Vec<OnboardCheck> {
    let Some(diagnostics) =
        crate::browser_companion_diagnostics::collect_browser_companion_diagnostics(config).await
    else {
        return Vec::new();
    };

    let level = if diagnostics.install_ready() && diagnostics.runtime_ready {
        OnboardCheckLevel::Pass
    } else {
        OnboardCheckLevel::Warn
    };
    let detail = if diagnostics.install_ready() {
        diagnostics
            .runtime_gate_detail()
            .unwrap_or_else(|| diagnostics.install_detail())
    } else {
        diagnostics.install_detail()
    };

    vec![OnboardCheck {
        name: crate::browser_companion_diagnostics::BROWSER_COMPANION_INSTALL_CHECK_NAME,
        level,
        detail,
        non_interactive_warning_policy: OnboardNonInteractiveWarningPolicy::Block,
    }]
}

fn provider_model_probe_requires_explicit_model_detail(
    provider_prefix: &str,
    error: &str,
    recommended_onboarding_model: Option<&str>,
) -> String {
    match recommended_onboarding_model {
        Some(model) => format!(
            "{provider_prefix}: model catalog probe failed ({error}); current config still uses `model = auto`; rerun onboarding and accept reviewed model `{model}`, or set `provider.model` / `preferred_models` explicitly"
        ),
        None => format!(
            "{provider_prefix}: model catalog probe failed ({error}); current config still uses `model = auto`; set `provider.model` explicitly or configure `preferred_models` before retrying"
        ),
    }
}

fn non_interactive_preflight_failure_message(checks: &[OnboardCheck]) -> String {
    let detail = checks
        .iter()
        .find(|check| check.level == OnboardCheckLevel::Fail)
        .map(|check| {
            let mut detail = check.detail.clone();
            if check.name == "provider model probe"
                && check.detail.contains(
                    crate::provider_route_diagnostics::MODEL_CATALOG_TRANSPORT_FAILED_MARKER,
                )
                && let Some(route_probe) = checks.iter().find(|candidate| {
                    candidate.name
                        == crate::provider_route_diagnostics::PROVIDER_ROUTE_PROBE_CHECK_NAME
                })
            {
                detail.push_str(" provider route probe: ");
                detail.push_str(route_probe.detail.as_str());
            }
            detail
        })
        .unwrap_or_else(|| "preflight checks failed".to_owned());
    format!("onboard preflight failed: {detail}")
}

fn config_validation_failure_message(checks: &[OnboardCheck]) -> Option<String> {
    checks
        .iter()
        .find(|check| check.name == "config validation" && check.level == OnboardCheckLevel::Fail)
        .map(|check| format!("onboard preflight failed: {}", check.detail))
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

    let mut detail = provider_credential_env_hint(provider)
        .map(|env_name| format!("{env_name} is not set"))
        .unwrap_or_else(|| "provider credentials are not configured".to_owned());
    if let Some(hint) = provider.auth_guidance_hint() {
        detail.push(' ');
        detail.push_str(hint.as_str());
    }
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

fn provider_route_probe_preflight_check(
    probe: &crate::provider_route_diagnostics::ProviderRouteProbe,
) -> OnboardCheck {
    OnboardCheck {
        name: crate::provider_route_diagnostics::PROVIDER_ROUTE_PROBE_CHECK_NAME,
        level: match probe.level {
            crate::provider_route_diagnostics::ProviderRouteProbeLevel::Pass => {
                OnboardCheckLevel::Pass
            }
            crate::provider_route_diagnostics::ProviderRouteProbeLevel::Warn => {
                OnboardCheckLevel::Warn
            }
            crate::provider_route_diagnostics::ProviderRouteProbeLevel::Fail => {
                OnboardCheckLevel::Fail
            }
        },
        detail: probe.detail.clone(),
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
                | OnboardNonInteractiveWarningPolicy::AcceptedByPreferredModels
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

fn configured_provider_credential_env_binding(
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
}

fn provider_has_inline_credential(provider: &mvp::config::ProviderConfig) -> bool {
    provider
        .api_key
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        || provider
            .oauth_access_token
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
}

fn selected_provider_credential_env_field(
    provider: &mvp::config::ProviderConfig,
    selected_env_name: &str,
) -> ProviderCredentialEnvField {
    let normalized = normalize_provider_credential_env_name(selected_env_name);
    let matches_oauth = normalized.as_deref().is_some_and(|env_name| {
        provider.kind.default_oauth_access_token_env() == Some(env_name)
            || provider
                .kind
                .oauth_access_token_env_aliases()
                .contains(&env_name)
            || provider
                .oauth_access_token_env
                .as_deref()
                .and_then(normalize_provider_credential_env_name)
                .as_deref()
                == Some(env_name)
    });
    let matches_api_key = normalized.as_deref().is_some_and(|env_name| {
        provider.kind.default_api_key_env() == Some(env_name)
            || provider.kind.api_key_env_aliases().contains(&env_name)
            || provider
                .api_key_env
                .as_deref()
                .and_then(normalize_provider_credential_env_name)
                .as_deref()
                == Some(env_name)
    });

    match (matches_oauth, matches_api_key) {
        (true, false) => ProviderCredentialEnvField::OAuthAccessToken,
        (false, true) => ProviderCredentialEnvField::ApiKey,
        (true, true) => configured_provider_credential_env_binding(provider)
            .or_else(|| preferred_provider_credential_env_binding(provider))
            .map(|binding| binding.field)
            .unwrap_or(ProviderCredentialEnvField::ApiKey),
        (false, false) => ProviderCredentialEnvField::ApiKey,
    }
}

fn push_provider_credential_env_hint(hints: &mut Vec<String>, maybe_env_name: Option<&str>) {
    let Some(env_name) = maybe_env_name.and_then(normalize_provider_credential_env_name) else {
        return;
    };
    if !hints.iter().any(|existing| existing == &env_name) {
        hints.push(env_name);
    }
}

fn provider_credential_env_name_is_safe(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }

    let mut config = mvp::config::LoongClawConfig::default();
    config.provider.api_key = None;
    config.provider.api_key_env = Some(trimmed.to_owned());
    config.validate().is_ok()
}

fn normalize_provider_credential_env_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || !provider_credential_env_name_is_safe(trimmed) {
        return None;
    }
    Some(trimmed.to_owned())
}

fn render_provider_credential_source_value(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim();
    if trimmed.is_empty() {
        return None;
    }
    normalize_provider_credential_env_name(trimmed)
        .or_else(|| Some("environment variable".to_owned()))
}

fn render_configured_provider_credential_source_value(
    provider: &mvp::config::ProviderConfig,
) -> Option<String> {
    provider
        .oauth_access_token_env
        .as_deref()
        .and_then(|value| render_provider_credential_source_value(Some(value)))
        .or_else(|| {
            provider
                .api_key_env
                .as_deref()
                .and_then(|value| render_provider_credential_source_value(Some(value)))
        })
}

fn provider_has_configured_credential_env(provider: &mvp::config::ProviderConfig) -> bool {
    provider
        .oauth_access_token_env
        .as_deref()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
        || provider
            .api_key_env
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty())
}

pub fn preferred_api_key_env_default(config: &mvp::config::LoongClawConfig) -> String {
    let provider = &config.provider;
    if let Some(binding) = configured_provider_credential_env_binding(provider) {
        return binding.env_name;
    }
    if provider_has_inline_credential(provider) {
        return String::new();
    }
    preferred_provider_credential_env_binding(provider)
        .map(|binding| binding.env_name)
        .unwrap_or_default()
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
    let recommended_plan_available = import_candidates.iter().any(|candidate| {
        candidate.source_kind == crate::migration::ImportSourceKind::RecommendedPlan
    });
    let mut lines = render_onboard_header(
        OnboardHeaderStyle::Compact,
        width,
        "guided setup for provider, channels, and workspace guidance",
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
    let screen_options = build_onboard_entry_screen_options(options);
    lines.extend(render_onboard_option_lines(&screen_options, width));
    let footer_lines = append_escape_cancel_hint(
        render_onboard_entry_default_choice_footer_line(options)
            .into_iter()
            .collect::<Vec<_>>(),
    );
    if !footer_lines.is_empty() {
        lines.push(String::new());
        lines.extend(render_onboard_wrapped_display_lines(footer_lines, width));
    }
    lines
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
    let recommended_plan_available = import_candidates.iter().any(|candidate| {
        candidate.source_kind == crate::migration::ImportSourceKind::RecommendedPlan
    });
    let mut lines = render_onboard_header(
        OnboardHeaderStyle::Compact,
        width,
        "guided setup for provider, channels, and workspace guidance",
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
    if !options.is_empty() {
        lines.push(String::new());
        lines.push(crate::onboard_presentation::entry_choice_section_heading().to_owned());
    }
    lines.extend(render_onboard_wrapped_display_lines(
        append_escape_cancel_hint(Vec::<String>::new()),
        width,
    ));
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
    intro_lines.extend(crate::migration::render::render_candidate_preview_lines(
        &migration_candidate_for_onboard_display(candidate),
        width,
    ));
    intro_lines.extend(crate::migration::render::render_provider_selection_lines(
        &provider_selection,
        width,
    ));

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
    let mut lines =
        render_onboard_compact_header(width, flow_style.header_subtitle(), color_enabled);
    lines.push(String::new());
    lines.push("review setup".to_owned());
    lines.push(flow_style.progress_line());
    if let Some(source) = import_source {
        append_onboard_review_section(
            &mut lines,
            "starting point",
            mvp::presentation::render_wrapped_text_line(
                "- starting point: ",
                &onboard_starting_point_label(None, source),
                width,
            ),
        );
    }
    append_onboard_review_section(
        &mut lines,
        "configuration",
        render_onboard_review_digest_lines(config, width),
    );
    let review_candidate = build_onboard_review_candidate_with_selected_context(
        config,
        workspace_guidance,
        selected_candidate,
    );
    append_onboard_review_section(
        &mut lines,
        "draft source",
        crate::migration::render::render_candidate_preview_lines(&review_candidate, width),
    );
    lines
}

fn append_onboard_review_section(lines: &mut Vec<String>, title: &str, section_lines: Vec<String>) {
    if section_lines.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push(title.to_owned());
    lines.extend(section_lines);
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
        provider_endpoint: config.provider.region_endpoint_note(),
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
    let mut lines = render_onboard_compact_header(width, "setup complete", color_enabled);
    lines.push(String::new());
    lines.push("onboarding complete".to_owned());
    if !summary.next_actions.is_empty() {
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
        if !secondary_actions.is_empty() {
            lines.push("also available".to_owned());
            lines.extend(secondary_actions.into_iter().flat_map(|action| {
                mvp::presentation::render_wrapped_text_line(
                    &format!("- {}: ", action.label),
                    &action.command,
                    width,
                )
            }));
        }
    }

    lines.push("saved setup".to_owned());
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
    if let Some(provider_endpoint) = summary.provider_endpoint.as_deref() {
        lines.extend(mvp::presentation::render_wrapped_text_line(
            "- provider endpoint: ",
            provider_endpoint,
            width,
        ));
    }
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

fn render_default_choice_footer_line(key: &str, description: &str) -> String {
    format!("press Enter to use default {key}, {description}")
}

fn render_prompt_with_default_text(label: &str, default: &str) -> String {
    format!("{label} (default: {default}): ")
}

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
        render_provider_credential_source_value(Some(prompt_default)).unwrap_or_default();
    let suggested_env =
        render_provider_credential_source_value(Some(suggested_env)).unwrap_or_default();
    let current_env =
        configured_provider_credential_env_binding(&config.provider).and_then(|binding| {
            render_provider_credential_source_value(Some(binding.env_name.as_str()))
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

fn append_escape_cancel_hint(mut lines: Vec<String>) -> Vec<String> {
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
    color_enabled: bool,
) -> Vec<String> {
    let footer_lines = append_escape_cancel_hint(footer_lines);
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
    let hint_lines = append_escape_cancel_hint(hint_lines);
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
        OnboardHeaderStyle::Compact,
        width,
        shortcut_kind.subtitle(),
        shortcut_kind.title(),
        None,
        context_lines,
        build_onboard_shortcut_screen_options(shortcut_kind),
        vec![render_shortcut_default_choice_footer_line(shortcut_kind)],
        color_enabled,
    )
}

fn render_onboard_shortcut_header_lines_with_style(
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
        OnboardHeaderStyle::Compact,
        width,
        shortcut_kind.subtitle(),
        shortcut_kind.title(),
        None,
        context_lines,
        Vec::new(),
        Vec::new(),
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
        OnboardHeaderStyle::Brand,
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
        if let Some(hint) = preflight_attention_hint_line(checks) {
            summary_lines.push(hint.to_owned());
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
        let footer_lines = append_escape_cancel_hint(vec![render_default_choice_footer_line(
            "n",
            crate::onboard_presentation::preflight_default_choice_description(),
        )]);
        lines.extend(render_onboard_wrapped_display_lines(footer_lines, width));
    }
    lines
}

fn preflight_attention_hint_line(checks: &[OnboardCheck]) -> Option<&'static str> {
    if checks.iter().any(|check| {
        matches!(
            check.non_interactive_warning_policy,
            OnboardNonInteractiveWarningPolicy::RequiresExplicitModel
        )
    }) {
        return Some(crate::onboard_presentation::preflight_explicit_model_rerun_hint());
    }

    if checks.iter().any(|check| {
        matches!(
            check.non_interactive_warning_policy,
            OnboardNonInteractiveWarningPolicy::RequiresExplicitModelWithoutReviewedDefault
        )
    }) {
        return Some(crate::onboard_presentation::preflight_explicit_model_only_rerun_hint());
    }
    None
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
    let footer_lines = append_escape_cancel_hint(vec![render_default_choice_footer_line(
        "y",
        crate::onboard_presentation::write_confirmation_default_choice_description(),
    )]);
    lines.extend(render_onboard_wrapped_display_lines(footer_lines, width));
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
        OnboardHeaderStyle::Compact,
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
    let preferred_fallback_models = config.provider.configured_auto_model_candidates();
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
        .recommended_onboarding_model()
        .filter(|default_model| *default_model != config.provider.model)
    {
        context_lines.push(format!("- recommended model: {default_model}"));
    }
    if !preferred_fallback_models.is_empty() {
        context_lines.push(format!(
            "- configured preferred fallback: {}",
            preferred_fallback_models.join(", ")
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
    if !preferred_fallback_models.is_empty() && config.provider.explicit_model().is_none() {
        hint_lines.push(format!(
            "- type `auto` to let runtime try configured preferred fallbacks first: {}",
            preferred_fallback_models.join(", ")
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
        render_provider_credential_source_value(Some(default_api_key_env))
    {
        context_lines.push(format!("- suggested source: {suggested_source}"));
    }

    let example_env_name = provider_credential_env_hint(&config.provider)
        .unwrap_or_else(|| "PROVIDER_API_KEY".to_owned());
    let mut hint_lines = vec![render_api_key_env_selection_default_hint_line(
        config,
        default_api_key_env,
        prompt_default,
    )];
    hint_lines.push("- enter an env var name, not the secret value itself".to_owned());
    hint_lines.push(format!("- example: {example_env_name}"));
    if prompt_default.trim().is_empty() {
        if provider_has_inline_credential(&config.provider) {
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
    if let Some(provider_endpoint) = config.provider.region_endpoint_note() {
        lines.extend(mvp::presentation::render_wrapped_text_line(
            "- provider endpoint: ",
            &provider_endpoint,
            width,
        ));
    }

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
    if let Some(configured_env) = render_configured_provider_credential_source_value(provider) {
        return Some(OnboardingCredentialSummary {
            label: "credential source",
            value: configured_env,
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
    preferred_provider_credential_env_binding(provider)
        .and_then(|binding| {
            render_provider_credential_source_value(Some(binding.env_name.as_str()))
        })
        .map(|credential_env| OnboardingCredentialSummary {
            label: "credential source",
            value: credential_env,
        })
}

fn provider_supports_blank_api_key_env(config: &mvp::config::LoongClawConfig) -> bool {
    provider_has_inline_credential(&config.provider)
        || provider_has_configured_credential_env(&config.provider)
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
    use std::ffi::OsString;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, MutexGuard};

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

    fn write_browser_companion_script(script_path: &Path, body: &str) {
        let mut file = std::fs::File::create(script_path).expect("create browser companion script");
        file.write_all(body.as_bytes())
            .expect("write browser companion script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = file.metadata().expect("script metadata").permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(script_path, permissions)
                .expect("chmod browser companion script");
        }
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
        config.provider.api_key = Some("inline-openai-key".to_owned());
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
        let temp_dir = browser_companion_temp_dir("runtime-gate");
        let script_path = temp_dir.join("browser-companion");
        write_browser_companion_script(
            &script_path,
            "#!/bin/sh\necho 'loongclaw-browser-companion 1.5.0'\n",
        );

        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.api_key = Some("inline-openai-key".to_owned());
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.command = Some(script_path.display().to_string());
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
        let temp_dir = browser_companion_temp_dir("runtime-ready");
        let script_path = temp_dir.join("browser-companion");
        write_browser_companion_script(
            &script_path,
            "#!/bin/sh\necho 'loongclaw-browser-companion 1.5.0'\n",
        );

        let mut config = mvp::config::LoongClawConfig::default();
        config.provider.api_key = Some("inline-openai-key".to_owned());
        config.tools.browser_companion.enabled = true;
        config.tools.browser_companion.command = Some(script_path.display().to_string());
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
            "MiniMax-M1".to_owned(),
            "MiniMax-M1".to_owned(),
            "MiniMax-Text-01".to_owned(),
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
            check.detail.contains("MiniMax-M1"),
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
        config.provider.preferred_models = vec!["MiniMax-M1".to_owned()];
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

        let check = provider_credential_check(&config);

        assert_eq!(check.name, "provider credentials");
        assert_eq!(check.level, OnboardCheckLevel::Warn);
        assert!(check.detail.contains("ARK_API_KEY"));
        assert!(check.detail.contains("Authorization: Bearer <ARK_API_KEY>"));
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
            error.contains("provider.api_key_env"),
            "the validation error should identify the bad field: {error}"
        );
        assert!(
            !error.contains(secret),
            "non-interactive validation must not echo the secret-like input: {error}"
        );
    }

    #[test]
    fn apply_selected_api_key_env_routes_openai_oauth_env_to_oauth_binding() {
        let mut provider = mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::Openai,
            api_key_env: Some("OPENAI_API_KEY".to_owned()),
            ..mvp::config::ProviderConfig::default()
        };

        apply_selected_api_key_env(&mut provider, "OPENAI_CODEX_OAUTH_TOKEN".to_owned());

        assert_eq!(
            provider.oauth_access_token_env.as_deref(),
            Some("OPENAI_CODEX_OAUTH_TOKEN")
        );
        assert_eq!(
            provider.api_key_env, None,
            "switching to the OpenAI oauth env should clear the stale api-key env binding"
        );
    }

    #[test]
    fn apply_selected_api_key_env_routes_unknown_openai_env_to_api_key_binding() {
        let mut provider = mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::Openai,
            oauth_access_token_env: Some("OPENAI_CODEX_OAUTH_TOKEN".to_owned()),
            ..mvp::config::ProviderConfig::default()
        };

        apply_selected_api_key_env(&mut provider, "OPENAI_ALT_BEARER".to_owned());

        assert_eq!(
            provider.api_key_env.as_deref(),
            Some("OPENAI_ALT_BEARER"),
            "unknown env names should stay on the explicit api-key field instead of being silently rebound to oauth"
        );
        assert_eq!(
            provider.oauth_access_token_env, None,
            "switching to a custom env name should clear the stale oauth binding"
        );
    }

    #[test]
    fn provider_matches_for_review_ignores_credential_field_explicitness() {
        let current = mvp::config::ProviderConfig {
            kind: mvp::config::ProviderKind::Openai,
            model: "gpt-4.1".to_owned(),
            api_key: Some("inline-secret".to_owned()),
            ..mvp::config::ProviderConfig::default()
        };

        let mut api_key_env_update = current.clone();
        apply_selected_api_key_env(&mut api_key_env_update, "OPENAI_API_KEY".to_owned());
        assert!(api_key_env_update.api_key_env_explicit);
        assert!(
            provider_matches_for_review(&current, &api_key_env_update),
            "review matching should ignore explicit api_key_env metadata when the provider identity is otherwise unchanged"
        );

        let mut oauth_env_update = current.clone();
        apply_selected_api_key_env(&mut oauth_env_update, "OPENAI_CODEX_OAUTH_TOKEN".to_owned());
        assert!(oauth_env_update.oauth_access_token_env_explicit);
        assert!(
            provider_matches_for_review(&current, &oauth_env_update),
            "review matching should ignore explicit oauth_access_token_env metadata when the provider identity is otherwise unchanged"
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
            selected == "MiniMax-M2.5",
            "interactive onboarding should prefill the provider-recommended explicit model for MiniMax instead of leaving the operator on hidden runtime fallbacks: {selected:?}"
        );
    }

    #[test]
    fn resolve_model_selection_preserves_minimax_auto_non_interactively() {
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
            selected == "auto",
            "non-interactive onboarding should preserve auto for MiniMax instead of silently rewriting the operator config to the reviewed default: {selected:?}"
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
    fn resolve_model_selection_preserves_deepseek_auto_non_interactively() {
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
            selected == "auto",
            "non-interactive onboarding should preserve auto for DeepSeek instead of silently rewriting the operator config to the reviewed default: {selected:?}"
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
        config.provider.preferred_models = vec!["MiniMax-M1".to_owned()];

        let lines = render_model_selection_screen_lines_with_default(&config, "MiniMax-M2.5", 80);
        let rendered = lines.join("\n");

        assert!(
            rendered.contains("type `auto`")
                && rendered.contains("configured preferred fallbacks first")
                && rendered.contains("MiniMax-M1"),
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
