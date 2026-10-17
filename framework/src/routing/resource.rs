//! Resource routing - Laravel `Route::resource(...)` parity.
//!
//! Laravel's `ResourceRegistrar` (`Illuminate/Routing/ResourceRegistrar.php`)
//! generates the standard 7-action REST surface from a controller class
//! name + a base path. Suprnova ships a Rust-shaped equivalent:
//! consumers implement the [`ResourceController`] trait and pass an
//! instance through [`Router::resource`] / [`Router::api_resource`] to
//! generate the same 7 (or 4, for API) routes plus their conventional
//! route names.
//!
//! ## Default routes
//!
//! For a resource named `posts` at path `/posts`:
//!
//! | Verb   | Path                    | Trait method     | Name           |
//! |--------|-------------------------|------------------|----------------|
//! | GET    | `/posts`                | `index`          | `posts.index`  |
//! | GET    | `/posts/create`         | `create`         | `posts.create` |
//! | POST   | `/posts`                | `store`          | `posts.store`  |
//! | GET    | `/posts/{post}`         | `show`           | `posts.show`   |
//! | GET    | `/posts/{post}/edit`    | `edit`           | `posts.edit`   |
//! | PUT    | `/posts/{post}`         | `update`         | `posts.update` |
//! | PATCH  | `/posts/{post}`         | `update`         | _shares update_|
//! | DELETE | `/posts/{post}`         | `destroy`        | `posts.destroy`|
//!
//! `api_resource` drops `create` and `edit` (the form-rendering routes
//! that an API doesn't need), matching Laravel's
//! `ResourceRegistrar::apiResourceDefaults`.
//!
//! ## Customizing the surface
//!
//! [`ResourceRoutes`] (returned by [`Router::resource`]) supports the
//! Laravel-shaped chain:
//!
//! ```rust,no_run
//! # use suprnova::routing::{Router, ResourceController, ResourceAction};
//! # struct PostsCtl;
//! # impl ResourceController for PostsCtl {}
//! Router::new().resource("posts", PostsCtl)
//!     .only(&[ResourceAction::Index, ResourceAction::Show])
//!     .names([("index", "posts.list")])
//!     .parameter("post_id");
//! ```
//!
//! - [`ResourceRoutes::only`] restricts the generated set to a list.
//! - [`ResourceRoutes::except`] excludes a list from the default set.
//! - [`ResourceRoutes::names`] overrides route names per action.
//! - [`ResourceRoutes::parameters`] renames path parameters by resource
//!   segment (Laravel's `parameters(['users' => 'user_id'])`).
//! - [`ResourceRoutes::scoped`], [`ResourceRoutes::with_trashed`] and
//!   [`ResourceRoutes::missing`] set the routes' binding fields and
//!   scoping, soft-deleted rows and missing-row answer.
//! - [`ResourceRoutes::middleware`] runs a [`ControllerMiddleware`] on the
//!   actions it is scoped to.
//!
//! ## Controller middleware
//!
//! A controller declares its own middleware, Laravel's `HasMiddleware`:
//! [`ResourceController::middleware`] returns a list of
//! [`ControllerMiddleware`] values, each scoped with `only` or `except`. A
//! module named by `resource!` declares the same list in a
//! `pub fn middleware()`. The resource's routes run each one after the
//! group's middleware and after the middleware given at registration.
//!
//! A dotted name nests: `users.posts` registers `/users/{user}/posts` and
//! `/users/{user}/posts/{post}` under `users.posts.*`.
//!
//! ## Actions that bind
//!
//! A [`ResourceController`] action takes the request. For actions that
//! take bound arguments, `resource!("posts", controllers::posts)` names a
//! module whose `#[handler]` functions are the actions, found by action
//! name, and builds a [`ResourceDef`] with the same chain.
//!
//! ## Dual-API
//!
//! - `only` (Laravel) + `keep` (Rust) - both alias.
//! - `except` (Laravel) + `drop` (Rust) - both alias.
//! - `names` (Laravel) + `rename` (Rust) - both alias.

use super::binding::{HandlerRef, MissingHook, RouteBindingOptions, boxed_missing};
use super::macros::{convert_route_params, join_paths};
use super::router::{BoxedHandler, Router};
use crate::FrameworkError;
use crate::auth::{Auth, Authenticatable};
use crate::authorization::Gate;
use crate::http::{HttpResponse, Request, Response};
use crate::middleware::{BoxedMiddleware, Middleware, Next, boxed_as};
use crate::session::SessionBlock;
use crate::session::blocking::register_route_block;
use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// One action in the standard REST resource surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceAction {
    /// `GET /<base>` - list resources.
    Index,
    /// `GET /<base>/create` - show a creation form (web-only).
    Create,
    /// `POST /<base>` - persist a new resource.
    Store,
    /// `GET /<base>/{id}` - show a single resource.
    Show,
    /// `GET /<base>/{id}/edit` - show an edit form (web-only).
    Edit,
    /// `PUT|PATCH /<base>/{id}` - update.
    Update,
    /// `DELETE /<base>/{id}` - destroy.
    Destroy,
}

impl ResourceAction {
    /// Stable string name used for `.names()` overrides and `parameters()`
    /// lookups. Matches Laravel's lowercase action keys.
    pub fn key(self) -> &'static str {
        match self {
            ResourceAction::Index => "index",
            ResourceAction::Create => "create",
            ResourceAction::Store => "store",
            ResourceAction::Show => "show",
            ResourceAction::Edit => "edit",
            ResourceAction::Update => "update",
            ResourceAction::Destroy => "destroy",
        }
    }

    /// The seven web-resource defaults in canonical order. Mirrors
    /// `ResourceRegistrar::$resourceDefaults`.
    pub fn web_defaults() -> &'static [ResourceAction] {
        &[
            ResourceAction::Index,
            ResourceAction::Create,
            ResourceAction::Store,
            ResourceAction::Show,
            ResourceAction::Edit,
            ResourceAction::Update,
            ResourceAction::Destroy,
        ]
    }

    /// The five API-resource defaults (drops `create` and `edit`).
    /// Mirrors `ResourceRegistrar::apiResourceDefaults`.
    pub fn api_defaults() -> &'static [ResourceAction] {
        &[
            ResourceAction::Index,
            ResourceAction::Store,
            ResourceAction::Show,
            ResourceAction::Update,
            ResourceAction::Destroy,
        ]
    }

    fn from_key(s: &str) -> Option<Self> {
        Some(match s {
            "index" => ResourceAction::Index,
            "create" => ResourceAction::Create,
            "store" => ResourceAction::Store,
            "show" => ResourceAction::Show,
            "edit" => ResourceAction::Edit,
            "update" => ResourceAction::Update,
            "destroy" => ResourceAction::Destroy,
            _ => return None,
        })
    }
}

