//! Live components, the islands the pages mount, and the Live routes.
//!
//! `registry()` builds the immutable registry of every Live component;
//! `bootstrap::register()` binds it so the server, the workers, and the
//! `suprnova live:*` commands all see the same components. `mounts()`
//! declares where each island renders: a controller mounts its page's
//! islands from it, and `routes()` seals the same declarations into the
//! router at startup. `routes()` also installs the reserved Live routes
//! behind this application's session authentication, tenant, and rate-limit
//! middleware, and serves the stylesheets and scripts of the library
//! components under `templates/suprnova-ui/`. `routes_with_render_cache()`
//! is what `cmd/main.rs` calls: `routes()` followed by the RenderCache
//! middleware.
//!
//! `suprnova live:make <name>` adds a component module and registers it here.

pub mod forgot_password_form;
pub mod login_form;
pub mod note_form;
pub mod note_list;
pub mod note_stats;
pub mod profile_name;
pub mod register_form;
pub mod reset_password_form;
pub mod verify_email;

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use suprnova::live::action::route_intent;
use suprnova::live::{
    ActionOutcome, ActionResult, CanonicalValue, ComponentError, LiveMount, LiveRegistry,
    LiveTenantMiddleware, LiveTenantResolver, RegistryError,
};
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::render_cache::{RenderCache, RenderCacheConfig};
use suprnova::validator::ValidationErrors;
use suprnova::{
    Auth, AuthMiddleware, FrameworkError, Gate, RateLimitMiddleware, Request, Router,
    SlidingWindowConfig, async_trait,
};

use crate::flash::Toast;
use crate::models::user::User;

/// Builds the registry of every Live component in this application.
pub fn registry() -> Result<LiveRegistry, RegistryError> {
    let registry = LiveRegistry::builder()
        .register::<forgot_password_form::ForgotPasswordForm>()?
        .register::<login_form::LoginForm>()?
        .register::<note_form::NoteForm>()?
        .register::<note_list::NoteList>()?
        .register::<note_stats::NoteStats>()?
        .register::<profile_name::ProfileName>()?
        .register::<register_form::RegisterForm>()?
        .register::<reset_password_form::ResetPasswordForm>()?
        .register::<verify_email::VerifyEmail>()?
        .build();
    Ok(registry)
}

/// Where each island of the kit's pages renders: the page's route, the
/// island's slot, and its key in the document.
///
/// The forms a guest uses are public seeds: any visitor may render one, and
/// it becomes the visitor's own on its first request. Every other island is
/// identity-bound: it belongs to the signed-in user and the session, and its
/// route must authenticate.
pub struct Mounts {
    /// The sign-in form on `/login`.
    pub login: LiveMount<login_form::LoginForm>,
    /// The registration form on `/register`.
    pub register: LiveMount<register_form::RegisterForm>,
    /// The form that asks for a reset link on `/forgot-password`.
    pub forgot_password: LiveMount<forgot_password_form::ForgotPasswordForm>,
    /// The form that takes the new password on `/reset-password`.
    pub reset_password: LiveMount<reset_password_form::ResetPasswordForm>,
    /// The verification notice and its resend button on `/verify-email`.
    pub verify_email: LiveMount<verify_email::VerifyEmail>,
    /// The display-name form on the dashboard.
    pub profile: LiveMount<profile_name::ProfileName>,
    /// The note counts and the five newest notes on the dashboard.
    pub stats: LiveMount<note_stats::NoteStats>,
    /// The form that writes a note on `/notes`.
    pub note_form: LiveMount<note_form::NoteForm>,
    /// The searchable list of notes on `/notes`.
    pub note_list: LiveMount<note_list::NoteList>,
}

impl Mounts {
    fn declare() -> Result<Self, FrameworkError> {
        Ok(Self {
            login: LiveMount::public_seed("/login", "form", "login-form")?,
            register: LiveMount::public_seed("/register", "form", "register-form")?,
            forgot_password: LiveMount::public_seed(
                "/forgot-password",
                "form",
                "forgot-password-form",
            )?,
            reset_password: LiveMount::public_seed(
                "/reset-password",
                "form",
                "reset-password-form",
            )?,
            verify_email: LiveMount::identity_bound("/verify-email", "notice", "verify-email")?,
            profile: LiveMount::identity_bound("/dashboard", "profile", "dashboard-profile")?,
            stats: LiveMount::identity_bound("/dashboard", "stats", "dashboard-stats")?,
            note_form: LiveMount::identity_bound("/notes", "form", "notes-form")?,
            note_list: LiveMount::identity_bound("/notes", "list", "notes-list")?,
        })
    }
}

