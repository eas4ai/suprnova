//! Phase 10C T3 - `#[suprnova::scopes(Model)]` impl-block attribute.
//!
//! Wraps an `impl Model { ... }` block and, for every method whose
//! signature matches the scope shape
//! `fn name(query: Builder<Self>[, args...]) -> Builder<Self>`,
//! emits two additional callable forms from one declaration:
//!
//! 1. **Static helper** on the model: `Model::active(args...)` -
//!    starts from `Self::query()` and applies the scope.
//! 2. **Builder extension**: `Builder<Model>::active(args...)` -
//!    chainable extension method.
//!
//! A method that has nothing to do with a builder passes through
//! unchanged, so scopes and ordinary inherent methods can share one impl
//! block. A method that takes or returns a `Builder` in any other form
//! than the scope shape is a scope written wrong, and is a compile error
//! that names the accepted shape: left alone it would become an ordinary
//! method, no `.name()` would reach the builder, and the first sign would
//! be an error at some distant call site. A helper that legitimately
//! handles a builder says so with `#[not_scope]`.
//!
//! # Why impl-block-level, not per-method
//!
//! Proc-macro attribute macros on `ImplItemFn` may only emit
//! `ImplItem`s back into the impl block - they cannot emit
//! module-scope items (trait declarations, blanket impls). The scope
//! pattern needs both (an extension trait that surfaces `<name>` on
//! `Builder<M>` plus the trait impl that wires the method body), so
//! the attribute moves up to `ItemImpl`. From `ItemImpl` the macro can
//! emit any sequence of module-scope items including the rewritten
//! impl block.
//!
//! This mirrors `#[suprnova::observer(M)]`, which is also impl-block
//! level for the same reason (it emits one adapter struct + one
//! `Listener` impl per observed method, all at module scope).
//!
//! # Why per-model trait naming
//!
//! Each scope emits a trait named `HasScope_<scope>_<Model>` instead
//! of a generic-over-`M` trait. Two reasons:
//!
//! 1. **Same scope name on two models in the same module.** With a
//!    generic trait `HasScope_active<M>` and blanket impl
//!    `impl<M> HasScope_active<M> for Builder<M> where M:
//!    Has__scope_active`, the trait declaration emits once per
//!    `#[scopes]` invocation. Two `#[scopes]` calls in the same module
//!    that both define `active` would re-declare the trait and the
//!    blanket impl, breaking compilation. Per-model trait names
//!    (`HasScope_active_T3User` and `HasScope_active_T3Article`) sit
//!    in distinct namespaces and never collide.
//!
//! 2. **Simpler emission.** The generic-over-`M` shape needed a
//!    second `Has__scope_<name>` marker trait to carry the apply
//!    method, plus a blanket impl bridging the two. Per-model
//!    collapses to ONE trait per scope with a concrete impl on
//!    `Builder<Model>`. Less code, less surface, same UX.
//!
//! The trait suffix is the last path segment of the model type
//! argument (e.g. `crate::models::User` → `User`). Two `User` types
//! in the same module from different paths would collide, but that's
//! a pathological case rustc itself would already flag.
//!
//! # Bringing the chainable form into scope
//!
//! The trait `HasScope_<scope>_<Model>` is emitted at module scope
//! with `pub` visibility. Inside the same module, `.<scope>()` on a
//! `Builder<Model>` resolves automatically (the trait is a sibling
//! item, in scope). To use the chainable form from a different
//! module, the consumer must `use` the trait - same as any extension
//! trait in Rust. The static helper `Model::<scope>(args)` is an
//! inherent method and works without an extra `use`.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    FnArg, ImplItem, ImplItemFn, ItemImpl, Pat, PatIdent, ReturnType, Type, Visibility,
    parse_macro_input, parse_quote,
};

/// Implementation of `#[suprnova::scopes(Model)]`.
pub fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    // ---- Parse the attribute (the model type) ---------------------------
    let model_ty: Type = match syn::parse(attr) {
        Ok(t) => t,
        Err(e) => {
            return syn::Error::new(
                e.span(),
                format!(
                    "#[suprnova::scopes(Model)] requires the model type as the \
                     attribute argument: {e}"
                ),
            )
            .to_compile_error()
            .into();
        }
    };

    // Derive the last-segment ident of the model type for trait suffix
    // names. `T3User` → `T3User`; `crate::models::User` → `User`.
    let model_ident = match last_path_segment_ident(&model_ty) {
        Some(id) => id,
        None => {
            return syn::Error::new_spanned(
                &model_ty,
                "#[suprnova::scopes(Model)] requires a named type as the model \
                 argument (e.g. `User`, `crate::models::User`)",
            )
            .to_compile_error()
            .into();
        }
    };

    // ---- Parse the impl block -------------------------------------------
    let input = parse_macro_input!(item as ItemImpl);

    match expand_impl(input, &model_ty, &model_ident) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// The expansion over `proc_macro2` types, which is what the unit tests
