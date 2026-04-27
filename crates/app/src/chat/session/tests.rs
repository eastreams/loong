use super::*;

fn sample_surface_state() -> SurfaceState {
    SurfaceState {
        startup_summary: Some(fallback_startup_summary("default")),
        active_provider_label: "OpenAI / gpt-5.4".to_owned(),
        session_title_override: None,
        last_approval: None,
        transcript: vec![SurfaceEntry {
            lines: vec![
                "you · prompt".to_owned(),
                "Summarize the repository.".to_owned(),
            ],
        }],
        composer: "hi".to_owned(),
        composer_cursor: 2,
        history: Vec::new(),
        history_index: None,
        scroll_offset: 0,
        sticky_bottom: true,
        selected_entry: Some(0),
        focus: SurfaceFocus::Composer,
        sidebar_visible: true,
        sidebar_tab: SidebarTab::Runtime,
        command_palette: None,
        overlay: None,
        live: LiveSurfaceModel::default(),
        footer_notice: "?: help · : command menu".to_owned(),
        pending_turn: false,
    }
}

fn sample_render_data() -> SurfaceRenderData {
    SurfaceRenderData {
        header_lines: vec![
            "LOONG  v0.1.2-alpha.1".to_owned(),
            "interactive chat".to_owned(),
        ],
        header_status_line:
            "session=default · provider=OpenAI / gpt-5.4 · acp:off · focus=composer".to_owned(),
        transcript_lines: vec![
            "▶ you · prompt".to_owned(),
            "Summarize the repository.".to_owned(),
            String::new(),
            "assistant · reply".to_owned(),
            "Repository mapped.".to_owned(),
        ],
        sidebar_visible: true,
        sidebar_tab: SidebarTab::Runtime,
        sidebar_lines: vec![
            "session: default".to_owned(),
            "config: ~/.loong/config.toml".to_owned(),
            "memory: ~/.loong/memory.sqlite3".to_owned(),
        ],
        composer_lines: vec![
            "draft · focus=composer".to_owned(),
            "hi▏".to_owned(),
            String::new(),
            "Enter send · ? help · : or / command menu".to_owned(),
        ],
        status_line: "?: help · : command menu · Esc clear · PgUp/PgDn transcript · Tab focus"
            .to_owned(),
    }
}

#[test]
fn sidebar_tab_cycles_forward_and_backward() {
    assert_eq!(SidebarTab::Session.next(), SidebarTab::Runtime);
    assert_eq!(SidebarTab::Runtime.next(), SidebarTab::Tools);
    assert_eq!(SidebarTab::Tools.next(), SidebarTab::Mission);
    assert_eq!(SidebarTab::Mission.next(), SidebarTab::Workers);
    assert_eq!(SidebarTab::Workers.next(), SidebarTab::Review);
    assert_eq!(SidebarTab::Review.next(), SidebarTab::Help);
    assert_eq!(SidebarTab::Help.next(), SidebarTab::Session);
    assert_eq!(SidebarTab::Session.previous(), SidebarTab::Help);
    assert_eq!(SidebarTab::Workers.previous(), SidebarTab::Mission);
    assert_eq!(SidebarTab::Review.previous(), SidebarTab::Workers);
    assert_eq!(SidebarTab::Help.previous(), SidebarTab::Review);
}

#[test]
fn clipped_display_line_adds_ellipsis_when_needed() {
    assert_eq!(clipped_display_line("abcdef", 4), "abc…");
    assert_eq!(clipped_display_line("abc", 4), "abc");
}

#[test]
fn composer_display_lines_wraps_and_limits_rows() {
    let lines = composer_display_lines("alpha beta gamma delta", 10, 2);
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("alpha"));
}

#[test]
fn command_palette_items_have_stable_default_selection() {
    let palette = CommandPaletteState::default();
    let items = CommandPaletteAction::items();
    assert_eq!(palette.selected, 0);
    assert_eq!(palette.query, "");
    assert_eq!(items[0].0, "/help");
    assert!(items.iter().any(|item| item.0 == "Jump to latest"));
}

#[test]
fn surface_focus_cycles_without_palette() {
    assert_eq!(
        SurfaceFocus::Composer.next(true, false),
        SurfaceFocus::Transcript
    );
    assert_eq!(
        SurfaceFocus::Transcript.next(true, false),
        SurfaceFocus::Sidebar
    );
    assert_eq!(
        SurfaceFocus::Sidebar.next(true, false),
        SurfaceFocus::Composer
    );
}

