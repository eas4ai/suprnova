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
