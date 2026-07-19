use super::{ToolPath, ToolPathError};

#[test]
fn tool_path_requires_nonempty_valid_segments() {
    assert_eq!(
        ToolPath::new(std::iter::empty::<String>()),
        Err(ToolPathError::Empty)
    );
    assert_eq!(
        ToolPath::new(["file", ""]),
        Err(ToolPathError::EmptySegment { index: 1 })
    );
    assert_eq!(
        ToolPath::new(["file/read"]),
        Err(ToolPathError::SegmentContainsSeparator { index: 0 })
    );
    assert_eq!(
        ToolPath::new(["file\nread"]),
        Err(ToolPathError::SegmentContainsControl { index: 0 })
    );
}

#[test]
fn tool_path_text_preserves_explicit_segment_boundaries() {
    let path = ToolPath::new(["a", "b", "c", "tool"]).expect("literal path is valid");

    assert_eq!(path.segments(), ["a", "b", "c", "tool"]);
    assert_eq!(path.to_string(), "/a/b/c/tool");
    assert_eq!("/a/b/c/tool".parse::<ToolPath>(), Ok(path));
}

#[test]
fn dotted_tool_name_remains_one_opaque_segment() {
    let path = ToolPath::new(["glob.search"]).expect("literal path is valid");

    assert_eq!(path.segments(), ["glob.search"]);
    assert_eq!(path.to_string(), "/glob.search");
    assert_ne!(
        path,
        ToolPath::new(["glob", "search"]).expect("literal path is valid")
    );
}

#[test]
fn dot_segments_have_no_navigation_semantics() {
    let path = ToolPath::new(["a", "..", ".", "tool"]).expect("literal path is valid");

    assert_eq!(path.to_string(), "/a/.././tool");
    assert_eq!(path.to_string().parse::<ToolPath>(), Ok(path));
}

#[test]
fn tool_path_serde_uses_the_canonical_text_form() {
    let path = ToolPath::new(["file", "read"]).expect("literal path is valid");
    let encoded = serde_json::to_string(&path).expect("serialize tool path");

    assert_eq!(encoded, r#""/file/read""#);
    assert_eq!(
        serde_json::from_str::<ToolPath>(&encoded).expect("deserialize tool path"),
        path
    );
    assert!(serde_json::from_str::<ToolPath>(r#""file/read""#).is_err());
    assert!(serde_json::from_str::<ToolPath>(r#""/""#).is_err());
    assert!(serde_json::from_str::<ToolPath>(r#""/file//read""#).is_err());
    assert!(serde_json::from_str::<ToolPath>(r#""/file/read/""#).is_err());
    assert!(serde_json::from_str::<ToolPath>(r#"["file","read"]"#).is_err());
}
