// The pin-projected state machines in this file are derived from Actix 0.13.5's
// ActorFuture combinators under the MIT license. See THIRD_PARTY_NOTICES.md.

use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;

use crate::{Actor, ActorScope};

/// A future that receives temporary access to actor state on every poll.
///
/// Implementations must not retain `actor` or `scope` after [`poll`](Self::poll)
/// returns. Loong actor tasks run on Tokio and only schedule owned work, so an
/// actor future that cannot cross threads or outlive its creator is not useful
/// to this runtime.
#[must_use = "actor futures do nothing unless scheduled or polled"]
pub trait ActorFuture<A: Actor>: Send + 'static {
    /// The value produced when the future completes.
    type Output;

    /// Advances the future with fresh actor and scope borrows.
    fn poll(
        self: Pin<&mut Self>,
        actor: &mut A,
        scope: &mut ActorScope<A>,
        task: &mut Context<'_>,
    ) -> Poll<Self::Output>;
}

/// Combinators for [`ActorFuture`].
#[must_use = "actor future combinators do nothing unless scheduled or polled"]
pub trait ActorFutureExt<A: Actor>: ActorFuture<A> {
    /// Maps the completed value while actor state is temporarily available.
    fn map<F, U>(self, f: F) -> Map<Self, F>
    where
        Self: Sized,
        F: FnOnce(Self::Output, &mut A, &mut ActorScope<A>) -> U + Send + 'static,
    {
        Map::new(self, f)
    }

    /// Starts another actor future after this one completes.
    fn then<F, Fut>(self, f: F) -> Then<Self, Fut, F>
    where
        Self: Sized,
        F: FnOnce(Self::Output, &mut A, &mut ActorScope<A>) -> Fut + Send + 'static,
        Fut: ActorFuture<A>,
    {
        Then::new(self, f)
    }
}

impl<A, F> ActorFutureExt<A> for F
where
    A: Actor,
    F: ActorFuture<A>,
{
}

/// Converts an ordinary future into an actor-typed, borrow-free future.
#[must_use = "the converted actor future must be scheduled or polled"]
pub trait IntoActorFuture<A: Actor>: Future + Send + Sized + 'static {
    /// Wraps this future without storing or borrowing actor state.
    fn into_actor(self) -> FutureActor<A, Self> {
        FutureActor {
            actor: PhantomData,
            future: self,
        }
    }
}

impl<A, F> IntoActorFuture<A> for F
where
    A: Actor,
    F: Future + Send + 'static,
{
}

pin_project! {
    /// An ordinary future viewed as an [`ActorFuture`].
    #[derive(Debug)]
    #[must_use = "futures do nothing unless polled"]
    pub struct FutureActor<A, F> {
        actor: PhantomData<fn() -> A>,
        #[pin]
        future: F,
    }
}

impl<A, F> ActorFuture<A> for FutureActor<A, F>
where
    A: Actor,
    F: Future + Send + 'static,
{
    type Output = F::Output;

    fn poll(
        self: Pin<&mut Self>,
        _actor: &mut A,
        _scope: &mut ActorScope<A>,
        task: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        self.project().future.poll(task)
    }
}

pin_project! {
    /// Future returned by [`ActorFutureExt::map`].
    #[derive(Debug)]
    #[must_use = "futures do nothing unless polled"]
    pub struct Map<Fut, F> {
        #[pin]
        state: MapState<Fut, F>,
    }
}

pin_project! {
    // Completion is an internal transition, not a constructible public state.
    #[project = MapStateProj]
    #[project_replace = MapStateProjReplace]
    #[derive(Debug)]
    enum MapState<Fut, F> {
        Incomplete {
            #[pin]
            future: Fut,
            f: F,
        },
        Complete,
    }
}

impl<Fut, F> Map<Fut, F> {
    fn new(future: Fut, f: F) -> Self {
        Self {
            state: MapState::Incomplete { future, f },
        }
    }
}

impl<A, Fut, F, U> ActorFuture<A> for Map<Fut, F>
where
    A: Actor,
    Fut: ActorFuture<A>,
    F: FnOnce(Fut::Output, &mut A, &mut ActorScope<A>) -> U + Send + 'static,
{
    type Output = U;

    fn poll(
        mut self: Pin<&mut Self>,
        actor: &mut A,
        scope: &mut ActorScope<A>,
        task: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        let mut this = self.as_mut().project();
        match this.state.as_mut().project() {
            MapStateProj::Incomplete { future, .. } => {
                let output = std::task::ready!(future.poll(actor, scope, task));
                match this.state.project_replace(MapState::Complete) {
                    MapStateProjReplace::Incomplete { f, .. } => {
                        Poll::Ready(f(output, actor, scope))
                    }
                    MapStateProjReplace::Complete => {
                        unreachable!("completed map was already handled")
                    }
                }
            }
            MapStateProj::Complete => panic!("Map polled after completion"),
        }
    }
}

pin_project! {
    /// Future returned by [`ActorFutureExt::then`].
    #[derive(Debug)]
    #[must_use = "futures do nothing unless polled"]
    pub struct Then<First, Second, F> {
        #[pin]
        state: ThenState<First, Second, F>,
    }
}

pin_project! {
    // As with Map, the wrapper exposes the combinator without exposing its state.
    #[project = ThenStateProj]
    #[derive(Debug)]
    enum ThenState<First, Second, F> {
        First {
            #[pin]
            future: First,
            f: Option<F>,
        },
        Second {
            #[pin]
            future: Second,
        },
        Complete,
    }
}

impl<First, Second, F> Then<First, Second, F> {
    fn new(future: First, f: F) -> Self {
        Self {
            state: ThenState::First { future, f: Some(f) },
        }
    }
}

impl<A, First, Second, F> ActorFuture<A> for Then<First, Second, F>
where
    A: Actor,
    First: ActorFuture<A>,
    Second: ActorFuture<A>,
    F: FnOnce(First::Output, &mut A, &mut ActorScope<A>) -> Second + Send + 'static,
{
    type Output = Second::Output;

    fn poll(
        mut self: Pin<&mut Self>,
        actor: &mut A,
        scope: &mut ActorScope<A>,
        task: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        loop {
            let mut this = self.as_mut().project();
            match this.state.as_mut().project() {
                ThenStateProj::First { future, f } => {
                    let output = std::task::ready!(future.poll(actor, scope, task));
                    let next = f.take().expect("the then closure is consumed exactly once")(
                        output, actor, scope,
                    );
                    this.state.set(ThenState::Second { future: next });
                }
                ThenStateProj::Second { future } => {
                    let output = std::task::ready!(future.poll(actor, scope, task));
                    this.state.set(ThenState::Complete);
                    return Poll::Ready(output);
                }
                ThenStateProj::Complete => panic!("Then polled after completion"),
            }
        }
    }
}
