use super::*;

// A catch owns its panic payload after the first unwind ends.
// Payload destruction must not start a second runtime unwind.
#[tokio::test]
async fn actor_work_contains_panic_payload_destruction() {
    let control = Control::new();
    let payload_dropped = Arc::new(AtomicBool::new(false));

    let work = await_actor_work(
        async {
            panic::panic_any(CascadingPanicPayload(Arc::clone(&payload_dropped)));
        },
        &control,
    )
    .await;

    assert!(matches!(work, Work::Panicked));
    assert!(payload_dropped.load(Ordering::SeqCst));
    assert_eq!(control.mode(), Mode::Failing);
}

// Ready Drop is checked in every graceful mode.
// Kill after entry locks hard-cutoff ordering.
// Poll panic proves polling and Drop use separate boundaries.
// The !Unpin probe rejects move-based cleanup.
#[tokio::test]
async fn actor_work_contains_future_drop_panics() {
    for (initial, kill_on_poll, panic_on_poll, expected_work, expected_mode) in [
        (None, false, false, Work::DropPanicked(()), Mode::Failing),
        (
            Some(Shutdown::Stop),
            false,
            false,
            Work::DropPanicked(()),
            Mode::Failing,
        ),
        (
            Some(Shutdown::Drain),
            false,
            false,
            Work::DropPanicked(()),
            Mode::Failing,
        ),
        (None, true, false, Work::Killed, Mode::Killing),
        (None, false, true, Work::Panicked, Mode::Failing),
    ] {
        let control = Control::new();
        if let Some(shutdown) = initial {
            assert_eq!(control.request(shutdown), ShutdownStatus::Requested);
        }
        let polled = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false));

        let work = await_actor_work(
            ActorWorkDropProbe {
                kill: kill_on_poll.then_some(&control),
                panic_on_poll,
                polled: Arc::clone(&polled),
                dropped: Arc::clone(&dropped),
                _pin: PhantomPinned,
            },
            &control,
        )
        .await;

        assert_eq!(work, expected_work);
        assert!(polled.load(Ordering::SeqCst));
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(control.mode(), expected_mode);
    }
}

// Output may leave containment only after its ready frame is retired.
// Returning first would leave user frame cleanup pending after delivery.
#[test]
fn actor_work_retires_ready_frame_before_delivering_output() {
    let control = Control::new();
    let frame_dropped = Arc::new(AtomicBool::new(false));
    let output_dropped = Arc::new(AtomicBool::new(false));
    let mut task = Context::from_waker(Waker::noop());
    let mut guarded = std::pin::pin!(ActorWorkGuard {
        state: ActorWorkState::Running {
            future: ReadyActorWorkFrame {
                output: Some(ActorWorkOutputDropProbe {
                    dropped: Arc::clone(&output_dropped),
                    panic_on_drop: false,
                }),
                dropped: Arc::clone(&frame_dropped),
                panic_on_drop: false,
            },
        },
        control: &control,
    });

    let Poll::Ready(Work::Complete(output)) = guarded.as_mut().poll(&mut task) else {
        panic!("ready work must deliver its output");
    };

    assert!(frame_dropped.load(Ordering::SeqCst));
    assert!(!output_dropped.load(Ordering::SeqCst));
    assert_eq!(control.mode(), Mode::Running);
    drop(output);
    assert!(output_dropped.load(Ordering::SeqCst));
}

// A frame Drop panic invalidates its ready output.
// The lifecycle owner retains that output for ordered teardown.
#[test]
fn actor_work_retains_invalidated_ready_output_for_ordered_drop() {
    let control = Control::new();
    let frame_dropped = Arc::new(AtomicBool::new(false));
    let output_dropped = Arc::new(AtomicBool::new(false));
    let mut task = Context::from_waker(Waker::noop());
    let mut guarded = std::pin::pin!(ActorWorkGuard {
        state: ActorWorkState::Running {
            future: ReadyActorWorkFrame {
                output: Some(ActorWorkOutputDropProbe {
                    dropped: Arc::clone(&output_dropped),
                    panic_on_drop: true,
                }),
                dropped: Arc::clone(&frame_dropped),
                panic_on_drop: true,
            },
        },
        control: &control,
    });

    let Poll::Ready(Work::DropPanicked(output)) = guarded.as_mut().poll(&mut task) else {
        panic!("frame Drop failure must retain its ready output");
    };
    assert!(frame_dropped.load(Ordering::SeqCst));
    assert!(!output_dropped.load(Ordering::SeqCst));
    assert_eq!(control.mode(), Mode::Failing);

    control.drop_user_value(output);
    assert!(output_dropped.load(Ordering::SeqCst));
}

