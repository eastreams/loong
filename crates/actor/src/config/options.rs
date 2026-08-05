use std::{fmt, marker::PhantomData, num::NonZeroUsize};

/// Default capacity used by built-in mailbox configurations.
#[doc(hidden)]
pub const DEFAULT_MAILBOX_CAPACITY: usize = 32;

/// Default limit used by built-in interleaving configurations.
#[doc(hidden)]
pub const DEFAULT_MAX_IN_FLIGHT: usize = 32;

/// Built-in spawn options for one actor.
///
/// `M` retains only mailbox values which may vary per spawn.
/// `I` retains only interleaving values which may vary per spawn.
/// The actor marker prevents options from crossing actor types.
pub struct ActorOptions<A, M, I> {
    mailbox: M,
    interleaving: I,
    actor: PhantomData<fn() -> A>,
}

impl<A, M, const DEFAULT: usize> ActorOptions<A, M, DynamicInterleaving<DEFAULT>> {
    /// Sets the maximum number of active interleaved replies.
    pub const fn with_max_in_flight(mut self, max_in_flight: NonZeroUsize) -> Self {
        self.interleaving.max_in_flight = Some(max_in_flight);
        self
    }

    /// Resolves this spawn's dynamic interleaved-reply limit.
    #[doc(hidden)]
    pub const fn max_in_flight(&self) -> NonZeroUsize {
        match self.interleaving.max_in_flight {
            Some(max_in_flight) => max_in_flight,
            None => DynamicInterleaving::<DEFAULT>::DEFAULT_MAX_IN_FLIGHT,
        }
    }
}

impl<A, I, const DEFAULT: usize> ActorOptions<A, DynamicMailbox<DEFAULT>, I> {
    /// Overrides this actor's dynamic mailbox capacity.
    pub const fn with_mailbox_capacity(mut self, capacity: NonZeroUsize) -> Self {
        self.mailbox.capacity = Some(capacity);
        self
    }

    /// Resolves this spawn's dynamic mailbox capacity.
    #[doc(hidden)]
    pub const fn mailbox_capacity(&self) -> NonZeroUsize {
        match self.mailbox.capacity {
            Some(capacity) => capacity,
            None => DynamicMailbox::<DEFAULT>::DEFAULT_CAPACITY,
        }
    }
}

impl<A, M: Clone, I: Clone> Clone for ActorOptions<A, M, I> {
    fn clone(&self) -> Self {
        Self {
            mailbox: self.mailbox.clone(),
            interleaving: self.interleaving.clone(),
            actor: PhantomData,
        }
    }
}

impl<A, M: Copy, I: Copy> Copy for ActorOptions<A, M, I> {}

impl<A, M: fmt::Debug, I: fmt::Debug> fmt::Debug for ActorOptions<A, M, I> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActorOptions")
            .field("mailbox", &self.mailbox)
            .field("interleaving", &self.interleaving)
            .finish()
    }
}

impl<A, M: PartialEq, I: PartialEq> PartialEq for ActorOptions<A, M, I> {
    fn eq(&self, other: &Self) -> bool {
        self.mailbox == other.mailbox && self.interleaving == other.interleaving
    }
}

impl<A, M: Eq, I: Eq> Eq for ActorOptions<A, M, I> {}

impl<A, M: Default, I: Default> Default for ActorOptions<A, M, I> {
    fn default() -> Self {
        Self {
            mailbox: M::default(),
            interleaving: I::default(),
            actor: PhantomData,
        }
    }
}

/// Spawn state for an actor without a mailbox.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoMailbox;

/// Spawn state for an actor with a fixed mailbox.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FixedMailbox;

/// Spawn state for an actor with an unbounded mailbox.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UnboundedMailbox;

/// Spawn state for an actor without interleaved replies.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoInterleaving;

/// Spawn state for an actor with a fixed interleaved-reply limit.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FixedInterleaving;

/// Spawn state for an actor with unbounded interleaved replies.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UnboundedInterleaving;

/// Spawn state for an actor with a dynamic mailbox.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DynamicMailbox<const DEFAULT: usize = DEFAULT_MAILBOX_CAPACITY> {
    capacity: Option<NonZeroUsize>,
}

impl<const DEFAULT: usize> DynamicMailbox<DEFAULT> {
    const DEFAULT_CAPACITY: NonZeroUsize =
        NonZeroUsize::new(DEFAULT).expect("mailbox capacity must be greater than zero");
}

impl<const DEFAULT: usize> Default for DynamicMailbox<DEFAULT> {
    fn default() -> Self {
        Self { capacity: None }
    }
}

/// Spawn state for an actor with a dynamic interleaved-reply limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DynamicInterleaving<const DEFAULT: usize = DEFAULT_MAX_IN_FLIGHT> {
    max_in_flight: Option<NonZeroUsize>,
}

impl<const DEFAULT: usize> DynamicInterleaving<DEFAULT> {
    const DEFAULT_MAX_IN_FLIGHT: NonZeroUsize =
        NonZeroUsize::new(DEFAULT).expect("interleaved reply limit must be greater than zero");
}

impl<const DEFAULT: usize> Default for DynamicInterleaving<DEFAULT> {
    fn default() -> Self {
        Self {
            max_in_flight: None,
        }
    }
}

/// Options whose mailbox capacity may change per spawn.
pub trait DynamicMailboxOptions: Sized {
    /// Overrides the mailbox capacity for one spawn.
    fn with_mailbox_capacity(self, capacity: NonZeroUsize) -> Self;
}

impl<A, I, const DEFAULT: usize> DynamicMailboxOptions
    for ActorOptions<A, DynamicMailbox<DEFAULT>, I>
{
    fn with_mailbox_capacity(self, capacity: NonZeroUsize) -> Self {
        ActorOptions::with_mailbox_capacity(self, capacity)
    }
}

/// Options whose interleaved-reply limit may change per spawn.
pub trait DynamicInterleavingOptions: Sized {
    /// Overrides the active interleaved-reply limit for one spawn.
    fn with_max_in_flight(self, max_in_flight: NonZeroUsize) -> Self;
}

impl<A, M, const DEFAULT: usize> DynamicInterleavingOptions
    for ActorOptions<A, M, DynamicInterleaving<DEFAULT>>
{
    fn with_max_in_flight(self, max_in_flight: NonZeroUsize) -> Self {
        ActorOptions::with_max_in_flight(self, max_in_flight)
    }
}