/// Factory that produces the authorization middleware for one resource
/// action. Built by [`ResourceRoutes::authorize_resource`], invoked once
/// per generated route at registration time. Returns `None` for actions
/// that have no authorization mapping (none currently - every standard
/// action maps to an ability).
type AuthorizeFactory = Box<dyn Fn(ResourceAction) -> Option<BoxedMiddleware> + Send + Sync>;

/// Map a resource action to the Gate ability it authorizes against,
/// matching Laravel's `authorizeResource` table:
///
/// | Action  | Ability |
/// |---------|---------|
/// | index   | view    |
/// | show    | view    |
/// | create  | create  |
/// | store   | create  |
/// | edit    | update  |
/// | update  | update  |
/// | destroy | delete  |
fn ability_for(action: ResourceAction) -> &'static str {
    match action {
        ResourceAction::Index | ResourceAction::Show => "view",
        ResourceAction::Create | ResourceAction::Store => "create",
        ResourceAction::Edit | ResourceAction::Update => "update",
        ResourceAction::Destroy => "delete",
    }
}

/// Per-route authorization middleware generated by
/// [`ResourceRoutes::authorize_resource`].
///
/// Resolves the user of the route's guard (the guard the last
/// `AuthMiddleware` that passed the request on checked, else the default
/// guard) as the concrete type `U` and runs the
/// resource action's mapped ability through the [`Gate`] against a default
/// instance of the resource marker `R`. A denial - or an unauthenticated
/// request - short-circuits the chain before the resource handler runs,
/// closing the "forgotten `Gate::authorize` in the handler body" gap.
///
/// The resource marker is a [`Default`] value of `R` (Laravel passes the
/// model class for the class-level abilities and a route-bound instance for
/// instance abilities; Suprnova's type-keyed gate discriminates on the
/// `R` *type*, so a default marker carries the same routing information for
/// the policy that the type does).
struct ResourceAuthorizeMiddleware<U, R> {
    ability: &'static str,
    _user: std::marker::PhantomData<fn() -> U>,
    _resource: std::marker::PhantomData<fn() -> R>,
}

#[async_trait]
impl<U, R> Middleware for ResourceAuthorizeMiddleware<U, R>
where
    U: Authenticatable + Clone + 'static,
    R: Default + Send + Sync + 'static,
{
    async fn handle(&self, request: Request, next: Next) -> Response {
        // Fail closed: no user on the route's guard (or the user is not a
        // `U`) is a denial, never a pass-through. Another guard's user never
        // stands in.
        let user = match Auth::route_user().await {
            Ok(user) => user.and_then(|user| user.as_any().downcast_ref::<U>().cloned()),
            Err(err) => return Err(HttpResponse::from(err)),
        };
        let Some(user) = user else {
            return Err(HttpResponse::from(FrameworkError::Unauthorized));
        };
        let resource = R::default();
        match Gate::authorize::<U, R>(self.ability, &user, &resource) {
            Ok(()) => next(request).await,
            Err(err) => Err(HttpResponse::from(err)),
        }
    }
}

/// What a [`ControllerMiddleware`] runs: a middleware value, or the name of
/// an alias or group, resolved when the resource registers.
#[derive(Clone)]
enum ControllerMiddlewareSource {
    Boxed(BoxedMiddleware),
    Named(String),
}

/// A middleware a resource controller declares, scoped to some of its
/// actions: Laravel's `Controllers\Middleware`.
///
/// List these in [`ResourceController::middleware`], in the
/// `pub fn middleware()` of a module named by `resource!`, or pass one to
/// [`ResourceRoutes::middleware`] or [`ResourceDef::middleware`]. Without
/// `only` or `except`, it runs on every action of the resource.
///
/// ```rust,no_run
/// use suprnova::routing::{ControllerMiddleware, ResourceAction, ResourceController};
/// # use suprnova::{async_trait, Middleware, Next, Request, Response};
/// # struct Audit;
/// # #[async_trait]
/// # impl Middleware for Audit {
/// #     async fn handle(&self, request: Request, next: Next) -> Response { next(request).await }
/// # }
///
/// struct PostsController;
///
/// impl ResourceController for PostsController {
///     fn middleware(&self) -> Vec<ControllerMiddleware> {
///         vec![
///             ControllerMiddleware::named("auth").except(&[ResourceAction::Index, ResourceAction::Show]),
///             ControllerMiddleware::new(Audit).only(&[ResourceAction::Destroy]),
///         ]
///     }
/// }
/// ```
///
/// An action keeps a middleware when `only` lists it, or `only` was never
/// called, and `except` does not list it. `only(&[])` therefore runs the
/// middleware on no action, as Laravel's `only([])` does. `update` covers
/// both `PUT` and `PATCH`.
#[derive(Clone)]
pub struct ControllerMiddleware {
    source: ControllerMiddlewareSource,
    only: Option<Vec<ResourceAction>>,
    except: Vec<ResourceAction>,
}

impl ControllerMiddleware {
    /// Run `middleware`. It is boxed once, and every action it is scoped
    /// to shares it.
    pub fn new<M: Middleware + 'static>(middleware: M) -> Self {
        Self {
            source: ControllerMiddlewareSource::Boxed(boxed_as(middleware)),
            only: None,
            except: Vec::new(),
        }
    }

    /// Run the middleware `name` stands for: an alias such as `"auth"`, an
    /// alias with arguments such as `"throttle:60,1"`, or a group.
    ///
    /// The name is resolved when the resource registers, as
    /// `middleware_named` resolves it on a route. A name that is not
    /// registered fails the registration: `try_register` returns the error
    /// and `register` panics at boot.
    pub fn named(name: &str) -> Self {
        Self {
            source: ControllerMiddlewareSource::Named(name.to_string()),
            only: None,
            except: Vec::new(),
        }
    }

    /// Run the middleware on these actions only. Mirrors Laravel's
    /// `Middleware::only`; a second call replaces the first.
    pub fn only(mut self, actions: &[ResourceAction]) -> Self {
        self.only = Some(actions.to_vec());
        self
    }

    /// Run the middleware on every action but these. Mirrors Laravel's
    /// `Middleware::except`; a second call replaces the first.
    pub fn except(mut self, actions: &[ResourceAction]) -> Self {
        self.except = actions.to_vec();
        self
    }

    /// Whether the middleware runs on `action`: Laravel's
    /// `methodExcludedByOptions`, negated.
    fn applies_to(&self, action: ResourceAction) -> bool {
        self.only.as_ref().is_none_or(|only| only.contains(&action))
            && !self.except.contains(&action)
    }

    /// The middleware this stands for, resolved once for every action.
    fn resolve(&self) -> Result<Vec<BoxedMiddleware>, FrameworkError> {
        match &self.source {
            ControllerMiddlewareSource::Boxed(middleware) => Ok(vec![middleware.clone()]),
            ControllerMiddlewareSource::Named(name) => {
                crate::middleware::resolve_named_middleware(name)
            }
        }
    }
}

