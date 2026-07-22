use loong_core::policy::context::PolicyContext;
use serde_json::{Value, json};

use crate::Context;
use crate::provider::parse_provider_failover_snapshot_payload;
use crate::trust::{
    embed_trust_event_payload, extract_trust_event_payload, provider_failover_trust_event,
};

use super::super::config::LoongConfig;
use super::persistence::persist_conversation_event;
use super::runtime::ConversationRuntime;

pub(super) async fn emit_provider_failover_trust_event_if_needed<
    R: ConversationRuntime + ?Sized,
>(
    config: &LoongConfig,
    runtime: &R,
    error_text: &str,
    ctx: &Context<'_>,
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
    let provenance_ref = "session";
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
        "subject": ctx.authorization_subject(),
        "provider_id": provider_id,
        "provider_failover": provider_failover,
    });
    let payload = embed_trust_event_payload(payload, trust_event);
    let extracted = extract_trust_event_payload(&payload);
    if extracted.is_none() {
        return;
    }
    let persist_result =
        persist_conversation_event(runtime, "trust_provider_failover", payload, ctx).await;
    if let Err(error) = persist_result {
        tracing::warn!(
            session_id = ctx.session().session_id(),
            event_kind = "trust_provider_failover",
            %error,
            "failed to persist trust event"
        );
    }
}
