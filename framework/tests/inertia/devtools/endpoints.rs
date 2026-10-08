//! PAR-074: `GET /_inertia/devtools/entries` and `entries/{id}`, their
//! filters and not-found answers, the gate outside the `local`
//! environment, and the session an entry request leaves as it found it.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use suprnova::auth::Authenticatable;
use suprnova::testing::{TestClient, TestContainer};
use suprnova::{
    Auth, Gate, HttpResponse, Inertia, InertiaResponse, Middleware, MiddlewareRegistry, Next,
    Redirect, Request, Response, Router,
};

use super::{app_env, client, devtools, entry_ids, inertia, session_client};

fn router() -> Router {
    Router::new()
        .get("/home", |req: Request| async move {
            InertiaResponse::new("Home")
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)
        })
        .get("/about", |req: Request| async move {
            InertiaResponse::new("About")
                .resolve(&req)
                .await
                .map_err(HttpResponse::from)
        })
        .post("/save", |_req: Request| async {
            let response: Response = Redirect::to("/home").with("status", "Saved").into();
            response
        })
        .get("/status", |_req: Request| async {
            let status =
                suprnova::session::session_mut(|session| session.get_flash::<String>("status"))
                    .flatten()
                    .unwrap_or_else(|| "none".to_string());
            suprnova::http::text(status)
        })
        .into()
}

