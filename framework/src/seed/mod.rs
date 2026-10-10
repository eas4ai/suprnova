//! Database seeders - process-global registry of ordered fixture
//! runners.
//!
//! Each seeder is a zero-sized type that implements [`Seeder`]; the
//! framework stores a function pointer per registered seeder and
//! runs them in **registration order** via [`run_all`]. Insertion
//! order matters because seeders typically have implicit dependencies
//! (users before posts, posts before comments).
//!
//! ```rust,no_run
//! use suprnova::{async_trait, FrameworkError, Seeder};
//! # struct UserFactory;
//! # impl UserFactory {
//! #     fn new() -> Self { UserFactory }
//! #     fn count(self, _n: usize) -> Self { self }
//! #     async fn create_many(self) -> Result<(), FrameworkError> { Ok(()) }
//! # }
//!
//! pub struct UsersSeeder;
//!
//! #[async_trait]
//! impl Seeder for UsersSeeder {
//!     fn name() -> &'static str { "UsersSeeder" }
//!     async fn run() -> Result<(), FrameworkError> {
//!         UserFactory::new().count(50).create_many().await?;
//!         Ok(())
//!     }
//! }
//!
//! # async fn bootstrap() -> Result<(), Box<dyn std::error::Error>> {
//! // In bootstrap:
//! suprnova::seed::register::<UsersSeeder>();
//!
//! // Later (e.g. the `db:seed` console command):
//! suprnova::seed::run_all().await?;
//! # Ok(()) }
//! ```
//!
//! # Registry semantics
//!
//! Matches the Phase 5B registries (`register_mailable_factory`,
//! `register_notification_factory`, `register_mail_renderer`):
//!
//! - The lazily initialized registry holds an `IndexMap<String, SeederFn>`
//!   and an optional root name. Entries stay in registration order, last-write-wins
//!   on the seeder name (re-registering the same name silently
//!   replaces the function pointer so tests can swap stubs).
//! - The trait method `Seeder::run()` is an associated function (no
//!   `&self`) because seeders are stateless side-effect runners; the
//!   `where Self: Sized` clause keeps the trait off the object-safe
//!   path, but the registry stores type-erased fn pointers anyway so
//!   object safety is not needed.
//!
//! # Selective execution (`run_one`)
//!
//! [`run_one`] looks up a single registered seeder by its stable
//! `name()` and runs it without running its peers. This is the
//! engine for `db:seed --class=<Name>` - the Laravel-side
//! `php artisan db:seed --class=UserSeeder` ergonomic. Lookup misses
//! return `Err(FrameworkError::not_found(...))` so the CLI surfaces
//! "no seeder registered for X" rather than silently succeeding.
//!
//! # Model-event muting (`without_events`)
//!
//! [`without_events`] is the Laravel-`WithoutModelEvents` analogue.
//! A `tokio::task_local!` flag is set for the duration of the passed
//! future; [`crate::eloquent::events::dispatch_after`] and
//! [`crate::eloquent::events::dispatch_cancellable`] check the flag
//! and short-circuit to `Ok(())` when it is set. That single check
//! at each chokepoint covers every model lifecycle event (both
//! cancellable and non-cancellable). The effect is task-scoped -
//! only seeders that opt in are affected, and the application's
//! own HTTP request paths continue to fire events normally. Nested
//! calls compose (the inner future inherits the outer flag).
//!
//! This covers `Model` writes and factories backed by Eloquent models.
//! A factory backed directly by a SeaORM model uses `ActiveModelTrait::insert`
//! without the Eloquent event path. See [`without_events`] for the distinction.

use crate::error::FrameworkError;
use crate::lock;
use async_trait::async_trait;
use futures::future::BoxFuture;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::collections::HashSet;
use std::future::Future;
use std::sync::RwLock;
use std::time::Instant;

/// Function-pointer view of a registered seeder. Captures the type
/// parameter through a closure produced in [`register`].
type SeederFn = fn(SeederParams) -> BoxFuture<'static, Result<(), FrameworkError>>;

/// Named values passed to a seeder so callers can reuse it with different inputs.
pub type SeederParams = crate::Attrs;

#[derive(Default)]
struct Registry {
    entries: IndexMap<String, SeederFn>,
    root: Option<String>,
}

static REGISTRY: RwLock<Option<Registry>> = RwLock::new(None);

#[derive(Default)]
struct Invocation {
    completed: HashSet<String>,
    running: HashSet<String>,
}