// Polling after Ready is a runtime contract violation, not an actor panic.
// The invariant check must remain outside the user future panic boundary.
#[test]
fn completed_actor_work_repoll_exposes_the_runtime_bug() {
    let control = Control::new();
    let mut task = Context::from_waker(Waker::noop());
    let mut guarded = std::pin::pin!(ActorWorkGuard {
        state: ActorWorkState::Running {
            future: std::future::ready(()),
        },
        control: &control,
    });
    assert_eq!(
        guarded.as_mut().poll(&mut task),
        Poll::Ready(Work::Complete(()))
    );

    let repoll = panic::catch_unwind(panic::AssertUnwindSafe(|| {
        let _ = guarded.as_mut().poll(&mut task);
    }));

    assert!(repoll.is_err());
    assert_eq!(control.mode(), Mode::Running);
}

// Final frame Drop occurs before terminal publication.
// Its panic loses only when a committed Kill already won.
#[test]
fn actor_task_contains_final_frame_drop_panic() {
    for (shutdown, expected_reason) in [
        (None, ExitReason::Panicked),
        (Some(Shutdown::Kill), ExitReason::Killed),
    ] {
        let (inner, _inbox) = test_actor_inner(1);
        let control = &inner.control;
        if let Some(shutdown) = shutdown {
            assert_eq!(control.request(shutdown), ShutdownStatus::Requested);
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let mut actor_task = Box::pin(ActorTask::new(
            Box::pin(ActorFrameDropProbe {
                status: ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated),
                dropped: Arc::clone(&dropped),
            }),
            ExitGuard::new(Arc::clone(&inner), None),
        ));
        let mut task = Context::from_waker(Waker::noop());
        let expected = ExitStatus::new(expected_reason, SubtreeStatus::Terminated);

        assert_eq!(actor_task.as_mut().poll(&mut task), Poll::Ready(expected));
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(control.mode(), Mode::Exited(expected));
    }
}

// Executor teardown commits Aborting before dropping the frame.
// A contained Drop panic cannot escape or strengthen Aborted.
#[test]
fn actor_task_contains_aborted_frame_drop_panic() {
    let (inner, _inbox) = test_actor_inner(1);
    let control = &inner.control;
    let saw_aborting = Arc::new(AtomicBool::new(false));
    let actor_task = ActorTask::new(
        Box::pin(AbortedFrameDropProbe {
            inner: Arc::clone(&inner),
            saw_aborting: Arc::clone(&saw_aborting),
        }),
        ExitGuard::new(Arc::clone(&inner), None),
    );

    drop(actor_task);

    assert!(saw_aborting.load(Ordering::SeqCst));
    assert_eq!(
        control.mode(),
        Mode::Exited(ExitStatus::new(
            ExitReason::Aborted,
            SubtreeStatus::Unconfirmed,
        ))
    );
}

// An unfinished guard means terminal publication never completed.
// Its fallback must override any tentative hard mode.
#[test]
fn unfinished_exit_guard_overrides_tentative_hard_mode() {
    let expected = ExitStatus::new(ExitReason::Aborted, SubtreeStatus::Unconfirmed);

    let (killing, _inbox) = test_actor_inner(1);
    assert_eq!(
        killing.control.request(Shutdown::Kill),
        ShutdownStatus::Requested
    );
    drop(ExitGuard::new(Arc::clone(&killing), None));
    assert_eq!(killing.control.exit_status(), Some(expected));

    let (failing, _inbox) = test_actor_inner(1);
    failing.control.begin_failure();
    drop(ExitGuard::new(Arc::clone(&failing), None));
    assert_eq!(failing.control.exit_status(), Some(expected));
}

// An aborted child reports uncertainty without stopping a running parent.
// The parent must still dispatch messages before a later Stop.
