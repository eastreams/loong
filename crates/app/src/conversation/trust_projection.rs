use serde_json::{Value, json};

use crate::provider::parse_provider_failover_snapshot_payload;
use crate::trust::{
    delegate_child_trust_event, embed_trust_event_payload, extract_trust_event_payload,
    provider_failover_trust_event, runtime_binding_missing_trust_event,
};

use super::super::config::LoongClawConfig;
#[cfg(feature = "memory-sqlite")]
use super::super::runtime_self_continuity::RuntimeSelfContinuity;
use super::persistence::persist_conversation_event;
use super::runtime::ConversationRuntime;
use super::runtime_binding::ConversationRuntimeBinding;
#[cfg(feature = "memory-sqlite")]
use super::subagent::{ConstrainedSubagentExecution, DelegateBuiltinProfile};
use super::turn_engine::TurnResult;
#[cfg(feature = "memory-sqlite")]
use crate::session::repository::{
    CreateSessionWithEventRequest, NewSessionRecord, SessionKind, SessionState,
};

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
    config: &LoongClawConfig,
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

#[cfg(feature = "memory-sqlite")]
pub(super) fn build_delegate_started_child_session_request(
    parent_session_id: &str,
    child_session_id: &str,
    child_label: Option<String>,
    task: &str,
    profile: Option<DelegateBuiltinProfile>,
    runtime_self_continuity: Option<&RuntimeSelfContinuity>,
    execution: &ConstrainedSubagentExecution,
) -> CreateSessionWithEventRequest {
    build_delegate_child_session_request(
        parent_session_id,
        child_session_id,
        child_label,
        task,
        profile,
        runtime_self_continuity,
        execution,
        SessionState::Running,
        "delegate_started",
        "delegate.inline",
    )
}

#[cfg(feature = "memory-sqlite")]
pub(super) fn build_delegate_queued_child_session_request(
    parent_session_id: &str,
    child_session_id: &str,
    child_label: Option<String>,
    task: &str,
    profile: Option<DelegateBuiltinProfile>,
    runtime_self_continuity: Option<&RuntimeSelfContinuity>,
    execution: &ConstrainedSubagentExecution,
) -> CreateSessionWithEventRequest {
    build_delegate_child_session_request(
        parent_session_id,
        child_session_id,
        child_label,
        task,
        profile,
        runtime_self_continuity,
        execution,
        SessionState::Ready,
        "delegate_queued",
        "delegate.async",
    )
}

#[cfg(feature = "memory-sqlite")]
fn build_delegate_child_session_request(
    parent_session_id: &str,
    child_session_id: &str,
    child_label: Option<String>,
    task: &str,
    profile: Option<DelegateBuiltinProfile>,
    runtime_self_continuity: Option<&RuntimeSelfContinuity>,
    execution: &ConstrainedSubagentExecution,
    session_state: SessionState,
    event_kind: &str,
    source_surface: &str,
) -> CreateSessionWithEventRequest {
    let event_payload_json = build_delegate_child_event_payload(
        parent_session_id,
        child_session_id,
        task,
        child_label.as_deref(),
        profile,
        runtime_self_continuity,
        execution,
        source_surface,
    );
    let session = NewSessionRecord {
        session_id: child_session_id.to_owned(),
        kind: SessionKind::DelegateChild,
        parent_session_id: Some(parent_session_id.to_owned()),
        label: child_label,
        state: session_state,
    };

    CreateSessionWithEventRequest {
        session,
        event_kind: event_kind.to_owned(),
        actor_session_id: Some(parent_session_id.to_owned()),
        event_payload_json,
    }
}

#[cfg(feature = "memory-sqlite")]
fn build_delegate_child_event_payload(
    parent_session_id: &str,
    child_session_id: &str,
    task: &str,
    child_label: Option<&str>,
    profile: Option<DelegateBuiltinProfile>,
    runtime_self_continuity: Option<&RuntimeSelfContinuity>,
    execution: &ConstrainedSubagentExecution,
    source_surface: &str,
) -> Value {
    let trust_event =
        delegate_child_trust_event(parent_session_id, child_session_id, source_surface);
    let event_payload_json = execution.spawn_payload_with_profile_and_runtime_self_continuity(
        task,
        child_label,
        profile,
        runtime_self_continuity,
    );
    let payload_with_trust =
        embed_trust_event_payload(event_payload_json.clone(), trust_event.clone());
    let extracted_trust_event = extract_trust_event_payload(&payload_with_trust);
    if extracted_trust_event.as_ref() != Some(&trust_event) {
        return event_payload_json;
    }

    payload_with_trust
}
