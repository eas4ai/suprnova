//! PAR-013, the clauses the page shows: the stack frames recorded where
//! an error first became a `FrameworkError`, or where a panic happened.
//! The application's frames are shown; the frames of the standard
//! library, the async runtime, other dependencies and the framework are
//! collapsed behind a count.
//!
//! # How a test tells "collapsed" from "shown"
//!
//! A collapsed group is a `<details>` element whose `<summary>` states how
//! many frames it holds. A frame outside every `<details>` element is
//! shown. So the tests read the page with every `<details>` element cut
//! out ([`outside_details`]) to see the shown frames, and read the
//! `<details>` elements on their own ([`details_elements`]) to see the
//! collapsed ones.
//!
//! The application in these tests is this test crate (`error`); its
//! frames are named `error::debug_error_page::...`.
//!
//! PAR-013's other clause, that with debug off the framework records no
//! frames, has no public observation point and is not tested here.

use sea_orm::ConnectionTrait;
use serial_test::serial;

use suprnova::testing::TestDatabase;
use suprnova::{DB, FrameworkError, HttpResponse, Model, Request, Response, Router};

use super::{BROWSER, Reply, assert_debug_page, debug_mode, get, ledger_routes};

/// The table `show_ledger_rows` queries. No migration creates it.
const MISSING_TABLE: &str = "ledger_rows_never_migrated";

/// A handler whose `?` turns the database's error into a
/// `FrameworkError`: the query names a table that does not exist.
///
/// It returns `Result<_, FrameworkError>`, the shape the dogfood app's
/// controllers use, and the route wraps it into a `Response`.
async fn show_ledger_rows(_req: Request) -> Result<HttpResponse, FrameworkError> {
    let connection = DB::connection()?;
    connection
        .inner()
        .execute_unprepared(&format!("SELECT * FROM {MISSING_TABLE}"))
        .await?;
    Ok(HttpResponse::text("rows"))
}

fn database_routes() -> Router {
    Router::new()
        .get("/ledger-rows", |req: Request| async move {
            show_ledger_rows(req).await.map_err(HttpResponse::from)
        })
        .into()
}

/// A model over a table no migration creates, so every query fails
/// inside the framework's Eloquent builder.
#[suprnova::model(table = "ledger_entries_never_migrated")]
pub struct LedgerEntry {
    pub id: i64,
    pub memo: String,
}

/// A handler whose error the framework creates: the Eloquent builder
/// turns the database's error into a `FrameworkError`, and the handler
/// only propagates it.
async fn list_ledger_entries(_req: Request) -> Response {
    let entries = LedgerEntry::query().get().await?;
    HttpResponse::text(entries.len().to_string()).ok()
}

/// A handler that answers with `abort_if`, whose own frame sits between
/// the error's constructor and the handler.
async fn close_ledger(_req: Request) -> Response {
    suprnova::abort_if(true, 500, "the ledger is closed for the night")?;
    HttpResponse::text("open").ok()
}

/// A handler that calls a dependency function that panics: indexing a
/// JSON number by key panics inside `serde_json`.
async fn stamp_ledger_total(_req: Request) -> Response {
    let mut total = serde_json::json!(42);
    total["currency"] = serde_json::json!("EUR");
    HttpResponse::text(total.to_string()).ok()
}

fn framework_created_routes() -> Router {
    Router::new()
        .get("/ledger-entries", list_ledger_entries)
        .get("/close-ledger", close_ledger)
        .get("/stamp-ledger-total", stamp_ledger_total)
        .into()
}

/// The page's HTML with every `<details>` element cut out: what is shown
/// without opening a collapsed group.
fn outside_details(html: &str) -> String {
    let mut shown = String::with_capacity(html.len());
    let mut depth = 0usize;
    let mut at = 0usize;
    while at < html.len() {
        if let Some(len) = tag_at(html, at, "<details") {
            if depth == 0 {
                shown.push(' ');
            }
            depth += 1;
            at += len;
        } else if let Some(len) = tag_at(html, at, "</details") {
            depth = depth.saturating_sub(1);
            at += len;
        } else {
            let ch = html[at..].chars().next().expect("at is a char boundary");
            if depth == 0 {
                shown.push(ch);
            }
            at += ch.len_utf8();
        }
    }
    shown
}

/// Each outermost `<details>` element of the page, start tag to end tag.
fn details_elements(html: &str) -> Vec<&str> {
    let mut elements = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut at = 0usize;
    while at < html.len() {
        if let Some(len) = tag_at(html, at, "<details") {
            if depth == 0 {
                start = at;
            }
            depth += 1;
            at += len;
        } else if let Some(len) = tag_at(html, at, "</details") {
            at += len;
            if depth == 1 {
                elements.push(&html[start..at]);
            }
            depth = depth.saturating_sub(1);
        } else {
            at += html[at..].chars().next().map_or(1, char::len_utf8);
        }
    }
    elements
}

