fn render_startup_header_lines(
    version: &str,
    tutorial: &str,
    sections: &[(String, Vec<String>)],
    tip_state: Option<&StartupTipRenderState>,
    eye_animation: StartupEyeAnimation,
    elapsed: Duration,
    width: u16,
) -> Vec<Line<'static>> {
    let mut rendered = Vec::new();

    rendered.push(Line::from(""));
    rendered.push(Line::from(""));
    rendered.extend(render_centered_logo_lines(width, eye_animation, elapsed));
    rendered.push(Line::from(""));
    rendered.extend(render_centered_startup_text_lines(
        version,
        width,
        Style::default()
            .fg(SURFACE_ACCENT)
            .add_modifier(Modifier::BOLD),
    ));

    let startup_status = sections
        .iter()
        .filter_map(|(title, values)| {
            values.first().map(|value| StartupStatusItem {
                title: title.clone(),
                value: value.clone(),
                count: value.parse::<usize>().ok(),
            })
        })
        .collect::<Vec<_>>();
    if !startup_status.is_empty() {
        rendered.push(Line::from(""));
        rendered.extend(render_startup_status_lines(&startup_status, width));
    }

    rendered.push(Line::from(""));
    let mut rendered_tip = false;
    if let Some(tip_state) = tip_state {
        rendered.extend(render_startup_tip_lines(tip_state, width));
        rendered_tip = true;
    } else if !tutorial.trim().is_empty() {
        let fallback_tip = StartupTipRenderState::steady(format!("• {tutorial}"));
        rendered.extend(render_startup_tip_lines(&fallback_tip, width));
        rendered_tip = true;
    }
    if rendered_tip {
        rendered.push(Line::from(""));
    }

    rendered
}

#[derive(Debug, Clone)]
struct StartupStatusItem {
    title: String,
    value: String,
    count: Option<usize>,
}

impl StartupStatusItem {
    fn display_width(&self) -> usize {
        crate::presentation::display_width(self.label_text().as_str())
            + self
                .marker_text()
                .map_or(0, |marker| 1 + crate::presentation::display_width(marker))
    }

    fn label_text(&self) -> String {
        self.count.map_or_else(
            || format!("{} · {}", self.title, self.value),
            |count| format!("{} ({count})", self.title),
        )
    }

    fn marker_text(&self) -> Option<&'static str> {
        self.count.map(|count| if count > 0 { "✓" } else { "✗" })
    }

    fn marker_style(&self) -> Style {
        let color = self
            .count
            .map(|count| {
                if count > 0 {
                    SURFACE_GREEN
                } else {
                    SURFACE_RED
                }
            })
            .unwrap_or(SURFACE_GRAY);
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    }

    fn spans(&self) -> Vec<Span<'static>> {
        let mut spans = vec![Span::styled(
            self.label_text(),
            Style::default()
                .fg(SURFACE_GRAY)
                .add_modifier(Modifier::BOLD),
        )];
        if let Some(marker) = self.marker_text() {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(marker, self.marker_style()));
        }
        spans
    }
}

fn render_startup_status_lines(items: &[StartupStatusItem], width: u16) -> Vec<Line<'static>> {
    const GAP: &str = "   ";
    let width = width as usize;
    let gap_width = crate::presentation::display_width(GAP);
    let joined_width = items
        .iter()
        .map(StartupStatusItem::display_width)
        .sum::<usize>()
        + gap_width.saturating_mul(items.len().saturating_sub(1));

    if joined_width <= width {
        let mut spans = vec![Span::raw(" ".repeat((width - joined_width) / 2))];
        for (index, item) in items.iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw(GAP));
            }
            spans.extend(item.spans());
        }
        return vec![Line::from(spans)];
    }

    items
        .iter()
        .map(|item| {
            let item_width = item.display_width();
            let mut spans = vec![Span::raw(" ".repeat(width.saturating_sub(item_width) / 2))];
            spans.extend(item.spans());
            Line::from(spans)
        })
        .collect()
}

fn render_centered_logo_lines(
    width: u16,
    eye_animation: StartupEyeAnimation,
    elapsed: Duration,
) -> Vec<Line<'static>> {
    let max_logo_width = STARTUP_WORDMARK
        .iter()
        .map(|line| crate::presentation::display_width(line))
        .max()
        .unwrap_or(0);
    let compact_logo_width = STARTUP_COMPACT_WORDMARK
        .iter()
        .map(|line| crate::presentation::display_width(line))
        .max()
        .unwrap_or(0);

    let available_full_logo_width = width as usize;
    let (logo_lines, base_logo_lines): (Vec<String>, Vec<String>) =
        if max_logo_width.saturating_add(STARTUP_FULL_WORDMARK_MARGIN) <= available_full_logo_width
        {
            (
                startup_wordmark_eye_frame_for_animation(eye_animation, elapsed),
                STARTUP_WORDMARK
                    .iter()
                    .map(|line| (*line).to_owned())
                    .collect(),
            )
        } else if compact_logo_width.saturating_add(STARTUP_COMPACT_WORDMARK_MARGIN)
            <= available_full_logo_width
        {
            (
                STARTUP_COMPACT_WORDMARK
                    .iter()
                    .map(|line| (*line).to_owned())
                    .collect(),
                STARTUP_COMPACT_WORDMARK
                    .iter()
                    .map(|line| (*line).to_owned())
                    .collect(),
            )
        } else {
            (vec!["LOONG".to_owned()], vec!["LOONG".to_owned()])
        };
    let target_width = logo_lines
        .iter()
        .map(|line| crate::presentation::display_width(line))
        .max()
        .unwrap_or(0);

    logo_lines
        .into_iter()
        .zip(base_logo_lines)
        .map(|(line, base_line)| {
            let centered = center_text_for_width(
                pad_text_to_display_width(line.as_str(), target_width).as_str(),
                width as usize,
            );
            let centered_base = center_text_for_width(
                pad_text_to_display_width(base_line.as_str(), target_width).as_str(),
                width as usize,
            );
            startup_logo_line_spans(centered, centered_base)
        })
        .collect()
}

