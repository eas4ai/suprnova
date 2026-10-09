//! The flash data a page shows once.
//!
//! A handler that finishes an action flashes a [`Toast`] under the key
//! `toast` with `Inertia::flash`, then redirects. The next page response
//! carries it as `page.flash.toast` and removes it, so the layout shows it
//! once and the page after that does not. Flash data never enters the
//! browser's history, so going back does not show it again.

use suprnova::InertiaProps;

/// What `page.flash` holds. The marker makes `suprnova generate-types` name
/// this struct as Inertia's `flashDataType`, which types `page.flash` on
/// every page. Handlers never build it: each flashes its `toast` alone.
#[derive(InertiaProps)]
#[inertia_props(flash)]
pub struct Flash {
    /// The message the page a redirect lands on shows, if one was flashed.
    pub toast: Option<Toast>,
}

/// A one-time message for the page a redirect lands on.
#[derive(InertiaProps)]
pub struct Toast {
    /// How the layout styles the message: `"success"`, `"info"` or
    /// `"error"`. A string rather than an enum, because `generate-types`
    /// types a string field as `string` and a Rust enum as `unknown`; the
    /// constructors below are the only way the scaffold sets it.
    pub kind: String,
    /// The text the layout shows.
    pub message: String,
}

impl Toast {
    /// A message about an action that worked.
    pub fn success(message: impl Into<String>) -> Self {
        Self::of_kind("success", message)
    }

    /// A message that informs without judging an outcome, such as one
    /// that must not say whether an account exists.
    pub fn info(message: impl Into<String>) -> Self {
        Self::of_kind("info", message)
    }

    /// A message about an action that failed.
    pub fn error(message: impl Into<String>) -> Self {
        Self::of_kind("error", message)
    }

    fn of_kind(kind: &str, message: impl Into<String>) -> Self {
        Self {
            kind: kind.to_owned(),
            message: message.into(),
        }
    }
}
