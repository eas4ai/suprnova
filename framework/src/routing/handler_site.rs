//! Where a `#[handler]` function sits: free, or inside an `impl` block
//! (BIND-003).
//!
//! The record `#[handler]` submits for the router names the function. A
//! free function is named by its name. A function inside an `impl` block
//! is named only through the block's type, which an attribute on the
//! function cannot see, so the handler names it: `#[handler(Self = Posts)]`.
//! These items let the generated code fail the build, with a message that
//! says what to write, when a handler inside an `impl` block leaves its
//! type out, and when the type it names is not the block's.

/// The value of the const `#[handler]` emits beside a free handler.
///
/// The handler's body matches `&(__FreeHandler,)` against `(NAME,)`, where
/// `NAME` is the const's name. Beside a free function the const is in
/// scope by that name, so the pattern compares against it and `NAME` is a
/// `__FreeHandler`. Inside an `impl` block the const is an associated
/// const, which a bare name does not reach, so the pattern binds `NAME`
/// to a `&__FreeHandler`, which [`__in_free_context`] refuses.
#[doc(hidden)]
#[derive(PartialEq, Eq)]
pub struct __FreeHandler;

/// Implemented by [`__FreeHandler`] alone; see its documentation.
#[doc(hidden)]
#[diagnostic::on_unimplemented(
    message = "`#[handler]` on a function inside an `impl` block must name the block's type: write `#[handler(Self = <Type>)]`",
    label = "this handler is inside an `impl` block",
    note = "the router finds a handler's record by the handler's type, and a record emitted inside an `impl` block can name the function only through that type"
)]
pub trait __FreeContext {}

impl __FreeContext for __FreeHandler {}

/// Compiles only for a [`__FreeHandler`]: the probe of a free handler.
#[doc(hidden)]
pub fn __in_free_context<T: __FreeContext>(_probe: T) {}

/// Implemented by a type for itself alone, so `#[handler(Self = Named)]`
/// inside `impl Other` fails the build.
#[doc(hidden)]
#[diagnostic::on_unimplemented(
    message = "`#[handler(Self = {Named})]` names `{Named}`, but the handler is inside `impl {Self}`",
    label = "name the type of the `impl` block"
)]
pub trait __IsSelf<Named: ?Sized> {}

impl<T: ?Sized> __IsSelf<T> for T {}

/// Compiles only when `SelfTy` and `Named` are one type.
#[doc(hidden)]
pub const fn __handler_self<SelfTy: ?Sized + __IsSelf<Named>, Named: ?Sized>() {}
