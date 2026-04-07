use std::io::{self, Write};
use std::path::PathBuf;
use std::process::Command;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;

use crate::CliResult;

use super::events::UiEvent;
use super::focus::FocusLayer;
use super::state;
use super::stats;

pub(super) fn build_osc52_copy_sequence(text: &str) -> String {
    let encoded_text = BASE64_STANDARD.encode(text.as_bytes());

    format!("\u{1b}]52;c;{encoded_text}\u{7}")
}

pub(super) fn copy_text_to_terminal_clipboard(text: &str) -> CliResult<()> {
    let copy_sequence = build_osc52_copy_sequence(text);
    let mut stdout = io::stdout();

    stdout
        .write_all(copy_sequence.as_bytes())
        .map_err(|error| format!("failed to write clipboard escape sequence: {error}"))?;
    stdout
        .flush()
        .map_err(|error| format!("failed to flush clipboard escape sequence: {error}"))?;

    Ok(())
}

pub(super) fn open_stats_overlay(shell: &mut state::Shell, args: &str) {
    let options = match stats::parse_stats_open_options(args) {
        Ok(options) => options,
        Err(error) => {
            shell.pane.add_system_message(&error);
            return;
        }
    };
    let Some(config) = shell.runtime_config.as_ref() else {
        shell.pane.add_system_message(
            "Stats view is unavailable before the chat runtime is initialized.",
        );
        return;
    };
    let current_session_id = shell.pane.session_id.clone();
    let snapshot = match stats::load_stats_snapshot(config, current_session_id.as_str()) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            shell
                .pane
                .add_system_message(&format!("Unable to load stats: {error}"));
            return;
        }
    };

    shell.stats_overlay = Some(state::StatsOverlayState::new(
        snapshot,
        options.tab,
        options.date_range,
    ));
    if !shell.focus.has(FocusLayer::StatsOverlay) {
        shell.focus.push(FocusLayer::StatsOverlay);
    }
    shell.pane.set_status("Stats overlay opened".to_owned());
}

pub(super) fn close_stats_overlay(shell: &mut state::Shell) {
    shell.stats_overlay = None;
    if shell.focus.top() == FocusLayer::StatsOverlay {
        shell.focus.pop();
    }
}

fn stats_overlay_entry_count(stats_overlay: &state::StatsOverlayState) -> usize {
    let range_view = stats_overlay.snapshot.range_view(stats_overlay.date_range);
    match stats_overlay.active_tab {
        stats::StatsTab::Overview => 0,
        stats::StatsTab::Models => range_view.model_totals.len(),
        stats::StatsTab::Sessions => range_view.session_rows.len(),
    }
}

fn clamp_stats_overlay_list_offset(stats_overlay: &mut state::StatsOverlayState) {
    let entry_count = stats_overlay_entry_count(stats_overlay);
    let max_offset = entry_count.saturating_sub(1);
    if stats_overlay.list_scroll_offset > max_offset {
        stats_overlay.list_scroll_offset = max_offset;
    }
}

pub(super) fn scroll_stats_overlay_up(shell: &mut state::Shell, amount: usize) {
    let Some(stats_overlay) = shell.stats_overlay.as_mut() else {
        return;
    };
    let current_offset = stats_overlay.list_scroll_offset;
    let next_offset = current_offset.saturating_sub(amount);
    stats_overlay.list_scroll_offset = next_offset;
}

pub(super) fn scroll_stats_overlay_down(shell: &mut state::Shell, amount: usize) {
    let Some(stats_overlay) = shell.stats_overlay.as_mut() else {
        return;
    };
    let current_offset = stats_overlay.list_scroll_offset;
    let next_offset = current_offset.saturating_add(amount);
    stats_overlay.list_scroll_offset = next_offset;
    clamp_stats_overlay_list_offset(stats_overlay);
}

pub(super) fn jump_stats_overlay_top(shell: &mut state::Shell) {
    let Some(stats_overlay) = shell.stats_overlay.as_mut() else {
        return;
    };
    stats_overlay.list_scroll_offset = 0;
}

