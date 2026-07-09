use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use loong_contracts::{
    Capability, ExecutionPlane, ExecutionRoute, HarnessKind, PlaneTier, ToolCoreRequest,
};
use loong_kernel::{InMemoryAuditSink, Kernel, NoopAuditSink, SystemClock, VerticalPackManifest};
use serde_json::json;

use super::*;
use crate::context::AppContextFactory;
use crate::tools::runtime_config::ToolRuntimeConfig;
use crate::tools::runtime_events::{
    ToolFileChangeKind, ToolRuntimeEvent, ToolRuntimeEventSink, with_tool_runtime_event_sink,
};

#[derive(Default)]
struct RecordingRuntimeSink {
    events: Mutex<Vec<ToolRuntimeEvent>>,
}

fn lock_runtime_events(
    sink: &RecordingRuntimeSink,
) -> std::sync::MutexGuard<'_, Vec<ToolRuntimeEvent>> {
    match sink.events.lock() {
        Ok(events) => events,
        Err(poisoned_events) => poisoned_events.into_inner(),
    }
}

impl ToolRuntimeEventSink for RecordingRuntimeSink {
    fn emit(&self, event: ToolRuntimeEvent) {
        let mut events = lock_runtime_events(self);
        events.push(event);
    }
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
}

fn test_pack() -> VerticalPackManifest {
    VerticalPackManifest {
        pack_id: "test-pack".to_owned(),
        domain: "test".to_owned(),
        version: "0.1.0".to_owned(),
        default_route: ExecutionRoute {
            harness_kind: HarnessKind::EmbeddedPi,
            adapter: None,
        },
        allowed_connectors: BTreeSet::new(),
        granted_capabilities: BTreeSet::from([
            Capability::InvokeTool,
            Capability::FilesystemRead,
            Capability::FilesystemWrite,
        ]),
        metadata: Default::default(),
    }
}

async fn execute_file_read_with_test_context(
    request: ToolCoreRequest,
    config: &ToolRuntimeConfig,
) -> Result<ToolCoreOutcome, String> {
    let mut kernel =
        Kernel::<AppContextFactory>::with_runtime(Arc::new(SystemClock), Arc::new(NoopAuditSink));
    let pack = Arc::new(test_pack());
    kernel
        .register_pack((*pack).clone())
        .map_err(|error| format!("register pack failed: {error}"))?;
    let token = kernel
        .issue_token("test-pack", "test-agent", 60)
        .map_err(|error| format!("issue token failed: {error}"))?;
    let kernel = Arc::new(kernel);
    let kernel_ctx = crate::KernelContext {
        kernel: kernel.clone(),
        pack,
        token,
        tool_runtime_config: config.clone(),
    };
    let policy_context =
        kernel_ctx.execution_context(ExecutionPlane::Tool, PlaneTier::Core, None, config)?;
    let _ = config;
    loong_tools::file::execute_file_read_tool_with_context::<AppContextFactory>(
        request,
        &policy_context,
    )
    .await
}

async fn execute_file_read_via_kernel_tool_registry(
    request: ToolCoreRequest,
    config: &ToolRuntimeConfig,
) -> Result<(ToolCoreOutcome, Arc<InMemoryAuditSink>), loong_kernel::KernelError> {
    let audit = Arc::new(InMemoryAuditSink::default());
    let mut kernel =
        Kernel::<AppContextFactory>::with_runtime(Arc::new(SystemClock), audit.clone());
    let pack = Arc::new(test_pack());
    kernel.register_pack((*pack).clone())?;
    crate::tools::register_kernel_tools(
        &mut kernel,
        config.clone(),
        crate::config::ObservabilityConfig::runtime_default(),
    )?;
    let token = kernel.issue_token("test-pack", "test-agent", 60)?;
    let kernel_ctx = crate::KernelContext {
        kernel: Arc::new(kernel),
        pack,
        token,
        tool_runtime_config: config.clone(),
    };
    let outcome = crate::tools::execute_kernel_tool_request(&kernel_ctx, request, false).await?;
    Ok((outcome, audit))
}

