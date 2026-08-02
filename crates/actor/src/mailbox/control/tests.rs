use std::{
    future::Future,
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Wake, Waker},
};

use crate::{
    Actor, ActorScope, CallError, ExitReason, ExitStatus, Shutdown, ShutdownStatus, SubtreeStatus,
};

use super::*;
use crate::mailbox::ActorInner;

struct TestActor;

impl Actor for TestActor {
    type SpawnArgs = ();

    async fn init(_args: (), _scope: &mut ActorScope<'_, Self>) -> Self {
        Self
    }
}

struct WakeCounter(AtomicUsize);

impl Wake for WakeCounter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct PanicWake(Arc<AtomicUsize>);

impl Wake for PanicWake {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("intentional notification panic");
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("intentional notification panic");
    }
}

// Tokio must see a Waker that never unwinds from wake.
// Direct invocation avoids depending on fanout bucket order.
#[test]
fn lifecycle_waker_proxy_contains_external_wake_panic() {
    let wakes = Arc::new(AtomicUsize::new(0));
    let external = Waker::from(Arc::new(PanicWake(Arc::clone(&wakes))));
    let proxy = Waker::from(Arc::new(PanicSafeWake(external)));

    let result = panic::catch_unwind(AssertUnwindSafe(|| proxy.wake_by_ref()));

    assert!(result.is_ok());
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
}

struct PanicWakeDrop(Arc<AtomicBool>);

impl Wake for PanicWakeDrop {
    fn wake(self: Arc<Self>) {
        panic!("intentional waker panic");
    }

    fn wake_by_ref(self: &Arc<Self>) {
        panic!("intentional waker panic");
    }
}

impl Drop for PanicWakeDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
        panic!("intentional waker drop panic");
    }
}

// Cancellation drops Wakers retained by the wrapped future.
// Their destructors must not unwind through cancellation.
#[test]
fn panic_safe_poll_contains_external_waker_drop_on_cancellation() {
    let control = Control::new();
    let mut mode = control.subscribe_mode();
    let dropped = Arc::new(AtomicBool::new(false));
    let waker = Waker::from(Arc::new(PanicWakeDrop(Arc::clone(&dropped))));
    let mut changed = Box::pin(poll_with_panic_safe_waker(mode.changed()));
    {
        let mut task = Context::from_waker(&waker);
        assert!(changed.as_mut().poll(&mut task).is_pending());
    }
    drop(waker);

    let result = panic::catch_unwind(AssertUnwindSafe(|| drop(changed)));

    assert!(result.is_ok());
    assert!(dropped.load(Ordering::SeqCst));
}

// Public observers may install arbitrary safe Wakers.
// One panic must not suppress another lifecycle waiter.
// It must not consume the private actor hint.
#[test]
fn public_notification_panic_preserves_every_other_waiter() {
    let control = Control::new();
    let public_wakes = Arc::new(AtomicUsize::new(0));
    let public_waker = Waker::from(Arc::new(PanicWake(Arc::clone(&public_wakes))));
    let mut public_task = Context::from_waker(&public_waker);
    let mut public_mode = control.subscribe_mode();
    let mut public_changed = Box::pin(poll_with_panic_safe_waker(public_mode.changed()));
    assert!(public_changed.as_mut().poll(&mut public_task).is_pending());

    let other_wakes = Arc::new(WakeCounter(AtomicUsize::new(0)));
    let other_waker = Waker::from(Arc::clone(&other_wakes));
    let mut other_task = Context::from_waker(&other_waker);
    let mut other_mode = control.subscribe_mode();
    let mut other_changed = Box::pin(poll_with_panic_safe_waker(other_mode.changed()));
    assert!(other_changed.as_mut().poll(&mut other_task).is_pending());

    let actor_wakes = Arc::new(WakeCounter(AtomicUsize::new(0)));
    let actor_waker = Waker::from(Arc::clone(&actor_wakes));
    let mut actor_task = Context::from_waker(&actor_waker);
    let mut actor_notified = Box::pin(control.actor_notified());
    assert!(actor_notified.as_mut().poll(&mut actor_task).is_pending());

    control.begin_failure();

    assert_eq!(public_wakes.load(Ordering::SeqCst), 1);
    assert_eq!(other_wakes.0.load(Ordering::SeqCst), 1);
    assert_eq!(actor_wakes.0.load(Ordering::SeqCst), 1);
    assert!(other_changed.as_mut().poll(&mut other_task).is_ready());
    assert!(actor_notified.as_mut().poll(&mut actor_task).is_ready());
    assert_eq!(control.mode(), Mode::Failing);
}

#[test]
fn kill_before_dispatch_rejects_the_queued_phase() {
    let (actor, _inbox) = ActorInner::<TestActor>::channel(1);
    assert_eq!(
        actor.control.request(Shutdown::Kill),
        ShutdownStatus::Requested
    );

    assert!(matches!(
        actor.begin_dispatch(),
        Err(CallError::BeforeDispatch(ExitReason::Killed))
    ));
}

// Kill and panic decide only this actor's local reason.
// Neither may erase an Unconfirmed descendant guarantee.
#[test]
fn hard_mode_precedence_preserves_unconfirmed_subtree() {
    let proposed = ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Unconfirmed);

    let killing = Control::new();
    assert_eq!(killing.request(Shutdown::Kill), ShutdownStatus::Requested);
    assert_eq!(
        killing.finish(proposed),
        ExitStatus::new(ExitReason::Killed, SubtreeStatus::Unconfirmed)
    );

    let failing = Control::new();
    failing.begin_failure();
    assert_eq!(
        failing.finish(proposed),
        ExitStatus::new(ExitReason::Panicked, SubtreeStatus::Unconfirmed)
    );

    let exited = Control::new();
    let terminal = ExitStatus::new(ExitReason::Killed, SubtreeStatus::Terminated);
    assert_eq!(exited.finish(terminal), terminal);
    assert_eq!(
        exited.finish(ExitStatus::new(
            ExitReason::Aborted,
            SubtreeStatus::Unconfirmed,
        )),
        terminal
    );
}

// Abort replaces an unfinished Kill or panic publication.
// It also removes any claimed descendant confirmation.
#[test]
fn abort_publication_replaces_unfinished_hard_modes() {
    let aborted = ExitStatus::new(ExitReason::Aborted, SubtreeStatus::Unconfirmed);
    let proposed = ExitStatus::new(ExitReason::Killed, SubtreeStatus::Terminated);

    let killing = Control::new();
    assert_eq!(killing.request(Shutdown::Kill), ShutdownStatus::Requested);
    killing.begin_abort();
    assert_eq!(killing.mode(), Mode::Aborting);
    assert_eq!(killing.finish(proposed), aborted);

    let failing = Control::new();
    failing.begin_failure();
    failing.begin_abort();
    assert_eq!(failing.mode(), Mode::Aborting);
    assert_eq!(failing.finish(proposed), aborted);

    let exited = Control::new();
    let terminal = ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated);
    assert_eq!(exited.finish(terminal), terminal);
    exited.begin_abort();
    assert_eq!(exited.mode(), Mode::Exited(terminal));
    assert_eq!(exited.exit_status(), Some(terminal));
}