fn startup_wordmark_eye_frame(frame_index: usize) -> Vec<String> {
    let Some(frame) = STARTUP_EYE_FRAMES
        .get(frame_index)
        .or_else(|| STARTUP_EYE_FRAMES.first())
    else {
        return STARTUP_WORDMARK
            .iter()
            .map(|line| (*line).to_owned())
            .collect();
    };

    STARTUP_WORDMARK
        .iter()
        .enumerate()
        .map(|(line_index, line)| {
            let Some(interior_row_index) = line_index.checked_sub(1) else {
                return (*line).to_owned();
            };
            let Some(pattern) = frame.get(interior_row_index) else {
                return (*line).to_owned();
            };
            apply_startup_eye_pattern(line, pattern)
        })
        .collect()
}

fn startup_wordmark_eye_frame_for_animation(
    animation: StartupEyeAnimation,
    elapsed: Duration,
) -> Vec<String> {
    match animation {
        StartupEyeAnimation::Ambient => {
            startup_wordmark_eye_frame(startup_logo_eye_frame_index(elapsed))
        }
        StartupEyeAnimation::Focus(_)
        | StartupEyeAnimation::Thinking(_)
        | StartupEyeAnimation::Confirm(_)
        | StartupEyeAnimation::Celebrate => {
            let (left_frame, right_frame, _) = startup_eye_frame_for_animation(animation, elapsed);
            STARTUP_WORDMARK
                .iter()
                .enumerate()
                .map(|(line_index, line)| {
                    let Some(interior_row_index) = line_index.checked_sub(1) else {
                        return (*line).to_owned();
                    };
                    let Some(left_pattern) = left_frame.get(interior_row_index) else {
                        return (*line).to_owned();
                    };
                    let Some(right_pattern) = right_frame.get(interior_row_index) else {
                        return (*line).to_owned();
                    };
                    apply_startup_eye_patterns(line, left_pattern, right_pattern)
                })
                .collect()
        }
    }
}

fn startup_eye_signature(animation: StartupEyeAnimation, elapsed: Duration) -> u16 {
    let (_, _, signature) = startup_eye_frame_for_animation(animation, elapsed);
    signature
}

fn startup_eye_frame_for_animation(
    animation: StartupEyeAnimation,
    elapsed: Duration,
) -> (&'static StartupEyeFrame, &'static StartupEyeFrame, u16) {
    match animation {
        StartupEyeAnimation::Ambient => {
            let frame_index = startup_logo_eye_frame_index(elapsed);
            let left_frame = STARTUP_EYE_FRAMES
                .get(frame_index)
                .or_else(|| STARTUP_EYE_FRAMES.first())
                .unwrap_or(&STARTUP_EYE_CENTER);
            let right_index = frame_index.saturating_add(3) % STARTUP_EYE_FRAMES.len().max(1);
            let right_frame = STARTUP_EYE_FRAMES
                .get(right_index)
                .or_else(|| STARTUP_EYE_FRAMES.first())
                .unwrap_or(&STARTUP_EYE_CENTER);
            (
                left_frame,
                right_frame,
                ((frame_index as u16) << 8) | right_index as u16,
            )
        }
        StartupEyeAnimation::Focus(focus) => {
            let step = startup_eye_step(elapsed, STARTUP_EYE_GUIDED_BLINK_PERIOD_STEPS);
            let (left_frame, right_frame) = if step + STARTUP_EYE_GUIDED_BLINK_WINDOW_STEPS
                >= STARTUP_EYE_GUIDED_BLINK_PERIOD_STEPS
            {
                startup_eye_half_lid_pair(focus)
            } else {
                startup_eye_focus_pair(focus)
            };
            (
                left_frame,
                right_frame,
                100 + startup_eye_focus_code(focus) + u16::from(step > 0),
            )
        }
        StartupEyeAnimation::Thinking(focus) => {
            let variant = startup_timed_choice(
                elapsed,
                &[
                    (260, 0u16),
                    (110, 1),
                    (180, 2),
                    (90, 3),
                    (220, 4),
                    (110, 5),
                    (320, 6),
                ],
            );
            let (left_frame, right_frame) = match variant {
                0 | 6 => startup_eye_focus_pair(focus),
                1 => startup_eye_neighbor_pair(focus, -1),
                2 => startup_eye_half_lid_pair(focus),
                3 => startup_eye_focus_pair(focus),
                4 => startup_eye_neighbor_pair(focus, 1),
                5 => startup_eye_half_lid_pair(focus),
                _ => startup_eye_focus_pair(focus),
            };
            (
                left_frame,
                right_frame,
                200 + startup_eye_focus_code(focus) * 8 + variant,
            )
        }
        StartupEyeAnimation::Confirm(focus) => {
            let variant = startup_timed_choice(
                elapsed,
                &[(70, 0u16), (90, 1), (80, 2), (120, 3), (90, 4), (180, 5)],
            );
            let (left_frame, right_frame) = match variant {
                0 | 5 => startup_eye_focus_pair(focus),
                1 | 2 => startup_eye_confirm_pair(focus),
                3 => startup_eye_half_lid_pair(focus),
                4 => startup_eye_confirm_pair(focus),
                _ => startup_eye_focus_pair(focus),
            };
            (
                left_frame,
                right_frame,
                300 + startup_eye_focus_code(focus) * 8 + variant,
            )
        }
        StartupEyeAnimation::Celebrate => {
            let variant = startup_timed_choice(
                elapsed,
                &[
                    (90, 0u16),
                    (100, 1),
                    (120, 2),
                    (100, 3),
                    (120, 4),
                    (100, 5),
                    (220, 6),
                ],
            );
            let (left_frame, right_frame) = match variant {
                0 | 4 => (&STARTUP_EYE_CONFIRM_LEFT, &STARTUP_EYE_CONFIRM_RIGHT),
                1 => (&STARTUP_EYE_CELEBRATE_A, &STARTUP_EYE_CELEBRATE_B),
                2 => (&STARTUP_EYE_CELEBRATE_B, &STARTUP_EYE_CELEBRATE_A),
                3 => (&STARTUP_EYE_CONFIRM_CENTER, &STARTUP_EYE_CONFIRM_CENTER),
                5 => (&STARTUP_EYE_CONFIRM_RIGHT, &STARTUP_EYE_CONFIRM_LEFT),
                6 => (&STARTUP_EYE_CENTER, &STARTUP_EYE_CENTER),
                _ => (&STARTUP_EYE_CENTER, &STARTUP_EYE_CENTER),
            };
            (left_frame, right_frame, 400 + variant)
        }
    }
}

