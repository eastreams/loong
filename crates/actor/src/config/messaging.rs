use std::{marker::PhantomData, num::NonZeroUsize};

use super::{InterleavingConfig, MessagingConfig, NoInterleaving, sealed};

const DEFAULT_CAPACITY: usize = 32;

/// An actor without public messaging.
#[doc(hidden)]
pub struct NoMessaging;

/// A mailbox with one fixed capacity.
#[doc(hidden)]
pub struct Mailbox<I, const N: usize = DEFAULT_CAPACITY>(PhantomData<fn() -> I>);

/// A mailbox configured during actor creation.
#[doc(hidden)]
pub struct DynamicMailbox<I, const DEFAULT: usize = DEFAULT_CAPACITY>(PhantomData<fn() -> I>);

/// A mailbox without a finite capacity.
#[doc(hidden)]
pub struct UnboundedMailbox<I>(PhantomData<fn() -> I>);

impl sealed::Messaging for NoMessaging {
    type Options = ();
    type Interleaving = NoInterleaving;
}

impl MessagingConfig for NoMessaging {}

impl<I: InterleavingConfig, const N: usize> sealed::Messaging for Mailbox<I, N> {
    type Options = ();
    type Interleaving = I;
}

impl<I: InterleavingConfig, const N: usize> MessagingConfig for Mailbox<I, N> {}
impl<I: InterleavingConfig, const N: usize> sealed::MailboxPolicy for Mailbox<I, N> {}

impl<I: InterleavingConfig, const DEFAULT: usize> sealed::Messaging for DynamicMailbox<I, DEFAULT> {
    type Options = Option<NonZeroUsize>;
    type Interleaving = I;
}

impl<I: InterleavingConfig, const DEFAULT: usize> MessagingConfig for DynamicMailbox<I, DEFAULT> {}
impl<I: InterleavingConfig, const DEFAULT: usize> sealed::MailboxPolicy
    for DynamicMailbox<I, DEFAULT>
{
}

impl<I: InterleavingConfig> sealed::Messaging for UnboundedMailbox<I> {
    type Options = ();
    type Interleaving = I;
}

impl<I: InterleavingConfig> MessagingConfig for UnboundedMailbox<I> {}
impl<I: InterleavingConfig> sealed::MailboxPolicy for UnboundedMailbox<I> {}
