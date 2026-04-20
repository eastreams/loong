use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::contracts::{Capability, ExecutionRoute};

/// Runtime projection of a `VerticalPackManifest`.
///
/// Created when a pack is registered. Holds resolved runtime state
/// derived from the declarative manifest. The membrane field provides
/// a namespace isolation tag (defaults to pack_id).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Namespace {
    pub pack_id: String,
    pub domain: String,
    pub membrane: String,
    pub default_route: ExecutionRoute,
    pub granted_capabilities: BTreeSet<Capability>,
}