/// The island declarations, made once on first use. A declaration builds
/// its component's contract, so the router and every request share one
/// instead of rebuilding it per page view.
pub fn mounts() -> Result<&'static Mounts, FrameworkError> {
    static MOUNTS: OnceLock<Mounts> = OnceLock::new();
    if let Some(mounts) = MOUNTS.get() {
        return Ok(mounts);
    }
    let declared = Mounts::declare()?;
    Ok(MOUNTS.get_or_init(|| declared))
}

/// Installs the reserved Live routes and seals every document and island
/// mount into the router.
///
/// The reserved action, upload, and asynchronous routes require a session,
/// a tenant decision, and a rate-limit decision; the guard attaches those
/// middleware to them. Authentication is optional there because the forms
/// on the guest pages act for visitors who are not signed in: a public
/// seed accepts their requests, and an identity-bound island refuses any
/// request without a signed-in user.
pub fn routes(router: Router) -> Result<Router, FrameworkError> {
    let mounts = mounts()?;
    let limiter = Arc::new(InMemoryRateLimiter::new());
    let router = router.try_live_with(|guard| {
        guard
            .middleware(AuthMiddleware::optional())
            .middleware(tenant())
            .middleware(RateLimitMiddleware::new(
                limiter,
                SlidingWindowConfig {
                    max_requests: 600,
                    window: Duration::from_secs(60),
                },
                |request: &Request| {
                    format!("live:{}", request.ip().unwrap_or_else(|| "anon".into()))
                },
            ))
    })?;
    router
        // The stylesheet and script beside each library view under
        // `templates/suprnova-ui/`, which the layouts link.
        .try_live_ui_assets()?
        // Documents that mount no island still render the bootstrap tags.
        .try_live_document("/")?
        .try_live_document("/notes/{id}")?
        .try_live_mount(&mounts.login)?
        .try_live_mount(&mounts.register)?
        .try_live_mount(&mounts.forgot_password)?
        .try_live_mount(&mounts.reset_password)?
        .try_live_mount(&mounts.verify_email)?
        .try_live_mount(&mounts.profile)?
        .try_live_mount(&mounts.stats)?
        .try_live_mount(&mounts.note_form)?
        .try_live_mount(&mounts.note_list)
}

/// `routes()` followed by the RenderCache middleware.
///
/// Separate from `routes()` rather than its last line, because
/// `RenderCache::install` is asynchronous: it probes the database for the
/// generation ledger's tables before it assembles a runtime, so a Migrator
/// that is missing `suprnova::render_cache::migration::Migration` fails once
/// at boot with an actionable message instead of on every request.
/// `cmd/main.rs` reaches this through `Application::try_routes_async`, which
/// prepares services and the immutable Live runtime first. No route here
/// declares a cache policy, so every page renders fresh; see
/// `manual/render-cache.md` to cache one.
///
/// Order matters: `register_global_middleware` appends, so this must run
/// after `bootstrap::register_http_stack`, whose session and locale
/// middleware establish the request-scoped state the cache middleware reads
/// while it builds a lookup key. Set `RENDER_CACHE_ENABLED=false` to turn
/// the cache off; the install then returns the router untouched and probes
/// nothing.
pub async fn routes_with_render_cache(router: Router) -> Result<Router, FrameworkError> {
    RenderCache::install(routes(router)?, RenderCacheConfig::from_env()?).await
}

/// The tenant decision every identity-bound Live document and request
/// carries. The authenticated route group in `routes.rs` adds it.
pub fn tenant() -> LiveTenantMiddleware {
    LiveTenantMiddleware::new(Arc::new(SingleTenant))
}

/// Defines the abilities Live asks the gate for.
///
/// A signed-in user may subscribe to the notes stream of their dashboard.
/// The stream's topic is `notes/:principal`, which Live fills with the
/// user's own id, so the subscription only ever carries that user's
/// updates; the route guard has already required the user.
pub fn authorize() {
    Gate::define::<String, String>(
        "live:{package_name}.note-stats.stream.notes",
        |_principal, _stream| true,
    );
}

