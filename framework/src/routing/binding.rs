//! Route binding at the router: what `#[handler]` records about a handler,
//! the binding settings of each route, the router-wide binders, the checks
//! that run before the first request, and the lookups a request runs.
//!
//! A handler argument binds from the route parameter its name names when
//! its type implements [`RouteBinding`]. `#[handler]` decides that at
//! compile time and records every argument in a [`HandlerRecord`]: the
//! parameter it reads, its type, and whether it binds, reads a path value
//! or reads the body. The router finds the record by the handler's type
//! wherever a handler is boxed. Before the first request, the router checks
//! every recorded route against its path and its models and builds a plan:
//! which arguments bind, in path order, which parent scopes each child,
//! which binding field each matches. A request runs the plan after the
//! route's middleware and before the handler, so a missing row answers 404
//! (or the route's `missing()` response) before the handler reads the body.
//!
//! A route whose handler carries no record, a closure or a generic
//! `#[handler]` function, is not checked before the first request, and a
//! closure binds nothing itself. The request carries the route's binding
//! settings instead, and a handler that binds without a plan, a generic one
//! or a recorded one a closure calls, describes its arguments at the
//! request: its concrete bound arguments get the plan a recorded handler's
//! would, built at its first request on the route and kept for the next
//! ones, so they bind in path order with the route's binding fields,
//! scoping, binders, `with_trashed()` and `missing()`. A problem the
//! startup checks would refuse answers that request with the refusal
//! instead of binding. A generic handler's generic arguments read the body.
//!
//! A route's `missing()` handler binds its own arguments under the same
//! settings when it runs: a recorded one through a plan the startup checks
//! build, one with no record through the route's settings. Its own binding
//! that finds nothing answers 404.

use crate::database::route_binding::{
    BoundChild, ChildBindings, RouteBinding, RouteBindingInfo, RouteLookup,
};
use crate::error::FrameworkError;
use crate::http::{FromParam, FromRequest, HttpResponse, Request, Response, RouteBindingState};
use crate::routing::router::BoxedHandler;
use hyper::Method;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::{Arc, OnceLock, RwLock};

/// A bound value, held without its type until the handler takes it back.
pub(crate) type Erased = Box<dyn Any + Send + Sync>;

/// A handler a route calls when one of its bindings finds nothing.
pub type MissingHandler = Arc<BoxedHandler>;

// ---------------------------------------------------------------------------
// Handler records
// ---------------------------------------------------------------------------

/// What `#[handler]` records about one handler function, for the checks
/// the router runs before the first request (BIND-004).
///
/// `#[handler]` submits one per non-generic handler through `inventory`,
/// keyed by the handler function's type, and the router reads it wherever
/// it boxes a handler.
pub struct HandlerRecord {
    handler: fn() -> TypeId,
    name: &'static str,
    module: &'static str,
    args: fn() -> Vec<HandlerArg>,
}

impl HandlerRecord {
    /// The record of the handler whose type `handler` returns, named
    /// `name` in `module`, with the arguments `args` lists in declaration
    /// order. Built by `#[handler]`.
    #[doc(hidden)]
    pub const fn new(
        handler: fn() -> TypeId,
        name: &'static str,
        module: &'static str,
        args: fn() -> Vec<HandlerArg>,
    ) -> Self {
        Self {
            handler,
            name,
            module,
            args,
        }
    }

    /// The handler's path, `app::controllers::posts::show`, for errors.
    pub fn path(&self) -> String {
        format!("{}::{}", self.module, self.name)
    }

    /// The handler's arguments, in declaration order.
    pub fn args(&self) -> Vec<HandlerArg> {
        (self.args)()
    }
}

impl std::fmt::Debug for HandlerRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HandlerRecord")
            .field("handler", &self.path())
            .finish_non_exhaustive()
    }
}

inventory::collect!(HandlerRecord);

/// The type of the value `value` points at. `#[handler]` uses it to name
/// a function's type, which no type syntax can spell.
#[doc(hidden)]
pub fn __type_id_of<T: 'static>(_value: &T) -> TypeId {
    TypeId::of::<T>()
}

/// The record of the handler `H`, when `#[handler]` recorded one.
pub(crate) fn record_of<H: 'static>() -> Option<&'static HandlerRecord> {
    static RECORDS: OnceLock<HashMap<TypeId, &'static HandlerRecord>> = OnceLock::new();
    RECORDS
        .get_or_init(|| {
            inventory::iter::<HandlerRecord>()
                .map(|record| ((record.handler)(), record))
                .collect()
        })
        .get(&TypeId::of::<H>())
        .copied()
}

/// One argument of a `#[handler]` function.
#[derive(Clone)]
pub struct HandlerArg {
    name: &'static str,
    type_name: &'static str,
    optional: bool,
    kind: HandlerArgKind,
}

/// How one handler argument reads the request.
#[derive(Clone)]
pub enum HandlerArgKind {
    /// It binds from the route parameter of its name, through
    /// [`RouteBinding`].
    Bound(BoundArg),
    /// It is a path value read through [`FromParam`] (`id: i64`).
    PathValue,
    /// It reads the request body: `Request`, or a form request.
    Body,
}

impl HandlerArg {
    /// An argument `name: type_name` that binds through `ops`. `optional`
    /// when it is an `Option`, which binds `None` when its optional
    /// parameter is absent.
    #[doc(hidden)]
    pub fn bound(
        name: &'static str,
        type_name: &'static str,
        optional: bool,
        ops: BoundArg,
    ) -> Self {
        Self {
            name,
            type_name,
            optional,
            kind: HandlerArgKind::Bound(ops),
        }
    }

    /// An argument read as a path value.
    #[doc(hidden)]
    pub fn path_value(name: &'static str, type_name: &'static str, optional: bool) -> Self {
        Self {
            name,
            type_name,
            optional,
            kind: HandlerArgKind::PathValue,
        }
    }

    /// An argument that reads the request body.
    #[doc(hidden)]
    pub fn body(name: &'static str, type_name: &'static str) -> Self {
        Self {
            name,
            type_name,
            optional: false,
            kind: HandlerArgKind::Body,
        }
    }

    /// The argument's name, which is the route parameter it reads unless
    /// it reads the body.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The argument's type, as written.
    pub fn type_name(&self) -> &'static str {
        self.type_name
    }

    /// Whether the argument is an `Option`.
    pub fn is_optional(&self) -> bool {
        self.optional
    }

    /// How the argument reads the request.
    pub fn kind(&self) -> &HandlerArgKind {
        &self.kind
    }
}

/// Look a child up under a parent held without its type.
type ChildLookup = for<'a> fn(
    &'a (dyn Any + Send + Sync),
    &'a str,
    &'a str,
    Option<&'a str>,
    bool,
) -> RouteLookup<'a, Option<BoundChild>>;

/// Every route's plan, or the startup refusal.
type Plans = Result<HashMap<(Method, String), RouteBinds>, FrameworkError>;

