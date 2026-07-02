use super::*;
use dialoguer::console::{Term, user_attended};
use dialoguer::theme::ColorfulTheme;
use std::io::{self, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

pub(super) trait OnboardPromptLineReader {
    fn read_blocking_line(&mut self) -> CliResult<OnboardPromptRead>;
    fn read_pending_line(&mut self) -> CliResult<Option<String>>;
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum OnboardPromptRead {
    Line(String),
    Eof,
}

#[derive(Debug)]
pub(super) enum StdioOnboardLineMessage {
    Line(String),
    Eof,
    Error(String),
}

type StdioOnboardLineSender = mpsc::SyncSender<StdioOnboardLineMessage>;

#[derive(Debug)]
pub(super) enum StdioOnboardLineReader {
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

#[cfg(test)]
pub(super) fn onboard_line_channel_with_capacity(
    buffer_size: usize,
) -> (StdioOnboardLineSender, Receiver<StdioOnboardLineMessage>) {
    assert!(
        buffer_size > 0,
        "onboard line reader buffer must be non-zero"
    );
    mpsc::sync_channel(buffer_size)
}

#[cfg(not(test))]
fn onboard_line_channel_with_capacity(
    buffer_size: usize,
) -> (StdioOnboardLineSender, Receiver<StdioOnboardLineMessage>) {
    assert!(
        buffer_size > 0,
        "onboard line reader buffer must be non-zero"
    );
    mpsc::sync_channel(buffer_size)
}

#[cfg(test)]
pub(super) fn onboard_paste_drain_window() -> Duration {
    env::var(ONBOARD_PASTE_DRAIN_WINDOW_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_ONBOARD_PASTE_DRAIN_WINDOW)
}

#[cfg(not(test))]
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
    #[cfg(test)]
    pub(super) fn background_from_receiver(receiver: Receiver<StdioOnboardLineMessage>) -> Self {
        Self::Background {
            receiver,
            paste_drain_window: onboard_paste_drain_window(),
        }
    }

    #[cfg(not(test))]
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

    #[cfg(test)]
    pub(super) fn from_spawn_result(result: io::Result<Receiver<StdioOnboardLineMessage>>) -> Self {
        match result {
            Ok(receiver) => Self::background_from_receiver(receiver),
            Err(error) => Self::Direct {
                degraded_notice: Some(format_onboard_line_reader_spawn_notice(&error)),
            },
        }
    }

    #[cfg(not(test))]
    fn from_spawn_result(result: io::Result<Receiver<StdioOnboardLineMessage>>) -> Self {
        match result {
            Ok(receiver) => Self::background_from_receiver(receiver),
            Err(error) => Self::Direct {
                degraded_notice: Some(format_onboard_line_reader_spawn_notice(&error)),
            },
        }
    }

    pub(super) fn take_degraded_notice(&mut self) -> Option<String> {
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
    pub(super) line_reader: Option<StdioOnboardLineReader>,
}

impl StdioOnboardUi {
    fn stdio_line_reader(&mut self) -> &mut StdioOnboardLineReader {
        self.line_reader
            .get_or_insert_with(StdioOnboardLineReader::default)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct OnboardPromptCapture {
    pub(super) raw: String,
    pub(super) dropped_line_count: usize,
    pub(super) reached_eof: bool,
}

pub(super) fn read_single_line_prompt_capture(
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

pub(super) fn rich_prompt_ui_available() -> bool {
    user_attended()
}

pub(super) fn rich_prompt_theme() -> ColorfulTheme {
    ColorfulTheme::default()
}

pub(super) fn rich_prompt_term() -> Term {
    Term::stdout()
}

pub(super) fn print_lines(
    ui: &mut impl OnboardUi,
    lines: impl IntoIterator<Item = String>,
) -> CliResult<()> {
    for line in lines {
        ui.print_line(&line)?;
    }
    Ok(())
}

pub(super) fn print_message(ui: &mut impl OnboardUi, line: impl Into<String>) -> CliResult<()> {
    ui.print_line(&line.into())
}

pub(super) fn is_explicit_onboard_clear_input(raw: &str) -> bool {
    raw.trim().eq_ignore_ascii_case(ONBOARD_CLEAR_INPUT_TOKEN)
}

pub(super) fn is_explicit_onboard_cancel_input(raw: &str) -> bool {
    matches!(raw.trim(), "\u{1b}")
}

pub(super) fn ensure_onboard_input_not_cancelled(raw: String) -> CliResult<String> {
    if is_explicit_onboard_cancel_input(raw.as_str()) {
        return Err("onboarding cancelled: escape input received".to_owned());
    }
    Ok(raw)
}
