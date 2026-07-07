use std::{collections::BTreeSet, path::Path};

use loong_contracts::{Capability, ExecutionPlane, PlaneTier};

pub trait PolicyContext: Send + Sync {
    /// Allowed capabilities
    fn capabilities(&self) -> BTreeSet<Capability>;
}

pub trait ActionContext: PolicyContext {
    fn execution_plane(&self) -> ExecutionPlane;

    fn plane_tier(&self) -> PlaneTier;
}

pub trait WorkspacePolicyContext: PolicyContext {
    fn workspace_root(&self) -> &Path;
}
