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

struct MessageOptions {
    reply: Type,
    stream: Option<Type>,
    explicit_reply: bool,
}

fn expand_message(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let MessageOptions {
        reply,
        stream,
        explicit_reply,
    } = message_options(&input.attrs)?;
    let actor = actor_crate_path()?;
    let ident = &input.ident;
    let (_, type_generics, _) = input.generics.split_for_impl();
    let message: Type = parse_quote!(#ident #type_generics);

    let mut generics = input.generics.clone();
    let predicates = &mut generics.make_where_clause().predicates;
    predicates.push(parse_quote!(#message: ::core::marker::Send + 'static));
    predicates.push(parse_quote!(#reply: ::core::marker::Send + 'static));

    let kind: Type;
    let reply_ty: Type;
    if let Some(item) = &stream {
        predicates.push(parse_quote!(#item: ::core::marker::Send + 'static));
        kind = parse_quote!(#actor::reply::StreamKind);
        reply_ty = parse_quote!(#actor::reply::StreamReply<#item, #reply>);
    } else {
        kind = parse_quote!(#actor::reply::SingleKind);
        reply_ty = reply.clone();
    }

    let (impl_generics, _, where_clause) = generics.split_for_impl();

    let has_reply_impl = (explicit_reply || stream.is_some()).then(|| {
        quote! {
            #[automatically_derived]
            impl #impl_generics #actor::HasReply for #message #where_clause {}
        }
    });

    let stream_message_impl = stream.as_ref().map(|item| {
        quote! {
            #[automatically_derived]
            impl #impl_generics #actor::reply::StreamReplyMessage for #message #where_clause {
                type Item = #item;
                type Final = #reply;
            }

            #[automatically_derived]
            impl #impl_generics #actor::reply::StreamMessage for #message #where_clause {}
        }
    });

    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #actor::Message for #message #where_clause {
            type Reply = #reply_ty;
            type Kind = #kind;
        }

        #has_reply_impl
        #stream_message_impl
    })
}

fn message_options(attrs: &[Attribute]) -> syn::Result<MessageOptions> {
    let mut message_attrs = attrs.iter().filter(|attr| attr.path().is_ident("message"));
    let Some(attr) = message_attrs.next() else {
        return Ok(MessageOptions {
            reply: parse_quote!(()),
            stream: None,
            explicit_reply: false,
        });
    };

    if let Some(duplicate) = message_attrs.next() {
        return Err(syn::Error::new_spanned(
            duplicate,
            "at most one `#[message(...)]` attribute is allowed",
        ));
    }

    let mut reply = None;
    let mut stream = None;
    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("reply") {
            if reply.is_some() {
                return Err(meta.error("duplicate `reply` option"));
            }
            reply = Some(meta.value()?.parse().map_err(|error: syn::Error| {
                syn::Error::new(error.span(), "expected a Rust type after `reply =`")
            })?);
            return Ok(());
        }

        if meta.path.is_ident("stream") {
            if stream.is_some() {
                return Err(meta.error("duplicate `stream` option"));
            }
            stream = Some(meta.value()?.parse().map_err(|error: syn::Error| {
                syn::Error::new(error.span(), "expected a Rust type after `stream =`")
            })?);
            return Ok(());
        }

        Err(meta
            .error("unsupported message option; expected `reply = <type>` or `stream = <type>`"))
    })?;

    if reply.is_none() && stream.is_none() {
        return Err(syn::Error::new_spanned(
            attr,
            "expected `reply = <type>` or `stream = <type>`",
        ));
    }

    let selected_reply = reply.clone().unwrap_or_else(|| parse_quote!(()));

    Ok(MessageOptions {
        reply: selected_reply,
        stream,
        explicit_reply: reply.is_some(),
    })
}