#[cfg(unix)]
#[test]
fn resolve_safe_file_path_rejects_symlink_escape_on_read() {
    let base = unique_temp_dir("loong-file-read");
    let root = base.join("root");
    let outside = base.join("outside");
    fs::create_dir_all(&root).expect("create root");
    fs::create_dir_all(&outside).expect("create outside");

    let outside_file = outside.join("secret.txt");
    fs::write(&outside_file, "secret").expect("write outside file");
    let link = root.join("secret-link");
    assert!(create_symlink(&outside_file, &link).is_ok());

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let error =
        resolve_safe_file_path_with_config("secret-link", &config).expect_err("escape denied");

    assert!(error.starts_with("policy_denied: "));
    assert!(error.contains("escapes configured file root"));
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn file_read_supports_line_window_pagination() {
    let base = unique_temp_dir("loongclaw-file-read-window");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("notes.txt"), "alpha\nbeta\ngamma\ndelta").expect("write fixture");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "file.read".to_owned(),
        payload: json!({
            "path": "notes.txt",
            "offset": 2,
            "limit": 2
        }),
    };

    let outcome = execute_file_read_with_test_context(request, &config)
        .await
        .expect("file.read window should succeed");

    assert_eq!(outcome.payload["content"], json!("beta\ngamma"));
    assert_eq!(outcome.payload["line_start"], json!(2));
    assert_eq!(outcome.payload["line_end"], json!(3));
    assert_eq!(outcome.payload["total_lines"], json!(4));
    assert_eq!(outcome.payload["next_offset"], json!(4));
    assert_eq!(outcome.payload["truncated"], json!(false));
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn kernel_routed_file_read_uses_typed_tool_registry() {
    let base = unique_temp_dir("loong-file-read-typed-registry");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("notes.txt"), "alpha\nbeta\ngamma").expect("write fixture");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "file.read".to_owned(),
        payload: json!({
            "path": "notes.txt",
            "offset": 2,
            "limit": 1
        }),
    };

    let (outcome, audit) = execute_file_read_via_kernel_tool_registry(request, &config)
        .await
        .expect("file.read should execute through typed registry");

    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["content"], json!("beta"));
    assert_eq!(outcome.payload["line_start"], json!(2));
    assert_eq!(outcome.payload["line_end"], json!(2));
    let events = audit.snapshot();
    assert!(events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::ToolInvocation {
                path,
                outcome: loong_kernel::ToolInvocationOutcome::Completed,
                ..
            } if path.as_str() == "read"
        )
    }));
    assert!(!events.iter().any(|event| {
        matches!(
            &event.kind,
            loong_kernel::AuditEventKind::PlaneInvoked {
                primary_adapter,
                ..
            } if primary_adapter.starts_with("legacy:")
        )
    }));
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn kernel_routed_file_read_input_error_does_not_fallback_to_legacy_adapter() {
    let base = unique_temp_dir("loong-file-read-typed-error");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "file.read".to_owned(),
        payload: json!({
            "path": "notes.txt",
            "offset": 0
        }),
    };

    let error = execute_file_read_via_kernel_tool_registry(request, &config)
        .await
        .expect_err("typed read input error should not fallback");

    assert!(
        format!("{error}").contains("read payload.offset must be a positive integer"),
        "expected file read input error, got: {error}"
    );
    let _ = fs::remove_dir_all(base);
}

#[tokio::test]
async fn file_read_rejects_line_offset_beyond_end_of_file() {
    let base = unique_temp_dir("loongclaw-file-read-window-bounds");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("notes.txt"), "alpha\nbeta").expect("write fixture");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "file.read".to_owned(),
        payload: json!({
            "path": "notes.txt",
            "offset": 3
        }),
    };

    let error = execute_file_read_with_test_context(request, &config)
        .await
        .expect_err("out-of-bounds file.read window should fail");

    assert!(error.contains("offset 3 is beyond end of file (2 lines total)"));
    let _ = fs::remove_dir_all(base);
}

