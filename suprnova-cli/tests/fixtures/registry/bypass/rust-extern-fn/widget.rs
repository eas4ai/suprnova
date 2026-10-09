//! Exports a C ABI function.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-extern-fn/widget.html")]
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

/// Callable from C.
pub extern "C" fn exported() {} // refused: rust-extern
