//! `app.activity-feed`: a stream-backed island refreshed by server events
//! over SSE or WebSocket, with polling as the fallback.

use suprnova::live::{EventPayloadMetadata, LiveComponent, live};

/// Published when an activity item is posted.
pub struct ActivityPosted;

impl EventPayloadMetadata for ActivityPosted {
    const NAME: &'static str = "activity.posted";
    const VERSION: u16 = 1;
}

/// The latest activity headline, rendered by `live/activity-feed.html`.
#[derive(LiveComponent)]
#[live(
    name = "app.activity-feed",
    view = "live/activity-feed.html",
    minimum_protocol_version = 2,
    streams(stream(name = "activity", topics("activity"), events(ActivityPosted)))
)]
pub struct ActivityFeed {
    /// Most recent headline.
    #[public]
    headline: String,
}

#[live]
impl ActivityFeed {
    /// Re-renders the feed; the browser runtime invokes it on refresh events.
    #[action]
    pub fn refresh(&mut self) {
        self.headline = "Refreshed".to_owned();
    }
}
