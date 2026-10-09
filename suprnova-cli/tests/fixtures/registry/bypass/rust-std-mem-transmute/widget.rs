//! Names `std::mem::transmute`, outside the admitted `std::mem` functions.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-std-mem-transmute/widget.html")]
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

/// A transmute in disguise.
pub const TRANSMUTE: unsafe fn(u32) -> f32 = std::mem::transmute::<u32, f32>; // refused: rust-path
