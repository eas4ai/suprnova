//! The JSON of the three paginators is the JSON of Laravel's, so a front
//! end that was written for Laravel's reads it.

use serde_json::{Value, json};
use suprnova::{CursorPaginator, LengthAwarePaginator, Paginator};

fn rows(from: u32, to: u32) -> Vec<u32> {
    (from..=to).collect()
}

#[test]
fn the_paginator_with_a_total_has_the_fields_of_laravel() {
    let page = LengthAwarePaginator::new(rows(11, 20), 25, 10, 2).with_path("/api/users");

    assert_eq!(
        serde_json::to_value(&page).unwrap(),
        json!({
            "current_page": 2,
            "data": [11, 12, 13, 14, 15, 16, 17, 18, 19, 20],
            "first_page_url": "/api/users?page=1",
            "from": 11,
            "last_page": 3,
            "last_page_url": "/api/users?page=3",
            "links": [
                {"url": "/api/users?page=1", "label": "&laquo; Previous", "page": 1, "active": false},
                {"url": "/api/users?page=1", "label": "1", "page": 1, "active": false},
                {"url": "/api/users?page=2", "label": "2", "page": 2, "active": true},
                {"url": "/api/users?page=3", "label": "3", "page": 3, "active": false},
                {"url": "/api/users?page=3", "label": "Next &raquo;", "page": 3, "active": false}
            ],
            "next_page_url": "/api/users?page=3",
            "path": "/api/users",
            "per_page": 10,
            "prev_page_url": "/api/users?page=1",
            "to": 20,
            "total": 25
        })
    );
}

#[test]
fn the_ends_of_the_range_have_no_url_to_step_to() {
    let first = LengthAwarePaginator::new(rows(1, 10), 25, 10, 1).with_path("/api/users");
    let json = serde_json::to_value(&first).unwrap();
    assert_eq!(json["prev_page_url"], Value::Null);
    assert_eq!(json["next_page_url"], "/api/users?page=2");
    assert_eq!(
        json["links"][0],
        json!({"url": null, "label": "&laquo; Previous", "page": null, "active": false})
    );

    let last = LengthAwarePaginator::new(rows(21, 25), 25, 10, 3).with_path("/api/users");
    let json = serde_json::to_value(&last).unwrap();
    assert_eq!(json["next_page_url"], Value::Null);
    assert_eq!(json["prev_page_url"], "/api/users?page=2");
    assert_eq!(
        json["links"][4],
        json!({"url": null, "label": "Next &raquo;", "page": null, "active": false})
    );
}

#[test]
fn a_long_range_leaves_pages_out_and_says_so_with_three_dots() {
    let page = LengthAwarePaginator::new(rows(1, 10), 500, 10, 25).with_path("/posts");
    let json = serde_json::to_value(&page).unwrap();

    let labels: Vec<&str> = json["links"]
        .as_array()
        .expect("an array of links")
        .iter()
        .map(|link| link["label"].as_str().expect("a label"))
        .collect();
    assert_eq!(
        labels,
        [
            "&laquo; Previous",
            "1",
            "2",
            "...",
            "22",
            "23",
            "24",
            "25",
            "26",
            "27",
            "28",
            "...",
            "49",
            "50",
            "Next &raquo;"
        ]
    );
    assert_eq!(
        json["links"][3],
        json!({"url": null, "label": "...", "active": false}),
        "the three dots lead nowhere, and have no page"
    );
    assert_eq!(json["links"][7]["active"], true);
    assert_eq!(json["links"][7]["url"], "/posts?page=25");
}

#[test]
fn the_filters_of_a_listing_are_a_part_of_the_path() {
    let page = LengthAwarePaginator::new(rows(1, 10), 25, 10, 1)
        .with_path("/api/users?role=admin")
        .with_page_name("users_page");
    let json = serde_json::to_value(&page).unwrap();

    assert_eq!(json["next_page_url"], "/api/users?role=admin&users_page=2");
    assert_eq!(json["last_page_url"], "/api/users?role=admin&users_page=3");
    assert!(json.get("page_name").is_none());
}

