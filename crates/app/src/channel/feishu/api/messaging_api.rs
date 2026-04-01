use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

use crate::CliResult;
use crate::channel::traits::error::{ApiError, ApiResult};
use crate::channel::traits::messaging::{Message, MessageContent};
use crate::channel::{
    ChannelOutboundTarget, ChannelOutboundTargetKind, ChannelPlatform, ChannelSession,
};

use super::resources::messages::FeishuOutboundMessageBody;
use super::resources::types::FeishuMessageDetail;

/// Generate an idempotency key for Feishu API requests
///
/// Format: `{timestamp_hex}-{pid_hex}-{counter_hex}`
/// This ensures uniqueness across multiple process instances.
pub(crate) fn generate_idempotency_key() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let pid = std::process::id() as u64;
    let counter = COUNTER.fetch_add(1, Ordering::SeqCst);

    format!("{:x}-{:x}-{:x}", timestamp, pid, counter)
}

/// Convert MessageContent to FeishuOutboundMessageBody
///
/// # Arguments
/// * `content` - The generic message content to convert
///
/// # Returns
/// * `Ok(FeishuOutboundMessageBody)` - Successfully converted message body
/// * `Err(ApiError)` - Conversion failed (e.g., unsupported content type)
pub(crate) fn convert_message_content_to_feishu(
    content: &MessageContent,
) -> ApiResult<FeishuOutboundMessageBody> {
    match content {
        MessageContent::Text { text } => Ok(FeishuOutboundMessageBody::Text(text.clone())),
        MessageContent::Markdown { text } => {
            Ok(FeishuOutboundMessageBody::MarkdownCard(text.clone()))
        }
        MessageContent::Rich { content } => Ok(FeishuOutboundMessageBody::Post(content.clone())),
        MessageContent::Image { .. } => Err(ApiError::NotSupported(
            "Image upload not yet supported".to_owned(),
        )),
        MessageContent::File { .. } => Err(ApiError::NotSupported(
            "File upload not yet supported".to_owned(),
        )),
        MessageContent::Audio { .. } => Err(ApiError::NotSupported(
            "Audio upload not yet supported".to_owned(),
        )),
        MessageContent::Media { .. } => Err(ApiError::NotSupported(
            "Media upload not yet supported".to_owned(),
        )),
        MessageContent::ShareChat { chat_id } => {
            Ok(FeishuOutboundMessageBody::ShareChat(chat_id.clone()))
        }
        MessageContent::ShareUser { user_id } => {
            Ok(FeishuOutboundMessageBody::ShareUser(user_id.clone()))
        }
    }
}

/// Convert FeishuMessageDetail to generic Message
///
/// # Arguments
/// * `detail` - The Feishu-specific message detail
///
/// # Returns
/// * `Ok(Message)` - Successfully converted message
/// * `Err(ApiError)` - Conversion failed (e.g., missing required fields)
pub(crate) fn convert_feishu_message_to_generic(detail: FeishuMessageDetail) -> ApiResult<Message> {
    let session = ChannelSession::new(
        ChannelPlatform::Feishu,
        detail.chat_id.clone().unwrap_or_default(),
    );

    // Parse timestamp from unix milliseconds
    let timestamp = parse_feishu_timestamp(detail.create_time.as_deref()).unwrap_or_else(Utc::now);

    // Convert message content based on type
    let content = convert_feishu_body_to_content(detail.message_type.as_deref(), &detail.body)?;

    Ok(Message {
        id: detail.message_id,
        session,
        sender_id: detail.sender_id.unwrap_or_default(),
        content,
        timestamp,
        parent_id: detail.parent_id,
        raw: Some(detail.body),
    })
}

/// Parse Feishu timestamp string to DateTime<Utc>
///
/// Feishu timestamps are typically in unix milliseconds as strings.
pub(crate) fn parse_feishu_timestamp(timestamp_str: Option<&str>) -> Option<DateTime<Utc>> {
    let timestamp_str = timestamp_str?;
    let millis: i64 = timestamp_str.parse().ok()?;
    Utc.timestamp_millis_opt(millis).single()
}

