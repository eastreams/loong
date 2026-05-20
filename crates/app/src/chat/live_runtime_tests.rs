use super::{
    CliChatLiveFileChangeView, CliChatLiveOutputView, CliChatLiveSurfaceRenderPayload,
    CliChatLiveSurfaceSink, CliChatLiveSurfaceSnapshot, CliChatLiveToolSnapshot,
    build_cli_chat_live_compact_observer_controller, render_cli_chat_live_compact_lines_with_width,
    render_cli_chat_live_surface_lines_with_width, render_live_preview_segment_lines,
};
use crate::conversation::{
    ConversationTurnPhase, ConversationTurnPhaseEvent, ConversationTurnToolState, ExecutionLane,
};
use crate::tools::runtime_events::ToolFileChangeKind;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicUsize, Ordering};

fn assert_uniform_display_width(lines: &[String]) {
    let Some(first_width) = lines
        .first()
        .map(|line| crate::presentation::display_width(line))
    else {
        return;
    };

    for line in lines {
        assert_eq!(
            crate::presentation::display_width(line),
            first_width,
            "table line has a different display width: {line:?}"
        );
    }
}

fn empty_output() -> CliChatLiveOutputView {
    CliChatLiveOutputView {
        text: String::new(),
        total_bytes: 0,
        total_lines: 0,
        truncated: false,
    }
}

#[test]
fn compact_render_shows_preview_without_card_chrome() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RequestingProvider,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 0,
        message_count: Some(4),
        estimated_tokens: Some(1200),
        first_token_latency_ms: None,
        draft_preview: Some("Hello there\nHow are you?".to_owned()),
        tools: Vec::new(),
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 40);
    let joined = lines.join("\n");

    assert!(joined.contains("Hello there"));
    assert!(joined.contains("How are you?"));
    assert!(!joined.contains("╭─"));
    assert!(!joined.contains("turn pipeline"));
}

#[test]
fn compact_render_includes_tool_activity_summary_without_card_chrome() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RunningTools,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Safe),
        tool_call_count: 1,
        message_count: Some(6),
        estimated_tokens: Some(1800),
        first_token_latency_ms: None,
        draft_preview: None,
        tools: vec![CliChatLiveToolSnapshot {
            tool_call_id: "call-1".to_owned(),
            name: Some("read_file".to_owned()),
            request_summary: Some("Read src/main.rs".to_owned()),
            args: "{\"path\":\"src/main.rs\"}".to_owned(),
            status: ConversationTurnToolState::Running,
            detail: Some("working".to_owned()),
            stdout: empty_output(),
            stderr: empty_output(),
            file_change: None,
            duration_ms: Some(12),
            exit_code: None,
        }],
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 60);
    let joined = lines.join("\n");

    assert!(joined.contains("• Called read_file · working"));
    assert!(joined.contains("↳ Read src/main.rs"));
    assert!(joined.contains("↳ args path=src/main.rs"));
    assert!(joined.contains("↳ metrics 12ms"));
    assert!(!joined.contains("╭─"));
    assert!(!joined.contains("tool activity]"));
}

#[test]
fn compact_render_compacts_structured_request_and_args_previews() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RunningTools,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 1,
        message_count: Some(3),
        estimated_tokens: Some(900),
        first_token_latency_ms: None,
        draft_preview: None,
        tools: vec![CliChatLiveToolSnapshot {
            tool_call_id: "call-2".to_owned(),
            name: Some("search".to_owned()),
            request_summary: Some("{\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}".to_owned()),
            args: "{\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}".to_owned(),
            status: ConversationTurnToolState::Running,
            detail: None,
            stdout: empty_output(),
            stderr: empty_output(),
            file_change: None,
            duration_ms: None,
            exit_code: None,
        }],
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 64);
    let joined = lines.join("\n");

    assert!(joined.contains("• Called search"));
    assert!(joined.contains("↳ request") || joined.contains("query=rust"));
    assert!(!joined.contains("↳ args query=rust"));
    assert!(joined.contains("limit=5"));
}

