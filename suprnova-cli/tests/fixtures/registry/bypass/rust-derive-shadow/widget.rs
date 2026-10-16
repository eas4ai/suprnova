//! Renames a Suprnova derive to `Clone`.
use suprnova::live::{LiveComponent, live};
use suprnova::Factory as Clone;

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-derive-shadow/widget.html")]
pub struct Widget {
    /// What the widget shows.
    #[public]
    value: String,
}

#[live]
impl Widget {
    /// Mounts the widget.
    #[mount]
    pub fn mount() -> Self {
        Self {
            value: String::new(),
        }
    }
}

/// Looks like a clone.
#[derive(Clone)] // refused: rust-derive
pub struct Row;
