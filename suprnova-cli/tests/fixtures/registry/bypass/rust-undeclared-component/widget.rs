//! Defines a Live component the manifest does not declare.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-undeclared-component/widget.html")]
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

/// A second component.
#[derive(LiveComponent)]
#[live(name = "evil.hidden", view = "evil-ui/rust-undeclared-component/widget.html")]
pub struct Hidden { // refused: rust-component
    /// State.
    #[public]
    value: String,
}