#[cfg(unix)]
#[test]
fn file_write_rejects_symlink_directory_escape() {
    let base = unique_temp_dir("loong-file-write");
    let root = base.join("root");
    let outside_dir = base.join("outside-dir");
    fs::create_dir_all(&root).expect("create root");
    fs::create_dir_all(&outside_dir).expect("create outside dir");

    let link = root.join("escape");
    assert!(create_symlink(&outside_dir, &link).is_ok());

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "file.write".to_owned(),
        payload: json!({
            "path": "escape/pwned.txt",
            "content": "owned",
            "create_dirs": true
        }),
    };
    let error = execute_file_write_tool_with_config(request, &config).expect_err("escape denied");

    assert!(error.starts_with("policy_denied: "));
    assert!(error.contains("escapes configured file root"));
    let _ = fs::remove_dir_all(base);
}

#[test]
fn file_write_allows_path_inside_root() {
    let base = unique_temp_dir("loong-file-safe");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");

    let config = ToolRuntimeConfig {
        file_root: Some(root.clone()),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "file.write".to_owned(),
        payload: json!({
            "path": "safe/note.txt",
            "content": "hello",
            "create_dirs": true
        }),
    };
    let result = execute_file_write_tool_with_config(request, &config);
    assert!(result.is_ok());

    let written = fs::read_to_string(root.join("safe/note.txt")).expect("read written file");
    assert_eq!(written, "hello");
    let _ = fs::remove_dir_all(base);
}

#[test]
fn resolve_safe_file_path_accepts_private_var_alias_inside_root() {
    let base = unique_temp_dir("loong-file-private-var-alias");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    let child = root.join("nested.txt");
    fs::write(&child, "ok").expect("write child");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let raw = child.display().to_string();
    let normalized_raw = if raw.starts_with("/private/var/") {
        raw.replacen("/private/var/", "/var/", 1)
    } else {
        raw
    };

    let resolved = resolve_safe_file_path_with_config(&normalized_raw, &config)
        .expect("alias path under root should resolve");

    assert_eq!(
        resolved,
        dunce::canonicalize(&child).expect("canonicalize child path")
    );
    let _ = fs::remove_dir_all(base);
}

#[test]
fn file_write_emits_create_change_preview_event() {
    let base = unique_temp_dir("loong-file-write-preview");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "file.write".to_owned(),
        payload: json!({
            "path": "preview.txt",
            "content": "alpha\nbeta\n",
            "create_dirs": true
        }),
    };
    let sink = Arc::new(RecordingRuntimeSink::default());
    let runtime_sink: Arc<dyn ToolRuntimeEventSink> = sink.clone();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime");

    let outcome = runtime.block_on(with_tool_runtime_event_sink(runtime_sink, async {
        execute_file_write_tool_with_config(request, &config)
    }));
    let outcome = outcome.expect("file.write should succeed");
    let events = lock_runtime_events(&sink);
    let preview = events.iter().find_map(|event| {
        if let ToolRuntimeEvent::FileChangePreview(preview) = event {
            return Some(preview);
        }

        None
    });
    let preview = preview.expect("file.write should emit change preview");

    assert_eq!(outcome.status, "ok");
    assert_eq!(preview.kind, ToolFileChangeKind::Create);
    assert_eq!(preview.added_lines, 2);
    assert_eq!(preview.removed_lines, 0);
    assert!(preview.path.ends_with("preview.txt"));
}

#[test]
fn file_write_emits_overwrite_change_preview_event() {
    let base = unique_temp_dir("loong-file-write-overwrite-preview");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    let target = root.join("preview.txt");
    fs::write(&target, "old line\nshared\n").expect("seed original file");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "file.write".to_owned(),
        payload: json!({
            "path": "preview.txt",
            "content": "new line\nshared\nextra\n",
            "overwrite": true
        }),
    };
    let sink = Arc::new(RecordingRuntimeSink::default());
    let runtime_sink: Arc<dyn ToolRuntimeEventSink> = sink.clone();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime");

    let outcome = runtime.block_on(with_tool_runtime_event_sink(runtime_sink, async {
        execute_file_write_tool_with_config(request, &config)
    }));
    let outcome = outcome.expect("file.write overwrite should succeed");
    let events = lock_runtime_events(&sink);
    let preview = events.iter().find_map(|event| {
        if let ToolRuntimeEvent::FileChangePreview(preview) = event {
            return Some(preview);
        }

        None
    });
    let preview = preview.expect("file.write overwrite should emit change preview");
    let preview_text = preview.preview.as_deref().unwrap_or_default();

    assert_eq!(outcome.status, "ok");
    assert_eq!(preview.kind, ToolFileChangeKind::Overwrite);
    assert_eq!(preview.added_lines, 2);
    assert_eq!(preview.removed_lines, 1);
    assert!(preview_text.contains("-old line"));
    assert!(preview_text.contains("+new line"));
    assert!(preview_text.contains("+extra"));
}

