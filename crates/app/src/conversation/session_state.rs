use std::collections::BTreeMap;
use std::sync::OnceLock;

use loong_kernel::mailbox::AgentMailbox;

#[cfg(test)]
use loong_kernel::mailbox::AgentPath;
pub(crate) use loong_kernel::mailbox::InterAgentMessage;
#[cfg(test)]
use loong_kernel::mailbox::MailboxContent;

fn normalize_session_id(session_id: &str) -> String {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        "default".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn mailboxes() -> &'static std::sync::Mutex<BTreeMap<String, AgentMailbox>> {
    static SESSION_MAILBOXES: OnceLock<std::sync::Mutex<BTreeMap<String, AgentMailbox>>> =
        OnceLock::new();
    SESSION_MAILBOXES.get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
}

pub(crate) fn mailbox_for_session(session_id: &str) -> AgentMailbox {
    let normalized = normalize_session_id(session_id);
    let mut guard = mailboxes()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    guard.entry(normalized).or_default().clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn mailbox_registry_reuses_same_session_mailbox() {
        let mailbox_a = mailbox_for_session("session-a");
        let mailbox_b = mailbox_for_session("session-a");

        let send_result = mailbox_a.send(InterAgentMessage {
            author: AgentPath::root(),
            recipient: AgentPath::root(),
            content: MailboxContent::DelegateResult {
                session_id: "child-1".to_owned(),
                frozen_result: json!({"status": "ok"}),
            },
            trigger_turn: true,
        });
        assert!(send_result.is_ok());

        let drained = mailbox_b.drain().await;
        assert_eq!(drained.len(), 1);
    }
}
