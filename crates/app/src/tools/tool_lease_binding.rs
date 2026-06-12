use serde_json::Value;

pub(crate) const TOOL_LEASE_TOKEN_ID_FIELD: &str = "_lease_token_id";
pub(crate) const TOOL_LEASE_SESSION_ID_FIELD: &str = "_lease_session_id";
pub(crate) const TOOL_LEASE_TURN_ID_FIELD: &str = "_lease_turn_id";

#[derive(Debug, Clone, Default)]
pub(crate) struct ToolLeaseBinding {
    pub(crate) token_id: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) turn_id: Option<String>,
}

pub(crate) fn inject_tool_lease_binding(
    payload: &mut serde_json::Map<String, Value>,
    token_id: Option<&str>,
    session_id: Option<&str>,
    turn_id: Option<&str>,
) {
    if let Some(token_id) = token_id {
        payload.insert(
            TOOL_LEASE_TOKEN_ID_FIELD.to_owned(),
            Value::String(token_id.to_owned()),
        );
    }
    if let Some(session_id) = session_id {
        payload.insert(
            TOOL_LEASE_SESSION_ID_FIELD.to_owned(),
            Value::String(session_id.to_owned()),
        );
    }
    if let Some(turn_id) = turn_id {
        payload.insert(
            TOOL_LEASE_TURN_ID_FIELD.to_owned(),
            Value::String(turn_id.to_owned()),
        );
    }
}

pub(crate) fn extract_tool_lease_binding(
    payload: &serde_json::Map<String, Value>,
) -> ToolLeaseBinding {
    let token_id = payload
        .get(TOOL_LEASE_TOKEN_ID_FIELD)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let session_id = payload
        .get(TOOL_LEASE_SESSION_ID_FIELD)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let turn_id = payload
        .get(TOOL_LEASE_TURN_ID_FIELD)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    ToolLeaseBinding {
        token_id,
        session_id,
        turn_id,
    }
}
