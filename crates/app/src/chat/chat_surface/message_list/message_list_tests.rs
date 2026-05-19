
use super::{
    MessageContent, MessageList, ReadToolRequest, STARTUP_COMPACT_WORDMARK, STARTUP_EYE_FRAMES,
    STARTUP_LOGO_EYE_FRAME_MS, STARTUP_TIP_FADE_MS, STARTUP_TIP_HOLD_MS, STARTUP_WORDMARK,
    ToolStatus, adjust_scroll_start_for_message_boundary, build_assistant_contents,
    content_plain_text, dominant_block_bg, extract_read_tool_request_from_json,
    format_read_request_display, startup_logo_eye_frame_index, startup_logo_eye_style,
    startup_tip_render_state, startup_wordmark_eye_frame,
};
use crate::chat::chat_surface::utils::{
    SURFACE_ACCENT, SURFACE_DIM_GRAY, SURFACE_GRAY, SURFACE_GREEN, SURFACE_RED, SURFACE_TOOL_BG,
    SURFACE_USER_MSG_BG,
};
use crate::test_support::ScopedEnv;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend, style::Color, text::Line};
use std::time::Duration;

#[test]
fn assistant_reply_promotes_diff_fences_to_diff_content() {
    let contents = build_assistant_contents("### Diff\n```diff\n- old\n+ new\n```");

    assert!(matches!(
        contents.first(),
        Some(MessageContent::Diff { title, content })
            if title.as_deref() == Some("Diff") && content.contains("- old") && content.contains("+ new")
    ));
}

#[test]
fn assistant_reply_promotes_tool_activity_callout_to_tool_block() {
    let contents = build_assistant_contents(
        "### Tool activity\n> [completed] read_file (id=call-1)\n> stdout: ok",
    );

    assert!(matches!(
        contents.first(),
        Some(MessageContent::ToolCall { title, status, lines })
            if title.eq_ignore_ascii_case("tool activity")
                && *status == ToolStatus::Success
                && !lines.is_empty()
    ));
}

#[test]
fn tool_activity_renders_without_background_block() {
    let mut list = MessageList::new();
    list.add_assistant_message(
        "### Tool activity\n> [completed] read_file (id=call-1)\n> stdout: ok".to_owned(),
    );

    let rendered = list.get_rendered_lines(40);
    assert!(
        rendered
            .iter()
            .filter(|line| line
                .spans
                .iter()
                .any(|span| { span.content.contains("Closed") || span.content.contains("stdout") }))
            .all(|line| dominant_block_bg(line).is_none())
    );
}

#[test]
fn tool_activity_wraps_long_called_lines_cleanly() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.state_write({\"mode\":\"workflow\",\"current_phase\":\"verification\",\"iteration\":2})".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(42)
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
            .all(|line| crate::presentation::display_width(line) <= 42)
    );
    assert!(rendered.iter().any(|line| line.contains("Called")));
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("demo_mcp.state_write"))
    );
}

#[test]
fn tool_activity_preserves_literal_plus_prefix_in_called_lines() {
    let mut list = MessageList::new();
    list.add_assistant_message(
        "### Tool activity\n> Called + added ~/.loong/config.toml".to_owned(),
    );

    let rendered = list
        .get_rendered_lines(40)
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
        !rendered
            .iter()
            .any(|line| line.contains("• Called - added"))
    );
}

#[test]
fn tool_activity_wraps_arrow_child_lines_cleanly() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> ↳ args {\"path\":\"src/README.md\",\"depth\":2,\"includeHidden\":false}".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(44)
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
            .all(|line| crate::presentation::display_width(line) <= 44)
    );
    assert!(rendered.iter().any(|line| line.contains("↳ args")));
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("path=src/README.md"))
    );
}

#[test]
fn tool_activity_compacts_bracket_status_lines_into_called_closed_flow() {
    let mut list = MessageList::new();
    list.add_assistant_message(
        "### Tool activity\n> [completed] read_file (id=call-1) - ok\n> stdout: done".to_owned(),
    );

    let rendered = list
        .get_rendered_lines(48)
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
            .any(|line| line.contains("• Closed read_file · ok"))
    );
    assert!(!rendered.iter().any(|line| line.contains("(id=call-1)")));
}

#[test]
fn bracket_status_lines_preserve_literal_plus_prefix() {
    let mut list = MessageList::new();
    list.add_assistant_message(
        "### Tool activity\n> [completed] + added ~/.loong/config.toml".to_owned(),
    );

    let rendered = list
        .get_rendered_lines(42)
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
            .any(|line| line.contains("• Closed + added"))
    );
    assert!(
        !rendered
            .iter()
            .any(|line| line.contains("• Closed - added"))
    );
}

#[test]
fn tool_activity_compacts_request_json_into_arrow_child_line() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> request: {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(52)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line.contains("↳ request")));
    assert!(rendered.iter().any(|line| line.contains("query=rust")));
    assert!(rendered.iter().any(|line| line.contains("limit=5")));
}

#[test]
fn tool_activity_compacts_plain_args_without_arrow_prefix() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> args {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(52)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line.contains("↳ args")));
    assert!(rendered.iter().any(|line| line.contains("query=rust")));
    assert!(rendered.iter().any(|line| line.contains("limit=5")));
}

#[test]
fn tool_activity_compacts_plain_args_with_colon_prefix() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> args: {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(52)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line.contains("↳ args")));
    assert!(rendered.iter().any(|line| line.contains("query=rust")));
    assert!(rendered.iter().any(|line| line.contains("limit=5")));
}

#[test]
fn tool_activity_compacts_indented_arrow_args_lines() {
    let mut list = MessageList::new();
    list.add_assistant_message(
        "### Tool activity\n>   ↳ args {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}"
            .to_owned(),
    );

    let rendered = list
        .get_rendered_lines(52)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line.contains("↳ args")));
    assert!(rendered.iter().any(|line| line.contains("query=rust")));
    assert!(rendered.iter().any(|line| line.contains("limit=5")));
}

#[test]
fn plain_called_closed_lines_render_with_bullet_status_flow() {
    let mut list = MessageList::new();
    list.add_assistant_message(
        "### Tool activity\n> Called demo_mcp.search\n> Closed demo_mcp.search · ok".to_owned(),
    );

    let rendered = list
        .get_rendered_lines(52)
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
            .any(|line| line.contains("• Called demo_mcp.search"))
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("• Closed demo_mcp.search · ok"))
    );
}

#[test]
fn plain_approval_and_denied_lines_render_with_bullet_status_flow() {
    let mut list = MessageList::new();
    list.add_assistant_message(
        "### Tool activity\n> Approval demo_mcp.search\n> Denied demo_mcp.search · blocked"
            .to_owned(),
    );

    let rendered = list
        .get_rendered_lines(56)
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
            .any(|line| line.contains("• Approval demo_mcp.search"))
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("• Denied demo_mcp.search · blocked"))
    );
}

#[test]
fn bracket_approval_and_denied_lines_normalize_into_status_flow() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> [needs_approval] read_file (id=call-1) - operator confirmation required\n> [denied] read_file (id=call-1) - blocked".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(64)
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
            .any(|line| line.contains("• Approval read_file · operator confirmation required"))
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("• Denied read_file · blocked"))
    );
}