#[test]
fn compact_render_promotes_command_request_into_primary_preview_line() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RunningTools,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 1,
        message_count: Some(3),
        estimated_tokens: Some(900),
        first_token_latency_ms: None,
        draft_preview: None,
        tools: vec![CliChatLiveToolSnapshot {
            tool_call_id: "call-bash".to_owned(),
            name: Some("bash".to_owned()),
            request_summary: None,
            args: "{\"cmd\":\"cargo test --workspace --all-features\"}".to_owned(),
            status: ConversationTurnToolState::Running,
            detail: Some("working".to_owned()),
            stdout: empty_output(),
            stderr: empty_output(),
            file_change: None,
            duration_ms: None,
            exit_code: None,
        }],
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 72);
    let joined = lines.join("\n");

    assert!(joined.contains("• Called bash · working"));
    assert!(joined.contains("↳ Command cargo test --workspace --all-features"));
    assert!(joined.contains("↳ args cmd=cargo test --workspace --all-features"));
}

#[test]
fn compact_render_promotes_search_request_into_primary_preview_line() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RunningTools,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 1,
        message_count: Some(3),
        estimated_tokens: Some(900),
        first_token_latency_ms: None,
        draft_preview: None,
        tools: vec![CliChatLiveToolSnapshot {
            tool_call_id: "call-search".to_owned(),
            name: Some("grep".to_owned()),
            request_summary: None,
            args: "{\"query\":\"稳定|wenjian|robust|stable\",\"path\":\"~/chat\"}".to_owned(),
            status: ConversationTurnToolState::Running,
            detail: Some("working".to_owned()),
            stdout: empty_output(),
            stderr: empty_output(),
            file_change: None,
            duration_ms: None,
            exit_code: None,
        }],
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 80);
    let joined = lines.join("\n");

    assert!(joined.contains("• Called grep · working"));
    assert!(joined.contains("↳ Search \"稳定|wenjian|robust|stable\" in ~/chat"));
}

#[test]
fn compact_render_promotes_glob_request_into_primary_preview_line() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RunningTools,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 1,
        message_count: Some(3),
        estimated_tokens: Some(900),
        first_token_latency_ms: None,
        draft_preview: None,
        tools: vec![CliChatLiveToolSnapshot {
            tool_call_id: "call-glob".to_owned(),
            name: Some("find_files".to_owned()),
            request_summary: None,
            args: "{\"glob\":\"src/**/*.rs\",\"path\":\"~/chat\"}".to_owned(),
            status: ConversationTurnToolState::Running,
            detail: Some("working".to_owned()),
            stdout: empty_output(),
            stderr: empty_output(),
            file_change: None,
            duration_ms: None,
            exit_code: None,
        }],
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 80);
    let joined = lines.join("\n");

    assert!(joined.contains("• Called find_files · working"));
    assert!(joined.contains("↳ Glob src/**/*.rs in ~/chat"));
}

#[test]
fn compact_render_compacts_stderr_and_file_children() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RunningTools,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 1,
        message_count: Some(3),
        estimated_tokens: Some(900),
        first_token_latency_ms: None,
        draft_preview: None,
        tools: vec![CliChatLiveToolSnapshot {
            tool_call_id: "call-3".to_owned(),
            name: Some("exec".to_owned()),
            request_summary: None,
            args: String::new(),
            status: ConversationTurnToolState::Completed,
            detail: Some("ok".to_owned()),
            stdout: empty_output(),
            stderr: CliChatLiveOutputView {
                text: "permission denied".to_owned(),
                total_bytes: 17,
                total_lines: 1,
                truncated: false,
            },
            file_change: Some(CliChatLiveFileChangeView {
                path: "src/lib.rs".to_owned(),
                operation: ToolFileChangeKind::Edit,
                added_lines: 2,
                removed_lines: 1,
                preview: None,
            }),
            duration_ms: Some(42),
            exit_code: Some(0),
        }],
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 64);
    let joined = lines.join("\n");

    assert!(joined.contains("• Closed exec · ok"));
    assert!(joined.contains("↳ stderr 1 lines · 17 bytes"));
    assert!(joined.contains("permission denied"));
    assert!(joined.contains("↳ file edit src/lib.rs (+2 / -1)"));
    assert!(joined.contains("↳ metrics 42ms · exit=0"));
}

