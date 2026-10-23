//! `app.feedback-gallery`: mounts every feedback component the library
//! ships, so `live:check`, the document tests and the browser matrix
//! exercise the real set (Cairn FDB-001 to FDB-004, FDB-006).

use serde::{Deserialize, Serialize};
use suprnova::live::{LiveComponent, live};

/// One transient outcome the toast region announces.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Toast {
    /// Stable key: the browser owns a dismissed toast's hidden state under it.
    pub key: String,
    /// info, success, warning or error.
    pub variant: String,
    /// The announced text.
    pub text: String,
}

/// A page that uses each shipped feedback component once, rendered by
/// `live/feedback-gallery.html`. The empty state's reason is a mount
/// parameter, so one document per reason proves FDB-003; the toasts are
/// server state the region announces once.
/// The checked loop-key filter the toast list uses for every toast.
pub mod filters {
    pub use suprnova::view::filters::live_key;
}

#[derive(LiveComponent)]
#[live(name = "app.feedback-gallery", view = "live/feedback-gallery.html")]
pub struct FeedbackGallery {
    /// How many times `save` ran; the info alert reports it.
    #[public]
    saved: u32,
    /// Set by `fail`: the error alert renders and an error toast joins it.
    #[public]
    failure: bool,
    /// Determinate upload progress in percent, advanced by `advance`.
    #[public]
    progress: u32,
    /// The empty state's reason: empty, no-results, no-permission or
    /// disconnected, from the document's query.
    #[public]
    reason: String,
    /// Whether the empty state may offer a create action; false for the
    /// no-permission reason.
    #[public]
    can_create: bool,
    /// Toasts in arrival order, keyed by their sequence number.
    #[public]
    toasts: Vec<Toast>,
}

const REASONS: [&str; 4] = ["empty", "no-results", "no-permission", "disconnected"];

#[live]
impl FeedbackGallery {
    /// Starts with nothing saved, no failure, no progress and no toasts;
    /// an unknown reason falls back to `empty`.
    #[mount]
    pub fn mount(reason: String) -> Self {
        let reason = if REASONS.contains(&reason.as_str()) {
            reason
        } else {
            "empty".to_owned()
        };
        let can_create = reason != "no-permission";
        Self {
            saved: 0,
            failure: false,
            progress: 0,
            reason,
            can_create,
            toasts: Vec::new(),
        }
    }

    /// Records a save and announces it as a success toast.
    #[action]
    pub fn save(&mut self) {
        self.saved += 1;
        self.failure = false;
        self.push_toast("success", format!("Saved {} times", self.saved));
    }

    /// Simulates a failed save: the persistent error alert renders and the
    /// toast repeats it, so the toast is never the only surface (FDB-004).
    #[action]
    pub fn fail(&mut self) {
        self.failure = true;
        self.push_toast("error", "The last save did not reach the server".to_owned());
    }

    /// Advances the determinate progress by a quarter, up to complete.
    #[action]
    pub fn advance(&mut self) {
        self.progress = (self.progress + 25).min(100);
    }

    /// Re-renders the list the skeleton stands in for; it changes nothing.
    #[action]
    pub fn refresh(&mut self) {}

    /// Restores the mounted state for the current reason.
    #[action]
    pub fn reset(&mut self) {
        let fresh = Self::mount(self.reason.clone());
        self.saved = fresh.saved;
        self.failure = fresh.failure;
        self.progress = fresh.progress;
        self.toasts = fresh.toasts;
    }

    fn push_toast(&mut self, variant: &str, text: String) {
        let key = format!("toast-{}", self.toasts.len() + 1);
        self.toasts.push(Toast {
            key,
            variant: variant.to_owned(),
            text,
        });
    }
}