#[test]
fn file_write_rejects_existing_file_without_overwrite_flag() {
    let base = unique_temp_dir("loong-file-overwrite-denied");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");

    let target_path = root.join("note.txt");
    fs::write(&target_path, "original").expect("seed original file");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "file.write".to_owned(),
        payload: json!({
            "path": "note.txt",
            "content": "updated",
            "create_dirs": true
        }),
    };
    let error = execute_file_write_tool_with_config(request, &config)
        .expect_err("existing file should require overwrite=true");

    assert!(
        error.contains("overwrite=true"),
        "unexpected error: {error}"
    );
    let written = fs::read_to_string(&target_path).expect("read original file");
    assert_eq!(written, "original");
    let _ = fs::remove_dir_all(base);
}

#[test]
fn file_write_allows_existing_file_with_overwrite_true() {
    let base = unique_temp_dir("loong-file-overwrite-allowed");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");

    let target_path = root.join("note.txt");
    fs::write(&target_path, "original").expect("seed original file");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "file.write".to_owned(),
        payload: json!({
            "path": "note.txt",
            "content": "updated",
            "create_dirs": true,
            "overwrite": true
        }),
    };
    let outcome = execute_file_write_tool_with_config(request, &config)
        .expect("overwrite=true should allow replacing an existing file");

    assert_eq!(outcome.status, "ok");
    let written = fs::read_to_string(&target_path).expect("read updated file");
    assert_eq!(written, "updated");
    let _ = fs::remove_dir_all(base);
}

#[cfg(unix)]
#[test]
fn file_write_rejects_dangling_symlink_even_with_overwrite_true() {
    let base = unique_temp_dir("loong-file-overwrite-symlink");
    let root = base.join("root");
    let outside = base.join("outside");
    fs::create_dir_all(&root).expect("create root");
    fs::create_dir_all(&outside).expect("create outside");

    let dangling_target = outside.join("secret.txt");
    let link_path = root.join("dangling-link");
    create_symlink(&dangling_target, &link_path).expect("create dangling symlink");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "file.write".to_owned(),
        payload: json!({
            "path": "dangling-link",
            "content": "updated",
            "overwrite": true
        }),
    };
    let error = execute_file_write_tool_with_config(request, &config).expect_err("symlink denied");

    assert!(error.contains("refuses to open symlink"));
    assert!(!dangling_target.exists());
    let _ = fs::remove_dir_all(base);
}

fn make_edit_blocks_request(path: &str, edits: &[(&str, &str)]) -> ToolCoreRequest {
    let edit_blocks = edits
        .iter()
        .map(|(old, new)| {
            json!({
                "old_text": old,
                "new_text": new,
            })
        })
        .collect::<Vec<_>>();
    ToolCoreRequest {
        tool_name: "file.edit".to_owned(),
        payload: json!({
            "path": path,
            "edits": edit_blocks,
        }),
    }
}

fn make_camel_case_edit_blocks_request(path: &str, edits: &[(&str, &str)]) -> ToolCoreRequest {
    let edit_blocks = edits
        .iter()
        .map(|(old, new)| {
            json!({
                "oldText": old,
                "newText": new,
            })
        })
        .collect::<Vec<_>>();
    ToolCoreRequest {
        tool_name: "file.edit".to_owned(),
        payload: json!({
            "path": path,
            "edits": edit_blocks,
        }),
    }
}

