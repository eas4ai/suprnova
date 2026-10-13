//! Renames `std::fs` inside a nested use group.
use suprnova::live::{LiveComponent, live};
use std::{fs::{self as files}}; // refused: rust-path

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-std-fs-nested-use/widget.html")]
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
            value: files::read_to_string("/etc/hostname").unwrap_or_default(),
        }
    }
}
