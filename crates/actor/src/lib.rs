#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

//! Typed local actors with bounded messaging and structured supervision.
//!
//! An [`Actor`] owns mutable state.
//! Its lifecycle hooks run serially.
//! Communication stays inside one process.
//! Actor tasks are `Send` and run on Tokio.
//! Messaging, interleaving, and child actor ownership are opt-in.
//!
//! # Quick start
//!
//! This actor accepts typed `Add` requests.
//! [`Handler`] produces each reply through an actor-access `cx` future.
//!
//! ```
//! use loac::{ExitReason, Shutdown, prelude::*};
//!
//! struct Counter(u64);
//!
//! #[actor(mailbox, interleaved = unbounded)]
//! impl Actor for Counter {
//!     type SpawnArgs = u64;
//!
//!     async fn init(initial: u64, _scope: &mut ActorScope<'_, Self>) -> Self {
//!         Self(initial)
//!     }
//! }
//!
//! #[derive(Message)]
//! #[message(reply = u64)]
//! struct Add(u64);
//!
//! impl Handler<Add> for Counter {
//!     async fn handle(message: Add, mut cx: Cx<'_, Self>) -> u64 {
//!         cx.with(|actor, _| {
//!             actor.0 += message.0;
//!             actor.0
//!         })
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let owner = loac::spawn::<Counter>(0);
//!     let counter = owner.actor_ref();
//!
//!     assert_eq!(counter.call(Add(2)).await?, 2);
//!     let status = owner.shutdown(Shutdown::Drain).await;
//!     assert_eq!(status.reason(), ExitReason::Drained);
//!     Ok(())
//! }
//! ```
//!
//! [`spawn`] returns before [`Actor::init`] completes.
//! Messages may enter the mailbox during initialization.
//! Handler dispatch starts after initialization.
//! The [`ActorOwner`] owns the root actor's lifecycle.
//!
//! # Choose actor capabilities
//!
//! Most actors use [`#[actor(...)]`](actor) on their [`Actor`] implementation.
//! The attribute generates the built-in runtime configuration.
//! Its reference documents every syntax, default, and constraint.
//! The generated [`MessageConfig`] opens three matched states.
//! They are [`Sender`](MessageConfig::Sender), [`Inbox`](MessageConfig::Inbox),
//! and [`Scheduler`](MessageConfig::Scheduler).
//! Without `mailbox`, the scheduler is [`Disabled`](scheduling::Disabled).
//! Without `interleaved`, a mailbox uses [`Serial`](scheduling::Serial).
//!
//! | Option | Purpose | When omitted |
//! | --- | --- | --- |
//! | `mailbox` | Enables typed [`send`](ActorRef::send) and [`call`](ActorRef::call) | Messaging methods are unavailable |
//! | `mailbox_budget = E` | Limits consecutive message dispatch | Uses `16` with a mailbox |
//! | `interleaved` | Enables [`Handler`]/[`StreamHandler`] dispatch and [`interleaved`](InterleavedFutureExt::interleaved) replies | The methods are unavailable |
//! | `children` | Enables [`spawn_child`](ActorScope::spawn_child) | The method is unavailable |
//!
//! `mailbox`, `interleaved`, and `children` support three limit profiles.
//! Those profiles are fixed, dynamic, and unbounded.
//! Dynamic profiles expose spawn-specific overrides:
//!
//! - [`with_mailbox_capacity`](DynamicMailboxOptions::with_mailbox_capacity);
//! - [`with_max_in_flight`](DynamicInterleavingOptions::with_max_in_flight);
//! - [`with_max_children`](DynamicChildrenOptions::with_max_children).
//!
//! Pass changed [`SpawnOptions`] to [`spawn_with`].
//!
//! # Manual no-mailbox configuration
//!
//! The [`#[actor(...)]`](actor) attribute covers ordinary actors.
//! Manual configurations exist for custom transports.
//! A mailbox manual config supplies its own transport.
//! A no-mailbox manual config may reuse the built-in states.
//! Implement [`ActorConfig`], [`MessageConfig`], and [`SupervisionConfig`].
//! Use [`transport::NoSender`], [`transport::NoInbox`], and
//! [`scheduling::Disabled`].
//!
//! A manual config may also supply its own options carrier.
//! Normally the empty carrier (`()`) suffices.
//!
//! # Core model
//!
//! | Type | Role |
//! | --- | --- |
//! | [`Actor`] | Owns state and serial lifecycle hooks |
//! | [`ActorRef`] | Provides a cloneable, typed non-owning handle |
//! | [`Recipient`] | Erases the actor type for one message capability |
//! | [`ActorOwner`] | Uniquely owns one root actor |
//! | [`ActorScope`] | Exposes temporary capabilities during actor work |
//!
//! An [`ActorRef`] may request shutdown.
//! It sends messages only when the actor has [`HasMailbox`].
//! Keeping an [`ActorRef`] does not keep its actor alive.
//! A [`Recipient`] handle has only one message capability and no lifecycle methods.
//!
//! # Messages
//!
//! Derive [`Message`] for each accepted request type.
//! A message with `#[message(reply = Type)]` implements [`HasReply`] and can be
//! used with [`ActorRef::call`]; without the attribute it is send-only and
//! accepts only [`ActorRef::send`]. [`Message::Reply`] defaults to `()` either
//! way.
//!
//! Implement [`Handler<M>`](Handler) for an actor-access `cx` reply future.
//! Implement [`RawHandler<M>`](RawHandler) when a reply must choose an explicit
//! scheduling strategy such as [`ReplyExt::exclusive`] or
//! [`ReplyExt::ready`].
//! One actor may handle many message types.
//!
//! [`ActorRef::call`] waits for acceptance and a typed reply.
//! [`ActorRef::send`] waits only for one-way message acceptance.
//! [`ActorRef::recipient`] creates a type-erased handle for one message type.
//! The `try_` variants never wait for mailbox capacity.
//! Their errors retain messages that were not accepted.
//! Method docs describe cancellation and shutdown races.
//!
//! # Reply progress
//!
//! After dispatch, a handler returns a reply strategy. The strategy controls
//! how the reply runs beside the rest of the actor.
//!
//! | Strategy | Selected by | Actor progress while the reply runs |
//! | --- | --- | --- |
//! | ready | [`value.ready()`](ReplyExt::ready) from a [`RawHandler`] or [`RawStreamHandler`] | The reply is already complete during dispatch |
//! | owned | A bare [`Future`] from a [`RawHandler`] or [`RawStreamHandler`] | A Tokio task runs it beside all actor work |
//! | interleaved | [`Handler`] async fn, [`StreamHandler`] async fn, or [`future.interleaved()`](InterleavedFutureExt::interleaved) | The actor task polls it fairly with mailbox, lifecycle, and other interleaved work |
//! | exclusive | [`future.exclusive()`](ReplyExt::exclusive) from a [`RawHandler`] or [`RawStreamHandler`] | Mailbox and actor-aware work pause until it finishes; owned tasks continue |
//!
//! [`Handler`] and [`StreamHandler`] always select interleaved scheduling, so
//! they require [`HasInterleaving`]. [`RawHandler`] and [`RawStreamHandler`]
//! may select any strategy. `ready` and `exclusive` need no interleaving
//! capability.
//! See [`reply`] for cancellation, panic, and scheduling details.
//! See [`scheduling`] for built-in scheduling profiles.
//!
//! # Ownership and child actors
//!
//! Each root actor has one [`ActorOwner`].
//! [`ActorScope::spawn_child`] starts a direct child actor.
//! The parent runtime owns that child actor.
//! Child spawning requires [`HasChildren`].
//! A [`Child`] is a typed, non-owning handle.
//! Parent shutdown reaches every owned child actor.
//!
//! Ownership forms a tree.
//! Actor references may cross tree boundaries.
//! They may also form communication cycles.
//! See [`supervision`] for built-in child actor profiles.
//!
//! # Shutdown and completion
//!
//! Every shutdown mode closes new message acceptance.
//!
//! | Mode | Behavior |
//! | --- | --- |
//! | [`Stop`](Shutdown::Stop) | Finishes dispatched replies and discards queued messages |
//! | [`Drain`](Shutdown::Drain) | Dispatches eligible queued messages and finishes resulting replies |
//! | [`Kill`](Shutdown::Kill) | Cancels cooperative work and skips graceful cleanup |
//!
//! Kill takes effect between polls.
//! It cannot interrupt synchronous code or user destructors.
//! [`Shutdown`] documents the complete retained-work contract.
//!
//! [`ActorOwner::shutdown`] requests a mode and waits.
//! [`ActorOwner::wait`] waits without requesting shutdown.
//! [`ActorRef::closed`] only observes actor termination.
//! [`ExitStatus`] separates local reason from subtree confirmation.
//!
//! # Progress boundaries
//!
//! Initialization and lifecycle hooks are serial.
//! They pause handler dispatch while pending.
//! Awaiting a self-call requires a later mailbox dispatch.
//! It cannot complete during initialization or exclusive work.
//! Communication cycles can therefore wait indefinitely.
//! Use [`map`](ActorFutureExt::map) or [`then`](ActorFutureExt::then) for local sequencing.
//!
//! # Advanced configuration
//!
//! The `actor` attribute covers built-in runtime shapes.
//! Custom configurations implement [`ActorConfig`] and [`MessageConfig`].
//! They also implement [`SupervisionConfig`].
//! [`MessageConfig`] opens transport and reply scheduling together.
//! Choose public profiles from [`scheduling`] and [`supervision`].
//! The [`transport`] module documents custom message transports.
//!
//! # Imports
//!
//! [`prelude`] contains actor-definition traits and extension methods.
//! Runtime operations remain explicit imports.
//! This keeps lifecycle choices visible at call sites.
//! The [examples index] lists runnable guides.
//!
//! [examples index]: https://github.com/eastreams/loong/blob/dev/crates/actor/examples/README.md

