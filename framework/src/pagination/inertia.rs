//! Bridge from `LengthAwarePaginator` / `CursorPaginator` to Inertia's
//! `ScrollMetadata` — the protocol for infinite-scroll props.

use serde_json::Value;

use crate::inertia::{ProvidesScrollMetadata, ScrollMetadata};

use super::{CursorPaginator, LengthAwarePaginator, Paginator};

/// Convert a paginator into an Inertia scroll prop: the metadata + the
/// row vec, which the caller wires onto an [`InertiaResponse`](crate::inertia::InertiaResponse) under a
/// chosen key.
pub trait IntoInertiaScroll<T> {
    /// Split this paginator into its Inertia scroll metadata and the
    /// underlying data rows.
    fn into_inertia_scroll(self) -> (ScrollMetadata, Vec<T>);
}

// The Inertia scroll `pageName` is hardcoded to `"page"` here rather
// than read from `LengthAwarePaginator::page_name` (a separate field
// that only affects `url_for_page`'s JSON:API-style pagination
// *links*). That's a pre-existing gap between the link key and the
// scroll key, not something this trait changes.
impl<T> ProvidesScrollMetadata for LengthAwarePaginator<T> {
    fn page_name(&self) -> String {
        "page".to_string()
    }

    fn previous_page(&self) -> Option<Value> {
        if self.current_page > 1 {
            Some(Value::from((self.current_page - 1) as i64))
        } else {
            None
        }
    }

    fn next_page(&self) -> Option<Value> {
        if self.has_more_pages() {
            Some(Value::from((self.current_page + 1) as i64))
        } else {
            None
        }
    }

    fn current_page(&self) -> Option<Value> {
        Some(Value::from(self.current_page as i64))
    }
}

impl<T> IntoInertiaScroll<T> for LengthAwarePaginator<T> {
    fn into_inertia_scroll(self) -> (ScrollMetadata, Vec<T>) {
        let meta = self.scroll_metadata();
        (meta, self.data)
    }
}

/// Same page-number protocol as [`LengthAwarePaginator`], with one
/// difference that follows from what a simple paginator knows: `next` is
/// derived from the `has_more` overflow probe rather than from a computed
/// last page, because there is no total to compute one from. That is the
/// entire point of the type — a listing over a table large enough to make
/// `COUNT(*)` the dominant cost of the request should not pay for one to
/// render a "next" link.
impl<T> ProvidesScrollMetadata for Paginator<T> {
    fn page_name(&self) -> String {
        "page".to_string()
    }

    fn previous_page(&self) -> Option<Value> {
        if self.current_page > 1 {
            Some(Value::from((self.current_page - 1) as i64))
        } else {
            None
        }
    }

    fn next_page(&self) -> Option<Value> {
        if self.has_more_pages() {
            Some(Value::from((self.current_page + 1) as i64))
        } else {
            None
        }
    }

    fn current_page(&self) -> Option<Value> {
        Some(Value::from(self.current_page as i64))
    }
}

impl<T> IntoInertiaScroll<T> for Paginator<T> {
    fn into_inertia_scroll(self) -> (ScrollMetadata, Vec<T>) {
        let meta = self.scroll_metadata();
        (meta, self.data)
    }
}

impl<T> ProvidesScrollMetadata for CursorPaginator<T> {
    fn page_name(&self) -> String {
        "cursor".to_string()
    }

    fn previous_page(&self) -> Option<Value> {
        self.prev_cursor.clone().map(Value::String)
    }

    fn next_page(&self) -> Option<Value> {
        self.next_cursor.clone().map(Value::String)
    }

    fn current_page(&self) -> Option<Value> {
        None
    }
}