#[test]
fn tool_activity_dedupes_consecutive_duplicate_approval_and_denied_lines() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Approval demo_mcp.search\n> Approval demo_mcp.search\n> Denied demo_mcp.search · blocked\n> Denied demo_mcp.search · blocked".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(64)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    let approval_count = rendered
        .iter()
        .filter(|line| line.contains("• Approval demo_mcp.search"))
        .count();
    let denied_count = rendered
        .iter()
        .filter(|line| line.contains("• Denied demo_mcp.search · blocked"))
        .count();

    assert_eq!(approval_count, 1);
    assert_eq!(denied_count, 1);
}

#[test]
fn tool_activity_dedupes_consecutive_bracket_approval_lines_with_different_ids() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> [needs_approval] read_file (id=call-1) - operator confirmation required\n> [needs_approval] read_file (id=call-2) - operator confirmation required".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(72)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    let approval_count = rendered
        .iter()
        .filter(|line| line.contains("• Approval read_file · operator confirmation required"))
        .count();

    assert_eq!(approval_count, 1);
}

#[test]
fn tool_activity_dedupes_args_when_request_and_args_match() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> request: {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}\n> args {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(60)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line.contains("↳ request")));
    assert!(
        !rendered
            .iter()
            .any(|line| line.contains("↳ args query=rust"))
    );
}

#[test]
fn tool_activity_resets_request_dedupe_for_new_called_group() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> request: {\"query\":\"rust\",\"limit\":5}\n> Called demo_mcp.search_again\n> request: {\"query\":\"rust\",\"limit\":5}".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(64)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    let request_label_count = rendered
        .iter()
        .filter(|line| line.contains("↳ request"))
        .count();
    let query_count = rendered
        .iter()
        .filter(|line| line.contains("query=rust"))
        .count();

    assert_eq!(request_label_count, 2);
    assert!(query_count >= 2);
}

#[test]
fn tool_activity_dedupes_request_when_matching_args_arrive_first() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> args {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}\n> request: {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(60)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line.contains("↳ args")));
    assert!(
        !rendered
            .iter()
            .any(|line| line.contains("↳ request query=rust"))
    );
}

#[test]
fn tool_activity_dedupes_args_with_colon_prefix_when_request_matches() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> request: {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}\n> args: {\"query\":\"rust\",\"limit\":5,\"scope\":\"repo\"}".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(60)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line.contains("↳ request")));
    assert!(
        !rendered
            .iter()
            .any(|line| line.contains("↳ args query=rust"))
    );
}

#[test]
fn tool_activity_dedupes_consecutive_duplicate_status_lines() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> Called demo_mcp.search\n> Closed demo_mcp.search · ok\n> Closed demo_mcp.search · ok".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(60)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    let called_count = rendered
        .iter()
        .filter(|line| line.contains("• Called demo_mcp.search"))
        .count();
    let closed_count = rendered
        .iter()
        .filter(|line| line.contains("• Closed demo_mcp.search · ok"))
        .count();

    assert_eq!(called_count, 1);
    assert_eq!(closed_count, 1);
}

#[test]
fn tool_activity_dedupes_consecutive_bracket_status_lines_with_different_ids() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> [completed] read_file (id=call-1) - ok\n> [completed] read_file (id=call-2) - ok".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(60)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    let closed_count = rendered
        .iter()
        .filter(|line| line.contains("• Closed read_file · ok"))
        .count();

    assert_eq!(closed_count, 1);
}

#[test]
fn tool_activity_dedupes_bracket_approval_lines_with_different_ids() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> [needs_approval] read_file (id=call-1) - operator confirmation required\n> [needs_approval] read_file (id=call-2) - operator confirmation required".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(72)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    let approval_count = rendered
        .iter()
        .filter(|line| line.contains("• Approval read_file · operator confirmation required"))
        .count();

    assert_eq!(approval_count, 1);
}

#[test]
fn tool_activity_compacts_file_and_metrics_into_arrow_children() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.edit\n> file: edit src/lib.rs (+2 / -1)\n> metrics: 42ms · exit=0".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(52)
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
            .any(|line| line.contains("↳ file edit src/lib.rs (+2 / -1)"))
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("↳ metrics 42ms · exit=0"))
    );
}

#[test]
fn tool_activity_compacts_stdout_into_arrow_children() {
    let mut list = MessageList::new();
    list.add_assistant_message(
        "### Tool activity\n> Called demo_mcp.exec\n> stdout: 2 lines · 22 bytes".to_owned(),
    );

    let rendered = list
        .get_rendered_lines(52)
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
            .any(|line| line.contains("↳ stdout 2 lines · 22 bytes"))
    );
}

#[test]
fn tool_activity_dedupes_consecutive_duplicate_stdout_lines() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.exec\n> stdout: 2 lines · 22 bytes\n> stdout: 2 lines · 22 bytes".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(60)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    let stdout_count = rendered
        .iter()
        .filter(|line| line.contains("↳ stdout 2 lines · 22 bytes"))
        .count();

    assert_eq!(stdout_count, 1);
}

#[test]
fn tool_activity_dedupes_consecutive_duplicate_stderr_lines() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.exec\n> stderr: 1 lines · 12 bytes\n> stderr: 1 lines · 12 bytes".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(60)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    let stderr_count = rendered
        .iter()
        .filter(|line| line.contains("↳ stderr 1 lines · 12 bytes"))
        .count();

    assert_eq!(stderr_count, 1);
}

#[test]
fn tool_activity_compacts_stderr_into_arrow_children() {
    let mut list = MessageList::new();
    list.add_assistant_message(
        "### Tool activity\n> Called demo_mcp.exec\n> stderr: 1 lines · 12 bytes".to_owned(),
    );

    let rendered = list
        .get_rendered_lines(52)
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
            .any(|line| line.contains("↳ stderr 1 lines · 12 bytes"))
    );
}

#[test]
fn tool_activity_dedupes_consecutive_duplicate_metrics_lines() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.exec\n> metrics: 42ms · exit=0\n> metrics: 42ms · exit=0".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(60)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    let metrics_count = rendered
        .iter()
        .filter(|line| line.contains("↳ metrics 42ms · exit=0"))
        .count();

    assert_eq!(metrics_count, 1);
}

#[test]
fn tool_activity_dedupes_consecutive_duplicate_file_lines() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.edit\n> file: edit src/lib.rs (+2 / -1)\n> file: edit src/lib.rs (+2 / -1)".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(64)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    let file_count = rendered
        .iter()
        .filter(|line| line.contains("↳ file edit src/lib.rs (+2 / -1)"))
        .count();

    assert_eq!(file_count, 1);
}

#[test]
fn run_tool_activity_renders_command_and_bounded_stream_preview_card() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> [completed] bash (id=call-1) - ok\n> args: {\"cmd\":\"cargo test --workspace --all-features\"}\n> stdout: first line\n> stdout: second line\n> stdout: third line\n> stdout: fourth line\n> stdout: fifth line\n> stderr: warning: slow\n> metrics: 842ms · exit=0"
                .to_owned(),
        );

    let rendered = list
        .get_rendered_lines(72)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let joined = rendered.join("\n");

    assert!(joined.contains("run cargo test --workspace --all-features ok"));
    assert!(joined.contains("tool: bash"));
    assert!(joined.contains("stdout … +1 earlier lines"));
    assert!(joined.contains("stdout second line"));
    assert!(joined.contains("stdout fifth line"));
    assert!(joined.contains("stderr warning: slow"));
    assert!(joined.contains("metrics 842ms · exit=0"));
}

#[test]
fn search_tool_activity_renders_semantic_preview_card() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called grep\n> args: {\"query\":\"稳定|wenjian|robust|stable\",\"path\":\"~/chat\"}\n> stdout: match one\n> stdout: match two"
                .to_owned(),
        );

    let rendered = list
        .get_rendered_lines(80)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let joined = rendered.join("\n");

    assert!(joined.contains("search \"稳定|wenjian|robust|stable\" in ~/chat"));
    assert!(joined.contains("tool: grep"));
    assert!(joined.contains("stdout match one"));
}

