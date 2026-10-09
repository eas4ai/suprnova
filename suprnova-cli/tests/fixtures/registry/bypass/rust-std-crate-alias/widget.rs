//! Renames the `std` crate and reaches `fs` through it.
use suprnova::live::{LiveComponent, live};
use ::std as base;

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-std-crate-alias/widget.html")]
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
            value: base::fs::read_to_string("/etc/hostname").unwrap_or_default(), // refused: rust-path,
        }
    }
}
