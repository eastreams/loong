use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::CliResult;

use super::client::FeishuUserInfo;
use super::principal::{FeishuGrantScopeSet, FeishuUserPrincipal};
use super::token_store::FeishuGrant;

const FEISHU_AUTHORIZE_URL: &str = "https://accounts.feishu.cn/open-apis/authen/v1/authorize";
pub const FEISHU_MESSAGE_WRITE_ACCEPTED_SCOPES: &[&str] =
    &["im:message", "im:message:send_as_bot", "im:message:send"];
pub const FEISHU_MESSAGE_WRITE_RECOMMENDED_SCOPES: &[&str] =
    &["im:message", "im:message:send_as_bot"];
pub const FEISHU_DOC_WRITE_ACCEPTED_SCOPES: &[&str] = &["docx:document"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuAuthStartSpec {
    pub app_id: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub state: String,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuTokenExchangeRequest {
    pub code: String,
    pub redirect_uri: Option<String>,
    pub code_verifier: Option<String>,
    pub scopes: FeishuGrantScopeSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuGrantStatus {
    pub has_grant: bool,
    pub access_token_expired: bool,
    pub refresh_token_expired: bool,
    pub missing_scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuGrantAnyScopeStatus {
    pub ready: bool,
    pub accepted_scopes: Vec<String>,
    pub matched_scopes: Vec<String>,
}

pub fn build_authorize_url(spec: &FeishuAuthStartSpec) -> CliResult<String> {
    let mut url = Url::parse(FEISHU_AUTHORIZE_URL)
        .map_err(|error| format!("build authorize url failed: {error}"))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("client_id", spec.app_id.trim());
        query.append_pair("response_type", "code");
        query.append_pair("redirect_uri", spec.redirect_uri.trim());
        if !spec.scopes.is_empty() {
            query.append_pair(
                "scope",
                FeishuGrantScopeSet::from_scopes(spec.scopes.clone())
                    .to_scope_csv()
                    .as_str(),
            );
        }
        if !spec.state.trim().is_empty() {
            query.append_pair("state", spec.state.trim());
        }
        if let Some(code_challenge) = spec
            .code_challenge
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            query.append_pair("code_challenge", code_challenge);
        }
        if let Some(code_challenge_method) = spec
            .code_challenge_method
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            query.append_pair("code_challenge_method", code_challenge_method);
        }
    }
    Ok(url.to_string())
}

pub fn parse_token_exchange_response(
    payload: &Value,
    now_s: i64,
    principal: FeishuUserPrincipal,
) -> CliResult<FeishuGrant> {
    let access_token = required_string(payload, "access_token")?;
    let refresh_token = required_string(payload, "refresh_token")?;
    let expires_in = payload
        .get("expires_in")
        .and_then(Value::as_i64)
        .ok_or_else(|| "feishu token payload missing expires_in".to_owned())?;
    let refresh_token_expires_in = payload
        .get("refresh_token_expires_in")
        .and_then(Value::as_i64)
        .ok_or_else(|| "feishu token payload missing refresh_token_expires_in".to_owned())?;
    let scope_csv = payload
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or_default();

    Ok(FeishuGrant {
        principal,
        access_token,
        refresh_token,
        scopes: FeishuGrantScopeSet::from_scopes(scope_csv.split_whitespace()),
        access_expires_at_s: now_s + expires_in,
        refresh_expires_at_s: now_s + refresh_token_expires_in,
        refreshed_at_s: now_s,
    })
}

pub fn map_user_info_to_principal(
    account_id: &str,
    user_info: &FeishuUserInfo,
) -> CliResult<FeishuUserPrincipal> {
    let open_id = user_info
        .open_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "feishu user info missing open_id".to_owned())?;
    Ok(FeishuUserPrincipal {
        account_id: account_id.trim().to_owned(),
        open_id: open_id.to_owned(),
        union_id: user_info.union_id.clone(),
        user_id: user_info.user_id.clone(),
        name: user_info.name.clone(),
        tenant_key: user_info.tenant_key.clone(),
        avatar_url: user_info.avatar_url.clone(),
        email: user_info.email.clone(),
        enterprise_email: user_info.enterprise_email.clone(),
    })
}

pub fn summarize_grant_status(
    grant: Option<&FeishuGrant>,
    now_s: i64,
    required_scopes: &[String],
) -> FeishuGrantStatus {
    let Some(grant) = grant else {
        return FeishuGrantStatus {
            has_grant: false,
            access_token_expired: true,
            refresh_token_expired: true,
            missing_scopes: required_scopes.to_vec(),
        };
    };

    let missing_scopes = required_scopes
        .iter()
        .filter(|scope| !grant.scopes.contains(scope))
        .cloned()
        .collect::<Vec<_>>();

    FeishuGrantStatus {
        has_grant: true,
        access_token_expired: grant.is_access_token_expired(now_s),
        refresh_token_expired: grant.is_refresh_token_expired(now_s),
        missing_scopes,
    }
}

fn required_string(payload: &Value, field: &str) -> CliResult<String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("feishu token payload missing {field}"))
}

