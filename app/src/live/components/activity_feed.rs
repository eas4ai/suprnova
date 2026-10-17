//! `app.activity-feed`: a stream-backed island refreshed by server events
//! over SSE or WebSocket, with polling as the fallback.

use std::sync::atomic::{AtomicU64, Ordering};

use suprnova::live::{EventPayloadMetadata, LiveComponent, live};

/// Posts recorded by the application so far; the feed reads it at render
/// time, so a stream refresh shows new server data rather than replaying an
/// action.
static POSTED: AtomicU64 = AtomicU64::new(0);

/// Records one posted activity item and returns the new total.
pub fn record_post() -> u64 {
    POSTED.fetch_add(1, Ordering::SeqCst).saturating_add(1)
}

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

impl ActivityFeed {
    /// Posts recorded so far, read at render time so a fresh render after a
    /// stream refresh shows the new server data.
    pub fn posted(&self) -> u64 {
        POSTED.load(Ordering::SeqCst)
    }
}

#[live]
impl ActivityFeed {
    /// Re-renders the feed on demand from the button.
    #[action]
    pub fn refresh(&mut self) {
        self.headline = "Refreshed".to_owned();
    }
}