tokio::task_local! {
    static INVOCATION: RefCell<Invocation>;
}

/// Shares successful calls across nested seeders and resets them for each top-level run.
pub(crate) async fn with_invocation<F: Future>(future: F) -> F::Output {
    if INVOCATION.try_with(|_| ()).is_ok() {
        future.await
    } else {
        INVOCATION
            .scope(RefCell::new(Invocation::default()), future)
            .await
    }
}

struct RunningSeeder(String);

impl Drop for RunningSeeder {
    fn drop(&mut self) {
        let _ = INVOCATION.try_with(|state| state.borrow_mut().running.remove(&self.0));
    }
}

tokio::task_local! {
    /// When set to `true`, [`crate::eloquent::events::dispatch_after`]
    /// and [`crate::eloquent::events::dispatch_cancellable`] short-
    /// circuit to `Ok(())` without invoking listeners. Established by
    /// [`without_events`] for the duration of the passed future.
    pub(crate) static EVENTS_MUTED: bool;
}

/// A database seeder - runs once via [`run_all`] to populate fixture
/// data. Seeders carry no per-instance state; the trait surface is a
/// stable name + an async run method that returns a Result.
#[async_trait]
pub trait Seeder: Send + Sync {
    /// Stable name used as the registry key. Re-registering the same
    /// name silently replaces the prior seeder (last-write-wins) -
    /// matches the Phase 5B factory registries' contract.
    fn name() -> &'static str
    where
        Self: Sized;

    /// Run the seeder. Idempotency is the seeder's responsibility -
    /// `run_all` does not snapshot or roll back, so a seeder that
    /// inserts unconditionally will produce duplicates on re-run.
    async fn run() -> Result<(), FrameworkError>
    where
        Self: Sized;

    /// Receives named inputs without changing existing `run()` implementations.
    /// Override this when your seeder reads parameters; the default calls `run()`.
    async fn run_with(_params: SeederParams) -> Result<(), FrameworkError>
    where
        Self: Sized,
    {
        Self::run().await
    }
}

/// Register a seeder type. Inserts it into the global registry under
/// its `name()`. Order matters - `run_all` visits seeders in the
/// order they were registered. Re-registering a name replaces the
/// prior function pointer in-place (IndexMap preserves the original
/// position, so test stubs slot in cleanly).
pub fn register<S: Seeder + 'static>() {
    let f: SeederFn = |params| Box::pin(S::run_with(params));
    match lock::write(&REGISTRY, "seeder registry") {
        Ok(mut g) => {
            g.get_or_insert_with(Registry::default)
                .entries
                .insert(S::name().to_string(), f);
        }
        Err(_) => {
            tracing::error!(
                seeder = S::name(),
                "Seeder registry lock poisoned; skipping registration."
            );
        }
    }
}

/// Registers the root so `db:seed` runs its ordering instead of every peer.
/// Child seeders still need `register`. Replacing the root selects the new root.
pub fn register_root<S: Seeder + 'static>() -> Result<(), FrameworkError> {
    let mut registry = lock::write(&REGISTRY, "seeder registry")?;
    let registry = registry.get_or_insert_with(Registry::default);
    registry
        .entries
        .insert(S::name().to_owned(), |params| Box::pin(S::run_with(params)));
    registry.root = Some(S::name().to_owned());
    Ok(())
}

/// Runs every peer for applications that explicitly want registration order.
/// Stops on error without rolling back earlier work. Nested calls share once tracking.
pub async fn run_all() -> Result<(), FrameworkError> {
    let names: Vec<String> = {
        let registry = lock::read(&REGISTRY, "seeder registry")?;
        registry
            .as_ref()
            .map(|registry| registry.entries.keys().cloned().collect())
            .unwrap_or_default()
    };
    call_silent(names).await
}

/// Runs the registered root so it owns child order and selection.
/// Applications without an explicit root retain the legacy `run_all` default.
pub async fn run_root() -> Result<(), FrameworkError> {
    let root = {
        let registry = lock::read(&REGISTRY, "seeder registry")?;
        registry.as_ref().and_then(|registry| registry.root.clone())
    };
    match root {
        Some(name) => run_one(&name).await,
        None => run_all().await,
    }
}

/// Runs one named seeder without progress so existing direct calls stay quiet.
/// An unknown name fails before the seeder or its output runs.
pub async fn run_one(name: &str) -> Result<(), FrameworkError> {
    with_invocation(execute(name, SeederParams::new(), true, false)).await
}

