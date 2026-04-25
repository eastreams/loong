use std::collections::BTreeSet;

use loong_app::conversation::turn_engine::TurnResult;
use loong_app::test_support::{FakeProviderBuilder, TurnTestHarness};
use loong_app::tools::runtime_config::ToolRuntimeConfig;
use loong_contracts::Capability;
use serde_json::json;

#[test]
fn fake_provider_builder_text_only() {
    let turn = FakeProviderBuilder::new().with_text("hello world").build();
    assert_eq!(turn.assistant_text, "hello world");
    assert!(turn.tool_intents.is_empty());
}

#[test]
fn fake_provider_builder_with_tool_call() {
    let turn = FakeProviderBuilder::new()
        .with_text("checking file")
        .with_tool_call("file.read", json!({"path": "test.txt"}))
        .build();
    assert_eq!(turn.assistant_text, "checking file");
    assert_eq!(turn.tool_intents.len(), 1);
    // Discoverable tools are bridged through tool.invoke with a lease.
    assert_eq!(turn.tool_intents[0].tool_name, "tool.invoke");
    assert_eq!(
        turn.tool_intents[0].args_json["tool_id"],
        json!("file.read")
    );
    assert_eq!(
        turn.tool_intents[0].args_json["arguments"],
        json!({"path": "test.txt"})
    );
    assert!(!turn.tool_intents[0].tool_call_id.is_empty());
}

#[test]
fn fake_provider_builder_unique_tool_call_ids() {
    let turn = FakeProviderBuilder::new()
        .with_tool_call("file.read", json!({"path": "a.txt"}))
        .with_tool_call("file.read", json!({"path": "b.txt"}))
        .build();
    assert_eq!(turn.tool_intents.len(), 2);
    assert_ne!(
        turn.tool_intents[0].tool_call_id,
        turn.tool_intents[1].tool_call_id
    );
}

#[test]
fn harness_builds_with_invoke_tool_capability() {
    let harness = TurnTestHarness::new();
    assert!(
        harness
            .kernel_ctx
            .token
            .allowed_capabilities
            .contains(&Capability::InvokeTool)
    );
    assert!(harness.temp_dir.exists());
}

#[test]
fn harness_builder_preserves_custom_capabilities_and_tool_config() {
    let harness = TurnTestHarness::builder()
        .capabilities(BTreeSet::from([Capability::MemoryRead]))
        .tool_config(ToolRuntimeConfig {
            shell_allow: BTreeSet::from(["echo".to_owned()]),
            ..ToolRuntimeConfig::default()
        })
        .build();

    assert!(
        harness
            .kernel_ctx
            .token
            .allowed_capabilities
            .contains(&Capability::MemoryRead)
    );
    assert!(
        !harness
            .kernel_ctx
            .token
            .allowed_capabilities
            .contains(&Capability::InvokeTool)
    );
    assert!(harness.temp_dir.exists());
}

#[test]
fn harness_temp_dirs_are_unique() {
    let h1 = TurnTestHarness::new();
    let h2 = TurnTestHarness::new();
    assert_ne!(h1.temp_dir, h2.temp_dir);
}