/// One handler's plan on a route without a record: what it binds, or
/// nothing, or the refusal its arguments meet on that route.
type UnrecordedPlan = Result<Option<Arc<RoutePlan>>, FrameworkError>;

/// The binding operations of one argument type `T`, without the type: what
/// the router calls to look a value up, to look a child up under it, and to
/// turn what a binder or a relation found into a `T`.
#[derive(Clone, Copy)]
pub struct BoundArg {
    info: fn() -> RouteBindingInfo,
    resolve: for<'a> fn(&'a str, Option<&'a str>, bool) -> RouteLookup<'a, Option<Erased>>,
    child: ChildLookup,
    adopt: fn(BoundChild) -> Result<Erased, BoundChild>,
}

impl BoundArg {
    /// The operations of `T`.
    pub fn of<T: RouteBinding>() -> Self {
        Self {
            info: T::route_binding_info,
            resolve: resolve_erased::<T>,
            child: child_erased::<T>,
            adopt: adopt_erased::<T>,
        }
    }

    /// What `T` tells the startup checks.
    pub fn info(&self) -> RouteBindingInfo {
        (self.info)()
    }
}

fn resolve_erased<'a, T: RouteBinding>(
    value: &'a str,
    field: Option<&'a str>,
    trashed: bool,
) -> RouteLookup<'a, Option<Erased>> {
    Box::pin(async move {
        let found = if trashed {
            T::resolve_soft_deletable_route_binding(value, field).await?
        } else {
            T::resolve_route_binding(value, field).await?
        };
        Ok(found.map(|found| Box::new(found) as Erased))
    })
}

fn child_erased<'a, T: RouteBinding>(
    parent: &'a (dyn Any + Send + Sync),
    child: &'a str,
    value: &'a str,
    field: Option<&'a str>,
    trashed: bool,
) -> RouteLookup<'a, Option<BoundChild>> {
    Box::pin(async move {
        let Some(parent) = parent.downcast_ref::<T>() else {
            return Err(FrameworkError::internal(format!(
                "the parent of the route parameter `{child}` is not a `{}`",
                std::any::type_name::<T>()
            )));
        };
        if trashed {
            parent
                .resolve_soft_deletable_child_route_binding(child, value, field)
                .await
        } else {
            parent
                .resolve_child_route_binding(child, value, field)
                .await
        }
    })
}

fn adopt_erased<T: RouteBinding>(value: BoundChild) -> Result<Erased, BoundChild> {
    T::__from_bound(value).map(|found| Box::new(found) as Erased)
}

// ---------------------------------------------------------------------------
// What the generated handler code calls
// ---------------------------------------------------------------------------

/// The request a `#[handler]` function reads its arguments from: the path
/// values, the values the router bound, and the request itself, which one
/// argument at most may take.
#[doc(hidden)]
pub struct HandlerInput {
    request: Option<Request>,
    params: HashMap<String, String>,
    bound: Option<Vec<Option<Erased>>>,
    /// The settings of a route that planned nothing for this handler.
    unplanned: Option<Arc<UnrecordedRoute>>,
}

impl HandlerInput {
    /// Take the request apart for the handler's arguments.
    pub fn new(mut request: Request) -> Self {
        let (bound, unplanned) = match request.take_route_bindings() {
            Some(RouteBindingState::Bound(values)) => (Some(values), None),
            Some(RouteBindingState::Unplanned(route)) => (None, Some(route)),
            None => (None, None),
        };
        let params = request.params().clone();
        Self {
            request: Some(request),
            params,
            bound,
            unplanned,
        }
    }

    /// Whether the route planned nothing for this handler and handed it its
    /// settings instead, so [`Self::__bind_unplanned`] binds its arguments.
    /// A recorded handler on its own route never does.
    #[inline]
    pub fn __binds_unplanned(&self) -> bool {
        self.unplanned.is_some()
    }

    /// The path value `name` as a `T`. A route whose handler is recorded
    /// declares every parameter its arguments read, which the router
    /// checks before the first request.
    pub fn path<T: FromParam>(&self, name: &str) -> Result<T, FrameworkError> {
        let value = self
            .params
            .get(name)
            .ok_or_else(|| FrameworkError::param(name))?;
        T::from_param(value)
    }

    /// The path value `name` as a `T`, or `None` when the optional
    /// parameter is absent.
    pub fn optional_path<T: FromParam>(&self, name: &str) -> Result<Option<T>, FrameworkError> {
        self.params
            .get(name)
            .map(|value| T::from_param(value))
            .transpose()
    }

    /// The request itself, for the one argument that reads the body.
    pub fn request(&mut self) -> Result<Request, FrameworkError> {
        self.request.take().ok_or_else(|| {
            FrameworkError::internal(
                "a handler has more than one argument that reads the request body",
            )
        })
    }

    /// The value the route bound for argument `index`. A handler the route
    /// handed neither values nor settings, one called outside a matched
    /// route or from inside another handler, looks the parameter `name` up
    /// by its route key instead.
    pub async fn bound<T: RouteBinding>(
        &mut self,
        index: usize,
        name: &str,
    ) -> Result<T, FrameworkError> {
        match self.optional_bound::<T>(index, name).await? {
            Some(found) => Ok(found),
            None => Err(FrameworkError::param(name)),
        }
    }

    /// Bind the concrete arguments of a handler the route planned nothing
    /// for, as the route binds a recorded handler's (BIND-004): a generic
    /// handler, a recorded one a closure route calls, or a `missing()`
    /// handler. `args` describes every argument in declaration order, and
    /// the route plans them once, keyed by `key`, which the generated code
    /// makes unique to the handler.
    ///
    /// `Ok(None)` once they are bound, or when the route handed no
    /// settings. A binding that finds nothing answers `Ok(Some(response))`
    /// with the route's `missing()` response or the 404 when
    /// `answers_missing`, which the generated code sets when the handler
    /// returns [`Response`], and the 404's error otherwise. A failed lookup
    /// or a refusal of the route's checks is the error.
    ///
    /// # Errors
    ///
    /// The 404 of a binding that finds nothing, for a handler that cannot
    /// return the `missing()` response; a lookup's error; or the refusal of
    /// a binding field, scoped child or binder the startup checks would
    /// refuse.
    pub async fn __bind_unplanned(
        &mut self,
        key: TypeId,
        handler: &'static str,
        args: impl FnOnce() -> Vec<HandlerArg>,
        answers_missing: bool,
    ) -> Result<Option<Response>, FrameworkError> {
        let Some(route) = self.unplanned.take() else {
            return Ok(None);
        };
        let Some(request) = self.request.as_ref() else {
            return Ok(None);
        };
        let matched = MatchedRoute {
            method: request.method().clone(),
            pattern: request.route_pattern().unwrap_or_default().to_owned(),
            params: self.params.clone(),
        };
        let Some(plan) = route.plan(key, handler, args)? else {
            // Nothing binds; no argument falls back to an unplanned lookup.
            self.bound = Some(Vec::new());
            return Ok(None);
        };
        match bind_values(&plan, &matched).await {
            Ok(values) => {
                self.bound = Some(values);
                Ok(None)
            }
            Err(Unbound::Miss(index)) => {
                let binding = &plan.bindings[index];
                match self.request.take() {
                    Some(request) if answers_missing => {
                        Ok(Some(miss(&plan, binding, request).await))
                    }
                    _ => Err(FrameworkError::model_not_found(binding.info.name())),
                }
            }
            Err(Unbound::Failed(error)) => Err(error),
        }
    }

