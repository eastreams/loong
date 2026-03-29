use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct OutboundHttpConfig {
    #[serde(default)]
    pub allow_private_hosts: bool,
}

impl OutboundHttpConfig {
    #[must_use]
    pub const fn is_default(&self) -> bool {
        !self.allow_private_hosts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbound_http_defaults_to_public_only_mode() {
        let config = OutboundHttpConfig::default();
        assert!(!config.allow_private_hosts);
        assert!(config.is_default());
    }
}
