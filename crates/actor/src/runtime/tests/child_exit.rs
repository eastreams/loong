use super::*;

#[tokio::test]
async fn aborted_child_does_not_stop_running_parent() {
    let (child_exit_tx, child_exit_rx) = oneshot::channel();
    let owner = loac::spawn::<AbortChildParent>(child_exit_tx);
    let actor = owner.actor_ref();

    let child_status = tokio::time::timeout(Duration::from_secs(1), child_exit_rx)
        .await
        .expect("the running parent must consume the child event")
        .expect("the child hook must complete");
    assert_eq!(child_status.reason(), ExitReason::Aborted);
    assert_eq!(child_status.subtree(), SubtreeStatus::Unconfirmed);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), actor.call(Ping))
            .await
            .expect("the parent must still dispatch mailbox work"),
        Ok(())
    );

    let status = tokio::time::timeout(Duration::from_secs(1), owner.shutdown(Shutdown::Stop))
        .await
        .expect("parent Stop must finish");
    assert_eq!(
        status,
        ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Unconfirmed)
    );
}

// A descendant abort weakens only the parent's subtree guarantee.
// The parent's own shutdown or panic reason must remain intact.
#[tokio::test]
async fn aborted_descendant_only_weakens_parent_subtree_status() {
    enum ParentExit {
        Shutdown(Shutdown),
        Panic,
    }

    for (exit, expected_reason) in [
        (ParentExit::Shutdown(Shutdown::Stop), ExitReason::Stopped),
        (ParentExit::Shutdown(Shutdown::Drain), ExitReason::Drained),
        (ParentExit::Shutdown(Shutdown::Kill), ExitReason::Killed),
        (ParentExit::Panic, ExitReason::Panicked),
    ] {
        let (child, _child_inbox) = test_actor_inner(1);
        child.control.begin_abort();
        child.control.finish(ExitStatus::new(
            ExitReason::Aborted,
            SubtreeStatus::Unconfirmed,
        ));
        let (inner, inbox) = test_actor_inner(1);
        let control = &inner.control;
        let mut scope = scope_state(&inner);
        let child_ref = ActorRef::new(Arc::clone(&child));
        scope.children.insert_ref(&child_ref);

        match exit {
            ParentExit::Shutdown(shutdown) => {
                assert_eq!(control.request(shutdown), ShutdownStatus::Requested);
            }
            ParentExit::Panic => enqueue_test_envelope(&inner, PanicEnvelope),
        }

        let options =
            <TestActor as ActorConfig>::Options::default().with_max_in_flight(NonZeroUsize::MIN);
        let (_, _, scheduler) = TestActor::open(&options);
        let task = ActorTask::new(
            Box::pin(run_actor::<TestActor>((), scope, inbox, scheduler)),
            ExitGuard::new(Arc::clone(&inner), None),
        );
        let status = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("parent teardown must finish");
        let expected = ExitStatus::new(expected_reason, SubtreeStatus::Unconfirmed);

        assert_eq!(status, expected);
        assert_eq!(control.exit_status(), Some(expected));
    }
}

// Dequeue observes completion but cannot release ownership.
// Reaping happens only when the event is handled.
#[tokio::test]
async fn dequeued_child_exit_keeps_registration_until_handled() {
    let observed = Arc::new(AtomicUsize::new(0));
    let (child, _child_inbox) = test_actor_inner(1);
    let status = ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated);
    child.control.finish(status);

    let options = <CountChildExit as ActorConfig>::Options::default();
    let (inner, _inbox, _scheduler) = ActorInner::<CountChildExit>::open(&options);
    let control = &inner.control;
    let mut state = scope_state(&inner);
    let child_ref = ActorRef::new(Arc::clone(&child));
    let child_id = state.children.insert_ref(&child_ref);
    state.children.publish(ChildExit::new(child_id, status));
    let event = std::future::poll_fn(|task| state.poll_child_exit(task)).await;
    assert_eq!(state.children.len(), 1);

    let mut actor = CountChildExit(Arc::clone(&observed));
    assert!(matches!(
        handle_child_exit(&mut actor, &mut state, event, control).await,
        Work::Complete(())
    ));
    assert_eq!(state.children.len(), 0);
    assert_eq!(observed.load(Ordering::SeqCst), 1);
}