#[test]
fn search_alias_tool_activity_renders_semantic_preview_card() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called search\n> args: {\"query\":\"rust\",\"path\":\"src\"}\n> stdout: match one"
                .to_owned(),
        );

    let rendered = list
        .get_rendered_lines(72)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let joined = rendered.join("\n");

    assert!(joined.contains("search \"rust\" in src"));
    assert!(joined.contains("tool: search"));
    assert!(joined.contains("stdout match one"));
}

#[test]
fn list_tool_activity_renders_semantic_preview_card() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called list_directory\n> args: {\"path\":\"~/chat/.omx\"}\n> stdout: agents\n> stdout: logs"
                .to_owned(),
        );

    let rendered = list
        .get_rendered_lines(72)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let joined = rendered.join("\n");

    assert!(joined.contains("list ~/chat/.omx"));
    assert!(joined.contains("tool: list_directory"));
    assert!(joined.contains("stdout agents"));
}

#[test]
fn glob_tool_activity_renders_semantic_preview_card() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called find_files\n> args: {\"glob\":\"src/**/*.rs\",\"path\":\"~/chat\"}\n> stdout: src/main.rs\n> stdout: src/lib.rs"
                .to_owned(),
        );

    let rendered = list
        .get_rendered_lines(80)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let joined = rendered.join("\n");

    assert!(joined.contains("glob src/**/*.rs in ~/chat"));
    assert!(joined.contains("tool: find_files"));
    assert!(joined.contains("stdout src/main.rs"));
}

#[test]
fn read_tool_preview_recognizes_namespaced_alias_and_nested_request_path() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called filesystem.open_file\n> request: {\"arguments\":{\"path\":\"src/main.rs\",\"offset\":5,\"limit\":2}}\n> stdout: fn main() {}"
                .to_owned(),
        );

    let rendered = list
        .get_rendered_lines(64)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let joined = rendered.join("\n");

    assert!(joined.contains("read src/main.rs:5-6"));
    assert!(joined.contains("preview:"));
    assert!(joined.contains("fn main() {}"));
}

#[test]
fn tool_activity_burst_keeps_unique_request_children_per_called_group() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called demo_mcp.search\n> request: {\"query\":\"rust\",\"limit\":5}\n> Called demo_mcp.search_again\n> request: {\"query\":\"rust\",\"limit\":5}\n> file: edit src/lib.rs (+2 / -1)\n> metrics: 42ms · exit=0".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(64)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    let request_label_count = rendered
        .iter()
        .filter(|line| line.contains("↳ request"))
        .count();

    assert_eq!(request_label_count, 2);
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("↳ file edit src/lib.rs"))
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("↳ metrics 42ms · exit=0"))
    );
}

#[test]
fn provider_error_promotes_to_structured_error_block() {
    let contents = build_assistant_contents(
        "[provider_error] provider returned status 401 for model `gpt-5.4` on attempt 1/3: {\"code\":\"INVALID_API_KEY\",\"message\":\"Invalid API key\"} | provider_failover={\"reason\":\"auth_rejected\",\"stage\":\"status_failure\",\"model\":\"gpt-5.4\",\"attempt\":1,\"max_attempts\":3,\"status_code\":401}",
    );

    assert!(matches!(
        contents.first(),
        Some(MessageContent::Error { title, summary, details })
            if title == "provider error"
                && summary == "401 · gpt-5.4 · 1/3"
                && details.iter().any(|line| {
                    line.contains("INVALID_API_KEY") && line.contains("Invalid API key")
                })
                && details.iter().any(|line| line.contains("auth_rejected"))
    ));
}

#[test]
fn provider_error_rendering_wraps_long_jsonish_details() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "[provider_error] provider returned status 401 for model `gpt-5.4` on attempt 1/3: {\"code\":\"INVALID_API_KEY\",\"message\":\"Invalid API key\"} | provider_failover={\"reason\":\"auth_rejected\",\"stage\":\"status_failure\",\"model\":\"gpt-5.4\",\"attempt\":1,\"max_attempts\":3,\"status_code\":401}".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(36)
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
            .all(|line| crate::presentation::display_width(line) <= 36)
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("[provider error]"))
    );
    assert!(rendered.iter().any(|line| line.contains("INVALID_API_KEY")));
    assert!(rendered.iter().any(|line| line.contains("auth_rejected")));
    assert!(
        !rendered
            .iter()
            .any(|line| line.contains("provider_failover."))
    );
}

#[test]
fn provider_error_renders_title_and_summary_inline_when_width_allows() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "[provider_error] status 401 · gpt-5.4 · attempt 1/3: {\"code\":\"INVALID_API_KEY\",\"message\":\"Invalid API key\"}".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(80)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| {
        line.contains("[provider error]") && line.contains("gpt-5.4") && line.contains("1/3")
    }));
}

#[test]
fn provider_error_summary_preserves_plain_label_like_text() {
    let rendered = super::render_error_block_lines(
        "provider error",
        "source: imported config at ~/.loong/config.toml",
        &[],
        24,
    )
    .into_iter()
    .map(|line| {
        line.spans
            .into_iter()
            .map(|span| span.content.to_string())
            .collect::<String>()
    })
    .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line == "[provider error]"));
    assert!(
        rendered
            .iter()
            .any(|line| line == "source: imported config")
    );
    assert!(
        rendered
            .iter()
            .any(|line| line == "at ~/.loong/config.toml")
    );
    assert!(
        !rendered
            .iter()
            .any(|line| line == "  at ~/.loong/config.toml")
    );
}

#[test]
fn provider_error_detail_preserves_literal_plus_prefix() {
    let rendered = super::render_error_block_lines(
        "provider error",
        "",
        &["+ added ~/.loong/config.toml".to_owned()],
        28,
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
fn provider_error_renders_without_background_block() {
    let mut list = MessageList::new();
    list.add_user_message("hi".to_owned());
    list.add_assistant_message(
            "[provider_error] provider returned status 401 for model `gpt-5.4` on attempt 1/3: {\"code\":\"INVALID_API_KEY\",\"message\":\"Invalid API key\"}".to_owned(),
        );

    let rendered = list.get_rendered_lines(40);
    assert!(
        rendered
            .iter()
            .filter(|line| line
                .spans
                .iter()
                .any(|span| span.content.contains("provider error")))
            .all(|line| dominant_block_bg(line).is_none())
    );
}

#[test]
fn provider_error_rendering_bounds_detail_noise() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "[provider_error] status 500 · model · attempt 1/3: {\"code\":\"SERVER\",\"message\":\"temporary failure\",\"request_id\":\"abc\",\"debug\":\"very long diagnostic payload that should not flood the transcript\"} | route=primary | retry_after=none | trace=hidden".to_owned(),
        );

    let rendered = list
        .get_rendered_lines(44)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let detail_rows = rendered.iter().filter(|line| line.contains("↳")).count();

    assert!(detail_rows <= super::PROVIDER_ERROR_MAX_DETAIL_ITEMS + 1);
    assert!(rendered.iter().any(|line| line.contains("more details")));
    assert!(
        rendered
            .iter()
            .all(|line| crate::presentation::display_width(line) <= 44)
    );
}