/// REST resource controller. Implement on a unit struct (or anything
/// `Send + Sync + 'static`) and pass to [`Router::resource`] /
/// [`Router::api_resource`].
///
/// All seven methods have a default implementation that returns
/// `404 Not Found`. Override the ones your resource supports;
/// `only`/`except` on the generated [`ResourceRoutes`] determine
/// which routes are actually registered.
///
/// Methods take a [`Request`] and return a [`Response`]. They run inside
/// the framework's normal middleware / handler pipeline - there is no
/// magic dependency injection; pull form data via `request.form_data()`
/// etc. just as you would in a function handler.
pub trait ResourceController: Send + Sync + 'static {
    /// `GET /<base>` - list resources.
    fn index(&self, request: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        let _ = request;
        Box::pin(async { not_implemented("index") })
    }

    /// `GET /<base>/create` - show a creation form.
    fn create(&self, request: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        let _ = request;
        Box::pin(async { not_implemented("create") })
    }

    /// `POST /<base>` - store a new resource.
    fn store(&self, request: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        let _ = request;
        Box::pin(async { not_implemented("store") })
    }

    /// `GET /<base>/{id}` - show a single resource.
    fn show(&self, request: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        let _ = request;
        Box::pin(async { not_implemented("show") })
    }

    /// `GET /<base>/{id}/edit` - show an edit form.
    fn edit(&self, request: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        let _ = request;
        Box::pin(async { not_implemented("edit") })
    }

    /// `PUT|PATCH /<base>/{id}` - update an existing resource.
    fn update(&self, request: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        let _ = request;
        Box::pin(async { not_implemented("update") })
    }

    /// `DELETE /<base>/{id}` - destroy a resource.
    fn destroy(&self, request: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        let _ = request;
        Box::pin(async { not_implemented("destroy") })
    }

    /// The middleware this controller runs on its actions: Laravel's
    /// `HasMiddleware::middleware`. Read once, when the resource registers.
    ///
    /// Each one runs on the actions it is scoped to, after the group's
    /// middleware and after the middleware given at registration with
    /// [`ResourceRoutes::middleware`]. None by default.
    fn middleware(&self) -> Vec<ControllerMiddleware> {
        Vec::new()
    }
}

fn not_implemented(action: &str) -> Response {
    Err(
        crate::http::HttpResponse::text(format!("Resource action '{action}' not implemented"))
            .status(404),
    )
}

/// What one resource registration says, in either form: its name, its
/// actions, and how its routes are named, parameterised and bound.
struct ResourceSpec {
    name: String,
    actions: Vec<ResourceAction>,
    name_overrides: std::collections::HashMap<String, String>,
    /// The parameter name of the last resource segment (`parameter()`).
    parameter: Option<String>,
    /// Parameter names by resource segment (`parameters()`).
    parameters: std::collections::HashMap<String, String>,
    /// Binding fields by parameter, set by `scoped()`, which also scopes
    /// every nested parameter to its parent.
    binding_fields: Option<std::collections::HashMap<String, String>>,
    /// The actions that bind soft-deleted rows (`with_trashed()`). An empty
    /// list means `show`, `edit` and `update`.
    trashed: Option<Vec<ResourceAction>>,
    /// What every route answers for a binding that finds nothing.
    missing: Option<MissingHook>,
    /// When `true`, route names are not registered. Used by
    /// nested-or-skip flows that want raw paths without polluting the
    /// process-global registry.
    suppress_names: bool,
    /// When set by `authorize_resource`, produces the per-action
    /// authorization middleware attached to each generated route.
    authorize: Option<AuthorizeFactory>,
    /// The middleware given at registration with `middleware()`, each on
    /// the actions it is scoped to.
    middleware: Vec<ControllerMiddleware>,
}

impl ResourceSpec {
    fn new(name: &str, actions: &[ResourceAction]) -> Self {
        Self {
            name: name.to_string(),
            actions: actions.to_vec(),
            name_overrides: std::collections::HashMap::new(),
            parameter: None,
            parameters: std::collections::HashMap::new(),
            binding_fields: None,
            trashed: None,
            missing: None,
            suppress_names: false,
            authorize: None,
            middleware: Vec::new(),
        }
    }
}

/// The handlers of a resource's actions.
enum ResourceHandlers {
    /// A [`ResourceController`], whose actions take the request.
    Controller(Arc<dyn ResourceController>),
    /// `#[handler]` functions, one per action, with what `#[handler]`
    /// recorded about each.
    Functions(Vec<(ResourceAction, Arc<BoxedHandler>, HandlerRef)>),
}

impl ResourceHandlers {
    fn handler(
        &self,
        action: ResourceAction,
    ) -> Result<(Arc<BoxedHandler>, HandlerRef), FrameworkError> {
        match self {
            ResourceHandlers::Controller(controller) => Ok((
                make_handler(controller.clone(), action),
                HandlerRef::BINDS_NOTHING,
            )),
            ResourceHandlers::Functions(functions) => functions
                .iter()
                .find(|(found, _, _)| *found == action)
                .map(|(_, handler, record)| (handler.clone(), *record))
                .ok_or_else(|| {
                    FrameworkError::internal(format!(
                        "the resource has no `{}` function for its `{}` action",
                        action.key(),
                        action.key()
                    ))
                }),
        }
    }
}

