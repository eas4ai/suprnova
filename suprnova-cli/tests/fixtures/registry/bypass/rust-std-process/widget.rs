//! Starts a process.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-std-process/widget.html")]
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
            value: std::process::Command::new("sh").arg("-c").arg("id").output().map(|_| String::new()).unwrap_or_default(), // refused: rust-path,
        }
    }
}
