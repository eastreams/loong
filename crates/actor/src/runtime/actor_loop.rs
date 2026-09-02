use super::shutdown::{drain_actor, fail_actor, stop_actor};
use super::*;

pub(crate) async fn run_actor<A: Actor>(
    args: A::SpawnArgs,
    mut state: ScopeState<A>,
    mut inbox: ActorInbox<A>,
    mut scheduler: ActorScheduler<A>,
) -> ExitStatus {
    let inner = Arc::clone(&state.actor_ref.0);
    let control = &inner.control;
    let owned = OwnedTasks::new(Arc::clone(&inner));

    let initialized = if let Some(_permit) = control.begin_initialization() {
        let mut scope = state.actor_scope();
        match panic::catch_unwind(AssertUnwindSafe(|| A::init(args, &mut scope))) {
            Ok(init) => await_actor_work(init, control).await,
            Err(payload) => {
                control.contain_panic(payload);
                Work::Panicked
            }
        }
    } else {
        control.drop_user_value(args);
        Work::Killed
    };

    let mut actor = match initialized {
        Work::Complete(actor) => actor,
        Work::Killed => return kill_actor(&mut state, &mut inbox, &owned, &mut scheduler).await,
        Work::Panicked => return fail_actor(&mut state, &mut inbox, &owned, &mut scheduler).await,
        Work::DropPanicked(actor) => {
            // The init frame failed after producing actor state.
            // Descendant cancellation must precede arbitrary actor Drop code.
            state.children().request_all(Shutdown::Kill);
            control.drop_user_value(actor);
            return fail_actor(&mut state, &mut inbox, &owned, &mut scheduler).await;
        }
    };

    // The scheduler owns every actor-aware reply future, and those futures may
    // still hold `Cx` handles into `actor` and `state` when they are dropped.
    // Re-bind `scheduler` as a local declared after `actor` so async-fn drop
    // glue releases it before `actor` on every exit path, including an aborted
    // actor task. Otherwise an abort could drop the actor first and let a
    // future's `Drop` call `Cx::with` through dangling pointers.
    let mut scheduler = scheduler;

    loop {
        match control.mode() {
            Mode::Running => {}
            Mode::Draining => {
                match panic::catch_unwind(AssertUnwindSafe(|| actor.on_shutdown(Shutdown::Drain))) {
                    Ok(()) => {}
                    Err(payload) => {
                        control.contain_panic(payload);
                        return fail_actor(&mut state, &mut inbox, &owned, &mut scheduler).await;
                    }
                }
                return drain_actor(
                    &mut actor,
                    &mut state,
                    &mut inbox,
                    &inner,
                    &owned,
                    &mut scheduler,
                )
                .await;
            }
            Mode::Stopping => {
                match panic::catch_unwind(AssertUnwindSafe(|| actor.on_shutdown(Shutdown::Stop))) {
                    Ok(()) => {}
                    Err(payload) => {
                        control.contain_panic(payload);
                        return fail_actor(&mut state, &mut inbox, &owned, &mut scheduler).await;
                    }
                }
                return stop_actor(
                    &mut actor,
                    &mut state,
                    &mut inbox,
                    control,
                    &owned,
                    &mut scheduler,
                )
                .await;
            }
            Mode::Killing => {
                return kill_actor(&mut state, &mut inbox, &owned, &mut scheduler).await;
            }
            Mode::Failing => {
                return fail_actor(&mut state, &mut inbox, &owned, &mut scheduler).await;
            }
            Mode::Exited(status) => return status,
            Mode::Aborting => {
                return ExitStatus::new(ExitReason::Aborted, SubtreeStatus::Unconfirmed);
            }
        }

        let turn = AssertUnwindSafe(actor_turn(
            &mut actor,
            &mut state,
            &mut inbox,
            &inner,
            &owned,
            &mut scheduler,
            true,
            Mode::Running,
        ))
        .catch_unwind()
        .await;

        let turn = match turn {
            Ok(turn) => turn,
            Err(payload) => {
                control.contain_panic(payload);
                return fail_actor(&mut state, &mut inbox, &owned, &mut scheduler).await;
            }
        };

        match turn {
            SchedulerTurn::LifecycleHint | SchedulerTurn::Progress => {}
            SchedulerTurn::Child(event) => {
                match handle_child_exit(&mut actor, &mut state, event, control).await {
                    Work::Complete(()) => {}
                    Work::Killed => {
                        return kill_actor(&mut state, &mut inbox, &owned, &mut scheduler).await;
                    }
                    Work::Panicked | Work::DropPanicked(()) => {
                        control.begin_failure();
                        return fail_actor(&mut state, &mut inbox, &owned, &mut scheduler).await;
                    }
                }
            }
            SchedulerTurn::InboxClosed => {
                control.begin_failure();
                return fail_actor(&mut state, &mut inbox, &owned, &mut scheduler).await;
            }
        }
    }
}

