#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

//! Typed local actors with bounded mailboxes and structured supervision.
//!
//! Implement [`Actor`] and a typed [`Handler`] for each accepted [`Message`]. A
//! handler synchronously selects its reply scheduling semantics: a bare
//! [`Future`] uses owned scheduling, while the [`reply`] module provides ready
//! and actor-aware strategies. That module documents actor borrowing, fairness,
//! in-flight capacity, panic, and Kill behavior.
//!
//! Use [`ActorRef::call`] for admission with backpressure or
//! [`ActorRef::try_call`] for immediate admission with message recovery. Request
//! failures retain their admission/dispatch phase in [`CallError`].
//!
//! Communication and lifecycle ownership are separate. An [`ActorRef`] is a
//! cloneable, non-owning address, while the unique [`ActorOwner`] owns a root
//! actor. [`ActorScope::spawn_child`] keeps child ownership in the parent scope,
//! forming a runtime-enforced tree. See [`Shutdown`] for lifecycle behavior.
//!
//! The ownership tree does not constrain the communication graph: addresses may
//! form cycles. [`ActorScope::myself`] documents self-call progress and deadlock
//! boundaries; use [`ActorFutureExt::map`] or [`ActorFutureExt::then`] for
//! consecutive actor-local work that does not require a mailbox boundary.

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

pub use actor::{Actor, Handler, Message};
pub use address::{ActorRef, Response};
pub use error::{CallError, SpawnChildError, TryCallError, TryCallErrorKind};
pub use future::{ActorFuture, ActorFutureExt, FutureActor, IntoActorFuture, Map, Then};
pub use reply::{IntoReply, ReplyExt};
pub use runtime::{ActorOwner, ActorScope, SpawnOptions, spawn, spawn_with};
pub use supervision::{Child, ChildExit, ChildId, ExitReason, Shutdown, ShutdownStatus};

// Heap type erasure is confined to heterogeneous scheduler/mailbox ownership
// and the once-per-actor task wrapper. Public reply construction stays generic.
pub(crate) type ErasedFuture<'a, T = ()> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
