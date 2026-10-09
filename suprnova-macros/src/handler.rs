//! Handler attribute macro implementation
//!
//! Transforms controller functions to extract typed arguments from the
//! request: route-bound values, path values and the request body.
//!
//! ## How an argument reads the request
//!
//! - `Request` passes the request through. It reads the body.
//! - An integer type or `String` is a path value, read from the route
//!   parameter named after the argument through `FromParam`; an `Option` of
//!   one is `None` when its optional parameter is absent.
//! - Any other type is decided by trait, at compile time, through the
//!   generated code: a type that implements `RouteBinding` binds from the
//!   route parameter named after the argument (`post: Post`), and any other
//!   type reads the body through `FromRequest` (a form request). An
//!   `Option` of a type that binds is `None` when its optional parameter is
//!   absent. A generic argument reads the body: the generated code cannot
//!   choose for it.
//!
//! The macro records every argument, with the parameter it reads, its type
//! and its kind, in a `HandlerRecord` it submits through `inventory`, keyed
//! by the function's type. The router reads it wherever it registers the
//! handler and, before the first request, refuses a route whose handler
//! reads a parameter its path does not declare, or reads the body twice. A
//! generic handler has no type to key a record by, so it is not checked
//! before the first request.
//!
//! When one of a handler's arguments may bind, its body starts with a
//! guard: if the route did not plan it but the request carries the route's
//! settings, it hands the route the same description of every argument,
//! and the route binds its concrete arguments as it binds a recorded
//! handler's: in path order, by the route's binding fields, scoped, through
//! the router's binders, with `with_trashed()` and `missing()`. That is a
//! generic handler, a recorded one a closure route or another handler calls
//! with the request, and a `missing()` handler without a plan. On its own
//! route a recorded handler's guard is false.
//!
//! ## A handler inside an `impl` block
//!
//! The record names the function, and an item the macro emits inside an
//! `impl` block cannot name `Self`, so a handler there names its type:
//! `#[handler(Self = Posts)]` on `show` inside `impl Posts`. The record
//! then names `<Posts>::show`, and the build checks that `Posts` is the
//! block's type. Without `Self = Type`, the probe the macro puts in a free
//! handler's body fails the build inside an `impl` block, saying what to
//! write (see `framework/src/routing/handler_site.rs`).
//!
//! ## Order
//!
//! The router binds the route-bound arguments in path order after the
//! route's middleware, before the handler runs, so a missing row answers
//! 404 before the body is read. In the handler, the other arguments are
//! extracted in declaration order. With `#[authorize]`, every path value is
//! extracted first, then one gate check runs per attribute in the order
//! written, then the body is read. That is the order of Laravel's
//! `SubstituteBindings` and `can` middleware ahead of a form request: the
//! check sees the bound model, a missing model is a 404 before the check,
//! and a denied request never reaches validation.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{quote, quote_spanned};
use syn::ext::IdentExt;
use syn::parse::{ParseStream, Parser};
use syn::spanned::Spanned;
use syn::{FnArg, GenericArgument, Ident, ItemFn, Pat, PatIdent, PatType, PathArguments, Type};

use crate::authorize::{AuthorizeSpec, Target, is_authorize_attr, parse_spec};

/// How an argument reads the request, as far as its spelling tells.
enum ArgKind {
    /// `Request`: the request itself.
    Request,
    /// An integer type or `String`: a path value.
    Path,
    /// `Option` of an integer type or `String`: an optional path value.
    OptionalPath(Type),
    /// Any other type: bound when it implements `RouteBinding`, the body
    /// otherwise, decided by the generated code.
    Probe,
    /// `Option` of any other type: an optional binding, or the body.
    OptionalProbe(Type),
    /// A type that names one of the function's type or const parameters:
    /// the body, read through `FromRequest`, even when the type the caller
    /// picks also binds. The generated code cannot choose for it (BIND-015).
    Generic,
}

/// One argument of the handler.
struct Arg<'a> {
    index: usize,
    pat: &'a Pat,
    ty: &'a Type,
    kind: ArgKind,
    /// The route parameter it reads: the name of its single binding.
    name: Option<String>,
}

/// Implementation of the `#[handler]` attribute macro
///
/// ```rust,ignore
/// // No parameters
/// #[handler]
/// pub async fn index() -> Response { ... }
///
/// // Request passthrough
/// #[handler]
/// pub async fn show(req: Request) -> Response { ... }
///
/// // Path value
/// #[handler]
/// pub async fn show(id: i64) -> Response { ... }
///
/// // Route binding, by type
/// #[handler]
/// pub async fn show(post: Post) -> Response { ... }
///
/// // Form request validation
/// #[handler]
/// pub async fn store(form: CreatePost) -> Response { ... }
///
/// // Mixed
/// #[handler]
/// pub async fn update(post: Post, form: UpdatePost) -> Response { ... }
/// ```
pub fn handler_impl(attr: TokenStream, input: TokenStream) -> TokenStream {
    handler_impl_inner(attr.into(), input.into()).into()
}

