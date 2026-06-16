use std::collections::BTreeMap;

use super::Tool;

pub struct ToolPath {
    segments: Vec<Box<str>>,
}

impl ToolPath {
    pub fn segments(&self) -> &[Box<str>] {
        &self.segments
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.segments.iter().map(AsRef::as_ref)
    }
}

pub enum ToolNode {
    Directory(ToolDirectory),
    Tool(Tool),
}

pub enum ToolNodeRef<'a> {
    Directory(&'a ToolDirectory),
    Tool(&'a Tool),
}

pub enum ToolNodeMut<'a> {
    Directory(&'a mut ToolDirectory),
    Tool(&'a mut Tool),
}

impl<'a> From<&'a ToolNode> for ToolNodeRef<'a> {
    fn from(value: &'a ToolNode) -> Self {
        match value {
            ToolNode::Directory(dir) => Self::Directory(dir),
            ToolNode::Tool(tool) => Self::Tool(tool),
        }
    }
}

impl<'a> From<&'a mut ToolNode> for ToolNodeMut<'a> {
    fn from(value: &'a mut ToolNode) -> Self {
        match value {
            ToolNode::Directory(dir) => Self::Directory(dir),
            ToolNode::Tool(tool) => Self::Tool(tool),
        }
    }
}

pub struct ToolDirectory {
    subdirs: BTreeMap<Box<str>, Box<ToolNode>>,
}

impl ToolDirectory {
    pub fn new() -> Self {
        Self {
            subdirs: BTreeMap::new(),
        }
    }

    pub fn child(&self, segment: &str) -> Option<ToolNodeRef<'_>> {
        self.subdirs.get(segment).map(|node| node.as_ref().into())
    }

    pub fn child_mut(&mut self, segment: &str) -> Option<ToolNodeMut<'_>> {
        self.subdirs
            .get_mut(segment)
            .map(|node| node.as_mut().into())
    }

    pub fn resolve_segments<I, S>(&self, segments: I) -> Option<ToolNodeRef<'_>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut node = ToolNodeRef::Directory(self);
        for segment in segments {
            match node {
                ToolNodeRef::Tool(_) => {
                    return None;
                }
                ToolNodeRef::Directory(dir) => {
                    node = dir.child(segment.as_ref())?;
                }
            }
        }
        Some(node)
    }

    pub fn resolve_segments_mut<I, S>(&mut self, segments: I) -> Option<ToolNodeMut<'_>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut node = ToolNodeMut::Directory(self);
        for segment in segments {
            match node {
                ToolNodeMut::Tool(_) => {
                    return None;
                }
                ToolNodeMut::Directory(dir) => {
                    node = dir.child_mut(segment.as_ref())?;
                }
            }
        }
        Some(node)
    }

    pub fn resolve(&self, path: &ToolPath) -> Option<ToolNodeRef<'_>> {
        self.resolve_segments(path.iter())
    }

    pub fn resolve_mut(&mut self, path: &ToolPath) -> Option<ToolNodeMut<'_>> {
        self.resolve_segments_mut(path.iter())
    }

    pub fn resolve_tool(&self, path: &ToolPath) -> Option<&Tool> {
        match self.resolve(path)? {
            ToolNodeRef::Directory(_) => None,
            ToolNodeRef::Tool(tool) => Some(tool),
        }
    }

    pub fn resolve_tool_mut(&mut self, path: &ToolPath) -> Option<&mut Tool> {
        match self.resolve_mut(path)? {
            ToolNodeMut::Directory(_) => None,
            ToolNodeMut::Tool(tool) => Some(tool),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use loong_contracts::{ToolEffectClass, ToolOrigin, ToolSchedulingClass, ToolSpec};
    use serde_json::json;

    use super::*;
    use crate::errors::ToolPlaneError;
    use crate::test_support::MockToolAdapter;
    use crate::tool::{ToolAdapter, ToolContext, ToolOutcome, ToolRequest, sealed::Sealed};

    fn path<const N: usize>(segments: [&str; N]) -> ToolPath {
        ToolPath {
            segments: segments.into_iter().map(Box::<str>::from).collect(),
        }
    }

    fn test_tool(name: &str) -> Tool {
        let spec = ToolSpec::builder()
            .name(name)
            .provider("test")
            .description("test tool")
            .origin(ToolOrigin::BuiltIn)
            .effect_class(ToolEffectClass::ReadOnly)
            .scheduling_class(ToolSchedulingClass::ParallelSafe)
            .required_capabilities(Vec::new())
            .input_schema(json!({}))
            .build()
            .expect("test tool spec should be valid");

        Tool {
            spec,
            adapter: Arc::new(MockToolAdapter),
        }
    }

    fn sample_directory() -> ToolDirectory {
        let mut file_dir = ToolDirectory::new();
        file_dir
            .subdirs
            .insert("read".into(), Box::new(ToolNode::Tool(test_tool("read"))));

        let mut root = ToolDirectory::new();
        root.subdirs
            .insert("file".into(), Box::new(ToolNode::Directory(file_dir)));
        root.subdirs.insert(
            "direct_tool".into(),
            Box::new(ToolNode::Tool(test_tool("direct_tool"))),
        );
        root
    }

    #[test]
    fn child_returns_immediate_node() {
        let root = sample_directory();

        assert!(matches!(
            root.child("file"),
            Some(ToolNodeRef::Directory(_))
        ));
        assert!(matches!(
            root.child("direct_tool"),
            Some(ToolNodeRef::Tool(_))
        ));
        assert!(root.child("missing").is_none());
    }

    #[test]
    fn resolve_segments_returns_root_for_empty_path() {
        let root = sample_directory();

        assert!(matches!(
            root.resolve_segments(std::iter::empty::<&str>()),
            Some(ToolNodeRef::Directory(_))
        ));
    }

    #[test]
    fn resolve_follows_nested_segments() {
        let root = sample_directory();
        let resolved = root
            .resolve(&path(["file", "read"]))
            .expect("file/read should resolve");

        let ToolNodeRef::Tool(tool) = resolved else {
            panic!("file/read should resolve to a tool");
        };
        assert_eq!(tool.spec.name.as_ref(), "read");
    }

    #[test]
    fn resolve_stops_when_path_continues_through_tool() {
        let root = sample_directory();

        assert!(root.resolve(&path(["direct_tool", "child"])).is_none());
    }

    #[test]
    fn resolve_tool_filters_directory_nodes() {
        let root = sample_directory();

        assert!(root.resolve_tool(&path(["file"])).is_none());
        assert_eq!(
            root.resolve_tool(&path(["file", "read"]))
                .expect("file/read should resolve to a tool")
                .spec
                .name
                .as_ref(),
            "read"
        );
    }

    #[test]
    fn resolve_tool_mut_allows_mutating_leaf_tool() {
        let mut root = sample_directory();
        let tool_path = path(["file", "read"]);

        let tool = root
            .resolve_tool_mut(&tool_path)
            .expect("file/read should resolve mutably");
        tool.spec.description = "updated description".into();

        assert_eq!(
            root.resolve_tool(&tool_path)
                .expect("file/read should still resolve")
                .spec
                .description
                .as_ref(),
            "updated description"
        );
    }
}
