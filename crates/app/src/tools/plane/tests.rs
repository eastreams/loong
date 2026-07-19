use loong_contracts::ToolPath;

// Path validation is covered by contracts; this test checks the builtin set.
#[allow(clippy::expect_used)]
fn tool_path(segment: &str) -> ToolPath {
    ToolPath::new([segment]).expect("test tool path must be valid")
}

#[cfg(feature = "tool-file")]
#[test]
fn builtin_tool_plane_exposes_registered_file_paths() {
    let plane = super::test_builtin_tool_plane();
    let paths = plane.registered_paths();

    assert!(paths.contains(&tool_path("read")));
    assert!(paths.contains(&tool_path("write")));
    assert!(paths.contains(&tool_path("edit")));
    assert!(paths.contains(&tool_path("glob.search")));
    assert!(paths.contains(&tool_path("content.search")));
}
