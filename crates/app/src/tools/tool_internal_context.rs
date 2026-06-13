#[cfg(test)]
use std::cell::Cell;
use std::future::Future;

use serde_json::Value;

use super::LOONG_INTERNAL_TOOL_CONTEXT_KEY;

tokio::task_local! {
    static TRUSTED_INTERNAL_TOOL_PAYLOAD_TASK: bool;
}

#[cfg(test)]
thread_local! {
    static TRUSTED_INTERNAL_TOOL_PAYLOAD_DEPTH: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn with_trusted_internal_tool_payload<T>(f: impl FnOnce() -> T) -> T {
    struct TrustedInternalToolPayloadGuard;

    impl Drop for TrustedInternalToolPayloadGuard {
        fn drop(&mut self) {
            TRUSTED_INTERNAL_TOOL_PAYLOAD_DEPTH.with(|depth| {
                depth.set(depth.get().saturating_sub(1));
            });
        }
    }

    TRUSTED_INTERNAL_TOOL_PAYLOAD_DEPTH.with(|depth| {
        depth.set(depth.get().saturating_add(1));
    });
    let _guard = TrustedInternalToolPayloadGuard;
    f()
}

pub(crate) async fn with_trusted_internal_tool_payload_async<T>(
    future: impl Future<Output = T>,
) -> T {
    if trusted_internal_tool_payload_enabled() {
        return future.await;
    }

    TRUSTED_INTERNAL_TOOL_PAYLOAD_TASK.scope(true, future).await
}

#[cfg(test)]
pub(crate) fn reset_runtime_home_state_for_tests() {
    super::tool_lease_authority::clear_tool_lease_secret_cache_for_tests();
}

pub(crate) fn trusted_internal_tool_payload_enabled() -> bool {
    #[cfg(test)]
    let test_enabled = TRUSTED_INTERNAL_TOOL_PAYLOAD_DEPTH.with(|depth| depth.get() > 0);
    #[cfg(not(test))]
    let test_enabled = false;

    test_enabled
        || TRUSTED_INTERNAL_TOOL_PAYLOAD_TASK
            .try_with(|enabled| *enabled)
            .unwrap_or(false)
}

pub(crate) fn payload_uses_reserved_internal_tool_context(payload: &Value) -> bool {
    reserved_internal_tool_context_key_in_payload(payload).is_some()
}

fn reserved_internal_tool_context_key_in_payload(payload: &Value) -> Option<&'static str> {
    payload
        .as_object()
        .and_then(reserved_internal_tool_context_key_in_map)
}

pub(crate) fn reserved_internal_tool_context_key_in_map(
    body: &serde_json::Map<String, Value>,
) -> Option<&'static str> {
    if body.contains_key(LOONG_INTERNAL_TOOL_CONTEXT_KEY) {
        Some(LOONG_INTERNAL_TOOL_CONTEXT_KEY)
    } else {
        None
    }
}

pub(crate) fn trusted_internal_tool_context_from_payload(
    payload: &Value,
) -> Option<&serde_json::Map<String, Value>> {
    let body = payload.as_object()?;
    let key = reserved_internal_tool_context_key_in_map(body)?;
    body.get(key)?.as_object()
}

pub(crate) fn take_trusted_internal_tool_context(
    body: &mut serde_json::Map<String, Value>,
) -> serde_json::Map<String, Value> {
    for key in [
        LOONG_INTERNAL_TOOL_CONTEXT_KEY,
        LOONG_INTERNAL_TOOL_CONTEXT_KEY,
    ] {
        let Some(value) = body.remove(key) else {
            continue;
        };
        if let Some(object) = value.as_object() {
            return object.clone();
        }
    }
    serde_json::Map::new()
}

pub(crate) fn ensure_untrusted_payload_does_not_use_reserved_internal_tool_context(
    tool_name: &str,
    payload: &Value,
    payload_path: &str,
) -> Result<(), String> {
    if trusted_internal_tool_payload_enabled() {
        return Ok(());
    }
    let Some(offending_key) = reserved_internal_tool_context_key_in_payload(payload) else {
        return Ok(());
    };

    Err(format!(
        "tool `{tool_name}` {payload_path}.{offending_key} is reserved for trusted internal tool context; retry without that field"
    ))
}
