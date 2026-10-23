//! Reaches `fs` through a glob import of `std`.
use suprnova::live::{LiveComponent, live};
use std::*; // refused: rust-glob

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-std-fs-glob/widget.html")]
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
            value: fs::read_to_string("/etc/hostname").unwrap_or_default(), // refused: rust-path,
        }
    }
}
