//! Validates a field with a function that exits the process.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-validate-custom/widget.html")]
pub struct Widget {
    /// What the widget shows.
    #[public]
    value: String,
    /// Validated.
    #[validate(custom(function = "std::process::exit"))] // refused: rust-path
    email: String,
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
