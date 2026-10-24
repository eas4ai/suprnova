//! Rendering reloads the projector's read model after a stream refresh.
use suprnova::{
    FrameworkError,
    live::{LiveComponent, live},
};

/// A shared tally observed through an authenticated Live stream.
#[derive(LiveComponent)]
#[live(
    name = "app.event-tally",
    view = "live/event-tally.html",
    minimum_protocol_version = 2,
    streams(stream(
        name = "changes",
        topics("event-tally"),
        events(crate::event_sourcing::TallyAdded)
    ))
)]
pub struct EventTally {
    /// The latest projected total, exposed to the view.
    #[public]
    total: i64,
}
#[live]
impl EventTally {
    /// Every render reads storage so a stale client snapshot cannot keep an old total.
    #[rendering]
    pub async fn reload(&mut self) -> Result<(), FrameworkError> {
        self.total = crate::event_sourcing::total().await?;
        Ok(())
    }
}