pub(super) fn jump_stats_overlay_bottom(shell: &mut state::Shell) {
    let Some(stats_overlay) = shell.stats_overlay.as_mut() else {
        return;
    };
    let entry_count = stats_overlay_entry_count(stats_overlay);
    let last_index = entry_count.saturating_sub(1);
    stats_overlay.list_scroll_offset = last_index;
    clamp_stats_overlay_list_offset(stats_overlay);
}

pub(super) fn close_diff_overlay(shell: &mut state::Shell) {
    shell.diff_overlay = None;
    if shell.focus.top() == FocusLayer::DiffOverlay {
        shell.focus.pop();
    }
}

pub(super) fn scroll_diff_overlay_up(shell: &mut state::Shell, amount: u16) {
    let diff_overlay = match shell.diff_overlay.as_mut() {
        Some(diff_overlay) => diff_overlay,
        None => return,
    };
    let next_offset = diff_overlay.scroll_offset.saturating_sub(amount);

    diff_overlay.scroll_offset = next_offset;
}

pub(super) fn scroll_diff_overlay_down(shell: &mut state::Shell, amount: u16) {
    let diff_overlay = match shell.diff_overlay.as_mut() {
        Some(diff_overlay) => diff_overlay,
        None => return,
    };
    let next_offset = diff_overlay.scroll_offset.saturating_add(amount);

    diff_overlay.scroll_offset = next_offset;
}

pub(super) fn select_diff_overlay_file(shell: &mut state::Shell, index: usize) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let diff_overlay = match shell.diff_overlay.as_mut() {
        Some(diff_overlay) => diff_overlay,
        None => return,
    };
    if diff_overlay.files.is_empty() {
        return;
    }

    let max_index = diff_overlay.files.len().saturating_sub(1);
    let selected_index = index.min(max_index);
    let selected_file = diff_overlay.files.get(selected_index);
    let detail_output = match load_diff_overlay_detail_output(
        cwd.as_path(),
        diff_overlay.mode.as_str(),
        selected_file,
    ) {
        Ok(output) => output,
        Err(error) => {
            shell
                .pane
                .set_status(format!("Diff detail failed: {error}"));
            return;
        }
    };

    diff_overlay.selected_file_index = selected_index;
    diff_overlay.detail_output = detail_output;
    diff_overlay.scroll_offset = 0;
}

pub(super) fn select_previous_diff_overlay_file(shell: &mut state::Shell) {
    let selected_index = match shell.diff_overlay.as_ref() {
        Some(diff_overlay) => diff_overlay.selected_file_index,
        None => return,
    };
    let previous_index = selected_index.saturating_sub(1);

    select_diff_overlay_file(shell, previous_index);
}

pub(super) fn select_next_diff_overlay_file(shell: &mut state::Shell) {
    let (selected_index, file_count) = match shell.diff_overlay.as_ref() {
        Some(diff_overlay) => (diff_overlay.selected_file_index, diff_overlay.files.len()),
        None => return,
    };
    if file_count == 0 {
        return;
    }

    let max_index = file_count.saturating_sub(1);
    let next_index = selected_index.saturating_add(1).min(max_index);

    select_diff_overlay_file(shell, next_index);
}

fn diff_overlay_scroll_step() -> u16 {
    10
}

fn parse_diff_overlay_status_column(column: Option<char>) -> Option<char> {
    match column {
        Some(' ') | None => None,
        Some(value) => Some(value),
    }
}

fn normalize_status_overlay_path(path: &str) -> String {
    path.rsplit_once(" -> ")
        .map(|(_, next)| next)
        .unwrap_or(path)
        .trim()
        .to_owned()
}

pub(super) fn parse_worktree_status_paths(status_output: &str) -> Vec<String> {
    let mut paths = Vec::new();

    for status_line in status_output.lines() {
        let trimmed_line = status_line.trim_end();
        if trimmed_line.is_empty() {
            continue;
        }

        let raw_path = trimmed_line.get(3..).unwrap_or("").trim();
        let normalized_path = normalize_status_overlay_path(raw_path);
        if normalized_path.is_empty() {
            continue;
        }

        paths.push(normalized_path);
    }

    paths.sort();
    paths.dedup();
    paths
}

