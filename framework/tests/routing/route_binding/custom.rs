//! BIND-007: replacing a binding. A model with
//! `#[model(custom_route_binding)]` implements `RouteBinding` itself and can
//! still call the default lookup; any other type that implements it binds
//! the same way; the router's `bind` and `model` bind a parameter name on
//! every route, before or after them, ahead of the type's own binding.

use suprnova::http::text;
use suprnova::testing::TestDatabase;
use suprnova::{
    BoundChild, FrameworkError, Model, Response, RouteBinding, RouteBindingInfo, Router, handler,
    model, request,
};

use super::{get, message, refusal, run_sql, serve};

/// A model that binds by its name, written in any case.
#[model(table = "cu_tags", custom_route_binding, fillable = ["name"])]
pub struct CuTag {
    pub id: i64,
    pub name: String,
}

#[suprnova::async_trait]
impl RouteBinding for CuTag {
    fn route_key_name() -> &'static str {
        "name"
    }
    fn route_key(&self) -> String {
        self.name.clone()
    }
    async fn resolve_route_binding(
        value: &str,
        _field: Option<&str>,
    ) -> Result<Option<Self>, FrameworkError> {
        // The default lookup stays callable as a function.
        suprnova::database::resolve_model_route_binding::<Self>(
            &value.to_lowercase(),
            Some("name"),
            false,
        )
        .await
    }
}

/// A type that is no model at all.
#[derive(Debug, Clone, PartialEq)]
pub struct CuRegion(String);

#[suprnova::async_trait]
impl RouteBinding for CuRegion {
    fn route_key_name() -> &'static str {
        "code"
    }
    fn route_key(&self) -> String {
        self.0.clone()
    }
    async fn resolve_route_binding(
        value: &str,
        _field: Option<&str>,
    ) -> Result<Option<Self>, FrameworkError> {
        Ok(["eu", "us"]
            .contains(&value)
            .then(|| CuRegion(value.to_uppercase())))
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

#[model(table = "cu_users", fillable = ["name"])]
pub struct CuUser {
    pub id: i64,
    pub name: String,
}

#[request]
pub struct CuForm {
    pub name: String,
}

#[handler]
pub async fn show_tag(tag: CuTag) -> Response {
    text(format!("tag {}", tag.name))
}

#[handler]
pub async fn show_region(region: CuRegion) -> Response {
    text(format!("region {}", region.0))
}

#[handler]
pub async fn show_user(user: CuUser) -> Response {
    text(format!("user {}", user.name))
}

#[handler]
pub async fn show_user_id(user_id: CuUser) -> Response {
    text(format!("user {}", user_id.name))
}

#[handler]
pub async fn show_number(user: i64) -> Response {
    text(user.to_string())
}

#[handler]
pub async fn store_form(user: CuForm) -> Response {
    text(user.name)
}

async fn fixture() -> TestDatabase {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    run_sql(
        &db,
        &[
            "CREATE TABLE cu_tags (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
            "CREATE TABLE cu_users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
            "INSERT INTO cu_tags (id, name) VALUES (1, 'rust')",
            "INSERT INTO cu_users (id, name) VALUES (1, 'ada'), (2, 'grace')",
        ],
    )
    .await;
    db
}

#[tokio::test]
async fn bind_007_a_model_can_replace_its_binding() {
    let _db = fixture().await;
    let addr = serve(Router::new().get("/tags/{tag}", show_tag).into()).await;
    assert_eq!(get(addr, "/tags/RUST").await, (200, "tag rust".to_owned()));
    assert_eq!(get(addr, "/tags/1").await.0, 404, "the id no longer binds");
}

#[tokio::test]
async fn bind_007_a_type_that_is_no_model_binds_the_same_way() {
    let addr = serve(Router::new().get("/regions/{region}", show_region).into()).await;
    assert_eq!(
        get(addr, "/regions/eu").await,
        (200, "region EU".to_owned())
    );
    let (status, body) = get(addr, "/regions/mars").await;
    assert_eq!(
        (status, message(&body).as_str()),
        (404, "CuRegion not found")
    );
}

#[tokio::test]
async fn bind_007_bind_wins_over_the_type_and_covers_routes_registered_before_it() {
    let _db = fixture().await;
    let router: Router = Router::new().get("/users/{user}", show_user).into();
    // Registered after the route, the binder still covers it, and binds by
    // name where the type would bind by id.
    let router = router.bind("user", |value: String, route| async move {
        assert_eq!(route.pattern(), "/users/{user}");
        CuUser::query().filter("name", value).first().await
    });
    let addr = serve(router).await;
    assert_eq!(
        get(addr, "/users/grace").await,
        (200, "user grace".to_owned())
    );
    assert_eq!(
        get(addr, "/users/1").await.0,
        404,
        "the type's binding is not used"
    );
}

#[tokio::test]
async fn bind_007_a_binder_name_reads_a_dash_as_an_underscore() {
    let _db = fixture().await;
    let router = Router::new()
        .bind("user-id", |value: String, _route| async move {
            CuUser::query().filter("name", value).first().await
        })
        .get("/by-name/{user_id}", show_user_id);
    let addr = serve(router.into()).await;
    assert_eq!(
        get(addr, "/by-name/ada").await,
        (200, "user ada".to_owned())
    );
}

#[tokio::test]
async fn bind_007_model_calls_its_fallback_for_a_missing_row() {
    let _db = fixture().await;
    let router = Router::new()
        .model::<CuUser, _, _>("user", |value| async move {
            Ok(CuUser {
                name: format!("guest {value}"),
                ..CuUser::default()
            })
        })
        .get("/users/{user}", show_user);
    let addr = serve(router.into()).await;
    assert_eq!(get(addr, "/users/1").await, (200, "user ada".to_owned()));
    assert_eq!(
        get(addr, "/users/99").await,
        (200, "user guest 99".to_owned()),
        "the fallback answers, not a 404"
    );
}

#[test]
fn bind_007_a_resolver_of_another_type_is_refused() {
    let router: Router = Router::new().get("/users/{user}", show_user).into();
    let router = router.bind(
        "user",
        |value: String, _route| async move { Ok(Some(value)) },
    );
    let error = refusal(&router);
    assert!(error.contains("GET /users/{user}"), "{error}");
    assert!(error.contains("String"), "{error}");
    assert!(error.contains("CuUser"), "{error}");
}

#[test]
fn bind_007_a_binder_for_a_path_value_or_a_form_request_is_refused() {
    let resolver = |value: String, _route: suprnova::MatchedRoute| async move {
        Ok(Some(CuUser {
            name: value,
            ..CuUser::default()
        }))
    };
    let router: Router = Router::new().get("/numbers/{user}", show_number).into();
    let error = refusal(&router.bind("user", resolver));
    assert!(error.contains("`user: i64`"), "{error}");

    let router: Router = Router::new().post("/forms/{user}", store_form).into();
    let error = refusal(&router.bind("user", resolver));
    assert!(error.contains("`user: CuForm`"), "{error}");
}