impl<T> IntoInertiaScroll<T> for CursorPaginator<T> {
    fn into_inertia_scroll(self) -> (ScrollMetadata, Vec<T>) {
        let meta = self.scroll_metadata();
        (meta, self.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_paginator_first_page_has_next_and_no_previous() {
        let (meta, data) = Paginator::new(vec![1, 2, 3], 1, 3, true).into_inertia_scroll();
        assert_eq!(meta.page_name, "page");
        assert_eq!(meta.current_page, Some(Value::from(1)));
        assert_eq!(meta.previous_page, None, "page 1 has nothing behind it");
        assert_eq!(meta.next_page, Some(Value::from(2)));
        assert_eq!(data, vec![1, 2, 3]);
    }

    #[test]
    fn simple_paginator_middle_page_has_both_neighbours() {
        let (meta, _) = Paginator::new(vec![4, 5, 6], 2, 3, true).into_inertia_scroll();
        assert_eq!(meta.previous_page, Some(Value::from(1)));
        assert_eq!(meta.next_page, Some(Value::from(3)));
    }

    /// The edge the `has_more` probe exists to detect. A length-aware
    /// paginator knows it is on the last page by comparing against a
    /// total; the simple paginator only knows that its `per_page + 1`
    /// fetch came back short, and that has to be enough to withhold the
    /// next link — otherwise clients page forever into an empty tail.
    #[test]
    fn simple_paginator_last_page_withholds_next() {
        let (meta, _) = Paginator::new(vec![7, 8], 3, 3, false).into_inertia_scroll();
        assert_eq!(meta.previous_page, Some(Value::from(2)));
        assert_eq!(meta.next_page, None, "has_more = false must suppress next");
    }

    #[test]
    fn simple_paginator_single_page_has_no_neighbours() {
        let (meta, _) = Paginator::new(vec![1], 1, 20, false).into_inertia_scroll();
        assert_eq!(meta.previous_page, None);
        assert_eq!(meta.next_page, None);
    }

    #[test]
    fn cursor_paginator_carries_opaque_cursors_under_the_cursor_page_name() {
        let (meta, _) = CursorPaginator::new(
            vec![1, 2],
            2,
            Some("next-token".to_string()),
            Some("prev-token".to_string()),
        )
        .into_inertia_scroll();
        assert_eq!(meta.page_name, "cursor");
        assert_eq!(meta.next_page, Some(Value::from("next-token")));
        assert_eq!(meta.previous_page, Some(Value::from("prev-token")));
    }

    // The two tests below assert literal expected values rather than
    // comparing `scroll_metadata()`'s output against
    // `into_inertia_scroll()`'s — the latter now calls the former
    // internally (this file's whole refactor), so a self-comparison
    // would pass even if both sides were wrong in the same way (e.g. a
    // `previous_page` that returned `current_page + 1`). Literal values
    // are what actually pins the refactor preserved the pre-refactor
    // field-by-field behavior.

    #[test]
    fn length_aware_paginator_provides_scroll_metadata_matches_into_inertia_scroll() {
        // total=9, per_page=3 -> last_page=3; current_page=2 is neither
        // the first nor the last page, so both neighbours are present.
        let paginator = LengthAwarePaginator::new(vec![4, 5, 6], 9, 3, 2);
        let via_trait = paginator.scroll_metadata();
        assert_eq!(via_trait.page_name, "page");
        assert_eq!(via_trait.previous_page, Some(Value::from(1)));
        assert_eq!(via_trait.next_page, Some(Value::from(3)));
        assert_eq!(via_trait.current_page, Some(Value::from(2)));

        let (via_conversion, data) = paginator.into_inertia_scroll();
        assert_eq!(via_conversion.page_name, "page");
        assert_eq!(via_conversion.previous_page, Some(Value::from(1)));
        assert_eq!(via_conversion.next_page, Some(Value::from(3)));
        assert_eq!(via_conversion.current_page, Some(Value::from(2)));
        assert_eq!(data, vec![4, 5, 6]);
    }

    #[test]
    fn cursor_paginator_provides_scroll_metadata_matches_into_inertia_scroll() {
        let paginator = CursorPaginator::new(
            vec![1, 2],
            2,
            Some("next-token".to_string()),
            Some("prev-token".to_string()),
        );
        let via_trait = paginator.scroll_metadata();
        assert_eq!(via_trait.page_name, "cursor");
        assert_eq!(via_trait.next_page, Some(Value::from("next-token")));
        assert_eq!(via_trait.previous_page, Some(Value::from("prev-token")));
        assert_eq!(via_trait.current_page, None);

        let (via_conversion, data) = paginator.into_inertia_scroll();
        assert_eq!(via_conversion.page_name, "cursor");
        assert_eq!(via_conversion.next_page, Some(Value::from("next-token")));
        assert_eq!(
            via_conversion.previous_page,
            Some(Value::from("prev-token"))
        );
        assert_eq!(via_conversion.current_page, None);
        assert_eq!(data, vec![1, 2]);
    }
}