fn startup_timed_choice(elapsed: Duration, schedule: &[(u64, u16)]) -> u16 {
    let total = schedule
        .iter()
        .map(|(duration, _)| *duration)
        .sum::<u64>()
        .max(1);
    let mut remaining = (elapsed.as_millis() as u64) % total;
    for (duration, value) in schedule {
        if remaining < *duration {
            return *value;
        }
        remaining = remaining.saturating_sub(*duration);
    }
    schedule.last().map(|(_, value)| *value).unwrap_or(0)
}

fn startup_eye_step(elapsed: Duration, period_steps: u64) -> u64 {
    (elapsed.as_millis() as u64 / STARTUP_LOGO_EYE_FRAME_MS.max(1)) % period_steps.max(1)
}

fn startup_eye_focus_frame(focus: StartupEyeFocus) -> &'static StartupEyeFrame {
    match focus {
        StartupEyeFocus::Center => &STARTUP_EYE_CENTER,
        StartupEyeFocus::Left => &STARTUP_EYE_LEFT,
        StartupEyeFocus::Right => &STARTUP_EYE_RIGHT,
        StartupEyeFocus::Up => &STARTUP_EYE_UP,
        StartupEyeFocus::DownCenter => &STARTUP_EYE_DOWN_CENTER,
        StartupEyeFocus::DownLeft => &STARTUP_EYE_DOWN_LEFT,
        StartupEyeFocus::DownRight => &STARTUP_EYE_DOWN_RIGHT,
    }
}

fn startup_eye_focus_pair(
    focus: StartupEyeFocus,
) -> (&'static StartupEyeFrame, &'static StartupEyeFrame) {
    match focus {
        StartupEyeFocus::Center => (&STARTUP_EYE_CENTER, &STARTUP_EYE_CENTER),
        StartupEyeFocus::Left => (&STARTUP_EYE_LEFT, &STARTUP_EYE_LEFT),
        StartupEyeFocus::Right => (&STARTUP_EYE_RIGHT, &STARTUP_EYE_RIGHT),
        StartupEyeFocus::Up => (&STARTUP_EYE_UP, &STARTUP_EYE_UP),
        StartupEyeFocus::DownCenter => (&STARTUP_EYE_DOWN_RIGHT, &STARTUP_EYE_DOWN_LEFT),
        StartupEyeFocus::DownLeft => (&STARTUP_EYE_DOWN_LEFT, &STARTUP_EYE_DOWN_CENTER),
        StartupEyeFocus::DownRight => (&STARTUP_EYE_DOWN_CENTER, &STARTUP_EYE_DOWN_RIGHT),
    }
}

#[allow(dead_code)]
fn startup_eye_half_lid_frame(focus: StartupEyeFocus) -> &'static StartupEyeFrame {
    match focus {
        StartupEyeFocus::Center | StartupEyeFocus::Up => &STARTUP_EYE_HALF_LID_CENTER,
        StartupEyeFocus::Left | StartupEyeFocus::DownLeft => &STARTUP_EYE_HALF_LID_LEFT,
        StartupEyeFocus::Right | StartupEyeFocus::DownRight => &STARTUP_EYE_HALF_LID_RIGHT,
        StartupEyeFocus::DownCenter => &STARTUP_EYE_HALF_LID_DOWN_CENTER,
    }
}

