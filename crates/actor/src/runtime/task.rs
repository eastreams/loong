use super::*;

/// Builds actor communication state without scheduling actor code.
///
/// A child must receive its parent-issued key before its task can start.
/// Preparation keeps that ordering explicit without placeholder state.
pub struct PreparedActor<A: Actor> {
    actor_ref: ActorRef<A>,
    future: ErasedFuture<'static, ExitStatus>,
}

impl<A: Actor> PreparedActor<A> {
    pub(crate) fn new(args: A::SpawnArgs, options: SpawnOptions<A>) -> Self {
        // Resolve borrowed options before any value enters the spawned task.
        let (inner, inbox, scheduler) = ActorInner::open(&options);
        let actor_ref = ActorRef::new(inner);

        let state = ScopeState {
            actor_ref: actor_ref.clone(),
            children: <A as SupervisionConfig>::open_children(&options),
        };
        let future = Box::pin(run_actor(args, state, inbox, scheduler));

        Self { actor_ref, future }
    }

    /// Returns the actor address without scheduling actor code.
    pub(crate) fn actor_ref(&self) -> ActorRef<A> {
        self.actor_ref.clone()
    }

    pub(crate) fn start_root(self) -> ActorRef<A> {
        self.start_task(None)
    }

    fn start_task(self, parent: Option<ParentLink>) -> ActorRef<A> {
        let Self { actor_ref, future } = self;
        let exit = ExitGuard::new(Arc::clone(&actor_ref.0), parent);
        drop(tokio::spawn(ActorTask::new(future, exit)));
        actor_ref
    }
}

/// Starts only a child actor whose parent already owns it.
pub(crate) fn start_child<A: Actor>(registered: RegisteredChild<A>) -> Child<A> {
    let (prepared, parent, id) = registered.into_parts();
    let actor_ref = prepared.start_task(Some(parent));
    Child::new(id, actor_ref)
}

pub(crate) struct ExitGuard<A: Actor> {
    actor: Arc<ActorInner<A>>,
    parent: Option<ParentLink>,
    finished: bool,
}

impl<A: Actor> ExitGuard<A> {
    pub(crate) fn new(actor: Arc<ActorInner<A>>, parent: Option<ParentLink>) -> Self {
        Self {
            actor,
            parent,
            finished: false,
        }
    }

    fn prepare_abort(&self) {
        self.actor.control.begin_abort();
    }

    fn complete(mut self, status: ExitStatus) -> ExitStatus {
        self.publish(status)
    }

    fn publish(&mut self, proposed: ExitStatus) -> ExitStatus {
        if self.finished {
            return self
                .actor
                .control
                .exit_status()
                .expect("a finished guard published an exit status");
        }
        self.finished = true;
        let status = self.actor.control.finish(proposed);
        if let Some(parent) = &self.parent {
            parent.publish(status);
        }
        status
    }
}

impl<A: Actor> Drop for ExitGuard<A> {
    fn drop(&mut self) {
        if !self.finished {
            // An unfinished guard means ActorTask did not complete publication.
            // Mark abort before publishing its conservative status.
            self.prepare_abort();
            let status = ExitStatus::new(ExitReason::Aborted, SubtreeStatus::Unconfirmed);
            let _ = self.publish(status);
        }
    }
}

pub(crate) struct ActorTask<A: Actor> {
    future: Option<ErasedFuture<'static, ExitStatus>>,
    exit: Option<ExitGuard<A>>,
}

impl<A: Actor> ActorTask<A> {
    pub(crate) fn new(future: ErasedFuture<'static, ExitStatus>, exit: ExitGuard<A>) -> Self {
        Self {
            future: Some(future),
            exit: Some(exit),
        }
    }
}

impl<A: Actor> Future for ActorTask<A> {
    type Output = ExitStatus;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let result = this
            .future
            .as_mut()
            .expect("completed actor tasks are not polled again")
            .as_mut()
            .poll(context);

