use super::{
    App, Focus, LiveTranscriptState, StartupBootstrapCapture, StartupOnboardingAction,
    StartupOnboardingInteractionKind, StartupOnboardingStage, StartupOnboardingState,
    StartupPersonalizationPreset, StartupProviderOption, StartupSetupPathChoice,
    StartupSkillOption, persist_startup_personalization, startup_eye_animation_for_state,
};
use crate::chat::chat_surface::command_palette::{
    CommandAction, CommandPalette, SkillEntry, slash_command_specs,
};
use crate::chat::chat_surface::composer::Composer;
use crate::chat::chat_surface::i18n::{I18nService, Language};
use crate::chat::chat_surface::message_list::{MessageList, StartupEyeAnimation, StartupEyeFocus};
use crate::chat::chat_surface::utils::SURFACE_USER_MSG_BG;
use crate::chat::{
    CliChatOptions, CliSessionRequirement, initialize_cli_turn_runtime_with_loaded_config,
};
use crate::config::{LoongConfig, ProviderConfig, ProviderKind, ReasoningEffort};
use crate::test_support::{ScopedEnv, unique_temp_dir};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend, layout::Rect, style::Style};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

fn blank_app() -> App {
    App {
        message_list: MessageList::new(),
        composer: Composer::new(),
        command_palette: CommandPalette::new(Language::En, Vec::new()),
        focus: Focus::Composer,
        pending_turn: false,
        turn_start: None,
        live_transcript: Arc::new(StdMutex::new(LiveTranscriptState::default())),
        pending_task: None,
        pending_steers: Default::default(),
        pending_queue: Default::default(),
        composer_follow_up_intent: false,
        pending_first_turn_bootstrap_addendum: None,
        awaiting_first_turn_bootstrap_reply: false,
        live_render_width: Arc::new(AtomicUsize::new(1)),
        live_rerender: None,
        spinner_seed: 1,
        last_pending_signature: None,
        last_live_transcript_signature: None,
        pending_render_cache: None,
        inline_skill_popup_active: false,
        last_render_width: 0,
        last_render_height: 0,
        last_transcript_area: Rect::default(),
        last_composer_area: Rect::default(),
        last_palette_area: Rect::default(),
        startup_onboarding: None,
        startup_version: "v0.1.0".to_owned(),
        startup_mcp_count: 0,
        detected_skills: Vec::new(),
        cwd: "/tmp/example".to_owned(),
        model: "gpt-test".to_owned(),
        title: None,
        last_terminal_title: None,
        title_attention_required: false,
        title_pending_approval_count: 0,
        i18n: I18nService::new(Language::En),
    }
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

fn skill(name: &str) -> SkillEntry {
    SkillEntry {
        name: name.to_owned(),
        description: format!("{name} description"),
        search_terms: vec![name.to_owned()],
        category_tag: "[Skill]".to_owned(),
        source_alias: None,
    }
}

fn onboarding_state() -> StartupOnboardingState {
    StartupOnboardingState {
        stage: StartupOnboardingStage::Language,
        language_options: vec![
            Language::En,
            Language::ZhCn,
            Language::ZhTw,
            Language::Ja,
            Language::Ru,
        ],
        language_index: 0,
        provider_options: vec![StartupProviderOption {
            kind: ProviderKind::Openai,
            auth_env_name: Some("OPENAI_API_KEY".to_owned()),
            is_current: true,
            label: "OpenAI".to_owned(),
            detail: "reuse the current config".to_owned(),
            recommended: true,
        }],
        provider_index: 0,
        skill_options: vec![StartupSkillOption {
            install_id: "agent-browser".to_owned(),
            display_name: "Agent Browser".to_owned(),
            summary: "browser automation".to_owned(),
            recommended: true,
        }],
        selected_skill_ids: BTreeSet::new(),
        skill_cursor: 0,
        setup_path_index: 0,
        personalization_index: 0,
        selected_personalization: None,
        web_search_provider_label: "DuckDuckGo".to_owned(),
        web_search_provider_detail: "web search still needs auth".to_owned(),
        provider_auth_env_name: None,
        provider_configuration_hint: None,
        enabled_channel_labels: Vec::new(),
        channel_follow_up_commands: Vec::new(),
        channel_status_commands: Vec::new(),
        channel_repair_commands: Vec::new(),
        startup_mcp_count: 0,
        detected_skill_count: 1,
        feedback: Some("demo feedback".to_owned()),
        last_interaction_at: std::time::Instant::now() - Duration::from_secs(5),
        last_interaction_kind: StartupOnboardingInteractionKind::Passive,
    }
}

#[test]
fn startup_eye_animation_tracks_active_onboarding_focus() {
    let mut state = onboarding_state();

    assert_eq!(
        startup_eye_animation_for_state(Some(&state)),
        StartupEyeAnimation::Focus(StartupEyeFocus::DownLeft)
    );

    state.stage = StartupOnboardingStage::Provider;
    state.provider_options = vec![
        StartupProviderOption {
            kind: ProviderKind::Openai,
            auth_env_name: None,
            is_current: true,
            label: "first".to_owned(),
            detail: "first".to_owned(),
            recommended: true,
        },
        StartupProviderOption {
            kind: ProviderKind::Anthropic,
            auth_env_name: None,
            is_current: false,
            label: "middle".to_owned(),
            detail: "middle".to_owned(),
            recommended: false,
        },
        StartupProviderOption {
            kind: ProviderKind::Gemini,
            auth_env_name: None,
            is_current: false,
            label: "last".to_owned(),
            detail: "last".to_owned(),
            recommended: false,
        },
    ];
    state.provider_index = 1;
    assert_eq!(
        startup_eye_animation_for_state(Some(&state)),
        StartupEyeAnimation::Focus(StartupEyeFocus::DownCenter)
    );

    state.stage = StartupOnboardingStage::SetupPath;
    state.setup_path_index = StartupSetupPathChoice::ProviderAndWeb as usize;
    assert_eq!(
        startup_eye_animation_for_state(Some(&state)),
        StartupEyeAnimation::Thinking(StartupEyeFocus::Right)
    );

    state.stage = StartupOnboardingStage::Skills;
    state.selected_skill_ids.insert("agent-browser".to_owned());
    assert_eq!(
        startup_eye_animation_for_state(Some(&state)),
        StartupEyeAnimation::Thinking(StartupEyeFocus::DownCenter)
    );

    state.stage = StartupOnboardingStage::Finish;
    assert_eq!(
        startup_eye_animation_for_state(Some(&state)),
        StartupEyeAnimation::Celebrate
    );
}

fn test_runtime_with_path(path: PathBuf) -> crate::chat::CliTurnRuntime {
    let mut config = LoongConfig::default();
    config.audit.path = unique_temp_dir("loong-chat-surface-audit")
        .join("audit")
        .join("events.jsonl")
        .display()
        .to_string();

    initialize_cli_turn_runtime_with_loaded_config(
        path,
        config,
        Some("chat-surface-test"),
        &CliChatOptions::default(),
        "chat-surface-test",
        CliSessionRequirement::RequireExplicit,
        false,
    )
    .expect("chat surface runtime")
}

#[test]
fn resize_reflow_tracks_width_and_height_changes() {
    assert!(super::resize_reflow_required(80, 24, 72, 24));
    assert!(super::resize_reflow_required(80, 24, 80, 32));
    assert!(!super::resize_reflow_required(80, 24, 80, 24));
}

#[test]
fn resize_live_rerender_waits_for_quiet_window() {
    assert!(!super::resize_live_rerender_ready(false, None));
    assert!(super::resize_live_rerender_ready(true, None));
    assert!(!super::resize_live_rerender_ready(
        true,
        Some(Duration::from_millis(32))
    ));
    assert!(super::resize_live_rerender_ready(
        true,
        Some(Duration::from_millis(70))
    ));
}

#[test]
fn pending_tool_animation_frames_cycle_between_dim_and_bright_states() {
    let early = super::pending_tool_animation_frame_for_elapsed(Duration::from_millis(0));
    let bright = super::pending_tool_animation_frame_for_elapsed(Duration::from_millis(360));

    assert_ne!(early, bright);
    assert_eq!(
        super::PENDING_TOOL_LABEL_COLORS[early],
        super::SURFACE_DIM_GRAY
    );
    assert_eq!(
        super::PENDING_TOOL_LABEL_COLORS[bright],
        super::Color::White
    );
}

fn sample_release() -> super::GithubRelease {
    super::GithubRelease {
            tag_name: "v9.9.9".to_owned(),
            published_at: Some("2026-04-20T00:00:00Z".to_owned()),
            html_url: Some("https://github.com/eastreams/loong/releases/tag/v9.9.9".to_owned()),
            body: Some(
                "- Added a very long changelog line that should wrap cleanly inside narrow startup surfaces without overflowing the transcript width.".to_owned(),
            ),
        }
}

fn buffer_lines(terminal: &Terminal<TestBackend>) -> Vec<String> {
    let buf = terminal.backend().buffer();
    let area = buf.area;
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect()
}

fn find_row(terminal: &Terminal<TestBackend>, needle: &str) -> Option<u16> {
    let buf = terminal.backend().buffer();
    let area = buf.area;
    for y in 0..area.height {
        let line = (0..area.width)
            .map(|x| buf[(x, y)].symbol())
            .collect::<String>();
        if line.contains(needle) {
            return Some(y);
        }
    }
    None
}

fn row_has_background(
    terminal: &Terminal<TestBackend>,
    row: u16,
    bg: ratatui::style::Color,
) -> bool {
    let buf = terminal.backend().buffer();
    let area = buf.area;
    (0..area.width).all(|x| buf[(x, row)].bg == bg)
}

#[test]
fn status_footer_truncates_long_cwd_from_the_left() {
    let cwd = std::env::current_dir()
        .expect("current dir")
        .join("nested")
        .join("session-tail-for-footer-test");
    let cwd = cwd.to_string_lossy();
    let line = super::build_status_footer_line(cwd.as_ref(), "gpt-5.4", 32);
    let rendered = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert_eq!(crate::presentation::display_width(&rendered), 32);
    assert!(rendered.contains("gpt-5.4"));
    assert!(rendered.contains("…"));
    assert!(rendered.contains("footer-test"));
    assert_eq!(rendered.chars().next(), cwd.chars().next());
}

#[test]
fn status_footer_truncates_model_when_width_is_extremely_narrow() {
    let line = super::build_status_footer_line("/tmp/project", "gpt-5.4-super-long-model-name", 12);
    let rendered = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert_eq!(crate::presentation::display_width(&rendered), 12);
    assert!(rendered.contains("…"));
}

#[test]
fn status_footer_respects_display_width_for_cjk_paths() {
    let line = super::build_status_footer_line("/tmp/项目/聊天记录", "gpt-5.4", 16);
    let rendered = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert_eq!(crate::presentation::display_width(&rendered), 16);
    assert!(rendered.contains("gpt-5.4"));
}

#[test]
fn middle_truncation_preserves_both_path_ends() {
    let path = std::env::current_dir()
        .expect("current dir")
        .join("worktrees")
        .join("project-name")
        .join("session");
    let path = path.to_string_lossy();
    let truncated = super::truncate_middle_for_width(path.as_ref(), 20);

    assert_eq!(truncated.chars().next(), path.chars().next());
    assert!(truncated.ends_with("session"));
    assert_eq!(crate::presentation::display_width(&truncated), 20);
}

#[test]
fn compact_path_label_prefers_last_path_component() {
    assert_eq!(
        super::compact_path_label("/tmp/workspace/project-x"),
        "project-x"
    );
    assert_eq!(super::compact_path_label("/"), "/");
    assert_eq!(super::compact_path_label(""), "~");
}

#[test]
fn build_loong_terminal_title_switches_prefix_by_activity() {
    assert_eq!(
        super::build_loong_terminal_title(
            "/tmp/workspace/project-x",
            super::LoongTerminalActivity::Idle,
            None
        ),
        "🐉 - project-x"
    );
    let working = super::build_loong_terminal_title(
        "/tmp/workspace/project-x",
        super::LoongTerminalActivity::Working,
        Some(std::time::Instant::now()),
    );
    assert!(working.ends_with(" - project-x"));
    let prefix = working.split(" - ").next().expect("title prefix");
    assert!(super::TERMINAL_TITLE_BRAILLE_FRAMES.contains(&prefix));
}

#[test]
fn terminal_title_braille_frame_uses_known_frames() {
    let frame = super::terminal_title_braille_frame(Some(std::time::Instant::now()));
    assert!(super::TERMINAL_TITLE_BRAILLE_FRAMES.contains(&frame));
}

#[test]
fn terminal_title_activity_requires_attention_for_bootstrap_reply() {
    let mut app = blank_app();
    app.awaiting_first_turn_bootstrap_reply = true;

    assert_eq!(
        super::app_terminal_title_activity(&app),
        super::LoongTerminalActivity::AttentionRequired
    );
}

#[test]
fn terminal_title_activity_requires_attention_for_approval_latch() {
    let mut app = blank_app();
    app.title_attention_required = true;

    assert_eq!(
        super::app_terminal_title_activity(&app),
        super::LoongTerminalActivity::AttentionRequired
    );
}

#[test]
fn terminal_title_activity_requires_attention_for_pending_approval_count() {
    let mut app = blank_app();
    app.title_pending_approval_count = 2;

    assert_eq!(
        super::app_terminal_title_activity(&app),
        super::LoongTerminalActivity::AttentionRequired
    );
}

#[test]
fn terminal_title_activity_requires_attention_for_live_needs_approval() {
    let app = blank_app();
    if let Ok(mut live) = app.live_transcript.lock() {
        live.tool_activity_lines =
            vec!["[needs_approval] shell.exec - operator confirmation required".to_owned()];
    }

    assert_eq!(
        super::app_terminal_title_activity(&app),
        super::LoongTerminalActivity::AttentionRequired
    );
}

#[test]
fn refresh_app_cwd_uses_runtime_working_directory() {
    let config_path = PathBuf::from("/tmp/loong-terminal-title-cwd.toml");
    let mut runtime = test_runtime_with_path(config_path);
    runtime.effective_working_directory = Some(PathBuf::from("/tmp/workspace/actual-project"));
    let mut app = blank_app();
    app.cwd = "/tmp/example".to_owned();

    super::refresh_app_cwd(&mut app, &runtime);

    assert_eq!(app.cwd, "/tmp/workspace/actual-project");
}

#[test]
fn resolve_cwd_change_path_supports_relative_paths_from_runtime_cwd() {
    let base = unique_temp_dir("loong-chat-surface-cwd-change");
    let nested = base.join("nested");
    std::fs::create_dir_all(&nested).expect("create nested cwd");
    let config_path = PathBuf::from("/tmp/loong-terminal-title-cwd-change.toml");
    let mut runtime = test_runtime_with_path(config_path);
    runtime.effective_working_directory = Some(base);

    let resolved = super::resolve_cwd_change_path(&runtime, "nested").expect("resolve cwd");

    assert_eq!(
        resolved,
        dunce::canonicalize(nested).expect("canonical nested")
    );
}

#[test]
fn cwd_command_updates_runtime_and_app_cwd() {
    let base = unique_temp_dir("loong-chat-surface-cwd-command");
    let nested = base.join("nested");
    std::fs::create_dir_all(&nested).expect("create nested cwd");
    let config_path = PathBuf::from("/tmp/loong-terminal-title-cwd-command.toml");
    let mut runtime = test_runtime_with_path(config_path);
    runtime.effective_working_directory = Some(base.clone());
    let mut app = blank_app();
    app.cwd = base.display().to_string();
    let backend = TestBackend::new(72, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");

    tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(super::run_surface_command(
            &mut terminal,
            &mut app,
            &mut runtime,
            &CliChatOptions::default(),
            "/cwd nested",
        ))
        .expect("run cwd command");

    assert_eq!(
        runtime.effective_working_directory,
        Some(dunce::canonicalize(&nested).expect("canonical nested"))
    );
    assert_eq!(
        PathBuf::from(&app.cwd),
        dunce::canonicalize(&nested).expect("canonical nested")
    );
}

#[test]
fn refresh_app_cwd_dependent_state_reloads_skills_from_new_cwd() {
    let base = unique_temp_dir("loong-chat-surface-cwd-skills-base");
    let nested = base.join("nested");
    std::fs::create_dir_all(nested.join("skills/demo-skill")).expect("create skills");
    std::fs::write(
        nested.join("skills/demo-skill/SKILL.md"),
        "---\nname: demo-skill\ndescription: nested skill\n---\n",
    )
    .expect("write skill");
    let config_path = PathBuf::from("/tmp/loong-terminal-title-cwd-skills.toml");
    let mut runtime = test_runtime_with_path(config_path);
    runtime.effective_working_directory = Some(nested.clone());
    let mut app = blank_app();
    app.detected_skills.clear();
    app.command_palette = CommandPalette::new(Language::En, Vec::new());

    super::refresh_app_cwd_dependent_state(&mut app, &runtime);

    assert_eq!(
        dunce::canonicalize(PathBuf::from(&app.cwd)).expect("canonical app cwd"),
        dunce::canonicalize(&nested).expect("canonical nested")
    );
    assert!(
        app.detected_skills
            .iter()
            .any(|skill| skill.name == "demo-skill")
    );
}

#[test]
fn refresh_app_cwd_dependent_state_preserves_skill_query() {
    let base = unique_temp_dir("loong-chat-surface-cwd-skill-query");
    let nested = base.join("nested");
    std::fs::create_dir_all(nested.join("skills/demo-skill")).expect("create skills");
    std::fs::write(
        nested.join("skills/demo-skill/SKILL.md"),
        "---\nname: demo-skill\ndescription: nested skill\n---\n",
    )
    .expect("write skill");
    let config_path = PathBuf::from("/tmp/loong-terminal-title-cwd-query.toml");
    let mut runtime = test_runtime_with_path(config_path);
    runtime.effective_working_directory = Some(nested);
    let mut app = blank_app();
    app.command_palette = CommandPalette::new(Language::En, Vec::new());
    app.command_palette.show_skills("demo");

    super::refresh_app_cwd_dependent_state(&mut app, &runtime);

    assert!(app.command_palette.is_skills_mode());
    assert_eq!(app.command_palette.query_text(), "demo");
    assert!(
        app.detected_skills
            .iter()
            .any(|skill| skill.name == "demo-skill")
    );
}

#[test]
fn render_cwd_command_uses_runtime_working_directory_fallback() {
    let runtime = test_runtime_with_path(PathBuf::from("/tmp/loong-config.toml"));

    let lines = super::render_cwd_command_lines_with_width(&runtime, 80);
    let rendered = lines.join("\n");
    assert!(rendered.contains("cwd"));
    assert!(!rendered.contains(runtime.resolved_path.display().to_string().as_str()));
}

#[test]
fn sanitize_terminal_title_collapses_whitespace_and_controls() {
    let sanitized = super::sanitize_terminal_title("  🐉 \n\t loong \u{202E} project  ");
    assert_eq!(sanitized, "🐉 loong project");
}

#[test]
fn sanitize_terminal_title_truncates_to_max_chars() {
    let title = "a".repeat(super::MAX_TERMINAL_TITLE_CHARS + 24);
    let sanitized = super::sanitize_terminal_title(&title);
    assert_eq!(sanitized.chars().count(), super::MAX_TERMINAL_TITLE_CHARS);
}

#[test]
fn startup_release_lines_wrap_to_requested_width() {
    let release = sample_release();
    let lines = super::format_startup_release_lines(&release, "v0.1.0", 80).expect("release lines");
    let mut list = MessageList::new();
    list.add_rendered_lines(lines);

    let rendered = list
        .get_rendered_lines(24)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(
        rendered
            .iter()
            .all(|line| line.is_empty() || crate::presentation::display_width(line) <= 24)
    );
    assert!(rendered.iter().any(|line| line.contains("What's New")));
    assert!(rendered.iter().any(|line| line.contains("Release:")));
}

#[test]
fn startup_release_lines_skip_current_version() {
    let release = sample_release();

    assert!(super::format_startup_release_lines(&release, "v9.9.9", 24).is_none());
}

#[test]
fn startup_version_line_is_product_only() {
    let version = super::startup_version_line();

    assert_eq!(version, format!("v{}", env!("CARGO_PKG_VERSION")));
    assert!(!version.contains(" · "));
}

#[test]
fn queue_footer_truncates_to_available_width() {
    let line = super::build_queue_footer_line(&I18nService::new(Language::En), 12, 14);
    let rendered = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert_eq!(crate::presentation::display_width(&rendered), 14);
    assert!(rendered.contains("queued ×12"));
}

#[test]
fn queue_footer_prefers_short_hint_before_truncating() {
    let line = super::build_queue_footer_line(&I18nService::new(Language::En), 2, 20);
    let rendered = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(rendered.contains("Tab to queue"));
    assert!(!rendered.contains("Tab to queue message"));
}

#[test]
fn restore_footer_truncates_to_available_width() {
    let line = super::build_restore_footer_line(&I18nService::new(Language::En), 12, 14);
    let rendered = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert_eq!(crate::presentation::display_width(&rendered), 14);
    assert!(rendered.contains("restore ×12"));
}

#[test]
fn restore_footer_prefers_short_hint_before_truncating() {
    let line = super::build_restore_footer_line(&I18nService::new(Language::En), 2, 32);
    let rendered = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(rendered.contains("restore queued"));
    assert!(!rendered.contains("to restore queued message"));
}

#[test]
fn footer_tracks_content_when_transcript_is_short() {
    let backend = TestBackend::new(50, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_user_message("hi".to_owned());
    app.message_list.add_assistant_message("hello".to_owned());

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal);
    let footer_row = lines
        .iter()
        .position(|line| line.contains("/tmp/example"))
        .expect("footer row");

    assert!(footer_row < lines.len().saturating_sub(1));
}

#[test]
fn wrapped_composer_expands_before_footer() {
    let backend = TestBackend::new(16, 12);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.composer.set_input("abcdefg".to_owned());

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal);
    let footer_row = lines
        .iter()
        .position(|line| line.contains("gpt-test"))
        .expect("footer row");
    let wrapped_row = lines
        .iter()
        .enumerate()
        .find_map(|(idx, line)| line.contains("defg").then_some(idx))
        .expect("wrapped composer row");

    assert!(footer_row > wrapped_row);
}

#[test]
fn footer_keeps_one_breathing_row_when_transcript_fills_available_height() {
    let backend = TestBackend::new(50, 12);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    for idx in 0..8 {
        app.message_list.add_user_message(format!("msg-{idx}"));
        app.message_list
            .add_assistant_message(format!("reply-{idx}"));
    }

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal);
    let footer_row = lines
        .iter()
        .position(|line| line.contains("/tmp/example"))
        .expect("footer row");

    assert_eq!(
        footer_row,
        lines
            .len()
            .saturating_sub(super::FOOTER_BOTTOM_BREATHING_HEIGHT as usize + 1)
    );
    assert!(lines.last().is_some_and(|line| line.trim().is_empty()));
}

