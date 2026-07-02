use std::collections::BTreeMap;

use crate::mvp;

#[derive(Debug, Clone)]
pub struct RuntimeSnapshotProviderState {
    pub active_profile_id: String,
    pub active_label: String,
    pub last_provider_id: Option<String>,
    pub saved_profile_ids: Vec<String>,
    pub transport_runtime: RuntimeSnapshotProviderTransportState,
    pub profiles: Vec<RuntimeSnapshotProviderProfileState>,
}

#[derive(Debug, Clone)]
pub struct RuntimeSnapshotProviderTransportState {
    pub http_client_cache_entries: usize,
    pub http_client_cache_hits: u64,
    pub http_client_cache_misses: u64,
    pub built_http_clients: u64,
    pub failover_total_events: usize,
    pub failover_continued_events: usize,
    pub failover_exhausted_events: usize,
    pub failover_by_reason: BTreeMap<String, usize>,
    pub failover_by_stage: BTreeMap<String, usize>,
    pub failover_by_provider: BTreeMap<String, usize>,
}

#[derive(Debug, Clone)]
pub struct RuntimeSnapshotProviderProfileState {
    pub profile_id: String,
    pub is_active: bool,
    pub default_for_kind: bool,
    pub descriptor: mvp::config::ProviderDescriptorDocument,
    pub kind: mvp::config::ProviderKind,
    pub model: String,
    pub wire_api: mvp::config::ProviderWireApi,
    pub base_url: String,
    pub endpoint: String,
    pub models_endpoint: String,
    pub protocol_family: &'static str,
    pub credential_resolved: bool,
    pub auth_env: Option<String>,
    pub reasoning_effort: Option<String>,
    pub temperature: f64,
    pub max_tokens: Option<u32>,
    pub request_timeout_ms: u64,
    pub retry_max_attempts: usize,
    pub header_names: Vec<String>,
    pub preferred_models: Vec<String>,
}
