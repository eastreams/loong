use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::CliResult;

use super::super::client::FeishuClient;
use super::cards;
use super::types::{
    FeishuMessageDetail, FeishuMessageHistoryPage, FeishuMessageSummary, FeishuMessageWriteReceipt,
    FeishuSearchMessagePage,
};

#[derive(Debug, Clone, PartialEq)]
pub enum FeishuOutboundMessageBody {
    Text(String),
    MarkdownCard(String),
    Post(Value),
    Image(String),
    File(String),
}

impl FeishuOutboundMessageBody {
    pub fn msg_type(&self) -> &'static str {
        match self {
            Self::Text(_) => "text",
            Self::MarkdownCard(_) => "interactive",
            Self::Post(_) => "post",
            Self::Image(_) => "image",
            Self::File(_) => "file",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuMessageHistoryQuery {
    pub container_id_type: String,
    pub container_id: String,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub sort_type: Option<String>,
    pub page_size: Option<usize>,
    pub page_token: Option<String>,
}

impl FeishuMessageHistoryQuery {
    pub fn validate(&self) -> CliResult<()> {
        if self.container_id_type.trim().is_empty() {
            return Err("feishu message history requires container_id_type".to_owned());
        }
        if self.container_id.trim().is_empty() {
            return Err("feishu message history requires container_id".to_owned());
        }
        Ok(())
    }

    fn query_pairs(&self) -> Vec<(String, String)> {
        let mut pairs = vec![
            (
                "container_id_type".to_owned(),
                self.container_id_type.trim().to_owned(),
            ),
            (
                "container_id".to_owned(),
                self.container_id.trim().to_owned(),
            ),
        ];
        push_optional_query(&mut pairs, "start_time", self.start_time.as_deref());
        push_optional_query(&mut pairs, "end_time", self.end_time.as_deref());
        push_optional_query(&mut pairs, "sort_type", self.sort_type.as_deref());
        if let Some(page_size) = self.page_size {
            pairs.push(("page_size".to_owned(), page_size.to_string()));
        }
        push_optional_query(&mut pairs, "page_token", self.page_token.as_deref());
        pairs
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuSearchMessagesQuery {
    pub user_id_type: Option<String>,
    pub page_size: Option<usize>,
    pub page_token: Option<String>,
    pub query: String,
    pub from_ids: Vec<String>,
    pub chat_ids: Vec<String>,
    pub message_type: Option<String>,
    pub at_chatter_ids: Vec<String>,
    pub from_type: Option<String>,
    pub chat_type: Option<String>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
}

impl FeishuSearchMessagesQuery {
    pub fn validate(&self) -> CliResult<()> {
        if self.query.trim().is_empty() {
            return Err("feishu message search requires query".to_owned());
        }
        Ok(())
    }

    fn query_pairs(&self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        push_optional_query(&mut pairs, "user_id_type", self.user_id_type.as_deref());
        if let Some(page_size) = self.page_size {
            pairs.push(("page_size".to_owned(), page_size.to_string()));
        }
        push_optional_query(&mut pairs, "page_token", self.page_token.as_deref());
        pairs
    }

    fn request_body(&self) -> Value {
        let mut body = serde_json::Map::new();
        body.insert(
            "query".to_owned(),
            Value::String(self.query.trim().to_owned()),
        );
        insert_string_array(&mut body, "from_ids", &self.from_ids);
        insert_string_array(&mut body, "chat_ids", &self.chat_ids);
        insert_string_array(&mut body, "at_chatter_ids", &self.at_chatter_ids);
        insert_optional_string(&mut body, "message_type", self.message_type.as_deref());
        insert_optional_string(&mut body, "from_type", self.from_type.as_deref());
        insert_optional_string(&mut body, "chat_type", self.chat_type.as_deref());
        insert_optional_string(&mut body, "start_time", self.start_time.as_deref());
        insert_optional_string(&mut body, "end_time", self.end_time.as_deref());
        Value::Object(body)
    }
}

pub async fn fetch_message_history(
    client: &FeishuClient,
    tenant_access_token: &str,
    query: &FeishuMessageHistoryQuery,
) -> CliResult<FeishuMessageHistoryPage> {
    query.validate()?;
    let payload = client
        .get_json(
            "/open-apis/im/v1/messages",
            Some(tenant_access_token),
            &query.query_pairs(),
        )
        .await?;
    parse_message_history_response(&payload)
}

pub async fn fetch_message_detail(
    client: &FeishuClient,
    tenant_access_token: &str,
    message_id: &str,
) -> CliResult<FeishuMessageDetail> {
    let payload = client
        .get_json(
            format!("/open-apis/im/v1/messages/{}", message_id.trim()).as_str(),
            Some(tenant_access_token),
            &[],
        )
        .await?;
    parse_message_detail_response(message_id, &payload)
}

pub async fn search_messages(
    client: &FeishuClient,
    user_access_token: &str,
    query: &FeishuSearchMessagesQuery,
) -> CliResult<FeishuSearchMessagePage> {
    query.validate()?;
    let payload = client
        .post_json(
            "/open-apis/search/v2/message",
            Some(user_access_token),
            &query.query_pairs(),
            &query.request_body(),
        )
        .await?;
    parse_search_messages_response(&payload)
}

pub async fn send_text_message(
    client: &FeishuClient,
    tenant_access_token: &str,
    receive_id_type: &str,
    receive_id: &str,
    text: &str,
    uuid: Option<&str>,
) -> CliResult<FeishuMessageWriteReceipt> {
    send_message(
        client,
        tenant_access_token,
        receive_id_type,
        receive_id,
        "text",
        serde_json::json!({"text": require_non_empty("feishu message send", "text", text)?}),
        uuid,
    )
    .await
}

pub async fn send_markdown_card_message(
    client: &FeishuClient,
    tenant_access_token: &str,
    receive_id_type: &str,
    receive_id: &str,
    text: &str,
    uuid: Option<&str>,
) -> CliResult<FeishuMessageWriteReceipt> {
    send_message(
        client,
        tenant_access_token,
        receive_id_type,
        receive_id,
        "interactive",
        cards::build_markdown_card(
            require_non_empty("feishu message send", "text", text)?.as_str(),
        ),
        uuid,
    )
    .await
}

pub async fn send_outbound_message(
    client: &FeishuClient,
    tenant_access_token: &str,
    receive_id_type: &str,
    receive_id: &str,
    body: &FeishuOutboundMessageBody,
    uuid: Option<&str>,
) -> CliResult<FeishuMessageWriteReceipt> {
    match body {
        FeishuOutboundMessageBody::Text(text) => {
            send_text_message(
                client,
                tenant_access_token,
                receive_id_type,
                receive_id,
                text,
                uuid,
            )
            .await
        }
        FeishuOutboundMessageBody::MarkdownCard(text) => {
            send_markdown_card_message(
                client,
                tenant_access_token,
                receive_id_type,
                receive_id,
                text,
                uuid,
            )
            .await
        }
        FeishuOutboundMessageBody::Post(content) => {
            send_message(
                client,
                tenant_access_token,
                receive_id_type,
                receive_id,
                "post",
                content.clone(),
                uuid,
            )
            .await
        }
        FeishuOutboundMessageBody::Image(image_key) => {
            send_message(
                client,
                tenant_access_token,
                receive_id_type,
                receive_id,
                "image",
                serde_json::json!({"image_key": image_key}),
                uuid,
            )
            .await
        }
        FeishuOutboundMessageBody::File(file_key) => {
            send_message(
                client,
                tenant_access_token,
                receive_id_type,
                receive_id,
                "file",
                serde_json::json!({"file_key": file_key}),
                uuid,
            )
            .await
        }
    }
}

pub async fn reply_text_message(
    client: &FeishuClient,
    tenant_access_token: &str,
    message_id: &str,
    text: &str,
    reply_in_thread: bool,
    uuid: Option<&str>,
) -> CliResult<FeishuMessageWriteReceipt> {
    reply_message(
        client,
        tenant_access_token,
        message_id,
        "text",
        serde_json::json!({"text": require_non_empty("feishu message reply", "text", text)?}),
        reply_in_thread,
        uuid,
    )
    .await
}

pub async fn reply_markdown_card_message(
    client: &FeishuClient,
    tenant_access_token: &str,
    message_id: &str,
    text: &str,
    reply_in_thread: bool,
    uuid: Option<&str>,
) -> CliResult<FeishuMessageWriteReceipt> {
    reply_message(
        client,
        tenant_access_token,
        message_id,
        "interactive",
        cards::build_markdown_card(
            require_non_empty("feishu message reply", "text", text)?.as_str(),
        ),
        reply_in_thread,
        uuid,
    )
    .await
}

pub async fn reply_outbound_message(
    client: &FeishuClient,
    tenant_access_token: &str,
    message_id: &str,
    body: &FeishuOutboundMessageBody,
    reply_in_thread: bool,
    uuid: Option<&str>,
) -> CliResult<FeishuMessageWriteReceipt> {
    match body {
        FeishuOutboundMessageBody::Text(text) => {
            reply_text_message(
                client,
                tenant_access_token,
                message_id,
                text,
                reply_in_thread,
                uuid,
            )
            .await
        }
        FeishuOutboundMessageBody::MarkdownCard(text) => {
            reply_markdown_card_message(
                client,
                tenant_access_token,
                message_id,
                text,
                reply_in_thread,
                uuid,
            )
            .await
        }
        FeishuOutboundMessageBody::Post(content) => {
            reply_message(
                client,
                tenant_access_token,
                message_id,
                "post",
                content.clone(),
                reply_in_thread,
                uuid,
            )
            .await
        }
        FeishuOutboundMessageBody::Image(image_key) => {
            reply_message(
                client,
                tenant_access_token,
                message_id,
                "image",
                serde_json::json!({"image_key": image_key}),
                reply_in_thread,
                uuid,
            )
            .await
        }
        FeishuOutboundMessageBody::File(file_key) => {
            reply_message(
                client,
                tenant_access_token,
                message_id,
                "file",
                serde_json::json!({"file_key": file_key}),
                reply_in_thread,
                uuid,
            )
            .await
        }
    }
}

pub fn resolve_outbound_message_body(
    action: &str,
    text_field: &str,
    as_card_field: &str,
    post_field: &str,
    image_key_field: &str,
    file_key_field: &str,
    text: Option<&str>,
    as_card: bool,
    post: Option<&Value>,
    image_key: Option<&str>,
    file_key: Option<&str>,
) -> CliResult<FeishuOutboundMessageBody> {
    let text = text.map(str::trim).filter(|value| !value.is_empty());
    let image_key = image_key.map(str::trim).filter(|value| !value.is_empty());
    let file_key = file_key.map(str::trim).filter(|value| !value.is_empty());
    let mut provided_fields = Vec::new();
    if text.is_some() {
        provided_fields.push(text_field);
    }
    if post.is_some() {
        provided_fields.push(post_field);
    }
    if image_key.is_some() {
        provided_fields.push(image_key_field);
    }
    if file_key.is_some() {
        provided_fields.push(file_key_field);
    }

    if provided_fields.is_empty() {
        return Err(format!(
            "{action} requires {text_field}, {post_field}, {image_key_field}, or {file_key_field}"
        ));
    }
    if provided_fields.len() > 1 {
        return Err(format!(
            "{action} accepts exactly one of {}, not both",
            provided_fields.join(", ")
        ));
    }

    if as_card && text.is_none() {
        let provided_field = provided_fields.first().copied().unwrap_or("payload.post");
        return Err(format!(
            "{action} does not allow {as_card_field} with {}",
            provided_field
        ));
    }

    match (text, post, image_key, file_key) {
        (Some(text), None, None, None) => {
            if as_card {
                Ok(FeishuOutboundMessageBody::MarkdownCard(text.to_owned()))
            } else {
                Ok(FeishuOutboundMessageBody::Text(text.to_owned()))
            }
        }
        (None, Some(post), None, None) => Ok(FeishuOutboundMessageBody::Post(
            normalize_post_message_content(action, post_field, post)?,
        )),
        (None, None, Some(image_key), None) => {
            Ok(FeishuOutboundMessageBody::Image(image_key.to_owned()))
        }
        (None, None, None, Some(file_key)) => {
            Ok(FeishuOutboundMessageBody::File(file_key.to_owned()))
        }
        _ => Err(format!(
            "{action} accepts exactly one of {text_field}, {post_field}, {image_key_field}, or {file_key_field}"
        )),
    }
}

pub fn parse_message_history_response(payload: &Value) -> CliResult<FeishuMessageHistoryPage> {
    let data = payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "feishu message history payload missing data object".to_owned())?;
    let items = data
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(parse_message_summary)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(FeishuMessageHistoryPage {
        has_more: data
            .get("has_more")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        page_token: opt_string(data.get("page_token")),
        items,
    })
}

pub fn parse_message_detail_response(
    message_id: &str,
    payload: &Value,
) -> CliResult<FeishuMessageDetail> {
    let data = payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "feishu message detail payload missing data object".to_owned())?;
    let item = data
        .get("items")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));
    let object = item
        .as_object()
        .ok_or_else(|| "feishu message detail item is not an object".to_owned())?;