/// `proc_macro2`-flavoured entry point. The outer `handler_impl` is a thin
/// shim that converts the host `proc_macro::TokenStream`. Splitting the work
/// here lets the unit tests below feed in token streams directly and assert
/// on the rendered output - the host `proc_macro::TokenStream` cannot be
/// constructed outside a real macro-expansion context.
fn handler_impl_inner(attr: TokenStream2, input: TokenStream2) -> TokenStream2 {
    let self_ty = match parse_handler_args(attr) {
        Ok(self_ty) => self_ty,
        Err(e) => return e.to_compile_error(),
    };
    let input_text = input.to_string();
    let mut input_fn: ItemFn = match syn::parse2(input) {
        Ok(f) => f,
        Err(e) => return e.to_compile_error(),
    };

    // `#[authorize]` attributes are applied here, so take them off the
    // function: left on, each would expand on its own.
    let (authorize_attrs, fn_attrs): (Vec<_>, Vec<_>) = std::mem::take(&mut input_fn.attrs)
        .into_iter()
        .partition(is_authorize_attr);
    let mut specs = Vec::with_capacity(authorize_attrs.len());
    for attr in &authorize_attrs {
        match parse_spec(attr) {
            Ok(spec) => specs.push(spec),
            Err(e) => return e.to_compile_error(),
        }
    }

    let fn_vis = &input_fn.vis;
    let fn_name = &input_fn.sig.ident;
    let fn_generics = &input_fn.sig.generics;
    let fn_output = &input_fn.sig.output;
    let fn_block = &input_fn.block;

    let is_async = input_fn.sig.asyncness.is_some();
    let async_token = if is_async {
        quote! { async }
    } else {
        quote! {}
    };

    if let Some(first) = authorize_attrs.first()
        && !is_async
    {
        return syn::Error::new_spanned(
            first,
            "#[authorize] needs an `async fn` handler: the check awaits the gate",
        )
        .to_compile_error();
    }

    // Classify every argument by its spelling.
    let generic_names = generic_param_names(fn_generics);
    let mut args = Vec::with_capacity(input_fn.sig.inputs.len());
    for (index, param) in input_fn.sig.inputs.iter().enumerate() {
        let pat_type: &PatType = match param {
            FnArg::Typed(pt) => pt,
            FnArg::Receiver(_) => {
                return syn::Error::new_spanned(
                    param,
                    "#[handler] does not support methods with self receiver",
                )
                .to_compile_error();
            }
        };
        let kind = if !bound_by_spelling(&pat_type.ty) && names_any(&pat_type.ty, &generic_names) {
            ArgKind::Generic
        } else {
            classify_param_type(&pat_type.ty)
        };
        let name = extract_param_name(&pat_type.pat);
        if name.is_none() && (reads_route(&kind) || bound_by_spelling(&pat_type.ty)) {
            return syn::Error::new_spanned(
                &pat_type.pat,
                "#[handler] reads a route parameter named after the \
                 parameter's binding, and this pattern has no single \
                 binding to name it by: write `id: i64`, or \
                 `RouteParam(user): RouteParam<T>`",
            )
            .to_compile_error();
        }
        args.push(Arg {
            index,
            pat: &pat_type.pat,
            ty: &pat_type.ty,
            kind,
            name,
        });
    }

    // Two `Request` arguments both read the body, which the spelling
    // already tells; any other pair is refused at startup from the record.
    let requests: Vec<&Arg> = args
        .iter()
        .filter(|arg| matches!(arg.kind, ArgKind::Request))
        .collect();
    if requests.len() > 1 {
        return syn::Error::new_spanned(
            requests[requests.len() - 1].pat,
            "#[handler] supports at most one argument that reads the request \
             body, and `Request` reads it: combining two would read the body \
             twice. Split the work across separate handlers.",
        )
        .to_compile_error();
    }
    if !is_async
        && let Some(arg) = args.iter().find(|arg| {
            matches!(
                arg.kind,
                ArgKind::Probe | ArgKind::OptionalProbe(_) | ArgKind::Generic
            )
        })
    {
        return syn::Error::new_spanned(
            arg.ty,
            "#[handler] needs an `async fn` to bind this argument or read the \
             request body into it",
        )
        .to_compile_error();
    }

    let checks = match authorize_checks(&specs, &args, fn_name) {
        Ok(checks) => checks,
        Err(e) => return e.to_compile_error(),
    };
    let targets: Vec<usize> = specs
        .iter()
        .filter_map(|spec| match &spec.target {
            Target::Param(name) => args
                .iter()
                .find(|arg| arg.name.as_deref() == Some(name.to_string().as_str()))
                .map(|arg| arg.index),
            Target::Type(_) => None,
        })
        .collect();

    // The record the router reads. A generic handler has no type to key it
    // by, and is not checked before the first request. A handler that may
    // bind hands the route the same description of its arguments at the
    // request, when the route planned nothing for it.
    let record = if has_type_generics(fn_generics) {
        generic_record_items(fn_name, self_ty.as_ref(), &args, &generic_names)
    } else {
        record_items(fn_name, self_ty.as_ref(), &args)
    };
    let unplanned = if args.is_empty() {
        TokenStream2::new()
    } else {
        unplanned_binding(
            fn_name,
            self_ty.as_ref(),
            has_type_generics(fn_generics),
            fn_output,
            &args,
        )
    };
    let (site_check, site_const) = handler_site(fn_name, self_ty.as_ref(), &input_text, record);

    // Pass 1: every argument that may bind takes its bound value, the
    // `#[authorize]` targets among them binding into their patterns.
    let mut bind_pass = Vec::new();
    // Pass 2: path values, then checks, then the body - or, without
    // `#[authorize]`, every remaining argument in declaration order.
    let mut path_pass = Vec::new();
    let mut body_pass = Vec::new();
    // Whether the passes call a probe, and so need its trait in scope, and
    // whether they change the input, and so need it `mut`: the body never
    // declares what it does not use.
    let mut calls_probe = false;
    let mut changes_input = false;
    for arg in &args {
        let pat = arg.pat;
        let ty = arg.ty;
        let index = arg.index;
        let name = arg.name.as_deref().unwrap_or("");
        let slot = quote::format_ident!("__suprnova_arg_{}", index);
        let is_target = targets.contains(&index);
        // A path value reads the input; every other argument takes from it.
        changes_input |= !matches!(arg.kind, ArgKind::Path | ArgKind::OptionalPath(_));
        calls_probe |= !is_target && matches!(arg.kind, ArgKind::Probe | ArgKind::OptionalProbe(_));
        match &arg.kind {
            ArgKind::Probe if is_target => bind_pass.push(quote! {
                let #pat: #ty = ::suprnova::routing::__authorize_target::<#ty>(
                    &mut __suprnova_input, #index, #name,
                ).await?;
            }),
            ArgKind::OptionalProbe(_) if is_target => bind_pass.push(quote! {
                let #pat: #ty = ::suprnova::routing::__authorize_target::<#ty>(
                    &mut __suprnova_input, #index, #name,
                ).await?;
            }),
            ArgKind::Probe => {
                bind_pass.push(quote! {
                    let #slot: ::core::option::Option<#ty> =
                        (&&::suprnova::routing::__ArgProbe::<#ty>::new())
                            .__bind(&mut __suprnova_input, #index, #name)
                            .await?;
                });
                let take = quote! {
                    let #pat: #ty = match #slot {
                        ::core::option::Option::Some(bound) => bound,
                        ::core::option::Option::None => {
                            (&&::suprnova::routing::__ArgProbe::<#ty>::new())
                                .__read_body(&mut __suprnova_input)
                                .await?
                        }
                    };
                };
                if checks.is_empty() {
                    path_pass.push(take);
                } else {
                    body_pass.push(take);
                }
            }
            ArgKind::OptionalProbe(inner) => {
                bind_pass.push(quote! {
                    let #slot: ::core::option::Option<#ty> =
                        (&&::suprnova::routing::__OptionalArgProbe::<#inner>::new())
                            .__bind(&mut __suprnova_input, #index, #name)
                            .await?;
                });
                let take = quote! {
                    let #pat: #ty = match #slot {
                        ::core::option::Option::Some(bound) => bound,
                        ::core::option::Option::None => {
                            (&&::suprnova::routing::__OptionalArgProbe::<#inner>::new())
                                .__read_body(&mut __suprnova_input)
                                .await?
                        }
                    };
                };
                if checks.is_empty() {
                    path_pass.push(take);
                } else {
                    body_pass.push(take);
                }
            }
            ArgKind::Path => path_pass.push(quote! {
                let #pat: #ty = __suprnova_input.path::<#ty>(#name)?;
            }),
            ArgKind::OptionalPath(inner) => path_pass.push(quote! {
                let #pat: #ty = __suprnova_input.optional_path::<#inner>(#name)?;
            }),
            ArgKind::Request => {
                let take = quote! {
                    let #pat: #ty = __suprnova_input.request()?;
                };
                if checks.is_empty() {
                    path_pass.push(take);
                } else {
                    body_pass.push(take);
                }
            }
            ArgKind::Generic => {
                let take = quote! {
                    let #pat: #ty = <#ty as ::suprnova::FromRequest>::from_request(
                        __suprnova_input.request()?,
                    )
                    .await?;
                };
                if checks.is_empty() {
                    path_pass.push(take);
                } else {
                    body_pass.push(take);
                }
            }
        }
    }

    let input_binding = if args.is_empty() {
        quote! { let _ = __suprnova_req; }
    } else if changes_input {
        quote! {
            let mut __suprnova_input = ::suprnova::routing::HandlerInput::new(__suprnova_req);
        }
    } else {
        quote! {
            let __suprnova_input = ::suprnova::routing::HandlerInput::new(__suprnova_req);
        }
    };
    let probes = if calls_probe {
        quote! { use ::suprnova::routing::__ArgSource as _; }
    } else {
        TokenStream2::new()
    };

    quote! {
        #(#fn_attrs)*
        #fn_vis #async_token fn #fn_name #fn_generics(__suprnova_req: ::suprnova::Request) #fn_output {
            #site_check
            #probes
            let __suprnova_precognitive = __suprnova_req.is_precognitive();
            #input_binding
            #unplanned
            #(#bind_pass)*
            #(#path_pass)*
            #(#checks)*
            #(#body_pass)*
            if __suprnova_precognitive {
                return ::core::result::Result::Err(
                    ::suprnova::FrameworkError::PrecognitionSuccess.into(),
                );
            }
            #fn_block
        }

        #site_const
    }
}

/// How a misuse of `#[handler]`'s argument is answered.
const HANDLER_ARGS_USAGE: &str = "#[handler] takes `Self = <Type>` on a function inside an \
                                  `impl` block: `#[handler(Self = Posts)]` inside `impl Posts`";

/// The attribute's argument: `Self = Type` for a handler inside an `impl`
/// block, or nothing for a free function. An argument that does not start
/// with `Self` is ignored, as every argument was before `Self = Type`
/// existed, so a handler written then keeps compiling (BIND-003).
fn parse_handler_args(attr: TokenStream2) -> syn::Result<Option<Type>> {
    let starts_with_self = matches!(
        attr.clone().into_iter().next(),
        Some(proc_macro2::TokenTree::Ident(ident)) if ident == "Self"
    );
    if !starts_with_self {
        return Ok(None);
    }
    let parser = |input: ParseStream| -> syn::Result<Type> {
        input.parse::<syn::Token![Self]>()?;
        input.parse::<syn::Token![=]>()?;
        let ty: Type = input.parse()?;
        if !input.is_empty() {
            return Err(input.error(HANDLER_ARGS_USAGE));
        }
        Ok(ty)
    };
    let span = attr.span();
    let ty = parser
        .parse2(attr)
        .map_err(|e| syn::Error::new(e.span(), HANDLER_ARGS_USAGE))?;
    if names_self(&ty) {
        return Err(syn::Error::new(
            span,
            "#[handler(Self = ...)] names the type of the `impl` block, as in \
             `#[handler(Self = Posts)]`: the record is emitted where `Self` is \
             not that type",
        ));
    }
    Ok(Some(ty))
}