    /// [`Self::bound`] for an `Option` argument: `None` when its optional
    /// parameter is absent.
    pub async fn optional_bound<T: RouteBinding>(
        &mut self,
        index: usize,
        name: &str,
    ) -> Result<Option<T>, FrameworkError> {
        if let Some(bound) = self.bound.as_mut() {
            return match bound.get_mut(index).and_then(Option::take) {
                Some(found) => found
                    .downcast::<T>()
                    .map(|found| Some(*found))
                    .map_err(|_| {
                        FrameworkError::internal(format!(
                            "the route bound `{name}` to another type than `{}`",
                            std::any::type_name::<T>()
                        ))
                    }),
                None => Ok(None),
            };
        }
        let Some(value) = self.params.get(name) else {
            return Ok(None);
        };
        match T::resolve_route_binding(value, None).await? {
            Some(found) => Ok(Some(found)),
            None => Err(FrameworkError::model_not_found(
                T::route_binding_info().name(),
            )),
        }
    }
}

/// Autoref-specialised probe the code `#[handler]` generates calls for an
/// argument whose spelling does not say how it reads the request: a type
/// that implements [`RouteBinding`] binds, any other reads the body.
#[doc(hidden)]
pub struct __ArgProbe<T>(PhantomData<fn() -> T>);

impl<T> __ArgProbe<T> {
    /// A probe for `T`.
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T> Default for __ArgProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// [`__ArgProbe`] for an `Option<T>` argument: an `Option` of a type that
/// binds is an optional binding; any other `Option` reads the body.
#[doc(hidden)]
pub struct __OptionalArgProbe<T>(PhantomData<fn() -> T>);

impl<T> __OptionalArgProbe<T> {
    /// A probe for `Option<T>`.
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T> Default for __OptionalArgProbe<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// How a probed argument reads the request. The generated code calls it
/// on `&&probe`: method lookup tries the impl on `&probe` first, which
/// binds and applies only to a type that implements [`RouteBinding`], then
/// the impl on the probe itself, which reads the body. One trait for every
/// probe means the generated code imports one name, and uses it wherever
/// it imports it.
#[doc(hidden)]
pub trait __ArgSource {
    /// The argument's type.
    type Value;
    /// The argument's record.
    fn __record(&self, name: &'static str, type_name: &'static str) -> HandlerArg;
    /// The bound value, or `None` for an argument that reads the body.
    fn __bind<'a>(
        &self,
        input: &'a mut HandlerInput,
        index: usize,
        name: &'a str,
    ) -> RouteLookup<'a, Option<Self::Value>>;
    /// The argument, read from the request. Never called for an argument
    /// that binds.
    fn __read_body<'a>(&self, input: &'a mut HandlerInput) -> RouteLookup<'a, Self::Value>;
}

impl<T: RouteBinding> __ArgSource for &__ArgProbe<T> {
    type Value = T;
    fn __record(&self, name: &'static str, type_name: &'static str) -> HandlerArg {
        HandlerArg::bound(name, type_name, false, BoundArg::of::<T>())
    }
    fn __bind<'a>(
        &self,
        input: &'a mut HandlerInput,
        index: usize,
        name: &'a str,
    ) -> RouteLookup<'a, Option<T>> {
        Box::pin(async move { input.bound::<T>(index, name).await.map(Some) })
    }
    fn __read_body<'a>(&self, _input: &'a mut HandlerInput) -> RouteLookup<'a, T> {
        Box::pin(async {
            Err(FrameworkError::internal(
                "a bound handler argument was read as the request body",
            ))
        })
    }
}

impl<T: FromRequest + 'static> __ArgSource for __ArgProbe<T> {
    type Value = T;
    fn __record(&self, name: &'static str, type_name: &'static str) -> HandlerArg {
        HandlerArg::body(name, type_name)
    }
    fn __bind<'a>(
        &self,
        _input: &'a mut HandlerInput,
        _index: usize,
        _name: &'a str,
    ) -> RouteLookup<'a, Option<T>> {
        Box::pin(async { Ok(None) })
    }
    fn __read_body<'a>(&self, input: &'a mut HandlerInput) -> RouteLookup<'a, T> {
        Box::pin(async move { T::from_request(input.request()?).await })
    }
}

impl<T: RouteBinding> __ArgSource for &__OptionalArgProbe<T> {
    type Value = Option<T>;
    fn __record(&self, name: &'static str, type_name: &'static str) -> HandlerArg {
        HandlerArg::bound(name, type_name, true, BoundArg::of::<T>())
    }
    fn __bind<'a>(
        &self,
        input: &'a mut HandlerInput,
        index: usize,
        name: &'a str,
    ) -> RouteLookup<'a, Option<Option<T>>> {
        Box::pin(async move { input.optional_bound::<T>(index, name).await.map(Some) })
    }
    fn __read_body<'a>(&self, _input: &'a mut HandlerInput) -> RouteLookup<'a, Option<T>> {
        Box::pin(async {
            Err(FrameworkError::internal(
                "a bound handler argument was read as the request body",
            ))
        })
    }
}

impl<T> __ArgSource for __OptionalArgProbe<T>
where
    Option<T>: FromRequest + 'static,
{
    type Value = Option<T>;
    fn __record(&self, name: &'static str, type_name: &'static str) -> HandlerArg {
        HandlerArg::body(name, type_name)
    }
    fn __bind<'a>(
        &self,
        _input: &'a mut HandlerInput,
        _index: usize,
        _name: &'a str,
    ) -> RouteLookup<'a, Option<Option<T>>> {
        Box::pin(async { Ok(None) })
    }
    fn __read_body<'a>(&self, input: &'a mut HandlerInput) -> RouteLookup<'a, Option<T>> {
        Box::pin(async move { <Option<T>>::from_request(input.request()?).await })
    }
}

/// The value an `#[authorize]` parameter target binds, which must
/// implement [`RouteBinding`]: the check runs before the body is read, on
/// a value the route supplies.
#[doc(hidden)]
pub async fn __authorize_target<T: RouteBinding>(
    input: &mut HandlerInput,
    index: usize,
    name: &str,
) -> Result<T, FrameworkError> {
    input.bound::<T>(index, name).await
}

/// Autoref-specialised probe of a handler's return type, for the code
/// `#[handler]` generates where the route planned nothing for the handler:
/// a handler that returns [`Response`] returns the route's `missing()`
/// response as it is, `Ok` or `Err`, as a recorded handler's route does.
#[doc(hidden)]
pub struct __HandlerOutput<O>(PhantomData<fn() -> O>);

