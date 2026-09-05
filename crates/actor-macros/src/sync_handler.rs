use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{ItemImpl, Type, parse_macro_input, parse_quote};

use crate::actor_crate_path;

/// Expands `#[loac::sync_handler]` on a `SyncHandler` impl.
pub(super) fn expand(args: TokenStream, input: TokenStream) -> TokenStream {
    if !args.is_empty() {
        return syn::Error::new_spanned(
            TokenStream2::from(args),
            "`#[loac::sync_handler]` does not accept arguments",
        )
        .into_compile_error()
        .into();
    }

    let item = parse_macro_input!(input as ItemImpl);

    let Some((trait_path, _for)) = &item.trait_ else {
        return syn::Error::new_spanned(
            &item,
            "`#[loac::sync_handler]` only applies to `impl SyncHandler<...> for ...` blocks",
        )
        .into_compile_error()
        .into();
    };

    // `impl SyncHandler<M> for Actor`
    let trait_path = trait_path.clone();
    let actor_ty = (*item.self_ty).clone();

    let message_ty = match sync_message_arg(&trait_path) {
        Ok(Some(message_ty)) => message_ty,
        Ok(None) => {
            return syn::Error::new_spanned(
                &trait_path,
                "expected `impl loac::SyncHandler<MessageType> for ...`",
            )
            .into_compile_error()
            .into();
        }
        Err(error) => {
            return syn::Error::new_spanned(&trait_path, error.to_string())
                .into_compile_error()
                .into();
        }
    };
    let Type::Path(message_ty) = message_ty else {
        return syn::Error::new_spanned(
            &message_ty,
            "expected a message type path in `SyncHandler<...>`",
        )
        .into_compile_error()
        .into();
    };
    let message_ty = message_ty.clone();

    let actor = match actor_crate_path() {
        Ok(path) => path,
        Err(error) => return error.into_compile_error().into(),
    };

    let mut dispatch_generics = item.generics.clone();
    dispatch_generics
        .make_where_clause()
        .predicates
        .push(parse_quote!(#message_ty: #actor::Message));

    let captures = item
        .generics
        .params
        .iter()
        .filter_map(|param| match param {
            syn::GenericParam::Type(ty) => Some(ty.ident.clone()),
            syn::GenericParam::Const(cnst) => Some(cnst.ident.clone()),
            syn::GenericParam::Lifetime(_) => None,
        })
        .collect::<Vec<_>>();
    let use_capture = if captures.is_empty() {
        quote!(use<>)
    } else {
        quote!(use<#(#captures),*>)
    };

    let (impl_generics, _, where_clause) = dispatch_generics.split_for_impl();
    let message_type = &message_ty;

    let original_impl = &item;
    let dispatch_impl = quote! {
        #[automatically_derived]
        impl #impl_generics #actor::DispatchHandler<
            #message_type,
            <#message_type as #actor::Message>::Kind
        > for #actor_ty #where_clause {
            fn handle(
                &mut self,
                message: #message_type,
                scope: &mut #actor::ActorScope<'_, Self>,
            ) -> impl #actor::IntoReply<Self, #message_type> + #use_capture {
                #actor::ReplyExt::ready(
                    <Self as #actor::SyncHandler<#message_type>>::handle(self, message, scope)
                )
            }
        }
    };

    let expanded = quote! {
        #original_impl
        #dispatch_impl
    };

    expanded.into()
}

fn sync_message_arg(trait_path: &syn::Path) -> syn::Result<Option<Type>> {
    // `SyncHandler<M>`; the last segment carries the message type argument.
    let segment = trait_path.segments.last().ok_or_else(|| {
        syn::Error::new_spanned(trait_path, "expected a `SyncHandler<...>` trait path")
    })?;
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return Err(syn::Error::new_spanned(
            trait_path,
            "expected `SyncHandler<MessageType>` with a type argument",
        ));
    };
    let mut types = args.args.iter().filter_map(|arg| {
        if let syn::GenericArgument::Type(ty) = arg {
            Some(ty.clone())
        } else {
            None
        }
    });
    let message_ty = types.next();
    if types.next().is_some() {
        return Err(syn::Error::new_spanned(
            trait_path,
            "`SyncHandler` takes exactly one message type argument",
        ));
    }
    Ok(message_ty)
}
