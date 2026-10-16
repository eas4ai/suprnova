//! Names Suprnova API whose capability is undecided.
use suprnova::live::{LiveComponent, live};
use suprnova::PasswordReset; // refused: rust-path

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-refused-api/widget.html")]
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
