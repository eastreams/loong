use std::{
    future::Future,
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc as std_mpsc,
    },
    task::{Context, Waker},
    time::Duration,
};

use tokio::sync::oneshot;

use crate::{CallError, ExitReason, Shutdown, ShutdownStatus};

use super::super::{Control, DispatchReply, Mode};
use super::PanicWake;

#[tokio::test]
async fn completion_before_kill_delivers_the_response() {
    let control = Control::new();
    let permit = Arc::clone(&control)
        .begin_dispatch()
        .expect("dispatch wins the gate");
    let (sender, receiver) = oneshot::channel();

    DispatchReply::new(sender, permit).complete(7_u8);
    assert_eq!(control.request(Shutdown::Kill), ShutdownStatus::Requested);

    assert_eq!(receiver.await, Ok(Ok(7)));
}

// The response value commits before its observer is notified.
// A panicking observer is not actor code.
// Its panic must not fail an otherwise healthy actor.
#[test]
fn response_waker_panic_does_not_fail_the_actor() {
    let control = Control::new();
    let permit = Arc::clone(&control)
        .begin_dispatch()
        .expect("dispatch wins the gate");
    let (sender, response) = oneshot::channel();
    let wakes = Arc::new(AtomicUsize::new(0));
    let waker = Waker::from(Arc::new(PanicWake(Arc::clone(&wakes))));
    let mut task = Context::from_waker(&waker);
    let mut response = Box::pin(response);
    assert!(response.as_mut().poll(&mut task).is_pending());

    DispatchReply::new(sender, permit).complete(7_u8);

    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    assert_eq!(response.as_mut().get_mut().try_recv(), Ok(Ok(7)));
    assert_eq!(control.mode(), Mode::Running);
}

struct PanicDropReply(Arc<AtomicBool>);

impl Drop for PanicDropReply {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
        panic!("intentional response drop panic");
    }
}

// A closed receiver leaves the response owned by the actor.
// Its destructor panic must remain contained.
// The actor must still record that user-code failure.
#[test]
fn undelivered_response_drop_panic_fails_the_actor() {
    let control = Control::new();
    let permit = Arc::clone(&control)
        .begin_dispatch()
        .expect("dispatch wins the gate");
    let (sender, response) = oneshot::channel();
    let dropped = Arc::new(AtomicBool::new(false));
    drop(response);

    DispatchReply::new(sender, permit).complete(PanicDropReply(Arc::clone(&dropped)));

    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(control.mode(), Mode::Failing);
}

#[tokio::test]
async fn kill_before_completion_reports_the_dispatching_phase() {
    let control = Control::new();
    let permit = Arc::clone(&control)
        .begin_dispatch()
        .expect("dispatch wins the gate");
    let (sender, receiver) = oneshot::channel();
    assert_eq!(control.request(Shutdown::Kill), ShutdownStatus::Requested);

    DispatchReply::new(sender, permit).complete(7_u8);

    assert!(matches!(
        receiver.await,
        Ok(Err(CallError::DuringDispatch(ExitReason::Killed)))
    ));
}

#[tokio::test]
async fn dropped_dispatch_reply_closes_admission_before_publishing_failure() {
    let control = Control::new();
    let permit = Arc::clone(&control)
        .begin_dispatch()
        .expect("dispatch wins the gate");
    let (sender, receiver) = oneshot::channel();

    drop(DispatchReply::<()>::new(sender, permit));

    assert!(matches!(
        receiver.await,
        Ok(Err(CallError::DuringDispatch(ExitReason::Panicked)))
    ));
    assert_eq!(control.mode(), Mode::Failing);
    assert!(!control.is_running());
}

// Lifecycle and response notifications can invoke safe custom wakers.
// Neither panic may escape Drop or combine with a later user destructor.
#[test]
fn dispatch_reply_drop_contains_notification_panics() {
    let control = Control::new();
    let permit = Arc::clone(&control)
        .begin_dispatch()
        .expect("dispatch wins the gate");
    let (sender, response) = oneshot::channel();
    let wakes = Arc::new(AtomicUsize::new(0));
    let waker = Waker::from(Arc::new(PanicWake(Arc::clone(&wakes))));
    let mut task = Context::from_waker(&waker);

    let mut mode = control.subscribe_mode();
    let mut changed = Box::pin(mode.changed());
    assert!(changed.as_mut().poll(&mut task).is_pending());
    let mut response = Box::pin(response);
    assert!(response.as_mut().poll(&mut task).is_pending());

    let dropped = panic::catch_unwind(AssertUnwindSafe(|| {
        drop(DispatchReply::<()>::new(sender, permit));
    }));

    assert!(dropped.is_ok());
    assert_eq!(wakes.load(Ordering::SeqCst), 2);
    assert_eq!(control.mode(), Mode::Failing);
    assert!(matches!(
        response.as_mut().get_mut().try_recv(),
        Ok(Err(CallError::DuringDispatch(ExitReason::Panicked)))
    ));
}

struct GateDropProbe {
    control: Arc<Control>,
    reentered: Arc<AtomicBool>,
}

impl Drop for GateDropProbe {
    fn drop(&mut self) {
        let _ = self.control.mode();
        self.reentered.store(true, Ordering::SeqCst);
    }
}

// A rejected user response may reenter lifecycle APIs from Drop. Completing
// on an OS thread turns accidental in-transaction destruction into a bounded
// failure instead of hanging the entire test process.
#[tokio::test]
async fn rejected_response_is_dropped_outside_the_lifecycle_gate() {
    let control = Control::new();
    let permit = Arc::clone(&control)
        .begin_dispatch()
        .expect("dispatch wins the gate");
    let (sender, receiver) = oneshot::channel();
    let reentered = Arc::new(AtomicBool::new(false));
    assert_eq!(control.request(Shutdown::Kill), ShutdownStatus::Requested);

    let reply = DispatchReply::new(sender, permit);
    let (done_tx, done_rx) = std_mpsc::sync_channel(1);
    let completion = std::thread::spawn({
        let reentered = Arc::clone(&reentered);
        move || {
            reply.complete(GateDropProbe { control, reentered });
            let _ = done_tx.send(());
        }
    });
    done_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("response Drop must not retain the lifecycle transaction");
    completion.join().expect("completion thread must not panic");

    assert!(matches!(
        receiver.await,
        Ok(Err(CallError::DuringDispatch(ExitReason::Killed)))
    ));
    assert!(reentered.load(Ordering::SeqCst));
}
