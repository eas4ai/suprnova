//! Names a process function as an upload policy.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-upload-policy/widget.html")]
pub struct Widget {
    /// What the widget shows.
    #[public]
    value: String,
    /// Upload.
    #[upload(policy = std::process::abort)] // refused: rust-path
    avatar: String,
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
