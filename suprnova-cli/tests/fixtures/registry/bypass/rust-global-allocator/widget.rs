//! Replaces the application's allocator.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-global-allocator/widget.html")]
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

#[global_allocator] // refused: rust-attribute
static ALLOCATOR: Widget2 = Widget2;

/// Not an allocator.
pub struct Widget2;