#[test]
fn inserts_blank_spacer_between_adjacent_colored_blocks() {
    let mut list = MessageList::new();
    list.add_user_message("hi".to_owned());
    list.add_assistant_message("### Tool activity\n> [completed] read_file".to_owned());

    let rendered = list.get_rendered_lines(40);
    let last_user_block_row = rendered
        .iter()
        .rposition(|line| dominant_block_bg(line) == Some(SURFACE_USER_MSG_BG))
        .expect("user block row");
    let first_tool_block_row = rendered
        .iter()
        .enumerate()
        .find_map(|(idx, line)| {
            line.spans
                .iter()
                .any(|span| span.content.contains("read_file"))
                .then_some(idx)
        })
        .expect("tool block row");

    assert!(first_tool_block_row > last_user_block_row);
    assert!(
        rendered[last_user_block_row + 1..first_tool_block_row]
            .iter()
            .any(|line| line
                .spans
                .iter()
                .all(|span| span.style.bg.is_none() && span.content.trim().is_empty()))
    );
}

#[test]
fn compacted_summary_promotes_to_compaction_block() {
    let text = "[session_local_recall_compacted_window]\nThis compacted checkpoint is session-local recall only.\nCompacted 4 earlier turns\nUser context:\n- earlier ask";
    let contents = build_assistant_contents(text);

    assert!(matches!(
        contents.first(),
        Some(MessageContent::Compaction {
            turn_count,
            summary,
            expanded
        })
            if *turn_count == 4 && summary.contains("User context") && !expanded
    ));
}

#[test]
fn toggle_latest_compaction_flips_expanded_state() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "[session_local_recall_compacted_window]\nThis compacted checkpoint is session-local recall only.\nCompacted 2 earlier turns\nUser context:\n- ask"
                .to_owned(),
        );

    assert!(list.toggle_latest_compaction());

    let Some(message) = list.messages.last() else {
        panic!("expected assistant message");
    };
    assert!(matches!(
        message.contents.first(),
        Some(MessageContent::Compaction { expanded, .. }) if *expanded
    ));
}

#[test]
fn assistant_reply_promotes_markdown_images_to_image_block() {
    let contents =
        build_assistant_contents("Here is the diagram\n\n![plan](https://example.com/plan.png)");

    assert!(matches!(
        contents.get(1),
        Some(MessageContent::Image { alt, url })
            if alt == "plan" && url == "https://example.com/plan.png"
    ));
}

#[test]
fn plain_assistant_reply_preserves_raw_text_without_section_rewrite() {
    let text = "可以。但我需要先看你当前项目里“配置在哪里”。\n\n我可以直接帮你改成 Responses API endpoint，常见位置包括：\n• .env\n• config.*\n• openai / client 初始化代码";
    let contents = build_assistant_contents(text);

    assert!(matches!(
        contents.as_slice(),
        [MessageContent::Markdown(markdown)]
            if markdown == text
    ));
}

#[test]
fn assistant_reply_does_not_leave_internal_tool_result_and_provider_transport_tail_inline() {
    let text = concat!(
        "我明白你的意思。\n\n",
        "我已经核到一件关键事实：当前配置里确实存在一个更宽的 file_root。\n\n",
        "[ok] {\"status\":\"ok\",\"tool\":\"read\",\"tool_call_id\":\"call-1\",\"payload_summary\":\"{\\\"path\\\":\\\"/workspace/demo/crates/daemon/src/lib.rs\\\",\\\"line_start\\\":1,\\\"line_end\\\":50}\",\"payload_chars\":2121,\"payload_truncated\":true}\n",
        "candidate_index=1 candidate_count=1 profile_index=1 profile_count=1 exhausted=true error=provider request failed for model `gpt-5.4` on attempt 3/3: error sending request for url (https://api.tonsof.blue/v1/chat/completions)"
    );

    let contents = build_assistant_contents(text);
    let plain_text = contents
        .iter()
        .filter_map(content_plain_text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        !plain_text.contains("[ok] {\"status\":\"ok\""),
        "raw tool result envelope should not leak into plain transcript markdown: {plain_text}"
    );
    assert!(
        !plain_text.contains("provider request failed for model"),
        "provider transport tail should not remain inline in the plain transcript: {plain_text}"
    );
}

#[test]
fn user_markdown_preserves_plain_label_like_text() {
    let line = Line::from("source: imported config at ~/.loong/config.toml");
    let rendered = super::render_user_markdown_lines(vec![line], 24)
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
            .any(|line| line.contains("source: imported"))
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("~/.loong/config.toml"))
    );
}

#[test]
fn rendered_system_line_preserves_plain_label_like_text() {
    let mut list = MessageList::new();
    list.add_rendered_lines(vec![
        "source: imported config at ~/.loong/config.toml".to_owned(),
    ]);

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
            .any(|line| line.contains("source: imported config"))
    );
    assert!(
        rendered
            .iter()
            .any(|line| line.contains("~/.loong/config.toml"))
    );
}

#[test]
fn tool_stream_preview_preserves_literal_plus_prefix() {
    let preview = super::ToolStreamPreview {
        lines: vec!["+ added ~/.loong/config.toml".to_owned()],
        omitted_count: 0,
        truncated_from_start: false,
    };

    let rendered =
        super::render_tool_stream_preview_section("stdout", &preview, 40, SURFACE_TOOL_BG)
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
            .any(|line| line.contains("+ added ~/.loong/config.toml"))
    );
    assert!(
        !rendered
            .iter()
            .any(|line| line.contains("- added ~/.loong/config.toml"))
    );
}

#[test]
fn read_text_excerpt_preserves_literal_plus_prefix() {
    let rendered = super::render_read_text_excerpt_lines(
        &["+ added ~/.loong/config.toml".to_owned()],
        36,
        SURFACE_TOOL_BG,
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
            .any(|line| line.contains("+ added ~/.loong/config.toml"))
    );
    assert!(
        !rendered
            .iter()
            .any(|line| line.contains("- added ~/.loong/config.toml"))
    );
}

#[test]
fn image_block_renders_bounded_source_and_media_actions() {
    let mut list = MessageList::new();
    list.add_assistant_message(
        "![long diagram](https://example.com/a/very/long/path/that/needs/wrapping/diagram.png)"
            .to_owned(),
    );

    let rendered = list
        .get_rendered_lines(36)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line.contains("[image]")));
    assert!(rendered.iter().any(|line| line.contains("source:")));
    assert!(rendered.iter().any(|line| line.contains("actions:")));
    assert!(rendered.iter().any(|line| line.contains("copy url")));
    assert!(
        !rendered
            .iter()
            .any(|line| line.contains("not available") || line.contains("unavailable"))
    );
    assert!(
        rendered
            .iter()
            .all(|line| crate::presentation::display_width(line) <= 36)
    );
}

#[test]
fn read_tool_image_activity_renders_preview_card() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let path = tempdir.path().join("sample.png");
    let image = image::RgbaImage::from_fn(2, 2, |x, y| {
        if (x + y) % 2 == 0 {
            image::Rgba([255, 0, 0, 255])
        } else {
            image::Rgba([0, 0, 255, 255])
        }
    });
    image.save(path.as_path()).expect("write png");
    let path_text = path.display().to_string();
    let args = serde_json::json!({
        "path": path_text,
    });

    let mut list = MessageList::new();
    list.add_assistant_message(format!(
        "### Tool activity\n> Called read\n> args: {args}\n> stdout: Read image file [image/png]"
    ));

    let rendered = list.get_rendered_lines(72);
    let text = rendered
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(text.iter().any(|line| line.contains("read ")));
    assert!(
        text.join("").contains("sample.png") || text.iter().any(|line| line.contains("read")),
        "read preview should still surface a readable path/card header: {text:#?}"
    );
    assert!(
        text.iter()
            .any(|line| line.contains("Read image file [image/png]"))
    );
    assert!(
        text.iter()
            .any(|line| line.contains("preview: 2×2 · image/png"))
    );
    assert!(text.iter().any(|line| line.contains('▀')));
    assert!(
        rendered
            .iter()
            .any(|line| dominant_block_bg(line) == Some(SURFACE_TOOL_BG))
    );
}

