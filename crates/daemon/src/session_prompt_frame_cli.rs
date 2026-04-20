use loong_app as mvp;
use serde_json::{Value, json};

pub(crate) async fn load_session_prompt_frame_payload(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
    session_id: &str,
) -> Value {
    let summary_limit = prompt_frame_summary_limit(memory_config);
    let binding = mvp::conversation::ConversationRuntimeBinding::direct();
    let summary_result = mvp::conversation::load_prompt_frame_event_summary(
        session_id,
        summary_limit,
        binding,
        memory_config,
    )
    .await;

    match summary_result {
        Ok(summary) => {
            let available = summary.snapshot_events > 0;
            json!({
                "available": available,
                "limit": summary_limit,
                "summary": summary,
            })
        }
        Err(error) => json!({
            "available": false,
            "limit": summary_limit,
            "error": error,
        }),
    }
}

pub(crate) fn render_prompt_frame_summary(prompt_frame: Option<&Value>) -> String {
    let Some(prompt_frame) = prompt_frame else {
        return "-".to_owned();
    };

    let available = prompt_frame
        .get("available")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !available {
        let error = prompt_frame
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("-");
        return format!("unavailable error={error}");
    }

    let Some(summary) = prompt_frame.get("summary") else {
        return "present".to_owned();
    };

    let phase = summary
        .get("latest_phase")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let total_tokens = summary
        .get("latest_total_estimated_tokens")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned());
    let stable_prefix = summary
        .get("latest_stable_prefix_hash")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let cached_prefix = summary
        .get("latest_cached_prefix_hash")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let stable_prefix_changes = summary
        .get("stable_prefix_hash_change_events")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "0".to_owned());
    let cached_prefix_changes = summary
        .get("cached_prefix_hash_change_events")
        .and_then(Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "0".to_owned());

    format!(
        "phase={phase} total_tokens={total_tokens} stable_prefix={stable_prefix} cached_prefix={cached_prefix} stable_prefix_changes={stable_prefix_changes} cached_prefix_changes={cached_prefix_changes}"
    )
}

fn prompt_frame_summary_limit(
    memory_config: &mvp::memory::runtime_config::MemoryRuntimeConfig,
) -> usize {
    let scaled_limit = memory_config.sliding_window.saturating_mul(4);
    scaled_limit.clamp(16, 128)
}
