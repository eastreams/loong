use std::{fmt, marker::PhantomData, num::NonZeroUsize};

/// Default capacity used by built-in mailbox configurations.
#[doc(hidden)]
pub const DEFAULT_MAILBOX_CAPACITY: usize = 32;

/// Default limit used by built-in interleaving configurations.
#[doc(hidden)]
pub const DEFAULT_MAX_IN_FLIGHT: usize = 32;

/// Default limit used by built-in child configurations.
#[doc(hidden)]
pub const DEFAULT_MAX_CHILDREN: usize = 32;

/// Built-in spawn options for one actor.
///
/// `M` retains only mailbox values which may vary per spawn.
/// `I` retains only interleaving values which may vary per spawn.
/// `C` retains only child limits which may vary per spawn.
/// The actor marker prevents options from crossing actor types.
pub struct ActorOptions<A, M, I, C> {
    mailbox: M,
    interleaving: I,
    children: C,
    actor: PhantomData<fn() -> A>,
}

impl<A, M, C, const DEFAULT: usize> ActorOptions<A, M, DynamicInterleaving<DEFAULT>, C> {
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

impl<A, I, C, const DEFAULT: usize> ActorOptions<A, DynamicMailbox<DEFAULT>, I, C> {
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

impl<A, M, I, const DEFAULT: usize> ActorOptions<A, M, I, DynamicChildren<DEFAULT>> {
    /// Overrides this actor's direct-child limit.
    pub const fn with_max_children(mut self, max_children: NonZeroUsize) -> Self {
        self.children.max_children = Some(max_children);
        self
    }

    /// Resolves this spawn's direct-child limit.
    #[doc(hidden)]
    pub const fn max_children(&self) -> NonZeroUsize {
        match self.children.max_children {
            Some(max_children) => max_children,
            None => DynamicChildren::<DEFAULT>::DEFAULT_MAX_CHILDREN,
        }
    }
}

impl<A, M: Clone, I: Clone, C: Clone> Clone for ActorOptions<A, M, I, C> {
    fn clone(&self) -> Self {
        Self {
            mailbox: self.mailbox.clone(),
            interleaving: self.interleaving.clone(),
            children: self.children.clone(),
            actor: PhantomData,
        }
    }
}

impl<A, M: Copy, I: Copy, C: Copy> Copy for ActorOptions<A, M, I, C> {}

impl<A, M: fmt::Debug, I: fmt::Debug, C: fmt::Debug> fmt::Debug for ActorOptions<A, M, I, C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActorOptions")
            .field("mailbox", &self.mailbox)
            .field("interleaving", &self.interleaving)
            .field("children", &self.children)
            .finish()
    }
}

impl<A, M: PartialEq, I: PartialEq, C: PartialEq> PartialEq for ActorOptions<A, M, I, C> {
    fn eq(&self, other: &Self) -> bool {
        self.mailbox == other.mailbox
            && self.interleaving == other.interleaving
            && self.children == other.children
    }
}

impl<A, M: Eq, I: Eq, C: Eq> Eq for ActorOptions<A, M, I, C> {}

impl<A, M: Default, I: Default, C: Default> Default for ActorOptions<A, M, I, C> {
    fn default() -> Self {
        Self {
            mailbox: M::default(),
            interleaving: I::default(),
            children: C::default(),
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

/// Spawn state for an actor without direct children.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoChildren;

/// Spawn state for an actor with a fixed child limit.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FixedChildren;

/// Spawn state for an actor with unbounded direct children.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UnboundedChildren;

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

/// Spawn state for an actor with a dynamic child limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DynamicChildren<const DEFAULT: usize = DEFAULT_MAX_CHILDREN> {
    max_children: Option<NonZeroUsize>,
}

impl<const DEFAULT: usize> DynamicChildren<DEFAULT> {
    const DEFAULT_MAX_CHILDREN: NonZeroUsize =
        NonZeroUsize::new(DEFAULT).expect("child limit must be greater than zero");
}

impl<const DEFAULT: usize> Default for DynamicChildren<DEFAULT> {
    fn default() -> Self {
        Self { max_children: None }
    }
}

/// Options whose mailbox capacity may change per spawn.
pub trait DynamicMailboxOptions: Sized {
    /// Overrides the mailbox capacity for one spawn.
    fn with_mailbox_capacity(self, capacity: NonZeroUsize) -> Self;
}

impl<A, I, C, const DEFAULT: usize> DynamicMailboxOptions
    for ActorOptions<A, DynamicMailbox<DEFAULT>, I, C>
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

impl<A, M, C, const DEFAULT: usize> DynamicInterleavingOptions
    for ActorOptions<A, M, DynamicInterleaving<DEFAULT>, C>
{
    fn with_max_in_flight(self, max_in_flight: NonZeroUsize) -> Self {
        ActorOptions::with_max_in_flight(self, max_in_flight)
    }
}

/// Options whose direct-child limit may change per spawn.
pub trait DynamicChildrenOptions: Sized {
    /// Overrides the direct-child limit for one spawn.
    fn with_max_children(self, max_children: NonZeroUsize) -> Self;
}

impl<A, M, I, const DEFAULT: usize> DynamicChildrenOptions
    for ActorOptions<A, M, I, DynamicChildren<DEFAULT>>
{
    fn with_max_children(self, max_children: NonZeroUsize) -> Self {
        ActorOptions::with_max_children(self, max_children)
    }
}
