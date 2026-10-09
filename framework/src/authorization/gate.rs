use std::any::Any;
use std::future::Future;
use std::sync::Arc;

use futures::FutureExt;

use super::Response;
use super::events::GateEvaluated;
use super::registry::{self, GateUser, global};
use crate::FrameworkError;
use crate::auth::{Auth, Authenticatable};
use crate::events::EventFacade;

/// Authorization gate facade.
///
/// ```rust,no_run
/// # use suprnova::Gate;
/// # struct User { is_admin: bool }
/// # struct Post { is_public: bool }
/// # let user = User { is_admin: false };
/// # let post = Post { is_public: true };
/// Gate::define::<User, Post>("view", |user, post| post.is_public || user.is_admin);
///
/// if Gate::allows("view", &user, &post) {
///     // ...
/// }
/// ```
///
/// # No `forUser`
///
/// Laravel's `Gate::forUser($user)->allows(...)` rebinds the gate's *implicit*
/// current-user resolver to a different user. Suprnova's gate takes the user
/// **explicitly** on every call - `Gate::allows(action, &user, &resource)` -
/// so "check as a different user" is just passing that user. The two
/// checks that resolve the user themselves,
/// [`inspect_current`](Self::inspect_current) and
/// [`none_current`](Self::none_current), ask the route's guard, the way
/// `#[authorize]` does; with the explicit API beside them, `forUser` has
/// nothing left to do.
///
/// # The `GateEvaluated` event
///
/// Every check dispatches one [`GateEvaluated`] event per action it
/// evaluates, after the before-hooks, the gate and the after-hooks ran, as
/// Laravel's `Gate::raw` does. An async check waits for the listeners. A
/// synchronous check cannot wait and must not block, so it polls the
/// dispatch once in place, which completes it when no listener has to wait
/// on anything, and leaves the rest to a task spawned on the current Tokio
/// runtime. Outside a Tokio runtime a synchronous check dispatches nothing,
/// so it never panics there. A listener's error is logged and never changes
/// the decision.
pub struct Gate;

impl Gate {
    // ── Sync API ──────────────────────────────────────────────────────────────

    /// Define a synchronous authorization closure for a given action.
    pub fn define<U: 'static, R: 'static>(
        action: &str,
        f: impl Fn(&U, &R) -> bool + Send + Sync + 'static,
    ) {
        global().register::<U, R>(action, f);
    }

    /// Define a nullable-user gate so a handler can authorize guests with `None`.
    /// Authenticated checks pass `Some(user)` through the usual hook pipeline.
    pub fn define_optional<U: 'static, R: 'static>(
        action: &str,
        f: impl Fn(Option<&U>, &R) -> bool + Send + Sync + 'static,
    ) {
        global().register_optional::<U, R>(action, f);
    }

    /// Define a nullable-user gate whose denial carries its own message and status.
    pub fn define_optional_with<U: 'static, R: 'static>(
        action: &str,
        f: impl Fn(Option<&U>, &R) -> Response + Send + Sync + 'static,
    ) {
        global().register_optional_with::<U, R>(action, f);
    }

    /// Define a synchronous gate whose closure returns a rich [`Response`]
    /// rather than a bare `bool` - so a denial can carry a message, code, and
    /// HTTP status that [`inspect`](Self::inspect) and [`Self::authorize`](Self::authorize)
    /// surface.
    ///
    /// ```rust,no_run
    /// use suprnova::authorization::Response;
    /// # use suprnova::Gate;
    /// # struct User { id: u64 }
    /// # struct Post { author_id: u64 }
    ///
    /// Gate::define_with::<User, Post>("update", |user, post| {
    ///     if post.author_id == user.id {
    ///         Response::allow()
    ///     } else {
    ///         Response::deny_with("You do not own this post.")
    ///     }
    /// });
    /// ```
    pub fn define_with<U: 'static, R: 'static>(
        action: &str,
        f: impl Fn(&U, &R) -> Response + Send + Sync + 'static,
    ) {
        global().register_with::<U, R>(action, f);
    }

