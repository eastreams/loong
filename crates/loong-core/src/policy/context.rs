use std::{collections::BTreeSet, path::Path};

use loong_contracts::Capability;

pub trait PolicyContext: Send + Sync {
    fn capabilities(&self) -> BTreeSet<Capability>;
}

pub trait WorkspacePolicyContext: PolicyContext {
    fn workspace_root(&self) -> &Path;
}