#[test]
fn compact_render_surfaces_approval_and_denied_status_lines() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RunningTools,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Safe),
        tool_call_count: 2,
        message_count: Some(5),
        estimated_tokens: Some(700),
        first_token_latency_ms: None,
        draft_preview: None,
        tools: vec![
            CliChatLiveToolSnapshot {
                tool_call_id: "call-a".to_owned(),
                name: Some("search".to_owned()),
                request_summary: None,
                args: String::new(),
                status: ConversationTurnToolState::NeedsApproval,
                detail: Some("operator confirmation required".to_owned()),
                stdout: empty_output(),
                stderr: empty_output(),
                file_change: None,
                duration_ms: None,
                exit_code: None,
            },
            CliChatLiveToolSnapshot {
                tool_call_id: "call-b".to_owned(),
                name: Some("search".to_owned()),
                request_summary: None,
                args: String::new(),
                status: ConversationTurnToolState::Denied,
                detail: Some("blocked".to_owned()),
                stdout: empty_output(),
                stderr: empty_output(),
                file_change: None,
                duration_ms: None,
                exit_code: None,
            },
        ],
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 64);
    let joined = lines.join("\n");

    assert!(joined.contains("• Approval search · operator confirmation required"));
    assert!(joined.contains("• Denied search · blocked"));
}

#[test]
fn compact_render_splits_think_blocks_into_reasoning_and_visible_reply() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RequestingProvider,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 0,
        message_count: Some(2),
        estimated_tokens: Some(512),
        first_token_latency_ms: None,
        draft_preview: Some("<think>quiet reasoning\nsecond line</think>Hello there".to_owned()),
        tools: Vec::new(),
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 50);
    let joined = lines.join("\n");

    assert!(joined.contains("quiet reasoning"));
    assert!(joined.contains("second line"));
    assert!(joined.contains("Hello there"));
    assert!(!joined.contains("<think>"));
    assert!(!joined.contains("</think>"));
}

#[test]
fn compact_render_collapses_outer_and_repeated_blank_lines() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RequestingProvider,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 0,
        message_count: Some(2),
        estimated_tokens: Some(512),
        first_token_latency_ms: None,
        draft_preview: Some("\n\n<think>reasoning line</think>\n\n\nvisible reply\n\n".to_owned()),
        tools: Vec::new(),
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 50);
    assert_eq!(
        lines,
        vec![
            "reasoning line".to_owned(),
            String::new(),
            "visible reply".to_owned()
        ]
    );
}

#[test]
fn compact_render_hides_partial_think_tag_prefixes() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RequestingProvider,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 0,
        message_count: Some(1),
        estimated_tokens: Some(128),
        first_token_latency_ms: None,
        draft_preview: Some("<thi".to_owned()),
        tools: Vec::new(),
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 40);

    assert!(lines.is_empty());
}

#[test]
fn compact_render_keeps_incomplete_trailing_paragraph_literal_while_streaming() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RequestingProvider,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 0,
        message_count: Some(1),
        estimated_tokens: Some(256),
        first_token_latency_ms: None,
        draft_preview: Some("hello\nworld from stream".to_owned()),
        tools: Vec::new(),
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 80);

    assert_eq!(
        lines,
        vec!["hello".to_owned(), "world from stream".to_owned()]
    );
}

#[test]
fn compact_render_drops_trailing_unclosed_code_fence_suffix() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RequestingProvider,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 0,
        message_count: Some(1),
        estimated_tokens: Some(256),
        first_token_latency_ms: None,
        draft_preview: Some("hello world\n```".to_owned()),
        tools: Vec::new(),
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 80);

    assert_eq!(lines, vec!["hello world".to_owned()]);
}