fn startup_eye_half_lid_pair(
    focus: StartupEyeFocus,
) -> (&'static StartupEyeFrame, &'static StartupEyeFrame) {
    match focus {
        StartupEyeFocus::Center => (&STARTUP_EYE_HALF_LID_CENTER, &STARTUP_EYE_HALF_LID_CENTER),
        StartupEyeFocus::Left => (&STARTUP_EYE_HALF_LID_LEFT, &STARTUP_EYE_HALF_LID_LEFT),
        StartupEyeFocus::Right => (&STARTUP_EYE_HALF_LID_RIGHT, &STARTUP_EYE_HALF_LID_RIGHT),
        StartupEyeFocus::Up => (&STARTUP_EYE_HALF_LID_CENTER, &STARTUP_EYE_HALF_LID_CENTER),
        StartupEyeFocus::DownCenter => (
            &STARTUP_EYE_HALF_LID_DOWN_RIGHT,
            &STARTUP_EYE_HALF_LID_DOWN_LEFT,
        ),
        StartupEyeFocus::DownLeft => (
            &STARTUP_EYE_HALF_LID_DOWN_LEFT,
            &STARTUP_EYE_HALF_LID_DOWN_CENTER,
        ),
        StartupEyeFocus::DownRight => (
            &STARTUP_EYE_HALF_LID_DOWN_CENTER,
            &STARTUP_EYE_HALF_LID_DOWN_RIGHT,
        ),
    }
}

#[allow(dead_code)]
fn startup_eye_confirm_frame(focus: StartupEyeFocus) -> &'static StartupEyeFrame {
    match focus {
        StartupEyeFocus::Center | StartupEyeFocus::Up => &STARTUP_EYE_CONFIRM_CENTER,
        StartupEyeFocus::Left | StartupEyeFocus::DownLeft => &STARTUP_EYE_CONFIRM_LEFT,
        StartupEyeFocus::Right | StartupEyeFocus::DownRight => &STARTUP_EYE_CONFIRM_RIGHT,
        StartupEyeFocus::DownCenter => &STARTUP_EYE_CONFIRM_DOWN_CENTER,
    }
}

fn startup_eye_confirm_pair(
    focus: StartupEyeFocus,
) -> (&'static StartupEyeFrame, &'static StartupEyeFrame) {
    match focus {
        StartupEyeFocus::Center => (&STARTUP_EYE_CONFIRM_CENTER, &STARTUP_EYE_CONFIRM_CENTER),
        StartupEyeFocus::Left => (&STARTUP_EYE_CONFIRM_LEFT, &STARTUP_EYE_CONFIRM_LEFT),
        StartupEyeFocus::Right => (&STARTUP_EYE_CONFIRM_RIGHT, &STARTUP_EYE_CONFIRM_RIGHT),
        StartupEyeFocus::Up => (&STARTUP_EYE_CONFIRM_CENTER, &STARTUP_EYE_CONFIRM_CENTER),
        StartupEyeFocus::DownCenter => (
            &STARTUP_EYE_CONFIRM_DOWN_RIGHT,
            &STARTUP_EYE_CONFIRM_DOWN_LEFT,
        ),
        StartupEyeFocus::DownLeft => (
            &STARTUP_EYE_CONFIRM_DOWN_LEFT,
            &STARTUP_EYE_CONFIRM_DOWN_CENTER,
        ),
        StartupEyeFocus::DownRight => (
            &STARTUP_EYE_CONFIRM_DOWN_CENTER,
            &STARTUP_EYE_CONFIRM_DOWN_RIGHT,
        ),
    }
}

#[allow(dead_code)]
fn startup_eye_neighbor_frame(
    focus: StartupEyeFocus,
    horizontal_delta: i8,
) -> &'static StartupEyeFrame {
    match (focus, horizontal_delta.signum()) {
        (StartupEyeFocus::DownCenter, -1) => &STARTUP_EYE_DOWN_LEFT,
        (StartupEyeFocus::DownCenter, 1) => &STARTUP_EYE_DOWN_RIGHT,
        (StartupEyeFocus::Center, -1) => &STARTUP_EYE_LEFT,
        (StartupEyeFocus::Center, 1) => &STARTUP_EYE_RIGHT,
        (StartupEyeFocus::Left | StartupEyeFocus::DownLeft, 1) => &STARTUP_EYE_CENTER,
        (StartupEyeFocus::Right | StartupEyeFocus::DownRight, -1) => &STARTUP_EYE_CENTER,
        (StartupEyeFocus::Up, -1) => &STARTUP_EYE_LEFT,
        (StartupEyeFocus::Up, 1) => &STARTUP_EYE_RIGHT,
        _ => startup_eye_focus_frame(focus),
    }
}

fn startup_eye_neighbor_pair(
    focus: StartupEyeFocus,
    horizontal_delta: i8,
) -> (&'static StartupEyeFrame, &'static StartupEyeFrame) {
    let (left_focus, right_focus) = match (focus, horizontal_delta.signum()) {
        (StartupEyeFocus::DownCenter, -1) => {
            (StartupEyeFocus::DownLeft, StartupEyeFocus::DownCenter)
        }
        (StartupEyeFocus::DownCenter, 1) => {
            (StartupEyeFocus::DownCenter, StartupEyeFocus::DownRight)
        }
        (StartupEyeFocus::Center, -1) => (StartupEyeFocus::Left, StartupEyeFocus::Center),
        (StartupEyeFocus::Center, 1) => (StartupEyeFocus::Center, StartupEyeFocus::Right),
        (StartupEyeFocus::Left, 1) => (StartupEyeFocus::Center, StartupEyeFocus::Right),
        (StartupEyeFocus::Right, -1) => (StartupEyeFocus::Left, StartupEyeFocus::Center),
        (StartupEyeFocus::Up, -1) => (StartupEyeFocus::Left, StartupEyeFocus::Center),
        (StartupEyeFocus::Up, 1) => (StartupEyeFocus::Center, StartupEyeFocus::Right),
        _ => (focus, focus),
    };
    (
        startup_eye_focus_frame(left_focus),
        startup_eye_focus_frame(right_focus),
    )
}

