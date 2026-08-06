//! Built-in actor-aware reply scheduling profiles.
//!
//! [`#[actor(...)]`](macro@crate::actor) selects one profile:
//!
//! - omitting `mailbox` selects [`Disabled`];
//! - `mailbox` without `interleaved` selects [`Serial`];
//! - fixed `interleaved` forms select [`Fixed`];
//! - dynamic `interleaved` forms select [`Dynamic`];
//! - unbounded `interleaved` selects [`Unbounded`].
//!
//! The macro reference documents syntax and defaults.
//! Manual [`MessageConfig`] implementations select a profile directly.
//!
//! A finite limit bounds active interleaved replies.
//! At the limit, dispatch pauses before the next handler.
//! The reply mode becomes known only after handler dispatch.
//! Every queued handler must therefore pass the same gate.
//!
//! Ready replies finish during dispatch.
//! Owned replies run in separate Tokio tasks.
//! Exclusive replies run one at a time in every profile.

mod queue;
mod runtime;

use std::{
    num::NonZeroUsize,
    panic::{self, AssertUnwindSafe},
    pin::Pin,
    task::{Context, Poll},
};

use crate::{
    Actor, ActorFuture, ActorScope,
    mailbox::{Control, Mode},
    transport::{MessageConfig, MessageInbox, MessageSender, NoInbox, NoSender},
};

pub(crate) use runtime::{ActorScheduler, RuntimeScheduler, SchedulerTurn, Seal, TurnContext};

use queue::Queue;

pub(crate) type ErasedActorFuture<A> = Pin<Box<dyn ActorFuture<A, Output = ()> + Send + 'static>>;

// Automatic frame destruction cannot borrow the actor's lifecycle control.
// Isolate each value so one Drop panic cannot skip sibling cleanup.
fn drop_without_unwind<T>(value: T) {
    if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(|| drop(value))) {
        Control::discard_panic(payload);
    }
}

/// A sealed runtime scheduling profile for one actor.
///
/// Custom configurations select a built-in profile.
/// They do not implement this trait directly.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot schedule replies for `{A}`",
    label = "select a scheduler compatible with this actor's capabilities"
)]
#[allow(
    private_bounds,
    private_interfaces,
    reason = "a private runtime bridge seals reply profiles"
)]
pub trait SchedulerProfile<A: Actor>: Send + 'static + Sized {
    /// Private strategy selected by this profile.
    #[doc(hidden)]
    type Strategy: runtime::SchedulerStrategy<A, Self>;
}

