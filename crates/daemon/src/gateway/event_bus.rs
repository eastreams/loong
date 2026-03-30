use serde_json::Value;
use tokio::sync::broadcast;

use crate::CliResult;
use crate::mvp::acp::AcpTurnEventSink;

/// Broadcast channel for streaming gateway events to SSE subscribers.
#[derive(Clone)]
pub struct GatewayEventBus {
    sender: broadcast::Sender<Value>,
}

impl GatewayEventBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Create a new subscriber receiver.
    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.sender.subscribe()
    }

    /// Create a sink that publishes events to this bus.
    pub fn sink(&self) -> BroadcastEventSink {
        BroadcastEventSink {
            sender: self.sender.clone(),
        }
    }
}

/// An `AcpTurnEventSink` that publishes events to a broadcast channel.
pub struct BroadcastEventSink {
    sender: broadcast::Sender<Value>,
}

impl AcpTurnEventSink for BroadcastEventSink {
    fn on_event(&self, event: &Value) -> CliResult<()> {
        // Ignore send errors — means no active subscribers, which is fine
        let _ = self.sender.send(event.clone());
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn broadcast_sink_delivers_to_subscriber() {
        let bus = GatewayEventBus::new(64);
        let mut rx = bus.subscribe();
        let sink = bus.sink();

        let event = json!({"event_type": "text_delta", "delta": {"text": "hello"}});
        sink.on_event(&event).unwrap();

        let received = rx.try_recv().unwrap();
        assert_eq!(received, event);
    }

    #[test]
    fn broadcast_sink_handles_no_subscribers() {
        let bus = GatewayEventBus::new(64);
        let sink = bus.sink();

        let event = json!({"event_type": "text_delta", "delta": {"text": "hello"}});
        let result = sink.on_event(&event);
        assert!(result.is_ok());
    }

    #[test]
    fn multiple_subscribers_each_receive_event() {
        let bus = GatewayEventBus::new(64);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();
        let sink = bus.sink();

        let event = json!({"event_type": "turn_complete"});
        sink.on_event(&event).unwrap();

        assert_eq!(rx1.try_recv().unwrap(), event);
        assert_eq!(rx2.try_recv().unwrap(), event);
    }
}