fn startup_eye_focus_code(focus: StartupEyeFocus) -> u16 {
    match focus {
        StartupEyeFocus::Center => 0,
        StartupEyeFocus::Left => 1,
        StartupEyeFocus::Right => 2,
        StartupEyeFocus::Up => 3,
        StartupEyeFocus::DownCenter => 4,
        StartupEyeFocus::DownLeft => 5,
        StartupEyeFocus::DownRight => 6,
    }
}

fn startup_logo_eye_frame_index(elapsed: Duration) -> usize {
    if reduced_motion_enabled() {
        return 0;
    }
    if STARTUP_EYE_FRAMES.is_empty() {
        return 0;
    }

    let sequence_step = elapsed.as_millis() as u64 / STARTUP_LOGO_EYE_FRAME_MS.max(1);
    sequence_step as usize % STARTUP_EYE_FRAMES.len()
}

fn apply_startup_eye_patterns(line: &str, left_pattern: &str, right_pattern: &str) -> String {
    debug_assert_eq!(
        left_pattern.chars().count(),
        STARTUP_EYE_INTERIOR_WIDTH,
        "startup eye pattern width must remain fixed"
    );
    debug_assert_eq!(
        right_pattern.chars().count(),
        STARTUP_EYE_INTERIOR_WIDTH,
        "startup eye pattern width must remain fixed"
    );

    let mut characters = line.chars().collect::<Vec<_>>();
    let cavity = STARTUP_EYE_CAVITY.chars().collect::<Vec<_>>();
    let left_pattern = left_pattern.chars().collect::<Vec<_>>();
    let right_pattern = right_pattern.chars().collect::<Vec<_>>();
    let mut search_from = 0;

    for eye_index in 0..2 {
        let Some(cavity_start) = find_startup_eye_cavity(&characters, &cavity, search_from) else {
            break;
        };
        let interior_start = cavity_start + 4;
        let pattern = if eye_index == 0 {
            &left_pattern
        } else {
            &right_pattern
        };
        for (offset, character) in pattern.iter().copied().enumerate() {
            if let Some(slot) = characters.get_mut(interior_start + offset) {
                *slot = character;
            }
        }
        search_from = cavity_start + cavity.len();
    }

    characters.into_iter().collect()
}

fn apply_startup_eye_pattern(line: &str, pattern: &str) -> String {
    apply_startup_eye_patterns(line, pattern, pattern)
}

fn find_startup_eye_cavity(haystack: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() || from >= haystack.len() {
        return None;
    }

    haystack
        .get(from..)
        .and_then(|tail| {
            tail.windows(needle.len())
                .position(|window| window == needle)
        })
        .map(|offset| from + offset)
}

fn startup_logo_line_spans(line: String, base_line: String) -> Line<'static> {
    let logo_style = Style::default()
        .fg(SURFACE_ACCENT)
        .add_modifier(Modifier::BOLD);
    let mut spans = Vec::new();

    for (character, base_character) in line.chars().zip(base_line.chars()) {
        if character == ' ' {
            spans.push(Span::raw(" "));
        } else if base_character == ' ' && character != ' ' {
            spans.push(Span::styled(
                character.to_string(),
                startup_logo_eye_style(character),
            ));
        } else {
            spans.push(Span::styled(character.to_string(), logo_style));
        }
    }

    Line::from(spans)
}

fn startup_logo_eye_style(character: char) -> Style {
    let color = match character {
        '░' | '▁' | '▂' => SURFACE_DIM_GRAY,
        '▒' | '▃' | '▄' => SURFACE_GRAY,
        '▓' | '▅' | '▆' => SURFACE_ACCENT,
        '█' | '▇' => Color::White,
        _ => Color::White,
    };

    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

fn render_centered_startup_text_lines(text: &str, width: u16, style: Style) -> Vec<Line<'static>> {
    let content_width = width.saturating_sub(4).max(1) as usize;
    crate::presentation::render_wrapped_plain_display_line(text, content_width)
        .into_iter()
        .map(|line| {
            let centered = center_text_for_width(line.as_str(), width as usize);
            Line::from(vec![Span::styled(centered, style)])
        })
        .collect()
}

fn center_text_for_width(text: &str, width: usize) -> String {
    let text_width = crate::presentation::display_width(text);
    if text_width >= width {
        return text.to_owned();
    }
    let left_pad = (width - text_width) / 2;
    format!("{}{}", " ".repeat(left_pad), text)
}

fn pad_text_to_display_width(text: &str, width: usize) -> String {
    let text_width = crate::presentation::display_width(text);
    if text_width >= width {
        return text.to_owned();
    }
    format!("{}{}", text, " ".repeat(width - text_width))
}

#[derive(Debug, Clone)]
struct StartupTipRenderState {
    text: String,
    bullet_color: Color,
    text_color: Color,
    emphasize: bool,
}

