use loong_runtime::tool_plane::ToolPath;

#[cfg(feature = "tool-file")]
#[test]
fn builtin_tool_plane_exposes_registered_file_paths() {
    let plane = super::test_builtin_tool_plane();
    let paths = plane.registered_paths();

    assert!(paths.contains(&ToolPath::from("read")));
    assert!(paths.contains(&ToolPath::from("write")));
    assert!(paths.contains(&ToolPath::from("edit")));
    assert!(paths.contains(&ToolPath::from("glob.search")));
    assert!(paths.contains(&ToolPath::from("content.search")));
}