/// drive: walk the impl items and split scope methods from passthroughs.
fn expand_impl(
    mut input: ItemImpl,
    model_ty: &Type,
    model_ident: &syn::Ident,
) -> syn::Result<TokenStream2> {
    let mut module_emissions: Vec<TokenStream2> = Vec::new();
    let mut new_impl_items: Vec<ImplItem> = Vec::with_capacity(input.items.len());

    for item in input.items.drain(..) {
        match item {
            ImplItem::Fn(mut f) => {
                if take_not_scope_marker(&mut f) {
                    // The author said it is not a scope: no check, no rewrite.
                    new_impl_items.push(ImplItem::Fn(f));
                    continue;
                }
                match try_expand_scope_fn(&mut f, model_ty, model_ident) {
                    ScopeExpand::Skip => {
                        // Not a scope - pass through unchanged.
                        new_impl_items.push(ImplItem::Fn(f));
                    }
                    ScopeExpand::Rewrite(rewrite) => {
                        // The renamed `__scope_<name>` inner fn stays in the
                        // impl block. We append it plus the static helper.
                        let ScopeRewrite {
                            static_helper,
                            module_items,
                        } = *rewrite;
                        new_impl_items.push(ImplItem::Fn(f));
                        new_impl_items.push(ImplItem::Fn(static_helper));
                        module_emissions.push(module_items);
                    }
                    ScopeExpand::Error(e) => return Err(e),
                }
            }
            other => new_impl_items.push(other),
        }
    }

    input.items = new_impl_items;

    Ok(quote! {
        #input

        #(#module_emissions)*
    })
}

/// Remove a `#[not_scope]` marker from `f` and say whether it was there.
/// The marker is this macro's own: it is stripped so the compiler never
/// sees an attribute it does not know.
fn take_not_scope_marker(f: &mut ImplItemFn) -> bool {
    let before = f.attrs.len();
    f.attrs.retain(|attr| !attr.path().is_ident("not_scope"));
    f.attrs.len() != before
}

/// What a near miss is told. Spanned on the signature that caused it.
const SCOPE_SHAPE: &str = "this method handles a `Builder` but does not have the scope \
     signature, so `#[suprnova::scopes]` would leave it an ordinary method and no \
     scope would reach the builder. A scope is \
     `fn name(query: Builder<Self>, ...) -> Builder<Self>`: it takes the builder by \
     value as its first parameter, names the model as `Self`, and returns the \
     builder. Mark a helper that is not a scope with `#[not_scope]`";

/// Result of the per-fn classifier.
///
/// `ImplItemFn` is a sizeable `syn` enum (its largest variants exceed
/// 400 bytes), so the `Rewrite` payload is boxed to keep
/// `ScopeExpand` itself small. Without the box, clippy's
/// `large_enum_variant` lint reports the unboxed variant.
enum ScopeExpand {
    /// Not a scope; preserve the fn as-is.
    Skip,
    /// Scope detected - replace with the renamed inner + static helper,
    /// and emit module-scope items.
    Rewrite(Box<ScopeRewrite>),
    /// Scope detected but invalid (e.g. pattern arg names).
    Error(syn::Error),
}

/// Payload of `ScopeExpand::Rewrite`. Held behind a `Box` in the enum
/// for size-balancing reasons; see `ScopeExpand` doc comment.
struct ScopeRewrite {
    static_helper: ImplItemFn,
    module_items: TokenStream2,
}

