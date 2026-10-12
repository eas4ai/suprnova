//! Lazy-loading prevention: Laravel's `Model::preventLazyLoading()`.
//!
//! A relation read through its relation method, such as
//! `post.author().first()` or `user.posts().get()`, runs one query for
//! one model. In a loop over the rows of one query that is the N+1
//! problem: a list of 50 posts whose template reads the author of each
//! post runs 51 queries. The eager loaders (`with([...])` on the query,
//! `load([...])` and `load_missing([...])` on a model or a collection)
//! read the same relation for every row with one query.
//!
//! Turn the process-wide switch on at boot outside production, and an
//! N+1 fails where it is written instead of under load:
//!
//! ```rust,no_run
//! use suprnova::eloquent::prevent_lazy_loading;
//!
//! # let production = false;
//! prevent_lazy_loading(!production);
//! ```
//!
//! ## What is a violation
//!
//! With the switch on, a read of a relation is refused when both hold:
//!
//! - the model came out of a query that returned more than one row: a
//!   `get`, `all` or `find_many`, a page of a paginator, a batch of a
//!   chunk or a lazy walk (`chunk`, `lazy`, `cursor`), a row of `each`
//!   over a query that matches more than one row, the rows of a through
//!   relation, or the related rows of an eager load that fetched more
//!   than one. The last batch of a chunk walk that holds one row is not
//!   marked, as a `get` of one row is not;
//! - the rows of the relation are not in the relation cache of the
//!   model, because it was not loaded with `with`, `load` or
//!   `load_missing`.
//!
//! The reads are `get()` and `first()` of every relation kind, and
//! `get()` of a `MorphTo` relation. A model from `find`, `first` or a
//! query that returned one row, a model built or created in the
//! process, and a loaded relation are never refused, as in Laravel.
//! `count()` of a relation loads no model and is not checked; its eager
//! form is `with_count`.
//!
//! ## What a violation does
//!
//! By default the read returns an error and runs no query. The error
//! names the model and the relation and says how to load the relation
//! eagerly. It holds no key and no value of the row.
//!
//! With a handler registered through [`handle_lazy_loading_violation`],
//! the handler is called and the read goes on. That is the mode for
//! staging, where a violation is logged rather than failed:
//!
//! ```rust,no_run
//! use suprnova::eloquent::{handle_lazy_loading_violation, prevent_lazy_loading};
//!
//! # fn boot() -> Result<(), suprnova::FrameworkError> {
//! prevent_lazy_loading(true);
//! handle_lazy_loading_violation(|violation| {
//!     eprintln!(
//!         "lazy loading of `{}` on `{}`",
//!         violation.relation, violation.model,
//!     );
//! })?;
//! # Ok(()) }
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use crate::eloquent::relations::EagerLoadCache;
use crate::error::FrameworkError;

/// Process-wide switch. Off by default, so a relation read behaves the
/// same with or without this module until an application turns it on.
static PREVENT_LAZY_LOADING: AtomicBool = AtomicBool::new(false);

/// The handler [`handle_lazy_loading_violation`] registers.
type ViolationHandler = Arc<dyn Fn(&LazyLoadingViolation) + Send + Sync>;

/// The registered handler, if any.
///
/// Only code of this module runs under the guard: the handler is cloned
/// out before it is called, and a replaced handler is dropped after the
/// guard. A handler that panics therefore poisons nothing.
static VIOLATION_HANDLER: RwLock<Option<ViolationHandler>> = RwLock::new(None);

/// The name the lock helpers put in the error of a poisoned lock.
const HANDLER_LOCK: &str = "lazy loading violation handler";

/// Turn lazy-loading prevention on or off for the whole process.
///
/// Laravel's `Model::preventLazyLoading()`. Call it once at boot,
/// typically with `true` everywhere but production. See the
/// [module documentation](crate::eloquent::lazy_loading) for what a
/// violation is.
pub fn prevent_lazy_loading(enabled: bool) {
    PREVENT_LAZY_LOADING.store(enabled, Ordering::SeqCst);
}

/// Whether lazy-loading prevention is on. Reads the flag
/// [`prevent_lazy_loading`] writes.
pub fn preventing_lazy_loading() -> bool {
    PREVENT_LAZY_LOADING.load(Ordering::SeqCst)
}

/// A relation read that lazy-loading prevention caught: the relation
/// was read on a model of a multi-row query without being loaded.
///
/// Handed to the handler of [`handle_lazy_loading_violation`]. It names
/// the two and nothing else, so a handler that logs it writes no data of
/// the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct LazyLoadingViolation {
    /// The model the relation was read on: the name of its struct, such
    /// as `"Post"`.
    pub model: &'static str,
    /// The relation, as `relations = { ... }` declares it, such as
    /// `"author"`.
    pub relation: &'static str,
}

