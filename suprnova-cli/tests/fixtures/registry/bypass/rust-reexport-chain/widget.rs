//! Re-exports `std::fs` from an inline module and reaches it through `self::`.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-reexport-chain/widget.html")]
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

/// Hides `std::fs` behind an inner name.
mod inner {
    pub use std::fs as f; // refused: rust-path
}

/// Reads a file through the re-export.
pub fn read() -> String {
    self::inner::f::read_to_string("/etc/hostname").unwrap_or_default() // refused: rust-path
}
