use std::collections::{BTreeMap, BTreeSet};

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::{
    contracts::{Capability, ExecutionRoute},
    errors::PackError,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerticalPackManifest {
    pub pack_id: String,
    pub domain: String,
    pub version: String,
    pub default_route: ExecutionRoute,
    pub allowed_connectors: BTreeSet<String>,
    pub granted_capabilities: BTreeSet<Capability>,
    pub metadata: BTreeMap<String, String>,
}

impl VerticalPackManifest {
    pub fn validate(&self) -> Result<(), PackError> {
        if self.pack_id.trim().is_empty() {
            return Err(PackError::EmptyPackId);
        }
        if self.domain.trim().is_empty() {
            return Err(PackError::EmptyDomain);
        }
        Version::parse(&self.version)
            .map_err(|_| PackError::InvalidVersion(self.version.clone()))?;
        if self.granted_capabilities.is_empty() {
            return Err(PackError::EmptyCapabilities);
        }
        Ok(())
    }

    pub fn grants(&self, capability: Capability) -> bool {
        self.granted_capabilities.contains(&capability)
    }

    pub fn allows_connector(&self, connector_name: &str) -> bool {
        self.allowed_connectors.contains(connector_name)
    }
}
