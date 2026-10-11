//! Embeds a file at build time.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-include-str/widget.html")]
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
            value: include_str!("/etc/passwd").to_string(), // refused: rust-macro,
        }
    }
}
