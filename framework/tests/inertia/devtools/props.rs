//! PAR-073: the props of a rendered page, each classified by kind, with
//! where a shared prop was shared and where a render prop was given, deep
//! paths without metadata pruned, and the value the client received.

use indexmap::IndexMap;
use serde_json::{Value, json};
use suprnova::testing::TestContainer;
use suprnova::{
    DeferOptions, FrameworkError, HttpResponse, Inertia, InertiaMiddlewareHooks, InertiaRequestExt,
    InertiaResponse, MiddlewareRegistry, Prop, Request, Router, ScrollMetadata,
};

use super::{client, devtools, entry_of, inertia};

fn router() -> Router {
    Router::new()
        .get("/feed", |req: Request| async move {
            InertiaResponse::new("Feed")
                .with("plain", "x")
                .always("flags", json!({"beta": true}))
                .optional("stats", || async { Ok::<_, FrameworkError>(json!({"count": 3})) })
                .defer_with("comments", DeferOptions::new().group("sidebar"), || async {
                    Ok::<_, FrameworkError>(json!(["first"]))
                })
                .defer_with("report", DeferOptions::new().rescue(), || async {
                    Err::<Value, _>(FrameworkError::internal("the report failed"))
                })
                .merge_prepend("posts", json!([1, 2]))
                .deep_merge("settings", json!({"theme": {"dark": true}}))
                .prop("tags", Prop::eager(json!([{"id": 1}])).merge().match_on("id"))
                .scroll("feed", ScrollMetadata::new("page"), json!({"data": [1]}))
                .once("plans", || async { Ok::<_, FrameworkError>(json!(["free"])) })
                .with("auth.user", json!({"name": "Ada"}))
                .prop("auth.permissions", Prop::eager(json!(["edit"])).always())
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)
        })
        .into()
}

/// The `props` and `propValues` of the entry of a visit to `/feed` with
/// `headers`.
async fn feed(headers: &[(&str, &str)]) -> (Value, Value) {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));
    let mut request = client.get("/feed").inertia();
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = request.send().await;
    response.assert_ok();
    let entry = entry_of(dir.path(), &response);
    (entry["props"].clone(), entry["propValues"].clone())
}

#[tokio::test]
async fn indt_each_kind_of_prop_is_classified() {
    let (props, _) = feed(&[]).await;
    assert_eq!(props["flags"]["inertiaType"], "always");
    assert_eq!(props["posts"]["inertiaType"], "merge");
    assert_eq!(props["posts"]["mergeDirection"], "prepend");
    assert_eq!(props["settings"]["inertiaType"], "merge");
    assert_eq!(props["settings"]["mergeDirection"], "append");
    assert_eq!(props["settings"]["deepMerge"], true);
    assert_eq!(props["tags"]["deepMerge"], true, "a match_on prop deep merges");
    assert_eq!(props["feed"]["inertiaType"], "scroll");
    assert_eq!(props["feed"]["mergeDirection"], "append");
    assert_eq!(props["plans"]["inertiaType"], "once");
    assert_eq!(props["plans"]["once"], true);
    assert_eq!(props["plain"]["inertiaType"], Value::Null);
    assert_eq!(props["plain"]["shared"], false);
    assert_eq!(props["errors"]["inertiaType"], "always");
    assert_eq!(props["errors"]["shared"], true);
    for absent in ["stats", "comments", "report"] {
        assert!(props.get(absent).is_none(), "{absent} was not delivered: {props}");
    }

    let (props, _) = feed(&[
        ("X-Inertia-Partial-Component", "Feed"),
        ("X-Inertia-Partial-Data", "stats"),
    ])
    .await;
    assert_eq!(props["stats"]["inertiaType"], "optional");
}

#[tokio::test]
async fn indt_a_deferred_prop_is_defer_only_on_the_extensions_deferred_visit() {
    let partial = [
        ("X-Inertia-Partial-Component", "Feed"),
        ("X-Inertia-Partial-Data", "comments"),
    ];
    let (by_hand, _) = feed(&partial).await;
    assert_eq!(by_hand["comments"]["inertiaType"], Value::Null);
    assert!(by_hand["comments"].get("deferGroup").is_none());

    let mut deferred = partial.to_vec();
    deferred.push(("X-Inertia-Devtools-Deferred", "true"));
    let (props, values) = feed(&deferred).await;
    assert_eq!(props["comments"]["inertiaType"], "defer");
    assert_eq!(props["comments"]["deferGroup"], "sidebar");
    assert_eq!(values["comments"], json!(["first"]));
}