#[test]
fn read_tool_request_json_parser_accepts_windows_escaped_paths() {
    let line = r#"args: {"path":"C:\\Users\\runneradmin\\AppData\\Local\\Temp\\sample.png","offset":10,"limit":3}"#;
    let request = extract_read_tool_request_from_json(line).expect("request");

    assert_eq!(
        request.path,
        r"C:\Users\runneradmin\AppData\Local\Temp\sample.png"
    );
    assert_eq!(request.offset, Some(10));
    assert_eq!(request.limit, Some(3));
}

#[test]
fn read_tool_text_activity_renders_path_and_excerpt_card() {
    let mut list = MessageList::new();
    list.add_assistant_message(
            "### Tool activity\n> Called read_file\n> args: {\"path\":\"docs/notes.md\",\"offset\":10,\"limit\":3}\n> stdout: # Notes\n> stdout: The quick brown fox jumps over the lazy dog."
                .to_owned(),
        );

    let rendered = list.get_rendered_lines(64);
    let text = rendered
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(text.iter().any(|line| line.contains("read ")));
    assert!(text.join("").contains("docs/notes.md:10-12"));
    assert!(text.iter().any(|line| line.contains("Read file")));
    assert!(text.iter().any(|line| line.contains("preview:")));
    assert!(text.iter().any(|line| line.contains("# Notes")));
    assert!(
        text.iter()
            .any(|line| line.contains("quick brown fox jumps"))
    );
    assert!(!text.iter().any(|line| line.contains('▀')));
    assert!(
        rendered
            .iter()
            .any(|line| dominant_block_bg(line) == Some(SURFACE_TOOL_BG))
    );
}

#[test]
fn read_tool_request_display_shortens_home_and_single_line_range() {
    let Some(home) = std::env::var_os("HOME").and_then(|home| home.into_string().ok()) else {
        return;
    };
    if home.is_empty() {
        return;
    }
    let request = ReadToolRequest {
        path: format!("{home}/project/src/lib.rs"),
        offset: Some(42),
        limit: Some(1),
    };

    assert_eq!(
        format_read_request_display(&request),
        "~/project/src/lib.rs:42"
    );
}

#[test]
fn assistant_markdown_table_renders_as_structured_grid() {
    let mut list = MessageList::new();
    list.add_assistant_message(
        "| 指标 | 数值 |\n| --- | --- |\n| 覆盖率 | 68% |\n| 平均响应时间 | 220ms |".to_owned(),
    );

    let rendered = list
        .get_rendered_lines(64)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line.contains("┌")));
    assert!(rendered.iter().any(|line| line.contains("指标")));
    assert!(rendered.iter().any(|line| line.contains("覆盖率")));
    assert!(rendered.iter().any(|line| line.contains("220ms")));
    assert!(!rendered.iter().any(|line| line.contains("| --- |")));
}

#[test]
fn assistant_markdown_code_block_preserves_line_breaks_and_green_styling() {
    let mut list = MessageList::new();
    list.add_assistant_message(
        "```rust
let alpha = 1;
let beta = alpha + 1;
```"
        .to_owned(),
    );

    let rendered = list.get_rendered_lines(48);
    let flattened = rendered
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let alpha_index = flattened
        .iter()
        .position(|line| line.contains("let alpha = 1;"))
        .expect("alpha line");
    let beta_index = flattened
        .iter()
        .position(|line| line.contains("let beta = alpha + 1;"))
        .expect("beta line");

    assert_ne!(alpha_index, beta_index);
    assert!(!flattened[alpha_index].contains("let beta = alpha + 1;"));
    assert!(!flattened[beta_index].contains("let alpha = 1;"));

    let alpha_span = rendered[alpha_index]
        .spans
        .iter()
        .find(|span| span.content.contains("let alpha = 1;"))
        .expect("alpha span");
    let beta_span = rendered[beta_index]
        .spans
        .iter()
        .find(|span| span.content.contains("let beta = alpha + 1;"))
        .expect("beta span");

    assert_eq!(alpha_span.style.fg, Some(SURFACE_GREEN));
    assert_eq!(beta_span.style.fg, Some(SURFACE_GREEN));
}

#[test]
fn assistant_reply_keeps_diff_code_and_tables_consistent_in_one_message() {
    let mut list = MessageList::new();
    list.add_assistant_message(
        "### Patch
```diff
-old value
+new value
```

### Commands
```bash
npm install
npm test
```

| Metric | Value |
| --- | --- |
| coverage | 68% |
| p95 | 220ms |"
            .to_owned(),
    );

    let rendered = list
        .get_rendered_lines(52)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(
            "
",
        );

    assert!(rendered.contains("[Patch]"));
    assert!(rendered.contains("old") && rendered.contains("value"));
    assert!(rendered.contains("new") && rendered.contains("value"));
    assert!(!rendered.contains("```diff"));
    assert!(rendered.contains("```bash"));
    assert!(rendered.contains("npm install"));
    assert!(rendered.contains("npm test"));
    assert!(rendered.contains("┌"));
    assert!(rendered.contains("coverage"));
    assert!(rendered.contains("220ms"));
    assert!(!rendered.contains("| --- |"));
}

#[test]
fn narrow_surface_keeps_code_and_table_blocks_readable() {
    let mut list = MessageList::new();
    list.add_assistant_message(
        "### Commands
```bash
cargo test -p loong-app --lib
```

| Metric | Value |
| --- | --- |
| coverage | 68% |"
            .to_owned(),
    );

    let rendered = list
        .get_rendered_lines(18)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(
            "
",
        );

    assert!(rendered.contains("```bash"));
    assert!(rendered.contains("cargo test"));
    assert!(rendered.contains("┌"));
    assert!(rendered.contains("68%"));
    assert!(rendered.contains("cove"));
}

#[test]
fn user_message_renders_single_bottom_padding_line() {
    let mut list = MessageList::new();
    list.add_user_message("你好".to_owned());

    let rendered = list
        .get_rendered_lines(20)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    let non_empty = rendered
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.trim_end().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(non_empty, vec!["  你好"]);
}

#[test]
fn rendered_lines_are_pre_padded_for_stable_cached_redraws() {
    let mut list = MessageList::new();
    list.add_assistant_message("hello".to_owned());

    let rendered = list
        .get_rendered_lines(18)
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
            .all(|line| crate::presentation::display_width(line) == 18)
    );
}

#[test]
fn mouse_scroll_is_symmetric() {
    let mut list = MessageList::new();
    list.set_scroll_offset_for_test(10);
    list.mouse_step = 3;

    list.handle_mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(list.scroll_offset_for_test(), 7);

    list.handle_mouse(MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(list.scroll_offset_for_test(), 10);
}

#[test]
fn key_scroll_uses_same_direction_model() {
    let mut list = MessageList::new();
    list.set_scroll_offset_for_test(5);
    list.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(list.scroll_offset_for_test(), 4);
    list.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(list.scroll_offset_for_test(), 5);
}

#[test]
fn space_scroll_matches_page_keys() {
    let mut list = MessageList::new();
    list.set_scroll_offset_for_test(20);

    list.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    assert_eq!(list.scroll_offset_for_test(), 8);

    list.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::SHIFT));
    assert_eq!(list.scroll_offset_for_test(), 20);
}

