#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

//! Actors own mutable state and run serial lifecycle work.
//! An actor with [`HasMailbox`] also processes typed messages.
//!
//! These actors are local to one process.
//! Their tasks remain `Send`.
//! `#[actor(mailbox)]` adds a public mailbox.
//! It stores accepted messages awaiting dispatch.
//! Dispatch starts the matching [`Handler`] implementation.
//!
//! # Define an actor
//!
//! Implement [`Actor`] for state owned by one actor.
//! Most implementations use `#[actor(...)]`.
//! Its options declare the actor's runtime capabilities.
//! Omitting `interleaved` removes that capability and its queue.
//! Custom transports implement [`ActorConfig`], [`MessageConfig`], and
//! [`ReplySchedulingConfig`] directly.
//! [`Actor::SpawnArgs`] owns its construction inputs.
//! [`Actor::init`] asynchronously builds the complete state.
//! Derive [`Message`] for every message type.
//! [`Message::Reply`] defines its successful call result.
//! The derive defaults that type to `()`.
//! Use `#[message(reply = Type)]` to select another type.
//! Implement [`SyncHandler<M>`](SyncHandler) for an immediate reply.
//! Implement [`Handler<M>`](Handler) for other reply strategies.
//!
//! # Start an actor
//!
//! Call [`spawn`] inside a Tokio runtime.
//! It returns the root actor's unique [`ActorOwner`].
//! It returns before initialization completes.
//! The actor task then awaits [`Actor::init`].
//! For a message actor, admission opens during initialization.
//! Its handler dispatch starts only after initialization succeeds.
//! Obtain an [`ActorRef`] through [`ActorOwner::actor_ref`].
//! References to actors with [`HasMailbox`] send typed messages.
//! An actor reference does not own lifecycle.
//!
//! # Send messages
//!
//! Use [`ActorRef::call`] when the sender needs a reply.
//! When full, `call` waits for mailbox capacity.
//! Once accepted, it waits for the request's reply.
//! Cancelling before acceptance discards the message.
//! Dropping a queued call may skip its handler.
//! After dispatch, dropping it only abandons the result.
//! [`CallError`] distinguishes rejection from known dispatch interruptions.
//!
//! Use [`ActorRef::send`] for one-way messages.
//! When full, `send` also waits for mailbox capacity.
//! A one-way send only reports mailbox acceptance.
//! It does not report handler completion.
//! One-way messages use `()` as [`Message::Reply`].
//! After acceptance, the sender cannot withdraw the message.
//! Stop or Kill may still discard it before dispatch.
//! A [`SendError`] retains an unaccepted message.
//!
//! [`ActorRef::try_call`] and [`ActorRef::try_send`] never wait for capacity.
//! Their rejection returns the original message.
//! Successful `try_call` returns a [`Response`] future.
//!
//! ```
//! use loong_actor::{ExitReason, Shutdown, SubtreeStatus, prelude::*, spawn};
//!
//! struct Counter(u64);
//!
//! #[actor(mailbox)]
//! impl Actor for Counter {
//!     type SpawnArgs = u64;
//!
//!     async fn init(
//!         initial: u64,
//!         _scope: &mut ActorScope<'_, Self>,
//!     ) -> Self {
//!         Self(initial)
//!     }
//! }
//!
//! #[derive(Message)]
//! #[message(reply = u64)]
//! struct Add(u64);
//!
//! impl SyncHandler<Add> for Counter {
//!     fn handle(
//!         &mut self,
//!         message: Add,
//!         _scope: &mut ActorScope<'_, Self>,
//!     ) -> u64 {
//!         self.0 += message.0;
//!         self.0
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     let owner = spawn::<Counter>(0);
//!     let counter = owner.actor_ref();
//!
//!     assert_eq!(counter.call(Add(2)).await, Ok(2));
//!     let status = owner.shutdown(Shutdown::Drain).await;
//!     assert_eq!(status.reason(), ExitReason::Drained);
//!     assert_eq!(status.subtree(), SubtreeStatus::Terminated);
//! }
//! ```
//!
//! # Replies and actor progress
//!
//! A reply strategy controls actor progress after dispatch.
//! [`SyncHandler`] automatically selects ready scheduling.
//! Inside [`Handler`], use [`ReplyExt::ready`] for a produced value.
//! Returning a bare [`Future`] selects owned scheduling automatically.
//! Owned futures receive no actor or scope access.
//! They can progress alongside other actor work.
//! [`ActorFuture`] values receive temporary actor access during each poll.
//! [`InterleavedFutureExt::interleaved`] allows other work between polls.
//! It requires the actor's [`HasInterleaving`] capability.
//! Fixed and dynamic configurations bound active replies.
//! A full finite limit pauses dispatch before another handler starts.
//! The handler's reply mode remains unknown until dispatch finishes.
//! Dynamic options expose `SpawnOptions::with_max_in_flight`.
//! Unbounded configurations may retain arbitrarily many active replies.
//! [`ReplyExt::exclusive`] pauses mailbox dispatch and interleaved replies.
//! It does not require [`HasInterleaving`].
//! It also pauses child actor exit hooks.
//! Already-dispatched owned replies still progress.
//! Kill can still cancel an exclusive reply between polls.
//! Use [`reply::Either`] when runtime branches need different strategies.
//! The [`reply`] module documents cancellation and shutdown behavior.
//!
//! # Ownership and shutdown
//!
//! Actor handles and lifecycle ownership are separate.
//! Each root actor has one [`ActorOwner`].
//! [`ActorScope::spawn_child`] registers direct child actors.
//! The parent runtime retains their lifecycle ownership.
//! After child cleanup, [`Actor::on_stop`] receives [`StopScope`].
//! This parent-child ownership forms the supervision tree.
//! Supervision links a child actor's lifecycle to its parent.
//! A [`Child`] is a non-owning child actor handle.
//! Cloned [`ActorRef`] values never keep actors alive.
//! Any [`ActorRef`] may request shutdown.
//! Only [`ActorOwner`] requests Kill when dropped.
//! Parent shutdown requests shutdown from every owned child actor.
//! [`ExitReason`] describes only one actor.
//! [`ExitStatus`] also reports the runtime's subtree guarantee.
//!
//! Every shutdown mode closes new message admission.
//! [`Shutdown::Stop`] lets dispatched replies finish.
//! It discards messages still awaiting dispatch.
//! [`Shutdown::Drain`] keeps accepted work eligible for dispatch.
//! An abandoned call may still be skipped before dispatch.
//! [`Shutdown::Kill`] discards queued messages.
//! It drops cooperative actor work between polls.
//! It skips graceful cleanup.
//! No mode interrupts a non-returning poll.
//! No mode interrupts non-returning user `Drop` code.
//! See [`Shutdown`] for each mode's complete behavior.
//!
//! Dropping [`ActorOwner`] requests Kill without waiting.
//! Dropping an [`ActorRef`] only drops that handle.
//! [`ExitStatus`] separates local reason and subtree confirmation.
//! [`ActorOwner::wait`] retains ownership while waiting.
//! [`ActorRef::closed`] only observes actor termination.
//! Either wait may remain pending indefinitely.
//!
//! # Communication graph
//!
//! Actor handles may cross supervision-tree boundaries.
//! They may also form cycles.
//! Cyclic calls may wait indefinitely.
//! Initialization blocks dispatch.
//! A call to the same actor cannot complete inside [`Actor::init`].
//! Actor-local continuations avoid another mailbox call.
//! Build them with [`ActorFutureExt::map`] or [`ActorFutureExt::then`].
//!
//! # Imports
//!
//! [`prelude`] contains traits used to define actors.
//! Runtime operations remain explicit imports.
//! This keeps lifecycle choices visible at call sites.

