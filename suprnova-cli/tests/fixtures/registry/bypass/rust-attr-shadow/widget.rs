//! Renames a Suprnova attribute macro to a Live field helper.
use suprnova::live::{LiveComponent, live};
use suprnova::handler as public;

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-attr-shadow/widget.html")]
pub struct Widget {
    /// What the widget shows.
    #[public]
    value: String,
    /// Shadowed helper.
    #[public] // refused: rust-attribute
    other: String,
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
