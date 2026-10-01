//! Simple paginator (no count query).
//!
//! [`Paginator<T>`] is the cheap-to-compute sibling of
//! [`LengthAwarePaginator`][crate::pagination::LengthAwarePaginator]:
//! it skips the `COUNT(*)` query entirely and instead fetches
//! `per_page + 1` rows to detect whether a next page exists. Use it
//! for large tables where a total row count is too expensive - every
//! page costs one query instead of two.
//!
//! ## JSON shape
//!
//! The paginator serialises to the shape of Laravel's
//! `Paginator::toArray()`, and `has_more` beside it:
//!
//! ```json
//! {
//!   "current_page": 2,
//!   "current_page_url": "/api/users?page=2",
//!   "data": [...],
//!   "first_page_url": "/api/users?page=1",
//!   "from": 11,
//!   "has_more": true,
//!   "next_page_url": "/api/users?page=3",
//!   "path": "/api/users",
//!   "per_page": 10,
//!   "prev_page_url": "/api/users?page=1",
//!   "to": 20
//! }
//! ```
//!
//! A URL is the `path` and the parameter `page`. With no path the URLs
//! are relative, `?page=2`, and `path` is left out of the JSON.

use serde::Serialize;
use serde::ser::SerializeMap;

/// Paginator without a total row count.
///
/// Equivalent to Laravel's `Paginator`. Returned by
/// [`Builder::simple_paginate`](crate::eloquent::Builder::simple_paginate).
#[derive(Debug, Clone)]
pub struct Paginator<T> {
    /// The rows on the current page.
    pub data: Vec<T>,
    /// 1-based current page index.
    pub current_page: u64,
    /// Page size used to slice the underlying query.
    pub per_page: u64,
    /// `true` when there is at least one more row past this page.
    /// Computed by fetching `per_page + 1` rows and checking for the
    /// overflow.
    pub has_more: bool,
    /// Optional base URL: the URL of a page is this path and the
    /// parameter `page`.
    pub path: Option<String>,
}

impl<T: Serialize> Serialize for Paginator<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("current_page", &self.current_page)?;
        map.serialize_entry("current_page_url", &self.url_for_page(self.current_page))?;
        map.serialize_entry("data", &self.data)?;
        map.serialize_entry("first_page_url", &self.url_for_page(1))?;
        map.serialize_entry("from", &self.first_item())?;
        map.serialize_entry("has_more", &self.has_more)?;
        map.serialize_entry("next_page_url", &self.next_page_url())?;
        if let Some(path) = &self.path {
            map.serialize_entry("path", path)?;
        }
        map.serialize_entry("per_page", &self.per_page)?;
        map.serialize_entry("prev_page_url", &self.previous_page_url())?;
        map.serialize_entry("to", &self.last_item())?;
        map.end()
    }
}

impl<T> Paginator<T> {
    /// Build a new simple paginator from its parts.
    pub fn new(data: Vec<T>, current_page: u64, per_page: u64, has_more: bool) -> Self {
        Self {
            data,
            current_page,
            per_page,
            has_more,
            path: None,
        }
    }

    /// Set the optional base URL. Returns `self` for builder-style
    /// chaining.
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// The URL of a page: the `path` and the parameter `page`. A page
    /// under 1 is page 1, as it is in Laravel's `url`.
    pub fn url_for_page(&self, page: u64) -> String {
        crate::pagination::build_query_url(self.path.as_deref(), "page", &page.max(1).to_string())
    }

    /// The URL of the page behind the current one, and `None` when
    /// there is no more row. Laravel's `nextPageUrl`.
    pub fn next_page_url(&self) -> Option<String> {
        self.has_more
            .then(|| self.url_for_page(self.current_page.saturating_add(1)))
    }

    /// The URL of the page before the current one, and `None` on the
    /// first page. Laravel's `previousPageUrl`.
    pub fn previous_page_url(&self) -> Option<String> {
        (self.current_page > 1).then(|| self.url_for_page(self.current_page - 1))
    }

    /// The number of the first row of the page among all rows, counted
    /// from 1, and `None` for a page with no rows. Laravel's
    /// `firstItem`.
    pub fn first_item(&self) -> Option<u64> {
        (!self.data.is_empty()).then(|| {
            self.current_page
                .saturating_sub(1)
                .saturating_mul(self.per_page)
                .saturating_add(1)
        })
    }

    /// The number of the last row of the page among all rows, and
    /// `None` for a page with no rows. Laravel's `lastItem`.
    pub fn last_item(&self) -> Option<u64> {
        let rows = u64::try_from(self.data.len()).unwrap_or(u64::MAX);
        self.first_item()
            .map(|first| first.saturating_add(rows.saturating_sub(1)))
    }

