#![cfg(test)]

use std::collections::BTreeSet;
#[cfg(feature = "tool-shell")]
use std::path::PathBuf;

use loong_app::conversation::turn_engine::TurnResult;
use loong_app::test_support::{FakeProviderBuilder, TurnTestHarness};
#[cfg(feature = "tool-shell")]
use loong_app::tools::runtime_config::BashExecRuntimePolicy;
#[cfg(feature = "tool-shell")]
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
    assert_eq!(turn.tool_intents[0].tool_name, "read");
    assert_eq!(turn.tool_intents[0].args_json, json!({"path": "test.txt"}));
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
            .app_ctx
            .token()
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

fn assert_final_tool_error_contains(result: TurnResult, expected_fragments: &[&str]) {
    #[allow(clippy::wildcard_enum_match_arm)]
    match result {
        TurnResult::FinalText(text) => {
            for expected in expected_fragments {
                assert!(
                    text.contains(expected),
                    "expected `{expected}` in final tool error, got: {text}"
                );
            }
        }
        other => panic!("expected FinalText tool error, got: {other:?}"),
    }
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
#[cfg(feature = "tool-shell")]
async fn integ_shell_exec_echo() {
    let harness = TurnTestHarness::with_tool_config(
        BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
            Capability::NetworkEgress,
        ]),
        ToolRuntimeConfig {
            bash_exec: BashExecRuntimePolicy {
                available: true,
                command: Some(PathBuf::from("bash")),
                ..BashExecRuntimePolicy::default()
            },
            ..ToolRuntimeConfig::default()
        },
    );

    let turn = FakeProviderBuilder::new()
        .with_tool_call("bash", json!({"command": "echo hello"}))
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
#[cfg(feature = "tool-shell")]
async fn integ_shell_exec_blocked_command() {
    let harness = TurnTestHarness::with_tool_config(
        BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
            Capability::NetworkEgress,
        ]),
        ToolRuntimeConfig {
            bash_exec: BashExecRuntimePolicy {
                available: true,
                command: Some(PathBuf::from("bash")),
                ..BashExecRuntimePolicy::default()
            },
            shell_default_mode: loong_app::tools::shell_policy_ext::ShellPolicyDefault::Deny,
            ..ToolRuntimeConfig::default()
        },
    );

    let turn = FakeProviderBuilder::new()
        .with_tool_call("bash", json!({"command": "echo denied_test_command"}))
        .build();
    let result = harness.execute(&turn).await;

    assert_final_tool_error_contains(result, &["kernel_policy_denied", "default-deny"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn integ_file_read_sandbox_rejects_path_escape() {
    let harness = TurnTestHarness::new();

    let turn = FakeProviderBuilder::new()
        .with_tool_call("file.read", json!({"path": "../../../etc/passwd"}))
        .build();
    let result = harness.execute(&turn).await;

    assert_final_tool_error_contains(result, &["kernel_policy_denied", "escapes"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn integ_missing_capability_denies_tool() {
    let harness = TurnTestHarness::with_capabilities(BTreeSet::from([Capability::MemoryRead]));

    let turn = FakeProviderBuilder::new()
        .with_tool_call("file.read", json!({"path": "anything.txt"}))
        .build();
    let result = harness.execute(&turn).await;

    assert_final_tool_error_contains(
        result,
        &["kernel_policy_denied", "invoke_tool", "filesystem_read"],
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg(feature = "tool-shell")]
async fn integ_audit_captures_tool_plane_invocation() {
    let harness = TurnTestHarness::with_tool_config(
        BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
            Capability::NetworkEgress,
        ]),
        ToolRuntimeConfig {
            bash_exec: BashExecRuntimePolicy {
                available: true,
                command: Some(PathBuf::from("bash")),
                ..BashExecRuntimePolicy::default()
            },
            ..ToolRuntimeConfig::default()
        },
    );

    let turn = FakeProviderBuilder::new()
        .with_tool_call("bash", json!({"command": "echo audit"}))
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
            let mentions_current_direct_read_contract = err.contains("direct_read_requires_one_of")
                || err.contains("expected exactly one of `path`, `query`, or `pattern`");
            assert!(
                mentions_repairable_shape && mentions_current_direct_read_contract,
                "expected repairable direct-read guidance, got: {err}"
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

    assert_final_tool_error_contains(result, &["kernel_policy_denied", "filesystem_write"]);
}
