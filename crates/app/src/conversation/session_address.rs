use crate::config::{normalize_dispatch_account_id, normalize_dispatch_channel_id};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationSessionAddress {
    pub session_id: String,
    pub channel_id: Option<String>,
    pub account_id: Option<String>,
    pub conversation_id: Option<String>,
    pub thread_id: Option<String>,
}

impl ConversationSessionAddress {
    pub fn from_session_id(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into().trim().to_owned(),
            ..Self::default()
        }
    }

    pub fn with_channel_scope(
        mut self,
        channel_id: impl Into<String>,
        conversation_id: impl Into<String>,
    ) -> Self {
        self.channel_id = normalize_dispatch_channel_id(channel_id.into().trim());
        self.conversation_id = trimmed_non_empty(conversation_id.into());
        self
    }

    pub fn with_account_id(mut self, account_id: impl Into<String>) -> Self {
        let account_id = account_id.into();
        self.account_id = normalize_dispatch_account_id(account_id.as_str());
        self
    }

    pub fn with_thread_id(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = trimmed_non_empty(thread_id.into());
        self
    }

    pub fn canonical_channel_id(&self) -> Option<String> {
        self.channel_id
            .as_deref()
            .and_then(normalize_dispatch_channel_id)
    }

    pub fn structured_channel_path(&self) -> Vec<String> {
        let mut path = Vec::new();
        if let Some(account_id) = self.account_id.as_ref().and_then(trimmed_non_empty) {
            path.push(account_id);
        }
        if let Some(conversation_id) = self.conversation_id.as_ref().and_then(trimmed_non_empty) {
            path.push(conversation_id);
        }
        if let Some(thread_id) = self.thread_id.as_ref().and_then(trimmed_non_empty) {
            path.push(thread_id);
        }
        path
    }

    pub fn structured_route_session_id(&self) -> Option<String> {
        let channel_id = self.canonical_channel_id()?;
        let path = self.structured_channel_path();
        if path.is_empty() {
            Some(channel_id)
        } else {
            Some(format!("{channel_id}:{}", path.join(":")))
        }
    }
}

fn trimmed_non_empty(value: impl AsRef<str>) -> Option<String> {
    let value = value.as_ref().trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::ConversationSessionAddress;

    #[test]
    fn structured_route_session_id_normalizes_channel_and_preserves_scope() {
        let address = ConversationSessionAddress::from_session_id("opaque")
            .with_channel_scope(" Feishu ", "oc_123")
            .with_account_id("lark_cli_a1b2c3")
            .with_thread_id("om_thread_1");

        assert_eq!(
            address.structured_route_session_id().as_deref(),
            Some("feishu:lark_cli_a1b2c3:oc_123:om_thread_1")
        );
    }
}