#[test]
fn slash_command_hint_surfaces_matches() {
    let hint = slash_command_hint("/hi").expect("hint");
    let mission_hint = slash_command_hint("/mi").expect("mission hint");
    let sessions_hint = slash_command_hint("/se").expect("sessions hint");

    assert!(hint.contains("/history"));
    assert!(mission_hint.contains("/mission"));
    assert!(sessions_hint.contains("/sessions"));
    assert!(slash_command_hint("hello").is_none());
}

#[test]
fn should_continue_multiline_detects_trailing_backslash() {
    assert!(should_continue_multiline("hello\\"));
    assert!(!should_continue_multiline("hello"));
}

#[test]
fn should_continue_multiline_at_cursor_requires_cursor_at_end() {
    assert!(should_continue_multiline_at_cursor("hello\\", 6));
    assert!(!should_continue_multiline_at_cursor("hello\\", 3));
    assert!(!should_continue_multiline_at_cursor("hello", 5));
}

#[test]
fn terminal_surface_allowed_requires_interactive_stdin_and_stdout() {
    assert!(terminal_surface_allowed(true, true));
    assert!(!terminal_surface_allowed(true, false));
    assert!(!terminal_surface_allowed(false, true));
    assert!(!terminal_surface_allowed(false, false));
}

#[test]
fn composer_text_with_cursor_inserts_marker() {
    assert_eq!(composer_text_with_cursor("abc", 1), "a▏bc");
    assert_eq!(composer_text_with_cursor("", 0), "▏");
}

#[test]
fn insert_and_remove_char_at_cursor_updates_cursor_position() {
    let mut value = "ac".to_owned();
    let mut cursor = 1;
    insert_char_at_cursor(&mut value, &mut cursor, 'b');
    assert_eq!(value, "abc");
    assert_eq!(cursor, 2);
    remove_char_before_cursor(&mut value, &mut cursor);
    assert_eq!(value, "ac");
    assert_eq!(cursor, 1);
}

#[test]
fn move_cursor_vertically_preserves_column_when_possible() {
    let value = "abc\ndefg\nxy";
    assert_eq!(move_cursor_vertically(value, 5, -1), 1);
    assert_eq!(move_cursor_vertically(value, 1, 1), 5);
    assert_eq!(move_cursor_vertically(value, 7, 1), 11);
}

#[test]
fn command_palette_items_include_jump_and_sticky_actions() {
    let labels = CommandPaletteAction::items()
        .iter()
        .map(|item| item.0)
        .collect::<Vec<_>>();

    assert!(labels.contains(&"Mission control"));
    assert!(labels.contains(&"Jump to latest"));
    assert!(labels.contains(&"Toggle sticky scroll"));
    assert!(labels.contains(&"Timeline"));
}

#[test]
fn filtered_command_palette_items_respects_query() {
    let filtered = filtered_command_palette_items("time");
    assert!(filtered.iter().any(|item| item.0 == "Timeline"));
    assert!(!filtered.iter().any(|item| item.0 == "/compact"));
}

#[test]
fn current_overlay_label_reports_overlay_kind() {
    let mut state = SurfaceState {
        startup_summary: None,
        active_provider_label: "provider / model".to_owned(),
        session_title_override: None,
        last_approval: None,
        transcript: Vec::new(),
        composer: String::new(),
        composer_cursor: 0,
        history: Vec::new(),
        history_index: None,
        scroll_offset: 0,
        sticky_bottom: true,
        selected_entry: None,
        focus: SurfaceFocus::Composer,
        sidebar_visible: true,
        sidebar_tab: SidebarTab::Session,
        command_palette: None,
        overlay: None,
        live: LiveSurfaceModel::default(),
        footer_notice: String::new(),
        pending_turn: false,
    };
    assert_eq!(current_overlay_label(&state), "none");
    state.overlay = Some(SurfaceOverlay::Welcome {
        screen: TuiScreenSpec {
            header_style: TuiHeaderStyle::Compact,
            subtitle: Some("interactive chat".to_owned()),
            title: Some("operator cockpit ready".to_owned()),
            progress_line: None,
            intro_lines: Vec::new(),
            sections: Vec::new(),
            choices: Vec::new(),
            footer_lines: Vec::new(),
        },
    });
    assert_eq!(current_overlay_label(&state), "welcome");
    state.overlay = Some(SurfaceOverlay::MissionControl {
        lines: vec!["scope: default".to_owned()],
    });
    assert_eq!(current_overlay_label(&state), "mission");
    state.overlay = Some(SurfaceOverlay::Timeline);
    assert_eq!(current_overlay_label(&state), "timeline");
    state.overlay = Some(SurfaceOverlay::Help);
    assert_eq!(current_overlay_label(&state), "help");
}

