//! Climbs out of the namespace module into the application.
use suprnova::live::{LiveComponent, live};
use super::super::models; // refused: rust-path

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-super-super/widget.html")]
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