fn normalize_numstat_overlay_path(path: &str) -> String {
    let trimmed = path.trim();
    if let Some((prefix, suffix)) = trimmed.split_once('{')
        && let Some((variant, remainder)) = suffix.split_once('}')
        && variant.contains(" => ")
    {
        let replacement = variant
            .rsplit_once(" => ")
            .map(|(_, next)| next)
            .unwrap_or(variant);
        return format!("{prefix}{replacement}{remainder}");
    }

    trimmed
        .rsplit_once(" => ")
        .map(|(_, next)| next.trim())
        .unwrap_or(trimmed)
        .to_owned()
}

fn accumulate_diff_overlay_count(current: Option<usize>, next: Option<usize>) -> Option<usize> {
    match (current, next) {
        (Some(current), Some(next)) => Some(current.saturating_add(next)),
        (Some(current), None) => Some(current),
        (None, Some(next)) => Some(next),
        (None, None) => None,
    }
}

fn diff_overlay_has_staged_changes(file: &state::DiffOverlayFileEntry) -> bool {
    file.index_status.is_some() && !file.untracked
}

fn diff_overlay_has_unstaged_changes(file: &state::DiffOverlayFileEntry) -> bool {
    file.worktree_status.is_some() && !file.untracked
}

pub(super) fn git_output(args: &[&str], cwd: &std::path::Path) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|error| format!("failed to run `git {}`: {error}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if stderr.is_empty() {
            return Err(format!(
                "`git {}` exited with status {}",
                args.join(" "),
                output.status
            ));
        }
        return Err(stderr);
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn normalize_diff_mode(args: &str) -> Result<&str, String> {
    let trimmed = args.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("status") {
        return Ok("status");
    }
    if trimmed.eq_ignore_ascii_case("full") {
        return Ok("full");
    }

    Err(format!(
        "unsupported diff mode `{trimmed}`; use `status` or `full`"
    ))
}

pub(super) fn parse_diff_overlay_files(
    status_output: &str,
    unstaged_numstat_output: &str,
    staged_numstat_output: &str,
) -> Vec<state::DiffOverlayFileEntry> {
    let mut files = Vec::new();

    for status_line in status_output.lines() {
        let trimmed_line = status_line.trim_end();
        if trimmed_line.is_empty() {
            continue;
        }

        let raw_status = trimmed_line.get(..2).unwrap_or("  ");
        let raw_path = trimmed_line.get(3..).unwrap_or("").trim();
        let path = normalize_status_overlay_path(raw_path);
        if path.is_empty() {
            continue;
        }

        let mut status_chars = raw_status.chars();
        let index_status = parse_diff_overlay_status_column(status_chars.next());
        let worktree_status = parse_diff_overlay_status_column(status_chars.next());
        let untracked = raw_status == "??";
        let file_entry = state::DiffOverlayFileEntry {
            path,
            index_status,
            worktree_status,
            added_lines: None,
            removed_lines: None,
            untracked,
        };
        files.push(file_entry);
    }

    merge_diff_overlay_numstat(&mut files, unstaged_numstat_output, false);
    merge_diff_overlay_numstat(&mut files, staged_numstat_output, true);

    files.sort_by(|left, right| left.path.cmp(&right.path));
    files
}

fn merge_diff_overlay_numstat(
    files: &mut Vec<state::DiffOverlayFileEntry>,
    numstat_output: &str,
    staged: bool,
) {
    for numstat_line in numstat_output.lines() {
        let mut parts = numstat_line.split('\t');
        let added_part = parts.next().unwrap_or("");
        let removed_part = parts.next().unwrap_or("");
        let path_part = normalize_numstat_overlay_path(parts.next().unwrap_or(""));
        if path_part.is_empty() {
            continue;
        }

        let added_lines = added_part.parse::<usize>().ok();
        let removed_lines = removed_part.parse::<usize>().ok();
        let existing_file = files.iter_mut().find(|file| file.path == path_part);
        if let Some(existing_file) = existing_file {
            existing_file.added_lines =
                accumulate_diff_overlay_count(existing_file.added_lines, added_lines);
            existing_file.removed_lines =
                accumulate_diff_overlay_count(existing_file.removed_lines, removed_lines);
            if staged {
                existing_file.index_status.get_or_insert('M');
            } else {
                existing_file.worktree_status.get_or_insert('M');
            }
            continue;
        }

        let file_entry = state::DiffOverlayFileEntry {
            path: path_part,
            index_status: staged.then_some('M'),
            worktree_status: (!staged).then_some('M'),
            added_lines,
            removed_lines,
            untracked: false,
        };
        files.push(file_entry);
    }
}