    /// Returns `true` when the gate exists and allows the action.
    /// Missing gates **deny by default**.
    ///
    /// Routes through [`inspect`](Self::inspect), so `before`/`after` hooks
    /// apply. Calling `allows` on an async-registered gate returns `false`
    /// (default deny). Use [`Self::allows_async`](Self::allows_async) to invoke async
    /// gates correctly.
    ///
    /// An async before-hook ([`Self::before_async`]) does not run here, because
    /// this path cannot wait for it: neither its allow nor its deny reaches
    /// this answer. The permissions that
    /// [`register_gate_bridge`](crate::rbac::register_gate_bridge) connects to
    /// the gate are answered by such a hook, so they answer
    /// [`Self::allows_async`] and never this function.
    pub fn allows<U: 'static, R: 'static>(action: &str, user: &U, resource: &R) -> bool {
        Self::inspect(action, user, resource).allowed()
    }

    /// Returns `true` when the gate denies the action.
    ///
    /// Skips async before-hooks like [`Self::allows`], so a denial from
    /// [`Self::before_async`] is not enforced here; see [`Self::denies_async`].
    pub fn denies<U: 'static, R: 'static>(action: &str, user: &U, resource: &R) -> bool {
        !Self::allows(action, user, resource)
    }

    /// Authorize the action, returning the denial as an error.
    ///
    /// A bare denial maps to `FrameworkError::Unauthorized` (403). A rich
    /// denial - from a [`define_with`](Self::define_with) gate that returned a
    /// `Response` with a custom message/status - maps to
    /// `FrameworkError::Domain` carrying that message and status (e.g. 404 from
    /// `Response::deny_as_not_found()`).
    ///
    /// Like [`Self::allows`], this skips async before-hooks, their denials
    /// included, so a permission connected by
    /// [`register_gate_bridge`](crate::rbac::register_gate_bridge) authorizes
    /// through [`Self::authorize_async`] only.
    pub fn authorize<U: 'static, R: 'static>(
        action: &str,
        user: &U,
        resource: &R,
    ) -> Result<(), FrameworkError> {
        Self::inspect(action, user, resource)
            .authorize()
            .map(|_| ())
    }

    // ── Async API ─────────────────────────────────────────────────────────────

    /// Define an asynchronous authorization closure for a given action.
    ///
    /// The closure must produce an *owned* future - references to `user` and
    /// `resource` cannot be held past the closure return. Copy or clone any
    /// data needed inside the future body before returning it.
    ///
    /// # Sync compatibility
    ///
    /// Async-registered gates return `false` from the sync [`Self::allows`] /
    /// [`Self::denies`] / [`Self::authorize`] methods (default deny). Always use
    /// [`Self::allows_async`] / [`Self::denies_async`] / [`Self::authorize_async`] for gates
    /// registered with `define_async`.
    pub fn define_async<U, R, F, Fut>(action: &str, f: F)
    where
        U: 'static,
        R: 'static,
        F: Fn(&U, &R) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = bool> + Send + 'static,
    {
        global().register_async::<U, R, F, Fut>(action, f);
    }

