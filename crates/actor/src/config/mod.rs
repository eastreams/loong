mod options;

#[cfg(test)]
mod tests;

pub use options::{
    ActorOptions, DEFAULT_MAILBOX_CAPACITY, DEFAULT_MAX_IN_FLIGHT, DynamicInterleaving,
    DynamicInterleavingOptions, DynamicMailbox, DynamicMailboxOptions, FixedInterleaving,
    FixedMailbox, NoInterleaving, NoMailbox, UnboundedInterleaving, UnboundedMailbox,
};

/// Spawn configuration selected by one actor type.
///
/// `#[actor]` generates this implementation for built-in transports.
/// Custom transports implement it with their own options carrier.
pub trait ActorConfig {
    /// Values resolved synchronously before the actor task starts.
    type Options: Default;
}

/// Configures actor-aware reply scheduling for one actor.
///
/// Custom actor configurations implement this beside [`ActorConfig`].
/// Choose one built-in [`scheduling`] profile.
/// This trait schedules actor-aware replies only.
/// Owned replies run in separate Tokio tasks.
///
/// [`scheduling`]: crate::scheduling
pub trait ReplySchedulingConfig: ActorConfig {
    /// The scheduler selected for this actor.
    type Scheduler: Send + 'static;

    /// Opens this actor's reply scheduler.
    fn open_scheduler(options: &Self::Options) -> Self::Scheduler;
}
