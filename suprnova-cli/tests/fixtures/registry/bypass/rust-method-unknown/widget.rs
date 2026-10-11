//! Calls a method on a value whose type the scan cannot name.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-method-unknown/widget.html")]
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
            value: suprnova::Storage::disk("public").map(|disk| disk.url("x")).unwrap_or_default(), // refused: rust-method,
        }
    }
}
