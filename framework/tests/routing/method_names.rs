//! A route's name belongs to its method and pattern: two methods on one
//! path each report their own name through `Request::route_name`, as
//! Laravel's `Request::route()->getName()` names the route that matched.
//!
//! Route names are process-wide and a name may be bound once, so every
//! test uses names of its own.

use std::pin::Pin;

use suprnova::routing::ResourceController;
use suprnova::testing::TestClient;
use suprnova::{
    BoundChild, FrameworkError, MatchedRoute, MiddlewareRegistry, Request, Response, RouteBinding,
    RouteBindingInfo, Router, get, group, handler, post, put,
};

/// The name of the route the request matched, or `-` for none.
async fn name_of(req: Request) -> Response {
    suprnova::http::text(req.route_name().unwrap_or_else(|| "-".to_string()))
}

/// What `method path` answered.
async fn answer(client: &TestClient, method: &str, path: &str) -> String {
    let response = match method {
        "GET" => client.get(path),
        "POST" => client.post(path),
        "PUT" => client.put(path),
        other => panic!("no {other} here"),
    }
    .send()
    .await;
    response.assert_ok();
    response.body_text()
}

#[tokio::test]
async fn two_methods_on_one_path_each_report_their_own_name() {
    let router: Router = Router::new()
        .get("/method-names/items", name_of)
        .name("method_names.items.index")
        .post("/method-names/items", name_of)
        .name("method_names.items.store")
        .put("/method-names/items", name_of)
        .into();
    let client = TestClient::new(router, MiddlewareRegistry::new());

    assert_eq!(
        answer(&client, "GET", "/method-names/items").await,
        "method_names.items.index"
    );
    assert_eq!(
        answer(&client, "POST", "/method-names/items").await,
        "method_names.items.store"
    );
    assert_eq!(
        answer(&client, "PUT", "/method-names/items").await,
        "-",
        "an unnamed route has no name, though another method on its path has one"
    );
}

#[tokio::test]
async fn a_group_of_macro_routes_names_each_method_on_one_path() {
    let router = group!("/method-names/users", {
        get!("/", name_of).name("index"),
        post!("/", name_of).name("store"),
        put!("/", name_of),
    })
    .name("method_names.users.")
    .register(Router::new());
    let client = TestClient::new(router, MiddlewareRegistry::new());

    assert_eq!(
        answer(&client, "GET", "/method-names/users").await,
        "method_names.users.index"
    );
    assert_eq!(
        answer(&client, "POST", "/method-names/users").await,
        "method_names.users.store"
    );
    assert_eq!(answer(&client, "PUT", "/method-names/users").await, "-");
}

/// A resource whose `index` and `store` answer with the route's name.
struct NamesCtl;

impl ResourceController for NamesCtl {
    fn index(&self, request: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(name_of(request))
    }
    fn store(&self, request: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(name_of(request))
    }
}

#[tokio::test]
async fn a_resource_names_index_and_store_on_its_one_path() {
    let router: Router = Router::new()
        .resource("method-names-posts", NamesCtl)
        .only(&[
            suprnova::routing::ResourceAction::Index,
            suprnova::routing::ResourceAction::Store,
        ])
        .into();
    let client = TestClient::new(router, MiddlewareRegistry::new());

    assert_eq!(
        answer(&client, "GET", "/method-names-posts").await,
        "method-names-posts.index"
    );
    assert_eq!(
        answer(&client, "POST", "/method-names-posts").await,
        "method-names-posts.store"
    );
}

/// A value a binder resolves to the name its `MatchedRoute` reports.
#[derive(Debug, Clone)]
pub struct SeenName(String);

#[suprnova::async_trait]
impl RouteBinding for SeenName {
    fn route_key_name() -> &'static str {
        "name"
    }
    fn route_key(&self) -> String {
        self.0.clone()
    }
    async fn resolve_route_binding(
        value: &str,
        _field: Option<&str>,
    ) -> Result<Option<Self>, FrameworkError> {
        Ok(Some(Self(value.to_string())))
    }
    async fn resolve_child_route_binding(
        &self,
        _child: &str,
        _value: &str,
        _field: Option<&str>,
    ) -> Result<Option<BoundChild>, FrameworkError> {
        Ok(None)
    }
    fn route_binding_info() -> RouteBindingInfo {
        RouteBindingInfo::of::<Self>()
    }
}

/// Answer with the name the binder saw.
#[handler]
pub async fn show_seen(member: SeenName) -> Response {
    suprnova::http::text(member.0)
}

#[tokio::test]
async fn a_binder_sees_the_name_of_the_route_that_matched_by_its_method() {
    // `GET` and `POST` on one pattern, named like a resource's `index` and
    // `store`; the binder reports the name its `MatchedRoute` carries as
    // the bound value, which the handler answers with.
    let router: Router = Router::new()
        .get("/method-names/members/{member}", show_seen)
        .name("method_names.members.index")
        .post("/method-names/members/{member}", show_seen)
        .name("method_names.members.store");
    let router = router.bind("member", |_value: String, route: MatchedRoute| async move {
        Ok(Some(SeenName(
            route.name().unwrap_or_else(|| "-".to_string()),
        )))
    });
    let client = TestClient::new(router, MiddlewareRegistry::new());

    assert_eq!(
        answer(&client, "GET", "/method-names/members/ada").await,
        "method_names.members.index"
    );
    assert_eq!(
        answer(&client, "POST", "/method-names/members/ada").await,
        "method_names.members.store"
    );
}
