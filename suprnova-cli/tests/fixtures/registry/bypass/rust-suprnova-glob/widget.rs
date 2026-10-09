//! Glob-imports Suprnova, so names cannot be listed.
use suprnova::live::{LiveComponent, live};
use suprnova::*; // refused: rust-glob

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-suprnova-glob/widget.html")]
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
