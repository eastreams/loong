#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

//! Typed local actors with bounded mailboxes and structured supervision.
//!
//! The runtime separates communication from ownership. An [`ActorRef`] is a
//! cloneable, non-owning address, while the unique [`ActorOwner`] controls a
//! root actor's lifetime. Child ownership stays inside the parent's
//! [`ActorScope`], forming a runtime-enforced tree.
//!
//! The tree governs lifecycle ownership, not the communication graph. Addresses
//! may still form cycles, and cyclic request/reply waits can deadlock.
//!
//! Handlers synchronously choose an explicit reply scheduling mode. Actor-aware
//! reply futures borrow actor state only for one poll at a time; owned replies
//! never borrow it. Dropping a future cannot undo effects that already occurred.
//!
//! Mailbox FIFO governs dispatch order, not reply completion order. Owned and
//! interleaved self-calls require an additional free in-flight slot; prefer
//! [`ActorFutureExt::map`] or [`ActorFutureExt::then`] for consecutive local
//! actor work. Kill is cooperative between polls and cannot interrupt a running
//! synchronous handler, a poll call that never returns, or user `Drop` code.

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
pub use reply::IntoReply;
pub use runtime::{ActorOwner, ActorScope, SpawnOptions, spawn, spawn_with};
pub use supervision::{Child, ChildExit, ChildId, ExitReason, Shutdown, ShutdownStatus};

// Heap type erasure is confined to heterogeneous scheduler/mailbox ownership
// and the once-per-actor task wrapper. Public reply construction stays generic.
pub(crate) type ErasedFuture<'a, T = ()> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
