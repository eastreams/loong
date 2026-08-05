#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Procedural macros for `loong-actor`.

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;

mod actor;
mod message;

/// Configures an actor implementation.
///
/// Options select runtime capabilities and policies.
/// A bare attribute enables no optional capability.
///
/// Supported options:
///
/// - `mailbox` enables public messaging.
/// - `mailbox_budget = E` limits consecutive message dispatches.
/// - `children` reserves typed supervision syntax.
/// - `interleaved` enables interleaved replies.
///
/// `children` currently changes no runtime behavior.
/// Every actor can currently own direct children.
///
/// `mailbox` has five forms:
///
/// - `mailbox` uses a fixed capacity of 32.
/// - `mailbox = N` uses a fixed const capacity.
/// - `mailbox = dynamic` uses a per-spawn capacity defaulting to 32.
/// - `mailbox = dynamic(N)` changes that per-spawn default.
/// - `mailbox = unbounded` removes the admission limit.
///
/// Dynamic mailbox forms expose `SpawnOptions::with_mailbox_capacity`.
/// Every finite mailbox capacity must exceed zero.
///
/// `interleaved` has five forms:
///
/// - `interleaved` uses a fixed limit of 32.
/// - `interleaved = N` uses a fixed const limit.
/// - `interleaved = dynamic` uses a per-spawn limit defaulting to 32.
/// - `interleaved = dynamic(N)` changes that per-spawn default.
/// - `interleaved = unbounded` removes the admission limit.
///
/// Every finite interleaved limit must exceed zero.
/// Dynamic forms expose `SpawnOptions::with_max_in_flight`.
/// Unbounded admission can retain arbitrarily many active replies.
/// Omitting `interleaved` provides no capability or reply queue.
/// Exclusive replies do not require this capability.
/// A full finite limit pauses mailbox dispatch.
/// It pauses before the next handler runs.
/// Ready, owned, and exclusive handlers wait behind this gate.
///
/// `interleaved` requires `mailbox`.
/// `mailbox_budget` also requires `mailbox`.
/// `mailbox_budget` accepts a nonzero const expression.
/// `mailbox_budget` defaults to 16 when omitted.
/// It does not force a Tokio task yield.
#[proc_macro_attribute]
pub fn actor(args: TokenStream, input: TokenStream) -> TokenStream {
    actor::expand(args, input)
}

/// Derives `loong_actor::Message` for a struct, enum, or union.
///
/// The reply type defaults to `()`. Use `#[message(reply = Type)]` to select
/// another owned `Send + 'static` Rust type. Generic parameters and existing
/// `where` predicates are preserved.
#[proc_macro_derive(Message, attributes(message))]
pub fn derive_message(input: TokenStream) -> TokenStream {
    message::expand(input)
}

fn actor_crate_path() -> syn::Result<TokenStream2> {
    // Proc macros lack `$crate`.
    // Resolve the package after dependency renaming.
    match crate_name("loong-actor").map_err(|error| {
        syn::Error::new(
            Span::call_site(),
            format!("could not resolve the `loong-actor` crate: {error}"),
        )
    })? {
        // `crate` may name a package binary or example.
        // The runtime exports one stable self alias.
        FoundCrate::Itself => Ok(quote!(::loong_actor)),
        FoundCrate::Name(name) => {
            let ident = syn::Ident::new(&name, Span::call_site());
            Ok(quote!(::#ident))
        }
    }
}