/// Whether `ty` mentions `Self` anywhere.
fn names_self(ty: &Type) -> bool {
    fn walk(tokens: TokenStream2) -> bool {
        tokens.into_iter().any(|token| match token {
            proc_macro2::TokenTree::Ident(ident) => ident == "Self",
            proc_macro2::TokenTree::Group(group) => walk(group.stream()),
            _ => false,
        })
    }
    walk(quote!(#ty))
}

/// FNV-1a over `text`: a name suffix that is the same on every build.
fn stable_hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The statement the handler's body starts with, and the const beside the
/// handler that holds its record.
///
/// The const's name carries a hash of the handler's tokens, so a parent
/// module's const reached through a glob import never stands in for a
/// child's. With `Self = Type`, the const is an associated const: it
/// checks that `Type` is the block's type, and the body uses it. Without,
/// the body matches the const by its bare name, which reaches it only when
/// the handler is free; inside an `impl` block the probe fails the build
/// and says to write `Self = Type` (see `__FreeHandler`).
fn handler_site(
    fn_name: &Ident,
    self_ty: Option<&Type>,
    input_text: &str,
    record: TokenStream2,
) -> (TokenStream2, TokenStream2) {
    let site = quote::format_ident!(
        "__SUPRNOVA_HANDLER_{}_{:016X}",
        fn_name.unraw().to_string().to_uppercase(),
        stable_hash(input_text)
    );
    match self_ty {
        Some(self_ty) => {
            let same = quote_spanned! {self_ty.span()=>
                ::suprnova::routing::__handler_self::<Self, #self_ty>();
            };
            (
                quote! { let () = Self::#site; },
                quote! {
                    #[doc(hidden)]
                    const #site: () = {
                        #same
                        #record
                    };
                },
            )
        }
        None => (
            quote! {
                match &(::suprnova::routing::__FreeHandler,) {
                    (#site,) => ::suprnova::routing::__in_free_context(#site),
                }
            },
            quote! {
                #[doc(hidden)]
                const #site: ::suprnova::routing::__FreeHandler = {
                    #record
                    ::suprnova::routing::__FreeHandler
                };
            },
        ),
    }
}

/// The names of the function's type and const parameters.
fn generic_param_names(generics: &syn::Generics) -> Vec<Ident> {
    generics
        .params
        .iter()
        .filter_map(|param| match param {
            syn::GenericParam::Type(param) => Some(param.ident.clone()),
            syn::GenericParam::Const(param) => Some(param.ident.clone()),
            syn::GenericParam::Lifetime(_) => None,
        })
        .collect()
}

/// Whether `ty` names one of `names` anywhere in its tokens: `T`,
/// `Option<T>`, `Form<T>`.
fn names_any(ty: &Type, names: &[Ident]) -> bool {
    fn walk(tokens: TokenStream2, names: &[Ident]) -> bool {
        tokens.into_iter().any(|token| match token {
            proc_macro2::TokenTree::Ident(ident) => names.contains(&ident),
            proc_macro2::TokenTree::Group(group) => walk(group.stream(), names),
            _ => false,
        })
    }
    !names.is_empty() && walk(quote!(#ty), names)
}

/// Whether the function has type or const generics. A generic function has
/// no single type to key its record by.
fn has_type_generics(generics: &syn::Generics) -> bool {
    generics
        .params
        .iter()
        .any(|param| !matches!(param, syn::GenericParam::Lifetime(_)))
}

/// Whether an argument of this kind reads a route parameter by name, for
/// certain: a path value. A probed argument may read the body instead, so
/// its name is only needed when it binds.
fn reads_route(kind: &ArgKind) -> bool {
    matches!(kind, ArgKind::Path | ArgKind::OptionalPath(_))
}

/// Whether the spelling alone says the argument binds from the route:
/// `RouteParam<T>`, or a path ending in `Model` (`user::Model`). Such an
/// argument needs a single binding to name its parameter by.
fn bound_by_spelling(ty: &Type) -> bool {
    match ty {
        Type::Path(type_path) => {
            let segments = &type_path.path.segments;
            is_route_param(ty)
                || (segments.len() >= 2
                    && segments.last().is_some_and(|last| last.ident == "Model"))
        }
        _ => false,
    }
}

/// A type as the record shows it: its tokens, without the spaces `quote`
/// puts between them.
fn type_text(ty: &Type) -> String {
    quote!(#ty).to_string().replace(' ', "")
}

/// The handler as a record or a refusal names it: `show`, or `Posts::show`
/// for one inside an `impl` block, after its module's path.
fn handler_display(fn_name: &Ident, self_ty: Option<&Type>) -> String {
    match self_ty {
        Some(self_ty) => format!("{}::{fn_name}", type_last_segment(self_ty)),
        None => fn_name.to_string(),
    }
}

/// The start of a handler's body when an argument may bind. When the
/// route planned nothing for the handler and handed it the route's
/// settings, the route plans the handler's arguments, described as a record
/// describes them, keyed by a type only this handler declares, and binds
/// the concrete ones as it binds a recorded handler's (BIND-004).
///
/// A binding that finds nothing answers the route's `missing()` response,
/// or the 404, in place of the handler. A handler that returns `Response`
/// returns that response as it is; any other return type, which cannot
/// carry it, gets the 404's error through `?`. The return type is named to
/// tell them apart, so one the body cannot name (`impl Trait`) always gets
/// the error.
fn unplanned_binding(
    fn_name: &Ident,
    self_ty: Option<&Type>,
    generic: bool,
    output: &syn::ReturnType,
    args: &[Arg],
) -> TokenStream2 {
    let display = handler_display(fn_name, self_ty);
    let entries = arg_entries(args);
    let binds = args
        .iter()
        .any(|arg| matches!(arg.kind, ArgKind::Probe | ArgKind::OptionalProbe(_)));
    // A probe's entry calls its trait.
    let probes = if binds {
        quote! { use ::suprnova::routing::__ArgSource as _; }
    } else {
        TokenStream2::new()
    };
    // The handler function's type, which tells the route's own handler
    // apart; a generic function's type cannot be named without its
    // arguments, and a generic handler is never the route's own.
    let handler_type = if generic {
        quote! { ::core::option::Option::None }
    } else {
        // Spanned at the attribute, as the record's name is: inside an
        // `impl` block without `Self = Type` it does not resolve, and the
        // error is the record's, at `#[handler]`.
        let mut fn_ref = fn_name.clone();
        fn_ref.set_span(Span::call_site());
        let handler = match self_ty {
            Some(self_ty) => quote! { <#self_ty>::#fn_ref },
            None => quote! { #fn_ref },
        };
        quote! { ::core::option::Option::Some(::suprnova::routing::__type_id_of(&#handler)) }
    };
    // The plan's key: a type only this handler declares, and the name of
    // the closure that describes its arguments, which carries a generic
    // handler's arguments, so each instantiation plans its own.
    let key = quote! {
        (
            ::std::any::TypeId::of::<__SuprnovaUnplannedHandler>(),
            ::std::any::type_name_of_val(&__suprnova_args),
        )
    };
    let bind = |answers_missing: TokenStream2| {
        quote! {
            __suprnova_input
                .__bind_unplanned(
                    #key,
                    ::core::concat!(::core::module_path!(), "::", #display),
                    __suprnova_args,
                    #answers_missing,
                )
                .await?
        }
    };
    let output = match output {
        syn::ReturnType::Type(_, ty) if !names_impl(ty) => Some(ty),
        _ => None,
    };
    let answer = match output {
        // Nothing may bind: the route's checks still apply (BIND-004).
        _ if !binds => quote! {
            __suprnova_input.__check_unplanned(
                #key,
                ::core::concat!(::core::module_path!(), "::", #display),
                __suprnova_args,
            )?;
        },
        Some(output) => {
            let bind = bind(quote! { (&&__suprnova_output).__answers_missing() });
            quote! {
                use ::suprnova::routing::__OutputSource as _;
                let __suprnova_output = ::suprnova::routing::__HandlerOutput::<#output>::new();
                if let ::core::option::Option::Some(__suprnova_answer) = #bind {
                    return (&&__suprnova_output).__missing_answer(__suprnova_answer);
                }
            }
        }
        None => {
            let bind = bind(quote! { false });
            quote! { let _ = #bind; }
        }
    };
    quote! {
        if __suprnova_input.__binds_unplanned(#handler_type) {
            struct __SuprnovaUnplannedHandler;
            let __suprnova_args = || {
                #probes
                ::std::vec![#(#entries),*]
            };
            #answer
        }
    }
}

/// Whether `ty` contains `impl Trait`, which the body cannot name.
fn names_impl(ty: &Type) -> bool {
    fn walk(tokens: TokenStream2) -> bool {
        tokens.into_iter().any(|token| match token {
            proc_macro2::TokenTree::Ident(ident) => ident == "impl",
            proc_macro2::TokenTree::Group(group) => walk(group.stream()),
            _ => false,
        })
    }
    walk(quote!(#ty))
}

/// One `HandlerArg` per argument, in declaration order. A probed
/// argument's entry calls its probe, so the trait must be in scope.
fn arg_entries<'b, 'a: 'b>(args: impl IntoIterator<Item = &'b Arg<'a>>) -> Vec<TokenStream2> {
    args.into_iter()
        .map(|arg| {
            let ty = arg.ty;
            let ty_text = type_text(ty);
            let name = arg.name.as_deref().unwrap_or("_");
            match &arg.kind {
                ArgKind::Request | ArgKind::Generic => quote! {
                    ::suprnova::routing::HandlerArg::body(#name, #ty_text)
                },
                ArgKind::Path => quote! {
                    ::suprnova::routing::HandlerArg::path_value(#name, #ty_text, false)
                },
                ArgKind::OptionalPath(_) => quote! {
                    ::suprnova::routing::HandlerArg::path_value(#name, #ty_text, true)
                },
                ArgKind::Probe => quote! {
                    (&&::suprnova::routing::__ArgProbe::<#ty>::new()).__record(#name, #ty_text)
                },
                ArgKind::OptionalProbe(inner) => quote! {
                    (&&::suprnova::routing::__OptionalArgProbe::<#inner>::new())
                        .__record(#name, #ty_text)
                },
            }
        })
        .collect()
}

/// The `inventory` record of a generic handler: its path, which the router
/// finds it by from an instantiation's type name, and the arguments whose
/// type does not depend on the instantiation, so a route whose own handler
/// it is can scope a child for the other handlers planned against it
/// (BIND-004). An argument bound with a type that names a type parameter
/// (`RouteParam<T>`) is listed by its parameter alone. A free function's
/// path is its module's and its name; one inside an `impl` block is its
/// type's name and its own.
fn generic_record_items(
    fn_name: &Ident,
    self_ty: Option<&Type>,
    args: &[Arg],
    generic_names: &[Ident],
) -> TokenStream2 {
    let display = handler_display(fn_name, self_ty);
    let name = fn_name.unraw().to_string();
    let path = match self_ty {
        Some(self_ty) => quote! {
            ::std::format!("{}::{}", ::std::any::type_name::<#self_ty>(), #name)
        },
        None => quote! {
            ::std::string::String::from(::core::concat!(::core::module_path!(), "::", #name))
        },
    };
    let (unresolved, resolved): (Vec<&Arg>, Vec<&Arg>) = args.iter().partition(|arg| {
        matches!(arg.kind, ArgKind::Probe | ArgKind::OptionalProbe(_))
            && names_any(arg.ty, generic_names)
    });
    let unresolved = unresolved
        .iter()
        .map(|arg| arg.name.clone().unwrap_or_else(|| "_".to_owned()));
    let probes = if resolved
        .iter()
        .any(|arg| matches!(arg.kind, ArgKind::Probe | ArgKind::OptionalProbe(_)))
    {
        quote! { use ::suprnova::routing::__ArgSource as _; }
    } else {
        TokenStream2::new()
    };
    let entries = arg_entries(resolved);
    quote! {
        fn __suprnova_generic_handler_path() -> ::std::string::String {
            #path
        }
        fn __suprnova_generic_handler_args() -> ::std::vec::Vec<::suprnova::routing::HandlerArg> {
            #probes
            ::std::vec![#(#entries),*]
        }
        ::suprnova::inventory::submit! {
            ::suprnova::routing::GenericHandlerRecord::new(
                __suprnova_generic_handler_path,
                #display,
                ::core::module_path!(),
                __suprnova_generic_handler_args,
                &[#(#unresolved),*],
            )
        }
    }
}

/// The `inventory` record of the handler: its type, its name and every
/// argument, for the router's startup checks (BIND-004). A free function
/// is named by its name, one inside an `impl` block through its type.
fn record_items(fn_name: &Ident, self_ty: Option<&Type>, args: &[Arg]) -> TokenStream2 {
    // Spanned at the attribute: inside an `impl` block without `Self =
    // Type` the name does not resolve, and the error points at `#[handler]`.
    let mut fn_ref = fn_name.clone();
    fn_ref.set_span(Span::call_site());
    let handler = match self_ty {
        Some(self_ty) => quote! { <#self_ty>::#fn_ref },
        None => quote! { #fn_ref },
    };
    let display = handler_display(fn_name, self_ty);
    // A probe's record comes through its trait, imported only when an
    // entry calls it.
    let probes = if args
        .iter()
        .any(|arg| matches!(arg.kind, ArgKind::Probe | ArgKind::OptionalProbe(_)))
    {
        quote! { use ::suprnova::routing::__ArgSource as _; }
    } else {
        TokenStream2::new()
    };
    let entries = arg_entries(args);
    quote! {
        fn __suprnova_handler_type() -> ::std::any::TypeId {
            ::suprnova::routing::__type_id_of(&#handler)
        }
        fn __suprnova_handler_args() -> ::std::vec::Vec<::suprnova::routing::HandlerArg> {
            #probes
            ::std::vec![#(#entries),*]
        }
        ::suprnova::inventory::submit! {
            ::suprnova::routing::HandlerRecord::new(
                __suprnova_handler_type,
                #display,
                ::core::module_path!(),
                __suprnova_handler_args,
            )
        }
    }
}

/// The last segment of a type path, `Posts` for `crate::controllers::Posts`,
/// as the record shows it; any other type as written.
fn type_last_segment(ty: &Type) -> String {
    match ty {
        Type::Path(type_path) if type_path.qself.is_none() => type_path
            .path
            .segments
            .last()
            .map(|segment| quote!(#segment).to_string().replace(' ', ""))
            .unwrap_or_else(|| type_text(ty)),
        _ => type_text(ty),
    }
}

/// Emit one gate check per `#[authorize]`, in the order written.
///
/// A parameter target must be the binding of a handler argument the route
/// supplies: a path value (`id: i64`), or an argument whose type implements
/// `RouteBinding`, which the generated code requires at compile time. A
/// `Request` target is a compile error spanned on the name in the
/// attribute.
fn authorize_checks(
    specs: &[AuthorizeSpec],
    args: &[Arg],
    fn_name: &Ident,
) -> syn::Result<Vec<TokenStream2>> {
    specs
        .iter()
        .map(|spec| {
            let ability = &spec.ability;
            let name = match &spec.target {
                Target::Type(ty) => {
                    return Ok(quote! {
                        ::suprnova::authorization::__authorize_handler_type::<#ty>(#ability).await?;
                    });
                }
                Target::Param(name) => name,
            };
            let found = args.iter().find_map(|arg| {
                let binding = route_binding(arg.pat)?;
                (binding.ident == *name).then_some((arg, binding))
            });
            let Some((arg, binding)) = found else {
                return Err(syn::Error::new_spanned(
                    name,
                    format!(
                        "#[authorize] names `{name}`, but `{fn_name}` takes no parameter \
                         named `{name}`; name a parameter the route binds, written \
                         as `{name}: Model` or `{name}: i64`"
                    ),
                ));
            };
            match &arg.kind {
                ArgKind::Request | ArgKind::Generic => {
                    return Err(syn::Error::new_spanned(
                        name,
                        format!(
                            "#[authorize] cannot check `{name}`: it reads the request body, \
                             and the check runs before the body is read; name a route-bound \
                             model or a path parameter"
                        ),
                    ));
                }
                ArgKind::OptionalPath(_) => {
                    return Err(syn::Error::new_spanned(
                        name,
                        format!(
                            "#[authorize] cannot check `{name}`: an optional path value may be \
                             absent; name a route-bound model or a path parameter"
                        ),
                    ));
                }
                _ => {}
            }
            // A `ref` binding already holds a reference.
            let value = if binding.by_ref.is_some() {
                quote! { #name }
            } else {
                quote! { &#name }
            };
            // A `RouteParam<M>` is checked as the `M` inside it, the type
            // its policy is registered for. A binding of the whole wrapper
            // derefs to it; `RouteParam(post)` already binds the `M`.
            let whole = matches!(arg.pat, Pat::Ident(_));
            let resource = if whole && is_route_param(arg.ty) {
                quote! { ::core::ops::Deref::deref(#value) }
            } else {
                value
            };
            Ok(quote! {
                ::suprnova::authorization::__authorize_handler(#ability, #resource).await?;
            })
        })
        .collect()
}

/// The route parameter an argument reads: the name of its binding (see
/// [`route_binding`]), or `_` for a wildcard. `None` when the pattern has
/// no single binding to name it by.
fn extract_param_name(pat: &Pat) -> Option<String> {
    match pat {
        Pat::Wild(_) => Some("_".to_string()),
        _ => route_binding(pat).map(|binding| binding.ident.to_string()),
    }
}

/// The one binding of a parameter pattern: the identifier of `user: T`, or
/// of the single field in a tuple-struct pattern such as
/// `RouteParam(user): RouteParam<T>`, which binds the model inside the
/// wrapper. `None` for any other pattern.
fn route_binding(pat: &Pat) -> Option<&PatIdent> {
    match pat {
        Pat::Ident(binding) => Some(binding),
        Pat::TupleStruct(tuple) if tuple.elems.len() == 1 => match tuple.elems.first() {
            Some(Pat::Ident(binding)) => Some(binding),
            _ => None,
        },
        _ => None,
    }
}

/// Classify the parameter type by its spelling.
fn classify_param_type(ty: &Type) -> ArgKind {
    let Type::Path(type_path) = ty else {
        return ArgKind::Probe;
    };
    if type_path.qself.is_some() {
        return ArgKind::Probe;
    }
    let segments = &type_path.path.segments;

    // `Request`, `suprnova::Request`, `::suprnova::Request`.
    if (segments.len() == 1 && segments[0].ident == "Request")
        || (segments.len() == 2
            && segments[0].ident == "suprnova"
            && segments[1].ident == "Request")
    {
        return ArgKind::Request;
    }

    if segments.len() == 1
        && segments[0].arguments.is_empty()
        && is_primitive_type_name(&segments[0].ident.to_string())
    {
        return ArgKind::Path;
    }

    if let Some(inner) = option_inner(type_path) {
        return if is_primitive_type(inner) {
            ArgKind::OptionalPath(inner.clone())
        } else {
            ArgKind::OptionalProbe(inner.clone())
        };
    }

    ArgKind::Probe
}

/// The `T` of `Option<T>`, spelled `Option`, `std::option::Option` or
/// `core::option::Option`.
fn option_inner(type_path: &syn::TypePath) -> Option<&Type> {
    let segments = &type_path.path.segments;
    let last = segments.last()?;
    let spelled = match segments.len() {
        1 => true,
        3 => {
            (segments[0].ident == "std" || segments[0].ident == "core")
                && segments[1].ident == "option"
        }
        _ => false,
    };
    if !spelled || last.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(generics) = &last.arguments else {
        return None;
    };
    match generics.args.first() {
        Some(GenericArgument::Type(inner)) if generics.args.len() == 1 => Some(inner),
        _ => None,
    }
}

/// Whether `ty` is a primitive path type by its spelling.
fn is_primitive_type(ty: &Type) -> bool {
    matches!(ty, Type::Path(path)
        if path.qself.is_none()
            && path.path.segments.len() == 1
            && path.path.segments[0].arguments.is_empty()
            && is_primitive_type_name(&path.path.segments[0].ident.to_string()))
}

/// Whether `ty` is the binding wrapper `RouteParam<M>`.
fn is_route_param(ty: &Type) -> bool {
    matches!(ty, Type::Path(type_path)
        if type_path.path.segments.last().is_some_and(|last| last.ident == "RouteParam"))
}

/// Check if a type name is a primitive that should use FromParam
fn is_primitive_type_name(name: &str) -> bool {
    matches!(
        name,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "isize"
            | "String"
    )
}

#[cfg(test)]
mod tests {
    //! Macro-expansion tests for `#[handler]`.
    //!
    //! Each test renders `handler_impl_inner` and reads the tokens: what an
    //! argument becomes, the order the extractions run in, and the record
    //! the router checks at startup. Behaviour through a real router lives
    //! in `framework/tests/routing/route_binding/`.
    use super::*;
    use quote::quote;

    /// Render `handler_impl_inner` against a function. A rejection renders
    /// a `compile_error! { ... }` with the message.
    fn expansion(src: proc_macro2::TokenStream) -> String {
        handler_impl_inner(proc_macro2::TokenStream::new(), src).to_string()
    }

    /// Render `handler_impl_inner` with the attribute's arguments `attr`.
    fn expansion_with(attr: proc_macro2::TokenStream, src: proc_macro2::TokenStream) -> String {
        handler_impl_inner(attr, src).to_string()
    }

    /// Byte offset of `needle` in `out`, failing the test when it is absent.
    fn position(out: &str, needle: &str) -> usize {
        out.find(needle)
            .unwrap_or_else(|| panic!("`{needle}` missing from expansion:\n{out}"))
    }

    /// Byte offset of the last `needle` in `out`.
    fn last_position(out: &str, needle: &str) -> usize {
        out.rfind(needle)
            .unwrap_or_else(|| panic!("`{needle}` missing from expansion:\n{out}"))
    }

    // ── What each argument becomes ───────────────────────────────────────────

    #[test]
    fn bind_015_a_type_alone_is_probed_for_route_binding() {
        let out = expansion(quote! {
            pub async fn show(post: Post) -> Response { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(
            out.contains("__ArgProbe :: < Post > :: new ()) . __bind (& mut __suprnova_input , 0usize , \"post\")"),
            "`post: Post` must take its bound value from the parameter `post`; got:\n{out}"
        );
    }

    #[test]
    fn bind_015_a_primitive_stays_a_path_value() {
        let out = expansion(quote! {
            pub async fn show(id: i64, slug: String) -> Response { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(out.contains("path :: < i64 > (\"id\")"), "got:\n{out}");
        assert!(out.contains("path :: < String > (\"slug\")"), "got:\n{out}");
        assert!(!out.contains("__ArgProbe"), "got:\n{out}");
    }

    #[test]
    fn bind_015_an_option_binds_none_when_its_parameter_is_absent() {
        let out = expansion(quote! {
            pub async fn show(post: Option<Post>, page: Option<u32>) -> Response { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(
            out.contains("__OptionalArgProbe :: < Post >"),
            "an `Option<Post>` must probe `Post`; got:\n{out}"
        );
        assert!(
            out.contains("optional_path :: < u32 > (\"page\")"),
            "an `Option<u32>` must be an optional path value; got:\n{out}"
        );
    }

    #[test]
    fn bind_015_a_generic_argument_reads_the_body_and_is_never_probed() {
        // `T` may implement `RouteBinding` too; it still reads the body.
        let out = expansion(quote! {
            pub async fn store<T: FromRequest + RouteBinding>(id: i64, form: Option<T>, thing: T)
                -> Response { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(
            out.contains("< Option < T > as :: suprnova :: FromRequest > :: from_request"),
            "`form: Option<T>` must read the body; got:\n{out}"
        );
        assert!(
            out.contains("< T as :: suprnova :: FromRequest > :: from_request"),
            "`thing: T` must read the body; got:\n{out}"
        );
        assert!(!out.contains("__ArgProbe"), "got:\n{out}");
        assert!(!out.contains("__OptionalArgProbe"), "got:\n{out}");
        assert!(out.contains("path :: < i64 > (\"id\")"), "got:\n{out}");
    }

    #[test]
    fn bind_015_a_generic_authorize_target_is_a_compile_error() {
        let out = expansion(quote! {
            #[authorize("update", form)]
            pub async fn update<T: FromRequest + RouteBinding>(form: T) -> Response { todo!() }
        });
        assert!(out.contains("compile_error"), "got:\n{out}");
        assert!(out.contains("reads the request body"), "got:\n{out}");
    }

    #[test]
    fn bind_003_existing_binding_forms_still_bind_by_their_binding() {
        // `RouteParam(user)` binds `user` inside the wrapper, and every
        // other spelling reads the parameter named after its binding.
        for param in [
            quote! { RouteParam(user): RouteParam<User> },
            quote! { RouteParam(mut user): RouteParam<User> },
            quote! { suprnova::RouteParam(user): suprnova::RouteParam<User> },
            quote! { user: RouteParam<User> },
            quote! { user: user::Model },
        ] {
            let out = expansion(quote! {
                pub async fn show(#param) -> Response { todo!() }
            });
            assert!(!out.contains("compile_error"), "got:\n{out}");
            assert!(
                out.contains("__bind (& mut __suprnova_input , 0usize , \"user\")"),
                "`{param}` must read the route parameter `user`; got:\n{out}"
            );
        }
    }

    #[test]
    fn route_bound_pattern_without_one_binding_is_a_compile_error() {
        // No single binding to name the route parameter after: reject it
        // rather than read a parameter the route does not have.
        for param in [
            quote! { user::Model { id, name, .. }: user::Model },
            quote! { RouteParam(User { id, .. }): RouteParam<User> },
            quote! { (a, b): i64 },
        ] {
            let out = expansion(quote! {
                pub async fn show(#param) -> Response { todo!() }
            });
            assert!(
                out.contains("compile_error"),
                "`{param}` must be rejected; got:\n{out}"
            );
            assert!(out.contains("route parameter"), "got:\n{out}");
        }
    }

    #[test]
    fn accepts_single_request() {
        let out = expansion(quote! {
            pub async fn show(req: Request) -> Response { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(out.contains("__suprnova_input . request ()"), "got:\n{out}");
    }

    #[test]
    fn accepts_zero_params() {
        let out = expansion(quote! {
            pub async fn index() -> Response { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
    }

    #[test]
    fn rejects_two_request_arguments() {
        let out = expansion(quote! {
            pub async fn show(a: Request, b: Request) -> Response { todo!() }
        });
        assert!(out.contains("compile_error"), "got:\n{out}");
        assert!(out.contains("request body"), "got:\n{out}");
    }

    #[test]
    fn bind_015_two_body_readers_are_left_to_the_startup_check() {
        // The macro cannot tell a form request from a model by spelling, so
        // a pair of body readers compiles; the record lists both and the
        // router refuses the route at startup.
        let out = expansion(quote! {
            pub async fn store(a: CreateUser, b: UpdateUser) -> Response { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert_eq!(out.matches("__read_body").count(), 2, "got:\n{out}");
        assert!(out.contains("__record (\"a\""), "got:\n{out}");
        assert!(out.contains("__record (\"b\""), "got:\n{out}");
    }

    #[test]
    fn a_sync_handler_cannot_probe_an_argument() {
        let out = expansion(quote! {
            pub fn show(post: Post) -> Response { todo!() }
        });
        assert!(out.contains("compile_error"), "got:\n{out}");
        assert!(out.contains("async fn"), "got:\n{out}");
    }

    // ── The record ───────────────────────────────────────────────────────────

    #[test]
    fn bind_004_the_handler_records_every_argument() {
        let out = expansion(quote! {
            pub async fn update(id: i64, post: Post, req: Request) -> Response { todo!() }
        });
        assert!(out.contains("HandlerRecord :: new"), "got:\n{out}");
        assert!(
            out.contains("__type_id_of (& update)"),
            "the record is keyed by the handler function's type; got:\n{out}"
        );
        assert!(
            out.contains("HandlerArg :: path_value (\"id\" , \"i64\" , false)"),
            "got:\n{out}"
        );
        assert!(
            out.contains("__record (\"post\" , \"Post\")"),
            "got:\n{out}"
        );
        assert!(
            out.contains("HandlerArg :: body (\"req\" , \"Request\")"),
            "got:\n{out}"
        );
    }

    #[test]
    fn bind_004_a_generic_handler_has_no_record() {
        let out = expansion(quote! {
            pub async fn show<T: Store>(store: T) -> Response { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(!out.contains(":: HandlerRecord :: new"), "got:\n{out}");
    }

    #[test]
    fn bind_004_a_generic_handler_records_what_its_route_scopes_through() {
        // The router finds it by its path, and reads which parameters it
        // binds as which type; one bound with a type that names a type
        // parameter is named alone.
        let out = expansion(quote! {
            pub async fn show<T: RouteBinding, F: FromRequest>(
                user: User,
                item: RouteParam<T>,
                form: F,
            ) -> Response { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(out.contains("GenericHandlerRecord :: new"), "got:\n{out}");
        assert!(
            out.contains("concat ! (:: core :: module_path ! () , \"::\" , \"show\")"),
            "a free handler's path is its module's and its name; got:\n{out}"
        );
        assert!(
            out.contains("__ArgProbe :: < User > :: new ()) . __record (\"user\" , \"User\")"),
            "got:\n{out}"
        );
        assert!(out.contains("& [\"item\"]"), "got:\n{out}");
        let record = &out[position(&out, "fn __suprnova_generic_handler_args")..];
        assert!(
            !record.contains("RouteParam < T >"),
            "a type that names a type parameter is not described; got:\n{out}"
        );
        // Inside an `impl` block, the path is the type's name and its own.
        let out = expansion_with(
            quote! { Self = Posts },
            quote! { pub async fn store<T: FromRequest>(form: T) -> Response { todo!() } },
        );
        assert!(out.contains("type_name :: < Posts > ()"), "got:\n{out}");
    }

    #[test]
    fn bind_004_a_handler_with_nothing_to_bind_meets_the_routes_checks_at_the_request() {
        // A path value or a body reader under a binder is refused at the
        // request where the route did not plan the handler.
        for src in [
            quote! { pub async fn show(id: i64) -> Response { todo!() } },
            quote! { pub fn show(id: String) -> Response { todo!() } },
            quote! { pub async fn show<T: FromRequest>(id: i64, form: T) -> Response { todo!() } },
        ] {
            let out = expansion(src);
            assert!(!out.contains("compile_error"), "got:\n{out}");
            assert!(
                position(&out, "__binds_unplanned (") < position(&out, "__check_unplanned ("),
                "got:\n{out}"
            );
            assert!(!out.contains("__bind_unplanned"), "got:\n{out}");
        }
        // A handler with no arguments reads nothing.
        let out = expansion(quote! { pub async fn index() -> Response { todo!() } });
        assert!(!out.contains("__binds_unplanned"), "got:\n{out}");
    }

    #[test]
    fn bind_004_a_generic_handler_plans_its_concrete_arguments_against_the_route() {
        // No record to key a startup plan by, so the generated code hands
        // the route every argument at the request, before any binds.
        let out = expansion(quote! {
            pub async fn show<T: FromRequest>(user: User, post: Option<Post>, form: T)
                -> Response { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(!out.contains(":: HandlerRecord :: new"), "got:\n{out}");
        // A generic handler is never the route's own, so it names no type.
        let guard = position(
            &out,
            "__binds_unplanned (:: core :: option :: Option :: None)",
        );
        let plan = position(&out, "__bind_unplanned (");
        assert!(guard < plan, "got:\n{out}");
        // Its instantiations plan apart: the key names the closure that
        // describes the arguments, whose name carries the type arguments.
        assert!(
            out.contains("type_name_of_val (& __suprnova_args)"),
            "got:\n{out}"
        );
        assert!(
            plan < position(&out, "__ArgProbe :: < User > :: new ()) . __bind"),
            "the route plans the arguments before the first binds; got:\n{out}"
        );
        assert!(
            plan < position(&out, "< T as :: suprnova :: FromRequest > :: from_request"),
            "and before the body is read; got:\n{out}"
        );
        assert!(
            out.contains("__ArgProbe :: < User > :: new ()) . __record (\"user\" , \"User\")"),
            "got:\n{out}"
        );
        assert!(
            out.contains(
                "__OptionalArgProbe :: < Post > :: new ()) . __record (\"post\" , \"Option<Post>\")"
            ),
            "got:\n{out}"
        );
        assert!(
            out.contains("HandlerArg :: body (\"form\" , \"T\")"),
            "the generic argument is described as the body reader it is; got:\n{out}"
        );
        // A handler whose miss the route answers returns that answer, by
        // its return type.
        assert!(
            out.contains("__HandlerOutput :: < Response > :: new ()"),
            "got:\n{out}"
        );
        assert!(
            out.contains("return (&& __suprnova_output) . __missing_answer (__suprnova_answer)"),
            "got:\n{out}"
        );
    }

    #[test]
    fn bind_004_a_recorded_handler_plans_at_the_request_only_where_its_route_did_not() {
        // A recorded handler a closure route or another handler calls, or
        // a `missing()` handler, gets the route's settings instead of a
        // plan; on its own route the guard is false and nothing else runs.
        let out = expansion(quote! {
            pub async fn show(id: i64, user: User, form: Form) -> Response { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(out.contains("HandlerRecord :: new"), "got:\n{out}");
        // It names its own type, so the route's own handler that binds
        // nothing does not plan again.
        let guard = position(
            &out,
            "if __suprnova_input . __binds_unplanned (:: core :: option :: Option :: Some \
             (:: suprnova :: routing :: __type_id_of (& show)))",
        );
        assert!(
            guard < position(&out, "__bind_unplanned ("),
            "the plan runs only behind the guard; got:\n{out}"
        );
        assert!(
            position(&out, "__bind_unplanned (")
                < position(&out, "__ArgProbe :: < User > :: new ()) . __bind"),
            "got:\n{out}"
        );
        assert!(
            out.contains("HandlerArg :: path_value (\"id\" , \"i64\" , false)"),
            "every argument is described, as the record describes it; got:\n{out}"
        );
        for src in [
            // Nothing that may bind.
            quote! { pub async fn show(id: i64, req: Request) -> Response { todo!() } },
            quote! { pub async fn show<T: FromRequest>(id: i64, form: T) -> Response { todo!() } },
            quote! { pub async fn show<T: FromRequest>(req: Request) -> Response { todo!() } },
        ] {
            let out = expansion(src);
            assert!(!out.contains("compile_error"), "got:\n{out}");
            assert!(!out.contains("__bind_unplanned"), "got:\n{out}");
        }
        // An `#[authorize]` target binds too.
        let out = expansion(quote! {
            #[authorize("view", post)]
            pub async fn show<T: FromRequest>(post: Post, form: T) -> Response { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(
            position(&out, "__bind_unplanned (") < position(&out, "__authorize_target"),
            "got:\n{out}"
        );
    }

    #[test]
    fn bind_009_only_a_handler_returning_response_returns_the_missing_response() {
        // Another return type cannot carry the route's `missing()`
        // response, so it asks for none and gets the 404's error.
        let out = expansion(quote! {
            pub async fn show(user: User) -> Result<HttpResponse, FrameworkError> { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(
            out.contains(
                "__HandlerOutput :: < Result < HttpResponse , FrameworkError > > :: new ()"
            ),
            "got:\n{out}"
        );
        assert!(out.contains("__answers_missing ()"), "got:\n{out}");
        // A return type the body cannot name asks for none.
        let out = expansion(quote! {
            pub async fn show(user: User) -> Result<impl Display, FrameworkError> { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(!out.contains("__HandlerOutput"), "got:\n{out}");
        assert!(out.contains("__bind_unplanned ("), "got:\n{out}");
        assert!(out.contains(", false ,) . await ?"), "got:\n{out}");
    }

    // ── A handler inside an `impl` block ─────────────────────────────────────

    #[test]
    fn bind_003_self_names_the_type_the_record_is_keyed_by() {
        let out = expansion_with(
            quote! { Self = Posts },
            quote! { pub async fn show(post: Post) -> Response { todo!() } },
        );
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(
            out.contains("__type_id_of (& < Posts > :: show)"),
            "the record names the function through its type; got:\n{out}"
        );
        assert!(
            out.contains("HandlerRecord :: new (__suprnova_handler_type , \"Posts::show\""),
            "the record names the handler `Posts::show`; got:\n{out}"
        );
        assert!(
            out.contains("__handler_self :: < Self , Posts >"),
            "the build checks `Self = Posts` names the impl's type; got:\n{out}"
        );
        assert!(
            out.contains("let () = Self :: __SUPRNOVA_HANDLER_SHOW_"),
            "the handler uses its record, so no unused item warns; got:\n{out}"
        );
        assert!(
            !out.contains("const _ :"),
            "an `impl` block refuses `const _`; got:\n{out}"
        );
    }

    #[test]
    fn bind_003_self_combines_with_every_argument_form_and_authorize() {
        let out = expansion_with(
            quote! { Self = crate::controllers::Posts },
            quote! {
                #[authorize("update", post)]
                pub async fn update(
                    id: i64,
                    post: Post,
                    wrapped: RouteParam<Post>,
                    bare: post::Model,
                    page: Option<u32>,
                    form: UpdatePost,
                ) -> Response { todo!() }
            },
        );
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(
            out.contains("__type_id_of (& < crate :: controllers :: Posts > :: update)"),
            "got:\n{out}"
        );
        assert!(out.contains("\"Posts::update\""), "got:\n{out}");
        assert!(
            out.contains("__authorize_handler (\"update\""),
            "got:\n{out}"
        );
        for record in [
            "HandlerArg :: path_value (\"id\" , \"i64\" , false)",
            "__record (\"post\" , \"Post\")",
            "__record (\"wrapped\" , \"RouteParam<Post>\")",
            "__record (\"bare\" , \"post::Model\")",
            "HandlerArg :: path_value (\"page\" , \"Option<u32>\" , true)",
            "__record (\"form\" , \"UpdatePost\")",
        ] {
            assert!(out.contains(record), "missing `{record}`; got:\n{out}");
        }

        // A sync handler and a generic one take `Self` too; the generic one
        // still has no record.
        let out = expansion_with(
            quote! { Self = Posts },
            quote! { pub fn ping(id: i64) -> Response { todo!() } },
        );
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(out.contains("< Posts > :: ping"), "got:\n{out}");
        let out = expansion_with(
            quote! { Self = Posts },
            quote! { pub async fn store<T: FromRequest>(form: T) -> Response { todo!() } },
        );
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(!out.contains(":: HandlerRecord :: new"), "got:\n{out}");
        assert!(
            out.contains("__handler_self :: < Self , Posts >"),
            "got:\n{out}"
        );
    }

    #[test]
    fn bind_003_a_free_handler_proves_it_is_outside_an_impl_block() {
        let out = expansion(quote! {
            pub async fn show(post: Post) -> Response { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        // The probe const is in scope by its bare name only outside an
        // `impl` block, where the pattern matches it; inside one the name
        // binds a reference and the free-function check fails.
        let probe = position(&out, "const __SUPRNOVA_HANDLER_SHOW_");
        assert!(
            out.contains("__in_free_context (__SUPRNOVA_HANDLER_SHOW_"),
            "got:\n{out}"
        );
        assert!(
            out[probe..].contains("__type_id_of (& show)"),
            "the free form names the function itself; got:\n{out}"
        );
    }

    #[test]
    fn bind_003_the_probe_name_differs_per_handler() {
        let first = expansion(quote! { pub async fn show(id: i64) -> Response { todo!() } });
        let second = expansion(quote! { pub async fn show(slug: String) -> Response { todo!() } });
        let name = |out: &str| {
            let start = position(out, "const __SUPRNOVA_HANDLER_SHOW_") + "const ".len();
            out[start..].split_whitespace().next().map(str::to_owned)
        };
        assert_ne!(
            name(&first),
            name(&second),
            "a parent module's probe reached through a glob import must not \
             stand in for a child's"
        );
    }

    #[test]
    fn bind_003_an_argument_that_does_not_start_with_self_is_ignored_as_before() {
        // `#[handler]` ignored its arguments before `Self = Type` existed, so
        // a handler written then with one keeps compiling (BIND-003).
        let item = quote! { pub async fn show(id: i64) -> Response { todo!() } };
        let plain = expansion_with(quote! {}, item.clone());
        for attr in [
            quote! { self = Posts },
            quote! { Type = Posts },
            quote! { Posts },
            quote! { anything(1, 2) },
        ] {
            let out = expansion_with(attr.clone(), item.clone());
            assert_eq!(out, plain, "`{attr}` must be ignored as it was before");
        }
    }

    #[test]
    fn bind_003_a_malformed_self_argument_is_a_compile_error() {
        for attr in [
            quote! { Self },
            quote! { Self = },
            quote! { Self = Posts, Self = Posts },
        ] {
            let out = expansion_with(
                attr.clone(),
                quote! { pub async fn show(id: i64) -> Response { todo!() } },
            );
            assert!(
                out.contains("compile_error"),
                "`{attr}` must be refused; got:\n{out}"
            );
            assert!(out.contains("Self = <Type>"), "`{attr}`; got:\n{out}");
        }
    }

    #[test]
    fn bind_003_self_must_name_a_type_other_than_self() {
        let out = expansion_with(
            quote! { Self = Self },
            quote! { pub async fn show(id: i64) -> Response { todo!() } },
        );
        assert!(out.contains("compile_error"), "got:\n{out}");
        assert!(out.contains("Self = Posts"), "got:\n{out}");
    }

    // ── Order ────────────────────────────────────────────────────────────────

    #[test]
    fn bind_015_bound_arguments_come_before_the_body_without_authorize() {
        // The form is declared first, yet every bound value is taken before
        // the body is read, so a missing row is a 404 before validation.
        let out = expansion(quote! {
            pub async fn update(form: UpdatePost, post: Post) -> Response { todo!() }
        });
        assert!(!out.contains("__authorize_handler"), "got:\n{out}");
        assert!(
            last_position(&out, "__bind (") < position(&out, "__read_body"),
            "every bound value must be taken before the body is read; got:\n{out}"
        );
    }

    // ── #[authorize] ─────────────────────────────────────────────────────────

    #[test]
    fn authorize_param_missing_from_the_signature_is_a_compile_error() {
        let out = expansion(quote! {
            #[authorize("update", post)]
            pub async fn update(id: i64) -> Response { todo!() }
        });
        assert!(out.contains("compile_error"), "got:\n{out}");
        assert!(
            out.contains("`update` takes no parameter named `post`"),
            "the message must name the handler and the missing parameter; got:\n{out}"
        );
    }

    #[test]
    fn authorize_names_the_binding_inside_a_route_param_pattern() {
        // `RouteParam(post)` binds `post` to the model inside the wrapper,
        // so the check runs against `post` itself, with no `Deref`.
        let out = expansion(quote! {
            #[authorize("update", post)]
            pub async fn update(RouteParam(post): RouteParam<Post>) -> Response { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(
            out.contains("__authorize_handler (\"update\" , & post)"),
            "got:\n{out}"
        );
        assert!(!out.contains("Deref"), "got:\n{out}");
    }

    #[test]
    fn bind_015_an_authorize_target_must_bind_or_be_a_path_value() {
        // A parameter target of a type the spelling does not settle must
        // implement `RouteBinding`, which the generated call requires.
        let out = expansion(quote! {
            #[authorize("update", form)]
            pub async fn update(form: UpdatePost) -> Response { todo!() }
        });
        assert!(
            out.contains("__authorize_target :: < UpdatePost >"),
            "the target must be required to bind at compile time; got:\n{out}"
        );

        let out = expansion(quote! {
            #[authorize("update", req)]
            pub async fn update(req: Request) -> Response { todo!() }
        });
        assert!(out.contains("compile_error"), "got:\n{out}");
        assert!(out.contains("request body"), "got:\n{out}");

        let out = expansion(quote! {
            #[authorize("show", page)]
            pub async fn show(page: Option<u32>) -> Response { todo!() }
        });
        assert!(out.contains("compile_error"), "got:\n{out}");
    }

    #[test]
    fn bind_015_a_primitive_authorize_target_checks_the_path_value() {
        let out = expansion(quote! {
            #[authorize("show", id)]
            pub async fn guarded(id: i64) -> Response {
                let _ = "BODY_MARKER";
                todo!()
            }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        let path = position(&out, "path :: < i64 > (\"id\")");
        let check = position(&out, "__authorize_handler (\"show\" , & id)");
        assert!(
            path < check && check < position(&out, "BODY_MARKER"),
            "got:\n{out}"
        );
    }

    #[test]
    fn authorize_on_a_sync_handler_is_a_compile_error() {
        let out = expansion(quote! {
            #[authorize("create", Post)]
            pub fn store() -> Response { todo!() }
        });
        assert!(out.contains("compile_error"), "got:\n{out}");
        assert!(out.contains("async"), "got:\n{out}");
    }

    #[test]
    fn authorize_with_malformed_arguments_is_a_compile_error() {
        for attr in [
            quote! { #[authorize(post)] },
            quote! { #[authorize("update")] },
            quote! { #[authorize("update", post, extra)] },
            quote! { #[authorize("", post)] },
            quote! { #[authorize(update, post)] },
            quote! { #[authorize] },
        ] {
            let out = expansion(quote! {
                #attr
                pub async fn update(post: RouteParam<Post>) -> Response { todo!() }
            });
            assert!(
                out.contains("compile_error"),
                "`{attr}` must be rejected; got:\n{out}"
            );
        }
    }

    #[test]
    fn authorize_param_form_checks_after_binding_and_before_body_and_form() {
        // The form comes first in the signature, yet it must be read after
        // the check: a denied user never sees what the form would reject.
        let out = expansion(quote! {
            #[authorize("update", post)]
            pub async fn update(form: UpdatePost, post: post::Model) -> Response {
                let _ = "BODY_MARKER";
                todo!()
            }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        let binding = position(&out, "__authorize_target :: < post :: Model >");
        let check = position(&out, "__authorize_handler (");
        let form = position(&out, "__read_body");
        let body = position(&out, "BODY_MARKER");
        assert!(
            binding < check && check < form && form < body,
            "order must be binding, check, form, body; got:\n{out}"
        );
        assert!(
            !out.contains("# [authorize"),
            "the attribute must not survive into the output; got:\n{out}"
        );
    }

    #[test]
    fn authorize_route_param_checks_the_inner_model() {
        let out = expansion(quote! {
            #[authorize("update", post)]
            pub async fn update(post: RouteParam<Post>) -> Response { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(
            out.contains("Deref :: deref (& post)"),
            "a RouteParam<M> must be authorized as the M inside it; got:\n{out}"
        );
    }

    #[test]
    fn authorize_type_form_checks_the_type() {
        for (attr, ty) in [
            (quote! { #[authorize("create", Post)] }, "< Post >"),
            (
                quote! { #[authorize("create", post::Model)] },
                "< post :: Model >",
            ),
            (
                quote! { #[authorize("create", crate::models::Post)] },
                "< crate :: models :: Post >",
            ),
        ] {
            let out = expansion(quote! {
                #attr
                pub async fn store(form: StorePost) -> Response {
                    let _ = "BODY_MARKER";
                    todo!()
                }
            });
            assert!(!out.contains("compile_error"), "got:\n{out}");
            let check = position(&out, "__authorize_handler_type");
            assert!(out[check..].contains(ty), "`{ty}` missing; got:\n{out}");
            assert!(
                check < position(&out, "__read_body") && check < position(&out, "BODY_MARKER"),
                "the check must precede the form and the body; got:\n{out}"
            );
        }
    }

    #[test]
    fn authorize_on_a_zero_parameter_handler_checks_before_the_body() {
        let out = expansion(quote! {
            #[authorize("create", Post)]
            pub async fn create() -> Response {
                let _ = "BODY_MARKER";
                todo!()
            }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        assert!(position(&out, "__authorize_handler_type") < position(&out, "BODY_MARKER"));
    }

    #[test]
    fn authorize_attributes_all_apply_in_written_order() {
        let out = expansion(quote! {
            #[authorize("view", post)]
            #[suprnova::authorize("publish", post)]
            pub async fn publish(post: RouteParam<Post>) -> Response { todo!() }
        });
        assert!(!out.contains("compile_error"), "got:\n{out}");
        // The handler's name, `publish`, is in the expansion too, so the
        // checks are found by their call.
        assert!(
            position(&out, "__authorize_handler (\"view\"")
                < position(&out, "__authorize_handler (\"publish\""),
            "checks must run in the order written; got:\n{out}"
        );
        assert!(!out.contains("authorize ("), "got:\n{out}");
    }

    // ── #[authorize] expanding on its own ────────────────────────────────────

    /// What the standalone attribute expands to.
    fn authorize_expansion(
        attr: proc_macro2::TokenStream,
        item: proc_macro2::TokenStream,
    ) -> String {
        crate::authorize::authorize_impl_inner(attr, item).to_string()
    }

    #[test]
    fn authorize_above_handler_moves_below_it() {
        // Written above `#[handler]`, the attribute expands first. It hands
        // itself back below `#[handler]`, which then applies every check.
        let out = authorize_expansion(
            quote! { "view", post },
            quote! {
                #[authorize("publish", post)]
                #[handler]
                pub async fn publish(post: RouteParam<Post>) -> Response { todo!() }
            },
        );
        assert!(!out.contains("compile_error"), "got:\n{out}");
        let handler = position(&out, "# [handler]");
        let view = position(&out, "\"view\"");
        let publish = position(&out, "\"publish\"");
        assert!(
            handler < view && view < publish,
            "`#[handler]` must come first, then the checks in written order; got:\n{out}"
        );
        assert_eq!(out.matches("handler").count(), 1, "got:\n{out}");
    }

    #[test]
    fn authorize_without_handler_is_a_compile_error() {
        let out = authorize_expansion(
            quote! { "update", post },
            quote! { pub async fn update(post: RouteParam<Post>) -> Response { todo!() } },
        );
        assert!(out.contains("compile_error"), "got:\n{out}");
        assert!(out.contains("#[handler]"), "got:\n{out}");

        let out = authorize_expansion(quote! { "update", Post }, quote! { pub struct Post; });
        assert!(out.contains("compile_error"), "got:\n{out}");
    }

    /// BIND-003: the input is `mut` only when an argument takes from it.
    /// Rustc never reports `unused_mut` on a binding whose name starts with
    /// an underscore, so the expansion is read instead.
    #[test]
    fn bind_003_the_input_is_mut_only_when_an_argument_takes_from_it() {
        for src in [
            quote! { pub async fn show(id: i64) -> Response { todo!() } },
            quote! { pub async fn show(id: i64, page: Option<u32>) -> Response { todo!() } },
            quote! { pub fn show(id: String) -> Response { todo!() } },
        ] {
            let out = expansion(src);
            assert!(out.contains("let __suprnova_input ="), "got:\n{out}");
            assert!(!out.contains("let mut __suprnova_input"), "got:\n{out}");
        }
        for src in [
            quote! { pub async fn show(id: i64, post: Post) -> Response { todo!() } },
            quote! { pub async fn show(post: Option<Post>) -> Response { todo!() } },
            quote! { pub async fn show(id: i64, req: Request) -> Response { todo!() } },
            quote! { pub async fn show<T: FromRequest>(id: i64, form: T) -> Response { todo!() } },
            quote! {
                #[authorize("view", post)]
                pub async fn show(post: Post) -> Response { todo!() }
            },
        ] {
            let out = expansion(src);
            assert!(out.contains("let mut __suprnova_input ="), "got:\n{out}");
        }
    }

    // ── The generated code compiled as written source ────────────────────────

    /// A crate under this test binary's target directory
    /// (`<target>/debug/deps/<binary>`), beside the route-binding fixtures,
    /// so it shares their check of `suprnova`.
    fn scratch_crate(name: &str) -> std::path::PathBuf {
        let binary = std::env::current_exe().expect("locate the test binary");
        let target = binary
            .ancestors()
            .nth(3)
            .expect("the test binary sits under <target>/<profile>/deps");
        target.join("tmp/route-binding").join(name)
    }

    /// `cargo check` a crate whose `src/lib.rs` is `source`, against the
    /// framework in this repository.
    fn check_source(name: &str, source: &str) -> std::process::Output {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let framework = manifest_dir
            .join("../framework")
            .canonicalize()
            .expect("locate the framework crate");
        let dir = scratch_crate(name);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).expect("clear the previous crate");
        }
        std::fs::create_dir_all(dir.join("src")).expect("create the crate");
        std::fs::write(
            dir.join("Cargo.toml"),
            format!(
                "[package]\n\
                 name = \"route-binding-{name}\"\n\
                 version = \"0.0.0\"\n\
                 edition = \"2024\"\n\
                 publish = false\n\
                 \n\
                 [workspace]\n\
                 \n\
                 [dependencies]\n\
                 suprnova = {{ path = {framework:?}, default-features = false }}\n\
                 sea-orm = {{ version = \"2.0\", default-features = false }}\n\
                 serde = {{ version = \"1\", features = [\"derive\"] }}\n\
                 tokio = {{ version = \"1\", features = [\"sync\"] }}\n\
                 validator = {{ version = \"0.20\", features = [\"derive\"] }}\n"
            ),
        )
        .expect("write the manifest");
        // The repository's lockfile keeps the crate on the dependency
        // versions the framework is tested with.
        std::fs::copy(manifest_dir.join("../Cargo.lock"), dir.join("Cargo.lock"))
            .expect("seed the lockfile");
        std::fs::write(dir.join("src/lib.rs"), source).expect("write the source");
        std::process::Command::new(env!("CARGO"))
            .args(["check", "--quiet"])
            .env("CARGO_TARGET_DIR", scratch_crate("target"))
            .env("CARGO_INCREMENTAL", "0")
            .current_dir(&dir)
            .output()
            .expect("run cargo check")
    }

    /// BIND-003: the code `#[handler]` and `#[model]` generate imports only
    /// the probe trait it calls, for every form of argument and kind of
    /// column. Rustc does not report a lint inside a macro's output, so
    /// the expansions are rendered and checked as written source, where
    /// `unused_imports` applies. Each handler takes one form of argument
    /// alone and each model one kind of column beside its key, so nothing
    /// else in an expansion uses what that form or kind brings in.
    #[test]
    fn bind_003_generated_code_imports_only_the_probe_trait_it_calls() {
        let models = [
            (
                quote! { table = "tags" },
                quote! { pub struct Tag { pub id: i64, pub name: String } },
            ),
            (
                quote! { table = "notes" },
                quote! { pub struct Note { pub id: i64, pub body: Option<String> } },
            ),
            (
                quote! { table = "documents" },
                quote! { pub struct Document { pub id: i64, pub meta: suprnova::serde_json::Value } },
            ),
            (
                quote! { table = "drafts" },
                quote! { pub struct Draft { pub id: i64, pub meta: Option<suprnova::serde_json::Value> } },
            ),
            (
                quote! { table = "blobs" },
                quote! { pub struct Blob { pub id: i64, pub digest: Vec<u8> } },
            ),
            (
                quote! { table = "posts", route_key = "title" },
                quote! {
                    pub struct Post {
                        pub id: i64,
                        pub title: String,
                        pub subtitle: Option<String>,
                        pub meta: suprnova::serde_json::Value,
                        pub extra: Option<suprnova::serde_json::Value>,
                        pub digest: Vec<u8>,
                    }
                },
            ),
        ];
        let handlers = [
            quote! { pub async fn bound(tag: Tag) -> Response { suprnova::http::text(tag.name) } },
            quote! { pub async fn form(form: UpdateTag) -> Response { suprnova::http::text(form.name) } },
            quote! {
                pub async fn maybe_bound(tag: Option<Tag>) -> Response {
                    suprnova::http::text(format!("{}", tag.is_some()))
                }
            },
            quote! {
                pub async fn wrapped(tag: RouteParam<Tag>) -> Response {
                    suprnova::http::text(tag.name.clone())
                }
            },
            quote! {
                pub async fn destructured(RouteParam(tag): RouteParam<Tag>) -> Response {
                    suprnova::http::text(tag.name)
                }
            },
            quote! { pub async fn bare(tag: tag::Model) -> Response { suprnova::http::text(tag.name) } },
            quote! { pub async fn path(id: i64) -> Response { suprnova::http::text(id.to_string()) } },
            quote! {
                pub async fn maybe_path(page: Option<u32>) -> Response {
                    suprnova::http::text(format!("{page:?}"))
                }
            },
            quote! {
                pub async fn raw(req: Request) -> Response {
                    suprnova::http::text(req.path().to_owned())
                }
            },
            quote! {
                pub async fn generic<T: FromRequest + Send + 'static>(form: T) -> Response {
                    let _ = form;
                    suprnova::http::text("generic")
                }
            },
            quote! {
                pub async fn other_output(
                    tag: Tag,
                ) -> Result<suprnova::HttpResponse, suprnova::FrameworkError> {
                    Ok(suprnova::HttpResponse::text(tag.name))
                }
            },
            quote! {
                pub async fn unnamed_output(tag: Tag) -> Result<impl std::fmt::Display, suprnova::FrameworkError> {
                    Ok(tag.name)
                }
            },
            quote! {
                pub async fn generic_bound<T: FromRequest + Send + 'static>(
                    tag: Tag,
                    other: Option<Tag>,
                    form: T,
                ) -> Response {
                    let _ = form;
                    suprnova::http::text(format!("{} {}", tag.name, other.is_some()))
                }
            },
            quote! {
                #[suprnova::authorize("view", tag)]
                pub async fn generic_guarded<T: FromRequest + Send + 'static>(
                    tag: Tag,
                    form: T,
                ) -> Response {
                    let _ = form;
                    suprnova::http::text(tag.name)
                }
            },
            quote! {
                #[suprnova::authorize("view", tag)]
                pub async fn guarded(tag: Tag) -> Response { suprnova::http::text(tag.name) }
            },
            quote! {
                #[suprnova::authorize("show", id)]
                pub async fn guarded_id(id: i64) -> Response {
                    suprnova::http::text(id.to_string())
                }
            },
            quote! {
                #[suprnova::authorize("update", tag)]
                pub async fn guarded_form(tag: Tag, form: UpdateTag) -> Response {
                    suprnova::http::text(format!("{} {}", tag.name, form.name))
                }
            },
            quote! { pub async fn index() -> Response { suprnova::http::text("index") } },
            quote! { pub fn ping() -> Response { suprnova::http::text("pong") } },
            quote! {
                pub async fn every(
                    id: i64,
                    page: Option<u32>,
                    tag: Tag,
                    other: Option<Tag>,
                    form: UpdateTag,
                ) -> Response {
                    suprnova::http::text(format!(
                        "{id} {page:?} {} {} {}",
                        tag.name,
                        other.is_some(),
                        form.name
                    ))
                }
            },
        ];
        let associated = [
            quote! { pub async fn show(tag: Tag) -> Response { suprnova::http::text(tag.name) } },
            quote! { pub async fn by_id(id: i64) -> Response { suprnova::http::text(id.to_string()) } },
            quote! {
                pub async fn generic_show<T: FromRequest + Send + 'static>(
                    tag: Tag,
                    form: T,
                ) -> Response {
                    let _ = form;
                    suprnova::http::text(tag.name)
                }
            },
        ];

        let mut source = String::from(
            "#![deny(unused_imports)]\n\
             use suprnova::{FromRequest, Request, Response, RouteParam};\n\
             impl suprnova::database::EntityExt for tag::Entity {}\n\
             #[suprnova::request]\n\
             pub struct UpdateTag { pub name: String }\n\
             pub struct Tags;\n",
        );
        for (attr, item) in models {
            let expanded = crate::model::expand(attr, item).expect("expand the model");
            source.push_str(&expanded.to_string());
            source.push('\n');
        }
        for item in handlers {
            source.push_str(&expansion(item));
            source.push('\n');
        }
        source.push_str("impl Tags {\n");
        for item in associated {
            source.push_str(&expansion_with(quote! { Self = Tags }, item));
            source.push('\n');
        }
        source.push_str("}\n");
        assert!(!source.contains("compile_error"), "got:\n{source}");

        let output = check_source("rendered", &source);
        assert!(
            output.status.success(),
            "the generated code must import only what it uses, yet the \
             check failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