#[test]
fn compact_render_keeps_visible_text_when_partial_closing_think_tag_arrives() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RequestingProvider,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 0,
        message_count: Some(1),
        estimated_tokens: Some(256),
        first_token_latency_ms: None,
        draft_preview: Some("visible answer</t".to_owned()),
        tools: Vec::new(),
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 80);

    assert_eq!(lines, vec!["visible answer".to_owned()]);
}

#[test]
fn compact_render_structures_markdown_tables_in_preview() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RequestingProvider,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 0,
        message_count: Some(1),
        estimated_tokens: Some(256),
        first_token_latency_ms: None,
        draft_preview: Some(
            "| 指标 | 数值 |\n| --- | --- |\n| 覆盖率 | 68% |\n| 平均响应时间 | 220ms |".to_owned(),
        ),
        tools: Vec::new(),
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 40);
    let joined = lines.join("\n");

    assert!(joined.contains("┌"));
    assert!(joined.contains("覆盖率"));
    assert!(joined.contains("220ms"));
    assert!(!joined.contains("| --- |"));
}

#[test]
fn compact_render_structures_provisional_markdown_tables_in_preview() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RequestingProvider,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 0,
        message_count: Some(1),
        estimated_tokens: Some(256),
        first_token_latency_ms: None,
        draft_preview: Some("| Name | Value |\n| A | 1 |\n| B | 2 |".to_owned()),
        tools: Vec::new(),
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 32);

    assert_eq!(
        lines,
        vec![
            "┌──────┬───────┐".to_owned(),
            "│ Name │ Value │".to_owned(),
            "├──────┼───────┤".to_owned(),
            "│ A    │ 1     │".to_owned(),
            "│ B    │ 2     │".to_owned(),
            "└──────┴───────┘".to_owned(),
        ]
    );
    assert_uniform_display_width(lines.as_slice());
}

#[test]
fn surface_render_structures_markdown_tables_in_preview() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RequestingProvider,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 0,
        message_count: Some(1),
        estimated_tokens: Some(256),
        first_token_latency_ms: None,
        draft_preview: Some(
            "| 指标 | 数值 |\n| --- | --- |\n| 覆盖率 | 68% |\n| 平均响应时间 | 220ms |".to_owned(),
        ),
        tools: Vec::new(),
    };

    let lines = render_cli_chat_live_surface_lines_with_width(&snapshot, 72);
    let joined = lines.join("\n");

    assert!(joined.contains("draft preview"));
    assert!(joined.contains("┌"));
    assert!(joined.contains("覆盖率"));
    assert!(joined.contains("220ms"));
    assert!(!joined.contains("| --- |"));
}

#[test]
fn compact_render_structures_fenced_diff_in_preview() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RequestingProvider,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 0,
        message_count: Some(1),
        estimated_tokens: Some(256),
        first_token_latency_ms: None,
        draft_preview: Some("```diff\n- old value\n+ new value\n```".to_owned()),
        tools: Vec::new(),
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 50);
    let joined = lines.join("\n");

    assert!(joined.contains("- old value"));
    assert!(joined.contains("+ new value"));
    assert!(!joined.contains("```diff"));
}

#[test]
fn compact_render_structures_provisional_diff_fence_in_preview() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RequestingProvider,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 0,
        message_count: Some(1),
        estimated_tokens: Some(256),
        first_token_latency_ms: None,
        draft_preview: Some("```di\n- old value\n+ new value".to_owned()),
        tools: Vec::new(),
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 50);
    let joined = lines.join("\n");

    assert!(joined.contains("- old value"), "{joined}");
    assert!(joined.contains("+ new value"), "{joined}");
    assert!(!joined.contains("```di"), "{joined}");
}

