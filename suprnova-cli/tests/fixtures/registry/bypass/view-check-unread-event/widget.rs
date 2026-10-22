//! Declares an event whose payload contract the component does not carry.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", view = "evil-ui/view-check-unread-event/widget.html", events(suprnova::live::LiveOutcomeAccepted))]
pub struct Widget { // refused: view-contract
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