#[test]
fn align_scroll_offset_to_selected_entry_keeps_entry_visible() {
    let entries = vec![
        SurfaceEntry {
            lines: vec!["entry 1".to_owned()],
        },
        SurfaceEntry {
            lines: vec!["entry 2".to_owned(), "entry 2 detail".to_owned()],
        },
        SurfaceEntry {
            lines: vec!["entry 3".to_owned()],
        },
    ];
    let viewport_height = 2;
    let current_scroll_offset = 0;
    let aligned_offset =
        align_scroll_offset_to_selected_entry(&entries, 1, viewport_height, current_scroll_offset);

    assert_eq!(aligned_offset, 2);
}

#[test]
fn default_export_path_uses_loong_exports_directory() {
    let export_path = PathBuf::from(default_export_path("session:/bad"));
    let file_name = export_path
        .file_name()
        .and_then(|value| value.to_str())
        .expect("export file name");
    let parent_dir = export_path
        .parent()
        .and_then(|value| value.file_name())
        .and_then(|value| value.to_str())
        .expect("export parent directory");

    assert_eq!(parent_dir, "exports");
    assert_eq!(file_name, "loong-session__bad-transcript.txt");
}

#[test]
fn terminal_surface_restore_sequence_resets_terminal_modes_before_exit() {
    let sequence = terminal_surface_restore_sequence();

    assert!(
        sequence.contains(BRACKETED_PASTE_DISABLE),
        "restore sequence should disable bracketed paste: {sequence:?}"
    );
    assert!(
        sequence.contains(CURSOR_KEYS_NORMAL),
        "restore sequence should restore normal cursor key mode: {sequence:?}"
    );
    assert!(
        sequence.contains(KEYPAD_NORMAL),
        "restore sequence should restore normal keypad mode: {sequence:?}"
    );
    assert!(
        sequence.contains(ANSI_RESET),
        "restore sequence should reset terminal styling: {sequence:?}"
    );
    assert!(
        sequence.ends_with(ALT_SCREEN_EXIT),
        "restore sequence should leave the alternate screen last: {sequence:?}"
    );
}

#[test]
fn ensure_parent_directory_exists_ignores_relative_files_without_parent() {
    let path = Path::new("transcript.txt");
    let result = ensure_parent_directory_exists(path);

    assert!(result.is_ok());
}

#[test]
fn render_surface_to_string_draws_ratatui_panels() {
    let rendered = render_surface_to_string(
        &sample_surface_state(),
        &sample_render_data(),
        Rect::new(0, 0, 120, 32),
    );

    assert!(rendered.contains("loong / chat"), "{rendered}");
    assert!(rendered.contains("transcript"), "{rendered}");
    assert!(rendered.contains("control deck"), "{rendered}");
    assert!(rendered.contains("compose"), "{rendered}");
    assert!(rendered.contains("controls"), "{rendered}");
    assert!(rendered.contains("OpenAI / gpt-5.4"), "{rendered}");
}

#[test]
fn render_surface_to_string_renders_command_menu_overlay() {
    let mut state = sample_surface_state();
    state.command_palette = Some(CommandPaletteState {
        selected: 0,
        query: "help".to_owned(),
    });

    let rendered =
        render_surface_to_string(&state, &sample_render_data(), Rect::new(0, 0, 120, 32));

    assert!(rendered.contains("command menu"), "{rendered}");
    assert!(rendered.contains("/help"), "{rendered}");
}

#[test]
fn render_surface_to_string_renders_welcome_overlay() {
    let mut state = sample_surface_state();
    state.overlay = Some(SurfaceOverlay::Welcome {
        screen: TuiScreenSpec {
            header_style: TuiHeaderStyle::Compact,
            subtitle: Some("interactive chat".to_owned()),
            title: Some("operator cockpit ready".to_owned()),
            progress_line: None,
            intro_lines: vec!["Start with a first answer.".to_owned()],
            sections: Vec::new(),
            choices: Vec::new(),
            footer_lines: vec!["Type to begin.".to_owned()],
        },
    });

    let rendered =
        render_surface_to_string(&state, &sample_render_data(), Rect::new(0, 0, 120, 32));

    assert!(rendered.contains("welcome"), "{rendered}");
    assert!(rendered.contains("operator cockpit ready"), "{rendered}");
    assert!(
        rendered.contains("Start with a first answer."),
        "{rendered}"
    );
}

