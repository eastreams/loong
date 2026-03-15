#![cfg(test)]

use serde_json::{Value, json};

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

fn encode_feishu_content(content: &Value) -> CliResult<String> {
    serde_json::to_string(content).map_err(|error| format!("feishu content encode failed: {error}"))
}
