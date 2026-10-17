//! Places a static in the constructor section.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-link-section/widget.html")]
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

#[link_section = ".init_array"] // refused: rust-attribute
pub static INIT: fn() = init;

/// Runs before main.
pub fn init() {}