// ── Real-execution integration tests ──────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn integ_file_read_returns_real_content() {
    let harness = TurnTestHarness::new();
    std::fs::write(
        harness.temp_dir.join("greeting.txt"),
        "hello from integration test",
    )
    .expect("seed file");

    let turn = FakeProviderBuilder::new()
        .with_tool_call("file.read", json!({"path": "greeting.txt"}))
        .build();
    let result = harness.execute(&turn).await;

    #[allow(clippy::wildcard_enum_match_arm)]
    match result {
        TurnResult::FinalText(text) => {
            assert!(
                text.contains("hello from integration test"),
                "expected file content in output, got: {text}"
            );
        }
        other => panic!("expected FinalText, got: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn integ_file_write_then_read_round_trip() {
    let harness = TurnTestHarness::new();

    // Write
    let write_turn = FakeProviderBuilder::new()
        .with_tool_call(
            "file.write",
            json!({"path": "round-trip.txt", "content": "written by tool"}),
        )
        .build();
    let write_result = harness.execute(&write_turn).await;
    #[allow(clippy::wildcard_enum_match_arm)]
    match &write_result {
        TurnResult::FinalText(text) => {
            assert!(
                text.contains("bytes_written") || text.contains("round-trip.txt"),
                "expected write metadata in write result, got: {text}"
            );
        }
        other => panic!("expected FinalText for write, got: {other:?}"),
    }

    // Read back
    let read_turn = FakeProviderBuilder::new()
        .with_tool_call("file.read", json!({"path": "round-trip.txt"}))
        .build();
    let read_result = harness.execute(&read_turn).await;
    #[allow(clippy::wildcard_enum_match_arm)]
    match read_result {
        TurnResult::FinalText(text) => {
            assert!(
                text.contains("written by tool"),
                "expected written content in read result, got: {text}"
            );
        }
        other => panic!("expected FinalText for read, got: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn integ_shell_exec_echo() {
    let harness = TurnTestHarness::with_tool_config(
        BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
        ]),
        ToolRuntimeConfig {
            shell_allow: BTreeSet::from(["echo".to_owned()]),
            ..ToolRuntimeConfig::default()
        },
    );

    let turn = FakeProviderBuilder::new()
        .with_tool_call("shell.exec", json!({"command": "echo", "args": ["hello"]}))
        .build();
    let result = harness.execute(&turn).await;

    #[allow(clippy::wildcard_enum_match_arm)]
    match result {
        TurnResult::FinalText(text) => {
            assert!(
                text.contains("hello"),
                "expected 'hello' in output, got: {text}"
            );
        }
        other => panic!("expected FinalText, got: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn integ_shell_exec_blocked_command() {
    let harness = TurnTestHarness::with_tool_config(
        BTreeSet::from([Capability::InvokeTool]),
        ToolRuntimeConfig {
            shell_deny: BTreeSet::from(["echo".to_owned()]),
            ..ToolRuntimeConfig::default()
        },
    );

    let turn = FakeProviderBuilder::new()
        .with_tool_call(
            "shell.exec",
            json!({"command": "echo", "args": ["denied_test_command"]}),
        )
        .build();
    let result = harness.execute(&turn).await;

    #[allow(clippy::wildcard_enum_match_arm)]
    match result {
        TurnResult::ToolDenied(err) => {
            assert!(
                err.contains("blocked by shell policy"),
                "expected policy-block reason, got: {err}"
            );
        }
        other => panic!("expected ToolDenied with policy reason, got: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn integ_file_read_sandbox_rejects_path_escape() {
    let harness = TurnTestHarness::new();

    let turn = FakeProviderBuilder::new()
        .with_tool_call("file.read", json!({"path": "../../../etc/passwd"}))
        .build();
    let result = harness.execute(&turn).await;

    #[allow(clippy::wildcard_enum_match_arm)]
    match result {
        TurnResult::ToolDenied(err) => {
            assert!(
                err.contains("escapes"),
                "expected 'escapes' in error, got: {err}"
            );
        }
        other => panic!("expected ToolDenied with 'escapes', got: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn integ_missing_capability_denies_tool() {
    let harness = TurnTestHarness::with_capabilities(BTreeSet::from([Capability::MemoryRead]));

    let turn = FakeProviderBuilder::new()
        .with_tool_call("file.read", json!({"path": "anything.txt"}))
        .build();
    let result = harness.execute(&turn).await;

    #[allow(clippy::wildcard_enum_match_arm)]
    match result {
        TurnResult::ToolDenied(reason) => {
            let lower = reason.to_lowercase();
            assert!(
                lower.contains("capability") || lower.contains("denied"),
                "expected capability/denied in reason, got: {reason}"
            );
        }
        other => panic!("expected ToolDenied, got: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn integ_audit_captures_tool_plane_invocation() {
    let harness = TurnTestHarness::with_tool_config(
        BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
        ]),
        ToolRuntimeConfig {
            shell_allow: BTreeSet::from(["echo".to_owned()]),
            ..ToolRuntimeConfig::default()
        },
    );

    let turn = FakeProviderBuilder::new()
        .with_tool_call("shell.exec", json!({"command": "echo", "args": ["audit"]}))
        .build();
    let result = harness.execute(&turn).await;
    assert!(
        matches!(result, TurnResult::FinalText(_)),
        "expected FinalText, got: {result:?}"
    );

    let events = harness.audit.snapshot();
    let has_tool_plane = events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::PlaneInvoked {
                plane: loong_contracts::ExecutionPlane::Tool,
                ..
            }
        )
    });
    assert!(
        has_tool_plane,
        "audit should contain PlaneInvoked{{Tool}} event, got: {:?}",
        events
            .iter()
            .map(|e| format!("{:?}", e.kind))
            .collect::<Vec<_>>()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn integ_malformed_tool_args_returns_error() {
    let harness = TurnTestHarness::new();

    let turn = FakeProviderBuilder::new()
        .with_tool_call("file.read", json!("not an object"))
        .build();
    let result = harness.execute(&turn).await;

    #[allow(clippy::wildcard_enum_match_arm)]
    match result {
        TurnResult::ToolError(err) => {
            let mentions_repairable_shape = err.contains("tool input needs repair");
            let mentions_object_requirement =
                err.contains("must be an object") || err.contains("must be object");
            assert!(
                mentions_repairable_shape && mentions_object_requirement,
                "expected repairable object-shape error, got: {err}"
            );
        }
        other => {
            panic!("expected ToolError with repairable object-shape guidance, got: {other:?}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::wildcard_enum_match_arm)]
async fn integ_file_write_denied_without_capability() {
    let harness = TurnTestHarness::with_capabilities(BTreeSet::from([Capability::InvokeTool]));

    let turn = FakeProviderBuilder::new()
        .with_tool_call(
            "file.write",
            json!({"path": "test.txt", "content": "hello"}),
        )
        .build();
    let result = harness.execute(&turn).await;

    match result {
        TurnResult::ToolDenied(err) => {
            assert!(
                err.contains("FilesystemWrite"),
                "expected 'FilesystemWrite' in reason, got: {err}"
            );
        }
        other => {
            panic!("expected ToolDenied with FilesystemWrite, got: {other:?}")
        }
    }
}
