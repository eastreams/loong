use super::*;

#[test]
fn snapshot_rejects_an_invalid_typed_identity() {
    let state = crate::tools::ToolSurfaceState {
        surface_id: "invalid/path".to_owned(),
        prompt_snippet: "must not be published".to_owned(),
        usage_guidance: String::new(),
        tool_ids: Vec::new(),
    };

    let error = agent_visible_summary_for_direct_state(None, &state)
        .expect_err("invalid identity must fail the snapshot");

    assert!(matches!(error, ToolMetadataError::InvalidPath { .. }));
}

#[test]
fn snapshot_uses_legacy_summary_only_for_an_unregistered_path() {
    let app_ctx = crate::context::bootstrap_test_app_context("snapshot-unregistered", 60)
        .expect("bootstrap context");
    let state = crate::tools::ToolSurfaceState {
        surface_id: "legacy-only".to_owned(),
        prompt_snippet: "legacy summary".to_owned(),
        usage_guidance: String::new(),
        tool_ids: Vec::new(),
    };

    let summary = agent_visible_summary_for_direct_state(Some(app_ctx.runtime()), &state)
        .expect("ordinary absence may use legacy metadata");

    assert_eq!(summary, "legacy summary");
}
