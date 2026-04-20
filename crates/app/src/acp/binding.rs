use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::{normalize_dispatch_account_id, normalize_dispatch_channel_id};

use super::backend::AcpSessionBootstrap;

pub const ACP_BINDING_ROUTE_SESSION_ID_METADATA: &str = "route_session_id";
pub const ACP_BINDING_CHANNEL_ID_METADATA: &str = "channel";
pub const ACP_BINDING_ACCOUNT_ID_METADATA: &str = "channel_account_id";
pub const ACP_BINDING_CONVERSATION_ID_METADATA: &str = "channel_conversation_id";
pub const ACP_BINDING_PARTICIPANT_ID_METADATA: &str = "channel_participant_id";
pub const ACP_BINDING_THREAD_ID_METADATA: &str = "channel_thread_id";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcpSessionBindingScope {
    pub route_session_id: String,
    pub channel_id: Option<String>,
    pub account_id: Option<String>,
    pub conversation_id: Option<String>,
    pub participant_id: Option<String>,
    pub thread_id: Option<String>,
}

impl AcpSessionBindingScope {
    pub fn from_bootstrap(bootstrap: &AcpSessionBootstrap) -> Option<Self> {
        bootstrap
            .binding
            .clone()
            .or_else(|| Self::from_metadata(&bootstrap.metadata))
    }

    pub fn from_metadata(metadata: &BTreeMap<String, String>) -> Option<Self> {
        let route_session_id = metadata
            .get(ACP_BINDING_ROUTE_SESSION_ID_METADATA)
            .and_then(|value| trimmed_non_empty(Some(value.as_str())))?;
        Some(Self {
            route_session_id,
            channel_id: metadata
                .get(ACP_BINDING_CHANNEL_ID_METADATA)
                .and_then(|value| normalize_dispatch_channel_id(value)),
            account_id: metadata
                .get(ACP_BINDING_ACCOUNT_ID_METADATA)
                .and_then(|value| normalize_dispatch_account_id(value)),
            conversation_id: metadata
                .get(ACP_BINDING_CONVERSATION_ID_METADATA)
                .and_then(|value| trimmed_non_empty(Some(value.as_str()))),
            participant_id: metadata
                .get(ACP_BINDING_PARTICIPANT_ID_METADATA)
                .and_then(|value| trimmed_non_empty(Some(value.as_str()))),
            thread_id: metadata
                .get(ACP_BINDING_THREAD_ID_METADATA)
                .and_then(|value| trimmed_non_empty(Some(value.as_str()))),
        })
    }
}

fn trimmed_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::acp::AcpSessionBootstrap;

    use super::AcpSessionBindingScope;

    #[test]
    fn binding_scope_normalizes_route_metadata() {
        let scope = AcpSessionBindingScope::from_metadata(&BTreeMap::from([
            (
                "route_session_id".to_owned(),
                "feishu:lark-prod:oc_123:om_thread_1".to_owned(),
            ),
            ("channel".to_owned(), " Feishu ".to_owned()),
            ("channel_account_id".to_owned(), "LARK PROD".to_owned()),
            ("channel_conversation_id".to_owned(), " oc_123 ".to_owned()),
            (
                "channel_participant_id".to_owned(),
                " ou_sender_1 ".to_owned(),
            ),
            ("channel_thread_id".to_owned(), " om_thread_1 ".to_owned()),
        ]))
        .expect("binding scope should parse");

        assert_eq!(
            scope.route_session_id,
            "feishu:lark-prod:oc_123:om_thread_1"
        );
        assert_eq!(scope.channel_id.as_deref(), Some("feishu"));
        assert_eq!(scope.account_id.as_deref(), Some("lark-prod"));
        assert_eq!(scope.conversation_id.as_deref(), Some("oc_123"));
        assert_eq!(scope.participant_id.as_deref(), Some("ou_sender_1"));
        assert_eq!(scope.thread_id.as_deref(), Some("om_thread_1"));
    }

    #[test]
    fn binding_scope_from_bootstrap_prefers_explicit_binding_over_metadata() {
        let bootstrap = AcpSessionBootstrap {
            session_key: "agent:codex:opaque".to_owned(),
            conversation_id: Some("opaque".to_owned()),
            binding: Some(AcpSessionBindingScope {
                route_session_id: "feishu:lark-prod:oc_123:om_thread_1".to_owned(),
                channel_id: Some("feishu".to_owned()),
                account_id: Some("lark-prod".to_owned()),
                conversation_id: Some("oc_123".to_owned()),
                participant_id: Some("ou_sender_1".to_owned()),
                thread_id: Some("om_thread_1".to_owned()),
            }),
            working_directory: None,
            initial_prompt: None,
            mode: None,
            mcp_servers: Vec::new(),
            metadata: BTreeMap::from([("route_session_id".to_owned(), "telegram:42".to_owned())]),
        };

        let scope = AcpSessionBindingScope::from_bootstrap(&bootstrap)
            .expect("explicit binding should be used");
        assert_eq!(
            scope.route_session_id,
            "feishu:lark-prod:oc_123:om_thread_1"
        );
        assert_eq!(scope.channel_id.as_deref(), Some("feishu"));
    }
}