/// The builder methods both resource forms share. Each acts on
/// `self.spec`.
macro_rules! resource_spec_methods {
    () => {
        /// Override route names per action. Pairs are
        /// `(action_key, new_name)`, e.g. `("index", "posts.list")`.
        /// Mirrors Laravel's `names(['index' => 'posts.list'])`.
        ///
        /// Unknown action keys are silently ignored (matching Laravel's
        /// permissiveness - typos surface as the default name still being
        /// registered).
        pub fn names<'a, I>(mut self, overrides: I) -> Self
        where
            I: IntoIterator<Item = (&'a str, &'a str)>,
        {
            for (key, name) in overrides {
                if ResourceAction::from_key(key).is_some() {
                    self.spec
                        .name_overrides
                        .insert(key.to_string(), name.to_string());
                }
            }
            self
        }

        /// Rust-side alias of [`Self::names`].
        pub fn rename<'a, I>(self, overrides: I) -> Self
        where
            I: IntoIterator<Item = (&'a str, &'a str)>,
        {
            self.names(overrides)
        }

        /// Override the path-parameter name of the resource itself, the
        /// last segment of a nested name, from the default (the singular
        /// of the resource name - e.g. `posts` → `{post}`). Mirrors a
        /// single-pair `parameters(['posts' => 'post_id'])` call.
        pub fn parameter(mut self, name: &str) -> Self {
            self.spec.parameter = Some(name.to_string());
            self
        }

        /// Rename path parameters by resource segment: pairs are
        /// `(segment, parameter)`, e.g. `[("users", "author")]` makes
        /// `users.posts` register `/users/{author}/posts/{post}`. Mirrors
        /// Laravel's `parameters(['users' => 'author'])`.
        pub fn parameters<'a, I>(mut self, renames: I) -> Self
        where
            I: IntoIterator<Item = (&'a str, &'a str)>,
        {
            for (segment, parameter) in renames {
                self.spec
                    .parameters
                    .insert(segment.to_string(), parameter.to_string());
            }
            self
        }

        /// Scope the resource's bindings: each pair gives a parameter a
        /// binding field, `[("post", "slug")]` registering `{post:slug}`,
        /// and every nested parameter is looked up through its parent's
        /// relation (BIND-006). Mirrors Laravel's `scoped(['post' => 'slug'])`;
        /// an empty list scopes without fields.
        pub fn scoped<'a, I>(mut self, fields: I) -> Self
        where
            I: IntoIterator<Item = (&'a str, &'a str)>,
        {
            let mut map = self.spec.binding_fields.take().unwrap_or_default();
            for (parameter, field) in fields {
                map.insert(parameter.to_string(), field.to_string());
            }
            self.spec.binding_fields = Some(map);
            self
        }

        /// Bind soft-deleted rows on the routes of `actions`; `show`,
        /// `edit` and `update` when the list is empty (BIND-008). Mirrors
        /// Laravel's `withTrashed()`.
        pub fn with_trashed(mut self, actions: &[ResourceAction]) -> Self {
            self.spec.trashed = Some(actions.to_vec());
            self
        }

        /// Answer with `handler` instead of a 404 when a binding of one of
        /// the resource's routes finds nothing (BIND-009). Mirrors
        /// Laravel's `missing()`.
        pub fn missing<H, Fut>(mut self, handler: H) -> Self
        where
            H: Fn(Request) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Response> + Send + 'static,
        {
            self.spec.missing = Some(boxed_missing(handler));
            self
        }

        /// Suppress route-name registration entirely. Useful for nested
        /// resources where the parent already owns the namespace, or for
        /// tests that don't want the process-global registry touched.
        /// No Laravel analogue (Laravel has no opt-out flag); Rust-side
        /// convenience.
        pub fn unnamed(mut self) -> Self {
            self.spec.suppress_names = true;
            self
        }

        /// Gate every generated resource route behind its conventional ability,
        /// matching Laravel's `authorizeResource`.
        ///
        /// Without this, each generated `index`/`show`/`store`/`update`/`destroy`
        /// route is ungated unless the controller body remembers to call
        /// [`Gate::authorize`] itself - and a single forgotten `destroy` ships an
        /// ungated delete. `authorize_resource` closes that gap by attaching an
        /// authorization middleware to every route, mapping each action to its
        /// ability:
        ///
        /// | Action          | Ability |
        /// |-----------------|---------|
        /// | index / show    | `view`  |
        /// | create / store  | `create`|
        /// | edit / update   | `update`|
        /// | destroy         | `delete`|
        ///
        /// The middleware resolves the user of the route's guard as `U` and runs
        /// the mapped ability through the [`Gate`] against a [`Default`] value of
        /// the resource marker `R` (the gate discriminates on the `R` *type*, so
        /// the marker carries the same routing information for the policy that a
        /// model class would in Laravel). The route's guard is the one the last
        /// `AuthMiddleware` that passed the request on checked, else the default
        /// guard. No user on that guard, a user that is not a `U`, or a denied
        /// ability short-circuits the chain with `403` (or the gate's custom
        /// status) **before** the resource handler runs - fail-closed. Another
        /// guard's user never stands in.
        ///
        /// Define the abilities with [`Gate::define`] /
        /// [`Gate::define_with`] (or a `#[policy]`) keyed on `(ability, U, R)`.
        ///
        /// # Example
        ///
        /// ```rust,ignore
        /// Gate::define::<User, Post>("view",   |u, _p| u.is_member);
        /// Gate::define::<User, Post>("create", |u, _p| u.is_author);
        /// Gate::define::<User, Post>("update", |u, _p| u.is_author);
        /// Gate::define::<User, Post>("delete", |u, _p| u.is_admin);
        ///
        /// let router: Router = Router::new()
        ///     .resource("posts", PostsCtl)
        ///     .authorize_resource::<User, Post>()
        ///     .into();
        /// ```
        ///
        /// (Kept `ignore`: `authorize_resource::<U, R>` requires a `U:
        /// Authenticatable` user model and a gate-keyed `R` resource type
        /// that only a full application crate provides.)
        pub fn authorize_resource<U, R>(mut self) -> Self
        where
            U: Authenticatable + Clone + 'static,
            R: Default + Send + Sync + 'static,
        {
            self.spec.authorize = Some(Box::new(|action: ResourceAction| {
                let mw = ResourceAuthorizeMiddleware::<U, R> {
                    ability: ability_for(action),
                    _user: std::marker::PhantomData,
                    _resource: std::marker::PhantomData,
                };
                Some(boxed_as(mw))
            }));
            self
        }

        /// Run `middleware` on the actions it is scoped to, as a
        /// controller's own list does (Laravel's `middlewareFor`, scoped by
        /// `only` and `except`). It runs after the group's middleware and
        /// before the middleware the controller declares.
        ///
        /// A name that is not registered fails the registration:
        /// `try_register` returns the error and `register` panics at boot.
        pub fn middleware(mut self, middleware: ControllerMiddleware) -> Self {
            self.spec.middleware.push(middleware);
            self
        }
    };
}