fn load_single_diff_overlay_detail_output(
    cwd: &std::path::Path,
    mode: &str,
    path: &str,
    staged: bool,
) -> Result<String, String> {
    match (mode, staged) {
        ("full", true) => git_output(
            &[
                "diff",
                "--cached",
                "--no-ext-diff",
                "--patch",
                "--no-color",
                "--",
                path,
            ],
            cwd,
        ),
        ("full", false) => git_output(
            &["diff", "--no-ext-diff", "--patch", "--no-color", "--", path],
            cwd,
        ),
        (_, true) => git_output(
            &[
                "diff",
                "--cached",
                "--no-ext-diff",
                "--stat",
                "--no-color",
                "--",
                path,
            ],
            cwd,
        ),
        (_, false) => git_output(
            &["diff", "--no-ext-diff", "--stat", "--no-color", "--", path],
            cwd,
        ),
    }
}

fn push_diff_overlay_detail_section(sections: &mut Vec<String>, title: &str, output: String) {
    let trimmed_output = output.trim();
    if trimmed_output.is_empty() {
        return;
    }

    sections.push(format!("{title}\n\n{trimmed_output}"));
}

fn load_diff_overlay_detail_output(
    cwd: &std::path::Path,
    mode: &str,
    file: Option<&state::DiffOverlayFileEntry>,
) -> Result<String, String> {
    let Some(file) = file else {
        return Ok(String::new());
    };

    if file.untracked {
        return Ok(format!(
            "Untracked file.\n\nStage `{}` to inspect a tracked patch, or review the file directly from the workspace.",
            file.path
        ));
    }

    let mut sections = Vec::new();
    if diff_overlay_has_staged_changes(file) {
        let staged_output =
            load_single_diff_overlay_detail_output(cwd, mode, file.path.as_str(), true)?;
        push_diff_overlay_detail_section(&mut sections, "Staged changes", staged_output);
    }
    if diff_overlay_has_unstaged_changes(file) {
        let worktree_output =
            load_single_diff_overlay_detail_output(cwd, mode, file.path.as_str(), false)?;
        push_diff_overlay_detail_section(&mut sections, "Unstaged changes", worktree_output);
    }

    Ok(sections.join("\n\n"))
}

pub(super) fn show_diff_surface(shell: &mut state::Shell, args: &str) {
    let mode = match normalize_diff_mode(args) {
        Ok(mode) => mode,
        Err(error) => {
            shell.pane.add_system_message(&error);
            return;
        }
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let status_output = match git_output(&["status", "--short"], cwd.as_path()) {
        Ok(output) => output,
        Err(error) => {
            shell
                .pane
                .add_system_message(&format!("Unable to inspect working tree status: {error}"));
            return;
        }
    };
    let unstaged_numstat_output = match git_output(
        &["diff", "--no-ext-diff", "--numstat", "--no-color"],
        cwd.as_path(),
    ) {
        Ok(output) => output,
        Err(error) => {
            shell
                .pane
                .add_system_message(&format!("Unable to inspect working tree diff: {error}"));
            return;
        }
    };
    let staged_numstat_output = match git_output(
        &[
            "diff",
            "--cached",
            "--no-ext-diff",
            "--numstat",
            "--no-color",
        ],
        cwd.as_path(),
    ) {
        Ok(output) => output,
        Err(error) => {
            shell
                .pane
                .add_system_message(&format!("Unable to inspect working tree diff: {error}"));
            return;
        }
    };
    let cwd_display = cwd
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| cwd.display().to_string());
    let files = parse_diff_overlay_files(
        status_output.as_str(),
        unstaged_numstat_output.as_str(),
        staged_numstat_output.as_str(),
    );
    let detail_output = match load_diff_overlay_detail_output(cwd.as_path(), mode, files.first()) {
        Ok(output) => output,
        Err(error) => {
            shell
                .pane
                .add_system_message(&format!("Unable to inspect working tree diff: {error}"));
            return;
        }
    };
    let diff_overlay = state::DiffOverlayState {
        mode: mode.to_owned(),
        cwd_display,
        files,
        selected_file_index: 0,
        detail_output,
        scroll_offset: 0,
    };

    shell.diff_overlay = Some(diff_overlay);
    if !shell.focus.has(FocusLayer::DiffOverlay) {
        shell.focus.push(FocusLayer::DiffOverlay);
    }
    shell.pane.set_status("Diff overlay opened".to_owned());
}

