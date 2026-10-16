//! Names `core::ptr`, outside the effect-free subset.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-core-ptr/widget.html")]
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
            value: core::ptr::null::<u8>().is_null().to_string(), // refused: rust-path,
        }
    }
}
