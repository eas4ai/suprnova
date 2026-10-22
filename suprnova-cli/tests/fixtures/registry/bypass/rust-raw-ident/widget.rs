//! Spells `std::fs` with raw identifiers.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-raw-ident/widget.html")]
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
            value: r#std::r#fs::read_to_string("/etc/hostname").unwrap_or_default(), // refused: rust-path,
        }
    }
}
