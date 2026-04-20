use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Mutex, mpsc, watch};

pub const ROOT_AGENT_PATH: &str = "/root";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AgentPath(String);

impl AgentPath {
    pub fn from_string(raw: impl AsRef<str>) -> Result<Self, String> {
        let normalized = normalize_agent_path(raw.as_ref())?;
        Ok(Self(normalized))
    }

    pub fn root() -> Self {
        Self(ROOT_AGENT_PATH.to_owned())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn join(&self, child: impl AsRef<str>) -> Result<Self, String> {
        let child = child.as_ref().trim();
        if child.is_empty() {
            return Err("agent_path_invalid: child segment must not be empty".to_owned());
        }
        if child.contains('/') {
            return Err("agent_path_invalid: child segment must not contain `/`".to_owned());
        }
        if !is_valid_segment(child) {
            return Err(format!(
                "agent_path_invalid: child segment `{child}` contains unsupported characters"
            ));
        }
        Self::from_string(format!("{}/{}", self.0, child))
    }
}

impl Default for AgentPath {
    fn default() -> Self {
        Self::root()
    }
}

impl AsRef<str> for AgentPath {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MailboxContent {
    DelegateResult {
        session_id: String,
        frozen_result: Value,
    },
    StatusNotification {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterAgentMessage {
    pub author: AgentPath,
    pub recipient: AgentPath,
    pub content: MailboxContent,
    pub trigger_turn: bool,
}

#[derive(Debug)]
struct AgentMailboxState {
    receiver: Mutex<mpsc::UnboundedReceiver<InterAgentMessage>>,
    sequence: AtomicU64,
    notifier: watch::Sender<u64>,
}

#[derive(Debug, Clone)]
pub struct AgentMailbox {
    sender: mpsc::UnboundedSender<InterAgentMessage>,
    state: Arc<AgentMailboxState>,
}

impl AgentMailbox {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        let (notifier, _) = watch::channel(0_u64);
        Self {
            sender,
            state: Arc::new(AgentMailboxState {
                receiver: Mutex::new(receiver),
                sequence: AtomicU64::new(0),
                notifier,
            }),
        }
    }

    pub fn send(&self, msg: InterAgentMessage) -> Result<(), String> {
        self.sender
            .send(msg)
            .map_err(|error| format!("agent_mailbox_closed: {error}"))?;
        let next_seq = self.state.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.state.notifier.send(next_seq);
        Ok(())
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.state.notifier.subscribe()
    }

    pub async fn drain(&self) -> Vec<InterAgentMessage> {
        let mut receiver = self.state.receiver.lock().await;
        let mut drained = VecDeque::new();
        while let Ok(message) = receiver.try_recv() {
            drained.push_back(message);
        }
        drained.into_iter().collect()
    }
}

impl Default for AgentMailbox {
    fn default() -> Self {
        Self::new()
    }
}

fn normalize_agent_path(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("agent_path_invalid: path must not be empty".to_owned());
    }
    if !trimmed.starts_with('/') {
        return Err("agent_path_invalid: path must start with `/`".to_owned());
    }

    let mut segments = Vec::new();
    for segment in trimmed.split('/').skip(1) {
        if segment.is_empty() {
            return Err("agent_path_invalid: empty path segment".to_owned());
        }
        if !is_valid_segment(segment) {
            return Err(format!(
                "agent_path_invalid: segment `{segment}` contains unsupported characters"
            ));
        }
        segments.push(segment);
    }

    if segments.is_empty() {
        return Err("agent_path_invalid: root segment is required".to_owned());
    }

    Ok(format!("/{}", segments.join("/")))
}

fn is_valid_segment(segment: &str) -> bool {
    segment
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' || ch == ':')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn mailbox_send_subscribe_drain_lifecycle() {
        let mailbox = AgentMailbox::new();
        let mut subscription = mailbox.subscribe();

        let author = AgentPath::root();
        let recipient = author.join("task").unwrap_or_else(|_| AgentPath::root());
        let send_result = mailbox.send(InterAgentMessage {
            author,
            recipient,
            content: MailboxContent::StatusNotification {
                reason: "child_completed".to_owned(),
            },
            trigger_turn: true,
        });
        assert!(send_result.is_ok());

        let changed_result = subscription.changed().await;
        assert!(changed_result.is_ok());

        let drained = mailbox.drain().await;
        assert_eq!(drained.len(), 1);
    }

    #[tokio::test]
    async fn mailbox_sequence_increments() {
        let mailbox = AgentMailbox::new();
        let mut subscription = mailbox.subscribe();

        let first = mailbox.send(InterAgentMessage {
            author: AgentPath::root(),
            recipient: AgentPath::root(),
            content: MailboxContent::StatusNotification {
                reason: "first".to_owned(),
            },
            trigger_turn: false,
        });
        assert!(first.is_ok());
        let first_changed = subscription.changed().await;
        assert!(first_changed.is_ok());
        let first_seq = *subscription.borrow();

        let second = mailbox.send(InterAgentMessage {
            author: AgentPath::root(),
            recipient: AgentPath::root(),
            content: MailboxContent::DelegateResult {
                session_id: "child-1".to_owned(),
                frozen_result: json!({"status": "ok"}),
            },
            trigger_turn: true,
        });
        assert!(second.is_ok());
        let second_changed = subscription.changed().await;
        assert!(second_changed.is_ok());
        let second_seq = *subscription.borrow();

        assert!(second_seq > first_seq);
    }

    #[tokio::test]
    async fn mailbox_supports_multiple_senders() {
        let mailbox = AgentMailbox::new();
        let mailbox_2 = mailbox.clone();

        let send_1 = mailbox.send(InterAgentMessage {
            author: AgentPath::root(),
            recipient: AgentPath::root(),
            content: MailboxContent::StatusNotification {
                reason: "a".to_owned(),
            },
            trigger_turn: false,
        });
        assert!(send_1.is_ok());

        let send_2 = mailbox_2.send(InterAgentMessage {
            author: AgentPath::root(),
            recipient: AgentPath::root(),
            content: MailboxContent::StatusNotification {
                reason: "b".to_owned(),
            },
            trigger_turn: false,
        });
        assert!(send_2.is_ok());

        let drained = mailbox.drain().await;
        assert_eq!(drained.len(), 2);
    }

    #[test]
    fn agent_path_validates_and_joins() {
        let root = AgentPath::from_string(ROOT_AGENT_PATH);
        assert!(root.is_ok());
        let root = root.unwrap_or_else(|_| AgentPath::root());

        let child = root.join("subtask");
        assert!(child.is_ok());
        let child = child.unwrap_or_else(|_| AgentPath::root());

        assert_eq!(child.as_str(), "/root/subtask");
        assert!(AgentPath::from_string("root/subtask").is_err());
        assert!(AgentPath::from_string("/root//subtask").is_err());
    }
}
