#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupEyeFocus {
    Center,
    Left,
    Right,
    Up,
    DownCenter,
    DownLeft,
    DownRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupEyeAnimation {
    Ambient,
    Focus(StartupEyeFocus),
    Thinking(StartupEyeFocus),
    Confirm(StartupEyeFocus),
    Celebrate,
}

const STARTUP_EYE_CENTER: StartupEyeFrame = ["    ", " ▂  ", " █  ", " ▄  ", "    "];
const STARTUP_EYE_LEFT: StartupEyeFrame = ["    ", "▂   ", "█   ", "▄   ", "    "];
const STARTUP_EYE_RIGHT: StartupEyeFrame = ["    ", "  ▂ ", "  █ ", "  ▄ ", "    "];
const STARTUP_EYE_UP: StartupEyeFrame = [" ▂  ", " █  ", " ▄  ", "    ", "    "];
const STARTUP_EYE_DOWN_CENTER: StartupEyeFrame = ["    ", "    ", " ▂  ", " █  ", " ▄  "];
const STARTUP_EYE_DOWN_LEFT: StartupEyeFrame = ["    ", "    ", "▂   ", "█   ", "▄   "];
const STARTUP_EYE_DOWN_RIGHT: StartupEyeFrame = ["    ", "    ", "  ▂ ", "  █ ", "  ▄ "];
const STARTUP_EYE_HALF_LID_CENTER: StartupEyeFrame = ["▒▒▒▒", "    ", " █  ", "    ", "    "];
const STARTUP_EYE_HALF_LID_LEFT: StartupEyeFrame = ["▒▒▒▒", "    ", "█   ", "    ", "    "];
const STARTUP_EYE_HALF_LID_RIGHT: StartupEyeFrame = ["▒▒▒▒", "    ", "  █ ", "    ", "    "];
const STARTUP_EYE_HALF_LID_DOWN_CENTER: StartupEyeFrame = ["    ", "▒▒▒▒", " ▂  ", " █  ", "    "];
const STARTUP_EYE_HALF_LID_DOWN_LEFT: StartupEyeFrame = ["    ", "▒▒▒▒", "▂   ", "█   ", "    "];
const STARTUP_EYE_HALF_LID_DOWN_RIGHT: StartupEyeFrame = ["    ", "▒▒▒▒", "  ▂ ", "  █ ", "    "];
const STARTUP_EYE_CONFIRM_CENTER: StartupEyeFrame = ["    ", " ▆  ", " █  ", " ▆  ", "    "];
const STARTUP_EYE_CONFIRM_DOWN_CENTER: StartupEyeFrame = ["    ", "    ", " ▄  ", " █  ", " ▇  "];
const STARTUP_EYE_CONFIRM_LEFT: StartupEyeFrame = ["    ", "▆   ", "█   ", "▆   ", "    "];
const STARTUP_EYE_CONFIRM_RIGHT: StartupEyeFrame = ["    ", "  ▆ ", "  █ ", "  ▆ ", "    "];
const STARTUP_EYE_CONFIRM_DOWN_LEFT: StartupEyeFrame = ["    ", "    ", "▄   ", "█   ", "▇   "];
const STARTUP_EYE_CONFIRM_DOWN_RIGHT: StartupEyeFrame = ["    ", "    ", "  ▄ ", "  █ ", "  ▇ "];
const STARTUP_EYE_CELEBRATE_A: StartupEyeFrame = [" ▂▂ ", " ▇▇ ", " ▄▄ ", "    ", "    "];
const STARTUP_EYE_CELEBRATE_B: StartupEyeFrame = [" ▄▄ ", " ▇▇ ", " ▂▂ ", "    ", "    "];

