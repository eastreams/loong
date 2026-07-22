#[test]
fn parse_read_payload_requires_path() {
    let error = FileReadRequest::parse_payload(&json!({})).expect_err("missing path should fail");

    assert_eq!(error, ToolInputError::missing_field("path"));
}

#[test]
fn parse_read_payload_requires_object() {
    let error =
        ReadRequest::parse_payload(&json!(["notes.txt"])).expect_err("array payload should fail");

    assert_eq!(error, ToolInputError::PayloadMustBeObject);
}

#[test]
fn parse_aggregate_read_payload_requires_one_mode() {
    let error = ReadRequest::parse_payload(&json!({})).expect_err("missing read mode should fail");

    assert_eq!(
        error,
        loong_contracts::ToolInputError::MissingOneOf {
            fields: vec!["path".to_owned(), "query".to_owned(), "pattern".to_owned()],
        }
    );
}

#[test]
fn parse_read_payload_keeps_window_fields() {
    let parsed = FileReadRequest::parse_payload(&json!({
        "path": "notes.txt",
        "offset": 2,
        "limit": 3,
        "max_bytes": 4,
    }))
    .expect("payload should parse");

    assert_eq!(parsed.target, "notes.txt");
    assert_eq!(parsed.offset, Some(2));
    assert_eq!(parsed.limit, Some(3));
    assert_eq!(parsed.max_bytes, 4);
}

#[test]
fn parse_read_payload_preserves_invalid_window_field() {
    let error = FileReadRequest::parse_payload(&json!({
        "path": "notes.txt",
        "offset": 0,
    }))
    .expect_err("zero offset should fail");

    assert_eq!(
        error,
        ToolInputError::InvalidField {
            field: "offset".to_owned(),
            reason: "must be a positive integer".to_owned(),
        }
    );
}

#[test]
fn parse_aggregate_read_payload_preserves_invalid_mode_field() {
    let error = ReadRequest::parse_payload(&json!({
        "path": 42,
    }))
    .expect_err("non-string path should fail");

    assert_eq!(
        error,
        ToolInputError::InvalidField {
            field: "path".to_owned(),
            reason: "must be a string".to_owned(),
        }
    );
}

#[test]
fn build_output_returns_domain_payload_without_legacy_envelope() {
    let request = FileReadRequest {
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
            "path": "notes.txt",
            "bytes": 5,
            "truncated": false,
            "content": "hello",
        })
    );
}

#[test]
fn parse_read_payload_accepts_glob_alias() {
    let parsed = ReadRequest::parse_payload(&json!({
        "glob": "README.md|AGENTS.md",
        "root": ".",
    }))
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
    let parsed = ReadRequest::parse_payload(&json!({
        "path": "notes.txt",
        "query": "needle",
        "glob": "*.txt",
    }))
    .expect("path mode should parse");

    assert!(matches!(parsed, ReadRequest::File(_)));
}

#[test]
fn parse_write_payload_requires_path() {
    let error = WriteRequest::parse_payload(&json!({
        "content": "hello",
    }))
    .expect_err("missing path should fail");

    assert_eq!(error, ToolInputError::missing_field("path"));
}

#[test]
fn parse_write_payload_requires_content() {
    let error = WriteRequest::parse_payload(&json!({
        "path": "notes.txt",
    }))
    .expect_err("missing content should fail");

    assert_eq!(error, ToolInputError::missing_field("content"));
}

#[test]
fn parse_write_payload_preserves_invalid_flag_field() {
    let error = WriteRequest::parse_payload(&json!({
        "path": "notes.txt",
        "content": "hello",
        "overwrite": "yes",
    }))
    .expect_err("non-boolean overwrite should fail");

    assert_eq!(
        error,
        ToolInputError::InvalidField {
            field: "overwrite".to_owned(),
            reason: "must be a boolean".to_owned(),
        }
    );
}

