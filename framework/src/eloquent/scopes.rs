//! Phase 10C T3 + T4 - local scopes (trait emissions from the
//! `#[suprnova::scopes]` macro live in `suprnova-macros/src/scopes.rs`)
//! and global scopes (this file).
//!
//! Global scopes apply automatically to every [`Model::query`] call.
//! Users register them at boot via [`ScopeRegistry::register::<M, S>`].
//! Each query can opt out of a single scope by type with
//! [`Builder::without_global_scope::<S>`] or bypass the registry
//! entirely with [`Builder::without_global_scopes`].
//!
//! ## Soft deletes coexistence
//!
//! Suprnova's [`SoftDeletes`][crate::eloquent::SoftDeletes] pathway
//! does **not** route through this registry - it ships its own inherent
//! `Model::query` override (emitted by `#[suprnova::model(soft_deletes)]`)
//! that prepends a `deleted_at IS NULL` filter, and a
//! `global_scopes_disabled: Vec<&'static str>` tag system on the
//! builder. The two paths coexist; T4 does not retroactively fold
//! soft-deletes into the registry.
//!
//! ## PK lookups
//!
//! Global scopes apply through [`Model::query`]. [`Model::find`],
//! [`Model::find_many`], and [`Model::all`] go through SeaORM's
//! `find_by_id` / `find().all()` directly and do **not** receive
//! registered scopes - matching Laravel's `Eloquent\Model::find`
//! semantics. Callers that want scoped PK lookups use
//! `Self::query().filter("id", pk).first().await`.
//!
//! [`Model::query`]: crate::eloquent::Model::query
//! [`Model::find`]: crate::eloquent::Model::find
//! [`Model::find_many`]: crate::eloquent::Model::find_many
//! [`Model::all`]: crate::eloquent::Model::all
//! [`Builder::without_global_scope::<S>`]: crate::eloquent::Builder::without_global_scope
//! [`Builder::without_global_scopes`]: crate::eloquent::Builder::without_global_scopes

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use sea_orm::{EntityTrait, IntoActiveModel, PrimaryKeyTrait};
use serde::Serialize;

use crate::eloquent::builder::Builder;
use crate::eloquent::model::Model;

/// A global scope that applies to every [`Model::query`] call for
/// model `M`. Register at boot via [`ScopeRegistry::register`].
///
/// ## Example
///
/// ```ignore
/// use suprnova::eloquent::scopes::{GlobalScope, ScopeRegistry};
/// use suprnova::{Builder, Model};
///
/// pub struct TenantScope;
///
/// impl GlobalScope<Article> for TenantScope {
///     fn apply(&self, query: Builder<Article>) -> Builder<Article> {
///         query.filter("tenant_id", current_tenant_id())
///     }
/// }
///
/// // At boot:
/// ScopeRegistry::register::<Article, _>(TenantScope);
///
/// // Every query is scoped automatically:
/// let rows = Article::query().get().await?;
///
/// // Opt out by type:
/// let unscoped = Article::without_global_scope::<TenantScope>().get().await?;
///
/// // Opt out of everything:
/// let all = Article::without_global_scopes().get().await?;
/// ```
///
/// [`Model::query`]: crate::eloquent::Model::query
///
/// The where-clause re-elaborates [`Model`]'s own bounds because
/// Rust's trait elaboration doesn't transitively propagate
/// associated-type bounds from a supertrait's where-clause to a
/// subtrait's method bodies - the same pattern [`FirstOrCreate`] and
/// [`SoftDeletes`] use for the same reason.
///
/// [`Model`]: crate::eloquent::Model
/// [`FirstOrCreate`]: crate::eloquent::FirstOrCreate
/// [`SoftDeletes`]: crate::eloquent::SoftDeletes
pub trait GlobalScope<M>: Send + Sync + 'static
where
    M: Model,
    M: From<<M::Entity as EntityTrait>::Model>,
    <M::Entity as EntityTrait>::Model: From<M>
        + IntoActiveModel<<M::Entity as EntityTrait>::ActiveModel>
        + Serialize
        + Send
        + Sync,
    <M::Entity as EntityTrait>::ActiveModel: Send,
    <<M::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
        Send + Into<sea_orm::Value>,
{
    /// Mutate the builder however the scope needs. Called once per
    /// `Model::query()` invocation. Return the (possibly modified)
    /// builder. The framework chains scopes in registration order.
    fn apply(&self, query: Builder<M>) -> Builder<M>;

    /// What this scope's filter depends on.
    ///
    /// Defaults to [`ScopeDependency::PerRequest`]: an undeclared scope is
    /// treated as reading per-request state, because an invisible tenant
    /// filter is the failure this rule exists to catch and a constant scope
    /// loses only its cache hits until it declares itself.
    fn dependency(&self) -> ScopeDependency {
        ScopeDependency::PerRequest
    }
}