/// Convert Feishu message body to generic MessageContent
///
/// # Arguments
/// * `msg_type` - The Feishu message type
/// * `body` - The raw message body from Feishu
pub(crate) fn convert_feishu_body_to_content(
    msg_type: Option<&str>,
    body: &Value,
) -> ApiResult<MessageContent> {
    match msg_type {
        Some("text") => {
            let text = body
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            Ok(MessageContent::Text { text })
        }
        Some("interactive") => {
            let text = extract_markdown_from_card(body);
            Ok(MessageContent::Markdown { text })
        }
        Some("post") => Ok(MessageContent::Rich {
            content: body.clone(),
        }),
        Some("image") => {
            let url = body
                .get("image_key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            Ok(MessageContent::Image {
                url,
                width: None,
                height: None,
            })
        }
        Some("file") => {
            let name = body
                .get("file_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let url = body
                .get("file_key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            Ok(MessageContent::File {
                name,
                url,
                size: None,
            })
        }
        Some("audio") => {
            let url = body
                .get("file_key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            Ok(MessageContent::Audio {
                url,
                duration: None,
            })
        }
        Some("media") => {
            let url = body
                .get("file_key")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let cover_url = body
                .get("cover_key")
                .and_then(Value::as_str)
                .map(|s| s.to_owned());
            Ok(MessageContent::Media {
                url,
                cover_url,
                duration: None,
            })
        }
        Some("share_chat") => {
            let chat_id = body
                .get("chat_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            Ok(MessageContent::ShareChat { chat_id })
        }
        Some("share_user") => {
            let user_id = body
                .get("user_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            Ok(MessageContent::ShareUser { user_id })
        }
        _ => Ok(MessageContent::Rich {
            content: body.clone(),
        }),
    }
}