pub fn summarize_message_write_scope_status(
    grant: Option<&FeishuGrant>,
) -> FeishuGrantAnyScopeStatus {
    summarize_any_scope_status(grant, FEISHU_MESSAGE_WRITE_ACCEPTED_SCOPES)
}

pub fn summarize_doc_write_scope_status(grant: Option<&FeishuGrant>) -> FeishuGrantAnyScopeStatus {
    summarize_any_scope_status(grant, FEISHU_DOC_WRITE_ACCEPTED_SCOPES)
}

fn summarize_any_scope_status(
    grant: Option<&FeishuGrant>,
    accepted: &[&str],
) -> FeishuGrantAnyScopeStatus {
    let accepted_scopes = accepted
        .iter()
        .copied()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let matched_scopes = grant
        .map(|grant| {
            accepted_scopes
                .iter()
                .filter(|scope| grant.scopes.contains(scope))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    FeishuGrantAnyScopeStatus {
        ready: !matched_scopes.is_empty(),
        accepted_scopes,
        matched_scopes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_grant(scopes: &[&str]) -> FeishuGrant {
        FeishuGrant {
            principal: FeishuUserPrincipal {
                account_id: "feishu_main".to_owned(),
                open_id: "ou_123".to_owned(),
                union_id: Some("on_456".to_owned()),
                user_id: Some("u_789".to_owned()),
                name: Some("Alice".to_owned()),
                tenant_key: Some("tenant_x".to_owned()),
                avatar_url: None,
                email: None,
                enterprise_email: None,
            },
            access_token: "u-token".to_owned(),
            refresh_token: "r-token".to_owned(),
            scopes: FeishuGrantScopeSet::from_scopes(scopes.iter().copied()),
            access_expires_at_s: 1_700_007_200,
            refresh_expires_at_s: 1_700_086_400,
            refreshed_at_s: 1_700_000_000,
        }
    }

    #[test]
    fn auth_start_builds_feishu_authorize_url_with_state_and_scopes() {
        let spec = FeishuAuthStartSpec {
            app_id: "cli_xxx".to_owned(),
            redirect_uri: "http://127.0.0.1:34819/callback".to_owned(),
            scopes: vec![
                "offline_access".to_owned(),
                "docx:document:readonly".to_owned(),
            ],
            state: "state-123".to_owned(),
            code_challenge: None,
            code_challenge_method: None,
        };

        let url = build_authorize_url(&spec).expect("build authorize url");

        assert!(url.contains("https://accounts.feishu.cn/open-apis/authen/v1/authorize"));
        assert!(url.contains("client_id=cli_xxx"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("state=state-123"));
    }

    #[test]
    fn exchange_response_converts_token_payload_into_grant() {
        let payload = serde_json::json!({
            "code": 0,
            "access_token": "u-token",
            "refresh_token": "r-token",
            "expires_in": 7200,
            "refresh_token_expires_in": 2592000,
            "scope": "offline_access docx:document:readonly",
            "token_type": "Bearer"
        });

        let grant = parse_token_exchange_response(
            &payload,
            1_700_000_000,
            FeishuUserPrincipal {
                account_id: "feishu_main".to_owned(),
                open_id: "ou_123".to_owned(),
                union_id: Some("on_456".to_owned()),
                user_id: Some("u_789".to_owned()),
                name: Some("Alice".to_owned()),
                tenant_key: Some("tenant_x".to_owned()),
                avatar_url: None,
                email: None,
                enterprise_email: None,
            },
        )
        .expect("parse grant");

        assert_eq!(grant.access_token, "u-token");
        assert!(grant.scopes.iter().any(|scope| scope == "offline_access"));
        assert_eq!(grant.access_expires_at_s, 1_700_007_200);
    }

    #[test]
    fn message_write_scope_status_accepts_any_supported_write_scope() {
        let grant = sample_grant(&["offline_access", "im:message:send_as_bot"]);

        let status = summarize_message_write_scope_status(Some(&grant));

        assert!(status.ready);
        assert_eq!(status.matched_scopes, vec!["im:message:send_as_bot"]);
        assert_eq!(
            status.accepted_scopes,
            vec![
                "im:message".to_owned(),
                "im:message:send_as_bot".to_owned(),
                "im:message:send".to_owned()
            ]
        );
    }

    #[test]
    fn message_write_scope_status_reports_not_ready_without_supported_scope() {
        let grant = sample_grant(&["offline_access", "im:message:readonly"]);

        let status = summarize_message_write_scope_status(Some(&grant));

        assert!(!status.ready);
        assert!(status.matched_scopes.is_empty());
    }

    #[test]
    fn doc_write_scope_status_reports_ready_with_docx_write_scope() {
        let grant = sample_grant(&["offline_access", "docx:document"]);

        let status = summarize_doc_write_scope_status(Some(&grant));

        assert!(status.ready);
        assert_eq!(status.matched_scopes, vec!["docx:document"]);
    }

    #[test]
    fn doc_write_scope_status_reports_not_ready_without_docx_write_scope() {
        let grant = sample_grant(&["offline_access", "docx:document:readonly"]);

        let status = summarize_doc_write_scope_status(Some(&grant));

        assert!(!status.ready);
        assert!(status.matched_scopes.is_empty());
    }
}
