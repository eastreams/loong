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
use super::*;
use std::path::PathBuf;

use serde_json::json;