        let Poll::Ready(status) = result else {
            return Poll::Pending;
        };

        // Frame destruction can fail before terminal publication.
        // The lifecycle gate preserves any Kill that already committed.
        let exit = this.exit.take().expect("an actor task owns one exit guard");
        let future = this
            .future
            .take()
            .expect("a ready actor task owns one final frame");
        exit.actor.control.drop_user_value(future);
        let status = exit.complete(status);
        Poll::Ready(status)
    }
}

impl<A: Actor> Drop for ActorTask<A> {
    fn drop(&mut self) {
        let Some(exit) = self.exit.take() else {
            return;
        };
        // Aborting records a local abort before frame destruction.
        // The guard publishes only after contained cleanup returns.
        exit.prepare_abort();
        if let Some(future) = self.future.take() {
            exit.actor.control.drop_user_value(future);
        }
        drop(exit);
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Work<T = ()> {
    Complete(T),
    Killed,
    Panicked,
    // The ready output still belongs to the lifecycle owner.
    // It may require child-first teardown before being dropped.
    DropPanicked(T),
}

pin_project! {
    // `project_replace` leaves `Done` if future Drop panics.
    // `PinnedDrop` therefore cannot drop the future twice.
    // Separate state keeps replacement independent from guard destruction.
    #[project = ActorWorkStateProj]
    #[project_replace = ActorWorkStateProjReplace]
    pub(crate) enum ActorWorkState<F> {
        Running {
            #[pin]
            future: F,
        },
        Done,
    }
}

pin_project! {
    /// Contains polling and destruction for one pinned lifecycle future.
pub(crate) struct ActorWorkGuard<'a, F> {
        #[pin]
        pub(crate) state: ActorWorkState<F>,
        pub(crate) control: &'a Control,
    }

    impl<F> PinnedDrop for ActorWorkGuard<'_, F> {
        fn drop(this: Pin<&mut Self>) {
            let _ = this.drop_future_panicked();
        }
    }
}

impl<'a, F> ActorWorkGuard<'a, F> {
    /// Retires the pinned future and reports a contained Drop panic.
    fn drop_future_panicked(mut self: Pin<&mut Self>) -> bool {
        let this = self.as_mut().project();
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            let _ = this.state.project_replace(ActorWorkState::Done);
        }));
        if let Err(payload) = result {
            this.control.contain_panic(payload);
            true
        } else {
            false
        }
    }
}

impl<F, T> Future for ActorWorkGuard<'_, F>
where
    F: Future<Output = T>,
{
    type Output = Work<T>;

    fn poll(mut self: Pin<&mut Self>, task: &mut Context<'_>) -> Poll<Self::Output> {
        let result = {
            let this = self.as_mut().project();
            let ActorWorkStateProj::Running { future } = this.state.project() else {
                unreachable!("completed actor work cannot be polled");
            };
            panic::catch_unwind(AssertUnwindSafe(|| future.poll(task)))
        };

        match result {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(output)) => {
                if self.as_mut().drop_future_panicked() {
                    Poll::Ready(Work::DropPanicked(output))
                } else {
                    Poll::Ready(Work::Complete(output))
                }
            }
            Err(payload) => {
                self.as_mut().project().control.contain_panic(payload);
                let _ = self.as_mut().drop_future_panicked();
                Poll::Ready(Work::Panicked)
            }
        }
    }
}

pub(crate) async fn await_actor_work<F>(future: F, control: &Control) -> Work<F::Output>
where
    F: Future + Send,
{
    let guarded = ActorWorkGuard {
        state: ActorWorkState::Running { future },
        control,
    };
    tokio::pin!(guarded);

    loop {
        if matches!(
            control.mode(),
            Mode::Killing | Mode::Failing | Mode::Aborting | Mode::Exited(_)
        ) {
            return Work::Killed;
        }

        tokio::select! {
            biased;
            () = control.actor_notified() => {}
            result = &mut guarded => return result,
        }
    }
}
