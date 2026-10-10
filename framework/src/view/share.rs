//! Data shared with every server-rendered view, Laravel's `View::share`.
//!
//! A view reads its own fields by name. Shared data is a second channel: a
//! value every render can read through Askama's runtime values, with the
//! `value` filter, so a layout can show the application's name or the
//! signed-in user without every view struct carrying a field for it.
//!
//! The application's values live in the application container, so a test
//! under `TestContainer::fake()` sees only what it shared itself. A
//! request's values live in the request's container scope, a task-local
//! the framework opens for each request, so one request never sees
//! another's, even while both render at once.

use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use crate::FrameworkError;
use crate::container::{App, TASK_CONTAINER, TEST_CONTAINER};

/// A shared value, type-erased the way the container stores values.
type SharedValue = Arc<dyn Any + Send + Sync>;

/// The application's shared values: one per application container.
#[derive(Clone, Default)]
struct ApplicationShares(Arc<RwLock<BTreeMap<String, SharedValue>>>);

/// One request's shared values: one per container scope.
#[derive(Default)]
struct RequestShares(Mutex<BTreeMap<String, SharedValue>>);

/// Laravel's `View` facade for data shared with every server-rendered
/// view.
///
/// ```rust,no_run
/// use suprnova::{FrameworkError, Request, View};
///
/// // At boot: every view can read `app_name`.
/// View::share("app_name", "Acme".to_string());
///
/// // In a middleware or handler: only this request's views see `viewer`.
/// fn share_viewer(request: &Request) -> Result<(), FrameworkError> {
///     if let Some(name) = request.header("X-Viewer") {
///         View::share_for_request("viewer", name.to_string())?;
///     }
///     Ok(())
/// }
/// ```
///
/// A template reads a shared value with Askama's `value` filter, naming
/// the type that was shared. A value no one shared fails the render, so
/// test for it where it can be missing:
///
/// ```text
/// {% if let Ok(name) = "app_name"|value::<String> %}{{ name }}{% endif %}
/// ```
pub struct View;

impl View {
    /// Share `value` under `key` with every view the application renders
    /// from now on. A later share of the same key replaces it.
    ///
    /// The value belongs to the application container: under
    /// `TestContainer::fake()` it belongs to that test's container and is
    /// gone with it.
    pub fn share<T: Any + Send + Sync>(key: impl Into<String>, value: T) {
        let shares = application_shares(true).unwrap_or_default();
        shares
            .0
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.into(), Arc::new(value));
    }

    /// Share `value` under `key` with the views the current request
    /// renders, and with no other request's. It wins over a value the
    /// application shared under the same key.
    ///
    /// A request runs in a container scope the framework opens for it, and
    /// the value lives in that scope. A page formed from a value shared this
    /// way, read in a template or through [`View::shared`], is never stored
    /// in the render cache, so it cannot be served to another visitor.
    ///
    /// # Errors
    ///
    /// Returns an error when no container scope is active: outside a
    /// request, a job, a command or `App::run_scoped`.
    pub fn share_for_request<T: Any + Send + Sync>(
        key: impl Into<String>,
        value: T,
    ) -> Result<(), FrameworkError> {
        let shares = request_shares()?;
        shares
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.into(), Arc::new(value));
        Ok(())
    }

    /// The value a render would read under `key`: the request's value when
    /// the current request shared one, else the application's. `None` when
    /// neither shared the key, or the value is not a `T`.
    ///
    /// When the current request shared `key`, the answer depends on that
    /// request alone, even when it is `None` because the value is not a
    /// `T`. No render cache key names it, so the render cache does not
    /// store a page formed while this answer was read, the same as for a
    /// template that reads the value. An answer from the application's
    /// values is the same for every request and leaves the page storable.
    pub fn shared<T: Any + Send + Sync>(key: &str) -> Option<Arc<T>> {
        let request = scoped_request_value::<RequestShares>().and_then(|shares| {
            shares
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(key)
                .cloned()
        });
        let value = match request {
            Some(value) => {
                crate::render_cache::collector::observe_unobservable_read();
                value
            }
            None => application_shares(false).and_then(|shares| {
                shares
                    .0
                    .read()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(key)
                    .cloned()
            })?,
        };
        value.downcast::<T>().ok()
    }
}