/// Calls named seeders in order and reports their progress through console output.
/// A failed seeder stops the list and has no DONE line; earlier work remains.
pub async fn call<I, S>(names: I) -> Result<(), FrameworkError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    call_list(names, false, false).await
}

/// Calls named seeders without progress when you need a quiet composition.
pub async fn call_silent<I, S>(names: I) -> Result<(), FrameworkError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    call_list(names, true, false).await
}

/// Skips seeders already successful in this invocation, including ordinary calls.
/// Failed runs can be retried. A new `db:seed` invocation starts with an empty set.
pub async fn call_once<I, S>(names: I) -> Result<(), FrameworkError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    call_list(names, false, true).await
}

/// Passes named inputs to `Seeder::run_with` so one seeder serves several callers.
pub async fn call_with(name: &str, params: SeederParams) -> Result<(), FrameworkError> {
    with_invocation(execute(name, params, false, false)).await
}

async fn call_list<I, S>(names: I, silent: bool, once: bool) -> Result<(), FrameworkError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    with_invocation(async {
        for name in names {
            execute(name.as_ref(), SeederParams::new(), silent, once).await?;
        }
        Ok(())
    })
    .await
}

async fn execute(
    name: &str,
    params: SeederParams,
    silent: bool,
    once: bool,
) -> Result<(), FrameworkError> {
    let skip = INVOCATION.with(|state| once && state.borrow().completed.contains(name));
    if skip {
        return Ok(());
    }
    let function = {
        let registry = lock::read(&REGISTRY, "seeder registry")?;
        registry
            .as_ref()
            .and_then(|registry| registry.entries.get(name).copied())
    }
    .ok_or_else(|| FrameworkError::not_found(format!("no seeder registered for `{name}`")))?;
    let recursive = INVOCATION.with(|state| !state.borrow_mut().running.insert(name.to_owned()));
    if recursive {
        return Err(FrameworkError::bad_request(format!(
            "recursive seeder call to `{name}`"
        )));
    }
    let _running = RunningSeeder(name.to_owned());
    tracing::info!(seeder = %name, "running seeder");
    if !silent {
        crate::console::line(format!("RUNNING {name}"));
    }
    let started = Instant::now();
    function(params).await?;
    INVOCATION.with(|state| state.borrow_mut().completed.insert(name.to_owned()));
    if !silent {
        crate::console::line(format!(
            "DONE {name} ({} ms)",
            started.elapsed().as_millis()
        ));
    }
    Ok(())
}

/// Number of currently-registered seeders. Useful for tests asserting
/// "the bootstrap registered all expected seeders."
///
/// Returns `0` on registry-lock poison after logging an error -
/// matches the "treat poison as empty" pattern used by the
/// registration path.
pub fn count() -> usize {
    try_count().unwrap_or_else(|_| {
        tracing::error!("Seeder registry lock poisoned; reporting count=0.");
        0
    })
}

/// [`count`], with a poisoned registry as the error it is. `db:seed`
/// decides on this whether there is anything to run, and a registry it
/// cannot read is not an empty one.
pub(crate) fn try_count() -> Result<usize, FrameworkError> {
    let g = lock::read(&REGISTRY, "seeder registry")?;
    Ok(g.as_ref().map(|m| m.entries.len()).unwrap_or(0))
}

/// Whether a seeder with the given name is registered.
///
/// Used by `db:seed --class=` argument validation and by tests
/// asserting that bootstrap registered the expected fixtures.
/// Returns `false` on registry-lock poison (matches `count()`).
pub fn is_registered(name: &str) -> bool {
    match lock::read(&REGISTRY, "seeder registry") {
        Ok(g) => g.as_ref().is_some_and(|m| m.entries.contains_key(name)),
        Err(_) => {
            tracing::error!("Seeder registry lock poisoned; reporting is_registered=false.");
            false
        }
    }
}