pub enum MessageContent {
    RenderedLines(Vec<String>),
    Markdown(String),
    Diff {
        title: Option<String>,
        content: String,
    },
    Image {
        alt: String,
        url: String,
    },
    ToolCall {
        title: String,
        lines: Vec<String>,
        status: ToolStatus,
    },
    Error {
        title: String,
        summary: String,
        details: Vec<String>,
    },
    Compaction {
        turn_count: usize,
        summary: String,
        expanded: bool,
    },
    StartupHeader {
        version: String,
        tutorial: String,
        sections: Vec<(String, Vec<String>)>,
        tips: Vec<String>,
        eye_animation: StartupEyeAnimation,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Pending,
    Success,
    Error,
}

pub struct Message {
    pub role: String,
    pub contents: Vec<MessageContent>,
}

pub struct MessageList {
    pub messages: Vec<Message>,
    page_step: u16,
    mouse_step: u16,
    last_render_height: u16,
    scroll_state: TranscriptScrollState,
    render_revision: u64,
    render_cache: Option<RenderCache>,
    viewport_cache: Option<ViewportRenderCache>,
    startup_animation_started_at: Instant,
    last_startup_animation_signature: Option<u64>,
}

struct RenderCache {
    width: u16,
    revision: u64,
    lines: Vec<Line<'static>>,
}

#[derive(Clone)]
struct ViewportRenderCache {
    width: u16,
    revision: u64,
    height: u16,
    scroll_start: usize,
    top_padding: usize,
    lines: Vec<Line<'static>>,
}

impl MessageList {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            page_step: 12,
            mouse_step: 3,
            last_render_height: 0,
            scroll_state: TranscriptScrollState::new(),
            render_revision: 0,
            render_cache: None,
            viewport_cache: None,
            startup_animation_started_at: Instant::now(),
            last_startup_animation_signature: None,
        }
    }

    pub fn add_user_message(&mut self, msg: String) {
        self.messages.push(Message {
            role: "You".to_string(),
            contents: vec![MessageContent::Markdown(msg)],
        });
        self.scroll_state.prepare_for_appended_content();
        self.invalidate_render_cache();
    }

    pub fn add_assistant_message(&mut self, msg: String) {
        let contents = build_assistant_contents(&msg);
        self.messages.push(Message {
            role: "Assistant".to_string(),
            contents,
        });
        self.scroll_state.prepare_for_appended_content();
        self.invalidate_render_cache();
    }

    pub fn add_rendered_lines(&mut self, lines: Vec<String>) {
        self.messages.push(Message {
            role: "System".to_string(),
            contents: vec![MessageContent::RenderedLines(lines)],
        });
        self.scroll_state.prepare_for_appended_content();
        self.invalidate_render_cache();
    }

    pub fn clear_transcript(&mut self) {
        self.messages.clear();
        self.scroll_state.reset();
        self.last_startup_animation_signature = None;
        self.invalidate_render_cache();
    }

    pub fn latest_copy_text(&self) -> Option<String> {
        self.messages
            .iter()
            .rev()
            .filter(|message| message.role != "System")
            .filter_map(message_plain_text)
            .find(|text| !text.trim().is_empty())
            .or_else(|| {
                self.messages
                    .iter()
                    .rev()
                    .filter_map(message_plain_text)
                    .find(|text| !text.trim().is_empty())
            })
    }

    pub fn export_markdown(&self) -> String {
        let mut sections = Vec::new();
        for message in &self.messages {
            let Some(body) = message_plain_text(message) else {
                continue;
            };
            if body.trim().is_empty() {
                continue;
            }
            sections.push(format!("## {}\n\n{}", message.role, body.trim_end()));
        }
        sections.join("\n\n")
    }

    #[cfg(test)]
    pub fn add_startup_header(
        &mut self,
        version: String,
        tutorial: String,
        sections: Vec<(String, Vec<String>)>,
    ) {
        self.add_startup_header_with_tips(version, tutorial, sections, Vec::new());
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn add_startup_header_with_tips(
        &mut self,
        version: String,
        tutorial: String,
        sections: Vec<(String, Vec<String>)>,
        tips: Vec<String>,
    ) {
        self.add_startup_header_with_tips_and_eye(
            version,
            tutorial,
            sections,
            tips,
            StartupEyeAnimation::Ambient,
        );
    }

    pub fn add_startup_header_with_tips_and_eye(
        &mut self,
        version: String,
        tutorial: String,
        sections: Vec<(String, Vec<String>)>,
        tips: Vec<String>,
        eye_animation: StartupEyeAnimation,
    ) {
        self.startup_animation_started_at = Instant::now();
        self.last_startup_animation_signature = None;
        self.messages.push(Message {
            role: "System".to_string(),
            contents: vec![MessageContent::StartupHeader {
                version,
                tutorial,
                sections,
                tips,
                eye_animation,
            }],
        });
        self.scroll_state.prepare_for_appended_content();
        self.invalidate_render_cache();
    }

    #[allow(dead_code)]
    pub fn replace_latest_startup_header_with_tips(
        &mut self,
        version: String,
        tutorial: String,
        sections: Vec<(String, Vec<String>)>,
        tips: Vec<String>,
    ) {
        self.replace_latest_startup_header_with_eye(
            version,
            tutorial,
            sections,
            tips,
            StartupEyeAnimation::Ambient,
        );
    }

    pub fn replace_latest_startup_header_with_eye(
        &mut self,
        version: String,
        tutorial: String,
        sections: Vec<(String, Vec<String>)>,
        tips: Vec<String>,
        eye_animation: StartupEyeAnimation,
    ) {
        for message in self.messages.iter_mut().rev() {
            for content in message.contents.iter_mut().rev() {
                if let MessageContent::StartupHeader {
                    version: current_version,
                    tutorial: current_tutorial,
                    sections: current_sections,
                    tips: current_tips,
                    eye_animation: current_eye_animation,
                } = content
                {
                    *current_version = version;
                    *current_tutorial = tutorial;
                    *current_sections = sections;
                    *current_tips = tips;
                    *current_eye_animation = eye_animation;
                    self.invalidate_render_cache();
                    return;
                }
            }
        }

        self.add_startup_header_with_tips_and_eye(version, tutorial, sections, tips, eye_animation);
    }

    pub fn toggle_latest_compaction(&mut self) -> bool {
        for message in self.messages.iter_mut().rev() {
            for content in message.contents.iter_mut().rev() {
                if let MessageContent::Compaction { expanded, .. } = content {
                    *expanded = !*expanded;
                    self.invalidate_render_cache();
                    return true;
                }
            }
        }
        false
    }

    pub fn get_rendered_lines(&mut self, width: u16) -> Vec<Line<'static>> {
        self.ensure_render_cache(width).clone()
    }

    pub fn rendered_line_count(&mut self, width: u16) -> usize {
        self.ensure_render_cache(width).len()
    }

    pub fn rendered_line_count_with_provisional_assistant(
        &mut self,
        width: u16,
        provisional_assistant_text: Option<&str>,
    ) -> usize {
        if provisional_assistant_text.is_none_or(|text| text.trim().is_empty()) {
            return self.rendered_line_count(width);
        }
        self.rendered_lines_with_provisional_assistant(width, provisional_assistant_text)
            .len()
    }

    fn ensure_render_cache(&mut self, width: u16) -> &Vec<Line<'static>> {
        let needs_rebuild = self
            .render_cache
            .as_ref()
            .is_none_or(|cache| cache.width != width || cache.revision != self.render_revision);
        if needs_rebuild {
            let lines = self.compute_rendered_lines(width);
            self.render_cache = Some(RenderCache {
                width,
                revision: self.render_revision,
                lines,
            });
        }
        self.render_cache
            .as_ref()
            .map(|cache| &cache.lines)
            .unwrap_or(&EMPTY_RENDER_LINES)
    }

    fn compute_rendered_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut text_lines = Vec::new();
        let mut previous_colored_block = false;

        for msg in &self.messages {
            for content in &msg.contents {
                let current_colored_block =
                    content_renders_colored_block(msg.role.as_str(), content);
                if previous_colored_block
                    && current_colored_block
                    && text_lines.last().is_none_or(|line| {
                        !(is_visual_blank_line(line) && dominant_block_bg(line).is_none())
                    })
                {
                    text_lines.push(Line::from(""));
                }
                match content {
                    MessageContent::RenderedLines(lines) => {
                        for line in lines {
                            if let Some(normalized) = normalize_rendered_system_line(line) {
                                if normalized.trim().is_empty() {
                                    text_lines.push(Line::from(""));
                                    continue;
                                }
                                text_lines.extend(render_rendered_system_line(&normalized, width));
                            }
                        }
                    }
                    MessageContent::StartupHeader {
                        version,
                        tutorial,
                        sections,
                        tips,
                        eye_animation,
                    } => {
                        let elapsed = self.startup_animation_started_at.elapsed();
                        let tip_state = startup_tip_render_state(tips, elapsed);
                        text_lines.extend(render_startup_header_lines(
                            version,
                            tutorial,
                            sections,
                            tip_state.as_ref(),
                            *eye_animation,
                            elapsed,
                            width,
                        ));
                    }
                    MessageContent::Markdown(md) => {
                        let is_user = msg.role == "You";
                        let markdown_width = width.saturating_sub(2) as usize;
                        let md_lines =
                            markdown::render_markdown_to_lines_with_width(md, Some(markdown_width));

                        if is_user {
                            let mut padding =
                                Line::from(vec![Span::raw(" ".repeat(width as usize))]);
                            for span in &mut padding.spans {
                                span.style = span.style.bg(SURFACE_USER_MSG_BG);
                            }
                            text_lines.push(padding);

                            for line in render_user_markdown_lines(md_lines, width) {
                                text_lines.push(user_block_line(line));
                            }
                            let mut padding =
                                Line::from(vec![Span::raw(" ".repeat(width as usize))]);
                            for span in &mut padding.spans {
                                span.style = span.style.bg(SURFACE_USER_MSG_BG);
                            }
                            text_lines.push(padding);
                        } else {
                            let wrapped_lines = wrap_assistant_markdown_lines(md_lines, width);
                            text_lines.push(Line::from(""));
                            text_lines.extend(wrapped_lines);
                            text_lines.push(Line::from(""));
                        }
                    }
                    MessageContent::Diff { title, content } => {
                        text_lines.extend(render_diff_block_lines(
                            title.as_deref(),
                            content.as_str(),
                            width,
                        ));
                    }
                    MessageContent::Image { alt, url } => {
                        text_lines.extend(render_image_block_lines(alt, url, width));
                    }
                    MessageContent::ToolCall {
                        title,
                        lines,
                        status,
                    } => {
                        text_lines.extend(render_tool_block_lines(title, lines, *status, width));
                    }
                    MessageContent::Error {
                        title,
                        summary,
                        details,
                    } => {
                        text_lines.extend(render_error_block_lines(title, summary, details, width));
                    }
                    MessageContent::Compaction {
                        turn_count,
                        summary,
                        expanded,
                    } => {
                        text_lines.extend(render_compaction_block_lines(
                            *turn_count,
                            summary.as_str(),
                            *expanded,
                            width,
                        ));
                    }
                }
                previous_colored_block = current_colored_block;
            }
        }

        for line in &mut text_lines {
            let is_user_bg = line
                .spans
                .iter()
                .any(|span| span.style.bg == Some(SURFACE_USER_MSG_BG));
            let is_compaction_bg = line
                .spans
                .iter()
                .any(|span| span.style.bg == Some(SURFACE_COMPACTION_BG));
            let background = if is_user_bg {
                Some(SURFACE_USER_MSG_BG)
            } else if is_compaction_bg {
                Some(SURFACE_COMPACTION_BG)
            } else {
                None
            };
            if let Some(background) = background {
                pad_and_bg(line, width, background);
            } else {
                pad_plain(line, width);
            }
        }

        text_lines
    }

    fn invalidate_render_cache(&mut self) {
        self.render_revision = self.render_revision.saturating_add(1);
        self.render_cache = None;
        self.viewport_cache = None;
        self.scroll_state.note_cache_invalidated();
    }

    pub fn trailing_colored_block(&mut self, width: u16) -> bool {
        self.ensure_render_cache(width)
            .iter()
            .rev()
            .find(|line| !is_visual_blank_line(line) || dominant_block_bg(line).is_some())
            .and_then(dominant_block_bg)
            .is_some()
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        self.page_step = page_step_for_height(area.height);
        self.mouse_step = mouse_step_for_height(area.height);
        let startup_mode = self.startup_mode_active();
        let (rendered_line_count, top_padding) = {
            let rendered_lines = self.ensure_render_cache(area.width);
            let top_padding = if startup_mode {
                startup_top_padding(rendered_lines.len(), area.height)
            } else {
                0
            };
            (rendered_lines.len(), top_padding)
        };

        let total_lines = rendered_line_count.saturating_add(top_padding);
        if total_lines == 0 {
            self.last_render_height = area.height;
            self.scroll_state.reset_for_empty_render();
            f.render_widget(
                Paragraph::new(Text::from(Vec::<Line<'static>>::new())),
                area,
            );
            return;
        }
        let max_scroll_start = total_lines.saturating_sub(area.height as usize);
        let raw_scroll_val = self.scroll_state.raw_scroll_start(max_scroll_start);
        let mut scroll_start = if self.scroll_state.follow_tail() {
            raw_scroll_val
        } else if !self.scroll_state.snap_on_next_render() {
            self.scroll_state.last_scroll_start().min(max_scroll_start)
        } else {
            let text_lines = self.get_rendered_lines(area.width);
            let centered_lines = if startup_mode {
                vertically_center_startup_lines(text_lines, area.height)
            } else {
                text_lines
            };
            adjust_scroll_start_for_message_boundary(&centered_lines, raw_scroll_val)
        };
        scroll_start = scroll_start.min(max_scroll_start);
        self.last_render_height = area.height;
        self.scroll_state
            .apply_rendered_scroll_start(max_scroll_start, scroll_start);

        let visible_lines = self.viewport_lines(area.width, area.height, scroll_start, top_padding);
        let paragraph = Paragraph::new(Text::from(visible_lines));

        f.render_widget(paragraph, area);
    }

    pub fn render_with_provisional_assistant(
        &mut self,
        f: &mut Frame,
        area: Rect,
        provisional_assistant_text: Option<&str>,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        if provisional_assistant_text.is_none_or(|text| text.trim().is_empty()) {
            self.render(f, area);
            return;
        }

        let rendered_lines =
            self.rendered_lines_with_provisional_assistant(area.width, provisional_assistant_text);
        self.render_explicit_lines(f, area, rendered_lines, self.startup_mode_active());
    }

    fn rendered_lines_with_provisional_assistant(
        &mut self,
        width: u16,
        provisional_assistant_text: Option<&str>,
    ) -> Vec<Line<'static>> {
        let mut rendered_lines = self.get_rendered_lines(width);
        let Some(text) = provisional_assistant_text.map(str::trim) else {
            return rendered_lines;
        };
        if text.is_empty() {
            return rendered_lines;
        }
        rendered_lines.extend(render_provisional_assistant_message_lines(text, width));
        rendered_lines
    }

    fn render_explicit_lines(
        &mut self,
        f: &mut Frame,
        area: Rect,
        rendered_lines: Vec<Line<'static>>,
        startup_mode: bool,
    ) {
        self.page_step = page_step_for_height(area.height);
        self.mouse_step = mouse_step_for_height(area.height);
        let top_padding = if startup_mode {
            startup_top_padding(rendered_lines.len(), area.height)
        } else {
            0
        };

        let total_lines = rendered_lines.len().saturating_add(top_padding);
        if total_lines == 0 {
            self.last_render_height = area.height;
            self.scroll_state.reset_for_empty_render();
            f.render_widget(
                Paragraph::new(Text::from(Vec::<Line<'static>>::new())),
                area,
            );
            return;
        }
        let max_scroll_start = total_lines.saturating_sub(area.height as usize);
        let raw_scroll_val = self.scroll_state.raw_scroll_start(max_scroll_start);
        let mut scroll_start = if self.scroll_state.follow_tail() {
            raw_scroll_val
        } else if !self.scroll_state.snap_on_next_render() {
            self.scroll_state.last_scroll_start().min(max_scroll_start)
        } else {
            let centered_lines = if startup_mode {
                vertically_center_startup_lines(rendered_lines.clone(), area.height)
            } else {
                rendered_lines.clone()
            };
            adjust_scroll_start_for_message_boundary(&centered_lines, raw_scroll_val)
        };
        scroll_start = scroll_start.min(max_scroll_start);
        self.last_render_height = area.height;
        self.scroll_state
            .apply_rendered_scroll_start(max_scroll_start, scroll_start);

        let visible_end = scroll_start.saturating_add(area.height as usize);
        let visible_lines = (scroll_start..visible_end)
            .filter_map(|visual_index| {
                if visual_index < top_padding {
                    Some(Line::from(""))
                } else {
                    rendered_lines
                        .get(visual_index.saturating_sub(top_padding))
                        .cloned()
                }
            })
            .collect::<Vec<_>>();
        let paragraph = Paragraph::new(Text::from(visible_lines));

        f.render_widget(paragraph, area);
    }

    fn viewport_lines(
        &mut self,
        width: u16,
        height: u16,
        scroll_start: usize,
        top_padding: usize,
    ) -> Vec<Line<'static>> {
        if let Some(cache) = self.viewport_cache.as_ref()
            && cache.width == width
            && cache.revision == self.render_revision
            && cache.height == height
            && cache.scroll_start == scroll_start
            && cache.top_padding == top_padding
        {
            return cache.lines.clone();
        }

        let visible_end = scroll_start.saturating_add(height as usize);
        let lines = {
            let rendered_lines = self.ensure_render_cache(width);
            (scroll_start..visible_end)
                .filter_map(|visual_index| {
                    if visual_index < top_padding {
                        Some(Line::from(""))
                    } else {
                        rendered_lines
                            .get(visual_index.saturating_sub(top_padding))
                            .cloned()
                    }
                })
                .collect::<Vec<_>>()
        };

        self.viewport_cache = Some(ViewportRenderCache {
            width,
            revision: self.render_revision,
            height,
            scroll_start,
            top_padding,
            lines: lines.clone(),
        });
        lines
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll_state.scroll_line_up(),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_state.scroll_line_down(),
            KeyCode::PageUp | KeyCode::Char(' ') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.scroll_state.scroll_page_up(self.page_step)
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.scroll_state.scroll_page_down(self.page_step)
            }
            KeyCode::Home => self.scroll_state.jump_home(),
            KeyCode::End => self.scroll_state.jump_end(),
            KeyCode::Backspace
            | KeyCode::Enter
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::PageUp
            | KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Delete
            | KeyCode::Insert
            | KeyCode::F(_)
            | KeyCode::Char(_)
            | KeyCode::Null
            | KeyCode::Esc
            | KeyCode::CapsLock
            | KeyCode::ScrollLock
            | KeyCode::NumLock
            | KeyCode::PrintScreen
            | KeyCode::Pause
            | KeyCode::Menu
            | KeyCode::KeypadBegin
            | KeyCode::Media(_)
            | KeyCode::Modifier(_) => {}
        }
    }

    pub fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        match mouse.kind {
            crossterm::event::MouseEventKind::ScrollUp => {
                self.scroll_state.scroll_page_up(self.mouse_step)
            }
            crossterm::event::MouseEventKind::ScrollDown => {
                self.scroll_state.scroll_page_down(self.mouse_step)
            }
            crossterm::event::MouseEventKind::Down(_)
            | crossterm::event::MouseEventKind::Up(_)
            | crossterm::event::MouseEventKind::Drag(_)
            | crossterm::event::MouseEventKind::Moved
            | crossterm::event::MouseEventKind::ScrollLeft
            | crossterm::event::MouseEventKind::ScrollRight => {}
        }
    }

    pub fn is_following_tail(&self) -> bool {
        self.scroll_state.follow_tail()
    }

    #[cfg(test)]
    pub(crate) fn scroll_offset_for_test(&self) -> u16 {
        self.scroll_state.scroll_offset()
    }

    #[cfg(test)]
    pub(crate) fn set_scroll_offset_for_test(&mut self, value: u16) {
        self.scroll_state.set_scroll_offset_for_test(value);
    }

    #[cfg(test)]
    pub(crate) fn last_scroll_start_for_test(&self) -> usize {
        self.scroll_state.last_scroll_start()
    }

    #[cfg(test)]
    pub(crate) fn set_last_scroll_start_for_test(&mut self, value: usize) {
        self.scroll_state.set_last_scroll_start_for_test(value);
    }

    #[cfg(test)]
    pub(crate) fn snap_scroll_on_next_render_for_test(&self) -> bool {
        self.scroll_state.snap_on_next_render()
    }

    #[cfg(test)]
    pub(crate) fn set_snap_scroll_on_next_render_for_test(&mut self, value: bool) {
        self.scroll_state.set_snap_on_next_render_for_test(value);
    }

    #[cfg(test)]
    pub(crate) fn rewind_startup_animation_for_test(&mut self, duration: Duration) {
        self.startup_animation_started_at = Instant::now() - duration;
    }

    pub fn refresh_startup_animation(&mut self) -> bool {
        if reduced_motion_enabled() {
            self.last_startup_animation_signature = None;
            return false;
        }
        let signature = self.startup_animation_signature();
        if signature == self.last_startup_animation_signature {
            return false;
        }
        self.last_startup_animation_signature = signature;
        if signature.is_some() {
            self.invalidate_render_cache();
            return true;
        }
        false
    }

    pub fn startup_animation_active(&self) -> bool {
        self.startup_animation_signature().is_some()
    }

    fn startup_animation_signature(&self) -> Option<u64> {
        if reduced_motion_enabled() {
            return None;
        }
        if !self.has_startup_header() {
            return None;
        }

        let elapsed = self.startup_animation_started_at.elapsed();
        let eye_signature = self
            .startup_eye_animation()
            .map(|animation| startup_eye_signature(animation, elapsed) as u64)
            .unwrap_or_else(|| startup_logo_eye_frame_index(elapsed) as u64);
        let tip_signature = self
            .startup_tips()
            .and_then(|tips| {
                let tip_count = tips.len();
                let (_, intensity_step) = startup_tip_cycle_state(tip_count, elapsed)?;
                let tip_index = startup_tip_index(tip_count, elapsed)?;
                Some(((tip_index as u64) << 8) | intensity_step)
            })
            .unwrap_or(0);

        Some((eye_signature << 16) | tip_signature)
    }

    fn startup_tips(&self) -> Option<&[String]> {
        if self
            .messages
            .iter()
            .any(|message| message.role == "You" || message.role == "Assistant")
        {
            return None;
        }

        self.messages
            .iter()
            .flat_map(|message| message.contents.iter())
            .find_map(|content| match content {
                MessageContent::StartupHeader { tips, .. } if !tips.is_empty() => {
                    Some(tips.as_slice())
                }
                MessageContent::RenderedLines(_)
                | MessageContent::Markdown(_)
                | MessageContent::Diff { .. }
                | MessageContent::Image { .. }
                | MessageContent::ToolCall { .. }
                | MessageContent::Error { .. }
                | MessageContent::Compaction { .. }
                | MessageContent::StartupHeader { .. } => None,
            })
    }

    fn startup_eye_animation(&self) -> Option<StartupEyeAnimation> {
        if self
            .messages
            .iter()
            .any(|message| message.role == "You" || message.role == "Assistant")
        {
            return None;
        }

        self.messages
            .iter()
            .flat_map(|message| message.contents.iter())
            .find_map(|content| match content {
                MessageContent::StartupHeader { eye_animation, .. } => Some(*eye_animation),
                MessageContent::RenderedLines(_)
                | MessageContent::Markdown(_)
                | MessageContent::Diff { .. }
                | MessageContent::Image { .. }
                | MessageContent::ToolCall { .. }
                | MessageContent::Error { .. }
                | MessageContent::Compaction { .. } => None,
            })
    }

    fn startup_mode_active(&self) -> bool {
        self.messages.iter().all(|message| message.role == "System") && self.has_startup_header()
    }

    fn has_startup_header(&self) -> bool {
        self.messages
            .iter()
            .flat_map(|message| message.contents.iter())
            .any(|content| matches!(content, MessageContent::StartupHeader { .. }))
    }
}