#[test]
fn file_edit_single_match_succeeds() {
    let base = unique_temp_dir("loong-file-edit-single");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    let target = root.join("file.txt");
    fs::write(&target, "hello world").expect("write");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let result = execute_file_edit_tool_with_config(
        make_edit_blocks_request("file.txt", &[("hello", "hi")]),
        &config,
    );
    assert!(result.is_ok(), "unexpected error: {result:?}");
    let outcome = result.unwrap();
    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["replacements_made"], 1);
    assert_eq!(fs::read_to_string(&target).unwrap(), "hi world");
    let _ = fs::remove_dir_all(base);
}

#[test]
fn file_edit_no_match_errors() {
    let base = unique_temp_dir("loong-file-edit-nomatch");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("file.txt"), "hello world").expect("write");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let err = execute_file_edit_tool_with_config(
        make_edit_blocks_request("file.txt", &[("nothere", "x")]),
        &config,
    )
    .expect_err("should fail");
    assert!(err.contains("old_text not found"), "got: {err}");
    let _ = fs::remove_dir_all(base);
}

#[test]
fn file_edit_multiple_match_errors() {
    let base = unique_temp_dir("loong-file-edit-multi");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("file.txt"), "a\na\n").expect("write");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let err = execute_file_edit_tool_with_config(
        make_edit_blocks_request("file.txt", &[("a", "b")]),
        &config,
    )
    .expect_err("should fail");
    assert!(err.contains("matches 2 locations"), "got: {err}");
    let _ = fs::remove_dir_all(base);
}

#[test]
fn file_edit_emits_change_preview_event() {
    let base = unique_temp_dir("loong-file-edit-preview");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    let target = root.join("file.txt");
    fs::write(&target, "old line\nshared\n").expect("write original file");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = make_edit_blocks_request("file.txt", &[("old line", "new line")]);
    let sink = Arc::new(RecordingRuntimeSink::default());
    let runtime_sink: Arc<dyn ToolRuntimeEventSink> = sink.clone();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime");

    let outcome = runtime.block_on(with_tool_runtime_event_sink(runtime_sink, async {
        execute_file_edit_tool_with_config(request, &config)
    }));
    let outcome = outcome.expect("file.edit should succeed");
    let events = lock_runtime_events(&sink);
    let preview = events.iter().find_map(|event| {
        if let ToolRuntimeEvent::FileChangePreview(preview) = event {
            return Some(preview);
        }

        None
    });
    let preview = preview.expect("file.edit should emit change preview");
    let preview_text = preview.preview.as_deref().unwrap_or_default();

    assert_eq!(outcome.status, "ok");
    assert_eq!(preview.kind, ToolFileChangeKind::Edit);
    assert_eq!(preview.added_lines, 1);
    assert_eq!(preview.removed_lines, 1);
    assert!(preview_text.contains("-old line"));
    assert!(preview_text.contains("+new line"));
}

#[test]
fn file_edit_exact_edit_blocks_apply_multiple_replacements() {
    let base = unique_temp_dir("loongclaw-file-edit-blocks");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    let target = root.join("file.txt");
    fs::write(&target, "alpha\nbeta\ngamma\n").expect("write");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let result = execute_file_edit_tool_with_config(
        make_edit_blocks_request("file.txt", &[("alpha", "ALPHA"), ("gamma", "GAMMA")]),
        &config,
    );
    assert!(result.is_ok(), "unexpected error: {result:?}");
    let outcome = result.unwrap();
    assert_eq!(outcome.status, "ok");
    assert_eq!(outcome.payload["replacements_made"], 2);
    assert_eq!(outcome.payload["edit_blocks_applied"], 2);
    let resolved_target = resolve_safe_file_path_with_config("file.txt", &config)
        .expect("resolved target path")
        .display()
        .to_string();
    assert_eq!(outcome.payload["continuation"]["recommended_tool"], "read");
    assert_eq!(
        outcome.payload["continuation"]["recommended_payload"]["path"],
        resolved_target
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), "ALPHA\nbeta\nGAMMA\n");
    let _ = fs::remove_dir_all(base);
}

