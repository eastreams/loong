use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;

use crate::CliResult;
use crate::mvp::acp::AcpTurnEventSink;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatewayEventRecord {
    pub seq: u64,
    pub payload: Value,
}

#[derive(Debug, Default)]
struct GatewayEventRetentionState {
    recent_events: VecDeque<GatewayEventRecord>,
}

/// Broadcast channel for streaming gateway events to SSE subscribers.
#[derive(Clone)]
pub struct GatewayEventBus {
    sender: broadcast::Sender<GatewayEventRecord>,
    next_seq: Arc<AtomicU64>,
    retention_limit: usize,
    retention_state: Arc<RwLock<GatewayEventRetentionState>>,
}

impl GatewayEventBus {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let (sender, _) = broadcast::channel(capacity);
        let next_seq = Arc::new(AtomicU64::new(0));
        let retention_limit = capacity;
        let retention_state = Arc::new(RwLock::new(GatewayEventRetentionState::default()));

        Self {
            sender,
            next_seq,
            retention_limit,
            retention_state,
        }
    }

    /// Create a new subscriber receiver.
    pub fn subscribe(&self) -> broadcast::Receiver<GatewayEventRecord> {
        self.sender.subscribe()
    }

    pub fn recent_events_after(&self, after_seq: u64, limit: usize) -> Vec<GatewayEventRecord> {
        let bounded_limit = limit.max(1);
        let retention_guard = self
            .retention_state
            .read()
            .unwrap_or_else(|error| error.into_inner());
        let mut matching_events = retention_guard
            .recent_events
            .iter()
            .rev()
            .filter(|record| record.seq > after_seq)
            .take(bounded_limit)
            .cloned()
            .collect::<Vec<_>>();
        matching_events.reverse();
        matching_events
    }

    pub fn publish(&self, payload: Value) -> GatewayEventRecord {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let record = GatewayEventRecord { seq, payload };

        {
            let retention_guard = self.retention_state.write();
            let mut retention_guard = retention_guard.unwrap_or_else(|error| error.into_inner());
            retention_guard.recent_events.push_back(record.clone());
            while retention_guard.recent_events.len() > self.retention_limit {
                retention_guard.recent_events.pop_front();
            }
        }

        let _ = self.sender.send(record.clone());

        record
    }

    /// Create a sink that publishes events to this bus.
    pub fn sink(&self) -> BroadcastEventSink {
        let bus = self.clone();
        BroadcastEventSink { bus }
    }
}

/// An `AcpTurnEventSink` that publishes events to a broadcast channel.
pub struct BroadcastEventSink {
    bus: GatewayEventBus,
}

impl AcpTurnEventSink for BroadcastEventSink {
    fn on_event(&self, event: &Value) -> CliResult<()> {
        let payload = event.clone();
        let _record = self.bus.publish(payload);
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
        assert_eq!(received.seq, 1);
        assert_eq!(received.payload, event);
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

        let received_one = rx1.try_recv().unwrap();
        let received_two = rx2.try_recv().unwrap();

        assert_eq!(received_one.seq, 1);
        assert_eq!(received_one.payload, event);
        assert_eq!(received_two.seq, 1);
        assert_eq!(received_two.payload, event);
    }

    #[test]
    fn recent_events_after_returns_bounded_suffix() {
        let bus = GatewayEventBus::new(3);

        let first = bus.publish(json!({"event_type": "first"}));
        let second = bus.publish(json!({"event_type": "second"}));
        let third = bus.publish(json!({"event_type": "third"}));
        let fourth = bus.publish(json!({"event_type": "fourth"}));

        assert_eq!(first.seq, 1);
        assert_eq!(second.seq, 2);
        assert_eq!(third.seq, 3);
        assert_eq!(fourth.seq, 4);

        let replay = bus.recent_events_after(1, 10);
        let replay_seqs = replay.iter().map(|record| record.seq).collect::<Vec<_>>();

        assert_eq!(replay_seqs, vec![2, 3, 4]);

        let bounded_replay = bus.recent_events_after(0, 2);
        let bounded_seqs = bounded_replay
            .iter()
            .map(|record| record.seq)
            .collect::<Vec<_>>();

        assert_eq!(bounded_seqs, vec![3, 4]);
    }
}
