//! A stream-backed feed with a searchable, URL-reflected query and an
//! upload, so its contract carries events, a stream, a URL binding and an
//! upload policy.

use suprnova::live::{EventPayloadMetadata, LiveComponent, UploadPolicy, UploadScan, live};

/// Published when an item is posted.
pub struct ItemPosted;

impl EventPayloadMetadata for ItemPosted {
    const NAME: &'static str = "acme.item-posted";
    const VERSION: u16 = 1;
}

/// The picture the form proposes, finalized by `save`.
fn picture_policy() -> UploadPolicy {
    let builder: suprnova::live::UploadPolicyBuilder = UploadPolicy::builder();
    let builder: suprnova::live::UploadPolicyBuilder = builder.scan(UploadScan::Disabled);
    let builder: suprnova::live::UploadPolicyBuilder = builder.finalize_action("save");
    builder.build()
}

/// The latest headline and the posts the application recorded.
#[derive(LiveComponent)]
#[live(
    name = "acme.feed",
    view = "acme-ui/feed/feed.html",
    minimum_protocol_version = 2,
    checker_contract_version = 2,
    streams(stream(name = "items", topics("acme.items"), events(ItemPosted)))
)]
pub struct Feed {
    /// The latest headline.
    #[public]
    headline: String,
    /// What the reader searches for, reflected in the URL.
    #[model(debounce = 250)]
    #[url(key = "q")]
    query: String,
    /// The pending picture upload handle.
    #[model]
    #[upload(policy = picture_policy)]
    picture: String,
    /// How many posts arrived.
    posted: u64,
}

impl Feed {
    /// Posts recorded so far.
    pub fn posted(&self) -> u64 {
        self.posted
    }
}

#[live]
impl Feed {
    /// Starts empty.
    #[mount]
    pub fn mount() -> Self {
        Self {
            headline: String::new(),
            query: String::new(),
            picture: String::new(),
            posted: 0,
        }
    }

    /// Re-renders the feed.
    #[action]
    pub fn refresh(&mut self) {
        self.headline = "Refreshed".to_owned();
    }

    /// Accepts the picture.
    #[action]
    pub fn save(&mut self) {}
}
