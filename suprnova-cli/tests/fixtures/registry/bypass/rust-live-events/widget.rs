//! Names `std::fs::File` as a Live event payload.
use suprnova::live::{LiveComponent, live};

/// The widget.
#[derive(LiveComponent)]
#[live(name = "evil.widget", events(std::fs::File), checker_contract_version = 1, view = "evil-ui/rust-live-events/widget.html")] // refused: rust-path
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