#[test]
fn footer_content_uses_left_indent_when_space_allows() {
    let backend = TestBackend::new(50, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_user_message("hi".to_owned());
    app.message_list.add_assistant_message("hello".to_owned());

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal);
    let footer_line = lines
        .iter()
        .find(|line| line.contains("/tmp/example"))
        .expect("footer line");

    assert!(footer_line.starts_with("  /tmp/example"));
}

#[test]
fn pending_band_hides_plain_live_reply_lines() {
    let backend = TestBackend::new(50, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_user_message("hi".to_owned());
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    if let Ok(mut live) = app.live_transcript.lock() {
        live.draft_preview = Some("streamed reply line".to_owned());
    }

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal);
    let transcript_row = lines
        .iter()
        .position(|line| line.contains("streamed reply line"))
        .expect("provisional transcript row");
    let composer_row = lines
        .iter()
        .position(|line| line.contains("›"))
        .expect("composer row");
    assert!(transcript_row < composer_row);
}

#[test]
fn composer_and_footer_only_reclaim_pending_preview_rows_after_turn_finishes() {
    let backend = TestBackend::new(60, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_user_message("hi".to_owned());
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    if let Ok(mut live) = app.live_transcript.lock() {
        live.draft_preview = Some("streamed reply line".to_owned());
    }

    terminal.draw(|f| app.render(f)).expect("draw pending");
    let pending_lines = buffer_lines(&terminal);
    let pending_composer_row = pending_lines
        .iter()
        .position(|line| line.contains("›"))
        .expect("pending composer row");
    let pending_footer_row = pending_lines
        .iter()
        .position(|line| line.contains("/tmp/example"))
        .expect("pending footer row");

    app.pending_turn = false;
    app.turn_start = None;
    if let Ok(mut live) = app.live_transcript.lock() {
        *live = LiveTranscriptState::default();
    }
    app.message_list
        .add_assistant_message("streamed reply line".to_owned());

    terminal.draw(|f| app.render(f)).expect("draw complete");
    let settled_lines = buffer_lines(&terminal);
    let settled_composer_row = settled_lines
        .iter()
        .position(|line| line.contains("›"))
        .expect("settled composer row");
    let settled_footer_row = settled_lines
        .iter()
        .position(|line| line.contains("/tmp/example"))
        .expect("settled footer row");

    let composer_reclaimed_rows = pending_composer_row.saturating_sub(settled_composer_row);
    let footer_reclaimed_rows = pending_footer_row.saturating_sub(settled_footer_row);

    assert!(
        composer_reclaimed_rows <= 2,
        "composer should only reclaim the pending preview rows, got pending={pending_composer_row} settled={settled_composer_row}"
    );
    assert!(
        footer_reclaimed_rows <= 2,
        "footer should only reclaim the pending preview rows, got pending={pending_footer_row} settled={settled_footer_row}"
    );
}

#[test]
fn spinner_stays_adjacent_to_composer_when_plain_live_reply_is_hidden() {
    let backend = TestBackend::new(60, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_user_message("hi".to_owned());
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    if let Ok(mut live) = app.live_transcript.lock() {
        live.draft_preview = Some("streamed reply line".to_owned());
    }

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal);
    let spinner_row = lines
        .iter()
        .position(|line| line.contains("..."))
        .expect("spinner row");
    let composer_row = lines
        .iter()
        .position(|line| line.contains("›"))
        .expect("composer row");
    let preview_row = lines
        .iter()
        .position(|line| line.contains("streamed reply line"))
        .expect("preview row");

    assert!(composer_row > spinner_row);
    assert!(preview_row < composer_row);
}

#[test]
fn split_surface_command_preserves_arguments() {
    assert_eq!(
        super::split_surface_command("/copy explicit text"),
        ("/copy", "explicit text")
    );
    assert_eq!(super::split_surface_command("  /diff  "), ("/diff", ""));
}

#[test]
fn recognized_surface_command_only_accepts_known_builtins() {
    assert_eq!(
        super::recognized_surface_command("/model gpt-5"),
        Some("/model gpt-5".to_owned())
    );
    assert_eq!(
        super::recognized_surface_command(":settings provider"),
        Some("/settings provider".to_owned())
    );
    assert_eq!(super::recognized_surface_command("/workers"), None);
    assert_eq!(
        super::recognized_surface_command("/fast_lane_summary"),
        None
    );
    assert_eq!(
        super::recognized_surface_command("/safe_lane_summary"),
        None
    );
    assert_eq!(
        super::recognized_surface_command("/turn_checkpoint_summary"),
        None
    );
    assert_eq!(
        super::recognized_surface_command("/turn_checkpoint_repair"),
        None
    );
    assert_eq!(super::recognized_surface_command("/unknown note"), None);
    assert_eq!(super::recognized_surface_command(":unknown note"), None);
    assert_eq!(super::recognized_surface_command("plain text"), None);
}

#[test]
fn staging_commands_populate_composer_drafts() {
    let mut app = blank_app();
    app.message_list
        .add_assistant_message("existing answer".to_owned());

    super::stage_simplify_prompt(&mut app, "").expect("simplify stage");
    assert!(app.composer.text().contains("existing answer"));
    assert!(app.composer.text().contains("simplify"));

    super::stage_plan_prompt(&mut app, "the rollout").expect("plan stage");
    assert!(app.composer.text().contains("the rollout"));
}

#[test]
fn export_filename_components_are_safe() {
    assert_eq!(super::safe_file_component("abc-DEF_123"), "abc-DEF_123");
    assert_eq!(super::safe_file_component("a/b:c"), "a-b-c");
}

#[test]
fn help_lines_match_chat_surface_controls() {
    let rendered = super::render_chat_surface_help_lines_with_width(80).join("\n");

    assert!(rendered.contains("Shift+Enter inserts a new line"));
    assert!(rendered.contains("Use / or : from an empty composer"));
    assert!(rendered.contains("Type $skill-name directly in the composer"));
    assert!(rendered.contains("printable keys return"));
    assert!(rendered.contains("Native terminal drag-selection remains available"));
    assert!(!rendered.contains("coming soon"));
    assert!(!rendered.contains("A trailing \\\\ keeps composing"));
    assert!(!rendered.contains("control deck"));
    assert!(!rendered.contains("Esc from an empty composer"));
}

#[test]
fn slash_usage_and_detail_cards_are_enabled_without_placeholder_copy() {
    let usage = super::render_slash_command_usage_lines_with_width(90).join("\n");
    assert!(usage.contains("Every command stays visible"));
    assert!(!usage.contains("coming soon"));
    assert!(!usage.contains("placeholder"));
    assert!(!usage.contains("not wired"));

    let share_spec = slash_command_specs()
        .iter()
        .find(|spec| spec.command == "/share")
        .expect("/share spec");
    let detail = super::render_slash_command_detail_lines_with_width(share_spec, 90).join("\n");
    assert!(detail.contains("enabled"));
    assert!(detail.contains("/share is available"));
    assert!(detail.contains("write a local transcript artifact"));
    assert!(!detail.contains("coming soon"));
    assert!(!detail.contains("placeholder"));
    assert!(!detail.contains("not wired"));
}

#[test]
fn permissions_command_keeps_yolo_default_copy_simple() {
    let rendered = super::render_permissions_command_lines_with_width(80).join("\n");

    assert!(rendered.contains("YOLO by default"));
    assert!(rendered.contains("Hey yo, you only live once, take care."));
    assert!(rendered.contains("commands"));
    assert!(rendered.contains("enabled"));
    assert!(rendered.contains("not part of the happy path"));
    assert!(!rendered.contains("current policy"));
    assert!(!rendered.contains("shell allow"));
    assert!(!rendered.contains("shell deny"));
    assert!(!rendered.contains("file root"));
}

#[test]
fn experimental_command_reports_enabled_surface_features() {
    let rendered = super::render_experimental_command_lines_with_width(80).join("\n");

    assert!(rendered.contains("streaming renderer"));
    assert!(rendered.contains("startup animation"));
    assert!(rendered.contains("resize smoothing"));
    assert!(rendered.contains("enabled"));
    assert!(!rendered.contains("disabled"));
    assert!(!rendered.contains("toggles remain config-driven"));
}

#[test]
fn typing_dollar_keeps_focus_in_composer_while_inline_skill_popup_filters() {
    let mut app = blank_app();
    app.command_palette = CommandPalette::new(
        Language::En,
        vec![skill("demo-skill"), skill("other-skill")],
    );

    assert!(
        app.composer
            .handle_key(crossterm::event::KeyEvent::new(
                KeyCode::Char('$'),
                KeyModifiers::NONE,
            ))
            .is_none()
    );
    app.sync_inline_skill_popup();
    assert_eq!(app.focus, Focus::Composer);
    assert!(app.inline_skill_popup_active);
    assert_eq!(app.composer.text(), "$");

    assert!(
        app.composer
            .handle_key(crossterm::event::KeyEvent::new(
                KeyCode::Char('d'),
                KeyModifiers::NONE,
            ))
            .is_none()
    );
    app.sync_inline_skill_popup();

    if let Some(action) = app
        .command_palette
        .handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        ))
    {
        let _ = app.apply_palette_action(action);
    }
    assert_eq!(app.composer.text(), "$demo-skill ");
    assert_eq!(app.focus, Focus::Composer);
}

