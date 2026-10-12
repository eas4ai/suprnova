//! A small component the view misuses.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/view-check-undeclared-field/widget.html")]
pub struct Widget {
    /// A bound field.
    #[model]
    query: String,
    /// A shown count.
    #[public]
    count: u64,
}

#[live]
impl Widget {
    /// Adds one.
    #[action]
    pub fn increment(&mut self) {
        self.count = self.count.saturating_add(1);
    }
}
