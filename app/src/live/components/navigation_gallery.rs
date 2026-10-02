//! `app.navigation-gallery`: mounts every navigation component the library
//! ships, so `live:check`, the document tests and the browser matrix
//! exercise the real set (Cairn NAV-001 to NAV-004, NAV-006).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use suprnova::live::action::{OutcomeMetadata, action_result, url_intent};
use suprnova::live::{ActionOutcome, ActionResult, CanonicalValue, LiveComponent, live};

/// One row of the feed the load-more control extends.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FeedItem {
    /// Stable key: the morph keeps the row across an append.
    pub key: String,
    /// The row's text.
    pub text: String,
}

/// Pages the Live pagination walks; route pagination links the same three.
const PAGES: u32 = 3;
/// Rows per load-more append and the feed's total.
const PAGE_SIZE: usize = 3;
const FEED_TOTAL: usize = 9;

/// A page that uses each shipped navigation component once, rendered by
/// `live/navigation-gallery.html`. The page number is a mount parameter
/// from the document's query; the Live pagination actions reflect the new
/// page into the current history entry (NAV-003) and the load-more action
/// appends keyed rows (NAV-006).
/// The checked loop-key filter the feed list uses for every appended row.
pub mod filters {
    pub use suprnova::view::filters::live_key;
}

#[derive(LiveComponent)]
#[live(
    name = "app.navigation-gallery",
    view = "live/navigation-gallery.html",
    minimum_protocol_version = 2
)]
pub struct NavigationGallery {
    /// The current page, 1 to `PAGES`.
    #[public]
    page: u32,
    /// The page count, for the position text.
    #[public]
    pages: u32,
    /// Whether the previous control is unavailable.
    #[public]
    first: bool,
    /// Whether the next control is unavailable.
    #[public]
    last: bool,
    /// The feed rows loaded so far.
    #[public]
    items: Vec<FeedItem>,
    /// Whether the server has no further feed page.
    #[public]
    exhausted: bool,
}

#[live]
impl NavigationGallery {
    /// Starts on the requested page (clamped to the range) with the first
    /// feed page loaded.
    #[mount]
    pub fn mount(page: u32) -> Self {
        let page = page.clamp(1, PAGES);
        let mut gallery = Self {
            page,
            pages: PAGES,
            first: page == 1,
            last: page == PAGES,
            items: Vec::new(),
            exhausted: false,
        };
        gallery.append_rows();
        gallery
    }

    /// Moves to the next page and reflects it into the query.
    #[action]
    pub fn next_page(&mut self) -> ActionResult {
        self.go_to(self.page + 1)
    }

    /// Moves to the previous page and reflects it into the query.
    #[action]
    pub fn previous_page(&mut self) -> ActionResult {
        self.go_to(self.page.saturating_sub(1))
    }

    /// Appends the next feed page; the control leaves the view on the last.
    #[action]
    pub fn load_more(&mut self) {
        self.append_rows();
    }

    fn go_to(&mut self, page: u32) -> ActionResult {
        self.page = page.clamp(1, PAGES);
        self.first = self.page == 1;
        self.last = self.page == PAGES;
        let query = CanonicalValue::Object(BTreeMap::from([(
            "page".to_owned(),
            CanonicalValue::number(f64::from(self.page)).expect("a page number is canonical"),
        )]));
        let metadata = OutcomeMetadata::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Some(url_intent(query).expect("a same-route page query is a valid URL intent")),
        )
        .expect("URL-only metadata is valid");
        action_result::<Self>(ActionOutcome::Render, metadata)
            .expect("a render with a URL intent is a valid result")
    }

    fn append_rows(&mut self) {
        let start = self.items.len();
        for index in start..(start + PAGE_SIZE).min(FEED_TOTAL) {
            self.items.push(FeedItem {
                key: format!("row-{}", index + 1),
                text: format!("Row {}", index + 1),
            });
        }
        self.exhausted = self.items.len() >= FEED_TOTAL;
    }
}
