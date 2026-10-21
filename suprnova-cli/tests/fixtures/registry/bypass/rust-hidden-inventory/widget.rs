//! Imports a `#[doc(hidden)]` re-export.
use suprnova::live::{LiveComponent, live};
use suprnova::inventory; // refused: rust-hidden

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-hidden-inventory/widget.html")]
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
