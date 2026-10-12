//! Renames `println!` to an admitted macro name.
use suprnova::live::{LiveComponent, live};
use std::println as format;

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-macro-rename/widget.html")]
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
            value: { format!("leak"); String::new() }, // refused: rust-macro,
        }
    }
}
