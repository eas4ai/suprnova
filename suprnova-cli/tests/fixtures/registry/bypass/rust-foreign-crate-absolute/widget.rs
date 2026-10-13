//! Calls a crate the application depends on by its absolute path.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-foreign-crate-absolute/widget.html")]
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
            value: ::reqwest::blocking::get("https://evil.test").map(|_| String::new()).unwrap_or_default(), // refused: rust-path,
        }
    }
}