impl StartupTipRenderState {
    fn steady(text: String) -> Self {
        Self {
            text,
            bullet_color: SURFACE_ACCENT,
            text_color: Color::White,
            emphasize: true,
        }
    }
}

fn render_startup_tip_lines(tip_state: &StartupTipRenderState, width: u16) -> Vec<Line<'static>> {
    let content_width = width.saturating_sub(6).max(1) as usize;
    crate::presentation::render_wrapped_plain_display_line(tip_state.text.as_str(), content_width)
        .into_iter()
        .map(|line| {
            let centered = center_text_for_width(line.as_str(), width as usize);
            let text_style = if tip_state.emphasize {
                Style::default()
                    .fg(tip_state.text_color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(tip_state.text_color)
            };
            let indent_width = centered.len().saturating_sub(centered.trim_start().len());
            let (indent, body) = centered.split_at(indent_width);
            if let Some(rest) = body.strip_prefix("• ") {
                Line::from(vec![
                    Span::raw(indent.to_owned()),
                    Span::styled("• ", Style::default().fg(tip_state.bullet_color)),
                    Span::styled(rest.to_owned(), text_style),
                ])
            } else {
                Line::from(vec![Span::styled(centered, text_style)])
            }
        })
        .collect()
}

fn startup_tip_render_state(tips: &[String], elapsed: Duration) -> Option<StartupTipRenderState> {
    let tip_count = tips.len();
    if reduced_motion_enabled() {
        return tips
            .first()
            .map(|tip| StartupTipRenderState::steady(format!("• {tip}")));
    }
    let tip_index = startup_tip_index(tip_count, elapsed)?;
    let (_, intensity_step) = startup_tip_cycle_state(tip_count, elapsed)?;
    let max_step = STARTUP_TIP_INTENSITY_STEPS.max(1);
    let tip_text = format!("• {}", tips.get(tip_index)?);
    let text_color =
        interpolate_rgb_color(SURFACE_DIM_GRAY, Color::White, intensity_step, max_step);
    let bullet_color =
        interpolate_rgb_color(SURFACE_GRAY, SURFACE_ACCENT, intensity_step, max_step);

    Some(StartupTipRenderState {
        text: tip_text,
        bullet_color,
        text_color,
        emphasize: intensity_step >= max_step.saturating_sub(1),
    })
}

fn startup_tip_index(tip_count: usize, elapsed: Duration) -> Option<usize> {
    let tip_count = tip_count.max(1) as u64;
    let cycle_ms = STARTUP_TIP_HOLD_MS
        .saturating_add(STARTUP_TIP_FADE_MS)
        .saturating_add(STARTUP_TIP_FADE_MS);
    let cycle_index = elapsed.as_millis() as u64 / cycle_ms.max(1);
    let cycle_phase = elapsed.as_millis() as u64 % cycle_ms.max(1);
    let current = cycle_index % tip_count;

    if cycle_phase < STARTUP_TIP_HOLD_MS.saturating_add(STARTUP_TIP_FADE_MS) {
        Some(current as usize)
    } else {
        Some(((current + 1) % tip_count) as usize)
    }
}

fn startup_tip_cycle_state(tip_count: usize, elapsed: Duration) -> Option<(usize, u64)> {
    if tip_count == 0 {
        return None;
    }

    let cycle_ms = STARTUP_TIP_HOLD_MS
        .saturating_add(STARTUP_TIP_FADE_MS)
        .saturating_add(STARTUP_TIP_FADE_MS);
    let animation_ms = elapsed.as_millis() as u64 / STARTUP_TIP_FRAME_MS.max(1);
    let cycle_index = animation_ms.saturating_mul(STARTUP_TIP_FRAME_MS) / cycle_ms.max(1);
    let cycle_phase = animation_ms.saturating_mul(STARTUP_TIP_FRAME_MS) % cycle_ms.max(1);
    let current_index = (cycle_index % tip_count as u64) as usize;
    let intensity = if cycle_phase < STARTUP_TIP_HOLD_MS {
        STARTUP_TIP_INTENSITY_STEPS
    } else if cycle_phase < STARTUP_TIP_HOLD_MS.saturating_add(STARTUP_TIP_FADE_MS) {
        let fade_progress = cycle_phase.saturating_sub(STARTUP_TIP_HOLD_MS);
        STARTUP_TIP_INTENSITY_STEPS.saturating_sub(
            fade_progress.saturating_mul(STARTUP_TIP_INTENSITY_STEPS) / STARTUP_TIP_FADE_MS.max(1),
        )
    } else {
        let fade_progress = cycle_phase
            .saturating_sub(STARTUP_TIP_HOLD_MS)
            .saturating_sub(STARTUP_TIP_FADE_MS);
        fade_progress.saturating_mul(STARTUP_TIP_INTENSITY_STEPS) / STARTUP_TIP_FADE_MS.max(1)
    };

    Some((current_index, intensity.min(STARTUP_TIP_INTENSITY_STEPS)))
}

fn interpolate_rgb_color(from: Color, to: Color, numerator: u64, denominator: u64) -> Color {
    let (from_r, from_g, from_b) = rgb_channels(from);
    let (to_r, to_g, to_b) = rgb_channels(to);
    let denominator = denominator.max(1);

    let blend = |start: u8, end: u8| -> u8 {
        let start = start as i64;
        let end = end as i64;
        let delta = end - start;
        let step = start + delta * numerator as i64 / denominator as i64;
        step.clamp(0, 255) as u8
    };

    Color::Rgb(
        blend(from_r, to_r),
        blend(from_g, to_g),
        blend(from_b, to_b),
    )
}

