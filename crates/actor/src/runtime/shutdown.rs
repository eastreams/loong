use super::*;

pub(crate) async fn stop_actor<A: Actor>(
    actor: &mut A,
    state: &mut ScopeState<A>,
    inbox: &mut ActorInbox<A>,
    control: &Control,
    owned: &OwnedTasks<A>,
    scheduler: &mut ActorScheduler<A>,
) -> ExitStatus {
    match close_and_discard(inbox, control, Mode::Stopping).await {
        DiscardOutcome::Complete => {}
        DiscardOutcome::ModeChanged => match control.mode() {
            Mode::Killing => return kill_actor(state, inbox, owned, scheduler).await,
            Mode::Failing => return fail_actor(state, inbox, owned, scheduler).await,
            // Lifecycle cannot return to a graceful mode. Aborting and Exited
            // belong to ActorTask's outer drop/publication path, which cannot
            // repoll this inner future after committing either state.
            mode => unreachable!("stop discard observed impossible mode: {mode:?}"),
        },
    }
    owned.close();
    match finish_replies(actor, state, control, owned, scheduler).await {
        Work::Complete(()) => {}
        Work::Killed => return kill_actor(state, inbox, owned, scheduler).await,
        Work::Panicked | Work::DropPanicked(()) => {
            return fail_actor(state, inbox, owned, scheduler).await;
        }
    }

    match graceful_finish(actor, state, control, Shutdown::Stop, ExitReason::Stopped).await {
        Work::Complete(()) => state.children().terminal_status(ExitReason::Stopped),
        Work::Killed => kill_actor(state, inbox, owned, scheduler).await,
        Work::Panicked | Work::DropPanicked(()) => fail_actor(state, inbox, owned, scheduler).await,
    }
}

pub(crate) async fn drain_actor<A: Actor>(
    actor: &mut A,
    state: &mut ScopeState<A>,
    inbox: &mut ActorInbox<A>,
    inner: &Arc<ActorInner<A>>,
    owned: &OwnedTasks<A>,
    scheduler: &mut ActorScheduler<A>,
) -> ExitStatus {
    let control = &inner.control;
    // Admission physically enqueues under the lifecycle transaction, so this
    // queue is stable once Drain commits. Capacity permits that never reached
    // admission are not accepted work and must not extend graceful shutdown.
    inbox.close();
    let mut inbox_drained = inbox.is_empty();
    if inbox_drained {
        owned.close();
    }
    loop {
        match control.mode() {
            Mode::Killing => {
                return kill_actor(state, inbox, owned, scheduler).await;
            }
            Mode::Failing => {
                return fail_actor(state, inbox, owned, scheduler).await;
            }
            Mode::Running | Mode::Draining | Mode::Stopping => {}
            Mode::Exited(status) => return status,
            Mode::Aborting => {
                return ExitStatus::new(ExitReason::Aborted, SubtreeStatus::Unconfirmed);
            }
        }

        if !inbox_drained && inbox.is_empty() {
            inbox_drained = true;
            owned.close();
        }

        let turn = AssertUnwindSafe(drain_turn(
            actor,
            state,
            inbox,
            inner,
            owned,
            scheduler,
            !inbox_drained,
        ))
        .catch_unwind()
        .await;

        let turn = match turn {
            Ok(turn) => turn,
            Err(payload) => {
                control.contain_panic(payload);
                return fail_actor(state, inbox, owned, scheduler).await;
            }
        };

        match turn {
            DrainTurn::RepliesFinished => break,
            DrainTurn::Scheduled(SchedulerTurn::LifecycleHint | SchedulerTurn::Progress) => {}
            DrainTurn::Scheduled(SchedulerTurn::Child(event)) => {
                match handle_child_exit(actor, state, event, control).await {
                    Work::Complete(()) => {}
                    Work::Killed => {
                        return kill_actor(state, inbox, owned, scheduler).await;
                    }
                    Work::Panicked | Work::DropPanicked(()) => {
                        control.begin_failure();
                        return fail_actor(state, inbox, owned, scheduler).await;
                    }
                }
            }
            DrainTurn::Scheduled(SchedulerTurn::InboxClosed) => {
                inbox_drained = true;
                owned.close();
            }
        }
    }

    match graceful_finish(actor, state, control, Shutdown::Drain, ExitReason::Drained).await {
        Work::Complete(()) => state.children().terminal_status(ExitReason::Drained),
        Work::Killed => kill_actor(state, inbox, owned, scheduler).await,
        Work::Panicked | Work::DropPanicked(()) => fail_actor(state, inbox, owned, scheduler).await,
    }
}

