use std::future::{Future, poll_fn};

use loong_actor::{Actor, ActorScope, ExitReason, Shutdown, ShutdownStatus, spawn};
use tokio::sync::oneshot;

use super::{fixtures::DropSignal, support::watchdog};

struct ControlledStart {
    entered: Option<oneshot::Sender<()>>,
    repolled: Option<oneshot::Sender<()>>,
    release: Option<oneshot::Receiver<()>>,
    completed: Option<oneshot::Sender<()>>,
    dropped: Option<oneshot::Sender<()>>,
}

impl Actor for ControlledStart {
    async fn on_start(&mut self, _scope: &mut ActorScope<Self>) {
        let release = self.release.take();
        let mut repolled = self.repolled.take();
        let completed = self.completed.take();
        let _dropped = DropSignal(self.dropped.take());
        if let Some(entered) = self.entered.take() {
            let _ = entered.send(());
        }
        if let Some(release) = release {
            // Manual polling must pin the oneshot receiver before obtaining Pin<&mut _>.
            let mut release = std::pin::pin!(release);
            // This wrapper only observes polls; the receiver below owns wake registration.
            let mut first_poll = true;
            let _ = poll_fn(|context| {
                // The first poll parks on release. A later poll proves that a
                // lifecycle wake reached the still-pending hook.
                if first_poll {
                    first_poll = false;
                } else if let Some(repolled) = repolled.take() {
                    let _ = repolled.send(());
                }
                release.as_mut().poll(context)
            })
            .await;
        }
        if let Some(completed) = completed {
            let _ = completed.send(());
        }
    }
}

#[tokio::test]
async fn stop_and_drain_wait_for_an_entered_lifecycle_hook() {
    for (shutdown, expected_reason) in [
        (Shutdown::Stop, ExitReason::Stopped),
        (Shutdown::Drain, ExitReason::Drained),
    ] {
        let (entered_tx, entered_rx) = oneshot::channel();
        let (repolled_tx, repolled_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let (completed_tx, completed_rx) = oneshot::channel();
        let mut owner = spawn(ControlledStart {
            entered: Some(entered_tx),
            repolled: Some(repolled_tx),
            release: Some(release_rx),
            completed: Some(completed_tx),
            dropped: None,
        });
        let actor = owner.actor_ref();

        watchdog(entered_rx).await.unwrap();
        assert_eq!(owner.request_shutdown(shutdown), ShutdownStatus::Requested);
        // Observe the lifecycle-driven repoll before releasing the hook, so the
        // test cannot pass merely because shutdown and release become ready together.
        watchdog(repolled_rx).await.unwrap();
        assert_eq!(owner.exit_status(), None);
        assert_eq!(actor.exit_status(), None);
        assert_eq!(
            owner.request_shutdown(shutdown),
            ShutdownStatus::InProgress(shutdown)
        );

        release_tx.send(()).unwrap();
        watchdog(completed_rx).await.unwrap();
        assert_eq!(watchdog(owner.wait()).await.reason(), expected_reason);
        assert_eq!(actor.exit_status().unwrap().reason(), expected_reason);
    }
}

#[tokio::test]
async fn kill_cancels_a_pending_lifecycle_hook() {
    let (entered_tx, entered_rx) = oneshot::channel();
    // Keep the sender alive so only Kill, not channel closure, can finish the hook.
    let (_release_tx, release_rx) = oneshot::channel();
    let (completed_tx, completed_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    let mut owner = spawn(ControlledStart {
        entered: Some(entered_tx),
        repolled: None,
        release: Some(release_rx),
        completed: Some(completed_tx),
        dropped: Some(dropped_tx),
    });
    let actor = owner.actor_ref();

    watchdog(entered_rx).await.unwrap();
    assert_eq!(
        owner.request_shutdown(Shutdown::Kill),
        ShutdownStatus::Requested
    );

    // DropSignal proves the pending hook was cancelled instead of completing normally.
    watchdog(dropped_rx).await.unwrap();
    assert!(watchdog(completed_rx).await.is_err());
    assert_eq!(watchdog(owner.wait()).await.reason(), ExitReason::Killed);
    assert_eq!(actor.exit_status().unwrap().reason(), ExitReason::Killed);
}

struct PanicOnStart;

impl Actor for PanicOnStart {
    async fn on_start(&mut self, _scope: &mut ActorScope<Self>) {
        panic!("intentional lifecycle hook panic");
    }
}

#[tokio::test]
async fn lifecycle_hook_panics_are_contained_and_reported() {
    let mut owner = spawn(PanicOnStart);
    let actor = owner.actor_ref();

    assert_eq!(watchdog(owner.wait()).await.reason(), ExitReason::Panicked);
    assert_eq!(actor.exit_status().unwrap().reason(), ExitReason::Panicked);
}
