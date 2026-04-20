use serde_json::{Value, json};

use crate::provider::parse_provider_failover_snapshot_payload;
use crate::trust::{
    embed_trust_event_payload, extract_trust_event_payload, provider_failover_trust_event,
    runtime_binding_missing_trust_event,
};

use super::super::config::LoongConfig;
use super::persistence::persist_conversation_event;
use super::runtime::ConversationRuntime;
use super::runtime_binding::ConversationRuntimeBinding;
use super::turn_engine::TurnResult;

pub(super) async fn emit_runtime_binding_trust_event_if_needed<R: ConversationRuntime + ?Sized>(
    runtime: &R,
    session_id: &str,
    turn_result: &TurnResult,
    binding: ConversationRuntimeBinding<'_>,
) {
    const NO_KERNEL_CONTEXT_REASON: &str = "no_kernel_context";

    let TurnResult::ToolDenied(failure) = turn_result else {
        return;
    };
    let missing_kernel_context =
        failure.code == NO_KERNEL_CONTEXT_REASON || failure.reason == NO_KERNEL_CONTEXT_REASON;
    let failure_code = if missing_kernel_context {
        Some(NO_KERNEL_CONTEXT_REASON)
    } else {
        None
    };
    let Some(failure_code) = failure_code else {
        return;
    };

    let provenance_ref = if binding.is_kernel_bound() {
        "kernel"
    } else {
        "direct"
    };
    let trust_event =
        runtime_binding_missing_trust_event(session_id, "conversation.binding", provenance_ref);
    let payload = json!({
        "source": "conversation_runtime",
        "failure_code": failure_code,
    });
    let payload = embed_trust_event_payload(payload, trust_event);
    let extracted = extract_trust_event_payload(&payload);
    if extracted.is_none() {
        return;
    }
    let binding_kind = if binding.is_kernel_bound() {
        "kernel"
    } else {
        "direct"
    };
    let persist_result = persist_conversation_event(
        runtime,
        session_id,
        "trust_binding_missing",
        payload,
        binding,
    )
    .await;
    if let Err(error) = persist_result {
        tracing::warn!(
            session_id,
            event_kind = "trust_binding_missing",
            binding_kind,
            %error,
            "failed to persist trust event"
        );
    }
}

pub(super) async fn emit_provider_failover_trust_event_if_needed<
    R: ConversationRuntime + ?Sized,
>(
    config: &LoongConfig,
    runtime: &R,
    session_id: &str,
    error_text: &str,
    binding: ConversationRuntimeBinding<'_>,
) {
    let Some(provider_failover) = parse_provider_failover_snapshot_payload(error_text) else {
        return;
    };

    let provider_id = config.provider.kind.profile().id;
    let reason_value = provider_failover.get("reason");
    let reason_code = reason_value
        .and_then(Value::as_str)
        .unwrap_or("provider_failover");
    let model_value = provider_failover.get("model");
    let model = model_value.and_then(Value::as_str).unwrap_or("unknown");
    let stage_value = provider_failover.get("stage");
    let stage = stage_value.and_then(Value::as_str).unwrap_or("unknown");
    let provenance_ref = if binding.is_kernel_bound() {
        "kernel"
    } else {
        "advisory_only"
    };
    let trust_event = provider_failover_trust_event(
        provider_id,
        "provider.failover",
        provenance_ref,
        reason_code,
        model,
        stage,
    );
    let payload = json!({
        "source": "provider_runtime",
        "binding": provenance_ref,
        "provider_id": provider_id,
        "provider_failover": provider_failover,
    });
    let payload = embed_trust_event_payload(payload, trust_event);
    let extracted = extract_trust_event_payload(&payload);
    if extracted.is_none() {
        return;
    }
    let binding_kind = if binding.is_kernel_bound() {
        "kernel"
    } else {
        "direct"
    };
    let persist_result = persist_conversation_event(
        runtime,
        session_id,
        "trust_provider_failover",
        payload,
        binding,
    )
    .await;
    if let Err(error) = persist_result {
        tracing::warn!(
            session_id,
            event_kind = "trust_provider_failover",
            binding_kind,
            %error,
            "failed to persist trust event"
        );
    }
}
