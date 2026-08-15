use std::num::NonZeroUsize;

use crate::{
    Actor, ActorFuture,
    transport::{MessageConfig, MessageInbox, MessageSender, NoInbox, NoSender},
};

use super::runtime;
use super::{DynamicLimit, Exclusive, FixedLimit, InterleavedState, SerialLane, UnboundedLimit};

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
/// Exclusive replies run one at a time.
pub struct Serial<A: Actor> {
    pub(super) exclusive: Exclusive<A>,
    pub(super) cursor: SerialLane,
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
    pub(super) state: InterleavedState<A, FixedLimit<N>>,
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
    pub(super) state: InterleavedState<A, DynamicLimit>,
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
    pub(super) state: InterleavedState<A, UnboundedLimit>,
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