#[test]
fn typing_dollar_without_available_skills_keeps_plain_text_without_popup() {
    let mut app = blank_app();

    assert!(
        app.composer
            .handle_key(crossterm::event::KeyEvent::new(
                KeyCode::Char('$'),
                KeyModifiers::NONE,
            ))
            .is_none()
    );
    app.sync_inline_skill_popup();

    assert_eq!(app.focus, Focus::Composer);
    assert!(!app.inline_skill_popup_active);
    assert_eq!(app.composer.text(), "$");
}

#[test]
fn confirming_inline_skill_popup_with_no_matches_closes_popup_and_keeps_text() {
    let mut app = blank_app();
    app.command_palette = CommandPalette::new(Language::En, vec![skill("demo-skill")]);
    app.composer.set_input("$zzz".to_owned());
    app.sync_inline_skill_popup();

    assert!(app.inline_skill_popup_active);

    app.confirm_inline_skill_popup();

    assert_eq!(app.composer.text(), "$zzz");
    assert_eq!(app.focus, Focus::Composer);
    assert!(!app.inline_skill_popup_active);
}

#[test]
fn read_skill_metadata_prefers_frontmatter_name_and_description() {
    let skill = super::read_skill_metadata(
        "folder-fallback".to_owned(),
        std::path::PathBuf::from("/tmp/nonexistent")
            .with_file_name("skill.md")
            .with_extension("tmp"),
        "[Repo]",
        "repo",
    );
    assert_eq!(skill.name, "folder-fallback");
    assert_eq!(skill.description, "available skill");
    assert_eq!(skill.category_tag, "[Repo]");

    let contents = r#"---
name: actual-skill
description: "actual description"
---

# Skill
"#;
    let dir = std::env::temp_dir().join(format!(
        "loong-chat-skill-meta-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("SKILL.md");
    std::fs::write(&path, contents).expect("write");

    let skill =
        super::read_skill_metadata("folder-fallback".to_owned(), path.clone(), "[Repo]", "repo");
    assert_eq!(skill.name, "actual-skill");
    assert_eq!(skill.description, "actual description");
    assert!(skill.search_terms.iter().any(|term| term == "repo"));

    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn detect_available_skills_reads_skill_metadata_from_workspace() {
    let root = std::env::temp_dir().join(format!(
        "loong-chat-skills-root-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let skills_dir = root.join("skills");
    std::fs::create_dir_all(&skills_dir).expect("mkdir skills");

    let alpha_dir = skills_dir.join("alpha");
    std::fs::create_dir_all(&alpha_dir).expect("mkdir alpha");
    std::fs::write(
        alpha_dir.join("SKILL.md"),
        "---\nname: alpha-skill\ndescription: alpha description\n---\n",
    )
    .expect("write alpha");

    let beta_dir = skills_dir.join("beta");
    std::fs::create_dir_all(&beta_dir).expect("mkdir beta");
    std::fs::write(
        beta_dir.join("SKILL.md"),
        "# Beta\nbeta fallback description\n",
    )
    .expect("write beta");

    let skills = super::detect_available_skills(Some(root.as_path()));

    let alpha = skills
        .iter()
        .find(|skill| skill.name == "alpha-skill")
        .expect("alpha skill");
    assert_eq!(alpha.description, "alpha description");
    assert_eq!(alpha.category_tag, "[Repo]");
    assert!(alpha.search_terms.iter().any(|term| term == "alpha"));

    let beta = skills
        .iter()
        .find(|skill| skill.name == "beta")
        .expect("beta skill");
    assert_eq!(beta.description, "beta fallback description");
    assert_eq!(beta.category_tag, "[Repo]");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn build_skill_search_terms_includes_folder_name_and_source_segments() {
    let terms = super::build_skill_search_terms("babysit-pr", "PR Babysitter", "repo");

    assert!(terms.iter().any(|term| term == "babysit-pr"));
    assert!(terms.iter().any(|term| term == "babysit"));
    assert!(terms.iter().any(|term| term == "pr"));
    assert!(terms.iter().any(|term| term == "PR Babysitter"));
    assert!(terms.iter().any(|term| term == "Babysitter"));
    assert!(terms.iter().any(|term| term == "repo"));
}

#[test]
fn confirming_inline_skill_popup_keeps_focus_in_composer() {
    let mut app = blank_app();
    app.command_palette = CommandPalette::new(Language::En, vec![skill("demo-skill")]);
    app.composer.set_input("$dem".to_owned());
    app.sync_inline_skill_popup();

    assert!(app.inline_skill_popup_active);

    app.confirm_inline_skill_popup();

    assert_eq!(app.composer.text(), "$demo-skill ");
    assert_eq!(app.focus, Focus::Composer);
    assert!(!app.inline_skill_popup_active);
}

#[test]
fn tab_confirms_inline_skill_popup_through_shared_key_handler() {
    let mut app = blank_app();
    app.command_palette = CommandPalette::new(Language::En, vec![skill("demo-skill")]);
    app.composer.set_input("$dem".to_owned());
    app.sync_inline_skill_popup();

    assert!(
        app.handle_inline_skill_popup_key(crossterm::event::KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::NONE,
        ))
    );

    assert_eq!(app.composer.text(), "$demo-skill ");
    assert_eq!(app.focus, Focus::Composer);
    assert!(!app.inline_skill_popup_active);
}

#[test]
fn confirming_inline_skill_keeps_surrounding_text_stable() {
    let mut app = blank_app();
    app.command_palette = CommandPalette::new(Language::En, vec![skill("demo-skill")]);
    app.composer.set_input("please $dem now".to_owned());
    for _ in 0..4 {
        let _ = app.composer.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Left,
            KeyModifiers::NONE,
        ));
    }
    app.sync_inline_skill_popup();

    app.confirm_inline_skill_popup();

    assert_eq!(app.composer.text(), "please $demo-skill now");
}

#[test]
fn confirming_inline_skill_works_with_cursor_inside_token_middle() {
    let mut app = blank_app();
    app.command_palette = CommandPalette::new(Language::En, vec![skill("demo-skill")]);
    app.composer.set_input("$demo now".to_owned());
    for _ in 0..4 {
        let _ = app.composer.handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Left,
            KeyModifiers::NONE,
        ));
    }
    app.sync_inline_skill_popup();

    app.confirm_inline_skill_popup();

    assert_eq!(app.composer.text(), "$demo-skill now");
}

#[test]
fn inline_skill_popup_mouse_click_works_while_composer_keeps_focus() {
    let backend = TestBackend::new(50, 14);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.command_palette = CommandPalette::new(Language::En, vec![skill("demo-skill")]);
    app.composer.set_input("$dem".to_owned());
    app.sync_inline_skill_popup();

    terminal.draw(|f| app.render(f)).expect("draw");
    let palette_row = app.last_palette_area.y;
    let palette_col = app.last_palette_area.x.saturating_add(1);
    app.handle_mouse_event(mouse(
        MouseEventKind::Down(MouseButton::Left),
        palette_col,
        palette_row,
    ));

    assert_eq!(app.focus, Focus::Composer);
    assert_eq!(app.composer.text(), "$demo-skill ");
}

#[test]
fn inline_skill_popup_mouse_scroll_updates_selection_while_composer_stays_focused() {
    let backend = TestBackend::new(50, 14);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.command_palette = CommandPalette::new(
        Language::En,
        vec![skill("demo-skill"), skill("other-skill")],
    );
    app.composer.set_input("$".to_owned());
    app.sync_inline_skill_popup();

    terminal.draw(|f| app.render(f)).expect("draw");
    let palette_row = app.last_palette_area.y;
    let palette_col = app.last_palette_area.x.saturating_add(1);
    app.handle_mouse_event(mouse(MouseEventKind::ScrollDown, palette_col, palette_row));
    app.confirm_inline_skill_popup();

    assert_eq!(app.focus, Focus::Composer);
    assert_eq!(app.composer.text(), "$other-skill ");
}

#[test]
fn mouse_scroll_routes_to_transcript_even_with_a_draft() {
    let backend = TestBackend::new(40, 12);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    for idx in 0..14 {
        app.message_list
            .add_assistant_message(format!("line-{idx}"));
    }
    app.composer.set_input("draft".to_owned());

    terminal.draw(|f| app.render(f)).expect("draw");
    let before = buffer_lines(&terminal).join("\n");

    let transcript_row = app.last_transcript_area.y.saturating_add(1);
    let transcript_col = app.last_transcript_area.x.saturating_add(1);
    app.handle_mouse_event(mouse(
        MouseEventKind::ScrollUp,
        transcript_col,
        transcript_row,
    ));

    terminal.draw(|f| app.render(f)).expect("draw after scroll");
    let after = buffer_lines(&terminal).join("\n");

    assert!(app.message_list.scroll_offset_for_test() > 0);
    assert_ne!(before, after);
    assert_eq!(app.focus, Focus::Composer);
}

#[test]
fn footer_shows_follow_hint_when_transcript_is_off_tail() {
    let backend = TestBackend::new(50, 12);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    for idx in 0..14 {
        app.message_list
            .add_assistant_message(format!("line-{idx}"));
    }

    terminal.draw(|f| app.render(f)).expect("draw");
    app.message_list.handle_key(crossterm::event::KeyEvent::new(
        KeyCode::Up,
        KeyModifiers::NONE,
    ));
    terminal.draw(|f| app.render(f)).expect("draw off tail");
    let lines = buffer_lines(&terminal).join("\n");

    assert!(lines.contains("PgDn / End"));
    assert!(!lines.contains("/tmp/example"));
}

#[test]
fn footer_returns_to_status_line_when_tail_is_restored() {
    let backend = TestBackend::new(50, 12);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    for idx in 0..14 {
        app.message_list
            .add_assistant_message(format!("line-{idx}"));
    }

    terminal.draw(|f| app.render(f)).expect("draw");
    app.message_list.handle_key(crossterm::event::KeyEvent::new(
        KeyCode::Up,
        KeyModifiers::NONE,
    ));
    terminal.draw(|f| app.render(f)).expect("draw off tail");
    app.message_list.handle_key(crossterm::event::KeyEvent::new(
        KeyCode::End,
        KeyModifiers::NONE,
    ));
    terminal
        .draw(|f| app.render(f))
        .expect("draw tail restored");
    let lines = buffer_lines(&terminal).join("\n");

    assert!(lines.contains("/tmp/example"));
    assert!(!lines.contains("PgDn / End"));
}

#[test]
fn mouse_scroll_over_palette_changes_selection_without_scrolling_transcript() {
    let backend = TestBackend::new(50, 14);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    for idx in 0..10 {
        app.message_list
            .add_assistant_message(format!("line-{idx}"));
    }
    app.message_list.set_scroll_offset_for_test(4);
    app.command_palette.show_commands(":");
    app.focus = Focus::CommandPalette;

    terminal.draw(|f| app.render(f)).expect("draw");
    let palette_row = app.last_palette_area.y.saturating_add(1);
    let palette_col = app.last_palette_area.x.saturating_add(1);
    app.handle_mouse_event(mouse(MouseEventKind::ScrollDown, palette_col, palette_row));

    assert_eq!(app.message_list.scroll_offset_for_test(), 4);
    match app
        .command_palette
        .handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )) {
        Some(CommandAction::RunCommand("/permissions")) => {}
        other => {
            panic!("expected palette mouse scroll to land on /permissions, got {other:?}")
        }
    }
}

#[test]
fn slash_palette_open_and_sync_mirror_query_into_composer() {
    let mut app = blank_app();

    super::open_slash_command_palette(&mut app, '/', "");
    assert_eq!(app.focus, Focus::CommandPalette);
    assert_eq!(app.composer.text(), "/");

    let _ = app
        .command_palette
        .handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Char('m'),
            KeyModifiers::NONE,
        ));
    let _ = app
        .command_palette
        .handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Char('o'),
            KeyModifiers::NONE,
        ));
    super::sync_slash_palette_composer(&mut app);

    assert_eq!(app.composer.text(), "/mo");
}

#[test]
fn clearing_slash_palette_buffer_resets_composer() {
    let mut app = blank_app();
    super::open_slash_command_palette(&mut app, '/', "model");

    super::clear_slash_palette_composer(&mut app);

    assert!(app.composer.is_empty());
}