// Derives use this name inside the runtime package.
// External callers may still rename their dependency.
extern crate self as loac;

use std::{future::Future, pin::Pin};

mod access;
mod actor;
mod address;
mod config;
mod error;
mod future;
mod lifecycle;
mod mailbox;
mod owned;
pub mod reply;
mod runtime;
pub mod scheduling;
pub mod supervision;
pub mod transport;
mod writer;

pub use access::Cx;
pub use actor::{
    Actor, DispatchHandler, Handler, HasChildren, HasInterleaving, HasMailbox, HasReply, Message,
    RawHandler, RawStreamHandler, StreamHandler,
};
pub use address::{ActorRef, Recipient, Response};
pub use config::{
    ActorConfig, DynamicChildrenOptions, DynamicInterleavingOptions, DynamicMailboxOptions,
    SupervisionConfig,
};
pub use error::{
    CallError, SendError, SendToError, TryCallError, TryCallErrorKind, TrySendError,
    TrySendErrorKind,
};
pub use future::{ActorFuture, ActorFutureExt, FutureActor, IntoActorFuture, Map, Then};
pub use lifecycle::{
    Child, ChildExit, ChildId, ExitReason, ExitStatus, Shutdown, ShutdownStatus, SubtreeStatus,
};
pub use loac_macros::{Message, actor};
pub use reply::{
    CxReply, CxStream, InterleavedFutureExt, IntoReply, IntoStreamReply, Items, RawKind,
    RawStreamKind, ReplyExt, StreamKind, StreamMessage, StreamReply, SyncKind,
};
pub use runtime::{
    ActorOwner, ActorScope, ActorSpawner, SpawnOptions, StopScope, spawn, spawn_with,
};
pub use transport::MessageConfig;
pub use writer::Writer;