impl<O> __HandlerOutput<O> {
    /// A probe for the return type `O`.
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<O> Default for __HandlerOutput<O> {
    fn default() -> Self {
        Self::new()
    }
}

/// What a handler of one return type can do with the route's `missing()`
/// response. The generated code calls it on `&&probe`: the impl on
/// `&probe`, for [`Response`], applies first; the impl on the probe itself
/// covers every other return type the generated code's `?` accepts.
#[doc(hidden)]
pub trait __OutputSource {
    /// The handler's return type.
    type Output;
    /// Whether the handler can return the `missing()` response.
    fn __answers_missing(&self) -> bool;
    /// The handler's return value for the `missing()` response.
    fn __missing_answer(&self, answer: Response) -> Self::Output;
}

impl __OutputSource for &__HandlerOutput<Response> {
    type Output = Response;
    fn __answers_missing(&self) -> bool {
        true
    }
    fn __missing_answer(&self, answer: Response) -> Response {
        answer
    }
}

impl<T, E: From<FrameworkError>> __OutputSource for __HandlerOutput<Result<T, E>> {
    type Output = Result<T, E>;
    fn __answers_missing(&self) -> bool {
        false
    }
    fn __missing_answer(&self, _answer: Response) -> Result<T, E> {
        // Never asked for: a handler that cannot return the response gets
        // the 404's error in its place.
        Err(E::from(FrameworkError::internal(
            "a missing() response reached a handler whose return type cannot carry it",
        )))
    }
}

// ---------------------------------------------------------------------------
// Route settings and router-wide binders
// ---------------------------------------------------------------------------

/// The binding settings of one route: whether its children are scoped,
/// whether it binds soft-deleted rows, and what it answers for a binding
/// that finds nothing.
#[derive(Clone, Default)]
pub(crate) struct RouteBindingOptions {
    /// `Some(true)` for `scope_bindings()`, `Some(false)` for
    /// `without_scoped_bindings()`, `None` for neither.
    pub(crate) scoped: Option<bool>,
    /// `with_trashed()`.
    pub(crate) with_trashed: bool,
    /// `missing(handler)`.
    pub(crate) missing: Option<MissingHook>,
}

impl RouteBindingOptions {
    /// These settings, with what `group` sets for every route of a group
    /// filled in where these set nothing.
    pub(crate) fn within(mut self, group: &RouteBindingOptions) -> Self {
        if self.scoped.is_none() {
            self.scoped = group.scoped;
        }
        self.with_trashed |= group.with_trashed;
        if self.missing.is_none() {
            self.missing = group.missing.clone();
        }
        self
    }
}

/// A `missing()` handler, boxed, with what `#[handler]` recorded about it,
/// so the startup checks see it as they see a route's handler (BIND-004).
#[derive(Clone)]
pub(crate) struct MissingHook {
    handler: MissingHandler,
    record: Option<&'static HandlerRecord>,
}

/// A route's `missing()` handler, with how its own bound arguments bind
/// when it runs: under the route's settings, as the route's handler's do.
pub(crate) struct RouteMissing {
    handler: MissingHandler,
    binds: MissingBinds,
}

/// How a `missing()` handler's bound arguments bind.
enum MissingBinds {
    /// A recorded handler: the plan the startup checks built for it against
    /// the route, `None` when it binds nothing.
    Planned(Option<Arc<RoutePlan>>),
    /// A handler with no record: the route's settings, which a generic one
    /// plans against at the request and a closure ignores.
    Unplanned(Arc<UnrecordedRoute>),
}

/// Run the `missing()` handler on `request`, its own bound arguments bound
/// first. Its own binding that finds nothing answers 404: the handler is
/// never called for its own miss.
async fn answer_missing(missing: &RouteMissing, mut request: Request) -> Response {
    match &missing.binds {
        MissingBinds::Planned(Some(plan)) => {
            let route = MatchedRoute {
                method: request.method().clone(),
                pattern: request.route_pattern().unwrap_or_default().to_owned(),
                params: request.params().clone(),
            };
            match bind_values(plan, &route).await {
                Ok(values) => request.set_route_bindings(RouteBindingState::Bound(values)),
                Err(Unbound::Miss(index)) => {
                    return Err(HttpResponse::from(FrameworkError::model_not_found(
                        plan.bindings[index].info.name(),
                    )));
                }
                Err(Unbound::Failed(error)) => return Err(HttpResponse::from(error)),
            }
        }
        MissingBinds::Planned(None) => {}
        MissingBinds::Unplanned(route) => {
            request.set_route_bindings(RouteBindingState::Unplanned(route.clone()));
        }
    }
    (missing.handler)(request).await
}

/// Box a `missing()` handler the way a route handler is boxed, keeping its
/// record.
pub(crate) fn boxed_missing<H, Fut>(handler: H) -> MissingHook
where
    H: Fn(Request) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Response> + Send + 'static,
{
    let record = record_of::<H>();
    let boxed: BoxedHandler = Box::new(move |req| Box::pin(handler(req)));
    MissingHook {
        handler: Arc::new(boxed),
        record,
    }
}

/// The route a binder resolves a value for: its method, its pattern, its
/// name and its parameters.
#[derive(Debug, Clone)]
pub struct MatchedRoute {
    method: Method,
    pattern: String,
    params: HashMap<String, String>,
}

impl MatchedRoute {
    /// The request's method.
    pub fn method(&self) -> &Method {
        &self.method
    }

    /// The route's pattern as it was written, `/posts/{post:slug}`.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// The route's name, if it has one.
    pub fn name(&self) -> Option<String> {
        crate::routing::route_name_for_pattern(&self.pattern)
    }

    /// The raw value of the parameter `name`.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.get(name).map(String::as_str)
    }
}

/// A binder's lookup: the value, the route, and whether the route binds
/// soft-deleted rows (`with_trashed()`).
type BinderFn =
    dyn Fn(String, MatchedRoute, bool) -> RouteLookup<'static, Option<BoundChild>> + Send + Sync;

/// A router-wide binder for one parameter name: `bind()` or `model()`.
#[derive(Clone)]
pub(crate) struct Binder {
    resolve: Arc<BinderFn>,
    type_id: TypeId,
    type_name: &'static str,
}

impl Binder {
    /// `Router::bind`: `resolver` turns the value and the route into an
    /// `R`, or nothing.
    pub(crate) fn bind<R, F, Fut>(resolver: F) -> Self
    where
        R: Send + Sync + 'static,
        F: Fn(String, MatchedRoute) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<R>, FrameworkError>> + Send + 'static,
    {
        let resolver = Arc::new(resolver);
        Self {
            resolve: Arc::new(move |value, route, _trashed| {
                let found = resolver(value, route);
                Box::pin(async move { Ok(found.await?.map(BoundChild::new)) })
            }),
            type_id: TypeId::of::<R>(),
            type_name: std::any::type_name::<R>(),
        }
    }