#[test]
fn a_paginator_with_no_rows_has_the_urls_of_page_one() {
    let empty: LengthAwarePaginator<u32> = LengthAwarePaginator::new(Vec::new(), 0, 10, 1);
    let json = serde_json::to_value(&empty).unwrap();

    assert_eq!(json["first_page_url"], "?page=1");
    assert_eq!(json["last_page_url"], "?page=1");
    assert_eq!(json["from"], Value::Null);
    assert_eq!(json["to"], Value::Null);
    assert_eq!(json["last_page"], 0);
    assert_eq!(
        json["links"].as_array().map(Vec::len),
        Some(2),
        "the link before and the link behind, and no page between them"
    );
    assert!(
        json.get("path").is_none(),
        "a path that is not set is left out"
    );
}

#[test]
fn the_simple_paginator_has_the_fields_of_laravel_and_has_more() {
    let page = Paginator::new(rows(11, 20), 2, 10, true).with_path("/api/users");

    assert_eq!(
        serde_json::to_value(&page).unwrap(),
        json!({
            "current_page": 2,
            "current_page_url": "/api/users?page=2",
            "data": [11, 12, 13, 14, 15, 16, 17, 18, 19, 20],
            "first_page_url": "/api/users?page=1",
            "from": 11,
            "has_more": true,
            "next_page_url": "/api/users?page=3",
            "path": "/api/users",
            "per_page": 10,
            "prev_page_url": "/api/users?page=1",
            "to": 20
        })
    );
}

#[test]
fn the_simple_paginator_at_its_ends() {
    let only = Paginator::new(rows(1, 4), 1, 10, false);
    let json = serde_json::to_value(&only).unwrap();
    assert_eq!(json["next_page_url"], Value::Null);
    assert_eq!(json["prev_page_url"], Value::Null);
    assert_eq!(json["from"], 1);
    assert_eq!(json["to"], 4);
    assert_eq!(json["current_page_url"], "?page=1");

    let past_the_end: Paginator<u32> = Paginator::new(Vec::new(), 9, 10, false);
    let json = serde_json::to_value(&past_the_end).unwrap();
    assert_eq!(json["from"], Value::Null);
    assert_eq!(json["to"], Value::Null);
    assert_eq!(json["prev_page_url"], "?page=8");
}

#[test]
fn the_cursor_paginator_has_the_urls_beside_its_cursors() {
    let page = CursorPaginator::new(
        rows(1, 3),
        3,
        Some("NEXT".to_owned()),
        Some("PREV".to_owned()),
    )
    .with_path("/api/items");

    assert_eq!(
        serde_json::to_value(&page).unwrap(),
        json!({
            "data": [1, 2, 3],
            "path": "/api/items",
            "per_page": 3,
            "next_cursor": "NEXT",
            "next_page_url": "/api/items?cursor=NEXT",
            "prev_cursor": "PREV",
            "prev_page_url": "/api/items?cursor=PREV"
        })
    );
}

#[test]
fn a_cursor_that_is_not_there_has_no_url() {
    let first = CursorPaginator::new(rows(1, 3), 3, Some("NEXT".to_owned()), None)
        .with_path("/api/items?sort=name")
        .with_cursor_name("after");
    let json = serde_json::to_value(&first).unwrap();

    assert_eq!(json["prev_cursor"], Value::Null);
    assert_eq!(json["prev_page_url"], Value::Null);
    assert_eq!(json["next_page_url"], "/api/items?sort=name&after=NEXT");
    assert!(json.get("cursor_name").is_none());
}

/// A handler gives the URL of the request as the path. That URL has the
/// page it was asked for in it.
#[test]
fn the_url_of_the_current_request_is_a_path() {
    let page = LengthAwarePaginator::new(vec![1, 2], 6, 2, 2)
        .with_path("/api/users?role=admin&page=2#list");
    let json = serde_json::to_value(&page).expect("the paginator is JSON");

    assert_eq!(json["first_page_url"], "/api/users?role=admin&page=1#list");
    assert_eq!(json["prev_page_url"], "/api/users?role=admin&page=1#list");
    assert_eq!(json["next_page_url"], "/api/users?role=admin&page=3#list");
    assert_eq!(json["last_page_url"], "/api/users?role=admin&page=3#list");
    assert_eq!(
        json["links"][2]["url"], "/api/users?role=admin&page=2#list",
        "the link of the current page has one page parameter"
    );
}
