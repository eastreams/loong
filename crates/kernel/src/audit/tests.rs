use std::{
    sync::atomic::{AtomicUsize, Ordering},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use super::{AuditEvent, AuditEventKind, AuditSink, InMemoryAuditSink, SharedAuditState};
use crate::{FixedClock, errors::AuditError};

struct BlockingFirstAuditSink {
    events: Mutex<Vec<AuditEvent>>,
    first_entered: Mutex<Option<mpsc::Sender<()>>>,
    release_first: Mutex<mpsc::Receiver<()>>,
    second_entered: mpsc::Sender<()>,
}

impl AuditSink for BlockingFirstAuditSink {
    fn record(&self, event: AuditEvent) -> Result<(), AuditError> {
        if event.event_id == "evt-0000000000000001" {
            self.first_entered
                .lock()
                .expect("first-entered sender lock")
                .take()
                .expect("first event should enter once")
                .send(())
                .expect("announce first event entry");
            self.release_first
                .lock()
                .expect("first-event release lock")
                .recv()
                .expect("wait for first-event release");
        } else {
            self.second_entered
                .send(())
                .expect("announce second event entry");
        }

        self.events.lock().expect("audit event lock").push(event);
        Ok(())
    }
}

#[derive(Default)]
struct CountingAuditSink {
    calls: AtomicUsize,
}

impl AuditSink for CountingAuditSink {
    fn record(&self, _event: AuditEvent) -> Result<(), AuditError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

#[test]
// The blocked-first schedule must remain visible as one scenario; extracting
// thread/channel setup would hide the ordering invariant under test.
#[allow(clippy::too_many_lines)]
fn shared_audit_state_serializes_event_ids_in_sink_write_order() {
    let (first_entered_tx, first_entered_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();
    let (second_entered_tx, second_entered_rx) = mpsc::channel();
    let (second_started_tx, second_started_rx) = mpsc::channel();
    let sink = Arc::new(BlockingFirstAuditSink {
        events: Mutex::new(Vec::new()),
        first_entered: Mutex::new(Some(first_entered_tx)),
        release_first: Mutex::new(release_first_rx),
        second_entered: second_entered_tx,
    });
    let state = Arc::new(SharedAuditState::new(
        Arc::new(FixedClock::new(42)),
        sink.clone(),
    ));

    let first_state = state.clone();
    let first = thread::spawn(move || {
        first_state.record(
            None,
            AuditEventKind::TokenRevoked {
                token_id: "first".to_owned(),
            },
        )
    });
    first_entered_rx
        .recv()
        .expect("first event should block inside the sink");

    let second = thread::spawn(move || {
        second_started_tx
            .send(())
            .expect("announce second record call");
        state.record(
            None,
            AuditEventKind::TokenRevoked {
                token_id: "second".to_owned(),
            },
        )
    });
    second_started_rx
        .recv()
        .expect("second writer should start concurrently");

    let _ = second_entered_rx.recv_timeout(Duration::from_secs(1));
    release_first_tx
        .send(())
        .expect("release the first sink write");

    first
        .join()
        .expect("join first audit writer")
        .expect("record first event");
    second
        .join()
        .expect("join second audit writer")
        .expect("record second event");

    let events = sink.events.lock().expect("audit event lock");
    let event_ids = events
        .iter()
        .map(|event| event.event_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(event_ids, ["evt-0000000000000001", "evt-0000000000000002"]);
}

#[test]
fn shared_audit_state_rejects_exhausted_authorization_attempt_ids() {
    let state = SharedAuditState::new(
        Arc::new(FixedClock::new(42)),
        Arc::new(InMemoryAuditSink::default()),
    );
    state
        .authorization_attempt_seq
        .store(u64::MAX, Ordering::Relaxed);

    let error = state
        .reserve_authorization_attempt_id()
        .expect_err("authorization attempt ids must not wrap");

    assert_eq!(error, AuditError::AuthorizationAttemptIdExhausted);
}

#[test]
fn independent_audit_states_allocate_distinct_grant_ids() {
    let first = SharedAuditState::new(
        Arc::new(FixedClock::new(42)),
        Arc::new(InMemoryAuditSink::default()),
    );
    let second = SharedAuditState::new(
        Arc::new(FixedClock::new(42)),
        Arc::new(InMemoryAuditSink::default()),
    );

    let first_id = first
        .reserve_grant_id()
        .expect("first grant id should allocate");
    let second_id = second
        .reserve_grant_id()
        .expect("second grant id should allocate");

    assert_ne!(first_id, second_id);
}

#[test]
fn shared_audit_state_rejects_exhausted_event_ids_without_writing() {
    let sink = Arc::new(CountingAuditSink::default());
    let state = SharedAuditState::new(Arc::new(FixedClock::new(42)), sink.clone());
    *state.event_seq.lock().expect("event sequence lock") = u64::MAX;

    let error = state
        .record(
            None,
            AuditEventKind::TokenRevoked {
                token_id: "must-not-write".to_owned(),
            },
        )
        .expect_err("audit event ids must not wrap");

    assert_eq!(error, AuditError::EventIdExhausted);
    assert_eq!(sink.calls.load(Ordering::Relaxed), 0);
}