// The macro references these through `__private`.
// The root re-export keeps rustc diagnostics free of `__private` paths.
#[doc(hidden)]
pub use config::{
    ActorOptions, DynamicChildren, DynamicInterleaving, DynamicMailbox, FixedChildren,
    FixedInterleaving, FixedMailbox, NoChildren, NoInterleaving, NoMailbox, UnboundedChildren,
    UnboundedInterleaving, UnboundedMailbox,
};

/// Implementation details used by generated actor configuration.
#[doc(hidden)]
pub mod __private {
    pub use crate::config::{
        ActorOptions, DEFAULT_MAILBOX_CAPACITY, DEFAULT_MAX_CHILDREN, DEFAULT_MAX_IN_FLIGHT,
        DynamicChildren, DynamicInterleaving, DynamicMailbox, FixedChildren, FixedInterleaving,
        FixedMailbox, NoChildren, NoInterleaving, NoMailbox, UnboundedChildren,
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
        Actor, ActorFuture, ActorFutureExt, ActorScope, ActorSpawner, Cx, CxReply, CxStream,
        DispatchHandler, DynamicChildrenOptions, DynamicInterleavingOptions, DynamicMailboxOptions,
        Handler, HasChildren, HasInterleaving, HasMailbox, HasReply, InterleavedFutureExt,
        IntoActorFuture, IntoReply, IntoStreamReply, Items, Message, RawHandler, RawKind,
        RawStreamHandler, RawStreamKind, ReplyExt, StopScope, StreamHandler, StreamMessage,
        StreamReply, SyncKind, Writer, actor, reply,
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