/// What a global scope's filter depends on.
///
/// A scope is invisible to RenderCache as a scope: only the accessors its
/// `apply` calls are observed. This declaration is how a scope says which
/// of the two it is, so the registry can tell "read nothing because there
/// was nothing to read" from "read per-request state through a seam
/// nothing can see".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeDependency {
    /// The filter is the same for every request (`published = true`).
    /// Evaluation records nothing beyond the query's own table reads.
    Constant,
    /// The filter reads per-request state. The read must go through an
    /// instrumented accessor (`suprnova::live::current_tenant()`,
    /// `Auth::id()`, `Lang::locale()`); an evaluation that records no
    /// resolvable read narrows the render to `Uncacheable` and names the
    /// scope in diagnostics.
    PerRequest,
}

/// Type-erased apply closure. The concrete `Arc<S>` is captured at
/// registration time; the closure downcasts a `Box<dyn Any>` back to
/// `Builder<M>`, runs the scope, and re-boxes the result. The
/// `register::<M, S>` generics guarantee the closure is only ever
/// invoked against `Builder<M>` of the matching type - `register`
/// stores the closure under `TypeId::of::<M>()`, and `apply_to::<M>`
/// looks it up under the same key.
type ErasedApply = Arc<dyn Fn(Box<dyn Any + Send>) -> Box<dyn Any + Send> + Send + Sync>;

/// One registered scope: its type, its erased `apply`, what it declared it
/// depends on, and the compile-time name diagnostics use for it.
#[derive(Clone)]
struct ScopeEntry {
    scope_type_id: TypeId,
    apply: ErasedApply,
    dependency: ScopeDependency,
    diagnostic_name: String,
}

struct PerModelScopes {
    /// Registered scopes in registration order, so they layer onto the
    /// WHERE clause in the order the user declared them.
    entries: Vec<ScopeEntry>,
}

/// The last path segment of a type's name, generic arguments kept:
/// `TenantScope`, or `TenantScope<Article>` for a generic scope.
///
/// Compile-time material and never request data, which is what lets the
/// name go through `observe_undeclared` at all - that function bounds a
/// name to 64 characters and a report to 32 of them, and the closed
/// diagnostics rule forbids anything derived from a request.
fn short_type_name<S: 'static>() -> String {
    let full = std::any::type_name::<S>();
    let (path, generics) = match full.split_once('<') {
        Some((path, rest)) => (path, Some(rest)),
        None => (full, None),
    };
    let leaf = path.rsplit("::").next().unwrap_or(path);
    match generics {
        Some(rest) => format!("{leaf}<{rest}"),
        None => leaf.to_owned(),
    }
}

static REGISTRY: OnceLock<RwLock<HashMap<TypeId, PerModelScopes>>> = OnceLock::new();

fn registry() -> &'static RwLock<HashMap<TypeId, PerModelScopes>> {
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

/// The process-global scope registry.
///
/// Registered at boot, applied automatically by [`Model::query`].
/// Storage is per-model (`TypeId::of::<M>()`), and per-model entries
/// are an ordered `Vec` so scope application matches registration
/// order.
///
/// [`Model::query`]: crate::eloquent::Model::query
pub struct ScopeRegistry;

impl ScopeRegistry {
    /// Register `scope` to apply on every `M::query()` call.
    ///
    /// The scope is wrapped in an `Arc` and stored under the
    /// `TypeId::of::<M>()` slot. Multiple scopes per model are
    /// supported; they apply in registration order.
    ///
    /// `S` must be a unit struct or otherwise carry no per-call
    /// state - the captured `Arc<S>` is shared across every query.
    /// Per-request state (e.g. the current tenant ID) belongs in a
    /// thread-local / `tokio::task_local!` / `AtomicI64` that the
    /// scope reads inside `apply`.
    pub fn register<M, S>(scope: S)
    where
        M: Model + 'static,
        M: From<<M::Entity as EntityTrait>::Model>,
        <M::Entity as EntityTrait>::Model: From<M>
            + IntoActiveModel<<M::Entity as EntityTrait>::ActiveModel>
            + Serialize
            + Send
            + Sync,
        <M::Entity as EntityTrait>::ActiveModel: Send,
        <<M::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
            Send + Into<sea_orm::Value>,
        S: GlobalScope<M> + 'static,
    {
        let dependency = scope.dependency();
        let diagnostic_name = format!("global_scope:{}", short_type_name::<S>());
        let scope = Arc::new(scope);
        let scope_type_id = TypeId::of::<S>();
        let model_type_id = TypeId::of::<M>();

        let apply: ErasedApply = Arc::new(move |b: Box<dyn Any + Send>| {
            let builder: Builder<M> = *b
                .downcast::<Builder<M>>()
                .expect("ScopeRegistry: erased apply dispatched to wrong model type");
            let result = scope.apply(builder);
            Box::new(result) as Box<dyn Any + Send>
        });

        // Domain 9 audit D9-B - degrade gracefully on poisoned lock
        // rather than propagating the panic into application boot.
        // An app whose scope registry is poisoned has bigger problems
        // than a missing scope; the error log lets ops surface it.
        match registry().write() {
            Ok(mut reg) => {
                reg.entry(model_type_id)
                    .or_insert_with(|| PerModelScopes {
                        entries: Vec::new(),
                    })
                    .entries
                    .push(ScopeEntry {
                        scope_type_id,
                        apply,
                        dependency,
                        diagnostic_name,
                    });
            }
            Err(_) => {
                tracing::error!(
                    model_type = std::any::type_name::<M>(),
                    scope_type = std::any::type_name::<S>(),
                    "ScopeRegistry write lock poisoned; skipping scope \
                     registration. Queries against this model will not \
                     have the scope applied."
                );
            }
        }
    }

