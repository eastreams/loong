#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

//! Actors own mutable state and process typed messages.
//!
//! These actors are local to one process.
//! Their tasks remain `Send`.
//! Each actor has a bounded mailbox.
//! A mailbox stores accepted messages awaiting dispatch.
//! Dispatch starts the matching [`Handler`] implementation.
//!
//! # Define an actor
//!
//! Implement [`Actor`] for state owned by one actor.
//! Implement [`Message`] for every message type.
//! Its [`Message::Reply`] type defines a successful call result.
//! Implement [`SyncHandler<M>`](SyncHandler) for an immediate reply.
//! Implement [`Handler<M>`](Handler) for other reply strategies.
//!
//! # Start an actor
//!
//! Call [`spawn`] inside a Tokio runtime.
//! It returns the root actor's unique [`ActorOwner`].
//! Obtain an [`ActorRef`] through [`ActorOwner::actor_ref`].
//! Actor references send messages.
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
//! use loong_actor::{ExitReason, Shutdown, prelude::*, spawn};
//!
//! struct Counter(u64);
//!
//! impl Actor for Counter {}
//!
//! struct Add(u64);
//!
//! impl Message for Add {
//!     type Reply = u64;
//! }
//!
//! impl SyncHandler<Add> for Counter {
//!     fn handle(
//!         &mut self,
//!         message: Add,
//!         _scope: &mut ActorScope<Self>,
//!     ) -> u64 {
//!         self.0 += message.0;
//!         self.0
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     let owner = spawn(Counter(0));
//!     let counter = owner.actor_ref();
//!
//!     assert_eq!(counter.call(Add(2)).await, Ok(2));
//!     assert_eq!(
//!         owner.shutdown(Shutdown::Drain).await,
//!         ExitReason::Drained
//!     );
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
//! [`ReplyExt::interleaved`] allows other actor work between polls.
//! [`ReplyExt::exclusive`] pauses mailbox dispatch and interleaved replies.
//! It also pauses child actor exit hooks.
//! Already-dispatched owned replies still progress.
//! Kill can still cancel an exclusive reply between polls.
//! Use [`reply::Either`] when runtime branches need different strategies.
//! The [`reply`] module documents cancellation and shutdown behavior.
//!
//! # Ownership and shutdown
//!
//! Message addresses and lifecycle ownership are separate.
//! Each root actor has one [`ActorOwner`].
//! Each [`ActorScope`] retains its direct child actors.
//! This parent-child ownership forms the supervision tree.
//! Supervision links a child actor's lifecycle to its parent.
//! A [`Child`] is a non-owning child actor handle.
//! Cloned [`ActorRef`] values never keep actors alive.
//! Parent shutdown requests shutdown from every owned child actor.
//! Strong [`ExitReason`] values appear after descendants exit.
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
//! Dropping an [`ActorRef`] only drops that address.
//! [`ExitReason`] describes why an actor ended.
//! [`ActorOwner::wait`] retains ownership while waiting.
//! [`ActorRef::closed`] only observes actor termination.
//! Either wait may remain pending indefinitely.
//!
//! # Communication graph
//!
//! Addresses may cross supervision-tree boundaries.
//! They may also form cycles.
//! Cyclic calls may wait indefinitely.
//! Actor-local continuations avoid another mailbox call.
//! Build them with [`ActorFutureExt::map`] or [`ActorFutureExt::then`].
//!
//! # Imports
//!
//! [`prelude`] contains traits used to define actors.
//! Runtime operations remain explicit imports.
//! This keeps lifecycle choices visible at call sites.

use std::{future::Future, pin::Pin};

mod actor;
mod address;
mod error;
mod future;
mod mailbox;
pub mod reply;
mod runtime;
mod scheduler;
mod supervision;

pub use actor::{Actor, Handler, Message, SyncHandler};
pub use address::{ActorRef, Response};
pub use error::{
    CallError, SendError, SpawnChildError, TryCallError, TryCallErrorKind, TrySendError,
    TrySendErrorKind,
};
pub use future::{ActorFuture, ActorFutureExt, FutureActor, IntoActorFuture, Map, Then};
pub use reply::{IntoReply, ReplyExt};
pub use runtime::{ActorOwner, ActorScope, SpawnOptions, spawn, spawn_with};
pub use supervision::{Child, ChildExit, ChildId, ExitReason, Shutdown, ShutdownStatus};

/// Common traits and types for defining actors and handlers.
///
/// This prelude deliberately stops at the actor definition boundary. Runtime
/// entry points, ownership handles, lifecycle controls, addresses, and errors
/// remain explicit imports so operational behavior stays visible at call sites.
pub mod prelude {
    pub use crate::{
        Actor, ActorFuture, ActorFutureExt, ActorScope, Handler, IntoActorFuture, IntoReply,
        Message, ReplyExt, SyncHandler, reply,
    };
}

// Heap type erasure is confined to heterogeneous scheduler/mailbox ownership
// and the once-per-actor task wrapper. Public reply construction stays generic.
pub(crate) type ErasedFuture<'a, T = ()> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