    Ok(FeishuMessageDetail {
        message_id: opt_string(object.get("message_id"))
            .unwrap_or_else(|| message_id.trim().to_owned()),
        chat_id: opt_string(object.get("chat_id")),
        root_id: opt_string(object.get("root_id")),
        parent_id: opt_string(object.get("parent_id")),
        message_type: opt_string(object.get("msg_type")),
        create_time: opt_string(object.get("create_time")),
        update_time: opt_string(object.get("update_time")),
        deleted: object.get("deleted").and_then(Value::as_bool),
        updated: object.get("updated").and_then(Value::as_bool),
        sender_id: object
            .get("sender")
            .and_then(|value| value.get("id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        sender_type: object
            .get("sender")
            .and_then(|value| value.get("sender_type"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        body: item,
    })
}

pub fn parse_search_messages_response(payload: &Value) -> CliResult<FeishuSearchMessagePage> {
    let data = payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "feishu message search payload missing data object".to_owned())?;
    let items = data
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(FeishuSearchMessagePage {
        has_more: data
            .get("has_more")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        page_token: opt_string(data.get("page_token")),
        items,
    })
}

pub fn parse_message_write_response(payload: &Value) -> CliResult<FeishuMessageWriteReceipt> {
    let data = payload
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| "feishu message write payload missing data object".to_owned())?;
    let message_id = opt_string(data.get("message_id"))
        .ok_or_else(|| "feishu message write payload missing data.message_id".to_owned())?;

    Ok(FeishuMessageWriteReceipt {
        message_id,
        root_id: opt_string(data.get("root_id")),
        parent_id: opt_string(data.get("parent_id")),
    })
}

fn parse_message_summary(item: &Value) -> Option<FeishuMessageSummary> {
    let object = item.as_object()?;
    Some(FeishuMessageSummary {
        message_id: opt_string(object.get("message_id"))?,
        chat_id: opt_string(object.get("chat_id")),
        root_id: opt_string(object.get("root_id")),
        parent_id: opt_string(object.get("parent_id")),
        message_type: opt_string(object.get("msg_type")),
        create_time: opt_string(object.get("create_time")),
        update_time: opt_string(object.get("update_time")),
    })
}

fn opt_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn push_optional_query(pairs: &mut Vec<(String, String)>, key: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        pairs.push((key.to_owned(), value.to_owned()));
    }
}

fn insert_optional_string(
    body: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<&str>,
) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        body.insert(key.to_owned(), Value::String(value.to_owned()));
    }
}