    /// `Router::model`: `M` by its route key, else what `fallback` makes of
    /// the value. On a route with `with_trashed()` it looks the value up
    /// through the soft-deletable lookup, as Laravel's `Route::model` does
    /// (BIND-008).
    pub(crate) fn model<M, F, Fut>(fallback: F) -> Self
    where
        M: RouteBinding,
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<M, FrameworkError>> + Send + 'static,
    {
        let fallback = Arc::new(fallback);
        let info = M::route_binding_info();
        Self {
            resolve: Arc::new(move |value, _route, trashed| {
                let fallback = fallback.clone();
                Box::pin(async move {
                    let found = if trashed {
                        M::resolve_soft_deletable_route_binding(&value, None).await?
                    } else {
                        M::resolve_route_binding(&value, None).await?
                    };
                    match found {
                        Some(found) => Ok(Some(BoundChild::new(found))),
                        None => Ok(Some(BoundChild::new(fallback(value).await?))),
                    }
                })
            }),
            type_id: info.type_id(),
            type_name: std::any::type_name::<M>(),
        }
    }
}

/// The parameter name a binder is keyed by: `-` read as `_`.
pub(crate) fn binder_key(name: &str) -> String {
    name.replace('-', "_")
}

// ---------------------------------------------------------------------------
// Route patterns
// ---------------------------------------------------------------------------

/// One `{...}` placeholder of a route pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Placeholder {
    /// The parameter's name.
    pub(crate) name: String,
    /// The binding field of `{name:field}`.
    pub(crate) field: Option<String>,
    /// `{name?}`.
    pub(crate) optional: bool,
}

/// Split the text between `{` and `}` into its name and binding field:
/// `post:slug?` is `post`, `slug`, optional. A catch-all, `*rest`, is
/// named `rest` and has no field.
pub(crate) fn split_placeholder(key: &str) -> (&str, Option<&str>, bool) {
    let (key, optional) = match key.strip_suffix('?') {
        Some(key) => (key, true),
        None => (key, false),
    };
    if let Some(rest) = key.strip_prefix('*') {
        return (rest, None, optional);
    }
    match key.split_once(':') {
        Some((name, field)) => (name, Some(field), optional),
        None => (key, None, optional),
    }
}

/// The placeholders of `pattern`, in path order.
pub(crate) fn placeholders(pattern: &str) -> Vec<Placeholder> {
    let mut found = Vec::new();
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('}') else {
            break;
        };
        let (name, field, optional) = split_placeholder(&rest[..close]);
        found.push(Placeholder {
            name: name.to_owned(),
            field: field.map(str::to_owned),
            optional,
        });
        rest = &rest[close + 1..];
    }
    found
}

/// `pattern` without its binding fields, for the matcher:
/// `/posts/{post:slug}` is `/posts/{post}`, `{post:slug?}` is `{post?}`.
pub(crate) fn without_binding_fields(pattern: &str) -> String {
    if !pattern.contains(':') {
        return pattern.to_owned();
    }
    let mut out = String::with_capacity(pattern.len());
    let mut rest = pattern;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..=open]);
        rest = &rest[open + 1..];
        let Some(close) = rest.find('}') else {
            out.push_str(rest);
            return out;
        };
        let key = &rest[..close];
        if key.starts_with('*') {
            out.push_str(key);
        } else {
            let (name, _field, optional) = split_placeholder(key);
            out.push_str(name);
            if optional {
                out.push('?');
            }
        }
        out.push('}');
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// The router's binding table, the checks and the plans
// ---------------------------------------------------------------------------

/// What one registered route carries for binding.
#[derive(Clone, Default)]
struct RouteEntry {
    record: Option<&'static HandlerRecord>,
    options: RouteBindingOptions,
}

/// The binding state of a [`Router`](crate::routing::Router): the record
/// and settings of every route, the router-wide binders, and, once checked,
/// the plan of every route.
#[derive(Default)]
pub(crate) struct RouterBindings {
    routes: HashMap<(Method, String), RouteEntry>,
    fallback: Option<&'static HandlerRecord>,
    binders: HashMap<String, Binder>,
    plans: OnceLock<Plans>,
}

impl RouterBindings {
    /// Record the handler of the route `(method, pattern)`.
    pub(crate) fn note_route(
        &mut self,
        method: Method,
        pattern: &str,
        record: Option<&'static HandlerRecord>,
    ) {
        self.reset();
        self.routes
            .entry((method, pattern.to_owned()))
            .or_default()
            .record = record;
    }

    /// Record the fallback route's handler.
    pub(crate) fn note_fallback(&mut self, record: Option<&'static HandlerRecord>) {
        self.reset();
        self.fallback = record;
    }

    /// The settings of the route `(method, pattern)`, to change them.
    pub(crate) fn options_mut(
        &mut self,
        method: Method,
        pattern: &str,
    ) -> &mut RouteBindingOptions {
        self.reset();
        &mut self
            .routes
            .entry((method, pattern.to_owned()))
            .or_default()
            .options
    }

    /// Bind the parameter `name` with `binder` on every route.
    pub(crate) fn add_binder(&mut self, name: &str, binder: Binder) {
        self.reset();
        self.binders.insert(binder_key(name), binder);
    }

    /// Forget the plans: the routes changed after they were checked.
    fn reset(&mut self) {
        self.plans = OnceLock::new();
    }

    /// Check every recorded route and build its plan, once.
    pub(crate) fn prepare(&self) -> Result<(), FrameworkError> {
        match self.plans.get_or_init(|| self.build_plans()) {
            Ok(_) => Ok(()),
            Err(error) => Err(error.clone()),
        }
    }

    /// What the route `(method, pattern)` binds before its handler runs, or
    /// leaves to an unrecorded handler. `Ok(None)` for a recorded handler
    /// that binds nothing.
    pub(crate) fn plan(
        &self,
        method: &Method,
        pattern: &str,
    ) -> Result<Option<RouteBinds>, FrameworkError> {
        match self.plans.get_or_init(|| self.build_plans()) {
            Ok(plans) => Ok(plans.get(&(method.clone(), pattern.to_owned())).cloned()),
            Err(error) => Err(error.clone()),
        }
    }

