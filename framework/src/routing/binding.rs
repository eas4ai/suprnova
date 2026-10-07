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
//! closure binds nothing itself. Every matched request carries its route's
//! binding settings, and a handler the route did not plan (a generic one, or
//! a recorded one that a closure route, middleware or another handler calls
//! with the request) describes its arguments at the request: its concrete
//! bound arguments get the plan a recorded handler's would, built at its
//! first request on the route and kept for the next ones, so they bind in
//! path order with the route's binding fields, scoping, binders,
//! `with_trashed()` and `missing()`. A binding field, scoped child or binder
//! the startup checks would refuse, a binder for a parameter the handler
//! reads without binding included, answers that request with the refusal
//! instead of binding. A generic handler's generic arguments read the body.
//!
//! A route's `missing()` handler binds its own arguments under the same
//! settings when it runs: a recorded one through a plan the startup checks
//! build, one with no record through the route's settings. Its own binding
//! that finds nothing answers 404.
//!
//! Any handler planned against a route scopes a child through the parent
//! the route's handler binds, binding that parent itself when it does not
//! take it, so it never gets a row the route refused. The router knows how
//! the route's handler binds each parameter from its record, or, for a
//! generic `#[handler]`, from the [`GenericHandlerRecord`] it finds by the
//! function's path. A closure, or a function that is not a `#[handler]`,
//! binds no parent, so nothing scopes the child. When the router cannot see
//! how the route's handler binds the parent (a generic handler it cannot
//! find, one that binds the parent with a type naming its type parameter,
//! or a function pointer), the request answers the refusal instead, and the
//! child is never looked up unscoped.

use crate::database::route_binding::{
    BoundChild, ChildBindings, RouteBinding, RouteBindingInfo, RouteLookup,
};
use crate::error::FrameworkError;
use crate::http::{FromParam, FromRequest, HttpResponse, Request, Response};
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

/// What `#[handler]` records about a generic handler function, so a route
/// whose own handler it is can scope a child for the other handlers planned
/// against it (BIND-004, BIND-006).
///
/// A generic function has no single type to key a [`HandlerRecord`] by, so
/// the record is keyed by the function's path, which the router reads from
/// the instantiation's type name with the type arguments left out. It lists
/// the arguments whose type does not depend on the instantiation; a
/// generic argument reads the body, and one bound with an
/// instantiation-dependent type (`RouteParam<T>`) is named in `unresolved`.
/// A generic handler is still exempt from the startup checks.
pub struct GenericHandlerRecord {
    path: fn() -> String,
    name: &'static str,
    module: &'static str,
    args: fn() -> Vec<HandlerArg>,
    unresolved: &'static [&'static str],
}

impl GenericHandlerRecord {
    /// The record of the generic handler at `path` (the module path and the
    /// function's name, or its `impl` type's name and the function's),
    /// named `name` in `module`, with the arguments `args` lists and the
    /// parameters `unresolved` names. Built by `#[handler]`.
    #[doc(hidden)]
    pub const fn new(
        path: fn() -> String,
        name: &'static str,
        module: &'static str,
        args: fn() -> Vec<HandlerArg>,
        unresolved: &'static [&'static str],
    ) -> Self {
        Self {
            path,
            name,
            module,
            args,
            unresolved,
        }
    }

    /// The handler's path, `app::controllers::posts::show`, for errors.
    pub fn path(&self) -> String {
        format!("{}::{}", self.module, self.name)
    }
}

impl std::fmt::Debug for GenericHandlerRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenericHandlerRecord")
            .field("handler", &self.path())
            .finish_non_exhaustive()
    }
}

inventory::collect!(GenericHandlerRecord);

/// What the router knows about a route's handler when it registers it: the
/// record of a `#[handler]` function, or, for a handler with none, what
/// binds the route's parameters.
#[derive(Clone, Copy)]
pub(crate) struct HandlerRef {
    record: Option<&'static HandlerRecord>,
    unrecorded: UnrecordedHandler,
}

impl HandlerRef {
    /// A handler that binds nothing: a resource controller's action.
    pub(crate) const BINDS_NOTHING: Self = Self {
        record: None,
        unrecorded: UnrecordedHandler::BindsNothing,
    };

    /// The handler's `#[handler]` record, when it has one.
    pub(crate) fn record(&self) -> Option<&'static HandlerRecord> {
        self.record
    }
}