#[test]
fn model_palette_entries_open_reasoning_for_reasoning_capable_models() {
    let temp_root = std::env::temp_dir().join(format!(
        "loong-model-palette-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_root).expect("create temp root");
    let config_path = temp_root.join("config.toml");
    crate::config::write(
        Some(config_path.to_string_lossy().as_ref()),
        &LoongConfig::default(),
        true,
    )
    .expect("seed config");
    let runtime = test_runtime_with_path(config_path);
    let current_model = runtime.config.provider.model.clone();

    let entries = super::build_model_palette_entries(
        &runtime,
        &[crate::provider::ProviderModelCatalogEntry {
            model: current_model.clone(),
            display_name: None,
            description: None,
            is_default: false,
            hidden: false,
            deprecated: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            supported_reasoning_effort_descriptions: Vec::new(),
        }],
    );

    let entry = entries
        .iter()
        .find(|entry| entry.label == current_model)
        .expect("current model entry");
    assert_eq!(entry.status_tag.as_deref(), Some("current"));
    assert!(matches!(
        entry.action,
        CommandAction::OpenModelReasoning(ref entry) if entry.model == current_model
    ));
}

#[test]
fn reasoning_palette_entries_include_default_and_current_effort() {
    let temp_root = std::env::temp_dir().join(format!(
        "loong-reasoning-palette-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_root).expect("create temp root");
    let config_path = temp_root.join("config.toml");
    crate::config::write(
        Some(config_path.to_string_lossy().as_ref()),
        &LoongConfig::default(),
        true,
    )
    .expect("seed config");
    let mut runtime = test_runtime_with_path(config_path);
    runtime.config.provider.reasoning_effort = Some(ReasoningEffort::High);
    let current_model = runtime.config.provider.model.clone();

    let (entries, selected_label) = super::build_reasoning_palette_entries(
        &runtime,
        &crate::provider::ProviderModelCatalogEntry {
            model: current_model.clone(),
            display_name: None,
            description: None,
            is_default: false,
            hidden: false,
            deprecated: false,
            default_reasoning_effort: None,
            supported_reasoning_efforts: Vec::new(),
            supported_reasoning_effort_descriptions: Vec::new(),
        },
    );

    assert_eq!(
        entries.first().map(|entry| entry.label.as_str()),
        Some("default")
    );
    assert_eq!(selected_label, "high");
    let high_entry = entries
        .iter()
        .find(|entry| entry.label == "high")
        .expect("high entry");
    assert_eq!(high_entry.status_tag.as_deref(), Some("current"));
    assert!(matches!(
        high_entry.action,
        CommandAction::ApplyModelSelection {
            ref model,
            reasoning_effort: Some(ReasoningEffort::High)
        } if model == &current_model
    ));
}

#[test]
fn reasoning_palette_default_row_surfaces_known_model_default_effort() {
    let temp_root = std::env::temp_dir().join(format!(
        "loong-reasoning-default-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_root).expect("create temp root");
    let config_path = temp_root.join("config.toml");
    crate::config::write(
        Some(config_path.to_string_lossy().as_ref()),
        &LoongConfig::default(),
        true,
    )
    .expect("seed config");
    let mut runtime = test_runtime_with_path(config_path);
    runtime.config.provider.model = "gpt-5.4".to_owned();

    let (entries, selected_label) = super::build_reasoning_palette_entries(
        &runtime,
        &crate::provider::ProviderModelCatalogEntry {
            model: "gpt-5.4".to_owned(),
            display_name: Some("GPT-5.4".to_owned()),
            description: Some("Strong model for everyday coding.".to_owned()),
            is_default: false,
            hidden: false,
            deprecated: false,
            default_reasoning_effort: Some(ReasoningEffort::Xhigh),
            supported_reasoning_efforts: vec![
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::Xhigh,
            ],
            supported_reasoning_effort_descriptions: Vec::new(),
        },
    );

    assert_eq!(selected_label, "default");
    let default_entry = entries.first().expect("default entry");
    assert_eq!(default_entry.label, "default");
    assert!(default_entry.description.contains("xhigh"));
}

#[test]
fn reasoning_palette_default_row_prefers_catalog_default_effort_over_fallback() {
    let temp_root = std::env::temp_dir().join(format!(
        "loong-reasoning-catalog-default-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_root).expect("create temp root");
    let config_path = temp_root.join("config.toml");
    crate::config::write(
        Some(config_path.to_string_lossy().as_ref()),
        &LoongConfig::default(),
        true,
    )
    .expect("seed config");
    let runtime = test_runtime_with_path(config_path);

    let (entries, selected_label) = super::build_reasoning_palette_entries(
        &runtime,
        &crate::provider::ProviderModelCatalogEntry {
            model: "custom-model".to_owned(),
            display_name: Some("Custom Model".to_owned()),
            description: Some("Custom provider test model".to_owned()),
            is_default: false,
            hidden: false,
            deprecated: false,
            default_reasoning_effort: Some(ReasoningEffort::High),
            supported_reasoning_efforts: vec![ReasoningEffort::Low, ReasoningEffort::High],
            supported_reasoning_effort_descriptions: Vec::new(),
        },
    );

    assert_eq!(selected_label, "default");
    let default_entry = entries.first().expect("default entry");
    assert!(default_entry.description.contains("high"));
}

#[test]
fn reasoning_palette_uses_catalog_reasoning_option_descriptions_when_present() {
    let runtime = test_runtime_with_path(PathBuf::from(
        "/tmp/loong-reasoning-option-description.toml",
    ));

    let (entries, _) = super::build_reasoning_palette_entries(
        &runtime,
        &crate::provider::ProviderModelCatalogEntry {
            model: "gpt-5.5".to_owned(),
            display_name: Some("GPT-5.5".to_owned()),
            description: Some("Frontier model".to_owned()),
            is_default: true,
            hidden: false,
            deprecated: false,
            default_reasoning_effort: Some(ReasoningEffort::Medium),
            supported_reasoning_efforts: vec![ReasoningEffort::Low, ReasoningEffort::High],
            supported_reasoning_effort_descriptions: vec![
                (
                    ReasoningEffort::Low,
                    "Fast responses with lighter reasoning".to_owned(),
                ),
                (
                    ReasoningEffort::High,
                    "Greater reasoning depth for complex problems".to_owned(),
                ),
            ],
        },
    );

    let low = entries
        .iter()
        .find(|entry| entry.label == "low")
        .expect("low entry");
    assert_eq!(low.description, "Fast responses with lighter reasoning");
    let high = entries
        .iter()
        .find(|entry| entry.label == "high")
        .expect("high entry");
    assert_eq!(
        high.description,
        "Greater reasoning depth for complex problems"
    );
}

#[test]
fn apply_model_selection_updates_runtime_and_footer_model() {
    let temp_root = std::env::temp_dir().join(format!(
        "loong-model-apply-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_root).expect("create temp root");
    let config_path = temp_root.join("config.toml");
    crate::config::write(
        Some(config_path.to_string_lossy().as_ref()),
        &LoongConfig::default(),
        true,
    )
    .expect("seed config");
    let mut runtime = test_runtime_with_path(config_path);
    let mut app = blank_app();

    super::apply_model_selection(
        &mut app,
        &mut runtime,
        "gpt-5.4".to_owned(),
        Some(ReasoningEffort::Xhigh),
    )
    .expect("apply model selection");

    assert_eq!(runtime.config.provider.model, "gpt-5.4");
    assert_eq!(
        runtime.config.provider.reasoning_effort,
        Some(ReasoningEffort::Xhigh)
    );
    assert_eq!(app.model, "gpt-5.4");
    assert_eq!(app.focus, Focus::Composer);
}

#[test]
fn model_command_opens_selector_surface_instead_of_static_card() {
    let temp_root = std::env::temp_dir().join(format!(
        "loong-model-command-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_root).expect("create temp root");
    let config_path = temp_root.join("config.toml");
    crate::config::write(
        Some(config_path.to_string_lossy().as_ref()),
        &LoongConfig::default(),
        true,
    )
    .expect("seed config");
    let mut runtime = test_runtime_with_path(config_path);
    runtime
        .config
        .provider
        .preferred_models
        .push("gpt-5.4".to_owned());
    runtime.config.provider.models_endpoint = Some("http://127.0.0.1:9/models".to_owned());
    runtime.config.provider.models_endpoint_explicit = true;
    let mut app = blank_app();
    let backend = TestBackend::new(72, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");

    tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(super::run_surface_command(
            &mut terminal,
            &mut app,
            &mut runtime,
            &CliChatOptions::default(),
            "/model",
        ))
        .expect("run model command");

    assert_eq!(app.focus, Focus::CommandPalette);
    match app
        .command_palette
        .handle_key(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )) {
        Some(CommandAction::OpenModelReasoning(entry))
            if entry.model == runtime.config.provider.model => {}
        other => panic!("expected /model to open model selector flow, got {other:?}"),
    }
}

#[test]
fn exact_model_catalog_match_finds_model_and_display_name() {
    let catalog = vec![
        crate::provider::ProviderModelCatalogEntry {
            model: "gpt-5.4".to_owned(),
            display_name: Some("GPT-5.4".to_owned()),
            description: Some("Strong model for everyday coding.".to_owned()),
            is_default: false,
            hidden: false,
            deprecated: false,
            default_reasoning_effort: Some(ReasoningEffort::Xhigh),
            supported_reasoning_efforts: vec![
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::Xhigh,
            ],
            supported_reasoning_effort_descriptions: Vec::new(),
        },
        crate::provider::ProviderModelCatalogEntry {
            model: "command-r".to_owned(),
            display_name: Some("Command R".to_owned()),
            description: Some("Cohere model".to_owned()),
            is_default: false,
            hidden: false,
            deprecated: false,
            default_reasoning_effort: Some(ReasoningEffort::High),
            supported_reasoning_efforts: vec![ReasoningEffort::High],
            supported_reasoning_effort_descriptions: Vec::new(),
        },
        crate::provider::ProviderModelCatalogEntry {
            model: "hidden-model".to_owned(),
            display_name: Some("Hidden Model".to_owned()),
            description: Some("Not shown by default".to_owned()),
            is_default: false,
            hidden: true,
            deprecated: false,
            default_reasoning_effort: Some(ReasoningEffort::Low),
            supported_reasoning_efforts: vec![ReasoningEffort::Low],
            supported_reasoning_effort_descriptions: Vec::new(),
        },
    ];

    assert_eq!(
        super::find_exact_model_catalog_entry(catalog.as_slice(), "gpt-5.4")
            .map(|entry| entry.model.as_str()),
        Some("gpt-5.4")
    );
    assert_eq!(
        super::find_exact_model_catalog_entry(catalog.as_slice(), "Command R")
            .map(|entry| entry.model.as_str()),
        Some("command-r")
    );
    assert_eq!(
        super::find_exact_model_catalog_entry(catalog.as_slice(), "hidden-model")
            .map(|entry| entry.model.as_str()),
        Some("hidden-model")
    );
}

#[test]
fn model_palette_entries_use_direct_apply_for_single_reasoning_option() {
    let provider = ProviderConfig {
        kind: ProviderKind::Cohere,
        model: "command-r".to_owned(),
        ..ProviderConfig::fresh_for_kind(ProviderKind::Cohere)
    };
    let runtime = test_runtime_with_path(PathBuf::from("/tmp/loong-model-single-effort.toml"));
    let runtime = crate::chat::CliTurnRuntime {
        config: LoongConfig {
            provider,
            ..runtime.config
        },
        ..runtime
    };

    let entries = super::build_model_palette_entries(
        &runtime,
        &[crate::provider::ProviderModelCatalogEntry {
            model: "command-r".to_owned(),
            display_name: Some("Command R".to_owned()),
            description: Some("Cohere model".to_owned()),
            is_default: false,
            hidden: false,
            deprecated: false,
            default_reasoning_effort: Some(ReasoningEffort::High),
            supported_reasoning_efforts: vec![ReasoningEffort::High],
            supported_reasoning_effort_descriptions: Vec::new(),
        }],
    );

    let entry = entries.first().expect("single model entry");
    assert!(matches!(
        entry.action,
        CommandAction::ApplyModelSelection {
            ref model,
            reasoning_effort: Some(ReasoningEffort::High)
        } if model == "command-r"
    ));
}

#[test]
fn model_palette_prefers_display_name_label_and_keeps_raw_id_in_description() {
    let runtime = test_runtime_with_path(PathBuf::from("/tmp/loong-model-display-name.toml"));

    let entries = super::build_model_palette_entries(
        &runtime,
        &[crate::provider::ProviderModelCatalogEntry {
            model: "gpt-5.4".to_owned(),
            display_name: Some("GPT-5.4 Frontier".to_owned()),
            description: Some("Strong model for everyday coding.".to_owned()),
            is_default: false,
            hidden: false,
            deprecated: false,
            default_reasoning_effort: Some(ReasoningEffort::Xhigh),
            supported_reasoning_efforts: vec![
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High,
                ReasoningEffort::Xhigh,
            ],
            supported_reasoning_effort_descriptions: Vec::new(),
        }],
    );

    let entry = entries.first().expect("display-name entry");
    assert_eq!(entry.label, "GPT-5.4 Frontier");
    assert!(entry.description.contains("gpt-5.4"));
    assert!(
        entry
            .description
            .contains("Strong model for everyday coding.")
    );
}

#[test]
fn model_palette_sorts_current_before_other_entries() {
    let provider = ProviderConfig {
        kind: ProviderKind::Openai,
        model: "current-model".to_owned(),
        ..ProviderConfig::fresh_for_kind(ProviderKind::Openai)
    };
    let runtime = test_runtime_with_path(PathBuf::from("/tmp/loong-model-sort.toml"));
    let runtime = crate::chat::CliTurnRuntime {
        config: LoongConfig {
            provider,
            ..runtime.config
        },
        ..runtime
    };

    let entries = super::build_model_palette_entries(
        &runtime,
        &[
            crate::provider::ProviderModelCatalogEntry {
                model: "zeta-model".to_owned(),
                display_name: Some("Zeta Model".to_owned()),
                description: None,
                is_default: false,
                hidden: false,
                deprecated: false,
                default_reasoning_effort: None,
                supported_reasoning_efforts: vec![ReasoningEffort::Medium],
                supported_reasoning_effort_descriptions: Vec::new(),
            },
            crate::provider::ProviderModelCatalogEntry {
                model: "alpha-model".to_owned(),
                display_name: Some("Alpha Model".to_owned()),
                description: None,
                is_default: false,
                hidden: false,
                deprecated: false,
                default_reasoning_effort: None,
                supported_reasoning_efforts: vec![ReasoningEffort::Medium],
                supported_reasoning_effort_descriptions: Vec::new(),
            },
            crate::provider::ProviderModelCatalogEntry {
                model: "current-model".to_owned(),
                display_name: Some("Current Model".to_owned()),
                description: None,
                is_default: false,
                hidden: false,
                deprecated: false,
                default_reasoning_effort: None,
                supported_reasoning_efforts: vec![ReasoningEffort::Medium],
                supported_reasoning_effort_descriptions: Vec::new(),
            },
        ],
    );

    assert_eq!(entries[0].status_tag.as_deref(), Some("current"));
    assert_eq!(entries[0].label, "Current Model");
    assert_eq!(entries[1].label, "Alpha Model");
    assert_eq!(entries[2].label, "Zeta Model");
}

#[test]
fn merged_model_catalog_entries_hide_remote_hidden_and_deprecated_models_by_default() {
    let provider = ProviderConfig::fresh_for_kind(ProviderKind::Openai);

    let merged = super::merged_model_catalog_entries(
        &provider,
        &[
            crate::provider::ProviderModelCatalogEntry {
                model: "hidden-remote".to_owned(),
                display_name: Some("Hidden Remote".to_owned()),
                description: Some("hidden".to_owned()),
                is_default: false,
                hidden: true,
                deprecated: false,
                default_reasoning_effort: Some(ReasoningEffort::Medium),
                supported_reasoning_efforts: vec![ReasoningEffort::Medium],
                supported_reasoning_effort_descriptions: Vec::new(),
            },
            crate::provider::ProviderModelCatalogEntry {
                model: "deprecated-remote".to_owned(),
                display_name: Some("Deprecated Remote".to_owned()),
                description: Some("deprecated".to_owned()),
                is_default: false,
                hidden: false,
                deprecated: true,
                default_reasoning_effort: Some(ReasoningEffort::Low),
                supported_reasoning_efforts: vec![ReasoningEffort::Low],
                supported_reasoning_effort_descriptions: Vec::new(),
            },
        ],
        false,
    );

    assert!(!merged.iter().any(|entry| entry.model == "hidden-remote"));
    assert!(
        !merged
            .iter()
            .any(|entry| entry.model == "deprecated-remote")
    );
}

#[test]
fn merged_model_catalog_entries_keep_current_local_candidate_even_if_hidden() {
    let provider = ProviderConfig {
        kind: ProviderKind::Openai,
        model: "hidden-current".to_owned(),
        ..ProviderConfig::fresh_for_kind(ProviderKind::Openai)
    };

    let merged = super::merged_model_catalog_entries(
        &provider,
        &[crate::provider::ProviderModelCatalogEntry {
            model: "hidden-current".to_owned(),
            display_name: Some("Hidden Current".to_owned()),
            description: Some("still current".to_owned()),
            is_default: false,
            hidden: true,
            deprecated: false,
            default_reasoning_effort: Some(ReasoningEffort::Medium),
            supported_reasoning_efforts: vec![ReasoningEffort::Medium],
            supported_reasoning_effort_descriptions: Vec::new(),
        }],
        false,
    );

    let current = merged
        .iter()
        .find(|entry| entry.model == "hidden-current")
        .expect("current hidden entry");
    assert!(current.hidden);
}

#[test]
fn mouse_clicking_skill_palette_inserts_into_composer() {
    let backend = TestBackend::new(50, 14);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.command_palette = CommandPalette::new(Language::En, vec![skill("demo-skill")]);
    app.command_palette.show_skills("$demo");
    app.focus = Focus::CommandPalette;

    terminal.draw(|f| app.render(f)).expect("draw");
    let palette_row = app.last_palette_area.y;
    let palette_col = app.last_palette_area.x.saturating_add(1);
    app.handle_mouse_event(mouse(
        MouseEventKind::Down(MouseButton::Left),
        palette_col,
        palette_row,
    ));

    assert_eq!(app.focus, Focus::Composer);
    assert_eq!(app.composer.take_input(), "$demo-skill ");
}

#[test]
fn mouse_clicking_composer_restores_focus() {
    let backend = TestBackend::new(50, 14);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.focus = Focus::MessageList;

    terminal.draw(|f| app.render(f)).expect("draw");
    let composer_row = app.last_composer_area.y;
    let composer_col = app.last_composer_area.x.saturating_add(1);
    app.handle_mouse_event(mouse(
        MouseEventKind::Down(MouseButton::Left),
        composer_col,
        composer_row,
    ));

    assert_eq!(app.focus, Focus::Composer);
}

#[test]
fn transcript_click_closes_inline_skill_popup_after_focus_change() {
    let backend = TestBackend::new(50, 14);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.command_palette = CommandPalette::new(Language::En, vec![skill("demo-skill")]);
    app.composer.set_input("$dem".to_owned());
    app.sync_inline_skill_popup();
    app.message_list.add_assistant_message("line-0".to_owned());
    app.message_list.add_assistant_message("line-1".to_owned());

    terminal.draw(|f| app.render(f)).expect("draw");
    let transcript_row = app.last_transcript_area.y.saturating_add(1);
    let transcript_col = app.last_transcript_area.x.saturating_add(1);
    app.handle_mouse_event(mouse(
        MouseEventKind::Down(MouseButton::Left),
        transcript_col,
        transcript_row,
    ));

    assert_eq!(app.focus, Focus::MessageList);
    assert!(!app.inline_skill_popup_active);
}

#[test]
fn composer_click_reopens_inline_skill_popup_after_transcript_focus() {
    let backend = TestBackend::new(50, 14);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.command_palette = CommandPalette::new(Language::En, vec![skill("demo-skill")]);
    app.composer.set_input("$dem".to_owned());
    app.focus = Focus::MessageList;
    app.message_list.add_assistant_message("line-0".to_owned());
    app.sync_inline_skill_popup();

    terminal.draw(|f| app.render(f)).expect("draw");
    let composer_row = app.last_composer_area.y;
    let composer_col = app.last_composer_area.x.saturating_add(1);
    app.handle_mouse_event(mouse(
        MouseEventKind::Down(MouseButton::Left),
        composer_col,
        composer_row,
    ));

    assert_eq!(app.focus, Focus::Composer);
    assert!(app.inline_skill_popup_active);
}

#[test]
fn startup_tip_leaves_blank_row_before_composer_separator() {
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_startup_header_with_tips(
        "0.1.0".to_owned(),
        "fallback".to_owned(),
        vec![
            ("Skills".to_owned(), vec!["0".to_owned()]),
            ("MCP".to_owned(), vec!["1".to_owned()]),
        ],
        vec!["rotating tip".to_owned()],
    );

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal);
    let composer_separator_row = app.last_composer_area.y.saturating_sub(1) as usize;
    let blank_row_before_separator = composer_separator_row.saturating_sub(1);

    assert!(lines.iter().any(|line| line.contains("rotating tip")));
    assert!(
        lines
            .get(blank_row_before_separator)
            .is_some_and(|line| line.trim().is_empty())
    );
}

#[test]
fn startup_header_remains_visible_after_first_message() {
    let backend = TestBackend::new(70, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_startup_header(
        "0.1.0".to_owned(),
        "tutorial".to_owned(),
        vec![("MCP".to_owned(), vec!["0".to_owned()])],
    );
    app.message_list.add_user_message("hi".to_owned());
    app.message_list.add_assistant_message("hello".to_owned());

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal).join("\n");
    assert!(lines.contains("0.1.0"));
    assert!(lines.contains("MCP (0)"));
    assert!(lines.contains("hi"));
    assert!(lines.contains("hello"));
}

#[test]
fn startup_logo_keeps_animating_with_composer_draft_after_first_message() {
    let mut env = ScopedEnv::new();
    env.remove("LOONG_TUI_REDUCED_MOTION");
    env.set("TERM", "xterm-256color");

    let backend = TestBackend::new(100, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list
        .add_startup_header("0.1.0".to_owned(), "tutorial".to_owned(), Vec::new());
    app.message_list.add_user_message("hi".to_owned());
    app.message_list.add_assistant_message("hello".to_owned());
    app.composer.set_input("draft".to_owned());

    terminal.draw(|f| app.render(f)).expect("draw");
    let before_lines = buffer_lines(&terminal);
    let before_header = before_lines
        .iter()
        .take(8)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");

    app.message_list
        .rewind_startup_animation_for_test(Duration::from_millis(100));
    assert!(app.message_list.refresh_startup_animation());

    terminal.draw(|f| app.render(f)).expect("draw");
    let after_lines = buffer_lines(&terminal);
    let after_header = after_lines
        .iter()
        .take(8)
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let after = after_lines.join("\n");

    assert_ne!(
        before_header, after_header,
        "startup header should continue animating"
    );
    assert!(after.contains("draft"));
    assert!(after.contains("hello"));
}

#[test]
fn pending_band_keeps_blank_padding_rows() {
    let backend = TestBackend::new(50, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_user_message("hi".to_owned());
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal);
    let spinner_row = lines
        .iter()
        .position(|line| line.contains("..."))
        .expect("spinner row");
    assert!(spinner_row > 0);
    assert!(lines[spinner_row - 1].trim().is_empty());
}

#[test]
fn compact_pending_lines_drops_padding_before_content_on_tiny_height() {
    let lines = super::build_pending_lines(
        Some(std::time::Instant::now()),
        &["visible reply".to_owned()],
        1,
        &std::collections::VecDeque::new(),
        &std::collections::VecDeque::new(),
        40,
    );

    let compacted = super::compact_pending_lines_for_height(lines, 3);
    let rendered = compacted
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert_eq!(rendered.len(), 3);
    assert!(rendered.iter().any(|line| line.contains("visible reply")));
}

#[test]
fn pending_band_hides_plain_streaming_preview_text() {
    let backend = TestBackend::new(60, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_user_message("hi".to_owned());
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    if let Ok(mut live) = app.live_transcript.lock() {
        live.draft_preview = Some("first streamed sentence\nsecond streamed sentence".to_owned());
    }

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal);
    let first_row = lines
        .iter()
        .position(|line| line.contains("first streamed sentence"))
        .expect("first preview row");
    let second_row = lines
        .iter()
        .position(|line| line.contains("second streamed sentence"))
        .expect("second preview row");
    let composer_row = lines
        .iter()
        .position(|line| line.contains("›"))
        .expect("composer row");
    assert!(first_row < composer_row);
    assert!(second_row < composer_row);
    assert!(!lines.iter().any(|line| line.contains("╭─")));
    assert!(!lines.iter().any(|line| line.contains("turn pipeline")));
}

#[test]
fn pending_preview_styles_tool_activity_without_flattening_it_into_plain_text() {
    let lines = super::build_pending_lines(
        Some(std::time::Instant::now()),
        &[
            "• Called read_file · working".to_owned(),
            "  ↳ stderr 1 lines · 42 bytes".to_owned(),
            "    - denied".to_owned(),
        ],
        1,
        &std::collections::VecDeque::new(),
        &std::collections::VecDeque::new(),
        72,
    );

    let called_line = lines
        .iter()
        .find(|line| {
            line.spans
                .iter()
                .any(|span| span.content.as_ref() == "Called ")
        })
        .expect("called line");
    let called_label = called_line
        .spans
        .iter()
        .find(|span| span.content.as_ref() == "Called ")
        .expect("called label");
    assert!(
        super::PENDING_TOOL_LABEL_COLORS.contains(
            &called_label
                .style
                .fg
                .expect("called label should have an animated foreground"),
        )
    );
    assert!(
        called_label
            .style
            .add_modifier
            .contains(ratatui::style::Modifier::BOLD)
    );

    let stderr_line = lines
        .iter()
        .find(|line| {
            line.spans
                .iter()
                .any(|span| span.content.as_ref() == "stderr ")
        })
        .expect("stderr line");
    let stderr_label = stderr_line
        .spans
        .iter()
        .find(|span| span.content.as_ref() == "stderr ")
        .expect("stderr label");
    assert_eq!(stderr_label.style.fg, Some(super::SURFACE_RED));

    let sample_line = lines
        .iter()
        .find(|line| {
            line.spans
                .iter()
                .any(|span| span.content.as_ref().contains("- denied"))
        })
        .expect("sample line");
    let sample_span = sample_line
        .spans
        .iter()
        .find(|span| span.content.as_ref().contains("- denied"))
        .expect("sample span");
    assert_eq!(sample_span.style.fg, Some(super::SURFACE_RED));
}

#[test]
fn pending_live_generic_line_preserves_plain_label_like_text() {
    let rendered = super::render_pending_live_line(
        "source: imported config at ~/.loong/config.toml",
        24,
        Style::default(),
        std::time::Instant::now(),
    )
    .into_iter()
    .map(|line| {
        line.spans
            .into_iter()
            .map(|span| span.content.to_string())
            .collect::<String>()
    })
    .collect::<Vec<_>>();

    assert!(
        rendered
            .iter()
            .any(|line| line == "  source: imported config")
    );
    assert!(
        rendered
            .iter()
            .any(|line| line == "  at ~/.loong/config.toml")
    );
    assert!(
        !rendered
            .iter()
            .any(|line| line == "    at ~/.loong/config.toml")
    );
}

#[test]
fn pending_tool_activity_preserves_literal_plus_prefix() {
    let rendered = super::build_pending_lines(
        Some(std::time::Instant::now()),
        &[
            "• Called + added ~/.loong/config.toml".to_owned(),
            "  ↳ stderr + added ~/.loong/config.toml".to_owned(),
            "    + added ~/.loong/config.toml".to_owned(),
        ],
        1,
        &std::collections::VecDeque::new(),
        &std::collections::VecDeque::new(),
        48,
    )
    .into_iter()
    .map(|line| {
        line.spans
            .into_iter()
            .map(|span| span.content.to_string())
            .collect::<String>()
    })
    .collect::<Vec<_>>();

    assert!(
        rendered
            .iter()
            .any(|line| line.contains("• Called + added"))
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("↳ stderr + added"))
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("+ added ~/.loong/config.toml"))
    );
    assert!(
        !rendered
            .iter()
            .any(|line| line.contains("• Called - added"))
    );
    assert!(
        !rendered
            .iter()
            .any(|line| line.contains("↳ stderr - added"))
    );
}

#[test]
fn pending_queue_preview_preserves_literal_plus_prefix() {
    let mut pending_queue = std::collections::VecDeque::new();
    pending_queue.push_back("+ added ~/.loong/config.toml".to_owned());

    let rendered = super::build_pending_lines(
        Some(std::time::Instant::now()),
        &[],
        1,
        &std::collections::VecDeque::new(),
        &pending_queue,
        42,
    )
    .into_iter()
    .map(|line| {
        line.spans
            .into_iter()
            .map(|span| span.content.to_string())
            .collect::<String>()
    })
    .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line.contains("↳ + added")));
    assert!(!rendered.iter().any(|line| line.contains("↳ - added")));
}

#[test]
fn pending_preview_hides_plain_streaming_reply_between_transcript_and_composer() {
    let backend = TestBackend::new(60, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_user_message("hi".to_owned());
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    if let Ok(mut live) = app.live_transcript.lock() {
        live.draft_preview = Some("streamed reply line".to_owned());
    }

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal);
    let user_row = lines
        .iter()
        .position(|line| line.contains("hi"))
        .expect("user row");
    let composer_row = lines
        .iter()
        .position(|line| line.contains("›"))
        .expect("composer row");

    assert!(composer_row > user_row);
    let preview_row = lines
        .iter()
        .position(|line| line.contains("streamed reply line"))
        .expect("preview row");
    assert!(preview_row > user_row);
    assert!(preview_row < composer_row);
}

#[test]
fn pending_preview_hides_reasoning_and_visible_reply_text() {
    let backend = TestBackend::new(70, 12);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_user_message("hi".to_owned());
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    if let Ok(mut live) = app.live_transcript.lock() {
        live.draft_preview = Some("quiet reasoning\nvisible reply".to_owned());
    }

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal).join("\n");
    assert!(lines.contains("quiet reasoning"));
    assert!(lines.contains("visible reply"));
}

#[test]
fn pending_preview_keeps_plain_reply_with_tool_like_prefix_out_of_pending_band() {
    let backend = TestBackend::new(70, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_user_message("hi".to_owned());
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    if let Ok(mut live) = app.live_transcript.lock() {
        live.draft_preview = Some("• not a tool call\nrequest: still plain prose".to_owned());
    }

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal);
    let composer_row = lines
        .iter()
        .position(|line| line.contains("›"))
        .expect("composer row");
    let bullet_row = lines
        .iter()
        .position(|line| line.contains("• not a tool call"))
        .expect("bullet reply row");
    let request_row = lines
        .iter()
        .position(|line| line.contains("request: still plain prose"))
        .expect("request reply row");

    assert!(bullet_row < composer_row);
    assert!(request_row < composer_row);
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Called not a tool call")),
        "plain transcript preview should not be restyled as pending tool activity"
    );
}

#[test]
fn pending_preview_no_longer_reserves_blank_row_for_plain_reply_preview() {
    let backend = TestBackend::new(70, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_user_message("hi".to_owned());
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    if let Ok(mut live) = app.live_transcript.lock() {
        live.draft_preview = Some("visible reply".to_owned());
    }

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal);
    let spinner_row = lines
        .iter()
        .position(|line| line.contains("..."))
        .expect("spinner row");
    let composer_row = lines
        .iter()
        .position(|line| line.contains("›"))
        .expect("composer row");
    let preview_row = lines
        .iter()
        .position(|line| line.contains("visible reply"))
        .expect("preview row");

    assert!(composer_row > spinner_row);
    assert!(preview_row < composer_row);
}

#[test]
fn pending_preview_does_not_render_plain_reply_lines() {
    let backend = TestBackend::new(70, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_user_message("hi".to_owned());
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    if let Ok(mut live) = app.live_transcript.lock() {
        live.draft_preview = Some("visible reply".to_owned());
    }

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal).join("\n");
    assert!(lines.contains("visible reply"));
}
#[test]
fn pending_preview_does_not_wrap_hidden_plain_reply_lines() {
    let backend = TestBackend::new(28, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_user_message("hi".to_owned());
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    if let Ok(mut live) = app.live_transcript.lock() {
        live.draft_preview = Some("visible reply wraps across the pending band".to_owned());
    }

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal).join("\n");
    assert!(lines.contains("visible reply"));
    assert!(lines.contains("pending band"));
}

#[test]
fn pending_preview_no_longer_expands_plain_reply_text_with_extra_height() {
    let backend = TestBackend::new(18, 24);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_user_message("hi".to_owned());
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    if let Ok(mut live) = app.live_transcript.lock() {
        live.draft_preview = Some(
            "a1 a2 a3 a4 a5 a6 a7 a8 a9 a10 a11 a12 a13 a14 a15 a16 a17 a18 a19 a20 omega"
                .to_owned(),
        );
    }

    terminal.draw(|f| app.render(f)).expect("draw");
    let rendered = buffer_lines(&terminal).join("\n");

    assert!(rendered.contains("omega"));
}

#[test]
fn pending_preview_hides_reasoning_reply_separator_when_plain_text_is_hidden() {
    let backend = TestBackend::new(70, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_user_message("hi".to_owned());
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    if let Ok(mut live) = app.live_transcript.lock() {
        live.draft_preview = Some("quiet reasoning\n\nvisible reply".to_owned());
    }

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal).join("\n");
    assert!(lines.contains("quiet reasoning"));
    assert!(lines.contains("visible reply"));
}

#[test]
fn pending_preview_does_not_style_hidden_reasoning_lines() {
    let backend = TestBackend::new(70, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_user_message("hi".to_owned());
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    if let Ok(mut live) = app.live_transcript.lock() {
        live.draft_preview = Some("quiet reasoning\n\nvisible reply".to_owned());
    }

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal).join("\n");
    assert!(lines.contains("quiet reasoning"));
    assert!(lines.contains("visible reply"));
}

#[test]
fn pending_preview_truncation_ignores_hidden_plain_reply_segments() {
    let backend = TestBackend::new(70, 12);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_user_message("hi".to_owned());
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    if let Ok(mut live) = app.live_transcript.lock() {
        live.draft_preview =
            Some("reason-1\nreason-2\nreason-3\nreason-4\n\nreply-1\nreply-2\nreply-3".to_owned());
    }

    terminal.draw(|f| app.render(f)).expect("draw");
    let rendered = buffer_lines(&terminal).join("\n");

    assert!(rendered.contains("reason-1"));
    assert!(rendered.contains("reason-2"));
    assert!(rendered.contains("reply-1"));
    assert!(rendered.contains("reply-2"));
}

#[test]
fn pending_live_lines_trim_outer_blank_lines_and_collapse_repeats() {
    let lines = Arc::new(StdMutex::new(LiveTranscriptState {
        tool_activity_lines: vec![
            String::new(),
            String::new(),
            "reasoning".to_owned(),
            String::new(),
            String::new(),
            "reply".to_owned(),
            String::new(),
            String::new(),
        ],
        draft_preview: None,
    }));

    let normalized = super::pending_live_lines(&lines, 6);
    assert_eq!(
        normalized,
        vec!["reasoning".to_owned(), String::new(), "reply".to_owned(),]
    );
}

#[test]
fn pending_live_lines_expand_with_larger_preview_budget() {
    let lines = Arc::new(StdMutex::new(LiveTranscriptState {
        tool_activity_lines: vec![
            "reason-1".to_owned(),
            "reason-2".to_owned(),
            "reason-3".to_owned(),
            String::new(),
            "reply-1".to_owned(),
            "reply-2".to_owned(),
            "reply-3".to_owned(),
            "reply-4".to_owned(),
        ],
        draft_preview: None,
    }));

    let compact = super::pending_live_lines(&lines, 4);
    let expanded = super::pending_live_lines(&lines, 7);

    assert!(compact.len() < expanded.len());
    assert!(expanded.iter().any(|line| line.contains("reply-3")));
}

#[test]
fn pending_signature_preview_budget_tracks_last_render_geometry() {
    let mut app = blank_app();
    app.last_render_width = 40;
    app.last_render_height = 20;

    assert!(super::pending_signature_preview_budget(&app) > 1);

    app.last_render_height = 8;
    assert_eq!(super::pending_signature_preview_budget(&app), 1);
}

#[test]
fn transcript_navigation_key_helper_keeps_printable_keys_for_composer() {
    assert!(super::is_transcript_navigation_key(
        crossterm::event::KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE,)
    ));
    assert!(super::is_transcript_navigation_key(
        crossterm::event::KeyEvent::new(KeyCode::Home, KeyModifiers::NONE,)
    ));
    assert!(!super::is_transcript_navigation_key(
        crossterm::event::KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE,)
    ));
    assert!(!super::is_transcript_navigation_key(
        crossterm::event::KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE,)
    ));
}

#[test]
fn transcript_focus_text_keys_enter_composer_immediately() {
    let mut app = blank_app();
    app.focus = Focus::MessageList;

    let submitted = super::route_transcript_key_to_composer(
        &mut app,
        crossterm::event::KeyEvent::new(KeyCode::Char('你'), KeyModifiers::NONE),
    );

    assert!(submitted.is_none());
    assert_eq!(app.focus, Focus::Composer);
    assert_eq!(app.composer.text(), "你");
}

#[test]
fn paste_event_always_restores_composer_focus_and_inserts_text() {
    let mut app = blank_app();
    app.focus = Focus::MessageList;

    super::paste_into_composer(&mut app, "alpha\r\nbeta");

    assert_eq!(app.focus, Focus::Composer);
    assert_eq!(app.composer.text(), "alpha\nbeta");
    assert!(!app.composer_follow_up_intent);
}

#[test]
fn paste_event_marks_pending_draft_as_follow_up() {
    let mut app = blank_app();
    app.focus = Focus::CommandPalette;
    app.pending_turn = true;

    super::paste_into_composer(&mut app, "queued follow-up");

    assert_eq!(app.focus, Focus::Composer);
    assert_eq!(app.composer.text(), "queued follow-up");
    assert!(app.composer_follow_up_intent);
}

#[test]
fn transcript_focus_enter_submits_existing_draft() {
    let mut app = blank_app();
    app.focus = Focus::MessageList;
    app.composer.set_input("send me".to_owned());

    let submitted = super::route_transcript_key_to_composer(
        &mut app,
        crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    );

    assert_eq!(submitted.as_deref(), Some("send me"));
    assert_eq!(app.focus, Focus::Composer);
    assert!(app.composer.is_empty());
}

#[test]
fn transcript_focus_capture_helper_rejects_navigation_and_modified_keys() {
    assert!(super::should_focus_composer_for_transcript_key(
        crossterm::event::KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE,)
    ));
    assert!(super::should_focus_composer_for_transcript_key(
        crossterm::event::KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE,)
    ));
    assert!(super::should_focus_composer_for_transcript_key(
        crossterm::event::KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE,)
    ));
    assert!(!super::should_focus_composer_for_transcript_key(
        crossterm::event::KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE,)
    ));
    assert!(!super::should_focus_composer_for_transcript_key(
        crossterm::event::KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL,)
    ));
}