/// The ids and request types the list endpoint answers for `query`.
async fn listed(client: &TestClient, query: &str) -> Vec<(String, String, String)> {
    let response = client
        .get(format!("/_inertia/devtools/entries{query}"))
        .send()
        .await;
    response.assert_ok();
    response
        .json()
        .as_array()
        .unwrap_or_else(|| panic!("a list: {}", response.body_text()))
        .iter()
        .map(|meta| {
            (
                meta["id"].as_str().unwrap().to_string(),
                meta["component"].as_str().unwrap_or_default().to_string(),
                meta["requestType"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[tokio::test]
async fn indt_entries_lists_newest_first_and_filters_by_component_type_offset_and_limit() {
    let _env = app_env("local").await;
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));
    let mut sent = Vec::new();
    for (path, headers) in [
        ("/home", vec![]),
        (
            "/home",
            vec![
                ("X-Inertia-Partial-Component", "Home"),
                ("X-Inertia-Partial-Data", "x"),
            ],
        ),
        ("/about", vec![]),
        ("/home", vec![("X-Inertia-Devtools-Poll", "true")]),
    ] {
        let mut request = client.get(path).inertia();
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = request.send().await;
        sent.push(
            response
                .header("x-inertia-devtools-id")
                .unwrap()
                .to_string(),
        );
    }

    let all = listed(&client, "").await;
    let ids: Vec<&String> = all.iter().map(|(id, _, _)| id).collect();
    let newest_first: Vec<&String> = sent.iter().rev().collect();
    assert_eq!(ids, newest_first);

    let home = listed(&client, "?component=Home").await;
    assert_eq!(home.len(), 3);
    assert!(home.iter().all(|(_, component, _)| component == "Home"));

    let types = listed(&client, "?type=navigate,partial").await;
    let kinds: Vec<&str> = types.iter().map(|(_, _, kind)| kind.as_str()).collect();
    assert_eq!(kinds, vec!["navigate", "partial", "navigate"]);

    let no_polls = listed(&client, "?exclude=poll").await;
    assert_eq!(no_polls.len(), 3);
    assert!(no_polls.iter().all(|(_, _, kind)| kind != "poll"));

    let page = listed(&client, "?offset=1&limit=2").await;
    let page_ids: Vec<&String> = page.iter().map(|(id, _, _)| id).collect();
    assert_eq!(page_ids, vec![&sent[2], &sent[1]]);
    assert_eq!(
        listed(&client, "?limit=0").await.len(),
        1,
        "a limit is at least 1"
    );
}

#[tokio::test]
async fn indt_one_entry_by_id_and_not_found_for_anything_else() {
    let _env = app_env("local").await;
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()));
    let visit = client.get("/home").inertia().send().await;
    let id = visit.header("x-inertia-devtools-id").unwrap();

    let found = client
        .get(format!("/_inertia/devtools/entries/{id}"))
        .send()
        .await;
    found.assert_ok();
    assert_eq!(found.json()["__meta"]["id"], id);
    assert_eq!(found.json()["__meta"]["component"], "Home");

    for missing in ["not-a-ulid", "01ARZ3NDEKTSV4RRFFQ69G5FAV"] {
        let response = client
            .get(format!("/_inertia/devtools/entries/{missing}"))
            .send()
            .await;
        response.assert_status(404);
        assert_eq!(
            response.json(),
            json!({"message": "Not found."}),
            "{missing}"
        );
    }
}

/// A user for the gate to judge.
struct Admin;

impl Authenticatable for Admin {
    fn get_auth_identifier(&self) -> String {
        "admin".to_string()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// Signs `Admin` in for the request, as an authentication middleware does.
struct ActingAsAdmin;

#[async_trait]
impl Middleware for ActingAsAdmin {
    async fn handle(&self, request: Request, next: Next) -> Response {
        Auth::set_user(Arc::new(Admin));
        next(request).await
    }
}

#[tokio::test]
async fn indt_outside_local_only_the_configured_gate_admits_a_request() {
    let _env = app_env("production").await;
    let _container = TestContainer::fake();
    let dir = tempfile::tempdir().unwrap();

    let forbidden = json!({"message": "Forbidden."});
    let ungated = client(router(), devtools(dir.path()));
    let response = ungated.get("/_inertia/devtools/entries").send().await;
    response.assert_status(403);
    assert_eq!(response.json(), forbidden, "no gate configured");

    Gate::define::<(), ()>("indt-devtools-guests", |_, _| false);
    let denied = client(router(), devtools(dir.path()).gate("indt-devtools-guests"));
    let response = denied.get("/_inertia/devtools/entries").send().await;
    response.assert_status(403);
    assert_eq!(response.json(), forbidden, "the gate denies a guest");

    Gate::define::<(), ()>("indt-devtools-open", |_, _| true);
    let open = client(router(), devtools(dir.path()).gate("indt-devtools-open"));
    open.get("/_inertia/devtools/entries")
        .send()
        .await
        .assert_ok();

    Gate::define::<Admin, ()>("indt-devtools-admins", |_, _| true);
    let admins = devtools(dir.path()).gate("indt-devtools-admins");
    let guest = client(router(), admins.clone());
    guest
        .get("/_inertia/devtools/entries")
        .send()
        .await
        .assert_status(403);
    let signed_in = TestClient::new(
        router(),
        MiddlewareRegistry::new()
            .append(ActingAsAdmin)
            .append(Inertia::middleware(&inertia(admins))),
    );
    signed_in
        .get("/_inertia/devtools/entries")
        .send()
        .await
        .assert_ok();
}

#[tokio::test]
async fn indt_in_local_a_request_is_admitted_without_a_gate() {
    let _env = app_env("local").await;
    let dir = tempfile::tempdir().unwrap();
    Gate::define::<(), ()>("indt-devtools-closed", |_, _| false);
    let client = client(router(), devtools(dir.path()).gate("indt-devtools-closed"));
    client
        .get("/_inertia/devtools/entries")
        .send()
        .await
        .assert_ok();
}

#[tokio::test]
async fn indt_an_entry_request_keeps_the_flash_and_the_previous_url() {
    let _env = app_env("local").await;
    let dir = tempfile::tempdir().unwrap();
    let (client, store) = session_client(router(), devtools(dir.path()));

    client.get("/home").send().await.assert_ok();
    assert_eq!(
        store.only_session().previous_url().as_deref(),
        Some("/home")
    );
    client.post("/save").send().await.assert_status(302);
    // The extension fetches the entry while the redirect is followed.
    client
        .get("/_inertia/devtools/entries")
        .send()
        .await
        .assert_ok();
    assert_eq!(
        store.only_session().previous_url().as_deref(),
        Some("/home"),
        "the entry request never becomes the previous URL"
    );
    client
        .get("/status")
        .send()
        .await
        .assert_ok()
        .assert_see("Saved");
}

#[tokio::test]
async fn indt_an_entry_request_is_never_recorded() {
    let _env = app_env("local").await;
    let dir = tempfile::tempdir().unwrap();
    let client = client(router(), devtools(dir.path()).except(Vec::<String>::new()));
    let visit = client.get("/home").inertia().send().await;
    let id = visit.header("x-inertia-devtools-id").unwrap().to_string();

    let list = client.get("/_inertia/devtools/entries").send().await;
    let one = client
        .get(format!("/_inertia/devtools/entries/{id}"))
        .send()
        .await;
    assert_eq!(list.header("x-inertia-devtools-id"), None);
    assert_eq!(one.header("x-inertia-devtools-id"), None);
    assert_eq!(
        entry_ids(dir.path()),
        vec![id],
        "only the page visit is stored"
    );
    let _: Value = list.json();
}

#[tokio::test]
async fn indt_with_the_stack_on_route_groups_the_endpoints_still_answer() {
    // `Inertia::install` registers process-wide middleware and retains its
    // configuration, so this runs alone in a child process.
    if crate::own_process_async::delegate(
        module_path!(),
        "indt_with_the_stack_on_route_groups_the_endpoints_still_answer",
    )
    .await
    {
        return;
    }
    let _env = app_env("local").await;
    let dir = tempfile::tempdir().unwrap();
    let before = suprnova::middleware::global_middleware_count();
    Inertia::install(&inertia(devtools(dir.path())).register_globally(false))
        .expect("dev-mode install needs no manifest");
    assert_eq!(
        suprnova::middleware::global_middleware_count(),
        before + 1,
        "one global middleware, for the endpoints"
    );

    let router: Router = Router::new()
        .group("/app", |r| {
            r.get("/page", |req: Request| async move {
                InertiaResponse::new("Home")
                    .resolve(&req)
                    .await
                    .map_err(HttpResponse::from)
            })
        })
        .middleware_named("inertia")
        .into();
    let router: Router = router
        .group("/api", |r| {
            r.get("/ping", |_req: Request| async {
                suprnova::http::text("pong")
            })
        })
        .into();
    let client = TestClient::new(router, MiddlewareRegistry::from_global());

    let page = client.get("/app/page").inertia().send().await;
    page.assert_ok();
    let id = page
        .header("x-inertia-devtools-id")
        .expect("the group's stack records its routes")
        .to_string();
    let api = client.get("/api/ping").send().await;
    api.assert_ok().assert_see("pong");
    assert_eq!(
        api.header("x-inertia-devtools-id"),
        None,
        "a route outside the groups is not recorded"
    );

    let found = client
        .get(format!("/_inertia/devtools/entries/{id}"))
        .send()
        .await;
    found.assert_ok();
    assert_eq!(found.json()["__meta"]["id"], id.as_str());
    assert_eq!(entry_ids(dir.path()), vec![id]);
}