#[test]
fn file_edit_exact_edit_blocks_reject_non_unique_matches() {
    let base = unique_temp_dir("loongclaw-file-edit-blocks-non-unique");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("file.txt"), "dup\ndup\n").expect("write");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let err = execute_file_edit_tool_with_config(
        make_edit_blocks_request("file.txt", &[("dup", "only once")]),
        &config,
    )
    .expect_err("non-unique block should fail");
    assert!(err.contains("matches 2 locations"), "got: {err}");
    let _ = fs::remove_dir_all(base);
}

#[test]
fn file_edit_exact_edit_blocks_accept_camel_case_aliases() {
    let base = unique_temp_dir("loongclaw-file-edit-blocks-camel");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    let target = root.join("file.txt");
    fs::write(&target, "hello world").expect("write");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let result = execute_file_edit_tool_with_config(
        make_camel_case_edit_blocks_request("file.txt", &[("hello", "hi")]),
        &config,
    );
    assert!(result.is_ok(), "unexpected error: {result:?}");
    assert_eq!(fs::read_to_string(&target).unwrap(), "hi world");
    let _ = fs::remove_dir_all(base);
}

#[test]
fn summarize_file_change_preview_preserves_shared_middle_lines_when_appending_tail() {
    let before_lines = vec!["old line".to_owned(), "shared".to_owned()];
    let after_lines = vec![
        "new line".to_owned(),
        "shared".to_owned(),
        "extra".to_owned(),
    ];

    let (added_lines, removed_lines, preview) =
        summarize_file_change_preview(before_lines.as_slice(), after_lines.as_slice());
    let preview = preview.expect("preview should exist");

    assert_eq!(added_lines, 2);
    assert_eq!(removed_lines, 1);
    assert!(preview.contains("-old line"), "preview: {preview}");
    assert!(preview.contains("+new line"), "preview: {preview}");
    assert!(preview.contains("+extra"), "preview: {preview}");
}

#[test]
fn file_edit_empty_old_string_errors() {
    let base = unique_temp_dir("loong-file-edit-empty");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("file.txt"), "hello").expect("write");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let err = execute_file_edit_tool_with_config(
        make_edit_blocks_request("file.txt", &[("", "x")]),
        &config,
    )
    .expect_err("should fail");
    assert!(err.contains("old_text must not be empty"), "got: {err}");
    let _ = fs::remove_dir_all(base);
}

#[cfg(unix)]
#[test]
fn file_edit_rejects_path_escape() {
    let base = unique_temp_dir("loong-file-edit-escape");
    let root = base.join("root");
    let outside = base.join("outside");
    fs::create_dir_all(&root).expect("create root");
    fs::create_dir_all(&outside).expect("create outside");

    let outside_file = outside.join("secret.txt");
    fs::write(&outside_file, "secret content here").expect("write outside");
    let link = root.join("escape-link");
    assert!(create_symlink(&outside_file, &link).is_ok());

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let err = execute_file_edit_tool_with_config(
        make_edit_blocks_request("escape-link", &[("secret", "pwned")]),
        &config,
    )
    .expect_err("escape denied");

    assert!(err.starts_with("policy_denied: "));
    assert!(err.contains("escapes configured file root"));
    let _ = fs::remove_dir_all(base);
}

#[test]
fn glob_search_returns_workspace_relative_matches() {
    let base = unique_temp_dir("loong-glob-search");
    let root = base.join("root");
    let nested = root.join("src/nested");
    fs::create_dir_all(&nested).expect("create nested root");
    fs::write(root.join("src/lib.rs"), "pub fn alpha() {}").expect("write lib");
    fs::write(nested.join("mod.rs"), "pub fn beta() {}").expect("write mod");
    fs::write(root.join("README.md"), "hello").expect("write readme");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "glob.search".to_owned(),
        payload: json!({
            "pattern": "src/**/*.rs",
            "max_results": 10
        }),
    };
    let outcome =
        execute_glob_search_tool_with_config(request, &config).expect("glob search succeeds");
    let matches = outcome.payload["matches"]
        .as_array()
        .expect("matches array");

    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0]["path"], "src/lib.rs");
    assert_eq!(matches[1]["path"], "src/nested/mod.rs");
    assert_eq!(outcome.payload["continuation"]["state"], "path_listing");
    assert_eq!(outcome.payload["continuation"]["is_terminal"], false);
    assert_eq!(outcome.payload["continuation"]["recommended_tool"], "read");
    assert_eq!(
        outcome.payload["continuation"]["recommended_payload"]["path"],
        "src/lib.rs"
    );
    let _ = fs::remove_dir_all(base);
}