#[test]
fn page_step_uses_viewport_height_minus_overlap() {
    assert_eq!(super::page_step_for_height(1), 1);
    assert_eq!(super::page_step_for_height(2), 1);
    assert_eq!(super::page_step_for_height(8), 6);
    assert_eq!(super::page_step_for_height(20), 18);
}

#[test]
fn mouse_step_tracks_viewport_height_fraction() {
    assert_eq!(super::mouse_step_for_height(1), 1);
    assert_eq!(super::mouse_step_for_height(8), 2);
    assert_eq!(super::mouse_step_for_height(20), 5);
}

#[test]
fn render_updates_page_scroll_step_from_viewport_height() {
    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut list = MessageList::new();
    for idx in 0..8 {
        list.add_assistant_message(format!("line-{idx}"));
    }
    list.set_scroll_offset_for_test(20);

    terminal.draw(|f| list.render(f, f.area())).expect("draw");
    let before = list.scroll_offset_for_test();
    list.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));

    assert_eq!(list.scroll_offset_for_test(), before.saturating_sub(6));
}

#[test]
fn render_updates_mouse_scroll_step_from_viewport_height() {
    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut list = MessageList::new();
    for idx in 0..8 {
        list.add_assistant_message(format!("line-{idx}"));
    }
    list.set_scroll_offset_for_test(10);

    terminal.draw(|f| list.render(f, f.area())).expect("draw");
    list.handle_mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    });

    assert_eq!(list.scroll_offset_for_test(), 8);
}

#[test]
fn resize_preserves_top_visible_line_when_scrolled_up() {
    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut list = MessageList::new();
    for idx in 0..20 {
        list.add_assistant_message(format!("line-{idx}"));
    }
    list.set_scroll_offset_for_test(10);

    terminal.draw(|f| list.render(f, f.area())).expect("draw");
    let before = terminal.backend().buffer().clone();
    let before_area = before.area;
    let before_top_line = (0..before_area.width)
        .map(|x| before[(x, 0)].symbol())
        .collect::<String>();

    terminal.backend_mut().resize(40, 10);
    terminal.draw(|f| list.render(f, f.area())).expect("draw");
    let after = terminal.backend().buffer().clone();
    let after_area = after.area;
    let after_top_line = (0..after_area.width)
        .map(|x| after[(x, 0)].symbol())
        .collect::<String>();

    assert_eq!(before_top_line.trim_end(), after_top_line.trim_end());
}

#[test]
fn new_messages_do_not_teleport_transcript_when_scrolled_up() {
    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut list = MessageList::new();
    for idx in 0..20 {
        list.add_assistant_message(format!("line-{idx}"));
    }
    list.set_scroll_offset_for_test(10);

    terminal.draw(|f| list.render(f, f.area())).expect("draw");
    let before = terminal.backend().buffer().clone();
    let before_area = before.area;
    let before_top_line = (0..before_area.width)
        .map(|x| before[(x, 0)].symbol())
        .collect::<String>();

    list.add_assistant_message("new-tail-line".to_owned());
    terminal.draw(|f| list.render(f, f.area())).expect("draw");
    let after = terminal.backend().buffer().clone();
    let after_area = after.area;
    let after_top_line = (0..after_area.width)
        .map(|x| after[(x, 0)].symbol())
        .collect::<String>();

    assert_eq!(before_top_line.trim_end(), after_top_line.trim_end());
}

#[test]
fn toggling_compaction_does_not_teleport_transcript_when_scrolled_up() {
    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut list = MessageList::new();
    for idx in 0..10 {
        list.add_assistant_message(format!("line-{idx}"));
    }
    list.add_assistant_message(
            "[session_local_recall_compacted_window]\nThis compacted checkpoint is session-local recall only.\nCompacted 2 earlier turns\nUser context:\n- ask"
                .to_owned(),
        );
    list.set_scroll_offset_for_test(6);

    terminal.draw(|f| list.render(f, f.area())).expect("draw");
    let before = terminal.backend().buffer().clone();
    let before_area = before.area;
    let before_top_line = (0..before_area.width)
        .map(|x| before[(x, 0)].symbol())
        .collect::<String>();

    assert!(list.toggle_latest_compaction());
    terminal.draw(|f| list.render(f, f.area())).expect("draw");
    let after = terminal.backend().buffer().clone();
    let after_area = after.area;
    let after_top_line = (0..after_area.width)
        .map(|x| after[(x, 0)].symbol())
        .collect::<String>();

    assert_eq!(before_top_line.trim_end(), after_top_line.trim_end());
}