impl Default for HandlerRef {
    fn default() -> Self {
        Self::BINDS_NOTHING
    }
}

/// What binds a route's parameters when its handler carries no record.
#[derive(Clone, Copy)]
enum UnrecordedHandler {
    /// A closure, or a function that is not a `#[handler]`: it binds
    /// nothing.
    BindsNothing,
    /// A generic `#[handler]` the router found by its path.
    Generic(&'static GenericHandlerRecord),
    /// A handler the router cannot see into, named by its type: a generic
    /// `#[handler]` it did not find, or a function pointer. What it binds
    /// is unknown.
    Unknown(&'static str),
}

/// What the router knows about the handler `H`.
pub(crate) fn handler_ref<H: 'static>() -> HandlerRef {
    match record_of::<H>() {
        Some(record) => HandlerRef {
            record: Some(record),
            unrecorded: UnrecordedHandler::BindsNothing,
        },
        None => HandlerRef {
            record: None,
            unrecorded: unrecorded_handler(std::any::type_name::<H>()),
        },
    }
}

/// What binds the parameters of a handler with no record, from its type's
/// name: a closure's ends `{{closure}}`, a plain function's is its path, and
/// a generic function's is its path with its type arguments after it.
fn unrecorded_handler(name: &'static str) -> UnrecordedHandler {
    static RECORDS: OnceLock<HashMap<String, &'static GenericHandlerRecord>> = OnceLock::new();
    let is_path = |text: &str| {
        !text.is_empty()
            && text
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == ':')
    };
    if name.ends_with("{{closure}}") || is_path(name) {
        return UnrecordedHandler::BindsNothing;
    }
    let found = generic_base(name)
        .filter(|base| is_path(base))
        .and_then(|base| {
            RECORDS
                .get_or_init(|| {
                    inventory::iter::<GenericHandlerRecord>()
                        .map(|record| ((record.path)(), record))
                        .collect()
                })
                .get(base)
                .copied()
        });
    match found {
        Some(record) => UnrecordedHandler::Generic(record),
        None => UnrecordedHandler::Unknown(name),
    }
}

/// `name` without the type arguments it ends with: `app::show` for
/// `app::show<app::Form, 3>`. `None` when it does not end with them.
fn generic_base(name: &str) -> Option<&str> {
    let mut depth = 0usize;
    for (index, c) in name.char_indices().rev() {
        match c {
            '>' => depth += 1,
            '<' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(&name[..index]);
                }
            }
            _ if depth == 0 => return None,
            _ => {}
        }
    }
    None
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
}

impl HandlerInput {
    /// Take the request apart for the handler's arguments.
    pub fn new(mut request: Request) -> Self {
        let bound = request.take_route_bindings();
        let params = request.params().clone();
        Self {
            request: Some(request),
            params,
            bound,
        }
    }

