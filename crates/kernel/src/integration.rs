use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{contracts::Capability, errors::IntegrationError, pack::VerticalPackManifest};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub provider_id: String,
    pub connector_name: String,
    pub version: String,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelConfig {
    pub channel_id: String,
    pub provider_id: String,
    pub endpoint: String,
    pub enabled: bool,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderTemplate {
    pub provider_id: String,
    pub default_connector_name: String,
    pub default_version: String,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IntegrationCatalog {
    providers: BTreeMap<String, ProviderConfig>,
    channels: BTreeMap<String, ChannelConfig>,
    templates: BTreeMap<String, ProviderTemplate>,
    revision: u64,
}

impl IntegrationCatalog {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn register_template(&mut self, template: ProviderTemplate) {
        self.templates
            .insert(template.provider_id.clone(), template);
    }

    #[must_use]
    pub fn template(&self, provider_id: &str) -> Option<&ProviderTemplate> {
        self.templates.get(provider_id)
    }

    pub fn upsert_provider(&mut self, provider: ProviderConfig) {
        self.providers
            .insert(provider.provider_id.clone(), provider);
        self.revision = self.revision.saturating_add(1);
    }

    pub fn upsert_channel(&mut self, channel: ChannelConfig) {
        self.channels.insert(channel.channel_id.clone(), channel);
        self.revision = self.revision.saturating_add(1);
    }

    #[must_use]
    pub fn provider(&self, provider_id: &str) -> Option<&ProviderConfig> {
        self.providers.get(provider_id)
    }

    #[must_use]
    pub fn channel(&self, channel_id: &str) -> Option<&ChannelConfig> {
        self.channels.get(channel_id)
    }

    #[must_use]
    pub fn providers(&self) -> Vec<ProviderConfig> {
        self.providers.values().cloned().collect()
    }

    #[must_use]
    pub fn channels_for_provider(&self, provider_id: &str) -> Vec<ChannelConfig> {
        self.channels
            .values()
            .filter(|channel| channel.provider_id == provider_id)
            .cloned()
            .collect()
    }

    pub fn apply_plan(
        &mut self,
        pack: &mut VerticalPackManifest,
        plan: &ProvisionPlan,
    ) -> Result<(), IntegrationError> {
        for action in &plan.actions {
            match action {
                ProvisionAction::AddProvider { provider, .. }
                | ProvisionAction::PatchProvider { provider, .. } => {
                    self.upsert_provider(provider.clone());
                }
                ProvisionAction::AddChannel { channel, .. }
                | ProvisionAction::PatchChannel { channel, .. } => {
                    self.upsert_channel(channel.clone());
                }
            }
        }

        for connector in &plan.pack_connector_additions {
            pack.allowed_connectors.insert(connector.clone());
        }
        for capability in &plan.pack_capability_additions {
            pack.granted_capabilities.insert(*capability);
        }

        Ok(())
    }

    pub fn apply_hotfix(&mut self, hotfix: &IntegrationHotfix) -> Result<(), IntegrationError> {
        match hotfix {
            IntegrationHotfix::ProviderVersion {
                provider_id,
                new_version,
            } => {
                let provider = self
                    .providers
                    .get_mut(provider_id)
                    .ok_or_else(|| IntegrationError::ProviderNotFound(provider_id.clone()))?;
                provider.version = new_version.clone();
                self.revision = self.revision.saturating_add(1);
            }
            IntegrationHotfix::ProviderConnector {
                provider_id,
                new_connector_name,
            } => {
                let provider = self
                    .providers
                    .get_mut(provider_id)
                    .ok_or_else(|| IntegrationError::ProviderNotFound(provider_id.clone()))?;
                provider.connector_name = new_connector_name.clone();
                self.revision = self.revision.saturating_add(1);
            }
            IntegrationHotfix::ChannelEndpoint {
                channel_id,
                new_endpoint,
            } => {
                let channel = self
                    .channels
                    .get_mut(channel_id)
                    .ok_or_else(|| IntegrationError::ChannelNotFound(channel_id.clone()))?;
                channel.endpoint = new_endpoint.clone();
                self.revision = self.revision.saturating_add(1);
            }
            IntegrationHotfix::ChannelEnabled {
                channel_id,
                enabled,
            } => {
                let channel = self
                    .channels
                    .get_mut(channel_id)
                    .ok_or_else(|| IntegrationError::ChannelNotFound(channel_id.clone()))?;
                channel.enabled = *enabled;
                self.revision = self.revision.saturating_add(1);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoProvisionRequest {
    pub provider_id: String,
    pub channel_id: String,
    pub connector_name: Option<String>,
    pub endpoint: Option<String>,
    pub required_capabilities: BTreeSet<Capability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProvisionAction {
    AddProvider {
        provider: ProviderConfig,
        reason: String,
    },
    PatchProvider {
        provider: ProviderConfig,
        reason: String,
    },
    AddChannel {
        channel: ChannelConfig,
        reason: String,
    },
    PatchChannel {
        channel: ChannelConfig,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProvisionPlan {
    pub actions: Vec<ProvisionAction>,
    pub pack_connector_additions: BTreeSet<String>,
    pub pack_capability_additions: BTreeSet<Capability>,
}

impl ProvisionPlan {
    #[must_use]
    pub fn is_noop(&self) -> bool {
        self.actions.is_empty()
            && self.pack_connector_additions.is_empty()
            && self.pack_capability_additions.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntegrationHotfix {
    ProviderVersion {
        provider_id: String,
        new_version: String,
    },
    ProviderConnector {
        provider_id: String,
        new_connector_name: String,
    },
    ChannelEndpoint {
        channel_id: String,
        new_endpoint: String,
    },
    ChannelEnabled {
        channel_id: String,
        enabled: bool,
    },
}

#[derive(Debug, Default)]
pub struct AutoProvisionAgent;

impl AutoProvisionAgent {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    pub fn plan(
        &self,
        catalog: &IntegrationCatalog,
        pack: &VerticalPackManifest,
        request: &AutoProvisionRequest,
    ) -> Result<ProvisionPlan, IntegrationError> {
        let mut plan = ProvisionPlan::default();

        let provider = match catalog.provider(&request.provider_id) {
            Some(existing) => {
                if let Some(expected_connector) = request.connector_name.as_deref() {
                    if existing.connector_name != expected_connector {
                        let mut patched = existing.clone();
                        patched.connector_name = expected_connector.to_owned();
                        plan.actions.push(ProvisionAction::PatchProvider {
                            provider: patched.clone(),
                            reason: "connector override required by request".to_owned(),
                        });
                        patched
                    } else {
                        existing.clone()
                    }
                } else {
                    existing.clone()
                }
            }
            None => {
                let new_provider = self.new_provider_from_request(catalog, request);
                plan.actions.push(ProvisionAction::AddProvider {
                    provider: new_provider.clone(),
                    reason: "provider missing and generated from template".to_owned(),
                });
                new_provider
            }
        };

        let channel = match catalog.channel(&request.channel_id) {
            Some(existing) => {
                let mut patched = existing.clone();
                let mut changed = false;
                if patched.provider_id != provider.provider_id {
                    patched.provider_id = provider.provider_id.clone();
                    changed = true;
                }
                if !patched.enabled {
                    patched.enabled = true;
                    changed = true;
                }
                if let Some(endpoint) = request.endpoint.as_deref()
                    && patched.endpoint != endpoint
                {
                    patched.endpoint = endpoint.to_owned();
                    changed = true;
                }

                if changed {
                    plan.actions.push(ProvisionAction::PatchChannel {
                        channel: patched.clone(),
                        reason: "channel requires repair for provider binding or endpoint"
                            .to_owned(),
                    });
                    patched
                } else {
                    existing.clone()
                }
            }
            None => {
                let new_channel = ChannelConfig {
                    channel_id: request.channel_id.clone(),
                    provider_id: provider.provider_id.clone(),
                    endpoint: request.endpoint.clone().unwrap_or_else(|| {
                        format!(
                            "https://{}.local/{}/invoke",
                            provider.provider_id, request.channel_id
                        )
                    }),
                    enabled: true,
                    metadata: BTreeMap::new(),
                };
                plan.actions.push(ProvisionAction::AddChannel {
                    channel: new_channel.clone(),
                    reason: "channel missing and generated from provider defaults".to_owned(),
                });
                new_channel
            }
        };

        let connector_name = provider.connector_name;
        if !pack.allowed_connectors.contains(&connector_name) {
            plan.pack_connector_additions.insert(connector_name);
        }

        if !pack
            .granted_capabilities
            .contains(&Capability::InvokeConnector)
        {
            plan.pack_capability_additions
                .insert(Capability::InvokeConnector);
        }
        for capability in &request.required_capabilities {
            if !pack.granted_capabilities.contains(capability) {
                plan.pack_capability_additions.insert(*capability);
            }
        }

        if !channel.enabled {
            return Err(IntegrationError::ChannelDisabled(channel.channel_id));
        }

        Ok(plan)
    }

    fn new_provider_from_request(
        &self,
        catalog: &IntegrationCatalog,
        request: &AutoProvisionRequest,
    ) -> ProviderConfig {
        if let Some(template) = catalog.template(&request.provider_id) {
            ProviderConfig {
                provider_id: template.provider_id.clone(),
                connector_name: request
                    .connector_name
                    .clone()
                    .unwrap_or_else(|| template.default_connector_name.clone()),
                version: template.default_version.clone(),
                metadata: template.metadata.clone(),
            }
        } else {
            ProviderConfig {
                provider_id: request.provider_id.clone(),
                connector_name: request
                    .connector_name
                    .clone()
                    .unwrap_or_else(|| format!("{}-connector", request.provider_id)),
                version: "0.1.0".to_owned(),
                metadata: BTreeMap::from([("source".to_owned(), "auto-generated".to_owned())]),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        contracts::{ExecutionRoute, HarnessKind},
        pack::VerticalPackManifest,
    };

    fn sample_pack() -> VerticalPackManifest {
        VerticalPackManifest {
            pack_id: "sample-pack".to_owned(),
            domain: "engineering".to_owned(),
            version: "0.1.0".to_owned(),
            default_route: ExecutionRoute {
                harness_kind: HarnessKind::EmbeddedPi,
                adapter: Some("pi-local".to_owned()),
            },
            allowed_connectors: BTreeSet::new(),
            granted_capabilities: BTreeSet::new(),
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn agent_plans_missing_provider_and_channel_then_pack_is_extended() {
        let agent = AutoProvisionAgent::new();
        let mut catalog = IntegrationCatalog::new();
        catalog.register_template(ProviderTemplate {
            provider_id: "openai".to_owned(),
            default_connector_name: "openai".to_owned(),
            default_version: "1.0.0".to_owned(),
            metadata: BTreeMap::from([("class".to_owned(), "llm".to_owned())]),
        });
        let mut pack = sample_pack();

        let request = AutoProvisionRequest {
            provider_id: "openai".to_owned(),
            channel_id: "chat-main".to_owned(),
            connector_name: None,
            endpoint: Some("https://api.openai.com/v1/chat/completions".to_owned()),
            required_capabilities: BTreeSet::from([
                Capability::InvokeTool,
                Capability::ObserveTelemetry,
            ]),
        };
        let plan = agent
            .plan(&catalog, &pack, &request)
            .expect("plan should succeed");
        assert!(!plan.is_noop());
        assert!(plan.pack_connector_additions.contains("openai"));
        assert!(
            plan.pack_capability_additions
                .contains(&Capability::InvokeConnector)
        );
        assert!(
            plan.pack_capability_additions
                .contains(&Capability::ObserveTelemetry)
        );

        catalog
            .apply_plan(&mut pack, &plan)
            .expect("apply plan should succeed");

        assert!(catalog.provider("openai").is_some());
        assert!(catalog.channel("chat-main").is_some());
        assert!(pack.allowed_connectors.contains("openai"));
        assert!(
            pack.granted_capabilities
                .contains(&Capability::InvokeConnector)
        );
        assert!(
            pack.granted_capabilities
                .contains(&Capability::ObserveTelemetry)
        );
    }

    #[test]
    fn hotfix_can_patch_channel_endpoint_without_reboot() {
        let mut catalog = IntegrationCatalog::new();
        catalog.upsert_provider(ProviderConfig {
            provider_id: "slack".to_owned(),
            connector_name: "slack".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::new(),
        });
        catalog.upsert_channel(ChannelConfig {
            channel_id: "alerts".to_owned(),
            provider_id: "slack".to_owned(),
            endpoint: "https://old.example/alerts".to_owned(),
            enabled: true,
            metadata: BTreeMap::new(),
        });

        catalog
            .apply_hotfix(&IntegrationHotfix::ChannelEndpoint {
                channel_id: "alerts".to_owned(),
                new_endpoint: "https://new.example/alerts".to_owned(),
            })
            .expect("hotfix should succeed");

        let channel = catalog.channel("alerts").expect("channel should exist");
        assert_eq!(channel.endpoint, "https://new.example/alerts");
        assert!(catalog.revision() >= 3);
    }

    #[test]
    fn planner_repairs_disabled_channel() {
        let agent = AutoProvisionAgent::new();
        let mut catalog = IntegrationCatalog::new();
        catalog.upsert_provider(ProviderConfig {
            provider_id: "github".to_owned(),
            connector_name: "github".to_owned(),
            version: "1.0.0".to_owned(),
            metadata: BTreeMap::new(),
        });
        catalog.upsert_channel(ChannelConfig {
            channel_id: "webhooks".to_owned(),
            provider_id: "github".to_owned(),
            endpoint: "https://api.github.com/webhooks".to_owned(),
            enabled: false,
            metadata: BTreeMap::new(),
        });

        let plan = agent
            .plan(
                &catalog,
                &sample_pack(),
                &AutoProvisionRequest {
                    provider_id: "github".to_owned(),
                    channel_id: "webhooks".to_owned(),
                    connector_name: None,
                    endpoint: None,
                    required_capabilities: BTreeSet::new(),
                },
            )
            .expect("plan should succeed");

        assert!(
            plan.actions
                .iter()
                .any(|action| matches!(action, ProvisionAction::PatchChannel { .. }))
        );
    }
}