#[test]
fn surface_render_structures_fenced_diff_in_preview() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RequestingProvider,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 0,
        message_count: Some(1),
        estimated_tokens: Some(256),
        first_token_latency_ms: None,
        draft_preview: Some("```diff\n- old value\n+ new value\n```".to_owned()),
        tools: Vec::new(),
    };

    let lines = render_cli_chat_live_surface_lines_with_width(&snapshot, 72);
    let joined = lines.join("\n");

    assert!(joined.contains("draft preview"), "{joined}");
    assert!(joined.contains("- old value"), "{joined}");
    assert!(joined.contains("+ new value"), "{joined}");
    assert!(!joined.contains("```diff"), "{joined}");
}

#[test]
fn compact_render_structures_fenced_code_block_in_preview() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RequestingProvider,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 0,
        message_count: Some(1),
        estimated_tokens: Some(256),
        first_token_latency_ms: None,
        draft_preview: Some("```bash\nnpm install\nnpm test\n```".to_owned()),
        tools: Vec::new(),
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 40);
    let joined = lines.join("\n");

    assert!(joined.contains("```bash"));
    assert!(joined.contains("npm install"));
    assert!(joined.contains("npm test"));
    assert!(joined.contains("```"));
}

#[test]
fn compact_render_structures_markdown_list_in_preview() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RequestingProvider,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 0,
        message_count: Some(1),
        estimated_tokens: Some(256),
        first_token_latency_ms: None,
        draft_preview: Some("## 本周进展\n- 修复崩溃\n- 提升性能".to_owned()),
        tools: Vec::new(),
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 40);
    let joined = lines.join("\n");

    assert!(joined.contains("## 本周进展"));
    assert!(joined.contains("• 修复崩溃"));
    assert!(joined.contains("• 提升性能"));
}

#[test]
fn preview_emit_waits_for_a_stable_initial_boundary() {
    let mut state = super::CliChatLiveSurfaceState {
        draft_preview: "hel".to_owned(),
        total_text_chars_seen: 3,
        ..Default::default()
    };

    assert!(!super::should_emit_cli_chat_live_preview(
        &state,
        40,
        Some(3)
    ));

    state.draft_preview.push(' ');
    state.total_text_chars_seen = 4;

    assert!(super::should_emit_cli_chat_live_preview(
        &state,
        40,
        Some(4)
    ));
}

#[test]
fn preview_emit_allows_a_readable_initial_phrase() {
    let state = super::CliChatLiveSurfaceState {
        draft_preview: "Draft response".to_owned(),
        total_text_chars_seen: "Draft response".chars().count(),
        ..Default::default()
    };

    assert!(super::should_emit_cli_chat_live_preview(
        &state,
        72,
        Some(42)
    ));
}

#[test]
fn preview_emit_forces_progress_after_large_unstable_burst() {
    let state = super::CliChatLiveSurfaceState {
        last_preview_emit_chars_seen: 8,
        draft_preview: "averylongunstablesuffixwithoutbreaks".to_owned(),
        total_text_chars_seen: 24,
        ..Default::default()
    };

    assert!(super::should_emit_cli_chat_live_preview(
        &state,
        8,
        Some(24)
    ));
}

#[test]
fn preview_emit_uses_visual_line_pressure_for_wrapped_cjk_text() {
    let state = super::CliChatLiveSurfaceState {
        draft_preview: "渲染表格边界".to_owned(),
        total_text_chars_seen: 5,
        ..Default::default()
    };

    assert!(super::should_emit_cli_chat_live_preview(&state, 8, Some(5)));
}

#[test]
fn preview_emit_mode_enters_catch_up_when_visual_backlog_grows() {
    let mut state = super::CliChatLiveSurfaceState::default();
    state.draft_preview =
        "line one wraps quickly\nline two wraps quickly\nline three wraps quickly".to_owned();
    state.total_text_chars_seen = state.draft_preview.chars().count();
    state.last_preview_emit_chars_seen = 8;
    state.last_preview_emit_visual_line_count = 1;

    assert_eq!(
        super::cli_chat_live_preview_emit_mode(&state, 18),
        super::CliChatLivePreviewEmitMode::CatchUp
    );
}