#[test]
fn render_surface_to_string_surfaces_review_tab_context() {
    let mut state = sample_surface_state();
    state.sidebar_tab = SidebarTab::Review;
    state.last_approval = Some(ApprovalSurfaceSummary {
        title: "tool approval".to_owned(),
        subtitle: Some("approval pending".to_owned()),
        request_items: vec!["tool: shell.exec".to_owned()],
        rationale_lines: vec!["Needs confirmation before continuing.".to_owned()],
        choice_lines: vec!["1: approve".to_owned(), "2: reject".to_owned()],
        footer_lines: vec!["Reply with 1 or 2".to_owned()],
    });
    let mut render_data = sample_render_data();
    render_data.sidebar_tab = SidebarTab::Review;
    render_data.sidebar_lines = vec![
        "approval: tool approval".to_owned(),
        "mode: approval pending".to_owned(),
        "request".to_owned(),
        "tool: shell.exec".to_owned(),
        "reason".to_owned(),
        "Needs confirmation before continuing.".to_owned(),
    ];

    let rendered = render_surface_to_string(&state, &render_data, Rect::new(0, 0, 120, 32));

    assert!(rendered.contains("approval: tool approval"), "{rendered}");
    assert!(rendered.contains("tool approval"), "{rendered}");
    assert!(rendered.contains("Needs confirmation"), "{rendered}");
}

#[test]
fn approval_queue_item_summary_formats_list_and_detail_lines() {
    let item = ApprovalQueueItemSummary {
        approval_request_id: "apr_123".to_owned(),
        status: "pending".to_owned(),
        tool_name: "exec".to_owned(),
        raw_tool_name: "shell.exec".to_owned(),
        request_summary: Some(
            "{\"command\":\"git\",\"timeout_ms\":3000,\"args_redacted\":1}".to_owned(),
        ),
        turn_id: "turn_9".to_owned(),
        requested_at: 42,
        reason: Some("governed tool requires approval".to_owned()),
        rule_id: Some("approval-visible".to_owned()),
        last_error: Some("still waiting".to_owned()),
    };

    let list_line = item.list_line();
    assert!(list_line.contains("apr_123"));
    assert!(
        list_line.contains("request={\"command\":\"git\",\"timeout_ms\":3000,\"args_redacted\":1}")
    );
    let detail = item.detail_lines().join("\n");
    assert!(detail.contains("approval_request_id=apr_123"));
    assert!(detail.contains("tool_name=exec"));
    assert!(detail.contains("raw_tool_name=shell.exec"));
    assert!(
        detail.contains(
            "request_summary={\"command\":\"git\",\"timeout_ms\":3000,\"args_redacted\":1}"
        )
    );
    assert!(detail.contains("rule_id=approval-visible"));
    assert!(detail.contains("last_error=still waiting"));
}

#[test]
fn worker_queue_item_summary_formats_list_and_detail_lines() {
    let item = WorkerQueueItemSummary {
        session_id: "child-1".to_owned(),
        label: "worker: lint".to_owned(),
        state: "running".to_owned(),
        kind: "delegate_child".to_owned(),
        parent_session_id: Some("root-session".to_owned()),
        turn_count: 3,
        updated_at: 77,
        last_error: Some("still working".to_owned()),
    };

    assert!(item.list_line().contains("worker: lint"));
    let detail = item.detail_lines().join("\n");
    assert!(detail.contains("session_id=child-1"));
    assert!(detail.contains("parent_session_id=root-session"));
    assert!(detail.contains("turn_count=3"));
    assert!(detail.contains("last_error=still working"));
}

#[test]
fn render_surface_to_string_surfaces_worker_tab_context() {
    let mut state = sample_surface_state();
    state.sidebar_tab = SidebarTab::Workers;
    let mut render_data = sample_render_data();
    render_data.sidebar_tab = SidebarTab::Workers;
    render_data.sidebar_lines = vec![
        "worker sessions: 1".to_owned(),
        "worker: lint state=running kind=delegate_child turns=3".to_owned(),
    ];

    let rendered = render_surface_to_string(&state, &render_data, Rect::new(0, 0, 120, 32));

    assert!(rendered.contains("worker sessions: 1"), "{rendered}");
    assert!(rendered.contains("worker: lint"), "{rendered}");
    assert!(rendered.contains("delegate_child"), "{rendered}");
}

#[test]
fn render_surface_to_string_surfaces_mission_overlay() {
    let mut state = sample_surface_state();
    state.overlay = Some(SurfaceOverlay::MissionControl {
        lines: vec![
            "scope: default".to_owned(),
            "lanes: sessions=2 · roots=1 · delegates=1 · approvals=1".to_owned(),
            "controls".to_owned(),
            "S sessions · W workers · R approval queue".to_owned(),
        ],
    });

    let rendered =
        render_surface_to_string(&state, &sample_render_data(), Rect::new(0, 0, 120, 32));

    assert!(rendered.contains("mission control"), "{rendered}");
    assert!(rendered.contains("lanes: sessions=2"), "{rendered}");
    assert!(rendered.contains("S sessions"), "{rendered}");
}
