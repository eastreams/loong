use std::{collections::BTreeSet, path::Path};

use loong_contracts::Capability;

use crate::policy::engine::{HasPolicyEngine, PolicyEngine};

pub trait PolicyContext: Send + Sync {
    fn capabilities(&self) -> BTreeSet<Capability>;
}

/// To make Policy: 'static + Any
pub trait PolicyContextFactory: 'static {
    type Context<'a>: PolicyContext;
}

pub type PolicyContextFor<'a, P> =
    <<<P as HasPolicyEngine>::PolicyEngine<'a> as PolicyEngine>::Factory as PolicyContextFactory>::Context<'a>;

pub trait WorkspacePolicyContext: PolicyContext {
    fn workspace_root(&self) -> &Path;
}
