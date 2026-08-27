//! Stateless provider clients and ordered failover.
//!
//! A [`Provider`] is a plain upstream client, not an actor. It owns no
//! mailbox and no lifecycle; the actor that initiates a stream hosts the
//! production work (as an owned reply future in `loac`).

mod failover;
mod provider;

pub use failover::{BorrowedProvider, Failover, RefFailover};
pub use provider::{Provider, StreamError};