fn rgb_channels(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Reset => (255, 255, 255),
        Color::Red => (255, 0, 0),
        Color::Green => (0, 128, 0),
        Color::Yellow => (255, 255, 0),
        Color::Blue => (0, 0, 255),
        Color::Magenta => (255, 0, 255),
        Color::Cyan => (0, 255, 255),
        Color::White => (255, 255, 255),
        Color::Black => (0, 0, 0),
        Color::Gray => (128, 128, 128),
        Color::DarkGray => (64, 64, 64),
        Color::LightRed => (255, 102, 102),
        Color::LightGreen => (144, 238, 144),
        Color::LightYellow => (255, 255, 153),
        Color::LightBlue => (173, 216, 230),
        Color::LightMagenta => (238, 130, 238),
        Color::LightCyan => (224, 255, 255),
        Color::Indexed(index) => (index, index, index),
    }
}

fn vertically_center_startup_lines(
    mut lines: Vec<Line<'static>>,
    available_height: u16,
) -> Vec<Line<'static>> {
    let top_padding = startup_top_padding(lines.len(), available_height);
    if top_padding == 0 {
        return lines;
    }

    let mut centered = Vec::with_capacity(lines.len() + top_padding);
    centered.extend((0..top_padding).map(|_| Line::from("")));
    centered.append(&mut lines);
    centered
}

fn startup_top_padding(line_count: usize, available_height: u16) -> usize {
    let available_height = available_height as usize;
    if line_count == 0 || line_count >= available_height {
        return 0;
    }

    ((available_height - line_count) / 2).max(2)
}

fn wrap_assistant_markdown_lines(lines: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    let lines = normalize_blank_lines(lines);
    let content_width = width.saturating_sub(2) as usize;
    let mut rendered = Vec::new();
    let mut paragraph_buffer = String::new();
    let mut in_code_block = false;

    let flush_paragraph =
        |rendered: &mut Vec<Line<'static>>, paragraph_buffer: &mut String, content_width: usize| {
            if paragraph_buffer.trim().is_empty() {
                paragraph_buffer.clear();
                return;
            }
            rendered.extend(render_assistant_plain_line(
                paragraph_buffer.as_str(),
                content_width,
                assistant_line_style(paragraph_buffer),
            ));
            paragraph_buffer.clear();
        };

    for line in lines {
        let plain = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        let trimmed = plain.trim_start();
        if plain.trim().is_empty() {
            flush_paragraph(&mut rendered, &mut paragraph_buffer, content_width);
            rendered.push(Line::from(""));
            continue;
        }

        if trimmed.starts_with("```") {
            flush_paragraph(&mut rendered, &mut paragraph_buffer, content_width);
            rendered.extend(render_assistant_plain_line(
                plain.as_str(),
                content_width,
                assistant_line_style(&plain),
            ));
            in_code_block = !in_code_block;
            continue;
        }

        if in_code_block {
            flush_paragraph(&mut rendered, &mut paragraph_buffer, content_width);
            rendered.extend(render_assistant_code_line(plain.as_str(), content_width));
            continue;
        }

        if let Some(split_bullets) = split_inline_bullet_runs(&plain) {
            flush_paragraph(&mut rendered, &mut paragraph_buffer, content_width);
            for bullet_line in split_bullets {
                rendered.extend(render_assistant_plain_line(
                    bullet_line.as_str(),
                    content_width,
                    assistant_line_style(&bullet_line),
                ));
            }
            continue;
        }

        if is_reflowable_assistant_line(&plain) {
            if !paragraph_buffer.is_empty() {
                paragraph_buffer.push_str(paragraph_joiner(&paragraph_buffer, &plain));
            }
            paragraph_buffer.push_str(plain.trim());
            continue;
        }

        flush_paragraph(&mut rendered, &mut paragraph_buffer, content_width);
        if is_assistant_table_line(plain.as_str()) {
            rendered.extend(render_assistant_table_line(plain.as_str(), content_width));
        } else {
            rendered.extend(render_assistant_plain_line(
                plain.as_str(),
                content_width,
                assistant_line_style(&plain),
            ));
        }
    }

    flush_paragraph(&mut rendered, &mut paragraph_buffer, content_width);

    rendered
}

fn is_assistant_table_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('┌')
        || trimmed.starts_with('├')
        || trimmed.starts_with('└')
        || trimmed.starts_with('│')
}

fn render_assistant_table_line(line: &str, content_width: usize) -> Vec<Line<'static>> {
    if crate::presentation::display_width(line) > content_width {
        return render_assistant_plain_line(line, content_width, assistant_line_style(line));
    }

    let border_style = Style::default().fg(SURFACE_DIM_GRAY);
    let cell_style = Style::default().fg(Color::White);
    let separator_line = line
        .trim()
        .chars()
        .all(|ch| ch.is_whitespace() || is_table_border_char(ch));
    let mut spans = vec![Span::raw("  ")];

    for ch in line.chars() {
        let style = if separator_line || is_table_border_char(ch) {
            border_style
        } else {
            cell_style
        };
        spans.push(Span::styled(ch.to_string(), style));
    }

    vec![Line::from(spans)]
}

