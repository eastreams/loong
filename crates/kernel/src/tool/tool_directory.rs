use std::{borrow::Cow, collections::BTreeMap, sync::Arc};

use loong_contracts::SharedStr;

use super::Tool;

pub struct ToolPath {
    segments: Vec<SharedStr>,
}

pub enum ToolNode {
    Directory(ToolDirectory),
    Tool(Tool),
}

pub struct ToolDirectory {
    subdirs: BTreeMap<SharedStr, Box<ToolNode>>,
}

impl ToolDirectory {
    pub fn new() -> Self {
        Self {
            subdirs: BTreeMap::new(),
        }
    }
}
