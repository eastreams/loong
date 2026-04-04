use serde::{Deserialize, Serialize};

const FEISHU_GROUP_MESSAGE_READ_SCOPE: &str = "im:message.group_msg";
const FEISHU_GROUP_MESSAGE_READ_SCOPE_LEGACY: &str = "im:message.group_msg:readonly";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuAccountBinding {
    pub account_id: String,
    pub label: String,
}

impl FeishuAccountBinding {
    pub fn new(account_id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            account_id: account_id.into(),
            label: label.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeishuUserPrincipal {
    pub account_id: String,
    pub open_id: String,
    pub union_id: Option<String>,
    pub user_id: Option<String>,
    pub name: Option<String>,
    pub tenant_key: Option<String>,
    pub avatar_url: Option<String>,
    pub email: Option<String>,
    pub enterprise_email: Option<String>,
}

impl FeishuUserPrincipal {
    pub fn storage_key(&self) -> String {
        format!("{}:{}", self.account_id.trim(), self.open_id.trim())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FeishuGrantScopeSet {
    scopes: Vec<String>,
}

impl FeishuGrantScopeSet {
    pub fn from_scopes<I, S>(scopes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut values = Vec::new();
        for scope in scopes {
            let value = scope.into();
            let trimmed = value.trim();
            if trimmed.is_empty() || values.iter().any(|existing| existing == trimmed) {
                continue;
            }
            values.push(trimmed.to_owned());
        }
        Self { scopes: values }
    }

    pub fn contains(&self, scope: &str) -> bool {
        let expected = scope.trim();
        !expected.is_empty()
            && self
                .scopes
                .iter()
                .any(|value| scopes_match(value.as_str(), expected))
    }

    pub fn iter(&self) -> impl Iterator<Item = &String> {
        self.scopes.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.scopes.is_empty()
    }

    pub fn to_scope_csv(&self) -> String {
        self.scopes.join(" ")
    }

    pub fn as_slice(&self) -> &[String] {
        &self.scopes
    }
}

impl From<Vec<String>> for FeishuGrantScopeSet {
    fn from(value: Vec<String>) -> Self {
        Self::from_scopes(value)
    }
}

fn scopes_match(stored: &str, expected: &str) -> bool {
    if stored == expected {
        return true;
    }

    matches!(
        (stored, expected),
        (
            FEISHU_GROUP_MESSAGE_READ_SCOPE,
            FEISHU_GROUP_MESSAGE_READ_SCOPE_LEGACY
        ) | (
            FEISHU_GROUP_MESSAGE_READ_SCOPE_LEGACY,
            FEISHU_GROUP_MESSAGE_READ_SCOPE
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_principal_key_is_stable_for_account_and_open_id() {
        let principal = FeishuUserPrincipal {
            account_id: "feishu_main".to_owned(),
            open_id: "ou_123".to_owned(),
            union_id: Some("on_456".to_owned()),
            user_id: Some("u_789".to_owned()),
            name: Some("Alice".to_owned()),
            tenant_key: Some("tenant_x".to_owned()),
            avatar_url: None,
            email: None,
            enterprise_email: None,
        };

        assert_eq!(principal.storage_key(), "feishu_main:ou_123");
    }

    #[test]
    fn account_binding_prefers_configured_account_id() {
        let binding = FeishuAccountBinding::new("feishu_main", "Feishu Main");
        assert_eq!(binding.account_id, "feishu_main");
        assert_eq!(binding.label, "Feishu Main");
    }

    #[test]
    fn grant_scope_set_dedupes_and_trims() {
        let scopes = FeishuGrantScopeSet::from_scopes([
            " offline_access ",
            "docx:document:readonly",
            "offline_access",
            "",
        ]);

        assert_eq!(
            scopes.as_slice(),
            &[
                "offline_access".to_owned(),
                "docx:document:readonly".to_owned()
            ]
        );
    }

    #[test]
    fn grant_scope_set_contains_group_message_scope_aliases() {
        let current = FeishuGrantScopeSet::from_scopes(["im:message.group_msg"]);
        assert!(current.contains("im:message.group_msg"));
        assert!(current.contains("im:message.group_msg:readonly"));

        let legacy = FeishuGrantScopeSet::from_scopes(["im:message.group_msg:readonly"]);
        assert!(legacy.contains("im:message.group_msg"));
        assert!(legacy.contains("im:message.group_msg:readonly"));
    }
}
