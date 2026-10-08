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