#[test]
fn preview_emit_mode_stays_smooth_for_small_stable_updates() {
    let mut state = super::CliChatLiveSurfaceState::default();
    state.draft_preview = "hello world ".to_owned();
    state.total_text_chars_seen = state.draft_preview.chars().count();
    state.last_preview_emit_chars_seen = 8;
    state.last_preview_emit_visual_line_count = 1;

    assert_eq!(
        super::cli_chat_live_preview_emit_mode(&state, 80),
        super::CliChatLivePreviewEmitMode::Smooth
    );
}

#[test]
fn preview_emit_cadence_slows_smooth_mode() {
    let mut state = super::CliChatLiveSurfaceState::default();
    state.draft_preview =
        "hello world this is a stable preview chunk with just enough size ".to_owned();
    state.total_text_chars_seen = state.draft_preview.chars().count();
    state.last_preview_emit_chars_seen = 8;
    state.last_preview_emit_visual_line_count = 1;
    state.last_preview_emit_elapsed_ms = Some(100);

    assert!(!super::should_emit_cli_chat_live_preview(
        &state,
        80,
        Some(110)
    ));
    assert!(super::should_emit_cli_chat_live_preview(
        &state,
        80,
        Some(140)
    ));
}

#[test]
fn preview_emit_cadence_keeps_catch_up_faster_than_smooth() {
    let mut state = super::CliChatLiveSurfaceState::default();
    state.draft_preview =
        "line one wraps quickly\nline two wraps quickly\nline three wraps quickly".to_owned();
    state.total_text_chars_seen = state.draft_preview.chars().count();
    state.last_preview_emit_chars_seen = 8;
    state.last_preview_emit_visual_line_count = 1;
    state.last_preview_emit_elapsed_ms = Some(100);

    assert!(super::should_emit_cli_chat_live_preview(
        &state,
        18,
        Some(118)
    ));
}

#[test]
fn preview_emit_on_wide_widths_no_longer_waits_for_huge_stable_bursts() {
    let state = super::CliChatLiveSurfaceState {
        draft_preview: "hello world this is a stable chunk ".to_owned(),
        last_preview_emit_chars_seen: 8,
        last_preview_emit_visual_line_count: 1,
        last_preview_emit_elapsed_ms: Some(100),
        total_text_chars_seen: 32,
        ..Default::default()
    };

    assert!(super::should_emit_cli_chat_live_preview(
        &state,
        80,
        Some(140)
    ));
}

#[test]
fn delta_commit_boundary_detects_newline_and_structural_tokens() {
    assert!(super::cli_chat_live_delta_has_commit_boundary(
        "line done\n"
    ));
    assert!(super::cli_chat_live_delta_has_commit_boundary("<think>"));
    assert!(super::cli_chat_live_delta_has_commit_boundary("</think>"));
    assert!(super::cli_chat_live_delta_has_commit_boundary("```rust"));
    assert!(!super::cli_chat_live_delta_has_commit_boundary(
        "plain delta"
    ));
}

#[test]
fn compact_observer_rerenders_preview_when_width_changes() {
    let captured_batches = Arc::new(StdMutex::new(Vec::<CliChatLiveSurfaceRenderPayload>::new()));
    let render_sink: CliChatLiveSurfaceSink = {
        let captured_batches = Arc::clone(&captured_batches);
        Arc::new(move |lines| {
            let mut batches = captured_batches
                .lock()
                .expect("captured batches lock should not be poisoned");
            batches.push(lines);
        })
    };
    let render_width = Arc::new(AtomicUsize::new(32));
    let (observer, rerender) =
        build_cli_chat_live_compact_observer_controller(Arc::clone(&render_width), render_sink);

    observer.on_phase(ConversationTurnPhaseEvent::requesting_provider(
        1,
        3,
        Some(96),
    ));
    observer.on_streaming_token(crate::acp::StreamingTokenEvent {
        event_type: "text_delta".to_owned(),
        delta: crate::acp::TokenDelta {
            text: Some("alpha beta gamma delta epsilon".to_owned()),
            tool_call: None,
        },
        index: None,
        elapsed_ms: Some(42),
    });

    render_width.store(12, Ordering::Relaxed);
    rerender();

    let batches = captured_batches
        .lock()
        .expect("captured batches lock should not be poisoned");
    let last_batch = batches.last().expect("rerender batch");
    assert!(last_batch.lines.len() > 1);
    assert!(
        last_batch
            .lines
            .iter()
            .any(|line| line.contains("alpha beta"))
    );
}

