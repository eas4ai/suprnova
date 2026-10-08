//! The datatable gallery: one island per table, with sort, filter and page
//! bound to the URL.
//!
//! The document mounts the table from its query, so a shared URL renders
//! the same view; every action reflects the current sort, direction,
//! filter and page back into the query through the action result's URL
//! intent, without a history entry.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use suprnova::live::action::{OutcomeMetadata, action_result, url_intent};
use suprnova::live::{ActionOutcome, ActionResult, CanonicalValue, LiveComponent, live};

/// The checked loop-key filter the rows use.
pub mod filters {
    pub use suprnova::view::filters::live_key;
}

/// One invoice row.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Invoice {
    /// The stable domain key.
    pub key: String,
    /// The invoice number.
    pub number: String,
    /// The customer.
    pub customer: String,
    /// The amount, formatted.
    pub amount: String,
    /// The status text.
    pub status: String,
}

const PAGE_SIZE: usize = 4;
const COLUMNS: [&str; 3] = ["number", "customer", "amount"];
const CUSTOMERS: [&str; 5] = ["Acme", "Globex", "Initech", "Umbrella", "Hooli"];
const STATUSES: [&str; 3] = ["Paid", "Open", "Overdue"];

/// The query parameters a datatable document mounts from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableQuery {
    /// The sort column.
    pub sort: String,
    /// The sort direction.
    pub direction: String,
    /// The filter text.
    pub filter: String,
    /// The one-based page.
    pub page: u64,
}

#[derive(LiveComponent)]
#[live(
    name = "app.datatable-gallery",
    view = "live/datatable-gallery.html",
    minimum_protocol_version = 2
)]
pub struct DatatableGallery {
    /// The column a sort submit proposes; the applied column is `sorted_by`.
    #[model]
    pub sort: String,
    /// The applied sort column: number, customer or amount.
    #[url(key = "sort")]
    pub sorted_by: String,
    /// The sort direction: asc or desc.
    #[url(key = "dir")]
    pub direction: String,
    /// The direction as the aria-sort word: ascending or descending.
    pub direction_word: String,
    /// The filter text matched against customer and status.
    #[model]
    #[url(key = "filter")]
    pub filter: String,
    /// The one-based page.
    #[url(key = "page")]
    pub page: u64,
    /// The rows on this page.
    pub rows: Vec<Invoice>,
    /// The rows that match the filter, across every page.
    pub count: usize,
    /// The number of pages.
    pub pages: u64,
    /// Whether this is the first page.
    pub first: bool,
    /// Whether this is the last page.
    pub last: bool,
}

#[live]
impl DatatableGallery {
    /// Mounts the table from the document's query; the page arrives as a
    /// plain number and widens into the URL-bound field.
    #[mount]
    pub fn mount(sort: String, direction: String, filter: String, page: u32) -> Self {
        let mut table = Self {
            sort: String::new(),
            sorted_by: String::new(),
            direction: String::new(),
            direction_word: String::new(),
            filter,
            page: 1,
            rows: Vec::new(),
            count: 0,
            pages: 1,
            first: true,
            last: true,
        };
        table.sorted_by = if COLUMNS.contains(&sort.as_str()) {
            sort
        } else {
            "number".to_owned()
        };
        table.sort = table.sorted_by.clone();
        table.direction = if direction == "desc" {
            direction
        } else {
            "asc".to_owned()
        };
        table.page = u64::from(page.max(1));
        table.compute();
        table
    }

    /// Sorts by the proposed column; the same column again flips the direction.
    #[action]
    pub fn sort(&mut self) -> ActionResult {
        self.normalize();
        let proposed = if COLUMNS.contains(&self.sort.as_str()) {
            self.sort.clone()
        } else {
            "number".to_owned()
        };
        self.direction = if proposed == self.sorted_by && self.direction != "desc" {
            "desc".to_owned()
        } else {
            "asc".to_owned()
        };
        self.sorted_by = proposed.clone();
        self.sort = proposed;
        self.page = 1;
        self.finish()
    }

