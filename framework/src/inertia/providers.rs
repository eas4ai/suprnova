//! Prop providers: values that expand into several props, or convert into
//! one, given what Laravel gives them.
//!
//! Laravel's `ProvidesInertiaProperties` and `ProvidesInertiaProperty`
//! (`inertia-laravel-3.5.1/src/`) let an object stand in for props: a page
//! or the shared props take any number of the first, each expanded at
//! render with a `RenderContext` of the component and the request; a prop
//! value may be the second, converted with a `PropertyContext` of its key
//! path, its sibling props and the request.

use indexmap::IndexMap;

use super::prop::{InertiaRequestExt, Prop};
use crate::error::FrameworkError;

/// What a [`ProvidesInertiaProperties`] value is expanded with: the page
/// component being rendered and the request - Laravel's `RenderContext`.
///
/// A struct rather than two parameters so it can grow, as Laravel's has,
/// without changing every implementation.
pub struct RenderContext<'a> {
    component: &'a str,
    request: &'a dyn InertiaRequestExt,
}

impl<'a> RenderContext<'a> {
    /// The context of rendering `component` for `request`.
    pub fn new(component: &'a str, request: &'a dyn InertiaRequestExt) -> Self {
        Self { component, request }
    }

    /// The page component being rendered, after any name transformation.
    pub fn component(&self) -> &'a str {
        self.component
    }

    /// The request the page is rendered for.
    pub fn request(&self) -> &'a dyn InertiaRequestExt {
        self.request
    }
}

/// A value that expands into props when a page renders - Laravel's
/// `ProvidesInertiaProperties`.
///
/// Give a page any number of them with
/// [`InertiaResponse::provide`](crate::InertiaResponse::provide), and the
/// shared props any number with
/// [`InertiaRegistry::share_provider`](crate::InertiaRegistry::share_provider).
/// Each is expanded once per render with a [`RenderContext`], so its props
/// can depend on the page and the request; its keys merge into the page in
/// the order the providers were given, a later one winning over an earlier
/// one, and a page's own props win over its providers'.
///
/// Expansion is synchronous, as Laravel's is: work that needs to wait
/// belongs in a lazy prop (`Prop::lazy`) the provider returns, which then
/// runs only when the prop is sent.
pub trait ProvidesInertiaProperties: Send + Sync {
    /// The props this value provides for one render.
    ///
    /// # Errors
    ///
    /// An error fails the response, as a resolver's error does.
    fn to_inertia_properties(
        &self,
        context: &RenderContext<'_>,
    ) -> Result<IndexMap<String, Prop>, FrameworkError>;
}

/// What a [`ProvidesInertiaProperty`] value is converted with: its key
/// path, its sibling props and the request - Laravel's `PropertyContext`.
pub struct PropertyContext<'a> {
    key: &'a str,
    props: &'a IndexMap<String, Prop>,
    request: &'a dyn InertiaRequestExt,
}

impl<'a> PropertyContext<'a> {
    /// The context of converting the prop at `key`, among `props`, for
    /// `request`.
    pub fn new(
        key: &'a str,
        props: &'a IndexMap<String, Prop>,
        request: &'a dyn InertiaRequestExt,
    ) -> Self {
        Self {
            key,
            props,
            request,
        }
    }

    /// The prop's key path, a dotted key as it was registered
    /// (`"auth.user"`).
    pub fn key(&self) -> &'a str {
        self.key
    }

    /// Every prop of the page before resolution, shared and the page's
    /// own, this one included - Laravel's sibling props. A sibling's value
    /// is there to read when it was given as a value
    /// ([`Prop::as_value`]); a resolver has not run.
    pub fn props(&self) -> &'a IndexMap<String, Prop> {
        self.props
    }

    /// The request the page is rendered for.
    pub fn request(&self) -> &'a dyn InertiaRequestExt {
        self.request
    }
}

/// A prop value that converts itself when the page is rendered - Laravel's
/// `ProvidesInertiaProperty`.
///
/// Attach one with [`Prop::property`] or
/// [`InertiaResponse::with_property`](crate::InertiaResponse::with_property).
/// The conversion runs only when the prop is sent, with a
/// [`PropertyContext`], so a value can format itself from a sibling prop
/// (a price from the page's currency) or from the request. The result
/// ships whole, as Laravel ships an object's conversion: a dotted `only`
/// entry does not narrow it.
pub trait ProvidesInertiaProperty: Send + Sync {
    /// The prop's value for this render.
    ///
    /// # Errors
    ///
    /// An error fails the response, or is rescued on a deferred prop
    /// carrying [`Prop::rescue`], as a resolver's error is.
    fn to_inertia_property(
        &self,
        context: &PropertyContext<'_>,
    ) -> Result<serde_json::Value, FrameworkError>;
}
