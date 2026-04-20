use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use super::super::backend::{AcpSessionBootstrap, AcpSessionMetadata, AcpSessionState};
use super::super::binding::AcpSessionBindingScope;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpManagerRuntimeCacheSnapshot {
    pub active_sessions: usize,
    pub idle_ttl_ms: u64,
    pub evicted_total: u64,
    pub last_evicted_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpManagerTurnSnapshot {
    pub active: usize,
    pub queue_depth: usize,
    pub completed: u64,
    pub failed: u64,
    pub average_latency_ms: u64,
    pub max_latency_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpManagerActorSnapshot {
    pub active: usize,
    pub queue_depth: usize,
    pub waiting: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpManagerSessionSnapshot {
    pub bound: usize,
    pub unbound: usize,
    pub activation_origin_counts: BTreeMap<String, usize>,
    pub backend_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpManagerObservabilitySnapshot {
    pub runtime_cache: AcpManagerRuntimeCacheSnapshot,
    pub sessions: AcpManagerSessionSnapshot,
    pub actors: AcpManagerActorSnapshot,
    pub turns: AcpManagerTurnSnapshot,
    pub errors_by_code: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct TurnLatencyStats {
    pub(super) completed: u64,
    pub(super) failed: u64,
    pub(super) total_ms: u64,
    pub(super) max_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RedactedBindingScopeForLog {
    pub(super) route_session_id: String,
    pub(super) channel_id: Option<String>,
    pub(super) account_id: Option<String>,
    pub(super) conversation_id: Option<String>,
    pub(super) thread_id: Option<String>,
}

const IDENTIFIER_FINGERPRINT_HEX_PREFIX_LEN: usize = 24;

pub(super) fn normalized_identifier(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_owned())
}

pub(super) fn normalized_conversation_id(raw: Option<&str>) -> Option<String> {
    normalized_identifier(raw)
}

pub(super) fn redact_identifier_for_log(raw: Option<&str>) -> Option<String> {
    let normalized = normalized_identifier(raw)?;
    Some(identifier_fingerprint(normalized.as_str()))
}

pub(super) fn identifier_fingerprint(raw: &str) -> String {
    let digest = Sha256::digest(raw.as_bytes());
    let hex = hex::encode(digest);
    let prefix = &hex[..IDENTIFIER_FINGERPRINT_HEX_PREFIX_LEN];
    format!("sha256:{prefix}")
}

pub(super) fn redact_binding_scope_for_log(
    binding: Option<&AcpSessionBindingScope>,
) -> Option<RedactedBindingScopeForLog> {
    let binding = binding?;

    Some(RedactedBindingScopeForLog {
        route_session_id: identifier_fingerprint(binding.route_session_id.as_str()),
        channel_id: binding.channel_id.clone(),
        account_id: redact_identifier_for_log(binding.account_id.as_deref()),
        conversation_id: redact_identifier_for_log(binding.conversation_id.as_deref()),
        thread_id: redact_identifier_for_log(binding.thread_id.as_deref()),
    })
}

pub(super) fn actor_key_for_bootstrap(bootstrap: &AcpSessionBootstrap) -> String {
    let binding = AcpSessionBindingScope::from_bootstrap(bootstrap);
    session_actor_key(
        bootstrap.session_key.as_str(),
        bootstrap.conversation_id.as_deref(),
        binding.as_ref(),
    )
}

pub(super) fn actor_key_for_metadata(metadata: &AcpSessionMetadata) -> String {
    session_actor_key(
        metadata.session_key.as_str(),
        metadata.conversation_id.as_deref(),
        metadata.binding.as_ref(),
    )
}

pub(super) fn session_actor_key(
    session_key: &str,
    conversation_id: Option<&str>,
    binding: Option<&AcpSessionBindingScope>,
) -> String {
    if let Some(binding) = binding {
        return format!("route:{}", binding.route_session_id);
    }
    if let Some(conversation_id) = normalized_conversation_id(conversation_id) {
        return format!("conversation:{conversation_id}");
    }
    format!("session:{}", session_key.trim())
}

pub(super) fn normalize_error_key(error: &str) -> String {
    let trimmed = error.trim();
    if trimmed.is_empty() {
        return "unknown".to_owned();
    }
    const MAX_ERROR_KEY_LEN: usize = 120;
    let mut truncated = String::new();
    for (idx, ch) in trimmed.chars().enumerate() {
        if idx >= MAX_ERROR_KEY_LEN {
            truncated.push_str("...");
            break;
        }
        truncated.push(ch);
    }
    truncated
}

pub(super) fn projected_status_state(
    state: AcpSessionState,
    active_turn: bool,
    pending_turns: usize,
) -> AcpSessionState {
    if !active_turn && pending_turns == 0 {
        return state;
    }

    if matches!(
        state,
        AcpSessionState::Cancelling | AcpSessionState::Error | AcpSessionState::Closed
    ) {
        state
    } else {
        AcpSessionState::Busy
    }
}

pub(super) fn bump_usize_count(counts: &mut BTreeMap<String, usize>, key: &str) {
    let entry = counts.entry(key.to_owned()).or_insert(0);
    *entry = entry.saturating_add(1);
}

pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
