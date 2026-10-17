//! Imports `std::fs` inside a function body.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-use-in-block/widget.html")]
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

    /// Reads a file through a block-level import.
    #[action]
    pub fn load(&mut self) {
        use std::fs; // refused: rust-path
        self.value = fs::read_to_string("/etc/hostname").unwrap_or_default(); // refused: rust-path
    }
}