fn page_step_for_height(height: u16) -> u16 {
    height.saturating_sub(2).max(1)
}

fn mouse_step_for_height(height: u16) -> u16 {
    page_step_for_height(height).saturating_add(3) / 4
}

fn pad_and_bg(line: &mut Line, width: u16, bg: Color) {
    let line_len: usize = line.spans.iter().map(|s| s.width()).sum();
    let pad_len = (width as usize).saturating_sub(line_len);
    if pad_len > 0 {
        line.spans.push(Span::raw(" ".repeat(pad_len)));
    }
    for span in &mut line.spans {
        span.style = span.style.bg(bg);
    }
}

fn pad_plain(line: &mut Line, width: u16) {
    let line_len: usize = line.spans.iter().map(|s| s.width()).sum();
    let pad_len = (width as usize).saturating_sub(line_len);
    if pad_len > 0 {
        line.spans.push(Span::raw(" ".repeat(pad_len)));
    }
}

fn adjust_scroll_start_for_message_boundary(lines: &[Line<'static>], start: usize) -> usize {
    let adjusted_for_block = adjust_scroll_start_for_block_boundary(lines, start);
    if adjusted_for_block == start && lines.get(start).and_then(dominant_block_bg).is_none() {
        return start;
    }
    let start = adjusted_for_block;
    if start == 0 || start >= lines.len() || lines.get(start).is_some_and(is_visual_blank_line) {
        return start;
    }

    let lookback = start.saturating_sub(4);
    for index in (lookback..start).rev() {
        if lines.get(index).is_some_and(is_visual_blank_line) {
            return index + 1;
        }
    }
    start
}

fn adjust_scroll_start_for_block_boundary(lines: &[Line<'static>], start: usize) -> usize {
    if start == 0 || start >= lines.len() {
        return start;
    }
    if lines.get(start).is_some_and(|line| line.spans.is_empty()) {
        return start;
    }
    let Some(bg) = lines.get(start).and_then(dominant_block_bg) else {
        return start;
    };
    let mut adjusted = start;
    while adjusted > 0 && lines.get(adjusted - 1).and_then(dominant_block_bg) == Some(bg) {
        adjusted -= 1;
    }
    if lines.get(adjusted).is_some_and(is_visual_blank_line) {
        let mut candidate = adjusted + 1;
        while candidate < lines.len()
            && lines.get(candidate).and_then(dominant_block_bg) == Some(bg)
        {
            if lines
                .get(candidate)
                .is_some_and(|line| !is_visual_blank_line(line))
            {
                return candidate;
            }
            candidate += 1;
        }
    }
    adjusted
}

fn is_visual_blank_line(line: &Line<'static>) -> bool {
    line.spans
        .iter()
        .all(|span| span.content.as_ref().trim().is_empty())
}

fn dominant_block_bg(line: &Line<'static>) -> Option<Color> {
    if line
        .spans
        .iter()
        .any(|span| span.style.bg == Some(SURFACE_USER_MSG_BG))
    {
        return Some(SURFACE_USER_MSG_BG);
    }
    if line
        .spans
        .iter()
        .any(|span| span.style.bg == Some(SURFACE_TOOL_BG))
    {
        return Some(SURFACE_TOOL_BG);
    }
    if line
        .spans
        .iter()
        .any(|span| span.style.bg == Some(SURFACE_COMPACTION_BG))
    {
        return Some(SURFACE_COMPACTION_BG);
    }
    None
}

fn normalize_rendered_system_line(line: &str) -> Option<String> {
    let trimmed = line.trim_end();

    if trimmed.starts_with("╭─ ") {
        return Some(trimmed.trim_start_matches("╭─ ").to_owned());
    }
    if trimmed == "╰─" {
        return None;
    }
    if trimmed == "│" {
        return Some(String::new());
    }
    if let Some(rest) = trimmed.strip_prefix("│ ") {
        return Some(rest.to_owned());
    }
    if let Some(rest) = trimmed.strip_prefix("│") {
        return Some(rest.trim_start().to_owned());
    }

    Some(trimmed.to_owned())
}

fn render_provisional_assistant_message_lines(text: &str, width: u16) -> Vec<Line<'static>> {
    let mut list = MessageList::new();
    list.add_assistant_message(text.to_owned());
    list.get_rendered_lines(width)
}

fn render_rendered_system_line(line: &str, width: u16) -> Vec<Line<'static>> {
    let content_width = width as usize;

    if let Some(rendered) = render_system_activity_headline(line, content_width) {
        return rendered;
    }

    if let Some(rendered) = render_system_activity_child(line, content_width) {
        return rendered;
    }

    let style = if line.trim_start().starts_with("… +") {
        Style::default()
            .fg(SURFACE_GRAY)
            .add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(SURFACE_DARK_GRAY)
    };

    crate::presentation::render_wrapped_plain_display_line(line, content_width)
        .into_iter()
        .map(|wrapped| Line::from(vec![Span::styled(wrapped, style)]))
        .collect()
}