    /// Phase 10C audit-fix AF4 - wipe every registered global scope.
    /// `#[doc(hidden)]` because this is a test-only escape hatch
    /// (mirrors [`crate::database::ConnectionRegistry::clear`]).
    /// Called from [`crate::testing::TestContainerGuard::drop`] so
    /// the next test in the same process starts with an empty scope
    /// registry. Production code never calls this - global scopes
    /// register at boot and live for the process lifetime.
    #[doc(hidden)]
    pub fn clear() {
        if let Some(lock) = REGISTRY.get()
            && let Ok(mut reg) = lock.write()
        {
            reg.clear();
        }
    }

    /// Apply every registered scope for `M` to `builder`. Skips any
    /// scope whose `TypeId` appears in `builder.excluded_scopes`.
    /// Returns `builder` unchanged when `builder.skip_all_scopes` is
    /// set or no scopes are registered for `M`.
    ///
    /// Public so the `#[suprnova::model]` macro can emit calls into
    /// user crates (the soft-delete `query()` override invokes
    /// `apply_to` after layering on `filter_null(deleted_at)`). End
    /// users go through `Model::query()` which dispatches here
    /// automatically; calling it directly is unusual but supported.
    pub fn apply_to<M>(builder: Builder<M>) -> Builder<M>
    where
        M: Model + 'static,
        M: From<<M::Entity as EntityTrait>::Model>,
        <M::Entity as EntityTrait>::Model: From<M>
            + IntoActiveModel<<M::Entity as EntityTrait>::ActiveModel>
            + Serialize
            + Send
            + Sync,
        <M::Entity as EntityTrait>::ActiveModel: Send,
        <<M::Entity as EntityTrait>::PrimaryKey as PrimaryKeyTrait>::ValueType:
            Send + Into<sea_orm::Value>,
    {
        if builder.skip_all_scopes {
            return builder;
        }

        // Snapshot the (cheap, Arc-cloned) per-model scope list under
        // the read lock, then release the lock before running user
        // code so a scope that itself touches the registry doesn't
        // deadlock.
        //
        // Domain 9 audit D9-B - degrade to no-scope-applied on
        // poison. Returning the unscoped builder preserves the
        // documented "no scope registered = query unchanged"
        // semantic; an error log lets ops see the underlying poison.
        let entries: Vec<ScopeEntry> = match registry().read() {
            Ok(reg) => match reg.get(&TypeId::of::<M>()) {
                Some(p) => p.entries.clone(),
                None => return builder,
            },
            Err(_) => {
                tracing::error!(
                    model_type = std::any::type_name::<M>(),
                    "ScopeRegistry read lock poisoned during apply_to; \
                     returning unscoped builder."
                );
                return builder;
            }
        };

        let excluded = builder.excluded_scopes.clone();
        let mut current: Box<dyn Any + Send> = Box::new(builder);
        // Read once: a collector either is active for this whole call or is
        // not, and `is_active` is the cheap check every read hook makes
        // before doing any work at all.
        let collecting = crate::render_cache::collector::is_active();
        for entry in entries {
            if excluded.contains(&entry.scope_type_id) {
                continue;
            }
            let before = if collecting {
                crate::render_cache::collector::resolvable_reads()
            } else {
                0
            };
            current = (entry.apply)(current);
            if collecting
                && entry.dependency == ScopeDependency::PerRequest
                && crate::render_cache::collector::resolvable_reads() == before
            {
                // A scope that said it reads per-request state and then read
                // nothing the collector can name: the filter is real, the
                // dependency is invisible, and the only safe answer is to
                // refuse to store the render and say which scope did it.
                crate::render_cache::collector::observe_undeclared(&entry.diagnostic_name);
            }
        }
        *current
            .downcast::<Builder<M>>()
            .expect("ScopeRegistry: apply pipeline preserved Builder<M> type")
    }

    /// Test-only escape hatch. Drops every registered scope. Used by
    /// the framework's own `#[cfg(test)]` blocks to keep test
    /// isolation simple; production code should NEVER call this.
    ///
    /// Mirrors [`Self::clear`] (the `#[doc(hidden)]` opt-in)'s poison
    /// handling - silently skip on poison rather than propagate.
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn __clear_for_tests() {
        if let Ok(mut reg) = registry().write() {
            reg.clear();
        }
    }
}
