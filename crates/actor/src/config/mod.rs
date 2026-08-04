mod options;

use std::num::NonZeroUsize;

#[cfg(test)]
mod tests;

pub use options::{
    ActorOptions, DEFAULT_MAILBOX_CAPACITY, DynamicMailbox, DynamicMailboxOptions, FixedMailbox,
    NoMailbox, UnboundedMailbox,
};

/// Spawn configuration selected by one actor type.
///
/// `#[actor]` generates this implementation for built-in transports.
/// Custom transports implement it with their own options carrier.
pub trait ActorConfig {
    /// Values resolved synchronously before the actor task starts.
    type Options: Default;
}

/// Configures interleaved reply admission for one actor.
///
/// Custom actor configurations implement this beside [`ActorConfig`].
pub trait InterleavingConfig: ActorConfig {
    /// Returns the active interleaved-reply limit.
    fn max_in_flight(options: &Self::Options) -> NonZeroUsize;
}