fn render_system_activity_headline(line: &str, content_width: usize) -> Option<Vec<Line<'static>>> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("• ")?;

    let (label, body, label_style) = if let Some(body) = rest.strip_prefix("Ran ") {
        (
            "Ran",
            body,
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
        )
    } else if let Some(body) = rest.strip_prefix("Explored ") {
        (
            "Explored",
            body,
            Style::default()
                .fg(ratatui::style::Color::White)
                .add_modifier(Modifier::BOLD),
        )
    } else if let Some(body) = rest.strip_prefix("Called ") {
        (
            "Called",
            body,
            Style::default()
                .fg(SURFACE_ACCENT)
                .add_modifier(Modifier::BOLD),
        )
    } else if let Some(body) = rest.strip_prefix("Closed ") {
        (
            "Closed",
            body,
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        return None;
    };

    let body_width = content_width
        .saturating_sub(2 + crate::presentation::display_width(label) + 1)
        .max(1);
    let wrapped = crate::presentation::render_wrapped_plain_display_line(body, body_width);

    Some(
        wrapped
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                if index == 0 {
                    Line::from(vec![
                        Span::styled("• ", Style::default().fg(SURFACE_GREEN)),
                        Span::styled(format!("{label} "), label_style),
                        Span::styled(
                            wrapped_line,
                            Style::default().fg(ratatui::style::Color::White),
                        ),
                    ])
                } else {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::raw(" ".repeat(crate::presentation::display_width(label) + 1)),
                        Span::styled(
                            wrapped_line,
                            Style::default().fg(ratatui::style::Color::White),
                        ),
                    ])
                }
            })
            .collect(),
    )
}

