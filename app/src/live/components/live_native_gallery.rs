//! The live-native gallery: the upload widget, the live feed and the
//! notification bell, the input OTP, the date picker and the combobox on one
//! stream-backed island (Cairn FORM-005 to FORM-008, FDB-005).
//!
//! The attachment is a real upload field finalized by `save_attachment`
//! through the application's upload finalizer, so the widget's states come
//! from the shipped protocol. `post` records an activity item and publishes
//! the `activity.posted` event the feed subscribes to, so the stream refresh
//! shows new server data. The one-time code is transient (FORM-004).

use serde::{Deserialize, Serialize};
use suprnova::live::{
    LiveComponent, UploadPolicy, UploadReplacement, UploadScan, UploadType, live,
};

use super::activity_feed::{ActivityPosted, record_post};

/// The checked filter the feed and the combobox use for their loop keys.
pub mod filters {
    pub use suprnova::view::filters::live_key;
}

/// One keyed feed entry.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FeedItem {
    /// The stable domain key.
    pub key: String,
    /// What happened.
    pub text: String,
    /// When, as the server rendered it.
    pub time: String,
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn attachment_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(512 * 1024)
        .replacement(UploadReplacement::RetirePrevious)
        .accept(UploadType::Png)
        .scan(UploadScan::Disabled)
        .finalize_action("save_attachment")
        .build()
}

#[derive(LiveComponent)]
#[live(
    name = "app.live-native-gallery",
    view = "live/live-native-gallery.html",
    minimum_protocol_version = 2,
    checker_contract_version = 2,
    streams(stream(name = "activity", topics("activity"), events(ActivityPosted)))
)]
pub struct LiveNativeGallery {
    /// The pending upload handle the browser proposes; finalized by `save_attachment`.
    #[model]
    #[upload(policy = attachment_policy)]
    attachment: String,
    /// The one-time code, transient so it never enters a snapshot (FORM-004).
    #[model(transient)]
    code: String,
    /// The renewal date, bound with the date picker.
    #[model]
    when: String,
    /// The country query, bound with the combobox.
    #[model(debounce = 250)]
    country: String,
    /// Attachments saved so far through the finalizer.
    #[public]
    saved: u32,
    /// Codes verified so far.
    #[public]
    verified: u32,
    /// Unread notifications; the bell shows it.
    #[public]
    unread: u32,
    /// The keyed feed entries, newest first.
    #[public]
    items: Vec<FeedItem>,
    /// The country choices the combobox offers, filtered by the query.
    #[public]
    countries: Vec<(String, String)>,
    /// The years the date strips offer.
    #[public]
    years: Vec<u32>,
    /// The month names the date strips offer.
    #[public]
    months: Vec<String>,
    /// The days the date strips offer.
    #[public]
    days: Vec<u32>,
    /// The OTP cell indexes.
    #[public]
    cells: Vec<u32>,
}

const COUNTRIES: [(&str, &str); 6] = [
    ("ca", "Canada"),
    ("cm", "Cameroon"),
    ("cl", "Chile"),
    ("de", "Germany"),
    ("nz", "New Zealand"),
    ("us", "United States"),
];

impl LiveNativeGallery {
    fn filter_countries(&mut self) {
        let needle = self.country.trim().to_lowercase();
        self.countries = COUNTRIES
            .iter()
            .filter(|(_, label)| needle.is_empty() || label.to_lowercase().contains(&needle))
            .map(|(value, label)| ((*value).to_owned(), (*label).to_owned()))
            .collect();
    }
}

#[live]
impl LiveNativeGallery {
    /// Starts with one seeded feed entry, no unread notifications and the full
    /// country list.
    #[mount]
    pub fn mount() -> Self {
        let mut gallery = Self {
            attachment: String::new(),
            code: String::new(),
            when: String::new(),
            country: String::new(),
            saved: 0,
            verified: 0,
            unread: 0,
            items: vec![FeedItem {
                key: "post-0".to_owned(),
                text: "Gallery opened".to_owned(),
                time: "09:00".to_owned(),
            }],
            countries: Vec::new(),
            years: (2026..=2028).collect(),
            months: MONTHS.iter().map(|month| (*month).to_owned()).collect(),
            days: (1..=31).collect(),
            cells: (0..6).collect(),
        };
        gallery.filter_countries();
        gallery
    }

    /// Finalizes the pending attachment through the application finalizer.
    #[action]
    pub fn save_attachment(&mut self) {
        self.saved = self.saved.saturating_add(1);
    }

    /// Accepts the one-time code and forgets it.
    #[action]
    pub fn verify(&mut self) {
        if self.code.len() == 6 && self.code.bytes().all(|byte| byte.is_ascii_digit()) {
            self.verified = self.verified.saturating_add(1);
        }
        self.code = String::new();
    }

    /// Submits the renewal form; the country choices already answer the
    /// query, because every render filters them.
    #[action]
    pub fn search(&mut self) {}

    /// Posts an activity item: a new keyed feed entry and one more unread
    /// notification, published so every subscribed feed refreshes.
    #[action]
    pub fn post(&mut self) {
        let count = record_post();
        self.items.insert(
            0,
            FeedItem {
                key: format!("post-{count}"),
                text: format!("Update {count} posted"),
                time: format!("09:{:02}", count % 60),
            },
        );
        self.unread = self.unread.saturating_add(1);
    }

    /// Answers the combobox's query on every render, a model update
    /// included, so the listbox the server renders is always the answer to
    /// the query it carries (FORM-008, FORM-012).
    #[rendering]
    pub fn answer_country_query(&mut self) {
        self.filter_countries();
    }
}
