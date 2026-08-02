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
/// Each option enables one runtime capability.
/// A bare attribute enables no optional capability.
///
/// Supported options:
///
/// - `mailbox` enables public messaging.
/// - `children` enables direct-child ownership.
/// - `interleaved` enables interleaved replies.
///
/// `interleaved` requires `mailbox`.
/// Omit an option to disable its capability.
/// A key-only option uses fixed capacity 32.
/// Use `option = Policy` to select another policy.
/// A const expression selects a fixed capacity.
/// `unbounded` has no finite limit.
/// `dynamic` has a default capacity of 32.
/// `dynamic(N)` selects another default.
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