async fn finish_replies<A: Actor>(
    actor: &mut A,
    state: &mut ScopeState<A>,
    control: &Control,
    owned: &OwnedTasks<A>,
    scheduler: &mut ActorScheduler<A>,
) -> Work {
    while !RuntimeScheduler::is_idle(scheduler) {
        match control.mode() {
            Mode::Killing => return Work::Killed,
            Mode::Failing => return Work::Panicked,
            Mode::Running | Mode::Draining | Mode::Stopping => {}
            Mode::Aborting | Mode::Exited(_) => return Work::Killed,
        }

        let result = AssertUnwindSafe(async {
            tokio::select! {
                biased;
                () = control.actor_notified() => {}
                () = std::future::poll_fn(|task| {
                    let mut scope = state.actor_scope();
                    RuntimeScheduler::poll_actor_replies(
                        scheduler,
                        actor,
                        &mut scope,
                        control,
                        Mode::Stopping,
                        task,
                    )
                }) => {}
            }
        })
        .catch_unwind()
        .await;

        if let Err(payload) = result {
            control.contain_panic(payload);
            return Work::Panicked;
        }
    }

    loop {
        match control.mode() {
            Mode::Killing => return Work::Killed,
            Mode::Failing => return Work::Panicked,
            Mode::Running | Mode::Draining | Mode::Stopping => {}
            Mode::Aborting | Mode::Exited(_) => return Work::Killed,
        }

        let result = AssertUnwindSafe(async {
            tokio::select! {
                biased;
                () = control.actor_notified() => false,
                () = owned.wait() => true,
            }
        })
        .catch_unwind()
        .await;

        match result {
            Ok(true) => return Work::Complete(()),
            Ok(false) => {}
            Err(payload) => {
                control.contain_panic(payload);
                return Work::Panicked;
            }
        }
    }
}

/// Graceful shutdown is post-order: a parent finishes the work retained by the
/// selected mode, then waits for children, and only then runs its cleanup hook.
pub(crate) async fn graceful_finish<A: Actor>(
    actor: &mut A,
    state: &mut ScopeState<A>,
    control: &Control,
    shutdown: Shutdown,
    reason: ExitReason,
) -> Work {
    // Keep child submission inside the biased lifecycle guard. A Kill already
    // committed before this poll must win before Stop or Drain reaches children.
    match await_actor_work(
        async {
            state.children().request_all(shutdown);
            state.children().wait_all().await;
        },
        control,
    )
    .await
    {
        Work::Complete(()) => {}
        Work::Killed => return Work::Killed,
        Work::Panicked => return Work::Panicked,
        Work::DropPanicked(()) => return Work::DropPanicked(()),
    }

    await_actor_work(
        async {
            let mut scope = state.stop_scope();
            actor.on_stop(reason, &mut scope).await;
        },
        control,
    )
    .await
}

pub(crate) async fn kill_actor<A: Actor>(
    state: &mut ScopeState<A>,
    inbox: &mut ActorInbox<A>,
    owned: &OwnedTasks<A>,
    scheduler: &mut ActorScheduler<A>,
) -> ExitStatus {
    let inner = Arc::clone(&state.actor_ref.0);
    let control = &inner.control;
    // Commit subtree cancellation before running arbitrary Drop code from actor
    // work. Children can then begin terminating even if a destructor is slow.
    inbox.close();
    state.children().request_all(Shutdown::Kill);
    owned.close();
    RuntimeScheduler::clear(scheduler, control);
    let mut expected_mode = Mode::Killing;
    loop {
        match close_and_discard(inbox, control, expected_mode).await {
            DiscardOutcome::Complete => break,
            DiscardOutcome::ModeChanged => expected_mode = control.mode(),
        }
    }
    owned.wait().await;
    state.children().wait_all().await;
    state.children().terminal_status(ExitReason::Killed)
}

pub(crate) async fn fail_actor<A: Actor>(
    state: &mut ScopeState<A>,
    inbox: &mut ActorInbox<A>,
    owned: &OwnedTasks<A>,
    scheduler: &mut ActorScheduler<A>,
) -> ExitStatus {
    let inner = Arc::clone(&state.actor_ref.0);
    let control = &inner.control;
    control.begin_failure();
    let reason = match control.mode() {
        Mode::Killing => ExitReason::Killed,
        Mode::Aborting => ExitReason::Aborted,
        Mode::Exited(status) => return status,
        Mode::Running | Mode::Draining | Mode::Stopping | Mode::Failing => ExitReason::Panicked,
    };
    inbox.close();
    state.children().request_all(Shutdown::Kill);
    owned.close();
    RuntimeScheduler::clear(scheduler, control);
    let mut expected_mode = control.mode();
    loop {
        match close_and_discard(inbox, control, expected_mode).await {
            DiscardOutcome::Complete => break,
            DiscardOutcome::ModeChanged => expected_mode = control.mode(),
        }
    }
    owned.wait().await;
    state.children().wait_all().await;
    state.children().terminal_status(reason)
}

pub(crate) const TEARDOWN_DROP_BUDGET: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiscardOutcome {
    Complete,
    ModeChanged,
}

/// Closes the inbox and drops its accepted envelopes in bounded batches.
///
/// Each envelope may run arbitrary user destructors. The lifecycle mode is
/// checked before selecting a value and again after dropping it, so a transition
/// returns [`DiscardOutcome::ModeChanged`] before another value is selected.
/// Every uninterrupted mode pass yields after [`TEARDOWN_DROP_BUDGET`] drops so
/// a large accepted queue cannot monopolize a current-thread executor.
pub(crate) async fn close_and_discard<A: Actor>(
    inbox: &mut ActorInbox<A>,
    control: &Control,
    expected_mode: Mode,
) -> DiscardOutcome {
    inbox.close();
    let mut dropped = 0;
    loop {
        if control.mode() != expected_mode {
            return DiscardOutcome::ModeChanged;
        }

        if !inbox.try_discard() {
            return DiscardOutcome::Complete;
        }

        if control.mode() != expected_mode {
            return DiscardOutcome::ModeChanged;
        }
        dropped += 1;
        if dropped == TEARDOWN_DROP_BUDGET {
            dropped = 0;
            tokio::task::yield_now().await;
        }
    }
}
