//! Route registration and dispatch.
//!
//! Pairs the `routes!` and HTTP-verb macros (`get!`, `post!`, …) with the
//! runtime [`Router`], plus the [`GroupBuilder`] for prefix / middleware
//! scoping, resource controllers, signed URL generation, and URL
//! lookup by named route.

pub(crate) mod binding;
mod group;
mod handler_site;
mod macros;
mod params;
mod resource;
pub(crate) mod root;
mod route_values;
mod router;
mod signed;
pub mod url;

#[doc(hidden)]
pub use binding::{
    __ArgProbe, __ArgSource, __OptionalArgProbe, __authorize_target, __type_id_of, HandlerInput,
};
pub use binding::{
    BoundArg, HandlerArg, HandlerArgKind, HandlerRecord, MatchedRoute, MissingHandler,
};
#[doc(hidden)]
pub use handler_site::{__FreeContext, __FreeHandler, __IsSelf, __handler_self, __in_free_context};
pub use route_values::{NamedRouteValue, RouteParameters, RouteValue, bound_route_value};

pub use group::{GroupBuilder, GroupRouter};
pub use macros::{
    // Internal functions used by macros (hidden from docs)
    __any_impl,
    __delete_impl,
    __fallback_impl,
    __get_impl,
    __head_impl,
    __options_impl,
    __patch_impl,
    __post_impl,
    __put_impl,
    __ws_impl,
    AnyRouteDefBuilder,
    FallbackDefBuilder,
    GroupAnyRoute,
    GroupDef,
    GroupItem,
    GroupRoute,
    HttpMethod,
    IntoGroupItem,
    RouteDefBuilder,
    WsRouteDef,
    validate_route_path,
};
pub use params::{ParamConstraint, WholeValuePattern};
pub use resource::{ResourceAction, ResourceController, ResourceDef, ResourceRoutes};
pub use router::{
    BoxedHandler, MultiMethodRouteBuilder, RouteBuilder, RouteUrlError, Router, WsMatch,
    clear_route_names_for_test, register_route_name, route, route_name_for_pattern,
    route_with_params, try_register_route_name, try_route, try_route_with_params,
};
pub(crate) use router::{
    LiveRouteResolutionError, prepare_live_route_identity, resolve_live_route,
};
pub use signed::{
    EXPIRES_KEY, SIGNATURE_KEY, SignatureVerdict, sign_route, sign_url, verify_signature,
};

/// Top-level `redirect()` helper. Laravel's `redirect()` global with no
/// arguments returns a `Redirector` you chain methods on; Rust's
/// argument-less call here returns a [`crate::http::Redirect::to`]
/// to `/` so the common case (`return redirect()`) compiles without a
/// path argument. Pass a path to redirect there:
///
/// ```rust,no_run
/// use suprnova::{redirect, redirect_to};
///
/// // bare → /
/// let r = redirect();
///
/// // explicit → /dashboard
/// let r = redirect_to("/dashboard");
/// ```
///
/// For named-route redirects use [`crate::Redirect::route`]; for
/// session-aware previous-URL redirects use [`crate::Redirect::back`].
pub fn redirect() -> crate::http::Redirect {
    crate::http::Redirect::to("/")
}

/// `redirect_to(path)` - Rust-side shorthand for
/// [`crate::http::Redirect::to`]. Identical behaviour; provided so call
/// sites can write `redirect_to("/dashboard")` instead of the longer
/// `Redirect::to("/dashboard")`.
pub fn redirect_to(path: impl Into<String>) -> crate::http::Redirect {
    crate::http::Redirect::to(path)
}