/// Extract markdown text from a Feishu card/interactive message
pub(crate) fn extract_markdown_from_card(body: &Value) -> String {
    body.get("card")
        .and_then(|card| card.get("elements"))
        .and_then(Value::as_array)
        .map(|elements| {
            elements
                .iter()
                .filter_map(|el| {
                    el.get("text")
                        .and_then(|t| t.get("content"))
                        .and_then(Value::as_str)
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Convert CliResult to ApiResult
pub(crate) fn convert_cli_result<T>(result: CliResult<T>) -> ApiResult<T> {
    result.map_err(|e| convert_string_error_to_api_error(&e))
}

/// Extract retry_after value from error message
fn extract_retry_after(error: &str) -> Option<u64> {
    // Look for patterns like "retry_after: 60" or "Retry-After: 120"
    let patterns = ["retry_after", "retry-after", "retry after"];
    let error_lower = error.to_lowercase();

    for pattern in patterns {
        if let Some(pos) = error_lower.find(pattern) {
            let after = &error[pos + pattern.len()..];
            // Find the first number after the pattern
            let num_str: String = after
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(secs) = num_str.parse::<u64>() {
                return Some(secs);
            }
        }
    }
    None
}

/// Convert error string to appropriate ApiError variant
pub(crate) fn convert_string_error_to_api_error(error: &str) -> ApiError {
    let error_lower = error.to_lowercase();

    // Check for specific error patterns
    if error_lower.contains("not found")
        || error_lower.contains("does not exist")
        || error_lower.contains("chat_id not exist")
        || error_lower.contains("message_id not exist")
    {
        return ApiError::NotFound(error.to_owned());
    }

    if error_lower.contains("unauthorized")
        || error_lower.contains("authentication")
        || error_lower.contains("token expired")
        || error_lower.contains("invalid token")
    {
        return ApiError::Auth(error.to_owned());
    }

    if error_lower.contains("rate limit")
        || error_lower.contains("frequency limit")
        || error_lower.contains("too many requests")
    {
        // Try to extract retry_after from error message
        let retry_after_secs = extract_retry_after(error).unwrap_or(60);
        return ApiError::RateLimited { retry_after_secs };
    }

    if error_lower.contains("invalid")
        || error_lower.contains("bad request")
        || error_lower.contains("validation")
    {
        return ApiError::InvalidRequest(error.to_owned());
    }

    if error_lower.contains("network")
        || error_lower.contains("timeout")
        || error_lower.contains("connection")
    {
        return ApiError::Network(error.to_owned());
    }

    if error_lower.contains("server error") || error_lower.contains("internal error") {
        return ApiError::Server(error.to_owned());
    }

    // Check if it's a FeishuApiError
    if error_lower.contains("feishu api error") {
        // Try to extract error code
        if let Some(code_str) = error
            .split("error ")
            .nth(1)
            .and_then(|s| s.split(':').next())
            && let Ok(code) = code_str.trim().parse::<i64>()
        {
            // Map common Feishu error codes
            return match code {
                10001 | 10003 | 10011 | 10012 => ApiError::Auth(error.to_owned()),
                10029 | 10031 | 10032 | 10033 => ApiError::NotFound(error.to_owned()),
                99991400 | 99991401 => ApiError::RateLimited {
                    retry_after_secs: 60,
                },
                20001 | 20002 => ApiError::InvalidRequest(error.to_owned()),
                2200 => ApiError::Server(error.to_owned()),
                _ => ApiError::platform_with_code("feishu", code.to_string(), error.to_owned()),
            };
        }
        return ApiError::platform("feishu", error.to_owned());
    }

    // Default to Other
    ApiError::Other(error.to_owned())
}

/// Extract receive_id and receive_id_type from ChannelOutboundTarget
pub(crate) fn extract_receive_params(
    target: &ChannelOutboundTarget,
) -> ApiResult<(String, String)> {
    match target.kind {
        ChannelOutboundTargetKind::ReceiveId => {
            let receive_id_type = target
                .feishu_receive_id_type()
                .unwrap_or("chat_id")
                .to_owned();
            Ok((target.id.clone(), receive_id_type))
        }
        ChannelOutboundTargetKind::MessageReply => {
            // For message reply, we need the message_id
            // The actual reply is handled separately by reply methods
            Err(ApiError::InvalidRequest(
                "Use reply() method for message replies".to_owned(),
            ))
        }
        ChannelOutboundTargetKind::Conversation => {
            // Treat conversation ID as chat_id
            Ok((target.id.clone(), "chat_id".to_owned()))
        }
        ChannelOutboundTargetKind::Address | ChannelOutboundTargetKind::Endpoint => {
            Err(ApiError::NotSupported(format!(
                "Target kind {:?} not supported for Feishu",
                target.kind
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn convert_text_content() {
        let content = MessageContent::Text {
            text: "Hello world".to_owned(),
        };
        let body = convert_message_content_to_feishu(&content).unwrap();
        assert!(matches!(body, FeishuOutboundMessageBody::Text(ref t) if t == "Hello world"));
    }

    #[test]
    fn convert_markdown_content() {
        let content = MessageContent::Markdown {
            text: "**Bold** text".to_owned(),
        };
        let body = convert_message_content_to_feishu(&content).unwrap();
        assert!(
            matches!(body, FeishuOutboundMessageBody::MarkdownCard(ref t) if t == "**Bold** text")
        );
    }

    #[test]
    fn convert_image_content_not_supported() {
        let content = MessageContent::Image {
            url: "http://example.com/image.png".to_owned(),
            width: None,
            height: None,
        };
        let result = convert_message_content_to_feishu(&content);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ApiError::NotSupported(_)));
    }

    #[test]
    fn parse_unix_timestamp_millis() {
        // 2024-01-01 00:00:00 UTC in milliseconds
        let ts = "1704067200000";
        let dt = parse_feishu_timestamp(Some(ts));
        assert!(dt.is_some());
        assert_eq!(dt.unwrap().timestamp(), 1704067200);
    }

    #[test]
    fn convert_text_body_to_content() {
        let body = json!({"text": "hello world"});
        let content = convert_feishu_body_to_content(Some("text"), &body).unwrap();
        assert!(
            matches!(&content, MessageContent::Text { text } if text == "hello world"),
            "expected Text with 'hello world', got {content:?}"
        );
    }

    #[test]
    fn convert_interactive_body_to_markdown() {
        let body = json!({
            "card": {
                "elements": [
                    {
                        "tag": "div",
                        "text": {
                            "tag": "lark_md",
                            "content": "**bold** text"
                        }
                    }
                ]
            }
        });
        let content = convert_feishu_body_to_content(Some("interactive"), &body).unwrap();
        assert!(
            matches!(&content, MessageContent::Markdown { text } if text == "**bold** text"),
            "expected Markdown with '**bold** text', got {content:?}"
        );
    }

    #[test]
    fn error_conversion_auth() {
        let err = convert_string_error_to_api_error("authentication failed: invalid token");
        assert!(matches!(err, ApiError::Auth(_)));
    }

    #[test]
    fn error_conversion_not_found() {
        let err = convert_string_error_to_api_error("chat_id not exist");
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    #[test]
    fn error_conversion_rate_limited() {
        let err = convert_string_error_to_api_error("request trigger frequency limit");
        assert!(matches!(err, ApiError::RateLimited { .. }));
    }
}