    /// `true` when the paginator is on the first page. Equivalent to
    /// Laravel's `AbstractPaginator::onFirstPage`.
    pub fn on_first_page(&self) -> bool {
        self.current_page <= 1
    }

    /// `true` when no further `?page=N` will yield rows.
    /// Equivalent to Laravel's `Paginator::onLastPage` (no `has_more`).
    pub fn on_last_page(&self) -> bool {
        !self.has_more
    }

    /// `true` when there is at least one more page to fetch.
    /// Equivalent to Laravel's `Paginator::hasMorePages` (which simply
    /// returns the `$hasMore` flag).
    pub fn has_more_pages(&self) -> bool {
        self.has_more
    }

    /// `true` when there are enough rows to span multiple pages.
    /// Equivalent to Laravel's `AbstractPaginator::hasPages`:
    /// either we're not on page 1 or there are more pages to fetch.
    pub fn has_pages(&self) -> bool {
        self.current_page != 1 || self.has_more
    }

    /// `true` when the page slice contains no rows. Equivalent to
    /// Laravel's `AbstractPaginator::isEmpty`.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// `true` when the page slice contains at least one row.
    /// Equivalent to Laravel's `AbstractPaginator::isNotEmpty`.
    pub fn is_not_empty(&self) -> bool {
        !self.data.is_empty()
    }

    /// Number of rows on the current page slice. Equivalent to
    /// Laravel's `AbstractPaginator::count`.
    pub fn count(&self) -> usize {
        self.data.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_sets_all_fields() {
        let p = Paginator::new(vec![1, 2, 3], 2, 10, true);
        assert_eq!(p.data.len(), 3);
        assert_eq!(p.current_page, 2);
        assert_eq!(p.per_page, 10);
        assert!(p.has_more);
        assert_eq!(p.path, None);
    }

    #[test]
    fn path_serializes_when_set() {
        let p = Paginator::new(vec![1, 2], 1, 10, true).with_path("/api/users");
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(
            json.get("path").and_then(|v| v.as_str()),
            Some("/api/users")
        );
    }

    #[test]
    fn path_omitted_when_unset() {
        let p = Paginator::new(vec![1, 2], 1, 10, true);
        let json = serde_json::to_value(&p).unwrap();
        assert!(json.get("path").is_none());
    }

    #[test]
    fn serializes_to_laravel_shape() {
        let p = Paginator::new(vec![10, 20, 30], 2, 10, false);
        let json = serde_json::to_value(&p).unwrap();
        let m = json.as_object().unwrap();
        assert!(m.contains_key("data"));
        assert!(m.contains_key("current_page"));
        assert!(m.contains_key("per_page"));
        assert!(m.contains_key("has_more"));
        assert_eq!(m.get("current_page").and_then(|v| v.as_u64()), Some(2));
        assert_eq!(m.get("per_page").and_then(|v| v.as_u64()), Some(10));
        assert_eq!(m.get("has_more").and_then(|v| v.as_bool()), Some(false));
    }

    #[test]
    fn predicates_track_page_position_and_has_more() {
        // First page, more pages ahead.
        let p = Paginator::new(vec![1; 10], 1, 10, true);
        assert!(p.on_first_page());
        assert!(!p.on_last_page());
        assert!(p.has_more_pages());
        assert!(p.has_pages());
        // Middle page.
        let p = Paginator::new(vec![1; 10], 3, 10, true);
        assert!(!p.on_first_page());
        assert!(!p.on_last_page());
        assert!(p.has_more_pages());
        assert!(p.has_pages());
        // Last page (has_more = false on a non-first page).
        let p = Paginator::new(vec![1; 5], 4, 10, false);
        assert!(!p.on_first_page());
        assert!(p.on_last_page());
        assert!(!p.has_more_pages());
        assert!(p.has_pages());
        // Single page (page 1, no more).
        let p = Paginator::new(vec![1; 5], 1, 10, false);
        assert!(p.on_first_page());
        assert!(p.on_last_page());
        assert!(!p.has_more_pages());
        assert!(!p.has_pages());
    }

    #[test]
    fn empty_and_count_predicates() {
        let p: Paginator<i32> = Paginator::new(vec![], 1, 10, false);
        assert!(p.is_empty());
        assert!(!p.is_not_empty());
        assert_eq!(p.count(), 0);

        let p = Paginator::new(vec![10, 20, 30], 1, 10, true);
        assert!(!p.is_empty());
        assert!(p.is_not_empty());
        assert_eq!(p.count(), 3);
    }
}