/// Classify an impl-block fn. Mutates `f` in place to rename the inner
/// to `__scope_<name>` if it's a scope.
fn try_expand_scope_fn(
    f: &mut ImplItemFn,
    model_ty: &Type,
    model_ident: &syn::Ident,
) -> ScopeExpand {
    // Rule 1: first param must be a typed arg whose Type matches
    // `Builder<Self>` (with or without leading `suprnova::` /
    // `::suprnova::` qualifier).
    let inputs = &f.sig.inputs;
    let first_is_builder_self = matches!(
        inputs.first(),
        Some(FnArg::Typed(t)) if is_builder_self_type(&t.ty)
    );
    // Rule 2: return type must be `Builder<Self>` (same accepted
    // qualifiers).
    let return_is_builder_self = matches!(
        &f.sig.output,
        ReturnType::Type(_, t) if is_builder_self_type(t)
    );

    if !(first_is_builder_self && return_is_builder_self) {
        // Either it has nothing to do with a builder, and is a helper, or
        // it handles one in the wrong form, and is a scope written wrong.
        let handles_a_builder = inputs.iter().any(|arg| match arg {
            FnArg::Typed(t) => mentions_builder(&t.ty),
            FnArg::Receiver(_) => false,
        }) || matches!(&f.sig.output, ReturnType::Type(_, t) if mentions_builder(t));
        return if handles_a_builder {
            ScopeExpand::Error(syn::Error::new_spanned(&f.sig, SCOPE_SHAPE))
        } else {
            ScopeExpand::Skip
        };
    }
    let return_ty = match &f.sig.output {
        ReturnType::Type(_, t) => t.clone(),
        ReturnType::Default => unreachable!("the return type was just matched"),
    };

    // At this point we're committed to treating this as a scope.
    // Subsequent issues (pattern args, etc.) become compile errors.
    let original_name = f.sig.ident.clone();
    let inner_name = format_ident!("__scope_{}", original_name);
    let vis = f.vis.clone();

    // Capture remaining params (after the leading builder arg) as
    // (ident, type) pairs. Pattern args are rejected because the
    // forwarding call site needs to reference each by name.
    let mut extra_params: Vec<(syn::Ident, Box<Type>)> = Vec::new();
    for arg in inputs.iter().skip(1) {
        match arg {
            FnArg::Typed(t) => match &*t.pat {
                Pat::Ident(PatIdent { ident, .. }) => {
                    extra_params.push((ident.clone(), t.ty.clone()));
                }
                _ => {
                    return ScopeExpand::Error(syn::Error::new_spanned(
                        &t.pat,
                        "#[suprnova::scopes] extra parameters must be plain \
                         `name: Type` bindings - pattern args (tuples, structs) \
                         are not supported because the emitted forwarding call \
                         site needs to reference each by name",
                    ));
                }
            },
            FnArg::Receiver(_) => {
                return ScopeExpand::Error(syn::Error::new_spanned(
                    arg,
                    "#[suprnova::scopes] scope methods cannot take `self` \
                     parameters",
                ));
            }
        }
    }

    let extra_idents: Vec<&syn::Ident> = extra_params.iter().map(|(i, _)| i).collect();
    let extra_arg_pairs: Vec<TokenStream2> = extra_params
        .iter()
        .map(|(i, t)| quote! { #i: #t })
        .collect();

    // ---- Rename the original to __scope_<name> in place ----------------
    f.sig.ident = inner_name.clone();
    // Ensure the inner is `pub` - the static helper and the trait impl
    // (which may live in a separate module if the user writes
    // `crate::models::User`) both need to reach it.
    f.vis = Visibility::Public(parse_quote!(pub));

    // ---- Build the static helper (a sibling impl item) ----------------
    //
    // `Model::active(args...)` starts from `Self::query()` and forwards
    // through the trait so the chainable and static forms always share
    // the same apply path. This means a future override of the trait
    // method (advanced cases, e.g. a per-model specialization that
    // bypasses the inner) is honoured from both entry points.
    let trait_name = scope_trait_ident(&original_name, model_ident);
    let static_helper_tokens = quote! {
        #vis fn #original_name(#(#extra_arg_pairs),*) -> #return_ty {
            <::suprnova::Builder<Self> as #trait_name>::#original_name(
                <Self as ::suprnova::eloquent::Model>::query(),
                #(#extra_idents),*
            )
        }
    };
    let static_helper: ImplItemFn = match syn::parse2(static_helper_tokens.clone()) {
        Ok(f) => f,
        Err(e) => {
            return ScopeExpand::Error(syn::Error::new(
                e.span(),
                format!(
                    "internal: failed to parse generated static helper for \
                     #[scopes] scope `{original_name}`: {e}"
                ),
            ));
        }
    };

    // ---- Build module-scope items -------------------------------------
    //
    // ONE trait per scope, per-model-named, with a single concrete impl
    // on `Builder<Model>`. The trait is `pub` so it can be brought into
    // scope by `use` from sibling modules.
    let model_path = model_ty;
    let module_items = quote! {
        #[allow(non_camel_case_types)]
        pub trait #trait_name {
            fn #original_name(self, #(#extra_arg_pairs),*) -> ::suprnova::Builder<#model_path>;
        }

        impl #trait_name for ::suprnova::Builder<#model_path> {
            fn #original_name(self, #(#extra_arg_pairs),*) -> ::suprnova::Builder<#model_path> {
                <#model_path>::#inner_name(self, #(#extra_idents),*)
            }
        }
    };

    ScopeExpand::Rewrite(Box::new(ScopeRewrite {
        static_helper,
        module_items,
    }))
}