fn tool_inspector_scroll_step() -> u16 {
    let terminal_size = crossterm::terminal::size();
    let (_, height) = terminal_size.unwrap_or((80, 24));
    let available_height = height.saturating_sub(8);
    let scroll_step = available_height / 2;

    scroll_step.max(1)
}

pub(super) fn open_tool_inspector(shell: &mut state::Shell) {
    let opened = shell.pane.open_latest_tool_inspector();
    if opened {
        if !shell.focus.has(FocusLayer::ToolInspector) {
            shell.focus.push(FocusLayer::ToolInspector);
        }
    } else {
        shell.pane.set_status("No tool details available".into());
    }
}

pub(super) fn close_tool_inspector(shell: &mut state::Shell) {
    shell.pane.close_tool_inspector();
    if shell.focus.top() == FocusLayer::ToolInspector {
        shell.focus.pop();
    }
}

pub(super) fn handle_overlay_key_event(
    shell: &mut state::Shell,
    key: KeyEvent,
    tx: &mpsc::UnboundedSender<UiEvent>,
) -> bool {
    match shell.focus.top() {
        FocusLayer::ClarifyDialog => {
            if let Some(ref mut dialog) = shell.pane.clarify_dialog {
                #[allow(clippy::wildcard_enum_match_arm)]
                match key.code {
                    KeyCode::Enter => {
                        let response = dialog.response();
                        shell.pane.clarify_dialog = None;
                        shell.focus.pop();
                        let _ = tx.send(UiEvent::Token {
                            content: format!("\n[user chose: {response}]\n"),
                            is_thinking: false,
                        });
                    }
                    KeyCode::Esc => {
                        shell.pane.clarify_dialog = None;
                        shell.focus.pop();
                    }
                    KeyCode::Up => dialog.select_up(),
                    KeyCode::Down => dialog.select_down(),
                    KeyCode::Left => dialog.move_cursor_left(),
                    KeyCode::Right => dialog.move_cursor_right(),
                    KeyCode::Backspace => dialog.delete_back(),
                    KeyCode::Char(ch) => dialog.insert_char(ch),
                    _ => {}
                }
            }
            true
        }
        FocusLayer::StatsOverlay => {
            if key.code == KeyCode::Esc || key.code == KeyCode::Char('q') {
                close_stats_overlay(shell);
                return true;
            }

            if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
                if let Some(stats_overlay) = shell.stats_overlay.as_mut() {
                    let copied_text = stats::render_copy_text(
                        &stats_overlay.snapshot,
                        stats_overlay.active_tab,
                        stats_overlay.date_range,
                    );
                    let copy_result = copy_text_to_terminal_clipboard(copied_text.as_str());
                    match copy_result {
                        Ok(()) => {
                            stats_overlay.copy_status = Some("copied".to_owned());
                            shell.pane.set_status("Stats copied".to_owned());
                        }
                        Err(error) => {
                            stats_overlay.copy_status = Some("copy failed".to_owned());
                            shell.pane.set_status(format!("Copy failed: {error}"));
                        }
                    }
                }
                return true;
            }

            if key.code == KeyCode::Tab || key.code == KeyCode::Right {
                if let Some(stats_overlay) = shell.stats_overlay.as_mut() {
                    stats_overlay.active_tab = stats_overlay.active_tab.next();
                    stats_overlay.list_scroll_offset = 0;
                }
                return true;
            }

            if key.code == KeyCode::BackTab || key.code == KeyCode::Left {
                if let Some(stats_overlay) = shell.stats_overlay.as_mut() {
                    stats_overlay.active_tab = stats_overlay.active_tab.previous();
                    stats_overlay.list_scroll_offset = 0;
                }
                return true;
            }

            if key.code == KeyCode::Char('r') && key.modifiers.is_empty() {
                if let Some(stats_overlay) = shell.stats_overlay.as_mut() {
                    stats_overlay.date_range = stats_overlay.date_range.next();
                    stats_overlay.list_scroll_offset = 0;
                }
                return true;
            }

            if key.code == KeyCode::Up {
                scroll_stats_overlay_up(shell, 1);
                return true;
            }

            if key.code == KeyCode::Down {
                scroll_stats_overlay_down(shell, 1);
                return true;
            }

            if key.code == KeyCode::PageUp {
                scroll_stats_overlay_up(shell, 5);
                return true;
            }

            if key.code == KeyCode::PageDown {
                scroll_stats_overlay_down(shell, 5);
                return true;
            }

            if key.code == KeyCode::Home {
                jump_stats_overlay_top(shell);
                return true;
            }

            if key.code == KeyCode::End {
                jump_stats_overlay_bottom(shell);
                return true;
            }

            true
        }
        FocusLayer::DiffOverlay => {
            let scroll_step = diff_overlay_scroll_step();

            if key.code == KeyCode::Esc || key.code == KeyCode::Char('q') {
                close_diff_overlay(shell);
                return true;
            }

            if key.code == KeyCode::Up {
                select_previous_diff_overlay_file(shell);
                return true;
            }

            if key.code == KeyCode::Down {
                select_next_diff_overlay_file(shell);
                return true;
            }

            if key.code == KeyCode::PageUp {
                scroll_diff_overlay_up(shell, scroll_step);
                return true;
            }

            if key.code == KeyCode::PageDown {
                scroll_diff_overlay_down(shell, scroll_step);
                return true;
            }

            if key.code == KeyCode::Home {
                let selected_index = if let Some(diff_overlay) = shell.diff_overlay.as_mut() {
                    if !diff_overlay.files.is_empty() {
                        diff_overlay.selected_file_index = 0;
                    }
                    diff_overlay.scroll_offset = 0;
                    diff_overlay.selected_file_index
                } else {
                    return true;
                };

                select_diff_overlay_file(shell, selected_index);
                return true;
            }

            if key.code == KeyCode::End {
                let last_index = if let Some(diff_overlay) = shell.diff_overlay.as_mut() {
                    let last_index = diff_overlay.files.len().saturating_sub(1);
                    diff_overlay.selected_file_index = last_index;
                    diff_overlay.scroll_offset = 0;
                    last_index
                } else {
                    return true;
                };

                select_diff_overlay_file(shell, last_index);
                return true;
            }

            true
        }
        FocusLayer::ToolInspector => {
            let scroll_step = tool_inspector_scroll_step();

            #[allow(clippy::wildcard_enum_match_arm)]
            match key.code {
                KeyCode::Esc => {
                    close_tool_inspector(shell);
                }
                KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    let _ = shell.pane.open_latest_tool_inspector();
                }
                KeyCode::Up => {
                    let _ = shell.pane.select_previous_tool_inspector_entry();
                }
                KeyCode::Down => {
                    let _ = shell.pane.select_next_tool_inspector_entry();
                }
                KeyCode::PageUp => shell.pane.scroll_tool_inspector_up(scroll_step),
                KeyCode::PageDown => shell.pane.scroll_tool_inspector_down(scroll_step),
                KeyCode::Home => {
                    let _ = shell.pane.select_first_tool_inspector_entry();
                }
                KeyCode::End => {
                    let _ = shell.pane.select_last_tool_inspector_entry();
                }
                _ => {}
            }
            true
        }
        FocusLayer::Help => {
            if key.code == KeyCode::Esc || key.code == KeyCode::Char('q') {
                shell.focus.pop();
            }
            true
        }
        FocusLayer::ThemePicker => false,
        FocusLayer::Composer | FocusLayer::Transcript | FocusLayer::SessionPicker => false,
    }
}
