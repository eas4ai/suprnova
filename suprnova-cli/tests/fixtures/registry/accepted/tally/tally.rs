//! Plain `std` code: collections, iterators, closures and formatting, none
//! of which reaches an effect.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use suprnova::live::{LiveComponent, live};

/// One tallied entry.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Entry {
    /// What was counted.
    pub label: String,
    /// How many.
    pub count: u64,
}

impl Entry {
    /// An entry with a count.
    pub fn new(label: &str, count: u64) -> Self {
        Entry {
            label: label.trim().to_lowercase(),
            count,
        }
    }
}

/// Totals by label.
#[derive(LiveComponent)]
#[live(name = "acme.tally", view = "acme-ui/tally/tally.html")]
pub struct Tally {
    /// The rows the view lists.
    #[public]
    rows: Vec<(String, u64)>,
    /// The raw entries.
    entries: Vec<Entry>,
}

#[live]
impl Tally {
    /// Starts with two entries.
    #[mount]
    pub fn mount() -> Self {
        let entries = vec![Entry::new(" Apples ", 3), Entry::new("pears", 2)];
        let mut tally = Self {
            rows: Vec::new(),
            entries,
        };
        tally.recount();
        tally
    }

    /// Adds one apple.
    #[action]
    pub fn add(&mut self) {
        self.entries.push(Entry::new("apples", 1));
        self.recount();
    }

    /// Rebuilds the rows from the entries.
    fn recount(&mut self) {
        let mut totals: BTreeMap<String, u64> = BTreeMap::new();
        for entry in &self.entries {
            *totals.entry(entry.label.clone()).or_insert(0) += entry.count;
        }
        let mut summary = String::new();
        let _ = write!(summary, "{} labels", totals.len());
        self.rows = totals
            .iter()
            .filter(|(label, _)| !label.is_empty())
            .map(|(label, total)| (format!("{label} ({summary})"), *total))
            .collect();
        let largest = self.entries.iter().map(|entry| entry.count).max().unwrap_or_default();
        if largest > 100 {
            self.rows.truncate(10);
        }
    }
}