fn insert_optional_bool(body: &mut serde_json::Map<String, Value>, key: &str, value: Option<bool>) {
    if let Some(value) = value.filter(|value| *value) {
        body.insert(key.to_owned(), Value::Bool(value));
    }
}

fn insert_string_array(body: &mut serde_json::Map<String, Value>, key: &str, values: &[String]) {
    let normalized = values
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| Value::String(value.to_owned()))
        .collect::<Vec<_>>();
    if !normalized.is_empty() {
        body.insert(key.to_owned(), Value::Array(normalized));
    }
}

async fn send_message(
    client: &FeishuClient,
    tenant_access_token: &str,
    receive_id_type: &str,
    receive_id: &str,
    msg_type: &str,
    content: Value,
    uuid: Option<&str>,
) -> CliResult<FeishuMessageWriteReceipt> {
    let receive_id = require_non_empty("feishu message send", "receive_id", receive_id)?;
    let receive_id_type =
        require_non_empty("feishu message send", "receive_id_type", receive_id_type)?;
    let msg_type = require_non_empty("feishu message send", "msg_type", msg_type)?;
    let mut body = serde_json::Map::new();
    body.insert("receive_id".to_owned(), Value::String(receive_id));
    body.insert("msg_type".to_owned(), Value::String(msg_type));
    body.insert(
        "content".to_owned(),
        Value::String(encode_feishu_content(&content)?),
    );
    insert_optional_string(&mut body, "uuid", uuid);
    let payload = client
        .post_json(
            "/open-apis/im/v1/messages",
            Some(tenant_access_token),
            &[("receive_id_type".to_owned(), receive_id_type)],
            &Value::Object(body),
        )
        .await?;
    parse_message_write_response(&payload)
}