#[test]
fn content_search_returns_line_column_and_snippet() {
    let base = unique_temp_dir("loong-content-search");
    let root = base.join("root");
    let nested = root.join("src");
    fs::create_dir_all(&nested).expect("create nested root");
    fs::write(
        nested.join("main.rs"),
        "fn main() {\n    println!(\"hello world\");\n}\n",
    )
    .expect("write main");
    fs::write(root.join("notes.txt"), "hello from notes").expect("write notes");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "content.search".to_owned(),
        payload: json!({
            "query": "hello world",
            "glob": "src/**/*.rs",
            "max_results": 5
        }),
    };
    let outcome =
        execute_content_search_tool_with_config(request, &config).expect("content search succeeds");
    let matches = outcome.payload["matches"]
        .as_array()
        .expect("matches array");
    let first = matches.first().expect("first match");

    assert_eq!(matches.len(), 1);
    assert_eq!(first["path"], "src/main.rs");
    assert_eq!(first["line"], 2);
    assert_eq!(first["column"], 15);
    assert_eq!(first["snippet"], "println!(\"hello world\");");
    let _ = fs::remove_dir_all(base);
}

#[test]
fn content_search_honors_explicit_root() {
    let base = unique_temp_dir("loong-content-search-root");
    let root = base.join("root");
    let include = root.join("include");
    let exclude = root.join("exclude");
    fs::create_dir_all(&include).expect("create include");
    fs::create_dir_all(&exclude).expect("create exclude");
    fs::write(include.join("a.txt"), "needle here").expect("write include");
    fs::write(exclude.join("b.txt"), "needle here too").expect("write exclude");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "content.search".to_owned(),
        payload: json!({
            "root": "include",
            "query": "needle"
        }),
    };
    let outcome =
        execute_content_search_tool_with_config(request, &config).expect("content search succeeds");
    let matches = outcome.payload["matches"]
        .as_array()
        .expect("matches array");

    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["path"], "a.txt");
    let _ = fs::remove_dir_all(base);
}

#[test]
fn content_search_does_not_mark_exact_limit_files_as_truncated() {
    let base = unique_temp_dir("loong-content-search-exact-limit");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("exact.txt"), "hello").expect("write exact-limit file");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "content.search".to_owned(),
        payload: json!({
            "query": "hello",
            "max_bytes_per_file": 5
        }),
    };
    let outcome =
        execute_content_search_tool_with_config(request, &config).expect("content search succeeds");
    let matches = outcome.payload["matches"]
        .as_array()
        .expect("matches array");
    let first = matches.first().expect("first match");

    assert_eq!(first["truncated_file"], false);
    let _ = fs::remove_dir_all(base);
}

#[test]
fn content_search_handles_unicode_case_insensitive_matches() {
    let base = unique_temp_dir("loong-content-search-unicode");
    let root = base.join("root");
    fs::create_dir_all(&root).expect("create root");
    fs::write(root.join("city.txt"), "Key value\n").expect("write city");

    let config = ToolRuntimeConfig {
        file_root: Some(root),
        ..ToolRuntimeConfig::default()
    };
    let request = ToolCoreRequest {
        tool_name: "content.search".to_owned(),
        payload: json!({
            "query": "key",
            "case_sensitive": false
        }),
    };
    let outcome =
        execute_content_search_tool_with_config(request, &config).expect("content search succeeds");
    let matches = outcome.payload["matches"]
        .as_array()
        .expect("matches array");
    let first = matches.first().expect("first match");

    assert_eq!(first["path"], "city.txt");
    assert_eq!(first["match_text"], "Key");
    let _ = fs::remove_dir_all(base);
}
