//! `#[suprnova::prunable]` - wraps `impl Prunable for T` (or
//! `impl MassPrunable for T`) and emits an inventory entry that
//! registers the type's pruner with the `model:prune` runtime.
//!
//! The macro detects which trait via the trait-path identifier
//! (`MassPrunable` → bulk-delete path; anything else → per-row path)
//! and emits the appropriate runner closure:
//!
//! - **Prunable** (per-row): iterates the scope, calls `pruning(&row)`
//!   on each, then force-deletes the row. `dry_run = true` returns the
//!   rowcount via `Builder::count()` without deleting anything.
//! - **MassPrunable** (set-based): renders the scope's WHERE clause
//!   into a single `DELETE FROM ... WHERE ...` via the Builder's
//!   `to_delete_sql_with_bindings_for` (shipped in T5) and runs that
//!   statement directly - atomic, single round-trip.
//!
//! The runner closure is `'static`-callable (no captured references)
//! because the inventory entry stores it as a fn-pointer-shaped
//! `PrunerFn`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{ItemImpl, Result, Type, parse2};

pub fn expand(item: TokenStream) -> Result<TokenStream> {
    let item_impl: ItemImpl = parse2(item)?;
    let self_ty = &item_impl.self_ty;

    // Take the last path-segment ident as the registry type name
    // (`crate::models::User` → `"User"`). This matches what the
    // `--model=Name` CLI flag reads, and what the test harness uses
    // for `prune_one(name, ...)`.
    let type_name = match &**self_ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .expect("Type::Path always has at least one segment")
            .ident
            .to_string(),
        _ => {
            return Err(syn::Error::new_spanned(
                self_ty,
                "#[prunable] must wrap an `impl Trait for T` block where `T` is a path type",
            ));
        }
    };

    // Detect Prunable vs MassPrunable by inspecting the trait path's
    // last segment. The macro accepts both `Prunable` and fully
    // qualified `suprnova::eloquent::Prunable` (or any rename) since
    // we only compare the final ident.
    let is_mass = item_impl
        .trait_
        .as_ref()
        .map(|(_, path, _)| {
            path.segments
                .last()
                .map(|s| s.ident == "MassPrunable")
                .unwrap_or(false)
        })
        .unwrap_or(false);

    let runner_body = if is_mass {
        quote! {
            let builder = <#self_ty as ::suprnova::eloquent::MassPrunable>::prunable();
            if dry_run {
                // Count the matching rows via a SELECT COUNT(*). The
                // builder's count() consumes self - we built a fresh
                // one above and don't reuse it after.
                builder.count().await.map(|c| c as u64)
            } else {
                // Bulk delete through the builder's own DELETE, the one
                // the dry run's count mirrors. It walks the WHERE AST
                // into `DELETE FROM table WHERE ...` - so a scope that
                // sets `.select(...)` / `.order_by(...)` cannot break
                // it - and it runs where the builder routes: its
                // `with_tx` or `on(name)`, an ambient `DB::transaction`,
                // or the model's `#[model(connection = "...")]`. The
                // count and the delete therefore reach the same rows.
                builder.force_delete_all().await
            }
        }
    } else {
        quote! {
            let builder = <#self_ty as ::suprnova::eloquent::Prunable>::prunable();
            if dry_run {
                builder.count().await.map(|c| c as u64)
            } else {
                let rows = builder.get().await?;
                let n = rows.len() as u64;
                for row in rows {
                    <#self_ty as ::suprnova::eloquent::Prunable>::pruning(&row).await?;
                    <#self_ty as ::suprnova::eloquent::Model>::force_delete(row).await?;
                }
                Ok(n)
            }
        }
    };

    Ok(quote! {
        #item_impl

        ::suprnova::inventory::submit! {
            ::suprnova::eloquent::PrunerEntry {
                type_name: #type_name,
                run: |dry_run: bool| {
                    ::std::boxed::Box::pin(async move { #runner_body })
                },
            }
        }
    })
}
