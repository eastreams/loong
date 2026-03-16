#[path = "../build_support/version_metadata.rs"]
mod version_metadata;

use version_metadata::BuildMetadata;

#[test]
fn non_release_build_prefers_git_branch_and_short_sha_when_not_overridden() {
    let metadata = version_metadata::resolve_build_metadata(
        None,
        None,
        None,
        Some("alpha-test"),
        Some("ec9ee5f0d57b9ef18110786407c3ccdb447bbcf7"),
    );

    assert_eq!(
        metadata,
        BuildMetadata {
            release_build: false,
            channel: Some("alpha-test".to_owned()),
            short_sha: Some("ec9ee5f".to_owned()),
        }
    );
}

#[test]
fn explicit_compile_overrides_win_over_git_detection() {
    let metadata = version_metadata::resolve_build_metadata(
        None,
        Some("custom-preview"),
        Some("1234567890abcdef"),
        Some("alpha-test"),
        Some("ec9ee5f0d57b9ef18110786407c3ccdb447bbcf7"),
    );

    assert_eq!(
        metadata,
        BuildMetadata {
            release_build: false,
            channel: Some("custom-preview".to_owned()),
            short_sha: Some("1234567".to_owned()),
        }
    );
}

#[test]
fn git_rerun_hint_targets_include_head_ref_and_packed_refs() {
    assert_eq!(
        version_metadata::git_rerun_hint_targets(Some(
            "refs/heads/fix/onboard-pr187-followup-20260316"
        )),
        vec![
            "HEAD".to_owned(),
            "refs/heads/fix/onboard-pr187-followup-20260316".to_owned(),
            "packed-refs".to_owned(),
        ]
    );
}

#[test]
fn git_rerun_hint_targets_handle_detached_head() {
    assert_eq!(
        version_metadata::git_rerun_hint_targets(None),
        vec!["HEAD".to_owned(), "packed-refs".to_owned(),]
    );
}

#[test]
fn release_build_clears_non_release_trace_metadata() {
    let metadata = version_metadata::resolve_build_metadata(
        Some("1"),
        Some("alpha-test"),
        Some("ec9ee5f0d57b9ef18110786407c3ccdb447bbcf7"),
        Some("alpha-test"),
        Some("ec9ee5f0d57b9ef18110786407c3ccdb447bbcf7"),
    );

    assert_eq!(
        metadata,
        BuildMetadata {
            release_build: true,
            channel: None,
            short_sha: None,
        }
    );
}

#[test]
fn non_release_build_allows_missing_git_metadata_when_detection_is_unavailable() {
    let metadata = version_metadata::resolve_build_metadata(None, None, None, None, None);

    assert_eq!(
        metadata,
        BuildMetadata {
            release_build: false,
            channel: None,
            short_sha: None,
        }
    );
}