    /// Whether [`Self::__bind_unplanned`] binds this handler's arguments:
    /// the route bound no values for it, and the request carries the
    /// route's settings. That is a generic handler, or one a closure route,
    /// middleware, another handler or a `missing()` handler calls. `handler`
    /// is the handler function's type, `None` for a generic one: the
    /// recorded handler the startup checks found binds nothing on the route
    /// does not plan again.
    #[inline]
    pub fn __binds_unplanned(&self, handler: Option<TypeId>) -> bool {
        self.bound.is_none()
            && self
                .request
                .as_ref()
                .and_then(Request::route_settings)
                .is_some_and(|route| route.idle.is_none() || route.idle != handler)
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

    /// The value the route bound for argument `index`. A handler called
    /// outside a matched route, whose request carries no route settings,
    /// looks the parameter `name` up by its route key instead.
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

    /// Bind the concrete arguments of a handler the route did not plan, as
    /// the route binds a recorded handler's (BIND-004): a generic handler,
    /// a recorded one a closure route or another handler calls, or a
    /// `missing()` handler with no record. `args` describes every argument
    /// in declaration order, and the route plans them once, keyed by `key`:
    /// a type the generated code makes unique to the handler, and the name
    /// of the handler's instantiation, which tells a generic handler's
    /// instantiations apart.
    ///
    /// `Ok(None)` once they are bound, or when the route handed no
    /// settings. A binding that finds nothing answers `Ok(Some(response))`
    /// with the route's `missing()` response or the 404 when
    /// `answers_missing`, which the generated code sets when the handler
    /// returns [`Response`]. Any other handler fails with the 404's error,
    /// and the route's `missing()` response is kept for the route to
    /// answer with when that error is its result. A failed lookup or a
    /// refusal of the route's checks is the error.
    ///
    /// # Errors
    ///
    /// The 404 of a binding that finds nothing, for a handler that cannot
    /// return the `missing()` response; a lookup's error; or the refusal of
    /// a binding field, scoped child or binder the startup checks would
    /// refuse.
    pub async fn __bind_unplanned(
        &mut self,
        key: (TypeId, &'static str),
        handler: &'static str,
        args: impl FnOnce() -> Vec<HandlerArg>,
        answers_missing: bool,
    ) -> Result<Option<Response>, FrameworkError> {
        let Some(request) = self.request.as_ref() else {
            return Ok(None);
        };
        let Some(route) = request.route_settings().cloned() else {
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
                let error = FrameworkError::model_not_found(binding.info.name());
                let Some(request) = self.request.take() else {
                    return Err(error);
                };
                if answers_missing {
                    return Ok(Some(miss(&plan, binding, request).await));
                }
                if plan.missing.is_some()
                    && !binding.info.is_unit_enum()
                    && let Some(slot) = request.missing_answer().cloned()
                {
                    let answer = miss(&plan, binding, request).await;
                    slot.keep(&error, answer);
                }
                Err(error)
            }
            Err(Unbound::Failed(error)) => Err(error),
        }
    }

    /// The checks a handler the route did not plan meets at the request when
    /// none of its arguments may bind (BIND-004): the same plan, keyed and
    /// kept as [`Self::__bind_unplanned`] keeps it, whose refusal of a
    /// binder that could never run is the error.
    ///
    /// # Errors
    ///
    /// The refusal of a binder, binding field or scoped child the startup
    /// checks would refuse.
    pub fn __check_unplanned(
        &self,
        key: (TypeId, &'static str),
        handler: &'static str,
        args: impl FnOnce() -> Vec<HandlerArg>,
    ) -> Result<(), FrameworkError> {
        match self.request.as_ref().and_then(Request::route_settings) {
            Some(route) => route.plan(key, handler, args).map(|_| ()),
            None => Ok(()),
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
    /// The plan the startup checks built for a recorded handler against the
    /// route; `None` when it binds nothing or carries no record.
    plan: Option<Arc<RoutePlan>>,
    /// The route's settings without its `missing()` handler, for a handler
    /// with no record and for any handler it calls with the request.
    settings: Arc<RouteSettings>,
}

/// Run the `missing()` handler on `request`, its own bound arguments bound
/// first. Its own binding that finds nothing answers 404: the handler is
/// never called for its own miss.
async fn answer_missing(missing: &RouteMissing, mut request: Request) -> Response {
    if let Some(plan) = &missing.plan {
        let route = MatchedRoute {
            method: request.method().clone(),
            pattern: request.route_pattern().unwrap_or_default().to_owned(),
            params: request.params().clone(),
        };
        match bind_values(plan, &route).await {
            Ok(values) => request.set_route_bindings(values),
            Err(Unbound::Miss(index)) => {
                return Err(HttpResponse::from(FrameworkError::model_not_found(
                    plan.bindings[index].info.name(),
                )));
            }
            Err(Unbound::Failed(error)) => return Err(HttpResponse::from(error)),
        }
    }
    request.set_route_settings(missing.settings.clone());
    (missing.handler)(request).await
}

/// The route's `missing()` response, kept for a route on which a
/// `#[handler]` that cannot return it (its return type is not
/// [`Response`]) ran: that handler fails with the 404's error. When the
/// route's handler, or a handler it called, kept it, the route answers with
/// it at the route handler's boundary, inside the middleware chain, as it
/// answers its own `missing()` response. When a handler route middleware
/// called kept it, the server answers with it after the chain, when the
/// 404 came back out of the chain.
#[derive(Default)]
pub(crate) struct MissingAnswer(std::sync::Mutex<Option<(String, Response)>>);

impl MissingAnswer {
    /// Keep `answer`, the `missing()` response, for the miss `error`.
    fn keep(&self, error: &FrameworkError, answer: Response) {
        let mut slot = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = Some((error.to_string(), answer));
    }

    /// Take what is kept, leaving the slot empty.
    fn take(&self) -> Option<(String, Response)> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    /// Put back what [`Self::take`] took.
    fn restore(&self, kept: Option<(String, Response)>) {
        if kept.is_some() {
            *self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = kept;
        }
    }

    /// The route's final response, after the middleware chain: the
    /// `missing()` response a handler route middleware called kept, when
    /// `response` is that miss's own 404, else `response`.
    pub(crate) fn answer(&self, response: HttpResponse) -> HttpResponse {
        match self.take() {
            Some((error, answer)) if is_the_miss(&response, &error) => {
                answer.unwrap_or_else(|answer| answer)
            }
            _ => response,
        }
    }
}

/// Whether `response` is the 404 of the miss whose error reads `error`.
fn is_the_miss(response: &HttpResponse, error: &str) -> bool {
    response.status_code() == 404
        && response
            .error_report()
            .and_then(|report| report.chain().first())
            .is_some_and(|first| first == error)
}

/// The handler of a route with a `missing()` handler: when the route's
/// handler, or a handler it called, kept the `missing()` response and its
/// result is that miss's own 404, the route answers with the kept response
/// here, inside the middleware chain, so the response-side middleware sees
/// it as it sees the route's own `missing()` response. A response a handler
/// route middleware called kept is left for the server.
pub(crate) fn missing_answer_handler(handler: Arc<BoxedHandler>) -> Arc<BoxedHandler> {
    let boxed: BoxedHandler = Box::new(move |request| {
        let handler = handler.clone();
        Box::pin(async move {
            let Some(slot) = request.missing_answer().cloned() else {
                return handler(request).await;
            };
            let earlier = slot.take();
            let result = handler(request).await;
            let result = match slot.take() {
                Some((error, answer)) => {
                    let (Ok(response) | Err(response)) = &result;
                    if is_the_miss(response, &error) {
                        answer
                    } else {
                        result
                    }
                }
                None => result,
            };
            slot.restore(earlier);
            result
        })
    });
    Arc::new(boxed)
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
    handler: HandlerRef,
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
    pub(crate) fn note_route(&mut self, method: Method, pattern: &str, handler: HandlerRef) {
        self.reset();
        self.routes
            .entry((method, pattern.to_owned()))
            .or_default()
            .handler = handler;
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

    /// How the route `(method, pattern)` binds: the plan its recorded
    /// handler runs, and the settings it hands any handler it did not plan.
    /// `Ok(None)` for a route the router never registered.
    pub(crate) fn plan(
        &self,
        method: &Method,
        pattern: &str,
    ) -> Result<Option<&RouteBinds>, FrameworkError> {
        match self.plans.get_or_init(|| self.build_plans()) {
            Ok(plans) => Ok(plans.get(&(method.clone(), pattern.to_owned()))),
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
            // How the route's own handler binds each parameter.
            let parents = Parents::of(&entry.handler);
            let site = PlanSite {
                route: &route,
                path: &path,
                options: &entry.options,
                binders: &self.binders,
                parents: &parents,
            };
            let before = problems.len();
            // A `missing()` hook gets the route's request, so it is checked
            // against the route's path, whatever its route's handler is, and
            // its bound arguments bind under the route's settings, binders
            // included. It may read a parameter a binder covers as a path
            // value, to see the raw value that missed, so the check for a
            // binder that could never run does not apply to it.
            let binders = shared_binders
                .get_or_insert_with(|| Arc::new(self.binders.clone()))
                .clone();
            // The settings a handler the route did not plan binds by: `role`
            // names it in a refusal, `idle` is the recorded handler the
            // startup checks found binds nothing here, and a `missing()`
            // handler's settings carry no `missing()` and no binder check.
            let settings = |role: &'static str,
                            checks_binders: bool,
                            idle: Option<TypeId>,
                            missing: Option<Arc<RouteMissing>>| {
                Arc::new(RouteSettings {
                    role,
                    route: route.clone(),
                    path: path.clone(),
                    options: entry.options.clone(),
                    binders: binders.clone(),
                    parents: parents.clone(),
                    checks_binders,
                    idle,
                    missing,
                    plans: RwLock::new(Vec::new()),
                })
            };
            let missing = match entry.options.missing.as_ref() {
                None => None,
                Some(hook) => {
                    // On a route where the router cannot see how the handler
                    // binds some parameter, the hook plans at the request,
                    // where a child it takes without that parent answers the
                    // refusal.
                    let plan = match hook.record.filter(|_| !parents.has_unknown()) {
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
                                Err(())
                            } else {
                                plan_route(
                                    &site,
                                    "`missing()` handler",
                                    &record.path(),
                                    &record.args(),
                                    None,
                                )
                                .map_err(|found| problems.extend(found))
                            }
                        }
                        None => Ok(None),
                    };
                    plan.ok().map(|plan| {
                        let idle = match (&plan, hook.record) {
                            (None, Some(record)) if !parents.has_unknown() => {
                                Some((record.handler)())
                            }
                            _ => None,
                        };
                        Arc::new(RouteMissing {
                            handler: hook.handler.clone(),
                            plan: plan.map(Arc::new),
                            settings: settings("`missing()` handler", false, idle, None),
                        })
                    })
                }
            };
            let plan = match entry.handler.record() {
                // A closure or a generic handler: a generic one, or a
                // recorded one the closure calls, plans its concrete
                // arguments at its first request (BIND-004).
                None => None,
                Some(record) => {
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
                    match plan_route(
                        &site,
                        "handler",
                        &record.path(),
                        &record.args(),
                        missing.clone(),
                    ) {
                        Ok(plan) => plan.map(Arc::new),
                        Err(found) => {
                            problems.extend(found);
                            continue;
                        }
                    }
                }
            };
            let idle = match (&plan, entry.handler.record()) {
                (None, Some(record)) => Some((record.handler)()),
                _ => None,
            };
            plans.insert(
                (method.clone(), pattern.clone()),
                RouteBinds {
                    plan,
                    settings: settings("handler", true, idle, missing),
                },
            );
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
        if matches!(
            arg.kind,
            HandlerArgKind::Bound(_) | HandlerArgKind::PathValue
        ) && !declared
        {
            problems.push(format!(
                "route `{route}`: {role} `{handler}` reads the route parameter `{}` \
                 (`{}: {}`), which the route's path does not declare",
                arg.name, arg.name, arg.type_name
            ));
        }
    }
    if let Some(binders) = binders {
        binder_problems(route, &handler, &args, path, binders, problems);
    }
}

/// BIND-007: a binder for a parameter that the handler `handler` reads
/// without binding (a path value, or the body) could never run.
fn binder_problems(
    route: &str,
    handler: &str,
    args: &[HandlerArg],
    path: &[Placeholder],
    binders: &HashMap<String, Binder>,
    problems: &mut Vec<String>,
) {
    for arg in args {
        let declared = path.iter().any(|p| p.name == arg.name);
        if matches!(arg.kind, HandlerArgKind::PathValue | HandlerArgKind::Body)
            && declared
            && binders.contains_key(&binder_key(arg.name))
        {
            problems.push(format!(
                "route `{route}`: a binder is registered for the parameter `{}`, but handler \
                 `{handler}` reads it as `{}: {}`, which does not implement `RouteBinding`, so \
                 the binder could never run",
                arg.name, arg.name, arg.type_name
            ));
        }
    }
}

/// One binding of a route's plan, in path order.
pub(crate) struct PlannedBinding {
    /// The slot it fills: its argument's index, or, for a parent bound only
    /// to scope a child, a slot after the handler's arguments.
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
    /// Every slot the plan fills: one per handler argument, then one per
    /// parent bound only to scope a child.
    slot_count: usize,
    with_trashed: bool,
    missing: Option<Arc<RouteMissing>>,
}

/// How a route binds: the plan its recorded handler runs before it, and
/// the settings it hands, by an `Arc` clone per request, to any handler it
/// did not plan.
pub(crate) struct RouteBinds {
    /// The plan the router built before the first request, `None` when the
    /// route's handler binds nothing or carries no record.
    plan: Option<Arc<RoutePlan>>,
    settings: Arc<RouteSettings>,
}

impl RouteBinds {
    /// The plan a request runs ahead of the route's handler.
    pub(crate) fn plan(&self) -> Option<&Arc<RoutePlan>> {
        self.plan.as_ref()
    }

    /// The route's binding settings.
    pub(crate) fn settings(&self) -> &Arc<RouteSettings> {
        &self.settings
    }
}

/// The binding settings of a route, for a handler the startup checks did
/// not plan: the handler of a route that carries no record (a closure, or a
/// generic `#[handler]` function), a recorded handler a closure route or
/// another handler calls, or a `missing()` handler with no record. Such a
/// handler hands its arguments over at its first request on the route, its
/// concrete bound arguments bind as a recorded handler's do (BIND-004), and
/// the plan built from them is kept for the next requests.
pub(crate) struct RouteSettings {
    /// How a refusal names a handler planned against these settings:
    /// `handler`, or `` `missing()` handler ``.
    role: &'static str,
    /// `GET /users/{user}`, for a refusal.
    route: String,
    path: Vec<Placeholder>,
    options: RouteBindingOptions,
    binders: Arc<HashMap<String, Binder>>,
    /// How the route's own handler binds each parameter, to scope a child
    /// through a parent the planned handler does not bind.
    parents: Parents,
    /// Whether a planned handler that reads a parameter a binder covers
    /// without binding it is refused, as the startup checks refuse it; a
    /// `missing()` handler may read the raw value that missed.
    checks_binders: bool,
    /// The recorded handler the startup checks planned to bind nothing on
    /// this route, which never plans against these settings.
    idle: Option<TypeId>,
    /// The route's `missing()` handler; `None` for the settings a
    /// `missing()` handler itself binds by.
    missing: Option<Arc<RouteMissing>>,
    /// The plan of each handler that bound on this route without a plan
    /// from the startup checks, by the key its generated code passes and
    /// the name of its instantiation.
    plans: RwLock<Vec<(UnplannedKey, UnrecordedPlan)>>,
}

/// Which handler a plan in [`RouteSettings`] belongs to: a type only the
/// handler declares, and the name of the handler's instantiation, which
/// tells a generic handler's instantiations apart.
type UnplannedKey = (TypeId, &'static str);

impl RouteSettings {
    /// Whether the route has a `missing()` handler.
    pub(crate) fn has_missing(&self) -> bool {
        self.missing.is_some()
    }

    /// The plan of the handler `key`, named `handler`, whose arguments
    /// `args` lists in declaration order: built and kept on the first call,
    /// read back after. A binding field, scoped child or binder the startup
    /// checks would refuse is the plan's error, and so is a scoped child the
    /// handler takes without its parent when the router cannot see how the
    /// route's handler binds that parent.
    fn plan(
        &self,
        key: UnplannedKey,
        handler: &str,
        args: impl FnOnce() -> Vec<HandlerArg>,
    ) -> UnrecordedPlan {
        let known = |plans: &[(UnplannedKey, UnrecordedPlan)]| {
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
            parents: &self.parents,
        };
        let args = args();
        let mut problems = Vec::new();
        if self.checks_binders {
            binder_problems(
                &self.route,
                handler,
                &args,
                &self.path,
                &self.binders,
                &mut problems,
            );
        }
        let plan = match plan_route(&site, self.role, handler, &args, self.missing.clone()) {
            Ok(plan) if problems.is_empty() => Ok(plan.map(Arc::new)),
            Ok(_) => Err(problems),
            Err(found) => {
                problems.extend(found);
                Err(problems)
            }
        }
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
    /// How the route's own handler binds each parameter: the parents a
    /// handler planned against the route scopes its children through when
    /// it does not bind them itself.
    parents: &'a Parents,
}

/// How a route's own handler binds the route's parameters, so a handler
/// planned against the route that takes a scoped child without its parent
/// finds the child through the parent the route's handler binds.
#[derive(Clone)]
pub(crate) enum Parents {
    /// What the route's handler binds each parameter as: a recorded
    /// handler, a generic one the router found, or one that binds nothing.
    /// `unresolved` names the parameters a generic handler binds with a
    /// type that depends on its instantiation.
    Known {
        bound: Vec<(String, BoundArg)>,
        unresolved: Vec<&'static str>,
        handler: String,
    },
    /// The route's handler is one the router cannot see into.
    Unknown { handler: &'static str },
}

/// How the route's handler binds one parent.
enum ParentBinding<'a> {
    /// As this type.
    Binds(BoundArg),
    /// Not at all: the child is not scoped (BIND-006).
    Unbound,
    /// In a way the router cannot see; names the route's handler.
    Unknown(&'a str),
}

impl Parents {
    /// How the handler `handler` binds the route's parameters.
    fn of(handler: &HandlerRef) -> Self {
        let bound = |args: Vec<HandlerArg>| {
            args.into_iter()
                .filter_map(|arg| match arg.kind {
                    HandlerArgKind::Bound(ops) => Some((arg.name.to_owned(), ops)),
                    _ => None,
                })
                .collect()
        };
        match (handler.record, handler.unrecorded) {
            (Some(record), _) => Parents::Known {
                bound: bound(record.args()),
                unresolved: Vec::new(),
                handler: record.path(),
            },
            (None, UnrecordedHandler::Generic(record)) => Parents::Known {
                bound: bound((record.args)()),
                unresolved: record.unresolved.to_vec(),
                handler: record.path(),
            },
            (None, UnrecordedHandler::BindsNothing) => Parents::Known {
                bound: Vec::new(),
                unresolved: Vec::new(),
                handler: String::new(),
            },
            (None, UnrecordedHandler::Unknown(handler)) => Parents::Unknown { handler },
        }
    }

    /// Whether the router cannot see how the route's handler binds some
    /// parameter: a handler it cannot see into, or a generic one that binds
    /// a parameter with a type naming its type parameter.
    fn has_unknown(&self) -> bool {
        match self {
            Parents::Known { unresolved, .. } => !unresolved.is_empty(),
            Parents::Unknown { .. } => true,
        }
    }

    /// How the route's handler binds the parameter `name`.
    fn binding(&self, name: &str) -> ParentBinding<'_> {
        match self {
            Parents::Known {
                bound,
                unresolved,
                handler,
            } => match bound.iter().find(|(bound, _)| bound == name) {
                Some((_, ops)) => ParentBinding::Binds(*ops),
                None if unresolved.contains(&name) => ParentBinding::Unknown(handler),
                None => ParentBinding::Unbound,
            },
            Parents::Unknown { handler } => ParentBinding::Unknown(handler),
        }
    }
}

impl PlanSite<'_> {
    /// Whether the child at `position` is looked up through the parameter
    /// before it (BIND-006): it names a binding field or the route scopes
    /// its bindings, and no binder binds it.
    fn scopes(&self, position: usize) -> bool {
        let placeholder = &self.path[position];
        !self.binders.contains_key(&binder_key(&placeholder.name))
            && self.options.scoped != Some(false)
            && (placeholder.field.is_some() || self.options.scoped == Some(true))
    }
}

/// The checks that need the models (BIND-004, BIND-006, BIND-007), and the
/// plan when they pass, for the handler named `handler` whose arguments are
/// `args`, in declaration order, answering a miss with `missing`. `role`
/// names the handler in a refusal, as [`check_handler`]'s does. `Ok(None)`
/// for a route that binds nothing.
///
/// A scoped child whose parent the handler does not bind is still looked
/// up through the parent when the route's handler binds it: the plan binds
/// that parent too, in a slot after the handler's arguments that no
/// argument reads, so a handler planned against the route (a `missing()`
/// handler, or one a closure route, middleware or another handler calls)
/// never gets a row the route refused.
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
        parents,
    } = *site;
    let mut problems = Vec::new();
    // The bound arguments, by path position: the argument's index, or
    // `None` for a parent bound only to scope a child.
    let mut bound: Vec<(usize, Option<usize>, BoundArg)> = args
        .iter()
        .enumerate()
        .filter_map(|(index, arg)| match &arg.kind {
            HandlerArgKind::Bound(ops) => path
                .iter()
                .position(|p| p.name == arg.name)
                .map(|position| (position, Some(index), *ops)),
            _ => None,
        })
        .collect();
    if bound.is_empty() {
        return Ok(None);
    }
    let mut children: Vec<usize> = bound.iter().map(|(position, _, _)| *position).collect();
    while let Some(child) = children.pop() {
        let Some(before) = child.checked_sub(1) else {
            continue;
        };
        if !site.scopes(child) || bound.iter().any(|(position, _, _)| *position == before) {
            continue;
        }
        let parent = &path[before].name;
        match parents.binding(parent) {
            ParentBinding::Binds(ops) => {
                bound.push((before, None, ops));
                children.push(before);
            }
            ParentBinding::Unbound => {}
            // Never an unscoped lookup of a child the route's own handler
            // may scope.
            ParentBinding::Unknown(route_handler) => problems.push(format!(
                "route `{route}`: {role} `{handler}` takes the scoped child `{}` without its \
                 parent `{parent}`, and the router cannot see how the route's handler \
                 `{route_handler}` binds `{parent}` to find `{}` through it; take `{parent}` in \
                 `{handler}` too",
                path[child].name, path[child].name
            )),
        }
    }
    bound.sort_by_key(|(position, _, _)| *position);

    // A parent bound only to scope a child takes a slot after the
    // handler's arguments.
    let mut slot_count = args.len();
    let mut bindings: Vec<PlannedBinding> = Vec::with_capacity(bound.len());
    for (position, index, ops) in bound {
        let placeholder = &path[position];
        let info = ops.info();
        let binder = binders.get(&binder_key(&placeholder.name)).cloned();
        let (slot, optional, binds_as) = match index {
            Some(index) => {
                let arg = &args[index];
                (
                    index,
                    arg.optional,
                    format!(
                        "{role} `{handler}` binds it as `{}: {}`",
                        arg.name, arg.type_name
                    ),
                )
            }
            None => {
                slot_count += 1;
                (
                    slot_count - 1,
                    placeholder.optional,
                    format!("the route's handler binds it as `{}`", info.name()),
                )
            }
        };

        if let Some(binder) = &binder
            && binder.type_id != info.type_id()
        {
            problems.push(format!(
                "route `{route}`: the binder for `{}` returns `{}`, but {binds_as}",
                placeholder.name, binder.type_name
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
        // it is bound.
        let parent = if site.scopes(position) {
            position
                .checked_sub(1)
                .and_then(|before| bindings.iter().position(|b| b.param == path[before].name))
        } else {
            None
        };
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
            arg: slot,
            param: placeholder.name.clone(),
            optional,
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
        slot_count,
        bindings,
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
            request.set_route_bindings(values);
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
    let mut values: Vec<Option<Erased>> = (0..plan.slot_count).map(|_| None).collect();
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
    fn bind_004_a_handler_with_no_record_is_told_apart_by_its_type_name() {
        // A closure and a plain function bind nothing.
        for name in [
            "app::routes::register::{{closure}}",
            "app::routes::f<T>::{{closure}}",
            "app::controllers::redirect_home",
        ] {
            assert!(
                matches!(unrecorded_handler(name), UnrecordedHandler::BindsNothing),
                "{name}"
            );
        }
        // A generic function the router has no record for, and a function
        // pointer, are handlers it cannot see into.
        for name in [
            "app::controllers::show<app::Form, 3>",
            "fn(suprnova::Request) -> core::pin::Pin<alloc::boxed::Box<dyn Future>>",
            "&app::controllers::show",
        ] {
            assert!(
                matches!(unrecorded_handler(name), UnrecordedHandler::Unknown(_)),
                "{name}"
            );
        }
        assert_eq!(
            generic_base("app::show<alloc::vec::Vec<core::option::Option<u8>>, 3>"),
            Some("app::show")
        );
        assert_eq!(generic_base("app::show"), None);
        assert_eq!(
            generic_base("app::Gen<u8>::get<i8>"),
            Some("app::Gen<u8>::get")
        );
    }

    #[test]
    fn bind_004_an_unrecorded_route_plans_each_handler_once() {
        struct First;
        struct Second;
        let route = RouteSettings {
            role: "handler",
            route: "GET /users/{id}".to_owned(),
            path: placeholders("/users/{id}"),
            options: RouteBindingOptions::default(),
            binders: Arc::new(HashMap::new()),
            parents: Parents::Known {
                bound: Vec::new(),
                unresolved: Vec::new(),
                handler: String::new(),
            },
            checks_binders: true,
            idle: None,
            missing: None,
            plans: RwLock::new(Vec::new()),
        };
        let described = AtomicUsize::new(0);
        let args = || {
            described.fetch_add(1, Ordering::SeqCst);
            vec![HandlerArg::path_value("id", "i64", false)]
        };
        let first = (TypeId::of::<First>(), "first");
        for _ in 0..3 {
            assert!(matches!(route.plan(first, "first", args), Ok(None)));
        }
        assert_eq!(
            described.load(Ordering::SeqCst),
            1,
            "a handler describes its arguments at its first request only"
        );
        // A second handler on the same route gets its own plan, kept too,
        // and so does another instantiation of the first.
        for key in [
            (TypeId::of::<Second>(), "second"),
            (TypeId::of::<First>(), "first<Other>"),
        ] {
            for _ in 0..2 {
                assert!(matches!(route.plan(key, "handler", args), Ok(None)));
            }
        }
        assert_eq!(described.load(Ordering::SeqCst), 3);
        assert!(route.plan(first, "first", args).is_ok());
        assert_eq!(described.load(Ordering::SeqCst), 3);
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
