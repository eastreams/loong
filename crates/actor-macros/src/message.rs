use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Attribute, DeriveInput, Type, parse_macro_input, parse_quote};

use crate::actor_crate_path;

/// Expands one message derive.
pub(super) fn expand(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    expand_message(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand_message(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let (explicit_reply, reply) = reply_type(&input.attrs)?;
    let actor = actor_crate_path()?;
    let ident = &input.ident;
    let (_, type_generics, _) = input.generics.split_for_impl();
    let message: Type = parse_quote!(#ident #type_generics);

    let mut generics = input.generics.clone();
    let predicates = &mut generics.make_where_clause().predicates;
    predicates.push(parse_quote!(#message: ::core::marker::Send + 'static));
    predicates.push(parse_quote!(#reply: ::core::marker::Send + 'static));
    let (impl_generics, _, where_clause) = generics.split_for_impl();

    let has_reply_impl = explicit_reply.then(|| {
        quote! {
            #[automatically_derived]
            impl #impl_generics #actor::HasReply for #message #where_clause {}
        }
    });

    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #actor::Message for #message #where_clause {
            type Reply = #reply;
        }

        #has_reply_impl
    })
}

fn reply_type(attrs: &[Attribute]) -> syn::Result<(bool, Type)> {
    let mut message_attrs = attrs.iter().filter(|attr| attr.path().is_ident("message"));
    let Some(attr) = message_attrs.next() else {
        return Ok((false, parse_quote!(())));
    };

    if let Some(duplicate) = message_attrs.next() {
        return Err(syn::Error::new_spanned(
            duplicate,
            "at most one `#[message(...)]` attribute is allowed",
        ));
    }

    let mut reply = None;
    attr.parse_nested_meta(|meta| {
        if !meta.path.is_ident("reply") {
            return Err(meta.error("unsupported message option; expected `reply = <type>`"));
        }
        if reply.is_some() {
            return Err(meta.error("duplicate `reply` option"));
        }

        reply = Some(meta.value()?.parse().map_err(|error: syn::Error| {
            syn::Error::new(error.span(), "expected a Rust type after `reply =`")
        })?);
        Ok(())
    })?;

    let reply = reply.ok_or_else(|| syn::Error::new_spanned(attr, "expected `reply = <type>`"))?;
    Ok((true, reply))
}
