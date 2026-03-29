use ratatui::text::Line;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TranscriptRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TranscriptEntry {
    pub(crate) role: TranscriptRole,
    pub(crate) text: String,
    pub(crate) streaming: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TranscriptState {
    entries: Vec<TranscriptEntry>,
}

impl TranscriptState {
    #[cfg(test)]
    pub(crate) fn entries(&self) -> &[TranscriptEntry] {
        &self.entries
    }

    pub(crate) fn push_message(&mut self, role: TranscriptRole, text: impl Into<String>) {
        self.entries.push(TranscriptEntry {
            role,
            text: text.into(),
            streaming: false,
        });
    }

    pub(crate) fn update_assistant_stream(&mut self, text: impl Into<String>) {
        let text = text.into();

        match self.entries.last_mut() {
            Some(entry) if entry.role == TranscriptRole::Assistant && entry.streaming => {
                entry.text = text;
            }
            _ => self.entries.push(TranscriptEntry {
                role: TranscriptRole::Assistant,
                text,
                streaming: true,
            }),
        }
    }

    pub(crate) fn finalize_assistant_message(&mut self, text: impl Into<String>) {
        let text = text.into();

        match self.entries.last_mut() {
            Some(entry) if entry.role == TranscriptRole::Assistant && entry.streaming => {
                entry.text = text;
                entry.streaming = false;
            }
            _ => self.entries.push(TranscriptEntry {
                role: TranscriptRole::Assistant,
                text,
                streaming: false,
            }),
        }
    }
}

pub(crate) fn render_transcript_lines(state: &TranscriptState) -> Vec<Line<'static>> {
    if state.entries.is_empty() {
        return vec![Line::from("assistant> TUI shell bootstrap ready.")];
    }

    state
        .entries
        .iter()
        .flat_map(|entry| {
            let role = match entry.role {
                TranscriptRole::User => "you",
                TranscriptRole::Assistant => "assistant",
            };
            let streaming_suffix = if entry.streaming { " (streaming)" } else { "" };
            let prefix = format!("{role}{streaming_suffix}> ");
            let continuation_indent = " ".repeat(prefix.chars().count());

            entry
                .text
                .split('\n')
                .enumerate()
                .map(|(index, line)| {
                    if index == 0 {
                        Line::from(format!("{prefix}{line}"))
                    } else {
                        Line::from(format!("{continuation_indent}{line}"))
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiline_entries_keep_prefix_on_first_line_and_indent_followups() {
        let mut state = TranscriptState::default();
        state.push_message(
            TranscriptRole::Assistant,
            "first line\nsecond line\nthird line",
        );

        let rendered = render_transcript_lines(&state);
        let lines = rendered
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<String>>();

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "assistant> first line");
        assert_eq!(lines[1], "           second line");
        assert_eq!(lines[2], "           third line");
    }
}
