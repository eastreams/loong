use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Expr, Ident, ItemImpl, Token,
    parse::{Parse, ParseStream},
    parse_macro_input,
};

use crate::actor_crate_path;

/// Expands one actor attribute.
pub(super) fn expand(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as ActorArgs);
    let implementation = parse_macro_input!(input as ItemImpl);

    expand_actor(&implementation, args)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand_actor(implementation: &ItemImpl, args: ActorArgs) -> syn::Result<TokenStream2> {
    validate_actor_impl(implementation)?;
    let actor = actor_crate_path()?;
    let self_ty = &implementation.self_ty;
    let (impl_generics, _, where_clause) = implementation.generics.split_for_impl();
    let cfg_attrs = implementation
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"));

    if let (None, Some((name, _))) = (&args.mailbox, &args.interleaved) {
        return Err(syn::Error::new(
            name.span(),
            "`interleaved` requires a `mailbox` option",
        ));
    }
    if let (None, Some((name, _))) = (&args.mailbox, &args.mailbox_budget) {
        return Err(syn::Error::new(
            name.span(),
            "`mailbox_budget` requires a `mailbox` option",
        ));
    }

    let mailbox_budget = args.mailbox_budget.as_ref().map(|(_, budget)| {
        quote! {
            const MAILBOX_DISPATCH_BUDGET: ::core::num::NonZeroUsize =
                ::core::num::NonZeroUsize::new(#budget)
                    .expect("mailbox budget must be greater than zero");
        }
    });

    let messaging = match args.mailbox {
        Some(mailbox) => {
            let interleaving = match args.interleaved {
                Some((_, capacity)) => capacity.expand_interleaving(&actor),
                None => quote!(#actor::__private::NoInterleaving),
            };
            mailbox.expand_mailbox(&actor, &interleaving)
        }
        None => quote!(#actor::__private::NoMessaging),
    };
    let supervision = match args.children {
        Some(children) => children.expand_children(&actor),
        None => quote!(#actor::__private::NoChildren),
    };

    Ok(quote! {
        #implementation

        #(#cfg_attrs)*
        #[automatically_derived]
        impl #impl_generics #actor::ActorConfig for #self_ty #where_clause {
            #mailbox_budget
            type Messaging = #messaging;
            type Supervision = #supervision;
        }
    })
}

fn validate_actor_impl(implementation: &ItemImpl) -> syn::Result<()> {
    let Some((path, _)) = &implementation.trait_ else {
        return Err(syn::Error::new_spanned(
            implementation,
            "expected `impl Actor for Type`",
        ));
    };
    if !path
        .segments
        .last()
        .is_some_and(|item| item.ident == "Actor")
    {
        return Err(syn::Error::new_spanned(
            path,
            "expected `impl Actor for Type`",
        ));
    }
    Ok(())
}

#[derive(Default)]
struct ActorArgs {
    mailbox: Option<CapacitySpec>,
    mailbox_budget: Option<(Ident, Expr)>,
    interleaved: Option<(Ident, CapacitySpec)>,
    children: Option<CapacitySpec>,
}

impl Parse for ActorArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut args = Self::default();
        while !input.is_empty() {
            let name: Ident = input.parse()?;

            if name == "mailbox_budget" {
                if args.mailbox_budget.is_some() {
                    return Err(syn::Error::new(
                        name.span(),
                        "duplicate `mailbox_budget` option",
                    ));
                }
                if !input.peek(Token![=]) {
                    return Err(syn::Error::new(
                        name.span(),
                        "`mailbox_budget` requires `= <const expression>`",
                    ));
                }
                input.parse::<Token![=]>()?;
                let budget: Expr = input.parse()?;
                validate_mailbox_budget_expression(&budget)?;
                args.mailbox_budget = Some((name, budget));
            } else {
                let capacity = if input.peek(Token![=]) {
                    input.parse::<Token![=]>()?;
                    input.parse()?
                } else {
                    CapacitySpec::Default
                };
                if name == "mailbox" {
                    if args.mailbox.is_some() {
                        return Err(syn::Error::new(name.span(), "duplicate `mailbox` option"));
                    }
                    args.mailbox = Some(capacity);
                } else if name == "interleaved" {
                    if args.interleaved.is_some() {
                        return Err(syn::Error::new(
                            name.span(),
                            "duplicate `interleaved` option",
                        ));
                    }
                    args.interleaved = Some((name, capacity));
                } else if name == "children" {
                    if args.children.is_some() {
                        return Err(syn::Error::new(name.span(), "duplicate `children` option"));
                    }
                    args.children = Some(capacity);
                } else {
                    return Err(syn::Error::new(
                        name.span(),
                        "unsupported actor option; expected `mailbox`, `mailbox_budget`, `interleaved`, or `children`",
                    ));
                }
            }

            if input.is_empty() {
                break;
            }
            input.parse::<Token![,]>()?;
        }
        Ok(args)
    }
}

enum CapacitySpec {
    Default,
    Unbounded,
    Fixed(Expr),
    Dynamic,
    DynamicWithDefault(Expr),
}

impl CapacitySpec {
    fn expand_mailbox(&self, actor: &TokenStream2, interleaving: &TokenStream2) -> TokenStream2 {
        match self {
            Self::Default => quote!(#actor::__private::Mailbox<#interleaving>),
            Self::Unbounded => {
                quote!(#actor::__private::UnboundedMailbox<#interleaving>)
            }
            Self::Fixed(capacity) => {
                let capacity = expand_const_argument(capacity);
                quote!(#actor::__private::Mailbox<#interleaving, #capacity>)
            }
            Self::Dynamic => quote!(#actor::__private::DynamicMailbox<#interleaving>),
            Self::DynamicWithDefault(capacity) => {
                let capacity = expand_const_argument(capacity);
                quote!(#actor::__private::DynamicMailbox<#interleaving, #capacity>)
            }
        }
    }

    fn expand_children(&self, actor: &TokenStream2) -> TokenStream2 {
        match self {
            Self::Default => quote!(#actor::__private::Children),
            Self::Unbounded => quote!(#actor::__private::UnboundedChildren),
            Self::Fixed(capacity) => {
                let capacity = expand_const_argument(capacity);
                quote!(#actor::__private::Children<#capacity>)
            }
            Self::Dynamic => quote!(#actor::__private::DynamicChildren),
            Self::DynamicWithDefault(capacity) => {
                let capacity = expand_const_argument(capacity);
                quote!(#actor::__private::DynamicChildren<#capacity>)
            }
        }
    }

    fn expand_interleaving(&self, actor: &TokenStream2) -> TokenStream2 {
        match self {
            Self::Default => quote!(#actor::__private::Interleaving),
            Self::Unbounded => quote!(#actor::__private::UnboundedInterleaving),
            Self::Fixed(capacity) => {
                let capacity = expand_const_argument(capacity);
                quote!(#actor::__private::Interleaving<#capacity>)
            }
            Self::Dynamic => quote!(#actor::__private::DynamicInterleaving),
            Self::DynamicWithDefault(capacity) => {
                let capacity = expand_const_argument(capacity);
                quote!(#actor::__private::DynamicInterleaving<#capacity>)
            }
        }
    }
}

impl Parse for CapacitySpec {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let capacity: Expr = input.parse()?;

        if let Some(policy) = expression_ident(&capacity) {
            if policy == "unbounded" {
                return Ok(Self::Unbounded);
            }
            if policy == "dynamic" {
                return Ok(Self::Dynamic);
            }
        }

        if let Expr::Call(call) = capacity {
            if expression_ident(&call.func).is_some_and(|policy| policy == "dynamic") {
                let capacity = match call.args.first() {
                    Some(capacity) if call.args.len() == 1 => capacity.clone(),
                    _ => {
                        return Err(syn::Error::new_spanned(
                            call,
                            "`dynamic(...)` requires one default capacity",
                        ));
                    }
                };
                validate_capacity_expression(&capacity)?;
                return Ok(Self::DynamicWithDefault(capacity));
            }

            let capacity = Expr::Call(call);
            validate_capacity_expression(&capacity)?;
            return Ok(Self::Fixed(capacity));
        }

        validate_capacity_expression(&capacity)?;
        Ok(Self::Fixed(capacity))
    }
}

// Policy keywords must be one unqualified identifier.
fn expression_ident(expression: &Expr) -> Option<&Ident> {
    let Expr::Path(path) = expression else {
        return None;
    };
    if path.qself.is_some() || path.path.segments.len() != 1 {
        return None;
    }
    Some(&path.path.segments[0].ident)
}

// Complex expressions require braces in const-generic arguments.
fn expand_const_argument(capacity: &Expr) -> TokenStream2 {
    match capacity {
        Expr::Lit(_) => quote!(#capacity),
        Expr::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 => {
            quote!(#capacity)
        }
        Expr::Block(_) => quote!(#capacity),
        _ => quote!({ #capacity }),
    }
}

fn validate_capacity_expression(capacity: &Expr) -> syn::Result<()> {
    let Expr::Lit(expression) = capacity else {
        return Ok(());
    };
    let syn::Lit::Int(capacity) = &expression.lit else {
        return Err(capacity_error(capacity));
    };
    if capacity.base10_parse::<usize>()? == 0 {
        return Err(syn::Error::new(
            capacity.span(),
            "capacity must be greater than zero",
        ));
    }
    Ok(())
}

fn validate_mailbox_budget_expression(budget: &Expr) -> syn::Result<()> {
    let Expr::Lit(expression) = budget else {
        return Ok(());
    };
    let syn::Lit::Int(budget) = &expression.lit else {
        return Err(syn::Error::new_spanned(
            budget,
            "expected an integer const expression",
        ));
    };
    if budget.base10_parse::<usize>()? == 0 {
        return Err(syn::Error::new(
            budget.span(),
            "mailbox budget must be greater than zero",
        ));
    }
    Ok(())
}

fn capacity_error(value: impl quote::ToTokens) -> syn::Error {
    syn::Error::new_spanned(
        value,
        "expected a const expression, `unbounded`, `dynamic`, or `dynamic(N)`",
    )
}
