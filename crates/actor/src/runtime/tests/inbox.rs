use super::*;

#[tokio::test]
async fn stop_discard_observes_kill_from_each_envelope_drop() {
    // The first destructor upgrades Stop to Kill. Leaving the second envelope
    // queued proves teardown observes the transition between individual Drops,
    // before hard teardown takes ownership of the remaining work.
    let (inner, mut inbox) = test_actor_inner(2);
    let second_dropped = Arc::new(AtomicBool::new(false));
    enqueue_test_envelope(
        &inner,
        TeardownEnvelope::RequestKill(Arc::downgrade(&inner)),
    );
    enqueue_test_envelope(
        &inner,
        TeardownEnvelope::MarkDropped(Arc::clone(&second_dropped)),
    );
    assert_eq!(
        inner.control.request(Shutdown::Stop),
        ShutdownStatus::Requested
    );

    assert_eq!(
        close_and_discard(&mut inbox, &inner.control, Mode::Stopping).await,
        DiscardOutcome::ModeChanged
    );
    assert_eq!(inner.control.mode(), Mode::Killing);
    assert!(!second_dropped.load(Ordering::SeqCst));

    assert_eq!(
        close_and_discard(&mut inbox, &inner.control, Mode::Killing).await,
        DiscardOutcome::Complete
    );
    assert!(second_dropped.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "current_thread")]
async fn queued_discard_yields_after_its_fixed_drop_budget() {
    // One manual poll must stop before the seventeenth Drop, and the next must
    // complete it. This locks the exact cooperative boundary while rejecting
    // both an unbounded drain and an implementation that yields too frequently.
    let (inner, mut inbox) = test_actor_inner(TEARDOWN_DROP_BUDGET + 1);
    for _ in 0..TEARDOWN_DROP_BUDGET {
        enqueue_test_envelope(&inner, TeardownEnvelope::Noop);
    }

    let last_dropped = Arc::new(AtomicBool::new(false));
    enqueue_test_envelope(
        &inner,
        TeardownEnvelope::MarkDropped(Arc::clone(&last_dropped)),
    );
    assert_eq!(
        inner.control.request(Shutdown::Stop),
        ShutdownStatus::Requested
    );

    let discard = close_and_discard(&mut inbox, &inner.control, Mode::Stopping);
    tokio::pin!(discard);
    let mut task = Context::from_waker(Waker::noop());

    assert_eq!(discard.as_mut().poll(&mut task), Poll::Pending);
    assert!(!last_dropped.load(Ordering::SeqCst));
    assert_eq!(
        discard.as_mut().poll(&mut task),
        Poll::Ready(DiscardOutcome::Complete)
    );
    assert!(last_dropped.load(Ordering::SeqCst));
}

struct CapacityPanicWake(AtomicUsize);

impl Wake for CapacityPanicWake {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("intentional capacity notification panic");
    }
}

// Tokio wakes capacity waiters while closing its receiver.
// ActorInbox contains that panic before discarding accepted envelopes.
// Each envelope then receives its own containment boundary.
#[test]
fn inbox_drop_contains_each_envelope_drop() {
    let (inner, inbox) = test_actor_inner(2);
    let panic_dropped = Arc::new(AtomicBool::new(false));
    let tail_dropped = Arc::new(AtomicBool::new(false));
    let tail_dropped_while_unwinding = Arc::new(AtomicBool::new(false));
    enqueue_test_envelope(&inner, TeardownEnvelope::Panic(Arc::clone(&panic_dropped)));
    enqueue_test_envelope(
        &inner,
        TeardownEnvelope::TrackDrop {
            dropped: Arc::clone(&tail_dropped),
            dropped_while_unwinding: Arc::clone(&tail_dropped_while_unwinding),
        },
    );
    let wakes = Arc::new(CapacityPanicWake(AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&wakes));
    let mut task = Context::from_waker(&waker);
    let mut reserve = Box::pin(inner.sender.reserve_owned());
    assert!(matches!(reserve.as_mut().poll(&mut task), Poll::Pending));
    assert_eq!(
        inner.control.request(Shutdown::Kill),
        ShutdownStatus::Requested
    );

    drop(inbox);
    assert!(panic_dropped.load(Ordering::SeqCst));
    assert!(tail_dropped.load(Ordering::SeqCst));
    assert!(!tail_dropped_while_unwinding.load(Ordering::SeqCst));
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
    assert_eq!(inner.control.mode(), Mode::Killing);
}

#[test]
fn unbounded_inbox_drop_contains_each_envelope_drop() {
    let options = <UnboundedTestActor as ActorConfig>::Options::default();
    let (inner, inbox, _scheduler) = ActorInner::<UnboundedTestActor>::open(&options);
    let panic_dropped = Arc::new(AtomicBool::new(false));
    let tail_dropped = Arc::new(AtomicBool::new(false));
    let tail_dropped_while_unwinding = Arc::new(AtomicBool::new(false));
    enqueue_test_envelope(&inner, TeardownEnvelope::Panic(Arc::clone(&panic_dropped)));
    enqueue_test_envelope(
        &inner,
        TeardownEnvelope::TrackDrop {
            dropped: Arc::clone(&tail_dropped),
            dropped_while_unwinding: Arc::clone(&tail_dropped_while_unwinding),
        },
    );
    assert_eq!(
        inner.control.request(Shutdown::Kill),
        ShutdownStatus::Requested
    );

    drop(inbox);
    assert!(panic_dropped.load(Ordering::SeqCst));
    assert!(tail_dropped.load(Ordering::SeqCst));
    assert!(!tail_dropped_while_unwinding.load(Ordering::SeqCst));
    assert_eq!(inner.control.mode(), Mode::Killing);
}