#[test]
fn compact_observer_commits_preview_immediately_on_newline_boundary() {
    let captured_batches = Arc::new(StdMutex::new(Vec::<CliChatLiveSurfaceRenderPayload>::new()));
    let render_sink: CliChatLiveSurfaceSink = {
        let captured_batches = Arc::clone(&captured_batches);
        Arc::new(move |lines| {
            let mut batches = captured_batches
                .lock()
                .expect("captured batches lock should not be poisoned");
            batches.push(lines);
        })
    };
    let render_width = Arc::new(AtomicUsize::new(80));
    let (observer, _) =
        build_cli_chat_live_compact_observer_controller(Arc::clone(&render_width), render_sink);

    observer.on_phase(ConversationTurnPhaseEvent::requesting_provider(
        1,
        3,
        Some(32),
    ));
    observer.on_streaming_token(crate::acp::StreamingTokenEvent {
        event_type: "text_delta".to_owned(),
        delta: crate::acp::TokenDelta {
            text: Some("ok\n".to_owned()),
            tool_call: None,
        },
        index: None,
        elapsed_ms: Some(8),
    });

    let batches = captured_batches
        .lock()
        .expect("captured batches lock should not be poisoned");
    let last_batch = batches.last().expect("newline-triggered batch");
    assert!(last_batch.lines.iter().any(|line| line.contains("ok")));
}

#[test]
fn compact_observer_skips_rerender_when_width_change_keeps_same_lines() {
    let captured_batches = Arc::new(StdMutex::new(Vec::<CliChatLiveSurfaceRenderPayload>::new()));
    let render_sink: CliChatLiveSurfaceSink = {
        let captured_batches = Arc::clone(&captured_batches);
        Arc::new(move |lines| {
            let mut batches = captured_batches
                .lock()
                .expect("captured batches lock should not be poisoned");
            batches.push(lines);
        })
    };
    let render_width = Arc::new(AtomicUsize::new(80));
    let (observer, rerender) =
        build_cli_chat_live_compact_observer_controller(Arc::clone(&render_width), render_sink);

    observer.on_phase(ConversationTurnPhaseEvent::requesting_provider(
        1,
        3,
        Some(64),
    ));
    observer.on_streaming_token(crate::acp::StreamingTokenEvent {
        event_type: "text_delta".to_owned(),
        delta: crate::acp::TokenDelta {
            text: Some("short line ".to_owned()),
            tool_call: None,
        },
        index: None,
        elapsed_ms: Some(10),
    });

    let batch_count_before = captured_batches
        .lock()
        .expect("captured batches lock should not be poisoned")
        .len();

    render_width.store(79, Ordering::Relaxed);
    rerender();

    let batch_count_after = captured_batches
        .lock()
        .expect("captured batches lock should not be poisoned")
        .len();

    assert_eq!(batch_count_after, batch_count_before);
}

#[test]
fn live_preview_keeps_command_lines_split_after_label() {
    let lines =
        render_live_preview_segment_lines("Command:\ncargo test --workspace --all-features", 80);

    assert_eq!(
        lines,
        vec![
            "Command:".to_owned(),
            "cargo test --workspace --all-features".to_owned(),
        ]
    );
}

#[test]
fn live_preview_keeps_path_lines_split_after_label() {
    let lines = render_live_preview_segment_lines("Path:\n~/chat/.omx/state.json", 80);

    assert_eq!(
        lines,
        vec!["Path:".to_owned(), "~/chat/.omx/state.json".to_owned(),]
    );
}