/// When a tag named by `open` (`"<details"` or `"</details"`) starts at
/// `at`, the length of the whole tag, through its `>`.
fn tag_at(html: &str, at: usize, open: &str) -> Option<usize> {
    let rest = html.get(at..)?;
    let head = rest.get(..open.len())?;
    if !head.eq_ignore_ascii_case(open) {
        return None;
    }
    let after = rest[open.len()..].chars().next()?;
    if !(after == '>' || after.is_ascii_whitespace()) {
        return None;
    }
    rest.find('>').map(|end| end + 1)
}

/// The text of the `<summary>` of a `<details>` element.
fn summary_of(element: &str) -> &str {
    let lower = element.to_ascii_lowercase();
    let Some(open) = lower.find("<summary") else {
        return "";
    };
    let Some(close) = lower[open..].find("</summary") else {
        return "";
    };
    &element[open..open + close]
}

/// Fail unless `function` is named among the frames the page shows.
fn assert_shown_frame(reply: &Reply, function: &str) {
    assert_debug_page(reply, 500);
    let shown = outside_details(&reply.body);
    assert!(
        shown.contains(function),
        "the page must show a frame naming `{function}` outside every collapsed \
         `<details>` group; page:\n{}",
        reply.body
    );
}

#[tokio::test]
#[serial]
async fn a_handler_whose_question_mark_turns_a_database_error_into_a_framework_error_is_a_shown_frame()
 {
    let _debug = debug_mode(true, &[]).await;
    let _database = TestDatabase::sqlite_memory()
        .await
        .expect("an in-memory SQLite database");

    let reply = get(database_routes(), "/ledger-rows", BROWSER).await;

    assert_shown_frame(&reply, "show_ledger_rows");
}

#[tokio::test]
#[serial]
async fn a_panicking_function_is_a_shown_frame() {
    let _debug = debug_mode(true, &[]).await;

    let reply = get(ledger_routes(), "/ledger-index", BROWSER).await;

    assert_shown_frame(&reply, "read_ledger_index_page");
}

#[tokio::test]
#[serial]
async fn a_handler_whose_error_the_eloquent_builder_creates_is_a_shown_frame() {
    let _debug = debug_mode(true, &[]).await;
    let _database = TestDatabase::sqlite_memory()
        .await
        .expect("an in-memory SQLite database");

    let reply = get(framework_created_routes(), "/ledger-entries", BROWSER).await;

    assert_shown_frame(&reply, "list_ledger_entries");
}

#[tokio::test]
#[serial]
async fn a_handler_that_answers_with_abort_if_is_a_shown_frame() {
    let _debug = debug_mode(true, &[]).await;

    let reply = get(framework_created_routes(), "/close-ledger", BROWSER).await;

    assert_shown_frame(&reply, "close_ledger");
}

#[tokio::test]
#[serial]
async fn a_handler_whose_dependency_panics_is_a_shown_frame() {
    let _debug = debug_mode(true, &[]).await;

    let reply = get(framework_created_routes(), "/stamp-ledger-total", BROWSER).await;

    assert_shown_frame(&reply, "stamp_ledger_total");
    let shown = outside_details(&reply.body);
    assert!(
        !shown.contains("serde_json::"),
        "the dependency's own frames are collapsed; page:\n{}",
        reply.body
    );
}

#[tokio::test]
#[serial]
async fn std_tokio_and_hyper_frames_are_shown_only_inside_collapsed_groups() {
    let _debug = debug_mode(true, &[]).await;
    let _database = TestDatabase::sqlite_memory()
        .await
        .expect("an in-memory SQLite database");

    let cases = [
        (
            "a `?`-converted database error",
            get(database_routes(), "/ledger-rows", BROWSER).await,
        ),
        (
            "a panic",
            get(ledger_routes(), "/ledger-index", BROWSER).await,
        ),
    ];
    for (case, reply) in cases {
        assert_debug_page(&reply, 500);

        let shown = outside_details(&reply.body);
        for prefix in ["std::", "core::", "alloc::", "tokio::", "hyper::"] {
            assert!(
                !shown.contains(prefix),
                "for {case}, a `{prefix}` frame is shown outside every collapsed \
                 `<details>` group; page:\n{}",
                reply.body
            );
        }

        // The groups are not empty: the runtime's and hyper's frames are
        // on the stack of every request, so they must be in a group.
        let groups = details_elements(&reply.body);
        for prefix in ["tokio::", "hyper::"] {
            assert!(
                groups.iter().any(|group| group.contains(prefix)),
                "for {case}, the `{prefix}` frames must be collapsed into a `<details>` \
                 group; page:\n{}",
                reply.body
            );
        }
        let counted = groups.iter().any(|group| {
            (group.contains("tokio::") || group.contains("std::") || group.contains("core::"))
                && summary_of(group).chars().any(|c| c.is_ascii_digit())
        });
        assert!(
            counted,
            "for {case}, a collapsed group must state its frame count in its `<summary>`; \
             page:\n{}",
            reply.body
        );
    }
}