#[test]
fn parse_write_payload_keeps_flags_and_defaults() {
    let defaulted = WriteRequest::parse_payload(&json!({
        "path": "notes.txt",
        "content": "",
    }))
    .expect("payload should parse");
    assert_eq!(defaulted.path, "notes.txt");
    assert_eq!(defaulted.content, "");
    assert!(defaulted.create_dirs);
    assert!(!defaulted.overwrite);

    let explicit = WriteRequest::parse_payload(&json!({
        "path": "notes.txt",
        "content": "hello",
        "create_dirs": false,
        "overwrite": true,
    }))
    .expect("payload should parse");
    assert!(!explicit.create_dirs);
    assert!(explicit.overwrite);
}

#[test]
fn build_write_output_returns_domain_payload_without_legacy_envelope() {
    let output = WriteOutput {
        path: PathBuf::from("notes.txt"),
        bytes_written: 5,
    };

    let payload: Value = output.into();

    assert_eq!(
        payload,
        json!({
            "path": "notes.txt",
            "bytes_written": 5,
        })
    );
}

#[test]
fn parse_edit_payload_accepts_exact_blocks() {
    let parsed = EditRequest::parse_payload(&json!({
        "path": "notes.txt",
        "edits": [
            {
                "old_text": "before",
                "newText": "after"
            }
        ]
    }))
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
fn parse_edit_payload_requires_edits() {
    let error = EditRequest::parse_payload(&json!({
        "path": "notes.txt",
    }))
    .expect_err("missing edits should fail");

    assert_eq!(error, ToolInputError::missing_field("edits"));
}

#[test]
fn parse_edit_payload_preserves_invalid_nested_field() {
    let error = EditRequest::parse_payload(&json!({
        "path": "notes.txt",
        "edits": [{
            "old_text": 42,
            "new_text": "after",
        }],
    }))
    .expect_err("non-string old_text should fail");

    assert_eq!(
        error,
        ToolInputError::InvalidField {
            field: "edits[0].old_text".to_owned(),
            reason: "must be a string".to_owned(),
        }
    );
}

#[test]
fn parse_glob_payload_preserves_missing_pattern_field() {
    let payload = json!({});
    let error = GlobReadRequest::parse_payload(
        payload
            .as_object()
            .expect("test payload should be an object"),
    )
    .expect_err("missing pattern should fail");

    assert_eq!(error, ToolInputError::missing_field("pattern"));
}

#[test]
fn parse_glob_payload_preserves_invalid_limit_field() {
    let payload = json!({
        "pattern": "*.rs",
        "max_results": 0,
    });
    let error = GlobReadRequest::parse_payload(
        payload
            .as_object()
            .expect("test payload should be an object"),
    )
    .expect_err("out-of-range max_results should fail");

    assert_eq!(
        error,
        ToolInputError::InvalidField {
            field: "max_results".to_owned(),
            reason: "must be between 1 and 200".to_owned(),
        }
    );
}

#[test]
fn parse_content_search_payload_preserves_missing_query_field() {
    let payload = json!({});
    let error = ContentSearchReadRequest::parse_payload(
        payload
            .as_object()
            .expect("test payload should be an object"),
    )
    .expect_err("missing query should fail");

    assert_eq!(error, ToolInputError::missing_field("query"));
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
fn build_edit_output_returns_domain_response_without_preview_content() {
    let output = EditOutput {
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

use loong_contracts::ToolInputError;
use serde_json::{Value, json};

#[test]
fn read_tool_error_preserves_fs_read_error_as_source() {
    let error = ReadToolError::from(loong_kernel::access::fs::FsReadError::ReadFile {
        path: PathBuf::from("notes.txt"),
        source: std::io::Error::other("test read failure"),
    });

    assert!(matches!(error, ReadToolError::Read(_)));
    std::error::Error::source(&error)
        .and_then(|source| source.downcast_ref::<loong_kernel::access::fs::FsReadError>())
        .expect("read tool error should retain its filesystem access source");
}