async fn reply_message(
    client: &FeishuClient,
    tenant_access_token: &str,
    message_id: &str,
    msg_type: &str,
    content: Value,
    reply_in_thread: bool,
    uuid: Option<&str>,
) -> CliResult<FeishuMessageWriteReceipt> {
    let message_id = require_non_empty("feishu message reply", "message_id", message_id)?;
    let msg_type = require_non_empty("feishu message reply", "msg_type", msg_type)?;
    let mut body = serde_json::Map::new();
    body.insert("msg_type".to_owned(), Value::String(msg_type));
    body.insert(
        "content".to_owned(),
        Value::String(encode_feishu_content(&content)?),
    );
    insert_optional_bool(&mut body, "reply_in_thread", Some(reply_in_thread));
    insert_optional_string(&mut body, "uuid", uuid);
    let payload = client
        .post_json(
            format!("/open-apis/im/v1/messages/{message_id}/reply").as_str(),
            Some(tenant_access_token),
            &[],
            &Value::Object(body),
        )
        .await?;
    parse_message_write_response(&payload)
}

fn normalize_post_message_content(
    action: &str,
    post_field: &str,
    post: &Value,
) -> CliResult<Value> {
    let locales = post
        .as_object()
        .ok_or_else(|| format!("{action} requires {post_field} to be a JSON object"))?;
    if locales.is_empty() {
        return Err(format!(
            "{action} requires {post_field} to include at least one locale"
        ));
    }

    let has_content = locales.values().any(|locale| {
        locale
            .as_object()
            .and_then(|value| value.get("content"))
            .and_then(Value::as_array)
            .is_some_and(|paragraphs| !paragraphs.is_empty())
    });
    if !has_content {
        return Err(format!(
            "{action} requires {post_field} to include at least one locale with non-empty content"
        ));
    }

    Ok(post.clone())
}