/// Pending resource registration. Returned by [`Router::resource`] /
/// [`Router::api_resource`]; absorbs Laravel-shaped chains
/// (`only`/`except`/`names`/`parameters`/`scoped`/`with_trashed`/`missing`)
/// before finalizing into a [`Router`].
///
/// The router consumes the builder either via the conversion
/// `Router::from(routes)` / `routes.into()` or explicitly via
/// [`ResourceRoutes::register`]. Both paths panic on collision with an
/// existing route, mirroring [`Router::get`]'s boot-time-fail-loud
/// policy. Use [`ResourceRoutes::try_register`] for a fallible variant.
pub struct ResourceRoutes {
    router: Router,
    spec: ResourceSpec,
    controller: Arc<dyn ResourceController>,
}

impl ResourceRoutes {
    fn new(
        router: Router,
        name: &str,
        controller: Arc<dyn ResourceController>,
        defaults: &[ResourceAction],
    ) -> Self {
        Self {
            router,
            spec: ResourceSpec::new(name, defaults),
            controller,
        }
    }

    /// Restrict the generated routes to `actions`. Mirrors Laravel's
    /// `only(['index', 'show'])`. Duplicates are de-duplicated.
    pub fn only(mut self, actions: &[ResourceAction]) -> Self {
        let allowed: std::collections::HashSet<ResourceAction> = actions.iter().copied().collect();
        self.spec.actions.retain(|a| allowed.contains(a));
        self
    }

    /// Rust-side alias of [`Self::only`]. Same behaviour; lets call sites
    /// pick the idiom that reads better in context.
    pub fn keep(self, actions: &[ResourceAction]) -> Self {
        self.only(actions)
    }

    /// Remove the listed actions from the generated set. Mirrors
    /// Laravel's `except(['destroy'])`.
    pub fn except(mut self, actions: &[ResourceAction]) -> Self {
        let blocked: std::collections::HashSet<ResourceAction> = actions.iter().copied().collect();
        self.spec.actions.retain(|a| !blocked.contains(a));
        self
    }

    /// Rust-side alias of [`Self::except`].
    pub fn drop(self, actions: &[ResourceAction]) -> Self {
        self.except(actions)
    }

    resource_spec_methods!();

    /// Finalize the resource registration into a [`Router`].
    ///
    /// # Panics
    ///
    /// Panics on a duplicate route registration (matching
    /// [`Router::get`]'s boot-time-fail-loud policy) or a duplicate
    /// route name. Use [`Self::try_register`] for a fallible variant.
    pub fn register(self) -> Router {
        self.try_register().unwrap_or_else(|e| panic!("{e}"))
    }

    /// Fallible sibling of [`Self::register`]. Returns
    /// `Err(FrameworkError)` on duplicate registration; otherwise
    /// identical.
    pub fn try_register(self) -> Result<Router, FrameworkError> {
        let declared = self.controller.middleware();
        register_resource(
            self.router,
            self.spec,
            ResourceHandlers::Controller(self.controller),
            declared,
            None,
        )
    }
}

impl From<ResourceRoutes> for Router {
    fn from(routes: ResourceRoutes) -> Self {
        routes.register()
    }
}

/// A resource whose actions are `#[handler]` functions of one module,
/// found by action name: what `resource!` and `api_resource!` build.
///
/// The actions take bound arguments as any handler does (BIND-011):
///
/// ```rust,ignore
/// // controllers::posts::{index, create, store, show, edit, update, destroy}
/// routes! {
///     resource!("posts", controllers::posts),
///     resource!("users.posts", controllers::user_posts, only = [index, show])
///         .scoped([("post", "slug")]),
/// }
/// ```
///
/// Register it with [`ResourceDef::register`], which `routes!` calls, or
/// [`ResourceDef::try_register`].
pub struct ResourceDef {
    spec: ResourceSpec,
    functions: Vec<(ResourceAction, Arc<BoxedHandler>, HandlerRef)>,
    /// What the module's `pub fn middleware()` returned, the list a
    /// [`ResourceController`] gives through its trait method.
    declared: Vec<ControllerMiddleware>,
}

impl ResourceDef {
    /// A resource named `name` with `actions`, before their functions are
    /// added. Built by `resource!`.
    #[doc(hidden)]
    pub fn __new(name: &str, actions: &[ResourceAction]) -> Self {
        Self {
            spec: ResourceSpec::new(name, actions),
            functions: Vec::new(),
            declared: Vec::new(),
        }
    }

    /// The middleware the module declares in `pub fn middleware()`, or none
    /// when it declares no such function. Built by `resource!`.
    #[doc(hidden)]
    pub fn __controller_middleware(mut self, middleware: Vec<ControllerMiddleware>) -> Self {
        self.declared = middleware;
        self
    }

    /// Add the function of one action. Built by `resource!`, which names
    /// the function `<module>::<action>`, so a selected action the module
    /// does not define fails to compile.
    #[doc(hidden)]
    pub fn __action<H, Fut>(mut self, action: ResourceAction, handler: H) -> Self
    where
        H: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        let record = super::binding::handler_ref::<H>();
        let boxed: BoxedHandler = Box::new(move |req| Box::pin(handler(req)));
        self.functions.push((action, Arc::new(boxed), record));
        self
    }

    resource_spec_methods!();

    /// Register the resource's routes on `router`.
    ///
    /// # Panics
    ///
    /// On a duplicate route or route name, at boot. Use
    /// [`Self::try_register`] for the error instead.
    pub fn register(self, router: Router) -> Router {
        self.try_register(router).unwrap_or_else(|e| panic!("{e}"))
    }

    /// Fallible sibling of [`Self::register`].
    pub fn try_register(self, router: Router) -> Result<Router, FrameworkError> {
        register_resource(
            router,
            self.spec,
            ResourceHandlers::Functions(self.functions),
            self.declared,
            None,
        )
    }

    /// Register the resource's routes inside a `group!`: under the group's
    /// prefix and name prefix, with the group's middleware, session block
    /// and binding settings ahead of the resource's own.
    pub(crate) fn try_register_in_group(
        self,
        router: Router,
        group: &GroupScope<'_>,
    ) -> Result<Router, FrameworkError> {
        register_resource(
            router,
            self.spec,
            ResourceHandlers::Functions(self.functions),
            self.declared,
            Some(group),
        )
    }
}

/// What a `group!` gives the resources inside it: its joined prefix and
/// name prefix, its middleware (the parent groups' first), its session
/// block and its binding settings.
pub(crate) struct GroupScope<'a> {
    pub(crate) prefix: &'a str,
    pub(crate) name_prefix: &'a str,
    pub(crate) middleware: &'a [BoxedMiddleware],
    pub(crate) block: Option<SessionBlock>,
    pub(crate) bindings: &'a RouteBindingOptions,
}

