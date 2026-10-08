//! PAR-047: partial reloads read `X-Inertia-Partial-Data` and
//! `X-Inertia-Partial-Except` as Laravel's `PropsResolver` does.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use suprnova::{InertiaResponse, Prop};

use super::support::{MockReq, counted, page_of};

#[tokio::test]
async fn inp_partial_data_drops_empty_segments_and_does_not_trim() {
    // `explode(',')` then `array_filter`: "a,, b" names "a" and " b".
    let req = MockReq::new("/p")
        .partial("P")
        .header("X-Inertia-Partial-Data", "a,, b");
    let resp = InertiaResponse::new("P")
        .with("a", 1)
        .with("b", 2)
        .with(" b", 3)
        .resolve(&req)
        .await
        .unwrap();
    let props = page_of(resp).await["props"].clone();
    let props = props.as_object().unwrap();
    assert!(
        props.contains_key("a"),
        "the empty segment must not drop `a`"
    );
    assert!(
        props.contains_key(" b"),
        "the entry is ` b`, untrimmed; got {props:?}"
    );
    assert!(
        !props.contains_key("b"),
        "a trimmed `b` must not be selected; got {props:?}"
    );
}

#[tokio::test]
async fn inp_an_empty_partial_data_header_counts_as_absent() {
    // Laravel's `parseHeader` returns null for an empty header, so the
    // reload is filtered by `except` alone. Suprnova dropped every prop.
    for empty in ["", ",,"] {
        let req = MockReq::new("/p")
            .partial("P")
            .header("X-Inertia-Partial-Data", empty)
            .header("X-Inertia-Partial-Except", empty);
        let resp = InertiaResponse::new("P")
            .with("a", 1)
            .with("b", 2)
            .resolve(&req)
            .await
            .unwrap();
        let page = page_of(resp).await;
        assert_eq!(page["props"]["a"], 1, "header {empty:?}: got {page}");
        assert_eq!(page["props"]["b"], 2, "header {empty:?}: got {page}");
    }
}

// ---- dotted entries narrow literal values only ----

#[tokio::test]
async fn inp_a_dotted_only_ships_a_resolver_backed_prop_whole() {
    let req = MockReq::new("/users")
        .partial("Users")
        .header("X-Inertia-Partial-Data", "users.name");
    let resp = InertiaResponse::new("Users")
        .prop(
            "users",
            Prop::lazy(|| async { json!({"name": "Ada", "email": "ada@example.com"}) }),
        )
        .resolve(&req)
        .await
        .unwrap();
    assert_eq!(
        page_of(resp).await["props"]["users"],
        json!({"name": "Ada", "email": "ada@example.com"}),
        "a value that came from a resolver is not walked"
    );
}

#[tokio::test]
async fn inp_a_dotted_only_ships_a_prop_object_whole() {
    // `optional`, `defer`, `merge` and `once` props are Laravel prop
    // objects, resolved rather than walked.
    let req = MockReq::new("/p")
        .partial("P")
        .header("X-Inertia-Partial-Data", "o.a,d.a,m.a,n.a");
    let whole = json!({"a": 1, "b": 2});
    let resp = InertiaResponse::new("P")
        .prop("o", Prop::eager(whole.clone()).optional())
        .prop("d", Prop::eager(whole.clone()).defer())
        .prop("m", Prop::eager(whole.clone()).merge())
        .prop("n", Prop::eager(whole.clone()).once())
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;
    for key in ["o", "d", "m", "n"] {
        assert_eq!(page["props"][key], whole, "{key}: got {page}");
    }
}

#[tokio::test]
async fn inp_a_dotted_only_still_narrows_a_literal_value() {
    let req = MockReq::new("/users")
        .partial("Users")
        .header("X-Inertia-Partial-Data", "user.name");
    let resp = InertiaResponse::new("Users")
        .with("user", json!({"name": "Ada", "email": "ada@example.com"}))
        .resolve(&req)
        .await
        .unwrap();
    assert_eq!(page_of(resp).await["props"]["user"], json!({"name": "Ada"}));
}

#[tokio::test]
async fn inp_a_dotted_prop_keys_plain_resolver_narrows_like_a_literal() {
    // Laravel's `unpackDotProps` calls a dotted key's closure before the
    // walk, so its value is walked like any literal.
    let req = MockReq::new("/p")
        .partial("P")
        .header("X-Inertia-Partial-Data", "auth.user.name");
    let resp = InertiaResponse::new("P")
        .prop(
            "auth.user",
            Prop::lazy(|| async { json!({"name": "Ada", "email": "ada@example.com"}) }),
        )
        .resolve(&req)
        .await
        .unwrap();
    assert_eq!(
        page_of(resp).await["props"]["auth"],
        json!({"user": {"name": "Ada"}})
    );
}

