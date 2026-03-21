use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::CliResult;

use super::types::FeishuCardUpdateReceipt;
use crate::channel::feishu::client::FeishuClient;

pub fn build_markdown_card(text: &str) -> Value {
    let content = text.trim();
    serde_json::json!({
        "config": {
            "wide_screen_mode": true
        },
        "elements": [{
            "tag": "markdown",
            "content": content
        }]
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeishuCardUpdateRequest {
    pub token: String,
    pub card: Value,
    pub open_ids: Vec<String>,
}

impl FeishuCardUpdateRequest {
    pub fn validate(&self) -> CliResult<()> {
        if self.token.trim().is_empty() {
            return Err("feishu card update requires token".to_owned());
        }
        if !self.card.is_object() {
            return Err("feishu card update requires card object".to_owned());
        }
        Ok(())
    }

    pub fn normalized_open_ids(&self) -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut normalized = Vec::new();
        for value in &self.open_ids {
            let trimmed = value.trim();
            if trimmed.is_empty() || !seen.insert(trimmed.to_owned()) {
                continue;
            }
            normalized.push(trimmed.to_owned());
        }
        normalized
    }

    fn request_body(&self) -> Value {
        let mut body = serde_json::Map::new();
        body.insert(
            "token".to_owned(),
            Value::String(self.token.trim().to_owned()),
        );
        body.insert("card".to_owned(), self.card.clone());
        let open_ids = self.normalized_open_ids();
        if !open_ids.is_empty() {
            body.insert(
                "open_ids".to_owned(),
                Value::Array(open_ids.into_iter().map(Value::String).collect()),
            );
        }
        Value::Object(body)
    }
}

pub async fn delay_update_message_card(
    client: &FeishuClient,
    tenant_access_token: &str,
    request: &FeishuCardUpdateRequest,
) -> CliResult<FeishuCardUpdateReceipt> {
    request.validate()?;
    let payload = client
        .post_json(
            "/open-apis/interactive/v1/card/update",
            Some(tenant_access_token),
            &[],
            &request.request_body(),
        )
        .await?;
    parse_card_update_response(&payload)
}

pub fn parse_card_update_response(payload: &Value) -> CliResult<FeishuCardUpdateReceipt> {
    Ok(FeishuCardUpdateReceipt {
        message: payload
            .get("msg")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_markdown_card_wraps_standard_markdown_layout() {
        assert_eq!(
            build_markdown_card("  approved  "),
            json!({
                "config": {
                    "wide_screen_mode": true
                },
                "elements": [{
                    "tag": "markdown",
                    "content": "approved"
                }]
            })
        );
    }

    #[test]
    fn feishu_card_update_request_requires_token() {
        let error = FeishuCardUpdateRequest {
            token: "  ".to_owned(),
            card: json!({"elements": []}),
            open_ids: Vec::new(),
        }
        .validate()
        .expect_err("missing token should fail");

        assert_eq!(error, "feishu card update requires token");
    }

    #[test]
    fn feishu_card_update_request_requires_card_object() {
        let error = FeishuCardUpdateRequest {
            token: "callback-token-1".to_owned(),
            card: json!("not-an-object"),
            open_ids: Vec::new(),
        }
        .validate()
        .expect_err("non-object card should fail");

        assert_eq!(error, "feishu card update requires card object");
    }

    #[test]
    fn feishu_card_update_request_body_normalizes_open_ids() {
        let request = FeishuCardUpdateRequest {
            token: "callback-token-1".to_owned(),
            card: json!({"elements": [{"tag": "markdown", "content": "done"}]}),
            open_ids: vec![
                "  ou_1 ".to_owned(),
                String::new(),
                "ou_1".to_owned(),
                "ou_2".to_owned(),
            ],
        };

        assert_eq!(request.normalized_open_ids(), vec!["ou_1", "ou_2"]);
        assert_eq!(
            request.request_body(),
            json!({
                "token": "callback-token-1",
                "card": {
                    "elements": [{
                        "tag": "markdown",
                        "content": "done"
                    }]
                },
                "open_ids": ["ou_1", "ou_2"]
            })
        );
    }

    #[test]
    fn feishu_card_update_request_body_omits_empty_open_ids() {
        let request = FeishuCardUpdateRequest {
            token: "callback-token-1".to_owned(),
            card: json!({"elements": []}),
            open_ids: vec![String::new(), "  ".to_owned()],
        };

        assert_eq!(
            request.request_body(),
            json!({
                "token": "callback-token-1",
                "card": {
                    "elements": []
                }
            })
        );
    }

    #[test]
    fn parse_feishu_card_update_response_reads_message() {
        let receipt = parse_card_update_response(&json!({
            "code": 0,
            "msg": "ok"
        }))
        .expect("parse card update response");

        assert_eq!(receipt.message.as_deref(), Some("ok"));
    }
}