#[test]
fn live_preview_keeps_logfmt_lines_out_of_paragraph_reflow() {
    let lines = render_live_preview_segment_lines(
        "prefix\n2026-04-25T11:02:58.547678Z WARN Loong.tools: tool execution failed requested_tool_name=file.read payload_kind=object duration_ms=0",
        96,
    );

    assert_eq!(lines.first().map(String::as_str), Some("prefix"));
    assert!(lines.iter().any(|line| line.contains("WARN Loong.tools:")));
    assert!(!lines[0].contains("WARN Loong.tools:"));
}

#[test]
fn live_preview_preserves_code_like_lines_without_markdown_fence() {
    let lines = render_live_preview_segment_lines(
        "import \"strings\"\nconst (\n    openAIToolCallTypeCustom = \"custom_tool_call\"\n)\nfunc RequiresOpenAIWSV2Continuation(reqBody map[string]any) bool {\n    return false\n}",
        96,
    );

    assert!(lines.iter().any(|line| line == "import \"strings\""));
    assert!(lines.iter().any(|line| line.contains("const (")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("func RequiresOpenAIWSV2Continuation"))
    );
}

#[test]
fn live_preview_preserves_plain_label_like_text_without_label_layout() {
    let lines =
        render_live_preview_segment_lines("source: imported config at ~/.loong/config.toml", 24);

    assert_eq!(
        lines,
        vec![
            "source: imported config".to_owned(),
            "at ~/.loong/config.toml".to_owned(),
        ]
    );
}

#[test]
fn compact_render_preserves_literal_plus_prefix_in_tool_preview_lines() {
    let snapshot = CliChatLiveSurfaceSnapshot {
        phase: ConversationTurnPhase::RunningTools,
        provider_round: Some(1),
        lane: Some(ExecutionLane::Fast),
        tool_call_count: 1,
        message_count: Some(3),
        estimated_tokens: Some(900),
        first_token_latency_ms: None,
        draft_preview: None,
        tools: vec![CliChatLiveToolSnapshot {
            tool_call_id: "call-plus".to_owned(),
            name: Some("edit".to_owned()),
            request_summary: None,
            args: String::new(),
            status: ConversationTurnToolState::Completed,
            detail: Some("ok".to_owned()),
            stdout: empty_output(),
            stderr: empty_output(),
            file_change: Some(CliChatLiveFileChangeView {
                path: "src/lib.rs".to_owned(),
                operation: ToolFileChangeKind::Edit,
                added_lines: 1,
                removed_lines: 0,
                preview: Some("+ added ~/.loong/config.toml".to_owned()),
            }),
            duration_ms: None,
            exit_code: None,
        }],
    };

    let lines = render_cli_chat_live_compact_lines_with_width(&snapshot, 40);
    let joined = lines.join("\n");

    assert!(joined.contains("+ added ~/.loong/config.toml"));
    assert!(!joined.contains("- added ~/.loong/config.toml"));
}

#[test]
fn preview_emit_stride_is_more_responsive_on_narrow_widths() {
    assert_eq!(super::cli_chat_live_preview_emit_stride(4), 8);
    assert_eq!(super::cli_chat_live_preview_emit_stride(12), 12);
    assert_eq!(super::cli_chat_live_preview_emit_stride(20), 20);
    assert_eq!(super::cli_chat_live_preview_emit_stride(80), 24);
}

#[test]
fn preview_buffer_limit_stays_large_even_for_narrow_widths() {
    assert_eq!(
        super::cli_chat_live_preview_char_limit(12),
        super::CLI_CHAT_LIVE_PREVIEW_MAX_BUFFER_CHARS
    );
    assert_eq!(
        super::cli_chat_live_tool_args_char_limit(12),
        super::CLI_CHAT_LIVE_TOOL_ARGS_MAX_BUFFER_CHARS
    );
    assert_eq!(
        super::cli_chat_live_output_char_limit(12),
        super::CLI_CHAT_LIVE_OUTPUT_MAX_BUFFER_CHARS
    );
}
