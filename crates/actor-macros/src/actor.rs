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
        options: mailbox_options,
        sender,
        inbox,
        open: open_mailbox,
    } = mailbox;

    let interleaving = match &args.interleaved {
        Some((_, interleaving)) => interleaving.expand_interleaving(&actor),
        None if args.mailbox.is_some() => InterleavingExpansion::serial(&actor),
        None => InterleavingExpansion::disabled(&actor),
    };
    let InterleavingExpansion {
        options: interleaving_options,
        scheduler,
        open: make_scheduler,
    } = interleaving;

    let supervision = match &args.children {
        Some(children) => children.expand_supervision(&actor),
        None => SupervisionExpansion::absent(&actor),
    };
    let SupervisionExpansion {
        options: supervision_options,
        children,
        open: open_children,
    } = supervision;

    Ok(quote! {
        #implementation

        #(#config_attrs)*
        #[automatically_derived]
        #[doc(hidden)]
        impl #impl_generics #actor::ActorConfig for #self_ty #where_clause {
            type Options = #actor::__private::ActorOptions<
                Self,
                #mailbox_options,
                #interleaving_options,
                #supervision_options,
            >;
        }

        #(#config_attrs)*
        #[automatically_derived]
        #[doc(hidden)]
        impl #impl_generics #actor::MessageConfig for #self_ty #where_clause {
            #mailbox_budget

            type Sender = #sender;
            type Inbox = #inbox;
            type Scheduler = #scheduler;

            fn open(
                options: &Self::Options,
            ) -> (Self::Sender, Self::Inbox, Self::Scheduler) {
                let (sender, inbox) = { #open_mailbox };
                let scheduler = { #make_scheduler };
                (sender, inbox, scheduler)
            }
        }

        #(#config_attrs)*
        #[automatically_derived]
        #[doc(hidden)]
        impl #impl_generics #actor::SupervisionConfig for #self_ty #where_clause {
            type Children = #children;

            fn open_children(options: &Self::Options) -> Self::Children {
                #open_children
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
            } else if name == "mailbox" {
                if args.mailbox.is_some() {
                    return Err(syn::Error::new(name.span(), "duplicate `mailbox` option"));
                }
                args.mailbox = Some(parse_capacity(input, "mailbox capacity")?);
            } else if name == "interleaved" {
                if args.interleaved.is_some() {
                    return Err(syn::Error::new(
                        name.span(),
                        "duplicate `interleaved` option",
                    ));
                }
                let limit = parse_capacity(input, "interleaved limit")?;
                args.interleaved = Some((name, limit));
            } else if name == "children" {
                if args.children.is_some() {
                    return Err(syn::Error::new(name.span(), "duplicate `children` option"));
                }
                args.children = Some(parse_capacity(input, "child capacity")?);
            } else {
                return Err(syn::Error::new(
                    name.span(),
                    "unsupported actor option; expected `mailbox`, `mailbox_budget`, `interleaved`, or `children`",
                ));
            }

            if input.is_empty() {
                break;
            }
            input.parse::<Token![,]>()?;
        }
        Ok(args)
    }
}

fn parse_capacity(input: ParseStream<'_>, quantity: &'static str) -> syn::Result<CapacitySpec> {
    if !input.peek(Token![=]) {
        return Ok(CapacitySpec::Default);
    }
    input.parse::<Token![=]>()?;
    CapacitySpec::parse(input, quantity)
}

enum CapacitySpec {
    Default,
    Unbounded,
    Fixed(Expr),
    Dynamic,
    DynamicWithDefault(Expr),
}