fn require_non_empty(action: &str, field: &str, value: &str) -> CliResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{action} requires {field}"));
    }
    Ok(trimmed.to_owned())
}

fn encode_feishu_content(content: &Value) -> CliResult<String> {
    serde_json::to_string(content).map_err(|error| format!("feishu content encode failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn message_history_query_requires_container_id() {
        let query = FeishuMessageHistoryQuery {
            container_id_type: "chat".to_owned(),
            container_id: String::new(),
            start_time: None,
            end_time: None,
            sort_type: None,
            page_size: Some(20),
            page_token: None,
        };

        let error = query.validate().expect_err("missing container should fail");
        assert!(error.contains("container_id"));
    }

    #[test]
    fn search_message_response_parses_message_ids_and_has_more_flag() {
        let payload = serde_json::json!({
            "code": 0,
            "msg": "success",
            "data": {
                "items": ["om_1", "om_2"],
                "page_token": "next-page",
                "has_more": true
            }
        });

        let page = parse_search_messages_response(&payload).expect("parse search response");
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.page_token.as_deref(), Some("next-page"));
        assert!(page.has_more);
    }

    #[test]
    fn resolve_outbound_message_body_accepts_post_content() {
        let body = resolve_outbound_message_body(
            "feishu message send",
            "payload.text",
            "payload.as_card",
            "payload.post",
            "payload.image_key",
            "payload.file_key",
            None,
            false,
            Some(&json!({
                "zh_cn": {
                    "title": "Ship update",
                    "content": [[{
                        "tag": "text",
                        "text": "rich ship"
                    }]]
                }
            })),
            None,
            None,
        )
        .expect("post content should validate");

        assert_eq!(body.msg_type(), "post");
    }

    #[test]
    fn resolve_outbound_message_body_accepts_image_key() {
        let body = resolve_outbound_message_body(
            "feishu message send",
            "payload.text",
            "payload.as_card",
            "payload.post",
            "payload.image_key",
            "payload.file_key",
            None,
            false,
            None,
            Some("img_v2_demo"),
            None,
        )
        .expect("image key should validate");

        assert_eq!(body.msg_type(), "image");
    }

    #[test]
    fn resolve_outbound_message_body_accepts_file_key() {
        let body = resolve_outbound_message_body(
            "feishu message reply",
            "payload.text",
            "payload.as_card",
            "payload.post",
            "payload.image_key",
            "payload.file_key",
            None,
            false,
            None,
            None,
            Some("file_v2_demo"),
        )
        .expect("file key should validate");

        assert_eq!(body.msg_type(), "file");
    }

    #[test]
    fn resolve_outbound_message_body_rejects_mixed_text_and_post_content() {
        let error = resolve_outbound_message_body(
            "feishu message send",
            "payload.text",
            "payload.as_card",
            "payload.post",
            "payload.image_key",
            "payload.file_key",
            Some("plain text"),
            false,
            Some(&json!({
                "zh_cn": {
                    "title": "Ship update",
                    "content": [[{
                        "tag": "text",
                        "text": "rich ship"
                    }]]
                }
            })),
            None,
            None,
        )
        .expect_err("mixed text and post content should fail");

        assert!(error.contains("payload.text"));
        assert!(error.contains("payload.post"));
        assert!(error.contains("not both"));
    }

    #[test]
    fn resolve_outbound_message_body_rejects_mixed_post_and_image_key() {
        let error = resolve_outbound_message_body(
            "feishu message send",
            "payload.text",
            "payload.as_card",
            "payload.post",
            "payload.image_key",
            "payload.file_key",
            None,
            false,
            Some(&json!({
                "zh_cn": {
                    "title": "Ship update",
                    "content": [[{
                        "tag": "text",
                        "text": "rich ship"
                    }]]
                }
            })),
            Some("img_v2_demo"),
            None,
        )
        .expect_err("mixed post and image key should fail");

        assert!(error.contains("payload.post"));
        assert!(error.contains("payload.image_key"));
        assert!(error.contains("not both"));
    }
}