#[tokio::test]
async fn indt_a_rescued_deferred_prop_is_marked_rescued() {
    let (props, values) = feed(&[
        ("X-Inertia-Partial-Component", "Feed"),
        ("X-Inertia-Partial-Data", "report"),
        ("X-Inertia-Devtools-Deferred", "true"),
    ])
    .await;
    assert_eq!(props["report"]["rescued"], true);
    assert_eq!(props["report"]["inertiaType"], "defer");
    assert!(values.get("report").is_none(), "the client received no value");
}

#[tokio::test]
async fn indt_a_reset_prop_is_marked_reset() {
    let (props, _) = feed(&[
        ("X-Inertia-Partial-Component", "Feed"),
        ("X-Inertia-Partial-Data", "posts"),
        ("X-Inertia-Reset", "posts"),
    ])
    .await;
    assert_eq!(props["posts"]["reset"], true);
    let (props, _) = feed(&[]).await;
    assert!(props["posts"].get("reset").is_none());
}

#[tokio::test]
async fn indt_deep_paths_without_metadata_are_pruned_and_values_are_what_the_client_got() {
    let (props, values) = feed(&[]).await;
    assert!(props.get("auth").is_some(), "every top-level key is kept");
    assert!(props.get("auth.user").is_none(), "a deep path without metadata is pruned");
    assert_eq!(props["auth.permissions"]["inertiaType"], "always");
    assert_eq!(
        values["auth"],
        json!({"user": {"name": "Ada"}, "permissions": ["edit"]})
    );
    assert_eq!(values["auth.permissions"], json!(["edit"]));
    assert_eq!(values["posts"], json!([1, 2]));
    assert!(values.get("stats").is_none(), "an optional prop that was not asked for");
    assert!(values.get("comments").is_none(), "a deferred prop that was only announced");
}

/// Share `appName`, returning the line of the call.
fn share_app_name() -> u32 {
    Inertia::share("appName", "Suprnova").expect("a string serializes");
    line!() - 1
}

/// The middleware hooks of the test, sharing `locale`.
struct LocaleHooks;

impl InertiaMiddlewareHooks for LocaleHooks {
    fn share(&self, _request: &dyn InertiaRequestExt) -> IndexMap<String, Prop> {
        let mut shared = IndexMap::new();
        shared.insert("locale".to_string(), Prop::eager(json!("en")));
        shared
    }
}

#[tokio::test]
async fn indt_a_shared_prop_names_its_share_call_and_a_render_prop_its_line() {
    let _container = TestContainer::fake();
    let share_line = share_app_name();
    let dir = tempfile::tempdir().unwrap();
    let config = inertia(devtools(dir.path())).hooks(LocaleHooks);
    let client = suprnova::testing::TestClient::new(
        router(),
        MiddlewareRegistry::new().append(Inertia::middleware(&config)),
    );
    let response = client.get("/feed").inertia().send().await;
    let entry = entry_of(dir.path(), &response);
    let props = &entry["props"];

    assert_eq!(props["appName"]["shared"], true);
    let source = &props["appName"]["shareSource"];
    assert!(source["file"].as_str().unwrap().ends_with("props.rs"), "{source}");
    assert_eq!(source["line"], share_line);

    assert_eq!(props["locale"]["shared"], true);
    assert_eq!(
        props["locale"]["shareSource"],
        json!({"file": std::any::type_name::<LocaleHooks>(), "line": 0})
    );
    assert!(props["appName"].get("renderSource").is_none());

    let render = &props["plain"]["renderSource"];
    let file = render["file"].as_str().unwrap();
    assert!(file.ends_with("props.rs"), "{render}");
    let text = std::fs::read_to_string(file).unwrap();
    let line = text
        .lines()
        .nth(render["line"].as_u64().unwrap() as usize - 1)
        .unwrap();
    assert!(line.contains(".with(\"plain\""), "{line}");
}
