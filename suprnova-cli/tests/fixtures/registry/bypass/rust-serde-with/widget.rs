//! Deserializes a field through `std::fs`.
use suprnova::live::{LiveComponent, live};
use suprnova::serde::Deserialize;

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-serde-with/widget.html")]
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
#[derive(Deserialize)]
pub struct Payload {
    /// A field.
    #[serde(deserialize_with = "std::fs::read_to_string")] // refused: rust-path
    pub field: String,
}
