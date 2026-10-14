//! The links of a paginator with a total: the row of page numbers a
//! pagination component draws.
//!
//! The JSON of a paginator is read by front ends that were written for
//! Laravel's: the pagination components of the Inertia starter kits, and
//! clients of an API. They draw `links` as it comes, so the entries, the
//! labels and the window of pages are the ones of Laravel.

use serde::Serialize;
use serde::ser::SerializeMap;

/// The pages that are shown on each side of the current one when the
/// paginator has too many pages to show them all. Laravel's `onEachSide`.
pub(crate) const ON_EACH_SIDE: u64 = 3;

/// The label of the link to the page before. It is Laravel's
/// `pagination.previous`, entity included: a front end puts it into the
/// page as markup.
pub(crate) const PREVIOUS_LABEL: &str = "&laquo; Previous";

/// The label of the link to the page behind. Laravel's `pagination.next`.
pub(crate) const NEXT_LABEL: &str = "Next &raquo;";

/// One entry of the `links` of a
/// [`LengthAwarePaginator`](super::LengthAwarePaginator): the link to the
/// page before, the link to a page by its number, the three dots where
/// pages are left out, or the link to the page behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageLink {
    /// Where the link leads, and `None` for a link that leads nowhere:
    /// the three dots, the page before the first one, the page behind
    /// the last one.
    pub url: Option<String>,
    /// What the link shows: the number of the page, `...`, or the label
    /// of the link to the page before or behind.
    pub label: String,
    /// The page the link leads to.
    pub page: Option<u64>,
    /// Whether the link is the one of the current page.
    pub active: bool,
    /// The three dots have no `page` in the JSON, where the other
    /// entries have one that may be null.
    dots: bool,
}

impl PageLink {
    pub(crate) fn to_page(page: u64, url: String, active: bool) -> Self {
        Self {
            url: Some(url),
            label: page.to_string(),
            page: Some(page),
            active,
            dots: false,
        }
    }

    pub(crate) fn dots() -> Self {
        Self {
            url: None,
            label: "...".to_owned(),
            page: None,
            active: false,
            dots: true,
        }
    }

    pub(crate) fn step(label: &str, page: Option<u64>, url: Option<String>) -> Self {
        Self {
            url,
            label: label.to_owned(),
            page,
            active: false,
            dots: false,
        }
    }
}

impl Serialize for PageLink {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(if self.dots { 3 } else { 4 }))?;
        map.serialize_entry("url", &self.url)?;
        map.serialize_entry("label", &self.label)?;
        if !self.dots {
            map.serialize_entry("page", &self.page)?;
        }
        map.serialize_entry("active", &self.active)?;
        map.end()
    }
}

/// The pages the row shows, in order, with `None` where pages are left
/// out. It is the window of Laravel's `UrlWindow`.
///
/// Up to `on_each_side * 2 + 7` pages are all shown. With more, the row
/// has the first two and the last two pages, and between them the pages
/// around the current one; near an end of the range, the pages of that
/// end stand for both.
pub(crate) fn page_window(current: u64, last: u64, on_each_side: u64) -> Vec<Option<u64>> {
    let pages = |from: u64, to: u64| (from..=to).map(Some);
    if last < on_each_side * 2 + 8 {
        return pages(1, last).collect();
    }
    let window = on_each_side + 4;
    let mut row: Vec<Option<u64>> = Vec::new();
    if current <= window {
        row.extend(pages(1, window + on_each_side));
        row.push(None);
        row.extend(pages(last - 1, last));
    } else if current > last - window {
        row.extend(pages(1, 2));
        row.push(None);
        row.extend(pages(last - (window + on_each_side - 1), last));
    } else {
        row.extend(pages(1, 2));
        row.push(None);
        row.extend(pages(current - on_each_side, current + on_each_side));
        row.push(None);
        row.extend(pages(last - 1, last));
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The row as a text: the numbers, and `..` where pages are left out.
    fn row(current: u64, last: u64) -> String {
        page_window(current, last, ON_EACH_SIDE)
            .into_iter()
            .map(|page| page.map_or("..".to_owned(), |page| page.to_string()))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn a_short_range_shows_every_page() {
        assert_eq!(row(1, 0), "");
        assert_eq!(row(1, 1), "1");
        assert_eq!(row(2, 3), "1 2 3");
        assert_eq!(row(7, 13), "1 2 3 4 5 6 7 8 9 10 11 12 13");
    }

    #[test]
    fn near_the_beginning_the_first_pages_stand_for_the_slider() {
        assert_eq!(row(1, 14), "1 2 3 4 5 6 7 8 9 10 .. 13 14");
        assert_eq!(row(7, 14), "1 2 3 4 5 6 7 8 9 10 .. 13 14");
        assert_eq!(row(7, 50), "1 2 3 4 5 6 7 8 9 10 .. 49 50");
    }

    #[test]
    fn in_the_middle_the_pages_around_the_current_one_are_shown() {
        assert_eq!(row(8, 50), "1 2 .. 5 6 7 8 9 10 11 .. 49 50");
        assert_eq!(row(25, 50), "1 2 .. 22 23 24 25 26 27 28 .. 49 50");
        assert_eq!(row(43, 50), "1 2 .. 40 41 42 43 44 45 46 .. 49 50");
    }

    #[test]
    fn near_the_end_the_last_pages_stand_for_the_slider() {
        assert_eq!(row(44, 50), "1 2 .. 41 42 43 44 45 46 47 48 49 50");
        assert_eq!(row(50, 50), "1 2 .. 41 42 43 44 45 46 47 48 49 50");
        assert_eq!(row(8, 14), "1 2 .. 5 6 7 8 9 10 11 12 13 14");
    }

    #[test]
    fn a_page_behind_the_last_one_is_near_the_end() {
        assert_eq!(row(99, 50), "1 2 .. 41 42 43 44 45 46 47 48 49 50");
    }

    #[test]
    fn the_three_dots_have_no_page_in_the_json() {
        let dots = serde_json::to_value(PageLink::dots()).unwrap();
        assert_eq!(
            dots,
            serde_json::json!({"url": null, "label": "...", "active": false})
        );

        let step = serde_json::to_value(PageLink::step(PREVIOUS_LABEL, None, None)).unwrap();
        assert_eq!(
            step,
            serde_json::json!({
                "url": null,
                "label": "&laquo; Previous",
                "page": null,
                "active": false
            }),
            "the link to a page there is not has a page, and it is null"
        );
    }
}