#[test]
fn new_messages_keep_bottom_anchor_when_following_tail() {
    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut list = MessageList::new();
    for idx in 0..20 {
        list.add_assistant_message(format!("line-{idx}"));
    }

    terminal.draw(|f| list.render(f, f.area())).expect("draw");
    list.add_assistant_message("new-tail-line".to_owned());
    terminal.draw(|f| list.render(f, f.area())).expect("draw");
    let after = terminal.backend().buffer().clone();
    let after_area = after.area;
    let flattened = (0..after_area.height)
        .map(|y| {
            (0..after_area.width)
                .map(|x| after[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(flattened.contains("new-tail-line"));
    assert_eq!(list.scroll_offset_for_test(), 0);
}

#[test]
fn latest_copy_text_prefers_latest_assistant_content() {
    let mut list = MessageList::new();
    list.add_user_message("question".to_owned());
    list.add_assistant_message("answer with details".to_owned());

    assert_eq!(
        list.latest_copy_text().as_deref(),
        Some("answer with details")
    );
}

#[test]
fn export_markdown_includes_roles_and_structured_blocks() {
    let mut list = MessageList::new();
    list.add_user_message("show diff".to_owned());
    list.add_assistant_message("```diff\n- old\n+ new\n```".to_owned());

    let exported = list.export_markdown();

    assert!(exported.contains("## You"));
    assert!(exported.contains("show diff"));
    assert!(exported.contains("## Assistant"));
    assert!(exported.contains("```diff"));
    assert!(exported.contains("+ new"));
}

#[test]
fn width_resize_preserves_bottom_anchor_for_wrapped_tail_content() {
    let backend = TestBackend::new(48, 8);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut list = MessageList::new();
    for idx in 0..8 {
        list.add_assistant_message(format!(
                "line-{idx} keeps a long wrapped transcript chunk stable while the terminal width shrinks"
            ));
    }

    terminal.draw(|f| list.render(f, f.area())).expect("draw");
    terminal.backend_mut().resize(24, 8);
    terminal.draw(|f| list.render(f, f.area())).expect("draw");
    let after = terminal.backend().buffer().clone();
    let after_area = after.area;
    let flattened = (0..after_area.height)
        .map(|y| {
            (0..after_area.width)
                .map(|x| after[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(flattened.contains("line-7"));
    assert_eq!(list.scroll_offset_for_test(), 0);
}

#[test]
fn resize_preserves_bottom_anchor_when_following_tail() {
    let backend = TestBackend::new(40, 8);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut list = MessageList::new();
    for idx in 0..20 {
        list.add_assistant_message(format!("line-{idx}"));
    }

    terminal.draw(|f| list.render(f, f.area())).expect("draw");
    terminal.backend_mut().resize(40, 10);
    terminal.draw(|f| list.render(f, f.area())).expect("draw");
    let after = terminal.backend().buffer().clone();
    let after_area = after.area;
    let flattened = (0..after_area.height)
        .map(|y| {
            (0..after_area.width)
                .map(|x| after[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(flattened.contains("line-19"));
    assert_eq!(list.scroll_offset_for_test(), 0);
}

#[test]
fn assistant_message_keeps_single_trailing_blank_line() {
    let mut list = MessageList::new();
    list.add_assistant_message("Hello.".to_owned());
    list.add_user_message("next".to_owned());

    let rendered = list
        .get_rendered_lines(20)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    let hello_index = rendered
        .iter()
        .position(|line| line.contains("Hello."))
        .expect("assistant line");
    let next_index = rendered
        .iter()
        .position(|line| line.contains("next"))
        .expect("next user line");

    assert_eq!(next_index.saturating_sub(hello_index), 3);
}

#[test]
fn transcript_does_not_start_with_a_forced_blank_row() {
    let mut list = MessageList::new();
    list.add_startup_header("0.1.0".to_owned(), "help".to_owned(), Vec::new());

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

    assert!(rendered.iter().any(|line| {
        line.contains("LOONG")
            || line.contains("░███")
            || line.contains("╭─╮")
            || line.contains("╰─╯")
    }));
    assert!(
        rendered
            .iter()
            .find(|line| !line.trim().is_empty())
            .is_some_and(|line| {
                line.contains("LOONG")
                    || line.contains("░███")
                    || line.contains("╭─╮")
                    || line.contains("╰─╯")
            })
    );
}

#[test]
fn clear_transcript_removes_messages_and_resets_scroll_state() {
    let mut list = MessageList::new();
    list.add_startup_header("0.1.0".to_owned(), "help".to_owned(), Vec::new());
    list.add_user_message("hello".to_owned());
    list.add_rendered_lines(vec!["system card".to_owned()]);
    let _ = list.get_rendered_lines(80);
    list.set_scroll_offset_for_test(6);
    list.set_last_scroll_start_for_test(2);
    list.set_snap_scroll_on_next_render_for_test(false);

    list.clear_transcript();

    assert!(list.messages.is_empty());
    assert_eq!(list.scroll_offset_for_test(), 0);
    assert_eq!(list.last_scroll_start_for_test(), 0);
    assert!(list.is_following_tail());
    assert!(list.snap_scroll_on_next_render_for_test());
    assert!(list.render_cache.is_none());
}

#[test]
fn startup_header_wraps_long_section_values_to_viewport_width() {
    let mut list = MessageList::new();
    list.add_startup_header(
        "0.1.0".to_owned(),
        "help".to_owned(),
        vec![("Skills".to_owned(), vec!["12".to_owned()])],
    );

    let rendered = list
        .get_rendered_lines(28)
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
            .all(|line| crate::presentation::display_width(line) <= 28)
    );
    assert!(rendered.iter().any(|line| line.contains("Skills (12)")));
}

#[test]
fn startup_status_markers_use_state_colors() {
    let mut list = MessageList::new();
    list.add_startup_header(
        "0.1.0".to_owned(),
        "help".to_owned(),
        vec![
            ("Skills".to_owned(), vec!["0".to_owned()]),
            ("MCP".to_owned(), vec!["2".to_owned()]),
        ],
    );

    let rendered = list.get_rendered_lines(80);
    let has_missing_marker = rendered
        .iter()
        .flat_map(|line| line.spans.iter())
        .any(|span| span.content.as_ref() == "✗" && span.style.fg == Some(SURFACE_RED));
    let has_ready_marker = rendered
        .iter()
        .flat_map(|line| line.spans.iter())
        .any(|span| span.content.as_ref() == "✓" && span.style.fg == Some(SURFACE_GREEN));

    assert!(has_missing_marker);
    assert!(has_ready_marker);
}

#[test]
fn startup_tip_keeps_blank_row_below_tip() {
    let mut list = MessageList::new();
    list.add_startup_header_with_tips(
        "0.1.0".to_owned(),
        "fallback".to_owned(),
        Vec::new(),
        vec!["rotating tip".to_owned()],
    );

    let rendered = list
        .get_rendered_lines(80)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let tip_index = rendered
        .iter()
        .position(|line| line.contains("rotating tip"))
        .expect("startup tip line");

    assert!(
        rendered
            .get(tip_index + 1)
            .is_some_and(|line| line.trim().is_empty())
    );
}

#[test]
fn startup_wordmarks_match_brand_art() {
    assert_eq!(
        STARTUP_WORDMARK,
        &[
            "░███░         ░████████░    ░████████░   ░█████████░    ░████████░",
            "░███░        ░███    ███░  ░███    ███░  ░███    ███░  ░███    ███░",
            "░███░        ░███    ███░  ░███    ███░  ░███    ███░  ░███",
            "░███░        ░███    ███░  ░███    ███░  ░███    ███░  ░███  █████░",
            "░███░        ░███    ███░  ░███    ███░  ░███    ███░  ░███    ███░",
            "░███░        ░███    ███░  ░███    ███░  ░███    ███░  ░███    ███░",
            "░██████████   ░████████░    ░████████░   ░███    ███░   ░████████░",
        ]
    );
    assert_eq!(
        STARTUP_COMPACT_WORDMARK,
        &[
            "╷  ╭─╮╭─╮╭╮╷╭─╴",
            "│  │ ││ ││╰┤│╶╮",
            "╰─╴╰─╯╰─╯╵ ╵╰─╯",
            "",
            "",
            "",
        ]
    );
}

#[test]
fn startup_wordmark_eye_frames_animate_the_two_o_letters() {
    assert_eq!(startup_logo_eye_frame_index(Duration::ZERO), 0);
    assert_eq!(STARTUP_EYE_FRAMES.len(), 60);

    let first_glance = startup_wordmark_eye_frame(0).join(
        "
",
    );
    let upper_wash = startup_wordmark_eye_frame(6).join(
        "
",
    );
    let far_right = startup_wordmark_eye_frame(16).join(
        "
",
    );
    let lower_glance = startup_wordmark_eye_frame(26).join(
        "
",
    );
    let shimmer = startup_wordmark_eye_frame(32).join(
        "
",
    );
    let vertical_sweep = startup_wordmark_eye_frame(40).join(
        "
",
    );

    assert!(first_glance.contains("░███ █  ███░  ░███ █  ███░"));
    assert!(upper_wash.contains("░███▓▓▓▓███░  ░███▓▓▓▓███░"));
    assert!(far_right.contains("░███  █████░  ░███  █████░"));
    assert!(lower_glance.contains("░███ █  ███░  ░███ █  ███░"));
    assert!(shimmer.contains("░███▒▒▒▒███░  ░███▒▒▒▒███░"));
    assert!(vertical_sweep.contains("░███ ▂  ███░  ░███ ▂  ███░"));
    assert_ne!(
        first_glance,
        STARTUP_WORDMARK.join(
            "
"
        )
    );
    assert_ne!(first_glance, far_right);
}

#[test]
fn startup_wordmark_eye_frames_keep_fixed_geometry() {
    for frame_index in 0..STARTUP_EYE_FRAMES.len() {
        let frame = startup_wordmark_eye_frame(frame_index);
        assert_eq!(frame.len(), STARTUP_WORDMARK.len());
        for (line, base_line) in frame.iter().zip(STARTUP_WORDMARK.iter()) {
            assert_eq!(
                crate::presentation::display_width(line),
                crate::presentation::display_width(base_line),
                "{line}"
            );
        }
    }
}

#[test]
fn startup_eye_shadow_blocks_use_layered_intensity() {
    assert_eq!(startup_logo_eye_style('░').fg, Some(SURFACE_DIM_GRAY));
    assert_eq!(startup_logo_eye_style('▒').fg, Some(SURFACE_GRAY));
    assert_eq!(startup_logo_eye_style('▓').fg, Some(SURFACE_ACCENT));
    assert_eq!(startup_logo_eye_style('█').fg, Some(Color::White));
}

#[test]
fn startup_header_uses_full_logo_when_viewport_is_wide() {
    let mut list = MessageList::new();
    list.add_startup_header("0.1.0".to_owned(), "help".to_owned(), Vec::new());

    let rendered = list
        .get_rendered_lines(120)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("░███░         ░████████░"));
    assert!(rendered.contains("░██████████   ░████████░"));
    assert!(!rendered.contains("╷  ╭─╮╭─╮╭╮╷╭─╴"));
}

#[test]
fn startup_header_uses_compact_logo_when_viewport_is_narrow() {
    let mut list = MessageList::new();
    list.add_startup_header("0.1.0".to_owned(), "help".to_owned(), Vec::new());

    let rendered = list
        .get_rendered_lines(24)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("╷  ╭─╮╭─╮╭╮╷╭─╴"));
    assert!(!rendered.contains("░████████░"));
}

#[test]
fn startup_header_animation_stays_active_after_first_message() {
    let mut env = ScopedEnv::new();
    env.remove("LOONG_TUI_REDUCED_MOTION");
    env.set("TERM", "xterm-256color");

    let mut list = MessageList::new();
    list.add_startup_header_with_tips(
        "0.1.0".to_owned(),
        "help".to_owned(),
        Vec::new(),
        vec!["first tip".to_owned(), "second tip".to_owned()],
    );
    list.add_user_message("hi".to_owned());
    list.add_assistant_message("hello".to_owned());

    assert!(list.startup_animation_active());
    list.last_startup_animation_signature = None;
    list.rewind_startup_animation_for_test(Duration::from_millis(
        STARTUP_LOGO_EYE_FRAME_MS.saturating_add(10),
    ));
    assert!(
        list.refresh_startup_animation(),
        "startup header should keep animating while it remains visible"
    );
}

#[test]
fn startup_tip_animation_fades_to_next_tip_after_cycle_boundary() {
    let tips = vec!["first tip".to_owned(), "second tip".to_owned()];
    let elapsed = Duration::from_millis(
        STARTUP_TIP_HOLD_MS + STARTUP_TIP_FADE_MS + (STARTUP_TIP_FADE_MS / 2),
    );

    let render_state =
        startup_tip_render_state(tips.as_slice(), elapsed).expect("startup tip render state");

    if super::reduced_motion_enabled() {
        assert!(render_state.text.contains("first tip"));
        return;
    }

    assert!(render_state.text.contains("second tip"));
    assert_ne!(render_state.text_color, Color::White);
    assert_ne!(render_state.bullet_color, SURFACE_ACCENT);
}

#[test]
fn startup_header_wraps_version_and_tutorial_to_viewport_width() {
    let mut list = MessageList::new();
    list.add_startup_header(
        "v0.1.0-alpha.3".to_owned(),
        "escape interrupt · : deck · / commands · ctrl+o compaction".to_owned(),
        Vec::new(),
    );

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
            .all(|line| crate::presentation::display_width(line) <= 24)
    );
    assert!(rendered.iter().any(|line| line.contains("0.1.0-alpha.3")));
    assert!(rendered.iter().any(|line| line.contains("compaction")));
}

#[test]
fn startup_header_version_line_does_not_duplicate_v_prefix() {
    let mut list = MessageList::new();
    list.add_startup_header("v0.1.0-alpha.3".to_owned(), "help".to_owned(), Vec::new());

    let rendered = list
        .get_rendered_lines(40)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("v0.1.0-alpha.3"));
    assert!(!rendered.contains("vv0.1.0-alpha.3"));
}

#[test]
fn startup_header_current_build_version_line_does_not_duplicate_v_prefix() {
    let version = crate::presentation::BuildVersionInfo::current().render_version_line();
    let mut list = MessageList::new();
    list.add_startup_header(version.clone(), "help".to_owned(), Vec::new());

    let rendered = list
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

    assert!(rendered.contains(version.as_str()));
    assert!(!rendered.contains(format!("v{version}").as_str()));
}

#[test]
fn assistant_messages_trim_renderer_blank_edges_and_keep_two_space_indent() {
    let mut list = MessageList::new();
    list.add_assistant_message("Hello.".to_owned());

    let rendered = list
        .get_rendered_lines(20)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    let hello_index = rendered
        .iter()
        .position(|line| line.contains("Hello."))
        .expect("assistant line");
    assert!(rendered[hello_index].starts_with("  Hello."));
    assert!(
        rendered
            .get(hello_index + 1)
            .is_some_and(|line| line.trim().is_empty())
    );
}

#[test]
fn assistant_inline_bullet_runs_split_into_separate_lines() {
    let mut list = MessageList::new();
    list.add_assistant_message("• first item • second item • third item".to_owned());

    let rendered = list
        .get_rendered_lines(36)
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(rendered.iter().any(|line| line.contains("• first item")));
    assert!(rendered.iter().any(|line| line.contains("• second item")));
    assert!(rendered.iter().any(|line| line.contains("• third item")));
}

#[test]
fn rendered_system_activity_headline_uses_colored_spans() {
    let mut list = MessageList::new();
    list.add_rendered_lines(vec!["• Ran cargo test -p loong-app".to_owned()]);

    let rendered = list.get_rendered_lines(48);
    let line = rendered
        .iter()
        .find(|line| {
            line.spans
                .iter()
                .any(|span| span.content.contains("cargo test"))
        })
        .expect("system activity line");

    assert_eq!(line.spans[0].content.as_ref(), "• ");
    assert_eq!(line.spans[0].style.fg, Some(SURFACE_GREEN));
    assert_eq!(line.spans[1].content.as_ref(), "Ran ");
    assert_eq!(line.spans[1].style.fg, Some(SURFACE_ACCENT));
}

#[test]
fn rendered_system_activity_child_uses_tree_and_action_styling() {
    let mut list = MessageList::new();
    list.add_rendered_lines(vec!["  └ Read app.rs".to_owned()]);

    let rendered = list.get_rendered_lines(32);
    let line = rendered
        .iter()
        .find(|line| {
            line.spans
                .iter()
                .any(|span| span.content.contains("app.rs"))
        })
        .expect("system child line");

    assert_eq!(line.spans[0].content.as_ref(), "  ");
    assert_eq!(line.spans[1].content.as_ref(), "└ ");
    assert_eq!(line.spans[1].style.fg, Some(SURFACE_GRAY));
    assert_eq!(line.spans[2].content.as_ref(), "Read ");
    assert_eq!(line.spans[2].style.fg, Some(SURFACE_ACCENT));
}

#[test]
fn scroll_start_snaps_to_user_block_boundary() {
    let mut list = MessageList::new();
    list.add_startup_header("0.1.0".to_owned(), "help".to_owned(), Vec::new());
    list.add_user_message("hello world".to_owned());
    list.add_assistant_message("reply".to_owned());

    let rendered = list.get_rendered_lines(24);
    let first_user_bg = rendered
        .iter()
        .position(|line| dominant_block_bg(line).is_some())
        .expect("user block should be present");
    let inside_user_block = first_user_bg + 1;

    let snapped = adjust_scroll_start_for_message_boundary(&rendered, inside_user_block);
    assert_eq!(snapped, first_user_bg + 1);
}