    /// Define an asynchronous gate whose future resolves to a rich [`Response`]
    /// (the async sibling of [`define_with`](Self::define_with)).
    pub fn define_async_with<U, R, F, Fut>(action: &str, f: F)
    where
        U: 'static,
        R: 'static,
        F: Fn(&U, &R) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        global().register_async_with::<U, R, F, Fut>(action, f);
    }

    /// Async version of [`Self::allows`]. Works for both sync- and async-registered gates.
    pub async fn allows_async<U: 'static, R: 'static>(
        action: &str,
        user: &U,
        resource: &R,
    ) -> bool {
        Self::inspect_async(action, user, resource).await.allowed()
    }

    /// Async version of [`Self::denies`].
    pub async fn denies_async<U: 'static, R: 'static>(
        action: &str,
        user: &U,
        resource: &R,
    ) -> bool {
        !Self::allows_async(action, user, resource).await
    }

    /// Async version of [`Self::authorize`].
    pub async fn authorize_async<U: 'static, R: 'static>(
        action: &str,
        user: &U,
        resource: &R,
    ) -> Result<(), FrameworkError> {
        Self::inspect_async(action, user, resource)
            .await
            .authorize()
            .map(|_| ())
    }

    // ── Rich decisions: inspect / raw + before / after hooks ────────────────

    /// Evaluate the action and return the rich [`Response`] - the
    /// allow/deny decision plus any message, code, and HTTP status. Mirrors
    /// Laravel's `Gate::inspect`. An undefined ability, or an evaluation
    /// nothing decided (no gate, no hook filled it in), yields the
    /// configured default denial response - a bare deny unless
    /// [`Self::default_denial_response`] set something else.
    ///
    /// This is the evaluation core: [`Self::allows`](Self::allows),
    /// [`Self::denies`](Self::denies), and [`Self::authorize`](Self::authorize) all route
    /// through it, so `before`/`after` hooks apply uniformly. It runs the
    /// synchronous before-hooks only; an async one, such as the hook
    /// [`register_gate_bridge`](crate::rbac::register_gate_bridge) installs,
    /// answers [`Self::inspect_async`].
    ///
    /// Dispatches [`GateEvaluated`] without waiting for its listeners; see
    /// [the type's notes](Self#the-gateevaluated-event).
    pub fn inspect<U: 'static, R: 'static>(action: &str, user: &U, resource: &R) -> Response {
        Self::evaluate(action, user, resource).unwrap_or_else(registry::default_denial)
    }

    /// Async sibling of [`inspect`](Self::inspect).
    pub async fn inspect_async<U: 'static, R: 'static>(
        action: &str,
        user: &U,
        resource: &R,
    ) -> Response {
        Self::inspect_async_keyed::<U, R>(action, user, resource, None).await
    }

    /// [`Self::inspect_async`] for a user held only as a type-erased value,
    /// such as the one [`Auth::user`](crate::Auth::user) resolves. The lookup
    /// keys by the concrete type behind the erasure, so the gates and hooks
    /// registered for that type answer. `#[authorize]` checks through this,
    /// because the macro does not know the application's user type.
    pub(crate) async fn inspect_erased_async<R: 'static>(
        action: &str,
        user: &(dyn Any + Send + Sync),
        resource: &R,
    ) -> Response {
        Self::inspect_async_keyed::<dyn Any + Send + Sync, R>(action, user, resource, None).await
    }

    /// [`Self::inspect_async`] for a user the check resolved itself, from a
    /// guard. The user is checked as its concrete type, through
    /// [`Self::inspect_erased_async`], and the [`GateEvaluated`] event
    /// carries its identifier.
    pub(crate) async fn inspect_authenticatable<R: 'static>(
        action: &str,
        user: Arc<dyn Authenticatable>,
        resource: &R,
    ) -> Response {
        let id = user.get_auth_identifier();
        let user: Arc<dyn Any + Send + Sync> = user.into_arc_any();
        Self::inspect_async_keyed::<dyn Any + Send + Sync, R>(action, &*user, resource, Some(id))
            .await
    }

    /// Consult a nullable policy for a guest, as `#[authorize]` and
    /// [`Self::inspect_current`] do when the route's guard has no user.
    /// `None` when no gate defined with [`Self::define_optional`] or
    /// [`Self::define_optional_with`] answers for the action and resource.
    /// Dispatches a [`GateEvaluated`] event with no user and waits for it.
    pub(crate) async fn inspect_guest<R: 'static>(action: &str, resource: &R) -> Option<Response> {
        let window = crate::render_cache::collector::begin_authorization_decision();
        let response = global().invoke_guest(action, resource);
        crate::render_cache::collector::end_authorization_decision(window);
        if let Some(event) = evaluated_event::<R>(action, || None, None, response.as_ref()) {
            dispatch_evaluated(event).await;
        }
        response
    }

    // The evaluation both async inspect forms share, so a concrete and a
    // type-erased user go through one pipeline and one default denial.
    async fn inspect_async_keyed<U: GateUser + ?Sized, R: 'static>(
        action: &str,
        user: &U,
        resource: &R,
        user_id: Option<String>,
    ) -> Response {
        Self::evaluate_async(action, user, resource, user_id)
            .await
            .unwrap_or_else(registry::default_denial)
    }

    // The evaluation every synchronous form shares: the before-hooks, the
    // gate and the after-hooks, inside the render cache's decision window,
    // then the `GateEvaluated` event, dispatched without waiting.
    fn evaluate<U: 'static, R: 'static>(action: &str, user: &U, resource: &R) -> Option<Response> {
        let window = crate::render_cache::collector::begin_authorization_decision();
        let response = global().raw::<U, R>(action, user, resource);
        crate::render_cache::collector::end_authorization_decision(window);
        let user_type = || Some(std::any::type_name::<U>());
        if let Some(event) = evaluated_event::<R>(action, user_type, None, response.as_ref()) {
            dispatch_evaluated_without_waiting(event);
        }
        response
    }

    // The evaluation every async form shares, the sibling of `evaluate`: it
    // waits for the `GateEvaluated` listeners. `user_id` is the identifier
    // of a user the check resolved itself, `None` for a user it was given.
    async fn evaluate_async<U: GateUser + ?Sized, R: 'static>(
        action: &str,
        user: &U,
        resource: &R,
        user_id: Option<String>,
    ) -> Option<Response> {
        let window = crate::render_cache::collector::begin_authorization_decision();
        let response = global().raw_async::<U, R>(action, user, resource).await;
        crate::render_cache::collector::end_authorization_decision(window);
        let user_type = || Some(global().user_type_name(user));
        if let Some(event) = evaluated_event::<R>(action, user_type, user_id, response.as_ref()) {
            dispatch_evaluated(event).await;
        }
        response
    }

    /// [`Self::inspect_async`] for the user of the route's guard, the user
    /// `#[authorize]` checks, as Laravel's `Gate::inspect` resolves the user
    /// itself.
    ///
    /// The route's guard is the one the last
    /// [`AuthMiddleware`](crate::AuthMiddleware) that passed the request on
    /// checked, or the default guard when it names none or none ran. The
    /// user is checked as its concrete type, so the gates, policies and
    /// hooks registered for that type answer, async ones included, and the
    /// [`GateEvaluated`] event carries the user's identifier.
    ///
    /// With no user signed in, a gate defined with
    /// [`Self::define_optional`] or [`Self::define_optional_with`] receives
    /// `None`, and every other ability answers the default denial (see
    /// [`Self::default_denial_response`]), as Laravel's gate denies a guest a
    /// callback that does not accept one. A guest is never an error here:
    /// the answer is a [`Response`], not the 401 `#[authorize]` returns.
    ///
    /// ```rust,no_run
    /// # use suprnova::{FrameworkError, Gate};
    /// # struct Post;
    /// # async fn ex(post: Post) -> Result<(), FrameworkError> {
    /// let decision = Gate::inspect_current("update", &post).await?;
    /// if decision.denied() {
    ///     println!("{}", decision.message().unwrap_or("Not allowed."));
    /// }
    /// # Ok(()) }
    /// ```
    ///
    /// # Errors
    ///
    /// The error the route's guard returns when it cannot resolve the user,
    /// such as a provider that cannot reach its database. The check does not
    /// guess an answer for a user it could not resolve.
    pub async fn inspect_current<R: 'static>(
        action: &str,
        resource: &R,
    ) -> Result<Response, FrameworkError> {
        let user = Auth::route_user().await?;
        Ok(Self::inspect_route_user(action, user, resource).await)
    }

    /// `true` when **none** of `actions` allows the user of the route's
    /// guard on `resource`: [`Self::none_async`] for the user
    /// [`Self::inspect_current`] checks, as Laravel's `Gate::none` resolves
    /// the user itself. Stops at the first action that allows.
    ///
    /// A guest is checked as [`Self::inspect_current`] checks one: only a
    /// gate defined with [`Self::define_optional`] or
    /// [`Self::define_optional_with`] can allow it. An empty `actions` is
    /// `true`.
    ///
    /// # Errors
    ///
    /// The error the route's guard returns when it cannot resolve the user.
    pub async fn none_current<R: 'static>(
        actions: &[&str],
        resource: &R,
    ) -> Result<bool, FrameworkError> {
        let user = Auth::route_user().await?;
        for action in actions {
            let decision = Self::inspect_route_user(action, user.clone(), resource).await;
            if decision.allowed() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    // Inspect `action` for the user the route's guard resolved, or for a
    // guest through the nullable policies, with the default denial when no
    // nullable policy answers.
    async fn inspect_route_user<R: 'static>(
        action: &str,
        user: Option<Arc<dyn Authenticatable>>,
        resource: &R,
    ) -> Response {
        match user {
            Some(user) => Self::inspect_authenticatable(action, user, resource).await,
            None => Self::inspect_guest(action, resource)
                .await
                .unwrap_or_else(registry::default_denial),
        }
    }

    /// The raw evaluation result, preserving the *undefined* case as `None`.
    ///
    /// Unlike [`inspect`](Self::inspect) (which normalizes `None` to a default
    /// deny), `raw` returns `None` when nothing decided - no `before` hook
    /// fired, no gate is registered for `(action, U, R)`, and no `after` hook
    /// filled in. This distinguishes "explicitly denied" from "no rule
    /// defined", mirroring Laravel's `Gate::raw`. As in
    /// [`inspect`](Self::inspect), async before-hooks do not run here; use
    /// [`Self::raw_async`] for them.
    ///
    /// Dispatches [`GateEvaluated`], with `decision: None` when nothing
    /// decided, without waiting for its listeners; see
    /// [the type's notes](Self#the-gateevaluated-event).
    pub fn raw<U: 'static, R: 'static>(action: &str, user: &U, resource: &R) -> Option<Response> {
        Self::evaluate(action, user, resource)
    }

    /// Async sibling of [`raw`](Self::raw). Waits for the listeners of the
    /// [`GateEvaluated`] event it dispatches.
    pub async fn raw_async<U: 'static, R: 'static>(
        action: &str,
        user: &U,
        resource: &R,
    ) -> Option<Response> {
        Self::evaluate_async(action, user, resource, None).await
    }

    /// Register a hook that runs **before** any gate for the user type `U`.
    ///
    /// Returning `Some(decision)` short-circuits all gates and other before
    /// hooks for that user type (first `Some` wins); returning `None` lets
    /// evaluation continue to the gate. The canonical use is a global override
    /// such as "administrators may do anything":
    ///
    /// ```rust,no_run
    /// # use suprnova::Gate;
    /// # struct User { is_admin: bool }
    /// Gate::before::<User>(|user, _action| user.is_admin.then_some(true));
    /// ```
    ///
    /// Hooks are keyed by the **user type** (`U`), not by resource, so a hook
    /// fires for every `(action, U, R)` regardless of resource - put
    /// resource-specific logic in the gate. Hooks are synchronous predicates;
    /// for async authorization logic use [`define_async`](Self::define_async)
    /// or [`define_async_with`](Self::define_async_with), and for a hook that
    /// has to wait on I/O use [`before_async`](Self::before_async). They apply
    /// to the async evaluation path too.
    pub fn before<U: 'static>(f: impl Fn(&U, &str) -> Option<bool> + Send + Sync + 'static) {
        global().register_before::<U>(f);
    }

    /// Register an **async** hook that runs **before** any gate for the user
    /// type `U`: the sibling of [`Self::before`] for a decision that has to
    /// wait on I/O, such as a database read.
    ///
    /// It shares one ordered list with the synchronous hooks: evaluation walks
    /// them in registration order, the first `Some(decision)` short-circuits,
    /// and `None` lets evaluation continue. Only the async evaluation path
    /// awaits it - [`Self::allows_async`], [`Self::denies_async`],
    /// [`Self::authorize_async`], [`Self::inspect_async`], [`Self::raw_async`],
    /// and the async multi-action forms. The synchronous path cannot wait for
    /// a future and skips the hook, so what it alone decides is invisible
    /// there. Never block a thread on the I/O inside a synchronous hook
    /// instead: that stalls a runtime worker for every check.
    ///
    /// Like [`define_async`](Self::define_async), the closure must return an
    /// *owned* future: copy what it needs out of `user` and `action` before
    /// building it.
    ///
    /// ```rust,no_run
    /// # use suprnova::Gate;
    /// # struct User { id: i64 }
    /// # async fn is_staff(_id: i64) -> bool { false }
    /// // Staff, as the directory service lists them, may do anything.
    /// Gate::before_async::<User, _, _>(|user, _action| {
    ///     let id = user.id;
    ///     async move { is_staff(id).await.then_some(true) }
    /// });
    /// ```
    ///
    /// # A denial here is not enforced everywhere
    ///
    /// A hook that answers `Some(false)` denies only on the async forms. The
    /// forms that cannot wait - [`Self::allows`], [`Self::denies`],
    /// [`Self::authorize`], [`Self::inspect`], [`Self::raw`], [`Self::any`],
    /// [`Self::none`], [`Self::check`], and
    /// [`Authorizable::can`](crate::Authorizable::can),
    /// [`Authorizable::cannot`](crate::Authorizable::cannot) and
    /// [`Authorizable::authorize`](crate::Authorizable::authorize) - skip the
    /// hook and go on as if it had answered nothing, so a gate that allows
    /// still allows there. A hook that must deny belongs in [`Self::before`],
    /// or else every check the application makes has to use an async form.
    pub fn before_async<U, F, Fut>(f: F)
    where
        U: 'static,
        F: Fn(&U, &str) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<bool>> + Send + 'static,
    {
        global().register_before_async::<U, F, Fut>(f);
    }

    /// Register a hook that runs **after** the gate for the user type `U`.
    ///
    /// Every after hook runs (so it can log the outcome), receiving the running
    /// decision as `Option<bool>` (`None` while still undecided). Following
    /// Laravel's `$result ??= $afterResult` semantic, an after hook can only
    /// **fill in** an undecided result - it cannot override an allow or deny
    /// that a before hook or gate already produced. Return `None` to record a
    /// no-op.
    ///
    /// ```rust,no_run
    /// # use suprnova::Gate;
    /// # struct User { is_superuser: bool }
    /// // Grant a fallback only when no gate is defined for the action:
    /// Gate::after::<User>(|user, _action, decided| {
    ///     decided.is_none().then(|| user.is_superuser)
    /// });
    /// ```
    ///
    /// The hook does not see the resource; register it with
    /// [`Self::after_with_arguments`] when it needs it.
    pub fn after<U: 'static>(
        f: impl Fn(&U, &str, Option<bool>) -> Option<bool> + Send + Sync + 'static,
    ) {
        global().register_after::<U>(f);
    }

    /// Register a hook that runs **after** the gate for the user type `U`
    /// and also receives the resource, as Laravel's after-callbacks receive
    /// `$arguments`.
    ///
    /// It follows the rules of [`Self::after`]: it runs on every check of a
    /// `U` (in registration order with the hooks `after` registers), it
    /// receives the running decision as `Option<bool>`, and it can only
    /// **fill in** an undecided result, never override an allow or a deny.
    /// It runs only for checks whose resource is an `R`; a check about
    /// another type skips it. It answers `bool` or nothing, never a
    /// [`Response`], as Laravel declares its after-callbacks `bool|null`.
    ///
    /// ```rust,no_run
    /// # use suprnova::Gate;
    /// # struct User { id: u64 }
    /// # struct Post { author_id: u64 }
    /// // When no gate decides, an author may act on their own post.
    /// Gate::after_with_arguments::<User, Post>(|user, _action, decided, post| {
    ///     decided.is_none().then(|| post.author_id == user.id)
    /// });
    /// ```
    pub fn after_with_arguments<U: 'static, R: 'static>(
        f: impl Fn(&U, &str, Option<bool>, &R) -> Option<bool> + Send + Sync + 'static,
    ) {
        global().register_after_with_arguments::<U, R>(f);
    }

    /// Set the process-global default response for a bare `false` denial.
    /// Mirrors Laravel's `Gate::defaultDenialResponse($response)`.
    ///
    /// This reshapes exactly two kinds of outcome: a bare `false` - from a
    /// bool gate ([`Self::define`] / [`Self::define_async`], including a
    /// `#[policy]` method returning `bool`), or from a [`Self::before`]
    /// short-circuit or an [`Self::after`] fill-in that decided `false` -
    /// and an evaluation nothing else decided at all: an undefined ability
    /// with no hook opinion either. It never touches a gate
    /// registered with [`Self::define_with`] / [`Self::define_async_with`]:
    /// a returned [`Response`] - rich or a bare [`Response::deny()`] - always
    /// passes through verbatim. That is Laravel's rule too: `inspect` only
    /// substitutes the default for a truly falsy callback result, never for
    /// a `Response` object the callback built itself.
    ///
    /// Like [`Self::before`] / [`Self::after`], this is process-global
    /// state, not scoped to one gate - set it once, typically in
    /// `bootstrap::register()`:
    ///
    /// ```rust,no_run
    /// use suprnova::authorization::Response;
    /// # use suprnova::Gate;
    /// // Hide every undecided/bare-denied ability behind a 404 instead of a 403.
    /// Gate::default_denial_response(Response::deny_as_not_found());
    /// ```
    ///
    /// # Divergence from Laravel: an allow-shaped default is rejected
    ///
    /// This is a *denial* default - passing an allow-shaped
    /// [`Response::allow()`] is logged and ignored (the previous default, or
    /// the bare deny, is kept) rather than accepted, because accepting it
    /// would silently invert every bare `false` gate result to allowed. This
    /// is the one fail-open direction on this surface, and Laravel has no
    /// equivalent guard against it; Suprnova adds one deliberately.
    pub fn default_denial_response(response: Response) {
        registry::set_default_denial(response);
    }

    /// Reset the default denial response to unset (bare [`Response::deny()`]).
    ///
    /// Hidden because it exists only so tests can isolate the process-global
    /// state [`Self::default_denial_response`] sets - application code
    /// configures the default once at boot and has no reason to clear it.
    #[doc(hidden)]
    pub fn clear_default_denial_response() {
        registry::clear_default_denial();
    }

    // ── Introspection ─────────────────────────────────────────────────────────

    /// `true` iff a gate (sync **or** async) is registered for the
    /// `(action, U, R)` tuple. Mirrors Laravel's `Gate::has` - handy
    /// for diagnostic UIs ("is this ability defined?") and for
    /// frontend Inertia props that ship the user's full ability map.
    ///
    /// Note: a registered gate is not the same as an allowed gate.
    /// `has` answers "does the framework know how to decide?", not
    /// "is the answer yes?".
    pub fn has<U: 'static, R: 'static>(action: &str) -> bool {
        global().has::<U, R>(action)
    }

    /// Every distinct action name registered across all `(U, R)`
    /// tuples, sorted + deduped. Mirrors Laravel's
    /// `Gate::abilities()`. Useful for admin UIs that need to list
    /// every defined ability for picker / role-mapping forms.
    pub fn abilities() -> Vec<String> {
        global().abilities()
    }

    // ── Multi-action dispatch (sync) ──────────────────────────────────────────

    /// `true` iff **any** of the supplied actions allow against the
    /// same `(user, resource)`. Mirrors Laravel's
    /// `Gate::any($abilities, $arguments)`. Short-circuits on the
    /// first allow - does not evaluate later actions.
    ///
    /// A missing gate among `actions` is treated as deny (matches
    /// the single-action [`Self::allows`] semantic).
    ///
    /// Skips async before-hooks like [`Self::allows`], their denials
    /// included; see [`Self::any_async`].
    pub fn any<U: 'static, R: 'static>(actions: &[&str], user: &U, resource: &R) -> bool {
        actions.iter().any(|a| Self::allows(a, user, resource))
    }

    /// `true` iff **none** of the supplied actions allow against the
    /// same `(user, resource)`. Mirrors Laravel's
    /// `Gate::none($abilities, $arguments)`. Short-circuits on the
    /// first allow (returning `false`).
    ///
    /// Skips async before-hooks like [`Self::allows`], their denials
    /// included; see [`Self::none_async`].
    pub fn none<U: 'static, R: 'static>(actions: &[&str], user: &U, resource: &R) -> bool {
        !Self::any(actions, user, resource)
    }

    /// `true` iff **every** action allows against the same
    /// `(user, resource)`. Mirrors Laravel's array-form
    /// `Gate::check([abilities], $arguments)`. Short-circuits on
    /// the first deny.
    ///
    /// An empty `actions` slice returns `true` (vacuously) -
    /// matches the standard `Iterator::all` semantic.
    ///
    /// Skips async before-hooks like [`Self::allows`], their denials
    /// included; see [`Self::check_async`].
    pub fn check<U: 'static, R: 'static>(actions: &[&str], user: &U, resource: &R) -> bool {
        actions.iter().all(|a| Self::allows(a, user, resource))
    }

    // ── Multi-action dispatch (async) ─────────────────────────────────────────

    /// Async sibling of [`Self::any`]. Sequentially awaits each gate; works
    /// for sync and async registrations alike (via
    /// [`Self::allows_async`]'s dispatch). Sequential rather than
    /// concurrent because most policy bodies are cheap and
    /// short-circuiting on the first allow saves the rest.
    pub async fn any_async<U: 'static, R: 'static>(
        actions: &[&str],
        user: &U,
        resource: &R,
    ) -> bool {
        for action in actions {
            if Self::allows_async(action, user, resource).await {
                return true;
            }
        }
        false
    }

    /// Async sibling of [`Self::none`].
    pub async fn none_async<U: 'static, R: 'static>(
        actions: &[&str],
        user: &U,
        resource: &R,
    ) -> bool {
        !Self::any_async(actions, user, resource).await
    }

    /// Async sibling of [`Self::check`].
    pub async fn check_async<U: 'static, R: 'static>(
        actions: &[&str],
        user: &U,
        resource: &R,
    ) -> bool {
        for action in actions {
            if !Self::allows_async(action, user, resource).await {
                return false;
            }
        }
        true
    }
}

