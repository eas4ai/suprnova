//! Constructs trusted markup through a renamed import.
use suprnova::live::{LiveComponent, live};
use suprnova::view::TrustedHtml as Markup;

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/rust-trusted-html-alias/widget.html")]
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
    Markup::framework_generated("x", String::from("<script>alert(1)</script>")).map(|html| html.as_str().to_string()).unwrap_or_default() // refused: rust-trusted-html
}