/// A sealed profile supporting actor-aware replies.
///
/// [`Disabled`] intentionally lacks this capability.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot schedule actor-aware replies for `{A}`",
    label = "select a scheduler compatible with this actor's mailbox"
)]
#[allow(
    private_bounds,
    private_interfaces,
    reason = "a private runtime bridge reserves reply scheduling"
)]
pub trait ReplyScheduler<A: Actor>: SchedulerProfile<A> {
    /// Pushes exclusive work through the sealed runtime bridge.
    #[doc(hidden)]
    fn __push_exclusive<F>(&mut self, _: runtime::Seal, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static;
}

/// A sealed profile supporting interleaved replies.
///
/// [`Serial`] intentionally does not implement this capability.
#[diagnostic::on_unimplemented(
    message = "`{A}` cannot schedule interleaved replies",
    label = "enable `interleaved` in this actor's `#[actor(...)]` configuration"
)]
#[allow(
    private_bounds,
    private_interfaces,
    reason = "a private runtime bridge reserves interleaved scheduling"
)]
pub trait InterleavedScheduler<A: Actor>: ReplyScheduler<A> {
    /// Pushes interleaved work through the sealed runtime bridge.
    #[doc(hidden)]
    fn __push_interleaved<F>(&mut self, _: runtime::Seal, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static;
}

/// The scheduler for actors without a mailbox.
///
/// This profile is zero-sized and schedules nothing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Disabled;

impl Disabled {
    /// Creates a disabled profile.
    pub const fn new() -> Self {
        Self
    }
}

/// The scheduler for mailbox actors without interleaving.
///
/// Ready replies require no scheduler storage.
/// Owned replies run in separate Tokio tasks.
/// Exclusive replies run one at a time.
pub struct Serial<A: Actor> {
    exclusive: Exclusive<A>,
    cursor: SerialLane,
}

impl<A: Actor> Serial<A> {
    /// Creates an empty serial profile.
    pub fn new() -> Self {
        Self {
            exclusive: Exclusive::new(),
            cursor: SerialLane::Mailbox,
        }
    }
}

impl<A: Actor> Default for Serial<A> {
    fn default() -> Self {
        Self::new()
    }
}

/// Schedules at most `N` active interleaved replies.
pub struct Fixed<A: Actor, const N: usize> {
    state: InterleavedState<A, FixedLimit<N>>,
}

impl<A: Actor, const N: usize> Fixed<A, N> {
    /// Creates an empty fixed profile.
    ///
    /// Compilation fails when `N` is zero.
    pub fn new() -> Self {
        const { assert!(N > 0, "interleaved limit must be greater than zero") };
        Self {
            state: InterleavedState::with_limit(FixedLimit),
        }
    }
}

impl<A: Actor, const N: usize> Default for Fixed<A, N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Schedules interleaved replies with a per-spawn limit.
pub struct Dynamic<A: Actor> {
    state: InterleavedState<A, DynamicLimit>,
}

impl<A: Actor> Dynamic<A> {
    /// Creates an empty profile with one resolved limit.
    pub fn new(limit: NonZeroUsize) -> Self {
        Self {
            state: InterleavedState::with_limit(DynamicLimit(limit)),
        }
    }
}

/// Schedules interleaved replies without a finite limit.
pub struct Unbounded<A: Actor> {
    state: InterleavedState<A, UnboundedLimit>,
}

impl<A: Actor> Unbounded<A> {
    /// Creates an empty unbounded profile.
    pub fn new() -> Self {
        Self {
            state: InterleavedState::with_limit(UnboundedLimit),
        }
    }
}

impl<A: Actor> Default for Unbounded<A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A> SchedulerProfile<A> for Disabled
where
    A: Actor + MessageConfig<Sender = NoSender, Inbox = NoInbox, Scheduler = Self>,
{
    type Strategy = runtime::DisabledStrategy;
}

impl<A> SchedulerProfile<A> for Serial<A>
where
    A: Actor + MessageConfig<Scheduler = Self>,
    A::Sender: MessageSender<A>,
    A::Inbox: MessageInbox<A>,
{
    type Strategy = runtime::SerialStrategy;
}

impl<A, const N: usize> SchedulerProfile<A> for Fixed<A, N>
where
    A: Actor + MessageConfig<Scheduler = Self>,
    A::Sender: MessageSender<A>,
    A::Inbox: MessageInbox<A>,
{
    type Strategy = runtime::InterleavedStrategy;
}

impl<A> SchedulerProfile<A> for Dynamic<A>
where
    A: Actor + MessageConfig<Scheduler = Self>,
    A::Sender: MessageSender<A>,
    A::Inbox: MessageInbox<A>,
{
    type Strategy = runtime::InterleavedStrategy;
}

impl<A> SchedulerProfile<A> for Unbounded<A>
where
    A: Actor + MessageConfig<Scheduler = Self>,
    A::Sender: MessageSender<A>,
    A::Inbox: MessageInbox<A>,
{
    type Strategy = runtime::InterleavedStrategy;
}

impl<A> ReplyScheduler<A> for Serial<A>
where
    A: Actor + MessageConfig<Scheduler = Self>,
    A::Sender: MessageSender<A>,
    A::Inbox: MessageInbox<A>,
{
    fn __push_exclusive<F>(&mut self, _: runtime::Seal, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static,
    {
        self.exclusive.push(future);
    }
}

impl<A, const N: usize> ReplyScheduler<A> for Fixed<A, N>
where
    A: Actor + MessageConfig<Scheduler = Self>,
    A::Sender: MessageSender<A>,
    A::Inbox: MessageInbox<A>,
{
    fn __push_exclusive<F>(&mut self, _: runtime::Seal, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static,
    {
        self.state.exclusive.push(future);
    }
}

impl<A> ReplyScheduler<A> for Dynamic<A>
where
    A: Actor + MessageConfig<Scheduler = Self>,
    A::Sender: MessageSender<A>,
    A::Inbox: MessageInbox<A>,
{
    fn __push_exclusive<F>(&mut self, _: runtime::Seal, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static,
    {
        self.state.exclusive.push(future);
    }
}

impl<A> ReplyScheduler<A> for Unbounded<A>
where
    A: Actor + MessageConfig<Scheduler = Self>,
    A::Sender: MessageSender<A>,
    A::Inbox: MessageInbox<A>,
{
    fn __push_exclusive<F>(&mut self, _: runtime::Seal, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static,
    {
        self.state.exclusive.push(future);
    }
}

impl<A, const N: usize> InterleavedScheduler<A> for Fixed<A, N>
where
    A: Actor + MessageConfig<Scheduler = Self>,
    A::Sender: MessageSender<A>,
    A::Inbox: MessageInbox<A>,
{
    fn __push_interleaved<F>(&mut self, _: runtime::Seal, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static,
    {
        self.state.push_interleaved(Box::pin(future));
    }
}

impl<A> InterleavedScheduler<A> for Dynamic<A>
where
    A: Actor + MessageConfig<Scheduler = Self>,
    A::Sender: MessageSender<A>,
    A::Inbox: MessageInbox<A>,
{
    fn __push_interleaved<F>(&mut self, _: runtime::Seal, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static,
    {
        self.state.push_interleaved(Box::pin(future));
    }
}

impl<A> InterleavedScheduler<A> for Unbounded<A>
where
    A: Actor + MessageConfig<Scheduler = Self>,
    A::Sender: MessageSender<A>,
    A::Inbox: MessageInbox<A>,
{
    fn __push_interleaved<F>(&mut self, _: runtime::Seal, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static,
    {
        self.state.push_interleaved(Box::pin(future));
    }
}

pub(crate) struct InterleavedState<A: Actor, L> {
    queue: Queue<A>,
    exclusive: Exclusive<A>,
    limit: L,
    pub(crate) cursor: InterleavedLane,
}

pub(crate) trait InterleavedProfile<A: Actor>: Send + 'static {
    type Limit: LimitPolicy;

    fn state(&mut self) -> &mut InterleavedState<A, Self::Limit>;
}

impl<A: Actor, const N: usize> InterleavedProfile<A> for Fixed<A, N> {
    type Limit = FixedLimit<N>;

    fn state(&mut self) -> &mut InterleavedState<A, Self::Limit> {
        &mut self.state
    }
}

impl<A: Actor> InterleavedProfile<A> for Dynamic<A> {
    type Limit = DynamicLimit;

    fn state(&mut self) -> &mut InterleavedState<A, Self::Limit> {
        &mut self.state
    }
}

impl<A: Actor> InterleavedProfile<A> for Unbounded<A> {
    type Limit = UnboundedLimit;

    fn state(&mut self) -> &mut InterleavedState<A, Self::Limit> {
        &mut self.state
    }
}

pub(crate) struct FixedLimit<const N: usize>;

pub(crate) struct DynamicLimit(NonZeroUsize);

pub(crate) struct UnboundedLimit;

pub(crate) trait LimitPolicy: Send + 'static {
    fn has_capacity(&self, active: usize) -> bool;
}

impl<const N: usize> LimitPolicy for FixedLimit<N> {
    fn has_capacity(&self, active: usize) -> bool {
        active < N
    }
}

impl LimitPolicy for DynamicLimit {
    fn has_capacity(&self, active: usize) -> bool {
        active < self.0.get()
    }
}

impl LimitPolicy for UnboundedLimit {
    fn has_capacity(&self, _active: usize) -> bool {
        true
    }
}

impl<A: Actor, L: LimitPolicy> InterleavedState<A, L> {
    fn with_limit(limit: L) -> Self {
        Self {
            queue: Queue::new(),
            exclusive: Exclusive::new(),
            limit,
            cursor: InterleavedLane::Mailbox,
        }
    }

    pub(crate) fn has_dispatch_capacity(&self) -> bool {
        self.exclusive.is_empty() && self.limit.has_capacity(self.queue.len())
    }

    pub(crate) fn has_interleaved(&self) -> bool {
        !self.queue.is_empty()
    }

    fn push_interleaved(&mut self, future: ErasedActorFuture<A>) {
        debug_assert!(self.has_dispatch_capacity());
        self.queue.push(future);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SerialLane {
    Mailbox,
    ChildExit,
}

impl SerialLane {
    const fn next(self) -> Self {
        match self {
            Self::Mailbox => Self::ChildExit,
            Self::ChildExit => Self::Mailbox,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InterleavedLane {
    Mailbox,
    Interleaved,
    ChildExit,
}

impl InterleavedLane {
    const fn next(self) -> Self {
        match self {
            Self::Mailbox => Self::Interleaved,
            Self::Interleaved => Self::ChildExit,
            Self::ChildExit => Self::Mailbox,
        }
    }
}

struct Exclusive<A: Actor> {
    future: Option<ErasedActorFuture<A>>,
}

impl<A: Actor> Exclusive<A> {
    fn new() -> Self {
        Self { future: None }
    }

    fn is_empty(&self) -> bool {
        self.future.is_none()
    }

    fn push<F>(&mut self, future: F)
    where
        F: ActorFuture<A, Output = ()> + Send + 'static,
    {
        debug_assert!(self.future.is_none(), "exclusive work cannot overlap");
        self.future = Some(Box::pin(future));
    }

    fn poll(
        &mut self,
        actor: &mut A,
        scope: &mut ActorScope<'_, A>,
        control: &Control,
        expected_mode: Mode,
        task: &mut Context<'_>,
    ) -> Poll<()> {
        if control.mode() != expected_mode {
            return Poll::Ready(());
        }
        let Some(future) = &mut self.future else {
            return Poll::Pending;
        };
        let result = future.as_mut().poll(actor, scope, task);
        if result.is_ready() {
            let completed = self
                .future
                .take()
                .expect("the completed exclusive future remains owned");
            control.drop_user_value(completed);
        }
        if control.mode() != expected_mode {
            return Poll::Ready(());
        }
        result
    }

    fn clear(&mut self, control: &Control) {
        if let Some(future) = self.future.take() {
            control.drop_user_value(future);
        }
    }
}

impl<A: Actor> Drop for Exclusive<A> {
    fn drop(&mut self) {
        if let Some(future) = self.future.take() {
            drop_without_unwind(future);
        }
    }
}