#[tokio::test]
async fn inp_a_dotted_only_that_resolves_to_nothing_yields_an_empty_array() {
    let req = MockReq::new("/p")
        .partial("P")
        .header("X-Inertia-Partial-Data", "missing.path,nested.inner.gone");
    let resp = InertiaResponse::new("P")
        .with("missing", json!({"other": 1}))
        .with("nested", json!({"inner": {"kept": 1}, "sibling": 2}))
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;
    assert_eq!(page["props"]["missing"], json!([]), "got {page}");
    assert_eq!(page["props"]["nested"], json!({"inner": []}), "got {page}");
}

#[tokio::test]
async fn inp_a_dotted_only_walks_into_lists_and_keeps_scalars_on_the_path() {
    // Laravel walks a literal list by index, and a scalar the path reaches
    // ships as it is rather than being dropped.
    let req = MockReq::new("/p").partial("P").header(
        "X-Inertia-Partial-Data",
        "rows.0.id,tail.1,config.level.nested",
    );
    let resp = InertiaResponse::new("P")
        .with(
            "rows",
            json!([{"id": 1, "name": "a"}, {"id": 2, "name": "b"}]),
        )
        .with("tail", json!(["a", "b"]))
        .with("config", json!({"theme": "dark", "level": 3}))
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;
    assert_eq!(page["props"]["rows"], json!([{"id": 1}]), "got {page}");
    // PHP keeps the index, and a list that no longer starts at 0 encodes
    // as an object.
    assert_eq!(page["props"]["tail"], json!({"1": "b"}), "got {page}");
    assert_eq!(page["props"]["config"], json!({"level": 3}), "got {page}");
}

// ---- optional and defer on a partial reload ----

#[tokio::test]
async fn inp_an_except_only_reload_resolves_optional_and_deferred_props() {
    // Laravel resolves `IgnoreFirstLoad` props on any partial reload whose
    // lists they pass; Suprnova required an `only` entry.
    let calls = Arc::new(AtomicUsize::new(0));
    let req = MockReq::new("/p")
        .partial("P")
        .header("X-Inertia-Partial-Except", "x");
    let resp = InertiaResponse::new("P")
        .prop("o", counted(calls.clone(), json!("optional")).optional())
        .prop("d", counted(calls.clone(), json!("deferred")).defer())
        .prop("x", counted(calls.clone(), json!("excepted")).optional())
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;
    assert_eq!(page["props"]["o"], "optional", "got {page}");
    assert_eq!(page["props"]["d"], "deferred", "got {page}");
    assert!(page["props"].get("x").is_none(), "got {page}");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "the excepted resolver must not run"
    );
    assert!(page.get("deferredProps").is_none());
}