/// What the `resource!` expansion calls. Not public API.
///
/// `resource!` glob-imports this module, then the controller module inside
/// it. A `pub fn middleware()` of the controller module shadows the empty
/// `middleware` below, so the expansion calls the
/// module's when there is one and this one otherwise: a proc macro cannot
/// see whether a module defines a function, but name resolution can.
#[doc(hidden)]
pub mod __resource_support {
    use super::{ControllerMiddleware, ResourceAction, ResourceDef};

    /// The middleware of a controller module that declares none.
    pub fn middleware() -> Vec<ControllerMiddleware> {
        Vec::new()
    }

    /// Start the resource. Called through the glob import, which keeps the
    /// import used when the controller module shadows `middleware`.
    pub fn __resource_def(name: &str, actions: &[ResourceAction]) -> ResourceDef {
        ResourceDef::__new(name, actions)
    }
}

/// Where a resource's routes go: the collection path (`index`, `store`,
/// `create`), the member path (`show`, `edit`, `update`, `destroy`), and
/// the name every route name starts with.
struct ResourceLayout {
    collection: String,
    member: String,
    route_name: String,
}

impl ResourceLayout {
    /// `posts` lays out `/posts` and `/posts/{post}`; a nested
    /// `users.posts` lays out `/users/{user}/posts` and
    /// `/users/{user}/posts/{post}`, named `users.posts.*` (BIND-011).
    fn of(spec: &ResourceSpec) -> Self {
        let name = spec.name.trim_start_matches('/');
        let segments: Vec<&str> = name.split('.').collect();
        let last = segments.len() - 1;
        let mut path = String::new();
        let mut member = String::new();
        for (index, segment) in segments.iter().enumerate() {
            let parameter = Self::parameter_of(spec, segment, index == last);
            let placeholder = match spec
                .binding_fields
                .as_ref()
                .and_then(|fields| fields.get(&parameter))
            {
                Some(field) => format!("{{{parameter}:{field}}}"),
                None => format!("{{{parameter}}}"),
            };
            path.push('/');
            path.push_str(segment);
            if index == last {
                member = format!("{path}/{placeholder}");
            } else {
                path.push('/');
                path.push_str(&placeholder);
            }
        }
        let route_name = if segments.len() > 1 {
            name.to_string()
        } else {
            path.trim_start_matches('/').replace('/', ".")
        };
        Self {
            collection: path,
            member,
            route_name,
        }
    }

    /// The parameter of one resource segment: the `parameter()` override
    /// for the resource itself, a `parameters()` rename, else the singular
    /// of the segment.
    fn parameter_of(spec: &ResourceSpec, segment: &str, is_last: bool) -> String {
        let key = segment.rsplit('/').next().unwrap_or(segment);
        if is_last && let Some(parameter) = &spec.parameter {
            return parameter.clone();
        }
        spec.parameters
            .get(key)
            .cloned()
            .unwrap_or_else(|| default_param_name(segment))
    }

    /// The `(method, path, default_name)` of one action.
    fn route(&self, action: ResourceAction) -> (hyper::Method, String, String) {
        let name = &self.route_name;
        match action {
            ResourceAction::Index => (
                hyper::Method::GET,
                self.collection.clone(),
                format!("{name}.index"),
            ),
            ResourceAction::Create => (
                hyper::Method::GET,
                format!("{}/create", self.collection),
                format!("{name}.create"),
            ),
            ResourceAction::Store => (
                hyper::Method::POST,
                self.collection.clone(),
                format!("{name}.store"),
            ),
            ResourceAction::Show => (
                hyper::Method::GET,
                self.member.clone(),
                format!("{name}.show"),
            ),
            ResourceAction::Edit => (
                hyper::Method::GET,
                format!("{}/edit", self.member),
                format!("{name}.edit"),
            ),
            ResourceAction::Update => (
                hyper::Method::PUT,
                self.member.clone(),
                format!("{name}.update"),
            ),
            ResourceAction::Destroy => (
                hyper::Method::DELETE,
                self.member.clone(),
                format!("{name}.destroy"),
            ),
        }
    }
}

/// Register every action of a resource: its route, its name, its
/// middleware and its binding settings. `update` answers PATCH beside PUT,
/// as Laravel registers it.
///
/// A route's middleware runs in Laravel's `gatherMiddleware` order: the
/// group's, then the middleware given at registration, then the
/// controller's own (`declared`: what [`ResourceController::middleware`]
/// or the module's `pub fn middleware()` returned), then the
/// `authorize_resource` check,
/// which needs the user the middleware before it authenticated. Each
/// [`ControllerMiddleware`] is resolved once, before any route is added,
/// so a name that is not registered fails the registration whole.
fn register_resource(
    mut router: Router,
    spec: ResourceSpec,
    handlers: ResourceHandlers,
    declared: Vec<ControllerMiddleware>,
    group: Option<&GroupScope<'_>>,
) -> Result<Router, FrameworkError> {
    let scoped_middleware: Vec<(ControllerMiddleware, Vec<BoxedMiddleware>)> = spec
        .middleware
        .iter()
        .cloned()
        .chain(declared)
        .map(|middleware| {
            let resolved = middleware.resolve()?;
            Ok((middleware, resolved))
        })
        .collect::<Result<_, FrameworkError>>()?;
    let layout = ResourceLayout::of(&spec);
    let trashed: Vec<ResourceAction> = match &spec.trashed {
        Some(listed) if !listed.is_empty() => listed.clone(),
        Some(_) => vec![
            ResourceAction::Show,
            ResourceAction::Edit,
            ResourceAction::Update,
        ],
        None => Vec::new(),
    };
    let options = |action: ResourceAction| {
        let own = RouteBindingOptions {
            scoped: spec.binding_fields.as_ref().map(|_| true),
            with_trashed: trashed.contains(&action),
            missing: spec.missing.clone(),
        };
        match group {
            Some(group) => own.within(group.bindings),
            None => own,
        }
    };

    for action in spec.actions.iter().copied() {
        let (method, path, default_name) = layout.route(action);
        // Inside a group the path takes the group's prefix, and a `:param`
        // in the prefix becomes `{param}`, as a group's routes do.
        let path = match group {
            Some(group) => convert_route_params(&join_paths(group.prefix, &path)),
            None => path,
        };
        let (handler, record) = handlers.handler(action)?;
        let mut methods = vec![method];
        if action == ResourceAction::Update {
            methods.push(hyper::Method::PATCH);
        }
        for method in methods.iter().cloned() {
            router.try_insert_method(&method, &path, handler.clone())?;
            router.note_route_record(method.clone(), &path, record);
            *router.bindings.options_mut(method.clone(), &path) = options(action);
            if let Some(group) = group {
                for middleware in group.middleware {
                    router.add_middleware(method.clone(), &path, middleware.clone());
                }
                if let Some(block) = group.block {
                    register_route_block(&method, &path, block);
                }
            }
            // Each middleware on the actions it is scoped to, keyed by the
            // matched pattern as the dispatcher looks it up. PATCH gets what
            // PUT gets, so neither verb skips a middleware of `update`.
            for (scope, resolved) in &scoped_middleware {
                if scope.applies_to(action) {
                    for middleware in resolved {
                        router.add_middleware(method.clone(), &path, middleware.clone());
                    }
                }
            }
            // The per-action authorization middleware (if
            // `authorize_resource` was called), last, after the middleware
            // that authenticates the user it checks.
            if let Some(factory) = spec.authorize.as_ref()
                && let Some(mw) = factory(action)
            {
                router.add_middleware(method, &path, mw);
            }
        }

        // The route NAME is claimed once in the process-wide table, and
        // recorded on the router for every verb of the action, so a PATCH
        // to `update` reports `posts.update` as its PUT does. A group's
        // name prefix goes in front, as it does for the group's routes.
        if !spec.suppress_names {
            let name = spec
                .name_overrides
                .get(action.key())
                .cloned()
                .unwrap_or(default_name);
            let effective_name = match group {
                Some(group) => format!("{}{name}", group.name_prefix),
                None => name,
            };
            super::router::try_register_route_name(&effective_name, &path)?;
            for method in methods {
                router.note_route_name(method, &path, &effective_name);
            }
        }
    }
    Ok(router)
}