    /// Applies the proposed filter and returns to the first page.
    #[action]
    pub fn filter(&mut self) -> ActionResult {
        self.normalize();
        self.page = 1;
        self.finish()
    }

    /// Moves to the next page.
    #[action]
    pub fn next_page(&mut self) -> ActionResult {
        self.normalize();
        self.page += 1;
        self.finish()
    }

    /// Moves to the previous page.
    #[action]
    pub fn previous_page(&mut self) -> ActionResult {
        self.normalize();
        self.page = self.page.saturating_sub(1).max(1);
        self.finish()
    }

    /// The URL-bound fields omit their defaults from the query, so a field
    /// that hydrated from the reflected URL may arrive empty; every action
    /// starts from the same applied state either way.
    fn normalize(&mut self) {
        if !COLUMNS.contains(&self.sorted_by.as_str()) {
            self.sorted_by = "number".to_owned();
        }
        if self.direction != "desc" {
            self.direction = "asc".to_owned();
        }
        self.page = self.page.max(1);
    }

    fn finish(&mut self) -> ActionResult {
        self.compute();
        let mut query = BTreeMap::new();
        if self.sorted_by != "number" {
            query.insert(
                "sort".to_owned(),
                CanonicalValue::String(self.sorted_by.clone()),
            );
        }
        if self.direction != "asc" {
            query.insert(
                "dir".to_owned(),
                CanonicalValue::String(self.direction.clone()),
            );
        }
        if !self.filter.is_empty() {
            query.insert(
                "filter".to_owned(),
                CanonicalValue::String(self.filter.clone()),
            );
        }
        if self.page != 1 {
            query.insert(
                "page".to_owned(),
                CanonicalValue::number(self.page as f64).expect("a page number is finite"),
            );
        }
        let metadata = OutcomeMetadata::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Some(
                url_intent(CanonicalValue::Object(query))
                    .expect("a same-route query is a valid URL intent"),
            ),
        )
        .expect("URL-only metadata is valid");
        action_result::<Self>(ActionOutcome::Render, metadata)
            .expect("a render with a URL intent is valid")
    }

    fn compute(&mut self) {
        self.direction_word = if self.direction == "desc" {
            "descending".to_owned()
        } else {
            "ascending".to_owned()
        };
        let needle = self.filter.trim().to_lowercase();
        let mut all: Vec<Invoice> = (1..=14)
            .map(|n| Invoice {
                key: format!("inv-{n}"),
                number: format!("{:04}", 1030 + n),
                customer: CUSTOMERS[(n * 3) % CUSTOMERS.len()].to_owned(),
                amount: format!("{}.00", 120 + ((n * 37) % 900)),
                status: STATUSES[n % STATUSES.len()].to_owned(),
            })
            .filter(|row| {
                needle.is_empty()
                    || row.customer.to_lowercase().contains(&needle)
                    || row.status.to_lowercase().contains(&needle)
            })
            .collect();
        all.sort_by(|a, b| match self.sorted_by.as_str() {
            "customer" => a.customer.cmp(&b.customer).then(a.number.cmp(&b.number)),
            "amount" => amount_of(&a.amount)
                .cmp(&amount_of(&b.amount))
                .then(a.number.cmp(&b.number)),
            _ => a.number.cmp(&b.number),
        });
        if self.direction == "desc" {
            all.reverse();
        }
        self.count = all.len();
        self.pages = (self.count.div_ceil(PAGE_SIZE)).max(1) as u64;
        self.page = self.page.clamp(1, self.pages);
        let start = ((self.page - 1) as usize) * PAGE_SIZE;
        self.rows = all.into_iter().skip(start).take(PAGE_SIZE).collect();
        self.first = self.page == 1;
        self.last = self.page == self.pages;
    }
}

fn amount_of(text: &str) -> u32 {
    text.split('.')
        .next()
        .and_then(|whole| whole.parse().ok())
        .unwrap_or(0)
}