#[tokio::test]
async fn inp_an_only_list_still_withholds_optional_props_it_does_not_name() {
    let calls = Arc::new(AtomicUsize::new(0));
    let req = MockReq::new("/p")
        .partial("P")
        .header("X-Inertia-Partial-Data", "a");
    let resp = InertiaResponse::new("P")
        .with("a", 1)
        .prop("o", counted(calls.clone(), json!("optional")).optional())
        .prop("d", counted(calls.clone(), json!("deferred")).defer())
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;
    assert!(page["props"].get("o").is_none() && page["props"].get("d").is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

// ---- merge and once instructions on a partial reload ----

fn names(page: &serde_json::Value, field: &str) -> Vec<String> {
    page.get(field)
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn inp_a_deeper_only_entry_ships_a_merge_prop_whole_without_the_instruction() {
    let req = MockReq::new("/feed")
        .partial("Feed")
        .header("X-Inertia-Partial-Data", "items.data");
    let resp = InertiaResponse::new("Feed")
        .merge("items", json!({"data": [{"id": 1}], "meta": {"total": 1}}))
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;
    assert_eq!(
        page["props"]["items"],
        json!({"data": [{"id": 1}], "meta": {"total": 1}}),
        "a merge prop is a prop object, so it ships whole"
    );
    assert!(
        names(&page, "mergeProps").is_empty(),
        "an only entry below the prop carries no merge instruction; got {page}"
    );
}

#[tokio::test]
async fn inp_an_only_entry_at_or_above_the_prop_keeps_the_merge_instruction() {
    for only in ["items", "items,other"] {
        let req = MockReq::new("/feed")
            .partial("Feed")
            .header("X-Inertia-Partial-Data", only);
        let resp = InertiaResponse::new("Feed")
            .merge("items", json!([{"id": 1}]))
            .resolve(&req)
            .await
            .unwrap();
        let page = page_of(resp).await;
        assert_eq!(names(&page, "mergeProps"), vec!["items".to_string()]);
    }
    // A dotted prop key under an ancestor entry.
    let req = MockReq::new("/feed")
        .partial("Feed")
        .header("X-Inertia-Partial-Data", "feed");
    let resp = InertiaResponse::new("Feed")
        .merge("feed.items", json!([{"id": 1}]))
        .resolve(&req)
        .await
        .unwrap();
    assert_eq!(
        names(&page_of(resp).await, "mergeProps"),
        vec!["feed.items".to_string()]
    );
}

#[tokio::test]
async fn inp_a_deeper_only_entry_emits_no_once_instruction() {
    let req = MockReq::new("/stats")
        .partial("Stats")
        .header("X-Inertia-Partial-Data", "stats.count");
    let resp = InertiaResponse::new("Stats")
        .prop(
            "stats",
            Prop::lazy(|| async { json!({"count": 1, "sum": 2}) }).once(),
        )
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;
    assert_eq!(page["props"]["stats"], json!({"count": 1, "sum": 2}));
    assert!(page.get("onceProps").is_none(), "got {page}");
}

#[tokio::test]
async fn inp_a_deeper_only_entry_keeps_a_scroll_cursor_but_no_merge_instruction() {
    // Laravel collects a scroll prop's cursor for every scroll prop it
    // resolves, and its merge instruction through the partial metadata
    // check.
    let req = MockReq::new("/posts")
        .partial("Posts")
        .header("X-Inertia-Partial-Data", "posts.0");
    let resp = InertiaResponse::new("Posts")
        .scroll(
            "posts",
            suprnova::ScrollMetadata::new("page").current(1).next(2),
            json!([{"id": 1}]),
        )
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;
    assert_eq!(page["props"]["posts"], json!([{"id": 1}]));
    assert_eq!(
        page["scrollProps"]["posts"]["pageName"], "page",
        "got {page}"
    );
    assert!(names(&page, "mergeProps").is_empty(), "got {page}");
}

#[tokio::test]
async fn inp_a_held_once_prop_keeps_its_once_entry_and_no_merge_entry() {
    let calls = Arc::new(AtomicUsize::new(0));
    let req = MockReq::new("/billing")
        .inertia()
        .header("X-Inertia-Except-Once-Props", "plans");
    let resp = InertiaResponse::new("Billing")
        .prop(
            "plans",
            counted(calls.clone(), json!([{"id": 1}])).merge().once(),
        )
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(page["onceProps"]["plans"]["prop"], "plans");
    assert!(
        names(&page, "mergeProps").is_empty(),
        "a held once prop carries only its once instruction; got {page}"
    );
}

#[tokio::test]
async fn inp_a_held_deferred_once_prop_keeps_its_merge_entry() {
    // Laravel's `IgnoreFirstLoad` branch runs before the "already loaded"
    // one, so a deferred prop the client holds keeps its merge entry.
    let req = MockReq::new("/billing")
        .inertia()
        .header("X-Inertia-Except-Once-Props", "plans");
    let resp = InertiaResponse::new("Billing")
        .prop(
            "plans",
            Prop::lazy(|| async { json!([{"id": 1}]) })
                .defer()
                .merge()
                .once(),
        )
        .resolve(&req)
        .await
        .unwrap();
    let page = page_of(resp).await;
    assert_eq!(names(&page, "mergeProps"), vec!["plans".to_string()]);
    assert_eq!(page["onceProps"]["plans"]["prop"], "plans");
}

// ---- the other Inertia list headers ----

#[tokio::test]
async fn inp_x_inertia_reset_is_parsed_without_trimming() {
    // "other, items" names "other" and " items", so `items` is not reset
    // and keeps its merge instruction.
    let page_for = |reset: &'static str| async move {
        let req = MockReq::new("/feed")
            .inertia()
            .header("X-Inertia-Reset", reset);
        let resp = InertiaResponse::new("Feed")
            .merge("items", json!([{"id": 1}]))
            .resolve(&req)
            .await
            .unwrap();
        page_of(resp).await
    };
    let untrimmed = page_for("other, items").await;
    assert_eq!(
        names(&untrimmed, "mergeProps"),
        vec!["items".to_string()],
        "` items` does not name `items`; got {untrimmed}"
    );
    // Empty segments are dropped, and the exact entry resets the prop.
    let exact = page_for(",items,").await;
    assert!(names(&exact, "mergeProps").is_empty(), "got {exact}");
}

#[tokio::test]
async fn inp_x_inertia_except_once_props_is_parsed_without_trimming() {
    let calls = Arc::new(AtomicUsize::new(0));
    let page_for = |held: &'static str| {
        let calls = calls.clone();
        async move {
            let req = MockReq::new("/billing")
                .inertia()
                .header("X-Inertia-Except-Once-Props", held);
            let resp = InertiaResponse::new("Billing")
                .prop("plans", counted(calls, json!([{"id": 1}])).once())
                .resolve(&req)
                .await
                .unwrap();
            page_of(resp).await
        }
    };
    let untrimmed = page_for("other, plans").await;
    assert_eq!(
        untrimmed["props"]["plans"],
        json!([{"id": 1}]),
        "` plans` does not name `plans`, so the client holds nothing; got {untrimmed}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // Empty segments are dropped, and the exact entry is a held value.
    let exact = page_for(",plans,").await;
    assert!(exact["props"].get("plans").is_none(), "got {exact}");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the held prop's resolver must not run"
    );
}