pub(crate) enum DrainTurn {
    Scheduled(SchedulerTurn),
    RepliesFinished,
}

// Scheduler work keeps priority over the owned-task completion barrier.
// The nested scheduler turn preserves lifecycle-first polling.
pub(crate) async fn drain_turn<A: Actor>(
    actor: &mut A,
    state: &mut ScopeState<A>,
    inbox: &mut ActorInbox<A>,
    inner: &Arc<ActorInner<A>>,
    owned: &OwnedTasks<A>,
    scheduler: &mut ActorScheduler<A>,
    receive_messages: bool,
) -> DrainTurn {
    let wait_for_owned = !receive_messages && RuntimeScheduler::is_idle(scheduler);

    tokio::select! {
        biased;
        turn = actor_turn(
            actor,
            state,
            inbox,
            inner,
            owned,
            scheduler,
            receive_messages,
            Mode::Draining,
        ) => DrainTurn::Scheduled(turn),
        () = owned.wait(), if wait_for_owned => DrainTurn::RepliesFinished,
    }
}

pub(crate) async fn handle_child_exit<A: Actor>(
    actor: &mut A,
    state: &mut ScopeState<A>,
    event: ChildExit,
    control: &Control,
) -> Work {
    if !state.children().reap(&event) {
        return Work::Complete(());
    }

    let Some(permit) = state.actor_ref.0.control.begin_child_hook() else {
        return Work::Complete(());
    };

    run_child_exit_hook(actor, state, event, control, permit).await
}

/// Makes the private gate proof mandatory at the only user hook call site.
async fn run_child_exit_hook<A: Actor>(
    actor: &mut A,
    state: &mut ScopeState<A>,
    event: ChildExit,
    control: &Control,
    _permit: HookEntryPermit,
) -> Work {
    await_actor_work(
        async {
            let mut scope = state.actor_scope();
            actor.on_child_exit(event, &mut scope).await;
        },
        control,
    )
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "one turn borrows each independent actor-task resource"
)]
// Scheduler profiles own their eligible lane rotation.
// Lifecycle keeps first poll rights across every profile.
pub(crate) async fn actor_turn<A: Actor>(
    actor: &mut A,
    state: &mut ScopeState<A>,
    inbox: &mut ActorInbox<A>,
    inner: &Arc<ActorInner<A>>,
    owned: &OwnedTasks<A>,
    scheduler: &mut ActorScheduler<A>,
    receive_messages: bool,
    expected_mode: Mode,
) -> SchedulerTurn {
    let control = &inner.control;
    let fair_turn = std::future::poll_fn(|task| {
        let mut turn = TurnContext {
            actor,
            state,
            inbox,
            inner,
            owned,
            receive_messages,
            expected_mode,
        };
        RuntimeScheduler::poll_turn(scheduler, &mut turn, task)
    });

    // The biased select lets lifecycle notifications, especially Kill,
    // preempt the fair scheduler turn.
    tokio::select! {
        biased;
        () = control.actor_notified() => SchedulerTurn::LifecycleHint,
        turn = fair_turn => turn,
    }
}