/// `path` under the public root, for a link an island renders: `/notes`
/// at the host root, `/billing/notes` behind a proxy that serves the
/// application under `/billing`.
pub fn href(path: &str) -> String {
    format!("{}{path}", suprnova::url::root())
}

/// The outcome of an action that finishes a flow: the browser loads the
/// page of the named route, and the toast, if any, shows there once.
pub(crate) fn redirect(route: &str, toast: Option<Toast>) -> Result<ActionResult, ComponentError> {
    let target = route_intent(route, CanonicalValue::Object(BTreeMap::new()))
        .map_err(|error| failed(&format!("redirecting to the `{route}` route"), error))?;
    if let Some(toast) = toast {
        toast
            .flash()
            .map_err(|error| failed("keeping a toast", error))?;
    }
    Ok(ActionResult::from_outcome(ActionOutcome::Redirect(target)))
}

/// The signed-in user. Every island that reads it is identity-bound and its
/// route sits behind the `auth` middleware, so `None` means the session lost
/// its user between the guard and here; refusing is the safe answer. An
/// action reads the user again on every request rather than trusting an id
/// kept in its state.
pub(crate) async fn current_user() -> Result<User, ComponentError> {
    Auth::user_as::<User>()
        .await
        .map_err(|error| failed("reading the signed-in user", error))?
        .ok_or_else(|| failed("reading the signed-in user", "no user is signed in"))
}

/// Runs `work` on its own Tokio task and waits for its result.
///
/// An action already runs deep in its worker thread's stack: the server,
/// the middleware, the session and the Live engine sit below it, and a
/// debug build keeps every one of those frames large. A password reset, or
/// a mail sent over SMTP, adds enough on top to overflow Tokio's 2 MiB
/// worker stack, which aborts the process. A new task starts at the top of
/// a worker's stack, so the kit runs those steps here. `work` runs outside
/// the request: it cannot read the session or the signed-in user, so hand
/// it what it needs.
pub(crate) async fn on_own_task<T>(
    step: &str,
    work: impl Future<Output = T> + Send + 'static,
) -> Result<T, ComponentError>
where
    T: Send + 'static,
{
    tokio::spawn(work)
        .await
        .map_err(|error| failed(step, error))
}

/// Logs why a step of a component failed, and answers the closed failure
/// the browser receives, which carries no detail of the error.
pub(crate) fn failed(step: &str, error: impl std::fmt::Display) -> ComponentError {
    tracing::error!(error = %error, "{step} failed");
    ComponentError::application_failure()
}

/// The message under each control of a form, by field: the first rule a
/// field failed, or a check the action ran itself, such as credentials that
/// match no account. A form's view reads it with [`FieldErrors::get`], and
/// an empty bag shows nothing.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct FieldErrors(BTreeMap<String, String>);

impl FieldErrors {
    /// The first message of each field `errors` names.
    pub fn from_rules(errors: &ValidationErrors) -> Self {
        Self(
            errors
                .field_errors()
                .into_iter()
                .filter_map(|(field, failures)| {
                    let failure = failures.first()?;
                    let message = failure.message.as_ref().map_or_else(
                        || format!("The {field} field is invalid."),
                        ToString::to_string,
                    );
                    Some((field.to_string(), message))
                })
                .collect(),
        )
    }

    /// A bag that holds one message, for a check the rules cannot make.
    pub fn one(field: &str, message: &str) -> Self {
        Self(BTreeMap::from([(field.to_owned(), message.to_owned())]))
    }

    /// The message of `field`, or an empty string when it has none.
    pub fn get(&self, field: &str) -> &str {
        self.0.get(field).map_or("", String::as_str)
    }

    /// Whether `field` has a message.
    pub fn has(&self, field: &str) -> bool {
        self.0.contains_key(field)
    }

    /// Whether no field has a message.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// This application serves one tenant, so every Live request is tenantless.
struct SingleTenant;

#[async_trait]
impl LiveTenantResolver for SingleTenant {
    async fn resolve(&self, _request: &Request) -> Result<Option<String>, FrameworkError> {
        Ok(None)
    }
}