// Build the `GateEvaluated` event of one evaluation, or `None` when nothing
// would see it. A check is a hot path and most applications listen to no
// gate event, so the user's type is named and the action copied only when
// the event would be delivered.
fn evaluated_event<R: 'static>(
    action: &str,
    user_type: impl FnOnce() -> Option<&'static str>,
    user_id: Option<String>,
    response: Option<&Response>,
) -> Option<GateEvaluated> {
    if !EventFacade::is_observed::<GateEvaluated>() {
        return None;
    }
    Some(GateEvaluated {
        user_type: user_type(),
        user_id,
        action: action.to_owned(),
        resource_type: std::any::type_name::<R>(),
        decision: response.map(Response::allowed),
    })
}

// Deliver the event of an evaluation and wait for its listeners. A
// listener's error is logged and never changes the decision: the event
// reports a check, it does not take part in it.
async fn dispatch_evaluated(event: GateEvaluated) {
    if let Err(error) = EventFacade::dispatch(event).await {
        tracing::warn!(
            %error,
            "a GateEvaluated listener failed; the gate decision stands"
        );
    }
}

// Deliver the event of a synchronous evaluation without blocking: poll the
// dispatch once in place, which completes it when no listener has to wait
// (a fake records it there, and a deferral scope of the calling task sees
// it), and hand what is still pending to a task on the current runtime.
// Outside a Tokio runtime nothing is dispatched: a listener could need the
// runtime, and the check must not panic.
fn dispatch_evaluated_without_waiting(event: GateEvaluated) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let mut pending = Box::pin(dispatch_evaluated(event));
    if (&mut pending).now_or_never().is_none() {
        handle.spawn(pending);
    }
}
