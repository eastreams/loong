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
        .filter(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
        .collect::<Vec<_>>();
    let config_attrs = &cfg_attrs;

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

    let mailbox = match &args.mailbox {
        Some(mailbox) => mailbox.expand_mailbox(&actor),
        None => MailboxExpansion::absent(&actor),
    };
    let MailboxExpansion {
        options,
        sender,
        inbox,
        open,
    } = mailbox;

    // Parsing reserves these options for their direct configuration units.
    // They do not project placeholder policy types in this expansion.
    let _ = args.interleaved;
    let _ = args.children;

    Ok(quote! {
        #implementation

        #(#config_attrs)*
        #[automatically_derived]
        impl #impl_generics #actor::ActorConfig for #self_ty #where_clause {
            type Options = #actor::__private::ActorOptions<Self, #options>;
        }

        #(#config_attrs)*
        #[automatically_derived]
        impl #impl_generics #actor::MessageConfig for #self_ty #where_clause {
            #mailbox_budget

            type Sender = #sender;
            type Inbox = #inbox;

            fn open(options: &Self::Options) -> (Self::Sender, Self::Inbox) {
                #open
            }
        }

        #(#config_attrs)*
        #[automatically_derived]
        impl #impl_generics #actor::InterleavingConfig for #self_ty #where_clause {
            fn max_in_flight(options: &Self::Options) -> ::core::num::NonZeroUsize {
                options.max_in_flight()
            }
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
    fn expand_mailbox(&self, actor: &TokenStream2) -> MailboxExpansion {
        match self {
            Self::Default => MailboxExpansion::bounded(
                actor,
                quote!(#actor::__private::DEFAULT_MAILBOX_CAPACITY),
            ),
            Self::Unbounded => MailboxExpansion::unbounded(actor),
            Self::Fixed(capacity) => {
                let capacity = expand_const_argument(capacity);
                MailboxExpansion::bounded(actor, capacity)
            }
            Self::Dynamic => MailboxExpansion::dynamic(actor, None),
            Self::DynamicWithDefault(capacity) => {
                let capacity = expand_const_argument(capacity);
                MailboxExpansion::dynamic(actor, Some(capacity))
            }
        }
    }
}

struct MailboxExpansion {
    options: TokenStream2,
    sender: TokenStream2,
    inbox: TokenStream2,
    open: TokenStream2,
}

impl MailboxExpansion {
    fn absent(actor: &TokenStream2) -> Self {
        Self {
            options: quote!(#actor::__private::NoMailbox),
            sender: quote!(#actor::__private::NoSender),
            inbox: quote!(#actor::__private::NoInbox),
            open: quote!(#actor::__private::NoSender::open()),
        }
    }

    fn bounded(actor: &TokenStream2, capacity: TokenStream2) -> Self {
        Self {
            options: quote!(#actor::__private::FixedMailbox),
            sender: quote!(#actor::__private::BoundedSender<Self>),
            inbox: quote!(#actor::__private::BoundedInbox<Self>),
            open: quote! {
                let capacity = const {
                    ::core::num::NonZeroUsize::new(#capacity)
                        .expect("mailbox capacity must be greater than zero")
                };
                #actor::__private::BoundedSender::<Self>::open(capacity)
            },
        }
    }

    fn dynamic(actor: &TokenStream2, default: Option<TokenStream2>) -> Self {
        let options = match default {
            Some(default) => quote!(#actor::__private::DynamicMailbox<#default>),
            None => quote!(#actor::__private::DynamicMailbox),
        };
        Self {
            options,
            sender: quote!(#actor::__private::BoundedSender<Self>),
            inbox: quote!(#actor::__private::BoundedInbox<Self>),
            open: quote! {
                #actor::__private::BoundedSender::<Self>::open(options.mailbox_capacity())
            },
        }
    }

    fn unbounded(actor: &TokenStream2) -> Self {
        Self {
            options: quote!(#actor::__private::UnboundedMailbox),
            sender: quote!(#actor::__private::UnboundedSender<Self>),
            inbox: quote!(#actor::__private::UnboundedInbox<Self>),
            open: quote!(#actor::__private::UnboundedSender::<Self>::open()),
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