/// Run `fut` with Eloquent model events muted.
///
/// The Laravel-`WithoutModelEvents` analogue. While the future is
/// awaiting, both [`crate::eloquent::events::dispatch_after`] and
/// [`crate::eloquent::events::dispatch_cancellable`] short-circuit
/// to `Ok(())` - covering every model lifecycle event (both
/// cancellable and non-cancellable) at its single chokepoint.
///
/// The effect is **task-scoped**: only the work performed inside
/// `fut` is muted; concurrent work on other tasks (HTTP request
/// handlers, other seeders, queue workers) continues to fire
/// events normally. Nested calls compose - the inner future
/// inherits the outer flag.
///
/// # When is this useful?
///
/// Model-driven inserts. A seeder that calls `User::create(...)` in
/// a loop fires `Creating` / `Saving` / `Created` / `Saved` on every
/// row, which invokes any registered `Observer<User>` and any
/// queued broadcast listeners. Wrapping that loop in
/// `seed::without_events` skips both the per-row cancellable veto
/// path and the per-row after-event fanout - handy for bulk seeds
/// that don't want to wake the broadcaster or trigger downstream
/// jobs.
///
/// Factories returning an Eloquent model dispatch the same events as
/// `Model::create`, so this helper mutes those inserts too. Factories
/// returning a raw SeaORM model use `ActiveModelTrait::insert` without
/// Eloquent events and need no muting.
///
/// # Example
///
/// ```rust,no_run
/// use suprnova::{seed, async_trait, FrameworkError, Seeder};
/// # #[derive(Default)]
/// # struct User { id: u64, name: String }
/// # impl User {
/// #     async fn create(_user: User) -> Result<(), FrameworkError> { Ok(()) }
/// # }
///
/// pub struct UsersSeeder;
///
/// #[async_trait]
/// impl Seeder for UsersSeeder {
///     fn name() -> &'static str { "UsersSeeder" }
///     async fn run() -> Result<(), FrameworkError> {
///         seed::without_events(async {
///             // Loop of Model::create calls - each would normally
///             // fire Creating/Saving/Created/Saved. Muted here.
///             for i in 0..50 {
///                 User::create(User {
///                     id: 0, name: format!("user{i}"), ..Default::default()
///                 }).await?;
///             }
///             Ok(())
///         }).await
///     }
/// }
/// ```
pub async fn without_events<F, T>(fut: F) -> T
where
    F: Future<Output = T>,
{
    EVENTS_MUTED.scope(true, fut).await
}

/// Returns `true` if the current task is executing inside a
/// [`without_events`] scope.
///
/// Used by the Eloquent event dispatch sites
/// ([`crate::eloquent::events::dispatch_after`] and
/// [`crate::eloquent::events::dispatch_cancellable`]) to decide
/// whether to short-circuit. Not exposed at the crate root - user
/// code should never need to check this directly; opting into
/// [`without_events`] is the public surface.
pub(crate) fn events_muted() -> bool {
    EVENTS_MUTED.try_with(|m| *m).unwrap_or(false)
}

/// Forget every registered seeder, so a test starts from a registry it
/// filled itself.
///
/// The registry is one for the process, and the guard of the test
/// container does not reset it: clearing a registry is a thing a test
/// asks for. A test that registers a seeder calls this first, and again
/// when it is done, so the test that runs behind it does not run the
/// seeders of this one. The tests that use the registry run one at a
/// time, because they share it.
///
/// Compiled with the `testing` feature alone. An application registers
/// its seeders once, when it boots, and has nothing to clear.
///
/// A poisoned registry is left as it is, without an error, as the other
/// resets of the test helpers do.
#[cfg(any(test, feature = "testing"))]
pub fn clear() {
    if let Ok(mut g) = lock::write(&REGISTRY, "seeder registry") {
        *g = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clears the poison the test below puts on the registry, whether the
    /// test passes or not, so the tests that run after it see a usable one.
    struct ClearPoison;

    impl Drop for ClearPoison {
        fn drop(&mut self) {
            REGISTRY.clear_poison();
        }
    }

    /// A bare `db:seed` against a registry it cannot read fails. It used to
    /// read the poisoned registry as empty and report "nothing to run".
    #[tokio::test]
    async fn db_seed_reports_a_poisoned_registry_instead_of_nothing_to_run() {
        let _clear = ClearPoison;
        let poisoner = std::thread::spawn(|| {
            let _held = REGISTRY.write();
            panic!("poison the seeder registry on purpose");
        });
        assert!(poisoner.join().is_err(), "the registry is poisoned");

        // `--force`, since an unset APP_ENV is production, where a bare
        // `db:seed` stops before it reads the registry.
        let argv = vec![
            "console".to_string(),
            "db:seed".to_string(),
            "--force".to_string(),
        ];
        let err = crate::console::dispatch_argv(argv)
            .await
            .expect_err("a registry that cannot be read must fail the command");
        assert!(
            err.to_string().contains("seeder registry lock poisoned"),
            "got: {err}"
        );
    }
}
