#[test]
fn parse_read_payload_requires_path() {
    let error = FileReadRequest::parse_payload("read".to_owned(), &json!({}))
        .expect_err("missing path should fail");

    assert_eq!(error, "read requires payload.path");
}

#[test]
fn parse_read_payload_keeps_window_fields() {
    let parsed = FileReadRequest::parse_payload(
        "read".to_owned(),
        &json!({
            "path": "notes.txt",
            "offset": 2,
            "limit": 3,
            "max_bytes": 4,
        }),
    )
    .expect("payload should parse");

    assert_eq!(parsed.target, "notes.txt");
    assert_eq!(parsed.offset, Some(2));
    assert_eq!(parsed.limit, Some(3));
    assert_eq!(parsed.max_bytes, 4);
}

#[test]
fn build_output_returns_typed_payload_without_legacy_status() {
    let request = FileReadRequest {
        tool_name: "read".to_owned(),
        target: "notes.txt".to_owned(),
        max_bytes: 1_024,
        offset: None,
        limit: None,
    };

    let output =
        ReadTool::build_file_output(request, PathBuf::from("notes.txt"), b"hello".to_vec())
            .expect("read output should build");
    let payload: Value = output.into();

    assert_eq!(
        payload,
        json!({
            "adapter": "core-tools",
            "tool_name": "read",
            "path": "notes.txt",
            "bytes": 5,
            "truncated": false,
            "content": "hello",
        })
    );
}

#[test]
fn parse_read_payload_accepts_glob_alias() {
    let parsed = ReadRequest::parse_payload(
        "read".to_owned(),
        &json!({
            "glob": "README.md|AGENTS.md",
            "root": ".",
        }),
    )
    .expect("glob alias should parse");

    assert!(matches!(
        parsed,
        ReadRequest::Glob(GlobReadRequest {
            pattern,
            ..
        }) if pattern == "{README.md,AGENTS.md}"
    ));
}

#[test]
fn parse_read_payload_prioritizes_path() {
    let parsed = ReadRequest::parse_payload(
        "read".to_owned(),
        &json!({
            "path": "notes.txt",
            "query": "needle",
            "glob": "*.txt",
        }),
    )
    .expect("path mode should parse");

    assert!(matches!(parsed, ReadRequest::File(_)));
}

#[test]
fn parse_write_payload_requires_path() {
    let error = WriteRequest::parse_payload(
        "write".to_owned(),
        &json!({
            "content": "hello",
        }),
    )
    .expect_err("missing path should fail");

    assert_eq!(error, "write requires payload.path");
}

#[test]
fn parse_write_payload_requires_content() {
    let error = WriteRequest::parse_payload(
        "write".to_owned(),
        &json!({
            "path": "notes.txt",
        }),
    )
    .expect_err("missing content should fail");

    assert_eq!(error, "write requires payload.content");
}

#[test]
fn parse_write_payload_keeps_flags_and_defaults() {
    let defaulted = WriteRequest::parse_payload(
        "write".to_owned(),
        &json!({
            "path": "notes.txt",
            "content": "",
        }),
    )
    .expect("payload should parse");
    assert_eq!(defaulted.path, "notes.txt");
    assert_eq!(defaulted.content, "");
    assert!(defaulted.create_dirs);
    assert!(!defaulted.overwrite);

    let explicit = WriteRequest::parse_payload(
        "write".to_owned(),
        &json!({
            "path": "notes.txt",
            "content": "hello",
            "create_dirs": false,
            "overwrite": true,
        }),
    )
    .expect("payload should parse");
    assert!(!explicit.create_dirs);
    assert!(explicit.overwrite);
}

#[test]
fn build_write_output_returns_typed_payload_without_legacy_status() {
    let output = WriteOutput {
        tool_name: "write".to_owned(),
        path: PathBuf::from("notes.txt"),
        bytes_written: 5,
    };

    let payload: Value = output.into();

    assert_eq!(
        payload,
        json!({
            "adapter": "core-tools",
            "tool_name": "write",
            "path": "notes.txt",
            "bytes_written": 5,
        })
    );
}

#[test]
fn parse_edit_payload_accepts_exact_blocks() {
    let parsed = EditRequest::parse_payload(
        "edit".to_owned(),
        &json!({
            "path": "notes.txt",
            "edits": [
                {
                    "old_text": "before",
                    "newText": "after"
                }
            ]
        }),
    )
    .expect("payload should parse");

    assert_eq!(parsed.path, "notes.txt");
    assert_eq!(
        parsed.blocks,
        vec![ExactTextEditBlock {
            old_text: "before".to_owned(),
            new_text: "after".to_owned(),
        }]
    );
}

#[test]
fn apply_edit_blocks_requires_unique_non_overlapping_matches() {
    let blocks = vec![ExactTextEditBlock {
        old_text: "hello".to_owned(),
        new_text: "hi".to_owned(),
    }];

    let applied =
        EditTool::apply_exact_edit_blocks("hello world", blocks.as_slice()).expect("edit applies");

    assert_eq!(applied.updated, "hi world");
    assert_eq!(applied.replacements_made, 1);

    let duplicate_error = EditTool::apply_exact_edit_blocks("hello hello", blocks.as_slice())
        .expect_err("duplicate old_text should fail");
    assert_eq!(
        duplicate_error,
        "edit_failed: edits[0].old_text matches 2 locations; each edit block must match uniquely in the original file"
    );
}

#[test]
fn build_edit_output_returns_response_without_preview_content() {
    let output = EditOutput {
        tool_name: "edit".to_owned(),
        path: PathBuf::from("notes.txt"),
        before: "before".to_owned(),
        after: "after".to_owned(),
        replacements_made: 1,
        edit_blocks_applied: 1,
    };

    let payload: Value = output.into();

    assert_eq!(
        payload,
        json!({
            "adapter": "core-tools",
            "tool_name": "edit",
            "path": "notes.txt",
            "replacements_made": 1,
            "bytes_written": 5,
            "edit_blocks_applied": 1,
            "continuation": {
                "state": "verify_file_change",
                "is_terminal": false,
                "recommended_tool": "read",
                "recommended_payload": {
                    "path": "notes.txt"
                },
                "note": "If the user still depends on the updated file contents, verify the file before finalizing."
            }
        })
    );
}
use super::*;
use std::path::PathBuf;

use serde_json::{Value, json};