impl LazyLoadingViolation {
    /// The error a violation is when no handler is registered.
    fn refusal(&self) -> FrameworkError {
        let LazyLoadingViolation { model, relation } = *self;
        FrameworkError::internal(format!(
            "lazy loading of the relation `{relation}` of `{model}` is prevented: this \
             `{model}` came from a query that returned more than one row, so reading \
             `{relation}` runs one query per row. Load it for every row at once with \
             `.with([\"{relation}\"])` on the query, or `.load([\"{relation}\"])` on the \
             collection"
        ))
    }
}

/// Register the handler of lazy-loading violations for the whole
/// process, in place of any handler registered before.
///
/// Laravel's `Model::handleLazyLoadingViolationUsing()`. With a handler
/// registered, a violation calls it and the read goes on and runs its
/// query; without one, the read returns an error. The handler is only
/// called while [`prevent_lazy_loading`] is on.
///
/// The handler runs on the task of the read, with no lock of the
/// framework held.
///
/// # Errors
///
/// [`FrameworkError::internal`] if the lock of the handler is poisoned.
pub fn handle_lazy_loading_violation<F>(handler: F) -> Result<(), FrameworkError>
where
    F: Fn(&LazyLoadingViolation) + Send + Sync + 'static,
{
    let handler: ViolationHandler = Arc::new(handler);
    replace_handler(Some(handler))
}

/// Remove the handler [`handle_lazy_loading_violation`] registered, so
/// a violation is an error again.
///
/// # Errors
///
/// [`FrameworkError::internal`] if the lock of the handler is poisoned.
pub fn clear_lazy_loading_violation_handler() -> Result<(), FrameworkError> {
    replace_handler(None)
}

/// Put `handler` in the slot. The handler it replaces is dropped after
/// the guard, because what it captured is code of the application and
/// its destructor must not run under the lock.
fn replace_handler(handler: Option<ViolationHandler>) -> Result<(), FrameworkError> {
    let replaced = {
        let mut slot = crate::lock::write(&VIOLATION_HANDLER, HANDLER_LOCK)?;
        std::mem::replace(&mut *slot, handler)
    };
    drop(replaced);
    Ok(())
}

/// The check a relation read runs before its query.
///
/// The relation method the `#[suprnova::model]` macro emits builds one
/// with [`Self::for_relation`] and hands it to the relation it returns;
/// the read methods of every relation kind call [`Self::check`] first.
/// The model is known only when the relation is built, and the relation
/// runs its query later, so the finding travels with the relation.
///
/// **Not part of the public API.** It is `pub` because code the macro
/// emits names it.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, Default)]
pub struct LazyLoadGuard {
    /// `Some` when a read of the relation would be a violation while
    /// the switch is on. The switch itself is read by [`Self::check`], at
    /// the time of the read.
    violation: Option<LazyLoadingViolation>,
}

impl LazyLoadGuard {
    /// The check for a read of `relation` on the model whose relation
    /// cache is `cache`. `model` is the name of the struct of the model.
    ///
    /// A read is a violation when the model came from a query that
    /// returned more than one row and the rows of the relation are not
    /// in its cache.
    #[doc(hidden)]
    pub fn for_relation(
        cache: &EagerLoadCache,
        model: &'static str,
        relation: &'static str,
    ) -> Self {
        let refused = cache.is_from_multi_row_query() && !cache.has_rows(relation);
        Self {
            violation: refused.then_some(LazyLoadingViolation { model, relation }),
        }
    }

    /// Run the check. `Ok` when the read is no violation or the switch
    /// is off. On a violation, the registered handler is called and the
    /// result is `Ok`; with no handler the result is the error, and the
    /// caller runs no query.
    ///
    /// # Errors
    ///
    /// [`FrameworkError::internal`] for a violation with no handler, or
    /// when the lock of the handler is poisoned.
    #[doc(hidden)]
    pub fn check(self) -> Result<(), FrameworkError> {
        let Some(violation) = self.violation else {
            return Ok(());
        };
        if !preventing_lazy_loading() {
            return Ok(());
        }
        // Cloned out of the lock, so the handler runs with no guard held.
        let handler = crate::lock::read(&VIOLATION_HANDLER, HANDLER_LOCK)?.clone();
        match handler {
            Some(handler) => {
                handler(&violation);
                Ok(())
            }
            None => Err(violation.refusal()),
        }
    }
}