/// The application's shares in the active container; see
/// [`application_value`].
fn application_shares(create: bool) -> Option<ApplicationShares> {
    application_value(create)
}

/// The `T` of the active container: the test container when a test
/// installed one, the application container otherwise. With `create`, a
/// container without one gets `T::default()`. `T` shares its state between
/// clones, so a value written through one clone is read through another.
///
/// The test container is read on its own, never falling back to the
/// application container as `App::get` does, so a test never writes into
/// or reads from a value the process's application container holds.
pub(super) fn application_value<T>(create: bool) -> Option<T>
where
    T: Any + Clone + Default + Send + Sync,
{
    let in_container = |container: &mut crate::container::Container| {
        container.get::<T>().or_else(|| {
            create.then(|| {
                let value = T::default();
                container.singleton(value.clone());
                value
            })
        })
    };
    if let Ok(found) = TASK_CONTAINER.try_with(|container| {
        in_container(&mut container.write().unwrap_or_else(PoisonError::into_inner))
    }) {
        return found;
    }
    if let Some(found) = TEST_CONTAINER.with(|slot| slot.borrow_mut().as_mut().map(in_container)) {
        return found;
    }
    // Two first writes at once must not each register a value and lose the
    // other's write.
    static REGISTER: Mutex<()> = Mutex::new(());
    let _register = REGISTER.lock().unwrap_or_else(PoisonError::into_inner);
    App::get::<T>().or_else(|| {
        create.then(|| {
            let value = T::default();
            App::singleton(value.clone());
            value
        })
    })
}

/// The current container scope's shares.
fn request_shares() -> Result<Arc<RequestShares>, FrameworkError> {
    request_value()
}

/// The `T` of the current container scope, which the framework opens for
/// each request: `T::default()` on the first call in the scope.
///
/// # Errors
///
/// Returns an error when no container scope is active.
pub(super) fn request_value<T>() -> Result<Arc<T>, FrameworkError>
where
    T: Any + Default + Send + Sync,
{
    let value =
        crate::container::scope::resolve(TypeId::of::<T>(), std::any::type_name::<T>(), &|| {
            Arc::new(T::default()) as SharedValue
        })?;
    value
        .downcast::<T>()
        .map_err(|_| FrameworkError::internal("a request-scoped view value has another type"))
}

/// [`request_value`], or `None` outside a container scope, without
/// building the error.
pub(super) fn scoped_request_value<T>() -> Option<Arc<T>>
where
    T: Any + Default + Send + Sync,
{
    crate::container::scope::ContainerScope::current()?;
    request_value().ok()
}

/// The runtime values one render reads: a copy of the request's shares
/// and the application's, taken when the render starts.
pub(super) struct RenderValues {
    request: BTreeMap<String, SharedValue>,
    application: BTreeMap<String, SharedValue>,
}

impl RenderValues {
    /// The values a render that starts now reads.
    pub(super) fn current() -> Self {
        // Outside a container scope there are no request values; checking
        // first keeps a render there from building an error it drops.
        let request = scoped_request_value::<RequestShares>()
            .map(|shares| {
                shares
                    .0
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone()
            })
            .unwrap_or_default();
        let application = application_shares(false)
            .map(|shares| {
                shares
                    .0
                    .read()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone()
            })
            .unwrap_or_default();
        Self {
            request,
            application,
        }
    }
}

impl askama::Values for RenderValues {
    fn get_value<'a>(&'a self, key: &str) -> Option<&'a dyn Any> {
        if let Some(value) = self.request.get(key) {
            // The render read a value of this request alone, which no cache
            // key names: the render cache must not store the page.
            crate::render_cache::collector::observe_unobservable_read();
            return Some(value.as_ref());
        }
        self.application
            .get(key)
            .map(|value| value.as_ref() as &dyn Any)
    }
}