#[test]
fn composer_routes_arrow_and_page_scroll_even_with_a_draft() {
    let mut app = blank_app();
    app.composer.set_input("draft".to_owned());

    assert!(super::should_route_composer_key_to_transcript(
        &app,
        crossterm::event::KeyEvent::new(KeyCode::Up, KeyModifiers::NONE,)
    ));
    assert!(super::should_route_composer_key_to_transcript(
        &app,
        crossterm::event::KeyEvent::new(KeyCode::Down, KeyModifiers::NONE,)
    ));
    assert!(super::should_route_composer_key_to_transcript(
        &app,
        crossterm::event::KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE,)
    ));
    assert!(super::should_route_composer_key_to_transcript(
        &app,
        crossterm::event::KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE,)
    ));
    assert!(!super::should_route_composer_key_to_transcript(
        &app,
        crossterm::event::KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE,)
    ));
    assert!(!super::should_route_composer_key_to_transcript(
        &app,
        crossterm::event::KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE,)
    ));
}

#[test]
fn submitted_message_is_not_treated_as_follow_up_after_pending_turn_finishes() {
    let mut app = blank_app();
    app.composer_follow_up_intent = true;

    assert!(!super::submitted_message_is_follow_up(&app, "follow up"));

    app.pending_turn = true;
    assert!(super::submitted_message_is_follow_up(&app, "follow up"));
    assert!(!super::submitted_message_is_follow_up(&app, "/status"));
    assert!(!super::submitted_message_is_follow_up(&app, ":status"));
}

