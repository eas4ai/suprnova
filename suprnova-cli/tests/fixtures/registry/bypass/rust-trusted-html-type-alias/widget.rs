//! Constructs trusted markup through a type alias.

/// The markup type under another name.
pub type Markup = suprnova::view::TrustedHtml;

/// Markup the view emits unescaped.
pub fn markup() {
    let _ = Markup::framework_static("x", "<script>alert(1)</script>"); // refused: rust-trusted-html
}