/// Build a [`BoxedHandler`] that dispatches into one of the trait
/// methods. Each handler closes over the shared `Arc<controller>` so
/// the same instance services every action; concurrent handlers see
/// the same `&self` (the trait requires `Send + Sync`).
fn make_handler(
    controller: Arc<dyn ResourceController>,
    action: ResourceAction,
) -> Arc<BoxedHandler> {
    let inner: BoxedHandler = Box::new(move |req: Request| {
        let c = controller.clone();
        let fut: Pin<Box<dyn Future<Output = Response> + Send>> = match action {
            ResourceAction::Index => c.index(req),
            ResourceAction::Create => c.create(req),
            ResourceAction::Store => c.store(req),
            ResourceAction::Show => c.show(req),
            ResourceAction::Edit => c.edit(req),
            ResourceAction::Update => c.update(req),
            ResourceAction::Destroy => c.destroy(req),
        };
        fut
    });
    Arc::new(inner)
}

/// Pluralise → singular for the default path parameter name.
/// "posts" → "post", "categories" → "category", "people" → "people"
/// (no rule covers irregular plurals - pass `parameter("person_id")`
/// at the call site for those).
fn default_param_name(resource: &str) -> String {
    // Take the last path segment (we may have called with `admin/posts`).
    let last = resource.rsplit('/').next().unwrap_or(resource);
    if let Some(stripped) = last.strip_suffix("ies") {
        // categories -> category
        format!("{stripped}y")
    } else if let Some(stripped) = last.strip_suffix('s') {
        // posts -> post
        stripped.to_string()
    } else {
        last.to_string()
    }
}

impl Router {
    /// Register a standard 7-action REST resource at `path`.
    ///
    /// `controller` must implement [`ResourceController`]. Generates
    /// `index`/`create`/`store`/`show`/`edit`/`update`/`destroy`
    /// with conventional route names (`<resource>.<action>`). Use the
    /// returned [`ResourceRoutes`] to restrict / rename / re-parameterise.
    /// A dotted name nests: `users.posts` registers
    /// `/users/{user}/posts/{post}` under `users.posts.*`.
    ///
    /// For actions that take bound arguments, name a module of
    /// `#[handler]` functions with `resource!` instead.
    ///
    /// Mirrors Laravel's `Route::resource($name, $controller)` from
    /// `Illuminate/Routing/Router.php:347`.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use std::future::Future;
    /// # use std::pin::Pin;
    /// use suprnova::routing::{Router, ResourceController, ResourceAction};
    /// use suprnova::http::{Request, Response, text};
    ///
    /// struct PostsCtl;
    /// impl ResourceController for PostsCtl {
    ///     fn index(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
    ///         Box::pin(async { text("...") })
    ///     }
    ///     fn show(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
    ///         Box::pin(async { text("...") })
    ///     }
    /// }
    ///
    /// let router: Router = Router::new()
    ///     .resource("posts", PostsCtl)
    ///     .only(&[ResourceAction::Index, ResourceAction::Show])
    ///     .into();
    /// ```
    pub fn resource<C: ResourceController>(self, name: &str, controller: C) -> ResourceRoutes {
        ResourceRoutes::new(
            self,
            name,
            Arc::new(controller),
            ResourceAction::web_defaults(),
        )
    }

    /// API-flavoured resource registration. Drops `create` and `edit`
    /// (form-rendering routes that an API doesn't need), matching
    /// Laravel's `Route::apiResource($name, $controller)`.
    pub fn api_resource<C: ResourceController>(self, name: &str, controller: C) -> ResourceRoutes {
        ResourceRoutes::new(
            self,
            name,
            Arc::new(controller),
            ResourceAction::api_defaults(),
        )
    }

