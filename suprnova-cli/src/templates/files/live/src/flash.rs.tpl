//! The toast a page shows once.
//!
//! A step that finishes a flow, such as a sign-in or a saved note, keeps a
//! [`Toast`] in the session with [`Toast::flash`] and redirects. The next
//! document takes it with [`Toast::take`], which removes it, and its layout
//! shows it, so a reload shows nothing.
//!
//! The toast is an ordinary session value the next document takes, not a
//! session flash. A flash ages on every request of the session, the Live
//! requests included: saving a note tells the user's open dashboards to
//! refresh, and that refresh, a request of the same session, would age the
//! toast away before the redirect lands. A Live request never takes the
//! toast, so it waits for the page.

use serde::{Deserialize, Serialize};
use suprnova::{FrameworkError, session_mut};

/// The session key the toast is kept under.
pub const TOAST_KEY: &str = "toast";

/// A one-time message for the page a redirect lands on.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Toast {
    /// How the layout styles the message: `success`, `info` or `error`,
    /// which are variants of the shipped toast component. A string rather
    /// than an enum, so the layout passes it to the component as it is; the
    /// constructors below are the only way the kit sets it.
    pub kind: String,
    /// The text the layout shows.
    pub message: String,
}

impl Toast {
    /// A message about a step that worked.
    pub fn success(message: impl Into<String>) -> Self {
        Self::of_kind("success", message)
    }

    /// A message that informs without judging an outcome, such as one that
    /// must not say whether an account exists.
    pub fn info(message: impl Into<String>) -> Self {
        Self::of_kind("info", message)
    }

    /// A message about a step that failed.
    pub fn error(message: impl Into<String>) -> Self {
        Self::of_kind("error", message)
    }

    fn of_kind(kind: &str, message: impl Into<String>) -> Self {
        Self {
            kind: kind.to_owned(),
            message: message.into(),
        }
    }

    /// Keeps this toast for the next page, replacing one no page took yet.
    /// An HTTP handler and a Live action both call it before they redirect.
    pub fn flash(self) -> Result<(), FrameworkError> {
        session_mut(|session| session.put(TOAST_KEY, &self))
            .ok_or_else(|| FrameworkError::internal("keeping a toast needs the session middleware"))
    }

    /// The toast an earlier request kept, if any. Reading it removes it
    /// from the session.
    pub fn take() -> Option<Self> {
        session_mut(|session| session.pull::<Toast>(TOAST_KEY)).flatten()
    }
}