// Derives use this name inside the runtime package.
// External callers may still rename their dependency.
extern crate self as loong_actor;

use std::{future::Future, pin::Pin};

mod actor;
mod address;
mod config;
mod error;
mod future;
mod mailbox;
mod owned;
pub mod reply;
mod runtime;
pub mod scheduling;
mod supervision;
pub mod transport;

pub use actor::{Actor, Handler, HasInterleaving, HasMailbox, Message, SyncHandler};
pub use address::{ActorRef, Response};
pub use config::{
    ActorConfig, DynamicInterleavingOptions, DynamicMailboxOptions, ReplySchedulingConfig,
};
pub use error::{
    CallError, SendError, TryCallError, TryCallErrorKind, TrySendError, TrySendErrorKind,
};
pub use future::{ActorFuture, ActorFutureExt, FutureActor, IntoActorFuture, Map, Then};
pub use loong_actor_macros::{Message, actor};
pub use reply::{InterleavedFutureExt, IntoReply, ReplyExt};
pub use runtime::{ActorOwner, ActorScope, SpawnOptions, StopScope, spawn, spawn_with};
pub use supervision::{
    Child, ChildExit, ChildId, ExitReason, ExitStatus, Shutdown, ShutdownStatus, SubtreeStatus,
};
pub use transport::MessageConfig;

/// Implementation details used by generated actor configuration.
#[doc(hidden)]
pub mod __private {
    pub use crate::config::{
        ActorOptions, DEFAULT_MAILBOX_CAPACITY, DEFAULT_MAX_IN_FLIGHT, DynamicInterleaving,
        DynamicMailbox, FixedInterleaving, FixedMailbox, NoInterleaving, NoMailbox,
        UnboundedInterleaving, UnboundedMailbox,
    };
    pub use crate::transport::{
        BoundedInbox, BoundedSender, NoInbox, NoSender, UnboundedInbox, UnboundedSender,
    };
}

/// Common traits and types for defining actors and handlers.
///
/// This prelude deliberately stops at the actor definition boundary. Runtime
/// entry points, ownership handles, lifecycle controls, addresses, and errors
/// remain explicit imports so operational behavior stays visible at call sites.
pub mod prelude {
    pub use crate::{
        Actor, ActorFuture, ActorFutureExt, ActorScope, DynamicInterleavingOptions,
        DynamicMailboxOptions, Handler, HasInterleaving, HasMailbox, InterleavedFutureExt,
        IntoActorFuture, IntoReply, Message, ReplyExt, StopScope, SyncHandler, actor, reply,
    };
}

// Heap type erasure is confined to heterogeneous scheduler/mailbox ownership
// and the once-per-actor task wrapper. Public reply construction stays generic.
pub(crate) type ErasedFuture<'a, T = ()> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[cfg(test)]
mod message_derive_tests {
    #[derive(crate::Message)]
    struct InternalMessage;

    // Unit targets make proc-macro-crate return `Itself`.
    // This proves expansions use the stable runtime alias.
    #[test]
    fn derive_resolves_runtime_package() {
        // This helper makes reply mismatches fail compilation.
        fn assert_message<M: crate::Message<Reply = ()>>() {}

        assert_message::<InternalMessage>();
    }
}