impl CapacitySpec {
    fn parse(input: ParseStream<'_>, quantity: &'static str) -> syn::Result<Self> {
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
                            format!("`dynamic(...)` requires one default {quantity}"),
                        ));
                    }
                };
                validate_capacity_expression(&capacity, quantity)?;
                return Ok(Self::DynamicWithDefault(capacity));
            }

            let capacity = Expr::Call(call);
            validate_capacity_expression(&capacity, quantity)?;
            return Ok(Self::Fixed(capacity));
        }

        validate_capacity_expression(&capacity, quantity)?;
        Ok(Self::Fixed(capacity))
    }

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

    fn expand_interleaving(&self, actor: &TokenStream2) -> InterleavingExpansion {
        match self {
            Self::Default => InterleavingExpansion::fixed(
                actor,
                quote!({ #actor::__private::DEFAULT_MAX_IN_FLIGHT }),
            ),
            Self::Unbounded => InterleavingExpansion::unbounded(actor),
            Self::Fixed(limit) => {
                let limit = expand_const_argument(limit);
                InterleavingExpansion::fixed(actor, limit)
            }
            Self::Dynamic => InterleavingExpansion::dynamic(actor, None),
            Self::DynamicWithDefault(limit) => {
                let limit = expand_const_argument(limit);
                InterleavingExpansion::dynamic(actor, Some(limit))
            }
        }
    }

    fn expand_supervision(&self, actor: &TokenStream2) -> SupervisionExpansion {
        match self {
            Self::Default => SupervisionExpansion::fixed(
                actor,
                quote!({ #actor::__private::DEFAULT_MAX_CHILDREN }),
            ),
            Self::Unbounded => SupervisionExpansion::unbounded(actor),
            Self::Fixed(capacity) => {
                let capacity = expand_const_argument(capacity);
                SupervisionExpansion::fixed(actor, capacity)
            }
            Self::Dynamic => SupervisionExpansion::dynamic(actor, None),
            Self::DynamicWithDefault(capacity) => {
                let capacity = expand_const_argument(capacity);
                SupervisionExpansion::dynamic(actor, Some(capacity))
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
        let nonzero_capacity =
            expand_nonzero_const(&capacity, "mailbox capacity must be greater than zero");
        Self {
            options: quote!(#actor::__private::FixedMailbox),
            sender: quote!(#actor::__private::BoundedSender<Self>),
            inbox: quote!(#actor::__private::BoundedInbox<Self>),
            open: quote! {
                let capacity = #nonzero_capacity;
                #actor::__private::BoundedSender::<Self>::open(capacity)
            },
        }
    }

    fn dynamic(actor: &TokenStream2, default: Option<TokenStream2>) -> Self {
        let validate_default = default.as_ref().map(|default| {
            let nonzero_default =
                expand_nonzero_const(default, "mailbox capacity must be greater than zero");
            quote!(let _ = #nonzero_default;)
        });
        let options = if let Some(default) = default {
            quote!(#actor::__private::DynamicMailbox<#default>)
        } else {
            quote!(#actor::__private::DynamicMailbox)
        };
        Self {
            options,
            sender: quote!(#actor::__private::BoundedSender<Self>),
            inbox: quote!(#actor::__private::BoundedInbox<Self>),
            open: quote! {
                #validate_default
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

struct InterleavingExpansion {
    options: TokenStream2,
    scheduler: TokenStream2,
    open: TokenStream2,
}

impl InterleavingExpansion {
    fn disabled(actor: &TokenStream2) -> Self {
        Self {
            options: quote!(#actor::__private::NoInterleaving),
            scheduler: quote!(#actor::scheduling::Disabled),
            open: quote!(#actor::scheduling::Disabled::new()),
        }
    }

    fn serial(actor: &TokenStream2) -> Self {
        Self {
            options: quote!(#actor::__private::NoInterleaving),
            scheduler: quote!(#actor::scheduling::Serial<Self>),
            open: quote!(#actor::scheduling::Serial::<Self>::new()),
        }
    }

    fn fixed(actor: &TokenStream2, limit: TokenStream2) -> Self {
        let nonzero_limit =
            expand_nonzero_const(&limit, "interleaved limit must be greater than zero");
        Self {
            options: quote!(#actor::__private::FixedInterleaving),
            scheduler: quote!(#actor::scheduling::Fixed<Self, #limit>),
            open: quote! {
                let _ = #nonzero_limit;
                #actor::scheduling::Fixed::<Self, #limit>::new()
            },
        }
    }

    fn dynamic(actor: &TokenStream2, default: Option<TokenStream2>) -> Self {
        let validate_default = default.as_ref().map(|default| {
            let nonzero_default =
                expand_nonzero_const(default, "interleaved limit must be greater than zero");
            quote!(let _ = #nonzero_default;)
        });
        let options = if let Some(default) = default {
            quote!(#actor::__private::DynamicInterleaving<#default>)
        } else {
            quote!(#actor::__private::DynamicInterleaving)
        };
        Self {
            options,
            scheduler: quote!(#actor::scheduling::Dynamic<Self>),
            open: quote! {
                #validate_default
                #actor::scheduling::Dynamic::<Self>::new(
                    options.max_in_flight(),
                )
            },
        }
    }

    fn unbounded(actor: &TokenStream2) -> Self {
        Self {
            options: quote!(#actor::__private::UnboundedInterleaving),
            scheduler: quote!(#actor::scheduling::Unbounded<Self>),
            open: quote!(#actor::scheduling::Unbounded::<Self>::new()),
        }
    }
}

struct SupervisionExpansion {
    options: TokenStream2,
    children: TokenStream2,
    open: TokenStream2,
}

impl SupervisionExpansion {
    fn absent(actor: &TokenStream2) -> Self {
        Self {
            options: quote!(#actor::__private::NoChildren),
            children: quote!(#actor::supervision::Disabled),
            open: quote!(#actor::supervision::Disabled::new()),
        }
    }

    fn fixed(actor: &TokenStream2, capacity: TokenStream2) -> Self {
        let nonzero_capacity =
            expand_nonzero_const(&capacity, "child capacity must be greater than zero");
        Self {
            options: quote!(#actor::__private::FixedChildren),
            children: quote!(#actor::supervision::Fixed<#capacity>),
            open: quote! {
                let _ = #nonzero_capacity;
                #actor::supervision::Fixed::<#capacity>::new()
            },
        }
    }

    fn dynamic(actor: &TokenStream2, default: Option<TokenStream2>) -> Self {
        let validate_default = default.as_ref().map(|default| {
            let nonzero_default =
                expand_nonzero_const(default, "child capacity must be greater than zero");
            quote!(let _ = #nonzero_default;)
        });
        let options = if let Some(default) = default {
            quote!(#actor::__private::DynamicChildren<#default>)
        } else {
            quote!(#actor::__private::DynamicChildren)
        };
        Self {
            options,
            children: quote!(#actor::supervision::Dynamic),
            open: quote! {
                #validate_default
                #actor::supervision::Dynamic::new(options.max_children())
            },
        }
    }

    fn unbounded(actor: &TokenStream2) -> Self {
        Self {
            options: quote!(#actor::__private::UnboundedChildren),
            children: quote!(#actor::supervision::Unbounded),
            open: quote!(#actor::supervision::Unbounded::new()),
        }
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

// Keep named zero failures at the actor definition.
fn expand_nonzero_const(value: &TokenStream2, message: &'static str) -> TokenStream2 {
    quote! {
        const {
            ::core::num::NonZeroUsize::new(#value).expect(#message)
        }
    }
}

fn validate_capacity_expression(capacity: &Expr, quantity: &'static str) -> syn::Result<()> {
    let Expr::Lit(expression) = capacity else {
        return Ok(());
    };
    let syn::Lit::Int(capacity) = &expression.lit else {
        return Err(capacity_error(capacity));
    };
    if capacity.base10_parse::<usize>()? == 0 {
        return Err(syn::Error::new(
            capacity.span(),
            format!("{quantity} must be greater than zero"),
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