    fn build_plans(&self) -> Plans {
        let mut problems = Vec::new();
        let mut plans = HashMap::new();
        // The binders every route without a record plans against, shared.
        let mut shared_binders: Option<Arc<HashMap<String, Binder>>> = None;
        if let Some(record) = self.fallback {
            let route = "the fallback route".to_owned();
            check_handler(
                &route,
                "handler",
                record,
                &[],
                Some(&self.binders),
                &mut problems,
            );
        }
        let mut routes: Vec<_> = self.routes.iter().collect();
        routes.sort_by(|a, b| {
            (a.0.1.as_str(), a.0.0.as_str()).cmp(&(b.0.1.as_str(), b.0.0.as_str()))
        });
        for ((method, pattern), entry) in routes {
            let route = format!("{method} {pattern}");
            let path = placeholders(pattern);
            let site = PlanSite {
                route: &route,
                path: &path,
                options: &entry.options,
                binders: &self.binders,
            };
            let before = problems.len();
            // A `missing()` hook gets the route's request, so it is checked
            // against the route's path, whatever its route's handler is, and
            // its bound arguments bind under the route's settings, binders
            // included. It may read a parameter a binder covers as a path
            // value, to see the raw value that missed, so the check for a
            // binder that could never run does not apply to it.
            let missing = match entry.options.missing.as_ref() {
                None => None,
                Some(hook) => {
                    let binds = match hook.record {
                        Some(record) => {
                            let checked = problems.len();
                            check_handler(
                                &route,
                                "`missing()` handler",
                                record,
                                &path,
                                None,
                                &mut problems,
                            );
                            if problems.len() > checked {
                                None
                            } else {
                                match plan_route(
                                    &site,
                                    "`missing()` handler",
                                    &record.path(),
                                    &record.args(),
                                    None,
                                ) {
                                    Ok(plan) => Some(MissingBinds::Planned(plan.map(Arc::new))),
                                    Err(found) => {
                                        problems.extend(found);
                                        None
                                    }
                                }
                            }
                        }
                        None => Some(MissingBinds::Unplanned(Arc::new(UnrecordedRoute {
                            route: route.clone(),
                            path: path.clone(),
                            options: entry.options.clone(),
                            binders: shared_binders
                                .get_or_insert_with(|| Arc::new(self.binders.clone()))
                                .clone(),
                            missing: None,
                            plans: RwLock::new(Vec::new()),
                        }))),
                    };
                    binds.map(|binds| {
                        Arc::new(RouteMissing {
                            handler: hook.handler.clone(),
                            binds,
                        })
                    })
                }
            };
            let Some(record) = entry.record else {
                // A closure or a generic handler: a generic one, or a
                // recorded one the closure calls, plans its concrete
                // arguments at its first request (BIND-004).
                let binders = shared_binders
                    .get_or_insert_with(|| Arc::new(self.binders.clone()))
                    .clone();
                plans.insert(
                    (method.clone(), pattern.clone()),
                    RouteBinds::Unrecorded(Arc::new(UnrecordedRoute {
                        route,
                        path,
                        options: entry.options.clone(),
                        binders,
                        missing,
                        plans: RwLock::new(Vec::new()),
                    })),
                );
                continue;
            };
            check_handler(
                &route,
                "handler",
                record,
                &path,
                Some(&self.binders),
                &mut problems,
            );
            if problems.len() > before {
                continue;
            }
            match plan_route(&site, "handler", &record.path(), &record.args(), missing) {
                Ok(Some(plan)) => {
                    plans.insert(
                        (method.clone(), pattern.clone()),
                        RouteBinds::Planned(Arc::new(plan)),
                    );
                }
                Ok(None) => {}
                Err(found) => problems.extend(found),
            }
        }
        if problems.is_empty() {
            Ok(plans)
        } else {
            Err(FrameworkError::internal(format!(
                "route binding refused to start:\n- {}",
                problems.join("\n- ")
            )))
        }
    }
}

