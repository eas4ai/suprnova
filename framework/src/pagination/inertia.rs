//! Bridge from `LengthAwarePaginator` / `CursorPaginator` to Inertia's
//! `ScrollMetadata` — the protocol for infinite-scroll props.

use serde_json::Value;

use crate::inertia::ScrollMetadata;

use super::{CursorPaginator, LengthAwarePaginator, Paginator};

/// Convert a paginator into an Inertia scroll prop: the metadata + the
/// row vec, which the caller wires onto an [`InertiaResponse`](crate::inertia::InertiaResponse) under a
/// chosen key.
pub trait IntoInertiaScroll<T> {
    /// Split this paginator into its Inertia scroll metadata and the
    /// underlying data rows.
    fn into_inertia_scroll(self) -> (ScrollMetadata, Vec<T>);
}

impl<T> IntoInertiaScroll<T> for LengthAwarePaginator<T> {
    fn into_inertia_scroll(self) -> (ScrollMetadata, Vec<T>) {
        let mut meta = ScrollMetadata::new("page");
        meta.current_page = Some(Value::from(self.current_page as i64));
        if self.current_page > 1 {
            meta.previous_page = Some(Value::from((self.current_page - 1) as i64));
        }
        if self.has_more_pages() {
            meta.next_page = Some(Value::from((self.current_page + 1) as i64));
        }
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
impl<T> IntoInertiaScroll<T> for Paginator<T> {
    fn into_inertia_scroll(self) -> (ScrollMetadata, Vec<T>) {
        let mut meta = ScrollMetadata::new("page");
        meta.current_page = Some(Value::from(self.current_page as i64));
        if self.current_page > 1 {
            meta.previous_page = Some(Value::from((self.current_page - 1) as i64));
        }
        if self.has_more {
            meta.next_page = Some(Value::from((self.current_page + 1) as i64));
        }
        (meta, self.data)
    }
}

impl<T> IntoInertiaScroll<T> for CursorPaginator<T> {
    fn into_inertia_scroll(self) -> (ScrollMetadata, Vec<T>) {
        let mut meta = ScrollMetadata::new("cursor");
        meta.next_page = self.next_cursor.map(Value::String);
        meta.previous_page = self.prev_cursor.map(Value::String);
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
}