/// Test whether a `Type` is one of the accepted `Builder<Self>` shapes.
///
/// Normalizes the type via `quote!` token-string round-trip and strips
/// whitespace so generics with extra spaces parse the same. Accepts
/// exactly:
///
/// - `Builder<Self>`
/// - `suprnova::Builder<Self>`
/// - `::suprnova::Builder<Self>`
///
/// Anything else (including `Builder<OtherModel>`, `Builder<Self>` with
/// extra generic args, or unrelated types) returns `false` so the
/// caller leaves the fn untouched.
fn is_builder_self_type(ty: &Type) -> bool {
    let s = quote!(#ty).to_string();
    let s = s.replace(' ', "");
    matches!(
        s.as_str(),
        "Builder<Self>" | "suprnova::Builder<Self>" | "::suprnova::Builder<Self>"
    )
}

/// Whether a type names a `Builder` anywhere in it: `&mut Builder<User>`,
/// `Builder<User>`, `Option<Builder<Self>>`. Read from the type's tokens,
/// so a path qualifier or a reference in front does not hide it.
fn mentions_builder(ty: &Type) -> bool {
    quote!(#ty)
        .into_iter()
        .any(|token| contains_builder_ident(&token))
}

fn contains_builder_ident(token: &proc_macro2::TokenTree) -> bool {
    match token {
        proc_macro2::TokenTree::Ident(ident) => ident == "Builder",
        proc_macro2::TokenTree::Group(group) => group
            .stream()
            .into_iter()
            .any(|inner| contains_builder_ident(&inner)),
        _ => false,
    }
}

/// Extract the final path segment ident of a model type so it can be
/// used as a suffix on the per-model trait name. Returns `None` for
/// types that aren't named (tuples, references, fn pointers, etc.).
fn last_path_segment_ident(ty: &Type) -> Option<syn::Ident> {
    match ty {
        Type::Path(tp) => tp.path.segments.last().map(|s| s.ident.clone()),
        _ => None,
    }
}

/// Build the per-(scope, model) extension trait ident:
/// `HasScope_<scope>_<Model>`. The two-segment underscore separator
/// keeps the structure obvious in error messages.
fn scope_trait_ident(scope_name: &syn::Ident, model_ident: &syn::Ident) -> syn::Ident {
    format_ident!("HasScope_{}_{}", scope_name, model_ident)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand_block(block: ItemImpl) -> syn::Result<String> {
        let model_ty: Type = parse_quote!(User);
        let model_ident = last_path_segment_ident(&model_ty).unwrap();
        expand_impl(block, &model_ty, &model_ident).map(|tokens| tokens.to_string())
    }

    #[test]
    fn a_scope_gains_its_static_and_chainable_forms() {
        let emitted = expand_block(parse_quote! {
            impl User {
                pub fn active(query: Builder<Self>) -> Builder<Self> {
                    query.filter("active", true)
                }
            }
        })
        .unwrap();

        assert!(emitted.contains("__scope_active"), "{emitted}");
        assert!(emitted.contains("HasScope_active_User"), "{emitted}");
    }

    #[test]
    fn a_method_that_handles_no_builder_passes_through() {
        let emitted = expand_block(parse_quote! {
            impl User {
                pub fn display_name(&self) -> String {
                    self.name.clone()
                }
                fn slug(name: &str) -> String {
                    name.to_lowercase()
                }
            }
        })
        .unwrap();

        assert!(emitted.contains("display_name"), "{emitted}");
        assert!(!emitted.contains("HasScope_"), "{emitted}");
    }

    #[test]
    fn a_scope_written_in_any_other_form_is_refused() {
        let near_misses: [ItemImpl; 5] = [
            parse_quote! { impl User {
                fn active(query: &mut Builder<User>) { query.filter("active", true); }
            } },
            parse_quote! { impl User {
                fn active(query: Builder<User>) -> Builder<User> { query }
            } },
            parse_quote! { impl User {
                fn active(query: Builder<Self>) { let _ = query; }
            } },
            parse_quote! { impl User {
                fn active(query: Builder<Self>) -> suprnova::Builder<User> { query }
            } },
            parse_quote! { impl User {
                fn active(limit: u64, query: Builder<Self>) -> Builder<Self> { query.limit(limit) }
            } },
        ];

        for block in near_misses {
            let shown = quote!(#block).to_string();
            let error = expand_block(block).expect_err(&shown).to_string();
            assert!(
                error.contains("fn name(query: Builder<Self>, ...) -> Builder<Self>"),
                "{shown} is refused with the accepted shape named, got: {error}"
            );
            assert!(error.contains("#[not_scope]"), "{error}");
        }
    }

    #[test]
    fn not_scope_marks_a_helper_that_handles_a_builder() {
        let emitted = expand_block(parse_quote! {
            impl User {
                #[not_scope]
                fn describe(query: &Builder<User>) -> String {
                    query.to_sql()
                }
            }
        })
        .unwrap();

        assert!(emitted.contains("describe"), "{emitted}");
        assert!(
            !emitted.contains("not_scope"),
            "the marker is stripped before the compiler sees it: {emitted}"
        );
        assert!(!emitted.contains("HasScope_"), "{emitted}");
    }
}
