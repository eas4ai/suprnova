//! PAR-072 and PAR-073: an entry names the route that matched, by its
//! method as well as its pattern, and records the response the client
//! got when the page's document fails to render.

use serde_json::Value;
use suprnova::testing::TestContainer;
use suprnova::{
    HttpResponse, InertiaConfig, InertiaResponse, InertiaRootTemplate, Redirect, Request, Response,
    Router,
};

use super::{client, devtools, entry_of};

/// A root document whose last expression fails to render.
#[suprnova::inertia_root(path = "inertia/failing.html")]
struct FailingDocument;

/// The filter `inertia/failing.html` applies.
mod filters {
    use suprnova::view::{FilterResult, FilterValues};

    /// Always fails, so the document fails after writing part of itself.
    #[suprnova::view_filter]
    pub fn refuse(_value: &str, _: &dyn FilterValues) -> FilterResult<String> {
        Err(std::fmt::Error.into())
    }
}

fn router() -> Router {
    Router::new()
        .get("/indt-members", |req: Request| async move {
            InertiaResponse::new("Members")
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)
        })
        .name("indt.members.index")
        .post("/indt-members", |_req: Request| async {
            let response: Response = Redirect::to("/indt-members").into();
            response
        })
        .name("indt.members.store")
}

#[tokio::test]
async fn indt_each_method_on_one_path_is_recorded_with_its_own_route_name() {
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));

    let index = client.get("/indt-members").inertia().send().await;
    index.assert_ok();
    let store = client.post("/indt-members").inertia().send().await;
    store.assert_status(302);

    let index = entry_of(dir.path(), &index);
    let store = entry_of(dir.path(), &store);
    assert_eq!(
        index["route"]["name"], "indt.members.index",
        "{}",
        index["route"]
    );
    assert_eq!(
        store["route"]["name"], "indt.members.store",
        "{}",
        store["route"]
    );
    assert_eq!(index["route"]["uri"], "/indt-members");
    assert_eq!(store["route"]["uri"], "/indt-members");
}

#[tokio::test]
async fn indt_a_page_whose_document_fails_to_render_records_the_error_response() {
    let _container = TestContainer::fake();
    let dir = tempfile::tempdir().unwrap();
    let router: Router = Router::new()
        .get("/indt-failing-document", |req: Request| async move {
            InertiaResponse::new("Members")
                .with("members", serde_json::json!(["Ada", "Grace"]))
                .with_config(
                    InertiaConfig::new()
                        .development(true)
                        .version("")
                        .root_template(InertiaRootTemplate::of::<FailingDocument>()),
                )
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)
        })
        .into();
    let client = client(router, devtools(dir.path()));

    let response = client.get("/indt-failing-document").send().await;
    response.assert_status(500);
    let got = response.body_text();
    let entry = entry_of(dir.path(), &response);

    assert_eq!(entry["__meta"]["status"], 500);
    assert_eq!(
        entry["__meta"]["component"],
        Value::Null,
        "no page reached the client: {}",
        entry["__meta"]
    );
    assert_eq!(entry["__meta"]["requestType"], "http");
    for empty in ["props", "propValues"] {
        assert!(
            entry[empty]
                .as_object()
                .is_none_or(serde_json::Map::is_empty),
            "{empty} holds nothing the client did not receive: {}",
            entry[empty]
        );
    }
    let body = &entry["http"]["responseBody"];
    assert_eq!(body["status"], "present", "{body}");
    let expected: Value = serde_json::from_str(&got).unwrap_or_else(|_| Value::String(got.clone()));
    assert_eq!(body["value"], expected, "the error the client got");
}