/// The checks that need the handler and the path only: at most one body
/// reader (BIND-015), every parameter an argument reads declared by the
/// path (BIND-013), and, given the router's `binders`, no binder for a
/// parameter an argument reads without binding (BIND-007). `role` names
/// the handler in the refusal: `handler`, or `` `missing()` handler ``.
fn check_handler(
    route: &str,
    role: &str,
    record: &HandlerRecord,
    path: &[Placeholder],
    binders: Option<&HashMap<String, Binder>>,
    problems: &mut Vec<String>,
) {
    let args = record.args();
    let handler = record.path();
    let body: Vec<&HandlerArg> = args
        .iter()
        .filter(|arg| matches!(arg.kind, HandlerArgKind::Body))
        .collect();
    if body.len() > 1 {
        problems.push(format!(
            "route `{route}`: {role} `{handler}` has {} arguments that read the request body ({}); \
             a handler may read it once, so fold them into one form request",
            body.len(),
            body.iter()
                .map(|arg| format!("`{}: {}`", arg.name, arg.type_name))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for arg in &args {
        let declared = path.iter().any(|p| p.name == arg.name);
        match &arg.kind {
            HandlerArgKind::Bound(_) | HandlerArgKind::PathValue if !declared => {
                problems.push(format!(
                    "route `{route}`: {role} `{handler}` reads the route parameter `{}` \
                     (`{}: {}`), which the route's path does not declare",
                    arg.name, arg.name, arg.type_name
                ));
            }
            HandlerArgKind::PathValue | HandlerArgKind::Body
                if declared
                    && binders
                        .is_some_and(|binders| binders.contains_key(&binder_key(arg.name))) =>
            {
                problems.push(format!(
                    "route `{route}`: a binder is registered for the parameter `{}`, but handler \
                     `{handler}` reads it as `{}: {}`, which does not implement `RouteBinding`, so \
                     the binder could never run",
                    arg.name, arg.name, arg.type_name
                ));
            }
            _ => {}
        }
    }
}

/// One binding of a route's plan, in path order.
pub(crate) struct PlannedBinding {
    arg: usize,
    param: String,
    optional: bool,
    ops: BoundArg,
    info: RouteBindingInfo,
    field: Option<String>,
    /// The planned binding (an index into the plan) that scopes this one.
    parent: Option<usize>,
    binder: Option<Binder>,
}

/// What a request on one route runs before its handler.
pub(crate) struct RoutePlan {
    bindings: Vec<PlannedBinding>,
    arg_count: usize,
    with_trashed: bool,
    missing: Option<Arc<RouteMissing>>,
}

/// How a route binds: before its recorded handler runs, or, for a handler
/// that carries no record, inside it.
#[derive(Clone)]
pub(crate) enum RouteBinds {
    /// The plan the router built before the first request, which a request
    /// runs ahead of the handler.
    Planned(Arc<RoutePlan>),
    /// The route's binding settings, for a generic handler to plan its
    /// concrete arguments against at the request.
    Unrecorded(Arc<UnrecordedRoute>),
}

/// The binding settings of a route, for a handler the startup checks did
/// not plan: the handler of a route that carries no record (a closure, or a
/// generic `#[handler]` function), a recorded handler a closure route
/// calls, or a `missing()` handler with no record. Such a handler hands its
/// arguments over at its first request on the route, its concrete bound
/// arguments bind as a recorded handler's do (BIND-004), and the plan built
/// from them is kept for the next requests.
pub(crate) struct UnrecordedRoute {
    /// `GET /users/{user}`, for a refusal.
    route: String,
    path: Vec<Placeholder>,
    options: RouteBindingOptions,
    binders: Arc<HashMap<String, Binder>>,
    /// The route's `missing()` handler; `None` for the settings a
    /// `missing()` handler itself binds by.
    missing: Option<Arc<RouteMissing>>,
    /// The plan of each handler that bound on this route, by the key its
    /// generated code passes. One handler in practice; a closure that calls
    /// two handlers gets one plan for each.
    plans: RwLock<Vec<(TypeId, UnrecordedPlan)>>,
}

impl UnrecordedRoute {
    /// The plan of the handler `key`, named `handler`, whose arguments
    /// `args` lists in declaration order: built and kept on the first call,
    /// read back after. A problem the startup checks would refuse a
    /// recorded handler for is the plan's error.
    fn plan(
        &self,
        key: TypeId,
        handler: &str,
        args: impl FnOnce() -> Vec<HandlerArg>,
    ) -> UnrecordedPlan {
        let known = |plans: &[(TypeId, UnrecordedPlan)]| {
            plans
                .iter()
                .find(|(planned, _)| *planned == key)
                .map(|(_, plan)| plan.clone())
        };
        let read = self
            .plans
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(plan) = known(&read) {
            return plan;
        }
        drop(read);
        let site = PlanSite {
            route: &self.route,
            path: &self.path,
            options: &self.options,
            binders: &self.binders,
        };
        let plan = plan_route(&site, "handler", handler, &args(), self.missing.clone())
            .map(|plan| plan.map(Arc::new))
            .map_err(|problems| {
                FrameworkError::internal(format!(
                    "route binding refused the request:\n- {}",
                    problems.join("\n- ")
                ))
            });
        let mut write = self
            .plans
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // A request that planned it meanwhile keeps its plan.
        if let Some(plan) = known(&write) {
            return plan;
        }
        write.push((key, plan.clone()));
        plan
    }
}

/// The route a plan is checked and built against: its name for a refusal
/// (`GET /users/{user}`), its path, its settings and the router's binders.
struct PlanSite<'a> {
    route: &'a str,
    path: &'a [Placeholder],
    options: &'a RouteBindingOptions,
    binders: &'a HashMap<String, Binder>,
}

/// The checks that need the models (BIND-004, BIND-006, BIND-007), and the
/// plan when they pass, for the handler named `handler` whose arguments are
/// `args`, in declaration order, answering a miss with `missing`. `role`
/// names the handler in a refusal, as [`check_handler`]'s does. `Ok(None)`
/// for a route that binds nothing.
fn plan_route(
    site: &PlanSite<'_>,
    role: &str,
    handler: &str,
    args: &[HandlerArg],
    missing: Option<Arc<RouteMissing>>,
) -> Result<Option<RoutePlan>, Vec<String>> {
    let PlanSite {
        route,
        path,
        options,
        binders,
    } = *site;
    let mut problems = Vec::new();
    // The bound arguments, in path order.
    let mut bound: Vec<(usize, usize, BoundArg)> = args
        .iter()
        .enumerate()
        .filter_map(|(index, arg)| match &arg.kind {
            HandlerArgKind::Bound(ops) => path
                .iter()
                .position(|p| p.name == arg.name)
                .map(|position| (position, index, *ops)),
            _ => None,
        })
        .collect();
    if bound.is_empty() {
        return Ok(None);
    }
    bound.sort_by_key(|(position, _, _)| *position);

    let mut bindings: Vec<PlannedBinding> = Vec::with_capacity(bound.len());
    for (position, index, ops) in bound {
        let arg = &args[index];
        let placeholder = &path[position];
        let info = ops.info();
        let binder = binders.get(&binder_key(&placeholder.name)).cloned();

        if let Some(binder) = &binder
            && binder.type_id != info.type_id()
        {
            problems.push(format!(
                "route `{route}`: the binder for `{}` returns `{}`, but {role} `{handler}` binds \
                 it as `{}: {}`",
                placeholder.name, binder.type_name, arg.name, arg.type_name
            ));
        }

        // BIND-004: a binding field must be a column the type binds by.
        if binder.is_none()
            && let Some(field) = &placeholder.field
            && let Some(columns) = info.columns()
        {
            match columns.iter().find(|column| column.name() == field) {
                None => problems.push(format!(
                    "route `{route}`: the binding field `{field}` of parameter `{}` is not a column \
                     `{}` binds by (it binds by: {})",
                    placeholder.name,
                    info.name(),
                    columns
                        .iter()
                        .filter(|column| column.parses())
                        .map(|column| column.name())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                Some(column) if !column.parses() => problems.push(format!(
                    "route `{route}`: the binding field `{field}` of parameter `{}` names a column of \
                     `{}` whose type cannot be parsed from a path segment",
                    placeholder.name,
                    info.name()
                )),
                Some(_) => {}
            }
        }

        // BIND-006: the parent is the parameter right before this one, when
        // an argument binds it.
        let parent = position
            .checked_sub(1)
            .and_then(|before| bindings.iter().position(|b| b.param == path[before].name));
        let scoped = binder.is_none()
            && options.scoped != Some(false)
            && (placeholder.field.is_some() || options.scoped == Some(true));
        let parent = if scoped { parent } else { None };
        if let Some(parent_index) = parent {
            let parent_binding = &bindings[parent_index];
            check_child_relation(
                route,
                &parent_binding.info,
                &placeholder.name,
                &info,
                &mut problems,
            );
        }

        bindings.push(PlannedBinding {
            arg: index,
            param: placeholder.name.clone(),
            optional: arg.optional,
            ops,
            info,
            field: if binder.is_some() {
                None
            } else {
                placeholder.field.clone()
            },
            parent,
            binder,
        });
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    Ok(Some(RoutePlan {
        bindings,
        arg_count: args.len(),
        with_trashed: options.with_trashed,
        missing,
    }))
}

/// BIND-006: a scoped child is found through the parent's relation named
/// by the child parameter in the plural, which must return the child's
/// type. A parent that resolves its own children is not checked.
fn check_child_relation(
    route: &str,
    parent: &RouteBindingInfo,
    child: &str,
    child_info: &RouteBindingInfo,
    problems: &mut Vec<String>,
) {
    let relation = crate::database::route_binding::child_relation_name(child);
    match parent.children() {
        ChildBindings::Custom => {}
        ChildBindings::None => problems.push(format!(
            "route `{route}`: the child parameter `{child}` is scoped to its parent `{}`, which \
             declares no relation `{relation}` to find it through",
            parent.name()
        )),
        ChildBindings::Relations => {
            let parent_type = parent.type_id();
            let entry = crate::eloquent::relations::relations()
                .find(|entry| (entry.parent_type)() == parent_type && entry.name == relation);
            match entry {
                None => problems.push(format!(
                    "route `{route}`: the child parameter `{child}` is scoped to its parent `{}`, \
                     which declares no relation `{relation}`",
                    parent.name()
                )),
                Some(entry) if entry.kind == crate::eloquent::RelationKind::MorphTo => {
                    problems.push(format!(
                        "route `{route}`: the relation `{relation}` of `{}` is a `MorphTo`, whose \
                         child type varies by row, so it cannot scope the child parameter `{child}`",
                        parent.name()
                    ))
                }
                Some(entry) if (entry.target_type)() != child_info.type_id() => {
                    problems.push(format!(
                        "route `{route}`: the relation `{relation}` of `{}` returns `{}`, but the \
                         child parameter `{child}` binds `{}`",
                        parent.name(),
                        entry.target_type_name,
                        child_info.name()
                    ))
                }
                Some(_) => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The lookups a request runs
// ---------------------------------------------------------------------------

/// The handler of a route with a plan: run the plan, then the handler.
pub(crate) fn planned_handler(
    plan: Arc<RoutePlan>,
    handler: Arc<BoxedHandler>,
) -> Arc<BoxedHandler> {
    let boxed: BoxedHandler = Box::new(move |request| {
        let plan = plan.clone();
        let handler = handler.clone();
        Box::pin(async move {
            match bind_request(&plan, request).await {
                Ok(request) => handler(request).await,
                Err(response) => response,
            }
        })
    });
    Arc::new(boxed)
}

/// What a binding that found nothing answers: the route's `missing()`
/// response, or a 404 naming the type. An enum's miss never reaches
/// `missing()` (BIND-010).
async fn miss(plan: &RoutePlan, binding: &PlannedBinding, request: Request) -> Response {
    if let Some(missing) = &plan.missing
        && !binding.info.is_unit_enum()
    {
        return answer_missing(missing, request).await;
    }
    Err(HttpResponse::from(FrameworkError::model_not_found(
        binding.info.name(),
    )))
}

/// Run `plan` on `request`: every binding in path order, each child under
/// its parent when scoped. The request comes back with the bound values;
/// a binding that finds nothing answers instead.
async fn bind_request(plan: &RoutePlan, mut request: Request) -> Result<Request, Response> {
    let route = MatchedRoute {
        method: request.method().clone(),
        pattern: request.route_pattern().unwrap_or_default().to_owned(),
        params: request.params().clone(),
    };
    match bind_values(plan, &route).await {
        Ok(values) => {
            request.set_route_bindings(RouteBindingState::Bound(values));
            Ok(request)
        }
        Err(Unbound::Miss(index)) => Err(miss(plan, &plan.bindings[index], request).await),
        Err(Unbound::Failed(error)) => Err(Err(HttpResponse::from(error))),
    }
}

/// Why a plan bound nothing for its handler.
enum Unbound {
    /// The planned binding at this index found no value, no row, no row
    /// its parent owns, or a value that does not parse.
    Miss(usize),
    /// A lookup failed.
    Failed(FrameworkError),
}

/// The values `plan` binds on `route`, one slot per handler argument,
/// looked up in path order, each child under its parent when scoped.
async fn bind_values(
    plan: &RoutePlan,
    route: &MatchedRoute,
) -> Result<Vec<Option<Erased>>, Unbound> {
    let mut values: Vec<Option<Erased>> = (0..plan.arg_count).map(|_| None).collect();
    for (index, binding) in plan.bindings.iter().enumerate() {
        let Some(value) = route.params.get(&binding.param) else {
            if binding.optional {
                continue;
            }
            return Err(Unbound::Miss(index));
        };
        match lookup(plan, binding, value, route, &values).await {
            Ok(Some(found)) => values[binding.arg] = Some(found),
            Ok(None) => return Err(Unbound::Miss(index)),
            Err(error) => return Err(Unbound::Failed(error)),
        }
    }
    Ok(values)
}

/// Look one binding up: through its binder, under its parent when scoped,
/// or through its type.
async fn lookup(
    plan: &RoutePlan,
    binding: &PlannedBinding,
    value: &str,
    route: &MatchedRoute,
    values: &[Option<Erased>],
) -> Result<Option<Erased>, FrameworkError> {
    if let Some(binder) = &binding.binder {
        return match (binder.resolve)(value.to_owned(), route.clone(), plan.with_trashed).await? {
            Some(found) => adopt(binding, found).map(Some),
            None => Ok(None),
        };
    }
    let field = binding.field.as_deref();
    if let Some(parent_index) = binding.parent {
        let parent = &plan.bindings[parent_index];
        if let Some(parent_value) = values[parent.arg].as_deref() {
            return match (parent.ops.child)(
                parent_value,
                &binding.param,
                value,
                field,
                plan.with_trashed,
            )
            .await?
            {
                Some(found) => adopt(binding, found).map(Some),
                None => Ok(None),
            };
        }
    }
    (binding.ops.resolve)(value, field, plan.with_trashed).await
}

/// A value a binder or a relation found, as the argument's type.
fn adopt(binding: &PlannedBinding, found: BoundChild) -> Result<Erased, FrameworkError> {
    (binding.ops.adopt)(found).map_err(|found| {
        FrameworkError::internal(format!(
            "the route parameter `{}` was resolved to a `{}`, which is not what its handler argument \
             binds (`{}`)",
            binding.param,
            found.type_name(),
            binding.info.name()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn bind_004_an_unrecorded_route_plans_each_handler_once() {
        struct First;
        struct Second;
        let route = UnrecordedRoute {
            route: "GET /users/{id}".to_owned(),
            path: placeholders("/users/{id}"),
            options: RouteBindingOptions::default(),
            binders: Arc::new(HashMap::new()),
            missing: None,
            plans: RwLock::new(Vec::new()),
        };
        let described = AtomicUsize::new(0);
        let args = || {
            described.fetch_add(1, Ordering::SeqCst);
            vec![HandlerArg::path_value("id", "i64", false)]
        };
        for _ in 0..3 {
            assert!(matches!(
                route.plan(TypeId::of::<First>(), "first", args),
                Ok(None)
            ));
        }
        assert_eq!(
            described.load(Ordering::SeqCst),
            1,
            "a handler describes its arguments at its first request only"
        );
        // A second handler on the same route gets its own plan, kept too.
        for _ in 0..2 {
            assert!(matches!(
                route.plan(TypeId::of::<Second>(), "second", args),
                Ok(None)
            ));
        }
        assert_eq!(described.load(Ordering::SeqCst), 2);
        assert!(route.plan(TypeId::of::<First>(), "first", args).is_ok());
        assert_eq!(described.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn placeholders_read_names_fields_and_optional_markers() {
        let found = placeholders("/users/{user}/posts/{post:slug?}/{*rest}");
        assert_eq!(
            found,
            vec![
                Placeholder {
                    name: "user".into(),
                    field: None,
                    optional: false
                },
                Placeholder {
                    name: "post".into(),
                    field: Some("slug".into()),
                    optional: true
                },
                Placeholder {
                    name: "rest".into(),
                    field: None,
                    optional: false
                },
            ]
        );
    }

    #[test]
    fn binding_fields_are_left_out_for_the_matcher() {
        assert_eq!(
            without_binding_fields("/posts/{post:slug}"),
            "/posts/{post}"
        );
        assert_eq!(
            without_binding_fields("/posts/{post:slug?}"),
            "/posts/{post?}"
        );
        assert_eq!(without_binding_fields("/files/{*rest}"), "/files/{*rest}");
        assert_eq!(
            without_binding_fields("/files/note:draft"),
            "/files/note:draft"
        );
    }

    #[test]
    fn binder_keys_read_a_dash_as_an_underscore() {
        assert_eq!(binder_key("user-id"), "user_id");
    }
}
