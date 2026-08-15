use std::{
    future::Future,
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

use loac::{ExitReason, Shutdown, ShutdownStatus, spawn_with};
use tokio::sync::oneshot;

use super::{Block, Notify, Record, SerialActor, single_slot_options, support::watchdog};

struct PanicWake(Arc<AtomicUsize>);

impl Wake for PanicWake {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("intentional capacity wake panic");
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("intentional capacity wake panic");
    }
}

struct PanicWakeDrop(Arc<AtomicBool>);

// Wake stays inert because this probe isolates Waker Drop.
#[allow(clippy::manual_noop_waker)]
impl Wake for PanicWakeDrop {
    fn wake(self: Arc<Self>) {}

    fn wake_by_ref(self: &Arc<Self>) {}
}

impl Drop for PanicWakeDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
        panic!("intentional capacity Waker drop panic");
    }
}

// Capacity release runs inside the actor task.
// A caller Waker must not unwind through that task.
#[tokio::test]
async fn capacity_waker_panic_does_not_fail_the_actor() {
    let owner = spawn_with::<SerialActor>(Arc::default(), single_slot_options());
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let current = tokio::spawn({
        let actor = actor.clone();
        async move {
            actor
                .call(Block {
                    entered: entered_tx,
                    release: release_rx,
                })
                .await
        }
    });

    watchdog(entered_rx).await.unwrap();
    let queued = actor.try_call(Record(1)).unwrap();
    let wakes = Arc::new(AtomicUsize::new(0));
    let waker = Waker::from(Arc::new(PanicWake(Arc::clone(&wakes))));
    let mut waiting = Box::pin(actor.call(Record(2)));
    {
        let mut task = Context::from_waker(&waker);
        assert!(matches!(waiting.as_mut().poll(&mut task), Poll::Pending));
    }
    drop(waker);

    release_tx.send(()).unwrap();
    assert_eq!(watchdog(current).await.unwrap(), Ok(()));
    assert_eq!(watchdog(queued).await, Ok(1));
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    assert_eq!(watchdog(waiting).await, Ok(2));
    assert_eq!(
        watchdog(owner.shutdown(Shutdown::Drain)).await.reason(),
        ExitReason::Drained
    );
}

// Stop wakes the lifecycle branch before capacity can change.
// Re-polling then replaces the reserve Waker.
// Its Drop panic must not destroy send recovery.
#[tokio::test]
async fn capacity_waker_drop_panic_preserves_the_waiting_message() {
    let mut owner = spawn_with::<SerialActor>(Arc::default(), single_slot_options());
    let actor = owner.actor_ref();
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let current = tokio::spawn({
        let actor = actor.clone();
        async move {
            actor
                .call(Block {
                    entered: entered_tx,
                    release: release_rx,
                })
                .await
        }
    });

    watchdog(entered_rx).await.unwrap();
    actor.try_send(Notify(1)).unwrap();
    let dropped = Arc::new(AtomicBool::new(false));
    let waker = Waker::from(Arc::new(PanicWakeDrop(Arc::clone(&dropped))));
    let mut waiting = Box::pin(actor.send(Notify(2)));
    {
        let mut task = Context::from_waker(&waker);
        assert!(matches!(waiting.as_mut().poll(&mut task), Poll::Pending));
    }
    drop(waker);

    assert_eq!(
        owner.request_shutdown(Shutdown::Stop),
        ShutdownStatus::Requested
    );
    let resumed = {
        let mut task = Context::from_waker(Waker::noop());
        panic::catch_unwind(AssertUnwindSafe(|| waiting.as_mut().poll(&mut task)))
    };
    let Poll::Ready(Err(rejected)) = resumed.expect("Waker Drop remains contained") else {
        panic!("Stop must reject the waiting send");
    };

    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(rejected.into_message().0, 2);
    release_tx.send(()).unwrap();
    assert_eq!(watchdog(current).await.unwrap(), Ok(()));
    assert_eq!(watchdog(owner.wait()).await.reason(), ExitReason::Stopped);
}