fn is_table_border_char(ch: char) -> bool {
    matches!(
        ch,
        '┌' | '┬' | '┐' | '├' | '┼' | '┤' | '└' | '┴' | '┘' | '─' | '│'
    )
}

fn render_assistant_plain_line(
    line: &str,
    content_width: usize,
    style: Style,
) -> Vec<Line<'static>> {
    crate::presentation::render_wrapped_plain_display_line(line, content_width)
        .into_iter()
        .map(|wrapped| Line::from(vec![Span::raw("  "), Span::styled(wrapped, style)]))
        .collect()
}

fn render_assistant_code_line(line: &str, content_width: usize) -> Vec<Line<'static>> {
    let code_style = Style::default().fg(SURFACE_GREEN);
    let (gutter, code) = line
        .strip_prefix("  ")
        .map_or(("", line), |rest| ("  ", rest));
    let code_width = content_width
        .saturating_sub(crate::presentation::display_width(gutter))
        .max(1);

    crate::presentation::render_wrapped_plain_display_line(code, code_width)
        .into_iter()
        .map(|wrapped| {
            let mut spans = vec![Span::raw("  ")];
            if !gutter.is_empty() {
                spans.push(Span::styled(gutter.to_owned(), code_style));
            }
            spans.push(Span::styled(wrapped, code_style));
            Line::from(spans)
        })
        .collect()
}

fn split_inline_bullet_runs(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if trimmed.matches("• ").count() < 2 {
        return None;
    }

    let items = trimmed
        .split("• ")
        .filter_map(|segment| {
            let segment = segment.trim();
            (!segment.is_empty()).then(|| format!("• {segment}"))
        })
        .collect::<Vec<_>>();

    (items.len() >= 2).then_some(items)
}

fn normalize_blank_lines(mut lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    while lines.first().is_some_and(is_visual_blank_line) {
        lines.remove(0);
    }
    while lines.last().is_some_and(is_visual_blank_line) {
        lines.pop();
    }

    let mut normalized = Vec::new();
    let mut last_was_blank = false;
    for line in lines {
        let is_blank = is_visual_blank_line(&line);
        if is_blank && last_was_blank {
            continue;
        }
        last_was_blank = is_blank;
        normalized.push(line);
    }
    normalized
}

fn render_user_markdown_lines(lines: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    let mut plain_lines = lines
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    while plain_lines
        .first()
        .is_some_and(|line| line.trim().is_empty())
    {
        plain_lines.remove(0);
    }
    while plain_lines
        .last()
        .is_some_and(|line| line.trim().is_empty())
    {
        plain_lines.pop();
    }

    let content_width = width.saturating_sub(2) as usize;
    let mut rendered = Vec::new();
    for line in plain_lines {
        if line.trim().is_empty() {
            rendered.push(Line::from(vec![Span::raw("")]));
            continue;
        }
        for wrapped in
            crate::presentation::render_wrapped_plain_display_line(line.as_str(), content_width)
        {
            rendered.push(Line::from(vec![Span::styled(
                wrapped,
                Style::default().fg(ratatui::style::Color::White),
            )]));
        }
    }
    rendered
}

fn is_reflowable_assistant_line(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty()
        && !trimmed.starts_with('#')
        && !trimmed.starts_with("```")
        && !trimmed.starts_with('┌')
        && !trimmed.starts_with('├')
        && !trimmed.starts_with('└')
        && !trimmed.starts_with('│')
        && !trimmed.starts_with("┃")
        && !trimmed.starts_with('>')
        && !trimmed.starts_with("- ")
        && !trimmed.starts_with("* ")
        && !trimmed.starts_with("• ")
        && !trimmed.starts_with("[image]")
}

fn paragraph_joiner(current: &str, next: &str) -> &'static str {
    if contains_cjk(current) || contains_cjk(next) {
        ""
    } else {
        " "
    }
}

fn contains_cjk(text: &str) -> bool {
    text.chars().any(|ch| {
        ('\u{4E00}'..='\u{9FFF}').contains(&ch)
            || ('\u{3040}'..='\u{30FF}').contains(&ch)
            || ('\u{AC00}'..='\u{D7AF}').contains(&ch)
    })
}

fn assistant_line_style(line: &str) -> Style {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        Style::default()
            .fg(SURFACE_HEADING)
            .add_modifier(Modifier::BOLD)
    } else if trimmed.starts_with("```") {
        Style::default().fg(SURFACE_DIM_GRAY)
    } else if trimmed.starts_with('┌')
        || trimmed.starts_with('├')
        || trimmed.starts_with('└')
        || trimmed.starts_with('│')
        || trimmed.starts_with("┃")
        || trimmed.starts_with('>')
    {
        Style::default().fg(SURFACE_GRAY)
    } else if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("• ")
    {
        Style::default().fg(ratatui::style::Color::White)
    } else if trimmed.starts_with("[image]") {
        Style::default().fg(SURFACE_ACCENT)
    } else {
        Style::default().fg(ratatui::style::Color::White)
    }
}

fn user_block_line(mut line: Line<'static>) -> Line<'static> {
    line.spans.insert(0, Span::raw("  "));
    if line.spans.is_empty() {
        line.spans
            .push(Span::styled("", Style::default().bg(SURFACE_USER_MSG_BG)));
        return line;
    }

    for span in &mut line.spans {
        span.style = span.style.bg(SURFACE_USER_MSG_BG);
    }
    line
}

