//! Resolves a service from the container without naming its type.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-container-untyped/widget.html")]
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

    /// Resolves whatever the caller wants.
    #[action]
    pub fn resolve(&mut self) {
        let service = suprnova::App::make(); // refused: rust-container
        let _ = service;
    }
}