#[test]
fn pending_footer_yields_to_queue_hint_when_draft_exists() {
    let backend = TestBackend::new(60, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    app.composer.set_input("queued draft".to_owned());

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal).join("\n");

    assert!(lines.contains("Tab to queue message"));
    assert!(!lines.contains("/tmp/example"));
}

#[test]
fn pending_footer_shows_restore_hint_when_queue_exists() {
    let backend = TestBackend::new(60, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    app.pending_queue.push_back("queued draft".to_owned());

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal).join("\n");

    assert!(lines.contains("queued ×1"));
    assert!(lines.contains("Option + Up") || lines.contains("Alt + Up"));
    assert!(!lines.contains("/tmp/example"));
}

#[test]
fn width_resize_keeps_provider_error_and_footer_visible() {
    let backend = TestBackend::new(72, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_assistant_message(
            "[provider_error] provider returned status 401 for model `gpt-5.4` on attempt 1/3: {\"code\":\"INVALID_API_KEY\",\"message\":\"Invalid API key\"} | provider_failover={\"reason\":\"auth_rejected\",\"stage\":\"status_failure\",\"model\":\"gpt-5.4\",\"attempt\":1,\"max_attempts\":3,\"status_code\":401}".to_owned(),
        );

    terminal.draw(|f| app.render(f)).expect("draw");
    terminal.backend_mut().resize(28, 18);
    terminal.draw(|f| app.render(f)).expect("draw");

    let lines = buffer_lines(&terminal);
    let provider_row = lines
        .iter()
        .position(|line| line.contains("provider error"))
        .expect("provider error row");
    let detail_row = lines
        .iter()
        .position(|line| line.contains("INVALID_API_KEY"))
        .expect("provider error detail row");
    let footer_row = lines
        .iter()
        .position(|line| line.contains("gpt-test"))
        .expect("footer row");

    assert!(provider_row < detail_row);
    assert!(detail_row < footer_row);
    assert!(footer_row > detail_row);
    assert!(lines.iter().any(|line| line.contains("401")));
}

#[test]
fn width_resize_does_not_surface_internal_tool_result_or_transport_tail_in_plain_reply() {
    let backend = TestBackend::new(72, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_assistant_message(
            concat!(
                "我明白你的意思。\n\n",
                "我已经核到一件关键事实：当前配置里确实存在一个更宽的 file_root。\n\n",
                "[ok] {\"status\":\"ok\",\"tool\":\"read\",\"tool_call_id\":\"call-1\",\"payload_summary\":\"{\\\"path\\\":\\\"/workspace/demo/crates/daemon/src/lib.rs\\\",\\\"line_start\\\":1,\\\"line_end\\\":50}\",\"payload_chars\":2121,\"payload_truncated\":true}\n",
                "candidate_index=1 candidate_count=1 profile_index=1 profile_count=1 exhausted=true error=provider request failed for model `gpt-5.4` on attempt 3/3: error sending request for url (https://api.tonsof.blue/v1/chat/completions)"
            )
            .to_owned(),
        );

    terminal.draw(|f| app.render(f)).expect("draw");
    terminal.backend_mut().resize(28, 18);
    terminal.draw(|f| app.render(f)).expect("draw");

    let lines = buffer_lines(&terminal).join("\n");
    assert!(
        !lines.trim().is_empty(),
        "sanitized plain reply should still leave visible assistant content after resize: {lines}"
    );
    assert!(!lines.contains("[ok] {\"status\":\"ok\""));
    assert!(!lines.contains("provider request failed for model"));
    assert!(!lines.contains("candidate_index=1"));
}

#[test]
fn width_resize_keeps_pending_restore_footer_and_previews_visible() {
    let backend = TestBackend::new(72, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    app.pending_steers
        .push_back("nudge the current answer toward the root cause".to_owned());
    app.pending_queue
        .push_back("after that, summarize the diff and keep the footer visible".to_owned());

    terminal.draw(|f| app.render(f)).expect("draw");
    terminal.backend_mut().resize(34, 18);
    terminal.draw(|f| app.render(f)).expect("draw");

    let lines = buffer_lines(&terminal);
    let steer_row = lines
        .iter()
        .position(|line| line.contains("root cause"))
        .expect("steer preview row");
    let queue_header_row = lines
        .iter()
        .position(|line| line.contains("Queued follow-up messages"))
        .expect("queued header row");
    let queued_row = lines
        .iter()
        .enumerate()
        .skip(queue_header_row + 1)
        .find_map(|(idx, line)| line.contains("↳").then_some(idx))
        .expect("queued preview row");
    let composer_row = lines
        .iter()
        .position(|line| line.contains("›"))
        .expect("composer row");
    let footer_row = lines
        .iter()
        .position(|line| line.contains("Option + Up") || line.contains("Alt + Up"))
        .expect("restore footer row");

    assert!(steer_row < queue_header_row);
    assert!(queue_header_row < queued_row);
    assert!(queued_row < composer_row);
    assert!(composer_row < footer_row);
    assert!(lines[queued_row].contains("↳"));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Option + Up") || line.contains("Alt + Up"))
    );
}

#[test]
fn off_tail_pending_resize_and_end_restore_tail_without_losing_state() {
    let backend = TestBackend::new(48, 18);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    for idx in 0..18 {
        app.message_list.add_assistant_message(format!(
            "line-{idx} keeps transcript stable while pending preview and resize interact"
        ));
    }

    terminal.draw(|f| app.render(f)).expect("draw");
    app.message_list.handle_key(crossterm::event::KeyEvent::new(
        KeyCode::Up,
        KeyModifiers::NONE,
    ));
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    if let Ok(mut live) = app.live_transcript.lock() {
        live.draft_preview = Some("streamed preview line".to_owned());
    }

    terminal.draw(|f| app.render(f)).expect("draw off tail");
    let off_tail_lines = buffer_lines(&terminal).join("\n");
    assert!(off_tail_lines.contains("PgDn / End"));

    app.message_list
        .add_assistant_message("new-tail-line after scroll".to_owned());
    terminal.backend_mut().resize(34, 18);
    terminal.draw(|f| app.render(f)).expect("draw resized");
    let resized_lines = buffer_lines(&terminal).join("\n");
    assert!(resized_lines.contains("PgDn / End"));

    app.message_list.handle_key(crossterm::event::KeyEvent::new(
        KeyCode::End,
        KeyModifiers::NONE,
    ));
    terminal.draw(|f| app.render(f)).expect("draw restored");
    let restored_lines = buffer_lines(&terminal).join("\n");

    assert!(restored_lines.contains("new-tail-line after scroll"));
    assert!(!restored_lines.contains("PgDn / End"));
    assert_eq!(app.message_list.scroll_offset_for_test(), 0);
}

#[test]
fn pending_preview_shows_queued_steer_and_follow_up_above_composer() {
    let backend = TestBackend::new(72, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    app.pending_steers
        .push_back("nudge the current answer toward the root cause".to_owned());
    app.pending_queue
        .push_back("after that, summarize the diff".to_owned());

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal);
    let steer_header_row = lines
        .iter()
        .position(|line| line.contains("Messages to be submitted after next tool call"))
        .expect("steer header");
    let steer_row = lines
        .iter()
        .position(|line| line.contains("nudge the current answer"))
        .expect("steer preview");
    let queue_header_row = lines
        .iter()
        .position(|line| line.contains("Queued follow-up messages"))
        .expect("queue header");
    let queued_row = lines
        .iter()
        .position(|line| line.contains("after that, summarize"))
        .expect("queued preview");
    let composer_row = lines
        .iter()
        .position(|line| line.contains("›"))
        .expect("composer row");

    assert!(steer_header_row < steer_row);
    assert!(lines[steer_row].contains("↳"));
    assert!(queue_header_row < queued_row);
    assert!(lines[queued_row].contains("↳"));
    assert!(steer_row < queued_row);
    assert!(queued_row < composer_row);
}

#[test]
fn pending_preview_collapses_extra_messages_into_overflow_count() {
    let backend = TestBackend::new(72, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    app.pending_steers.push_back("first steer".to_owned());
    app.pending_steers.push_back("second steer".to_owned());
    app.pending_steers.push_back("third steer".to_owned());
    app.pending_steers.push_back("fourth steer".to_owned());

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal);

    assert!(lines.iter().any(|line| line.contains("first steer")));
    assert!(lines.iter().any(|line| line.contains("third steer")));
    assert!(!lines.iter().any(|line| line.contains("fourth steer")));
    assert!(lines.iter().any(|line| line.contains("… +1 more")));
}

#[test]
fn pending_preview_caps_total_items_across_steer_and_follow_up_queues() {
    let backend = TestBackend::new(72, 20);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    app.pending_steers.push_back("first steer".to_owned());
    app.pending_steers.push_back("second steer".to_owned());
    app.pending_queue.push_back("first follow-up".to_owned());
    app.pending_queue.push_back("second follow-up".to_owned());

    terminal.draw(|f| app.render(f)).expect("draw");
    let lines = buffer_lines(&terminal);

    assert!(lines.iter().any(|line| line.contains("first steer")));
    assert!(lines.iter().any(|line| line.contains("second steer")));
    assert!(lines.iter().any(|line| line.contains("first follow-up")));
    assert!(!lines.iter().any(|line| line.contains("second follow-up")));
    assert!(lines.iter().any(|line| line.contains("… +1 more")));
}

#[test]
fn queue_pending_message_moves_draft_into_follow_up_queue() {
    let mut app = blank_app();
    app.composer.set_input("queued draft".to_owned());
    app.composer_follow_up_intent = true;

    super::queue_pending_message(&mut app);

    assert_eq!(app.pending_queue.len(), 1);
    assert_eq!(
        app.pending_queue.front().map(String::as_str),
        Some("queued draft")
    );
    assert!(app.composer.is_empty());
    assert!(!app.composer_follow_up_intent);
}

#[test]
fn dequeue_pending_steer_prefers_follow_up_queue_before_steer_stack() {
    let mut app = blank_app();
    app.pending_steers.push_back("steer text".to_owned());
    app.pending_queue.push_back("queued follow-up".to_owned());

    assert!(super::dequeue_pending_steer(&mut app));
    assert_eq!(app.composer.take_input(), "queued follow-up");
    assert_eq!(app.pending_steers.len(), 1);
}

#[test]
fn pending_signature_ignores_hidden_tail_lines() {
    let mut app = blank_app();
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    if let Ok(mut live) = app.live_transcript.lock() {
        live.tool_activity_lines = vec![
            "reason-1".to_owned(),
            "reason-2".to_owned(),
            "reason-3".to_owned(),
            String::new(),
            "reply-1".to_owned(),
            "reply-2".to_owned(),
            "hidden-tail".to_owned(),
        ];
    }
    let before = super::pending_render_signature(&app);
    if let Ok(mut live) = app.live_transcript.lock() {
        live.tool_activity_lines = vec![
            "reason-1".to_owned(),
            "reason-2".to_owned(),
            "reason-3".to_owned(),
            String::new(),
            "reply-1".to_owned(),
            "reply-2".to_owned(),
            "different-hidden-tail".to_owned(),
        ];
    }
    let after = super::pending_render_signature(&app);

    assert_eq!(before, after);
}

#[test]
fn pending_signature_changes_when_follow_up_preview_changes() {
    let mut app = blank_app();
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    app.pending_steers.push_back("first steer".to_owned());
    let before = super::pending_render_signature(&app);
    app.pending_steers.clear();
    app.pending_queue
        .push_back("first queued follow-up".to_owned());
    let after = super::pending_render_signature(&app);

    assert_ne!(before, after);
}

#[test]
fn pending_signature_ignores_plain_reply_preview_changes() {
    let mut app = blank_app();
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    if let Ok(mut live) = app.live_transcript.lock() {
        live.tool_activity_lines = vec!["reason-1".to_owned(), String::new(), "reply-1".to_owned()];
    }
    let before = super::pending_render_signature(&app);
    if let Ok(mut live) = app.live_transcript.lock() {
        live.tool_activity_lines = vec!["reason-1".to_owned(), String::new(), "reply-2".to_owned()];
    }
    let after = super::pending_render_signature(&app);

    assert_eq!(before, after);
}

#[test]
fn transcript_preview_signature_changes_when_plain_preview_changes() {
    let mut app = blank_app();
    app.pending_turn = true;
    app.last_render_width = 72;
    if let Ok(mut live) = app.live_transcript.lock() {
        live.draft_preview = Some("first preview".to_owned());
    }
    let before = super::transcript_preview_signature(&app);
    if let Ok(mut live) = app.live_transcript.lock() {
        live.draft_preview = Some("second preview".to_owned());
    }
    let after = super::transcript_preview_signature(&app);

    assert_ne!(before, after);
}

#[test]
fn startup_overflow_still_keeps_user_block_top_padding_visible() {
    let backend = TestBackend::new(50, 14);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_startup_header(
        "0.1.0".to_owned(),
        "tutorial".to_owned(),
        vec![
            (
                "MCP".to_owned(),
                vec!["one".to_owned(), "two".to_owned(), "three".to_owned()],
            ),
            (
                "Skills".to_owned(),
                vec![
                    "alpha".to_owned(),
                    "beta".to_owned(),
                    "gamma".to_owned(),
                    "delta".to_owned(),
                ],
            ),
        ],
    );
    app.message_list.add_user_message("hi".to_owned());
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());

    terminal.draw(|f| app.render(f)).expect("draw");
    let user_row = find_row(&terminal, "hi").expect("user row");
    assert!(user_row > 0);
    assert!(
        row_has_background(&terminal, user_row - 1, SURFACE_USER_MSG_BG),
        "expected the row above the visible user text to be the user block top padding"
    );
}

#[test]
fn pending_transcript_keeps_user_block_bottom_padding_visible() {
    let backend = TestBackend::new(50, 16);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_startup_header(
        "0.1.0".to_owned(),
        "tutorial".to_owned(),
        vec![
            (
                "MCP".to_owned(),
                vec!["one".to_owned(), "two".to_owned(), "three".to_owned()],
            ),
            (
                "Skills".to_owned(),
                vec![
                    "alpha".to_owned(),
                    "beta".to_owned(),
                    "gamma".to_owned(),
                    "delta".to_owned(),
                ],
            ),
        ],
    );
    app.message_list.add_user_message("nihao".to_owned());
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());

    terminal.draw(|f| app.render(f)).expect("draw");
    let user_row = find_row(&terminal, "nihao").expect("user row");
    let pending_row = find_row(&terminal, "...")
        .or_else(|| find_row(&terminal, "中"))
        .unwrap_or(0);

    assert!(row_has_background(
        &terminal,
        user_row + 1,
        SURFACE_USER_MSG_BG
    ));
    assert!(pending_row > user_row);
}

