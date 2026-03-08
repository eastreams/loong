use serde_json::{json, Value};

use crate::CliResult;

pub(in crate::channel::feishu) fn build_feishu_send_payload(
    receive_id: &str,
    msg_type: &str,
    content: Value,
) -> CliResult<Value> {
    let receive_id = receive_id.trim();
    if receive_id.is_empty() {
        return Err("feishu receive_id is empty".to_owned());
    }

    let msg_type = msg_type.trim();
    if msg_type.is_empty() {
        return Err("feishu msg_type is empty".to_owned());
    }

    Ok(json!({
        "receive_id": receive_id,
        "msg_type": msg_type,
        "content": encode_feishu_content(&content)?,
    }))
}

pub(in crate::channel::feishu) fn build_feishu_reply_payload(
    msg_type: &str,
    content: Value,
) -> CliResult<Value> {
    let msg_type = msg_type.trim();
    if msg_type.is_empty() {
        return Err("feishu reply msg_type is empty".to_owned());
    }

    Ok(json!({
        "msg_type": msg_type,
        "content": encode_feishu_content(&content)?,
    }))
}

pub(in crate::channel::feishu) fn ensure_feishu_response_ok(
    action: &str,
    payload: &Value,
) -> CliResult<()> {
    let code = payload.get("code").and_then(Value::as_i64).unwrap_or(-1);
    if code != 0 {
        return Err(format!("{action} returned code {code}: {payload}"));
    }
    Ok(())
}

fn encode_feishu_content(content: &Value) -> CliResult<String> {
    serde_json::to_string(content).map_err(|error| format!("feishu content encode failed: {error}"))
}
