use std::{fmt, marker::PhantomData, num::NonZeroUsize};

/// Default capacity used by built-in mailbox configurations.
#[doc(hidden)]
pub const DEFAULT_MAILBOX_CAPACITY: usize = 32;

const DEFAULT_MAX_IN_FLIGHT: usize = 32;

/// Built-in spawn options for one actor.
///
/// `M` retains only mailbox values which may vary per spawn.
/// The actor marker prevents options from crossing actor types.
pub struct ActorOptions<A, M> {
    mailbox: M,
    max_in_flight: NonZeroUsize,
    actor: PhantomData<fn() -> A>,
}

impl<A, M> ActorOptions<A, M> {
    /// Sets the maximum number of active interleaved replies.
    pub const fn with_max_in_flight(mut self, max_in_flight: NonZeroUsize) -> Self {
        self.max_in_flight = max_in_flight;
        self
    }

    /// Returns the maximum number of active interleaved replies.
    pub const fn max_in_flight(&self) -> NonZeroUsize {
        self.max_in_flight
    }
}

impl<A, const DEFAULT: usize> ActorOptions<A, DynamicMailbox<DEFAULT>> {
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

impl<A, M: Clone> Clone for ActorOptions<A, M> {
    fn clone(&self) -> Self {
        Self {
            mailbox: self.mailbox.clone(),
            max_in_flight: self.max_in_flight,
            actor: PhantomData,
        }
    }
}

impl<A, M: Copy> Copy for ActorOptions<A, M> {}

impl<A, M: fmt::Debug> fmt::Debug for ActorOptions<A, M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActorOptions")
            .field("mailbox", &self.mailbox)
            .field("max_in_flight", &self.max_in_flight)
            .finish()
    }
}

impl<A, M: PartialEq> PartialEq for ActorOptions<A, M> {
    fn eq(&self, other: &Self) -> bool {
        self.mailbox == other.mailbox && self.max_in_flight == other.max_in_flight
    }
}

impl<A, M: Eq> Eq for ActorOptions<A, M> {}

impl<A, M: Default> Default for ActorOptions<A, M> {
    fn default() -> Self {
        Self {
            mailbox: M::default(),
            max_in_flight: NonZeroUsize::new(DEFAULT_MAX_IN_FLIGHT)
                .expect("the default in-flight limit is nonzero"),
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

/// Options whose mailbox capacity may change per spawn.
pub trait DynamicMailboxOptions: Sized {
    /// Overrides the mailbox capacity for one spawn.
    fn with_mailbox_capacity(self, capacity: NonZeroUsize) -> Self;
}

impl<A, const DEFAULT: usize> DynamicMailboxOptions for ActorOptions<A, DynamicMailbox<DEFAULT>> {
    fn with_mailbox_capacity(self, capacity: NonZeroUsize) -> Self {
        ActorOptions::with_mailbox_capacity(self, capacity)
    }
}