#[test]
fn startup_overflow_with_pending_preview_keeps_user_block_visible_with_transcript_preview() {
    let backend = TestBackend::new(50, 16);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list.add_startup_header(
        "0.1.0".to_owned(),
        "tutorial".to_owned(),
        vec![
            (
                "MCP".to_owned(),
                vec!["one".to_owned(), "two".to_owned(), "three".to_owned()],
            ),
            (
                "Skills".to_owned(),
                vec![
                    "alpha".to_owned(),
                    "beta".to_owned(),
                    "gamma".to_owned(),
                    "delta".to_owned(),
                ],
            ),
        ],
    );
    app.message_list.add_user_message("hi".to_owned());
    app.pending_turn = true;
    app.turn_start = Some(std::time::Instant::now());
    if let Ok(mut live) = app.live_transcript.lock() {
        live.draft_preview = Some("pending reply".to_owned());
    }

    terminal.draw(|f| app.render(f)).expect("draw");
    let user_row = find_row(&terminal, "hi").expect("user row");
    let preview_row = find_row(&terminal, "pending reply").expect("preview row");
    let composer_row = find_row(&terminal, "›").expect("composer row");

    assert!(row_has_background(
        &terminal,
        user_row - 1,
        SURFACE_USER_MSG_BG
    ));
    assert!(preview_row > user_row);
    assert!(preview_row < composer_row);
    assert!(composer_row > user_row);
}

#[test]
fn startup_onboarding_renders_between_startup_header_and_composer() {
    let backend = TestBackend::new(72, 24);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app = blank_app();
    app.message_list
        .add_startup_header("0.1.0".to_owned(), "tutorial".to_owned(), Vec::new());
    app.startup_onboarding = Some(onboarding_state());

    terminal.draw(|f| app.render(f)).expect("draw");

    let version_row = find_row(&terminal, "0.1.0").expect("version row");
    let onboarding_row =
        find_row(&terminal, "onboarding · 1/6 · language").expect("onboarding row");
    let composer_row = find_row(&terminal, "›").expect("composer row");

    assert!(version_row < onboarding_row);
    assert!(onboarding_row < composer_row);
}

#[test]
fn startup_onboarding_language_confirmation_refreshes_header_copy() {
    let mut app = blank_app();
    app.detected_skills = vec![skill("demo-skill")];
    app.startup_mcp_count = 2;
    app.startup_version = "v0.1.0".to_owned();
    app.message_list.add_startup_header_with_tips(
        "v0.1.0".to_owned(),
        "ctrl+c exit".to_owned(),
        vec![
            ("Skills".to_owned(), vec!["1".to_owned()]),
            ("MCP".to_owned(), vec!["2".to_owned()]),
        ],
        vec!["type $skill".to_owned()],
    );
    let mut state = onboarding_state();
    state.language_index = 1;
    app.startup_onboarding = Some(state);

    let action = app
        .startup_onboarding
        .as_mut()
        .expect("onboarding state")
        .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(
        action,
        StartupOnboardingAction::ApplyLanguage(Language::ZhCn)
    );
    let mut runtime = test_runtime_with_path(PathBuf::from("/tmp/loong-language-test.toml"));
    assert!(
        app.apply_startup_onboarding_action(action, &mut runtime)
            .expect("apply onboarding action")
    );

    let rendered = app
        .message_list
        .get_rendered_lines(80)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("技能 (1)"));
    assert!(rendered.contains("ctrl+c 退出"));
}

#[test]
fn startup_onboarding_supports_all_shell_languages() {
    let mut env = ScopedEnv::new();
    env.set("LOONG_TUI_ONBOARD", "1");
    let runtime = test_runtime_with_path(PathBuf::from("/tmp/loong-language-list.toml"));
    let state =
        StartupOnboardingState::new(&runtime, Language::Ru).expect("startup onboarding state");

    assert_eq!(
        state.language_options,
        vec![
            Language::En,
            Language::ZhCn,
            Language::ZhTw,
            Language::Ja,
            Language::Ru,
        ]
    );
    assert_eq!(state.current_language(), Language::Ru);
}

#[test]
fn startup_onboarding_provider_stage_lists_all_sorted_provider_kinds() {
    let mut env = ScopedEnv::new();
    env.set("LOONG_TUI_ONBOARD", "1");
    let runtime = test_runtime_with_path(PathBuf::from("/tmp/loong-provider-list.toml"));

    let options = super::build_startup_provider_options(&runtime, Language::En);
    let labels = options
        .iter()
        .map(|option| option.label.as_str())
        .collect::<Vec<_>>();
    let expected = ProviderKind::all_sorted()
        .iter()
        .map(|kind| kind.display_name())
        .collect::<Vec<_>>();

    assert_eq!(labels, expected);
    let current_index = options
        .iter()
        .position(|option| option.kind == runtime.config.provider.kind)
        .expect("current provider should be present");
    assert!(options[current_index].recommended);
    assert!(options[current_index].is_current);
}

#[test]
fn startup_onboarding_escape_moves_back_without_dismissing() {
    let mut state = onboarding_state();
    state.stage = StartupOnboardingStage::Provider;

    let action = state.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(action, StartupOnboardingAction::Handled);
    assert_eq!(state.stage, StartupOnboardingStage::Language);

    let action = state.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(action, StartupOnboardingAction::Handled);
    assert_eq!(state.stage, StartupOnboardingStage::Language);
}

#[test]
fn apply_language_refreshes_localized_onboarding_runtime_copy() {
    let path = format!(
        "/tmp/loong-startup-language-refresh-{}-{}.toml",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("timestamp")
            .as_nanos()
    );
    let mut env = ScopedEnv::new();
    env.set("LOONG_TUI_ONBOARD", "1");
    let mut runtime = test_runtime_with_path(PathBuf::from(path));
    runtime.config.feishu.enabled = true;

    let mut app = blank_app();
    app.startup_onboarding = StartupOnboardingState::new(&runtime, Language::En);
    let state = app
        .startup_onboarding
        .as_mut()
        .expect("startup onboarding state");
    state.language_index = state
        .language_options
        .iter()
        .position(|language| *language == Language::ZhCn)
        .expect("zh-CN option");

    app.apply_startup_onboarding_action(
        StartupOnboardingAction::ApplyLanguage(Language::ZhCn),
        &mut runtime,
    )
    .expect("apply language");

    let state = app
        .startup_onboarding
        .as_ref()
        .expect("refreshed onboarding state");
    assert_eq!(state.current_language(), Language::ZhCn);
    assert_eq!(state.enabled_channel_labels, vec!["飞书".to_owned()]);
    let current_provider = state
        .provider_options
        .iter()
        .find(|option| option.is_current)
        .expect("current provider option");
    assert!(
        current_provider
            .detail
            .contains("沿用 config.toml 里的当前")
            && current_provider.detail.contains("凭证"),
        "provider detail should be localized after language apply: {}",
        current_provider.detail
    );
}

#[test]
fn persist_startup_provider_selection_updates_runtime_config_and_env_binding() {
    let temp_root = std::env::temp_dir().join(format!(
        "loong-startup-provider-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_root).expect("create temp root");
    let config_path = temp_root.join("config.toml");
    crate::config::write(
        Some(config_path.to_string_lossy().as_ref()),
        &LoongConfig::default(),
        true,
    )
    .expect("seed config");
    let mut runtime = test_runtime_with_path(config_path.clone());
    let mut env = ScopedEnv::new();
    env.set("ANTHROPIC_API_KEY", "test-key");

    let summary = super::persist_startup_provider_selection(
        &mut runtime,
        StartupProviderOption {
            kind: ProviderKind::Anthropic,
            auth_env_name: Some("ANTHROPIC_API_KEY".to_owned()),
            is_current: false,
            label: "Anthropic".to_owned(),
            detail: "detail".to_owned(),
            recommended: false,
        },
        Language::En,
    )
    .expect("persist provider selection");

    assert!(summary.contains("Anthropic"));
    assert!(summary.contains("ANTHROPIC_API_KEY"));
    assert_eq!(runtime.config.provider.kind, ProviderKind::Anthropic);
    assert_eq!(runtime.config.active_provider_id(), Some("anthropic"));
    assert!(runtime.config.providers.contains_key("anthropic"));
    assert_eq!(
        runtime.config.provider.resolved_auth_env_name().as_deref(),
        Some("ANTHROPIC_API_KEY")
    );
    let loaded =
        crate::config::load(Some(config_path.to_string_lossy().as_ref())).expect("reload config");
    assert_eq!(loaded.1.provider.kind, ProviderKind::Anthropic);
    assert_eq!(loaded.1.active_provider_id(), Some("anthropic"));
}

#[test]
fn startup_onboarding_skills_stage_toggles_selection_with_space() {
    let mut state = onboarding_state();
    state.stage = StartupOnboardingStage::Skills;
    state.feedback = None;

    let action = state.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));

    assert_eq!(action, StartupOnboardingAction::Handled);
    assert!(state.selected_skill_ids.contains("agent-browser"));
    assert_eq!(state.feedback.as_deref(), Some("selected 1 skill pack(s)."));
}

#[test]
fn startup_onboarding_setup_path_stage_surfaces_deeper_follow_up_details() {
    let mut state = onboarding_state();
    state.stage = StartupOnboardingStage::SetupPath;
    state.setup_path_index = 1;
    state.startup_mcp_count = 2;
    state.detected_skill_count = 5;
    state.provider_auth_env_name = Some("OPENAI_API_KEY".to_owned());
    state.provider_configuration_hint =
            Some("If you need to keep tuning provider base_url, model, or auth, `loong doctor` is the next check to run.".to_owned());

    let rendered = super::render_startup_onboarding_lines(&state, 90)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("provider + web setup"));
    assert!(rendered.contains("Provider auth env now: OPENAI_API_KEY."));
    assert!(rendered.contains("Web setup default: DuckDuckGo."));
    assert!(rendered.contains("loong doctor"));
    assert!(rendered.contains("loong onboard"));
}

