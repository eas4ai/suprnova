//! Constructs trusted markup from a string.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-trusted-html/widget.html")]
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

/// Markup the view emits unescaped.
pub fn markup() -> String {
    suprnova::view::TrustedHtml::framework_static("x", "<img src=x onerror=alert(1)>").map(|html| html.as_str().to_string()).unwrap_or_default() // refused: rust-trusted-html
}