    /// Bulk-register multiple resources from `(name, controller)` pairs.
    /// Mirrors `Route::resources(['posts' => PostsCtl::class, ...])`.
    ///
    /// All resources are registered with default actions. Use individual
    /// `resource()` calls when per-resource customisation is needed.
    pub fn resources<I>(mut self, resources: I) -> Self
    where
        I: IntoIterator<Item = (&'static str, Box<dyn ResourceController>)>,
    {
        for (name, ctl) in resources {
            self = ResourceRoutes::new(self, name, Arc::from(ctl), ResourceAction::web_defaults())
                .register();
        }
        self
    }

    /// Bulk API variant of [`Self::resources`]. Mirrors
    /// `Route::apiResources(...)`.
    pub fn api_resources<I>(mut self, resources: I) -> Self
    where
        I: IntoIterator<Item = (&'static str, Box<dyn ResourceController>)>,
    {
        for (name, ctl) in resources {
            self = ResourceRoutes::new(self, name, Arc::from(ctl), ResourceAction::api_defaults())
                .register();
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Request, Response, text};
    use hyper::Method;

    struct Ctl;
    impl ResourceController for Ctl {
        fn index(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
            Box::pin(async { text("index") })
        }
        fn show(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
            Box::pin(async { text("show") })
        }
    }

    #[test]
    fn web_resource_registers_seven_routes() {
        let router: Router = Router::new().resource("posts", Ctl).unnamed().into();
        assert!(router.match_route(&Method::GET, "/posts").is_some());
        assert!(router.match_route(&Method::GET, "/posts/create").is_some());
        assert!(router.match_route(&Method::POST, "/posts").is_some());
        assert!(router.match_route(&Method::GET, "/posts/42").is_some());
        assert!(router.match_route(&Method::GET, "/posts/42/edit").is_some());
        assert!(router.match_route(&Method::PUT, "/posts/42").is_some());
        assert!(router.match_route(&Method::DELETE, "/posts/42").is_some());
    }

    #[test]
    fn update_action_registers_both_put_and_patch() {
        // Laravel registers PUT and PATCH for the update action; both
        // verbs must route to the same handler. The action-loop
        // dispatches PUT directly; the PATCH-alongside step layers a
        // parallel entry on the same path. Without it, a client sending
        // `PATCH /posts/42` would 404 even though the resource declared
        // an `update` action.
        let router: Router = Router::new().resource("posts", Ctl).unnamed().into();
        assert!(
            router.match_route(&Method::PUT, "/posts/42").is_some(),
            "PUT /posts/{{id}} must route to update"
        );
        assert!(
            router.match_route(&Method::PATCH, "/posts/42").is_some(),
            "PATCH /posts/{{id}} must route to update"
        );
    }

    #[test]
    fn update_dropped_does_not_register_patch() {
        // The PATCH-alongside-PUT step is gated on Update being in the
        // action set. If a caller drops Update via `.except`, neither
        // PUT nor PATCH should be registered.
        let router: Router = Router::new()
            .resource("posts", Ctl)
            .except(&[ResourceAction::Update])
            .unnamed()
            .into();
        assert!(router.match_route(&Method::PUT, "/posts/42").is_none());
        assert!(router.match_route(&Method::PATCH, "/posts/42").is_none());
    }

    #[test]
    fn api_resource_drops_create_and_edit() {
        let router: Router = Router::new().api_resource("posts", Ctl).unnamed().into();
        assert!(router.match_route(&Method::GET, "/posts").is_some());
        assert!(router.match_route(&Method::POST, "/posts").is_some());
        assert!(router.match_route(&Method::GET, "/posts/42").is_some());
        assert!(router.match_route(&Method::PUT, "/posts/42").is_some());
        assert!(router.match_route(&Method::DELETE, "/posts/42").is_some());

        // No `/posts/{id}/edit` because the Edit action wasn't registered.
        // `/posts/create` would match Show via the `{post}` capture (matchit
        // is path-shape-based), but that's the same shadowing Laravel
        // exhibits and is the reason web `resource()` registers the explicit
        // `/create` route BEFORE `/{post}`. For API-mode resources callers
        // accept "create looks like a resource id" by design.
        assert!(router.match_route(&Method::GET, "/posts/42/edit").is_none());
    }

    #[test]
    fn only_restricts_to_listed_actions() {
        let router: Router = Router::new()
            .resource("posts", Ctl)
            .only(&[ResourceAction::Index, ResourceAction::Show])
            .unnamed()
            .into();
        assert!(router.match_route(&Method::GET, "/posts").is_some());
        assert!(router.match_route(&Method::GET, "/posts/1").is_some());
        // Store / update / destroy / create / edit must NOT be registered.
        assert!(router.match_route(&Method::POST, "/posts").is_none());
        assert!(router.match_route(&Method::PUT, "/posts/1").is_none());
        assert!(router.match_route(&Method::DELETE, "/posts/1").is_none());
    }

    #[test]
    fn except_drops_listed_actions() {
        let router: Router = Router::new()
            .resource("posts", Ctl)
            .except(&[ResourceAction::Destroy])
            .unnamed()
            .into();
        assert!(router.match_route(&Method::DELETE, "/posts/1").is_none());
        assert!(router.match_route(&Method::PUT, "/posts/1").is_some());
    }

    #[test]
    fn keep_is_alias_for_only() {
        let router: Router = Router::new()
            .resource("widgets", Ctl)
            .keep(&[ResourceAction::Index])
            .unnamed()
            .into();
        assert!(router.match_route(&Method::GET, "/widgets").is_some());
        assert!(router.match_route(&Method::POST, "/widgets").is_none());
    }

    #[test]
    fn drop_is_alias_for_except() {
        let router: Router = Router::new()
            .resource("widgets", Ctl)
            .drop(&[ResourceAction::Index])
            .unnamed()
            .into();
        assert!(router.match_route(&Method::GET, "/widgets").is_none());
        assert!(router.match_route(&Method::POST, "/widgets").is_some());
    }

    #[test]
    fn parameter_overrides_path_parameter_name() {
        let router: Router = Router::new()
            .resource("posts", Ctl)
            .parameter("post_id")
            .only(&[ResourceAction::Show])
            .unnamed()
            .into();
        let m = router.match_route(&Method::GET, "/posts/42");
        let (pattern, _h, params) = m.expect("show must match");
        assert_eq!(pattern, "/posts/{post_id}");
        assert_eq!(params.get("post_id"), Some(&"42".to_string()));
    }

    #[test]
    #[serial_test::serial(route_registry)]
    fn default_route_names_register() {
        crate::routing::clear_route_names_for_test();
        let _router: Router = Router::new().resource("posts", Ctl).into();
        let u = crate::routing::route("posts.index", &[]);
        assert_eq!(u.as_deref(), Some("/posts"));
        let u = crate::routing::route("posts.show", &[("post", "42")]);
        assert_eq!(u.as_deref(), Some("/posts/42"));
    }

    #[test]
    #[serial_test::serial(route_registry)]
    fn names_overrides_default_name() {
        crate::routing::clear_route_names_for_test();
        let _router: Router = Router::new()
            .resource("posts", Ctl)
            .only(&[ResourceAction::Index])
            .names([("index", "posts.list")])
            .into();
        assert_eq!(
            crate::routing::route("posts.list", &[]).as_deref(),
            Some("/posts"),
        );
        // Default name is NOT registered when overridden.
        assert!(crate::routing::route("posts.index", &[]).is_none());
    }

    #[test]
    fn default_param_singularises_common_plurals() {
        assert_eq!(default_param_name("posts"), "post");
        assert_eq!(default_param_name("categories"), "category");
        assert_eq!(default_param_name("people"), "people"); // irregular, untouched
        assert_eq!(default_param_name("admin/posts"), "post");
    }
}