#[test]
fn startup_onboarding_only_expands_the_selected_provider_detail() {
    let mut state = onboarding_state();
    state.stage = StartupOnboardingStage::Provider;
    state.provider_options = vec![
        StartupProviderOption {
            kind: ProviderKind::Openai,
            auth_env_name: None,
            is_current: true,
            label: "first provider".to_owned(),
            detail: "first provider detail".to_owned(),
            recommended: true,
        },
        StartupProviderOption {
            kind: ProviderKind::Anthropic,
            auth_env_name: None,
            is_current: false,
            label: "second provider".to_owned(),
            detail: "second provider detail".to_owned(),
            recommended: false,
        },
    ];
    state.provider_index = 1;

    let rendered = super::render_startup_onboarding_lines(&state, 90)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("first provider"));
    assert!(rendered.contains("second provider"));
    assert!(!rendered.contains("first provider detail"));
    assert!(rendered.contains("second provider detail"));
}

#[test]
fn startup_onboarding_setup_path_stage_surfaces_channel_follow_up_details() {
    let mut state = onboarding_state();
    state.stage = StartupOnboardingStage::SetupPath;
    state.setup_path_index = StartupSetupPathChoice::ChannelsAndDelivery as usize;
    state.enabled_channel_labels = vec!["飞书".to_owned(), "企业微信".to_owned()];
    state.channel_follow_up_commands =
        vec!["feishu serve".to_owned(), "channels serve wecom".to_owned()];
    state.channel_status_commands = vec!["loong doctor".to_owned()];
    state.channel_repair_commands = vec!["loong feishu onboard".to_owned()];

    let rendered = super::render_startup_onboarding_lines(&state, 90)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("channels + delivery"));
    assert!(rendered.contains("Enabled channels now: 飞书, 企业微信."));
    assert!(rendered.contains("Next runtime command: feishu serve."));
    assert!(rendered.contains("channels serve wecom"));
    assert!(rendered.contains("Health command: loong doctor."));
    assert!(rendered.contains("Repair path: loong feishu onboard."));
}

#[test]
fn startup_onboarding_channels_path_suggests_next_surfaces_when_none_are_enabled() {
    let mut state = onboarding_state();
    state.stage = StartupOnboardingStage::SetupPath;
    state.setup_path_index = StartupSetupPathChoice::ChannelsAndDelivery as usize;
    state.enabled_channel_labels.clear();
    state.channel_follow_up_commands.clear();
    state.channel_status_commands.clear();
    state.channel_repair_commands.clear();

    let rendered = super::render_startup_onboarding_lines(&state, 100)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Good first channels to wire"));
    assert!(rendered.contains("Telegram"));
    assert!(rendered.contains("continue setup through `loong onboard`"));
}

#[test]
fn startup_onboarding_uses_localized_setup_path_copy_in_chinese() {
    let mut state = onboarding_state();
    state.language_options = vec![Language::ZhCn];
    state.language_index = 0;
    state.stage = StartupOnboardingStage::SetupPath;
    state.setup_path_index = StartupSetupPathChoice::ChannelsAndDelivery as usize;
    state.enabled_channel_labels = vec!["飞书".to_owned()];
    state.channel_follow_up_commands = vec!["feishu serve".to_owned()];

    let rendered = super::render_startup_onboarding_lines(&state, 100)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("onboarding · 4/6 · 后续配置"));
    assert!(rendered.contains("当前已启用的 channel：飞书。"));
    assert!(rendered.contains("下一条 runtime command：feishu serve。"));
}

#[test]
fn startup_onboarding_channels_path_localizes_suggested_surfaces_in_chinese() {
    let mut state = onboarding_state();
    state.language_options = vec![Language::ZhCn];
    state.language_index = 0;
    state.stage = StartupOnboardingStage::SetupPath;
    state.setup_path_index = StartupSetupPathChoice::ChannelsAndDelivery as usize;
    state.enabled_channel_labels.clear();
    state.channel_follow_up_commands.clear();
    state.channel_status_commands.clear();
    state.channel_repair_commands.clear();

    let rendered = super::render_startup_onboarding_lines(&state, 100)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("建议优先接的 channels"));
    assert!(rendered.contains("飞书"));
    assert!(rendered.contains("当前还没有可直接运行的 channel runtime command"));
}

#[test]
fn persist_startup_personalization_localizes_summary_in_chinese() {
    let temp_root = std::env::temp_dir().join(format!(
        "loong-startup-personalization-zh-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_root).expect("create temp root");
    let config_path = temp_root.join("config.toml");
    crate::config::write(
        Some(config_path.to_string_lossy().as_ref()),
        &LoongConfig::default(),
        true,
    )
    .expect("seed config");
    let mut runtime = test_runtime_with_path(config_path);

    let summary = persist_startup_personalization(
        &mut runtime,
        StartupPersonalizationPreset::Concise,
        Language::ZhCn,
    )
    .expect("persist personalization");

    assert!(summary.contains("已保存"));
    assert!(summary.contains("简洁模式"));
}

#[test]
fn persist_startup_personalization_upgrades_memory_profile_and_saves_choice() {
    let temp_root = std::env::temp_dir().join(format!(
        "loong-startup-personalization-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_root).expect("create temp root");
    let config_path = temp_root.join("config.toml");
    crate::config::write(
        Some(config_path.to_string_lossy().as_ref()),
        &LoongConfig::default(),
        true,
    )
    .expect("seed config");
    let mut runtime = test_runtime_with_path(config_path);

    let summary = persist_startup_personalization(
        &mut runtime,
        StartupPersonalizationPreset::Thorough,
        Language::ZhCn,
    )
    .expect("persist personalization");

    assert!(summary.contains("profile_plus_window"));
    assert_eq!(
        runtime.config.memory.profile,
        crate::config::MemoryProfile::ProfilePlusWindow
    );
    let personalization = runtime
        .config
        .memory
        .personalization
        .as_ref()
        .expect("saved personalization");
    assert_eq!(
        personalization.response_density,
        Some(crate::config::ResponseDensity::Thorough)
    );
    assert_eq!(
        personalization.initiative_level,
        Some(crate::config::InitiativeLevel::HighInitiative)
    );
    assert_eq!(personalization.locale.as_deref(), Some("zh-CN"));
}

#[test]
fn persist_startup_personalization_turn_off_suppresses_future_prompts() {
    let temp_root = std::env::temp_dir().join(format!(
        "loong-startup-personalization-off-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_root).expect("create temp root");
    let config_path = temp_root.join("config.toml");
    crate::config::write(
        Some(config_path.to_string_lossy().as_ref()),
        &LoongConfig::default(),
        true,
    )
    .expect("seed config");
    let mut runtime = test_runtime_with_path(config_path);

    let summary = persist_startup_personalization(
        &mut runtime,
        StartupPersonalizationPreset::TurnOff,
        Language::En,
    )
    .expect("persist turn-off personalization");

    assert!(summary.contains("turned off"));
    let personalization = runtime
        .config
        .memory
        .personalization
        .as_ref()
        .expect("saved personalization");
    assert_eq!(
        personalization.prompt_state,
        crate::config::PersonalizationPromptState::Suppressed
    );
}

#[test]
fn configured_personalization_stages_first_turn_bootstrap_addendum() {
    let mut app = blank_app();
    app.startup_onboarding = Some(onboarding_state());
    let config_path = PathBuf::from("/tmp/loong-first-turn-bootstrap.toml");
    let mut runtime = test_runtime_with_path(config_path);

    app.apply_startup_onboarding_action(
        StartupOnboardingAction::PersistPersonalization(StartupPersonalizationPreset::Balanced),
        &mut runtime,
    )
    .expect("apply startup personalization");

    let addendum = app
        .pending_first_turn_bootstrap_addendum
        .as_deref()
        .expect("bootstrap addendum");
    assert!(addendum.contains("next real reply") || addendum.contains("下一次真正回复"));
}

#[test]
fn turn_off_personalization_does_not_stage_first_turn_bootstrap_addendum() {
    let mut app = blank_app();
    app.startup_onboarding = Some(onboarding_state());
    let config_path = PathBuf::from("/tmp/loong-first-turn-bootstrap-off.toml");
    let mut runtime = test_runtime_with_path(config_path);

    app.apply_startup_onboarding_action(
        StartupOnboardingAction::PersistPersonalization(StartupPersonalizationPreset::TurnOff),
        &mut runtime,
    )
    .expect("apply startup personalization");

    assert!(app.pending_first_turn_bootstrap_addendum.is_none());
}

#[test]
fn apply_first_turn_bootstrap_addendum_mutates_runtime_transiently() {
    let config_path = PathBuf::from("/tmp/loong-first-turn-bootstrap-runtime.toml");
    let mut runtime = test_runtime_with_path(config_path);
    runtime.config.cli.prompt_pack_id = Some("default".to_owned());

    super::apply_first_turn_bootstrap_addendum(&mut runtime, "bootstrap question here".to_owned());

    assert_eq!(
        runtime.config.cli.system_prompt_addendum.as_deref(),
        Some("bootstrap question here")
    );
}

#[test]
fn infer_startup_bootstrap_capture_supports_natural_language_english() {
    let capture = super::infer_startup_bootstrap_capture(
            "You can call me Chum. My pronouns are he/they. Your name is Loongy. Creature is dragon. Vibe is calm. Emoji is 🐉. My timezone is Asia/Shanghai. Boundaries are ask before destructive actions. Note: I work mostly late at night.",
        )
        .expect("capture");

    assert_eq!(capture.preferred_address.as_deref(), Some("Chum"));
    assert_eq!(capture.pronouns.as_deref(), Some("he/they"));
    assert_eq!(capture.agent_name.as_deref(), Some("Loongy"));
    assert_eq!(capture.creature.as_deref(), Some("dragon"));
    assert_eq!(capture.vibe.as_deref(), Some("calm"));
    assert_eq!(capture.emoji.as_deref(), Some("🐉"));
    assert_eq!(capture.timezone.as_deref(), Some("Asia/Shanghai"));
    assert_eq!(
        capture.standing_boundaries.as_deref(),
        Some("ask before destructive actions")
    );
    assert_eq!(
        capture.notes.as_deref(),
        Some("I work mostly late at night")
    );
}

#[test]
fn infer_startup_bootstrap_capture_supports_natural_language_chinese() {
    let capture = super::infer_startup_bootstrap_capture(
            "叫我伙伴。代词是 ta。你可以叫星龙。物种是龙，气质是沉稳，emoji 用✨。时区是 Asia/Shanghai。边界是先问再做破坏性操作。备注是我通常夜里工作。",
        )
        .expect("capture");

    assert_eq!(capture.preferred_address.as_deref(), Some("伙伴"));
    assert_eq!(capture.pronouns.as_deref(), Some("ta"));
    assert_eq!(capture.agent_name.as_deref(), Some("星龙"));
    assert_eq!(capture.creature.as_deref(), Some("龙"));
    assert_eq!(capture.vibe.as_deref(), Some("沉稳"));
    assert_eq!(capture.emoji.as_deref(), Some("✨"));
    assert_eq!(capture.timezone.as_deref(), Some("Asia/Shanghai"));
    assert_eq!(
        capture.standing_boundaries.as_deref(),
        Some("先问再做破坏性操作")
    );
    assert_eq!(capture.notes.as_deref(), Some("我通常夜里工作"));
}

#[test]
fn persist_startup_bootstrap_capture_updates_config_and_runtime_self_files() {
    let temp_root = std::env::temp_dir().join(format!(
        "loong-startup-bootstrap-capture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix time")
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_root).expect("create temp root");
    let config_path = temp_root.join("config.toml");
    crate::config::write(
        Some(config_path.to_string_lossy().as_ref()),
        &LoongConfig::default(),
        true,
    )
    .expect("seed config");
    let mut runtime = test_runtime_with_path(config_path);
    runtime.effective_working_directory = Some(temp_root.clone());

    let capture = StartupBootstrapCapture {
        preferred_address: Some("Chum".to_owned()),
        pronouns: Some("he/they".to_owned()),
        agent_name: Some("Loongy".to_owned()),
        creature: Some("dragon".to_owned()),
        vibe: Some("calm".to_owned()),
        emoji: Some("🐉".to_owned()),
        timezone: Some("Asia/Shanghai".to_owned()),
        standing_boundaries: Some("Ask before destructive actions.".to_owned()),
        notes: Some("Works mostly late at night.".to_owned()),
    };

    super::persist_startup_bootstrap_capture(&mut runtime, &capture)
        .expect("persist bootstrap capture");

    let personalization = runtime
        .config
        .memory
        .personalization
        .as_ref()
        .expect("personalization");
    assert_eq!(personalization.preferred_name.as_deref(), Some("Chum"));
    assert_eq!(personalization.timezone.as_deref(), Some("Asia/Shanghai"));
    assert_eq!(
        personalization.standing_boundaries.as_deref(),
        Some("Ask before destructive actions.")
    );

    let user = std::fs::read_to_string(temp_root.join("USER.md")).expect("USER.md");
    let identity = std::fs::read_to_string(temp_root.join("IDENTITY.md")).expect("IDENTITY.md");
    let soul = std::fs::read_to_string(temp_root.join("SOUL.md")).expect("SOUL.md");
    assert!(user.contains("Preferred address: Chum"));
    assert!(user.contains("Pronouns: he/they"));
    assert!(user.contains("Timezone: Asia/Shanghai"));
    assert!(user.contains("Standing boundaries: Ask before destructive actions."));
    assert!(user.contains("Notes: Works mostly late at night."));
    assert!(identity.contains("Name: Loongy"));
    assert!(identity.contains("Creature: dragon"));
    assert!(identity.contains("Vibe: calm"));
    assert!(identity.contains("Emoji: 🐉"));
    assert!(soul.contains("Preferred vibe: calm"));
    assert!(soul.contains("Signature emoji: 🐉"));
}

#[test]
fn unparsable_bootstrap_reply_keeps_waiting_for_capture() {
    let mut app = blank_app();
    app.awaiting_first_turn_bootstrap_reply = true;
    let config_path = PathBuf::from("/tmp/loong-bootstrap-waiting.toml");
    let mut runtime = test_runtime_with_path(config_path);

    super::maybe_capture_and_persist_first_turn_bootstrap_reply(
        &mut app,
        &mut runtime,
        "let's just continue for now",
    )
    .expect("capture reply");

    assert!(app.awaiting_first_turn_bootstrap_reply);
}

#[test]
fn bootstrap_reply_opt_out_clears_waiting_without_persisting() {
    let mut app = blank_app();
    app.awaiting_first_turn_bootstrap_reply = true;
    let config_path = PathBuf::from("/tmp/loong-bootstrap-opt-out.toml");
    let mut runtime = test_runtime_with_path(config_path);

    super::maybe_capture_and_persist_first_turn_bootstrap_reply(
        &mut app,
        &mut runtime,
        "skip for now",
    )
    .expect("capture reply");

    assert!(!app.awaiting_first_turn_bootstrap_reply);
    assert!(runtime.config.memory.personalization.is_none());
}

#[test]
fn finish_stage_summarizes_setup_path_and_personalization_choice() {
    let mut state = onboarding_state();
    state.stage = StartupOnboardingStage::Finish;
    state.setup_path_index = StartupSetupPathChoice::ProviderAndWeb as usize;
    state.selected_personalization = Some(StartupPersonalizationPreset::Balanced);

    let rendered = super::render_startup_onboarding_lines(&state, 90)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("setup path · provider + web setup"));
    assert!(rendered.contains("personalization · balanced operator"));
}

#[test]
fn finish_stage_turn_off_personalization_updates_finish_subtitle() {
    let mut state = onboarding_state();
    state.stage = StartupOnboardingStage::Finish;
    state.language_options = vec![Language::ZhCn];
    state.language_index = 0;
    state.selected_personalization = Some(StartupPersonalizationPreset::TurnOff);

    let rendered = super::render_startup_onboarding_lines(&state, 100)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("Loong 不会再主动弹个性化提示"));
    assert!(rendered.contains("loong personalize"));
}
