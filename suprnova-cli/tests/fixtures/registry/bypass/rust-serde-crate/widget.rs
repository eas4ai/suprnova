//! Points serde's derive at another crate.
use suprnova::live::{LiveComponent, live};
use suprnova::serde::Serialize;

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-serde-crate/widget.html")]
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

/// A payload.
#[derive(Serialize)]
#[serde(crate = "evil_serde")] // refused: rust-attribute
pub struct Payload {
    /// A field.
    pub field: u32,
}