fn render_system_activity_child(line: &str, content_width: usize) -> Option<Vec<Line<'static>>> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("└ ")?;

    let (label, body) = if let Some(body) = rest.strip_prefix("Read ") {
        ("Read", body)
    } else if let Some(body) = rest.strip_prefix("List ") {
        ("List", body)
    } else if let Some(body) = rest.strip_prefix("Search ") {
        ("Search", body)
    } else if let Some(body) = rest.strip_prefix("Inspect ") {
        ("Inspect", body)
    } else {
        return None;
    };

    let body_width = content_width
        .saturating_sub(2 + 2 + crate::presentation::display_width(label) + 1)
        .max(1);
    let wrapped = crate::presentation::render_wrapped_plain_display_line(body, body_width);

    Some(
        wrapped
            .into_iter()
            .enumerate()
            .map(|(index, wrapped_line)| {
                if index == 0 {
                    Line::from(vec![
                        Span::raw("  "),
                        Span::styled(
                            "└ ",
                            Style::default()
                                .fg(SURFACE_GRAY)
                                .add_modifier(Modifier::DIM),
                        ),
                        Span::styled(format!("{label} "), Style::default().fg(SURFACE_ACCENT)),
                        Span::styled(
                            wrapped_line,
                            Style::default().fg(ratatui::style::Color::White),
                        ),
                    ])
                } else {
                    Line::from(vec![
                        Span::raw("    "),
                        Span::raw(" ".repeat(crate::presentation::display_width(label) + 1)),
                        Span::styled(
                            wrapped_line,
                            Style::default().fg(ratatui::style::Color::White),
                        ),
                    ])
                }
            })
            .collect(),
    )
}

fn content_renders_colored_block(role: &str, content: &MessageContent) -> bool {
    match content {
        MessageContent::Markdown(_) => role == "You",
        MessageContent::Diff { .. }
        | MessageContent::ToolCall { .. }
        | MessageContent::Compaction { .. } => true,
        MessageContent::Error { .. }
        | MessageContent::RenderedLines(_)
        | MessageContent::Image { .. }
        | MessageContent::StartupHeader { .. } => false,
    }
}