// This recreates the original race window after actor_turn has dequeued a
// valid event. A graceful cutoff that commits in that window must retire the
// child without entering user code, for both graceful modes.
// A weak grandchild status must survive a normal direct-child reason.
#[tokio::test]
async fn graceful_cutoff_absorbs_a_dequeued_child_exit() {
    for shutdown in [Shutdown::Stop, Shutdown::Drain] {
        let strong = match shutdown {
            Shutdown::Stop => ExitReason::Stopped,
            Shutdown::Drain => ExitReason::Drained,
            Shutdown::Kill => unreachable!("the test uses graceful modes"),
        };
        let observed = Arc::new(AtomicUsize::new(0));
        let (child, _child_inbox) = test_actor_inner(1);

        let options = <CountChildExit as ActorConfig>::Options::default();
        let (inner, _inbox, _scheduler) = ActorInner::<CountChildExit>::open(&options);
        let control = &inner.control;
        let mut scope = scope_state(&inner);
        let child_ref = ActorRef::new(Arc::clone(&child));
        let child_id = scope.children.insert_ref(&child_ref);
        let mut actor = CountChildExit(Arc::clone(&observed));
        assert_eq!(control.request(shutdown), ShutdownStatus::Requested);
        assert!(matches!(
            handle_child_exit(
                &mut actor,
                &mut scope,
                ChildExit::new(
                    child_id,
                    ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Unconfirmed),
                ),
                control,
            )
            .await,
            Work::Complete(())
        ));
        assert_eq!(observed.load(Ordering::SeqCst), 0);
        assert_eq!(scope.children.len(), 0);
        assert_eq!(
            scope.children.terminal_status(strong),
            ExitStatus::new(strong, SubtreeStatus::Unconfirmed)
        );
    }
}

// Once hook entry wins the lifecycle gate, Stop and Drain must wait for that
// serial hook rather than cancelling it or publishing graceful completion
// around it. Kill cancellation is covered separately by lifecycle hook tests.
#[tokio::test]
async fn admitted_child_exit_hook_finishes_across_graceful_cutoff() {
    for shutdown in [Shutdown::Stop, Shutdown::Drain] {
        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let (completed_tx, completed_rx) = oneshot::channel();
        let (child, _child_inbox) = test_actor_inner(1);

        let options = <ControlledChildExit as ActorConfig>::Options::default();
        let (inner, _inbox, _scheduler) = ActorInner::<ControlledChildExit>::open(&options);
        let control = &inner.control;
        let mut scope = scope_state(&inner);
        let child_ref = ActorRef::new(Arc::clone(&child));
        let child_id = scope.children.insert_ref(&child_ref);
        let mut actor = ControlledChildExit {
            entered: Some(entered_tx),
            release: Some(release_rx),
            completed: Some(completed_tx),
        };
        let controller = ActorRef::new(Arc::clone(&inner));

        let (work, ()) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(
                handle_child_exit(
                    &mut actor,
                    &mut scope,
                    ChildExit::new(
                        child_id,
                        ExitStatus::new(ExitReason::Stopped, SubtreeStatus::Terminated),
                    ),
                    control,
                ),
                async move {
                    entered_rx.await.unwrap();
                    assert_eq!(
                        controller.request_shutdown(shutdown),
                        ShutdownStatus::Requested
                    );
                    release_tx.send(()).unwrap();
                }
            )
        })
        .await
        .expect("an admitted hook must remain live across graceful cutoff");

        assert!(matches!(work, Work::Complete(())));
        completed_rx.await.unwrap();
        assert_eq!(scope.children.len(), 0);
    }
}

// Reserving capacity is not admission. Drain must finish from the stable queue
// snapshot even if an internal raw permit remains alive and keeps mpsc from
// reporting channel termination.
