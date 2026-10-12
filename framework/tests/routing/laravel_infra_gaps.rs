//! Laravel infrastructure gaps owned by the routing suite: a handler
//! argument that names the route parameter it reads, with
//! `#[route_param("name")]` or as a raw identifier.

use suprnova::http::text;
use suprnova::{
    BoundChild, FrameworkError, Response, RouteBinding, RouteBindingInfo, Router, handler,
};

use crate::route_binding::{get, refusal, serve};

#[handler]
pub async fn show_post(#[route_param("post")] id: i64) -> Response {
    text(id.to_string())
}

#[handler]
pub async fn show_item(r#type: String) -> Response {
    text(r#type)
}

#[handler]
pub async fn show_page(#[route_param("page")] number: Option<u32>) -> Response {
    text(number.map_or_else(|| "none".to_owned(), |n| n.to_string()))
}

/// A bound value that echoes the route value it was bound from.
pub struct Slugged {
    value: String,
}

#[suprnova::async_trait]
impl RouteBinding for Slugged {
    fn route_key_name() -> &'static str {
        "slug"
    }
    fn route_key(&self) -> String {
        self.value.clone()
    }
    async fn resolve_route_binding(
        value: &str,
        _field: Option<&str>,
    ) -> Result<Option<Self>, FrameworkError> {
        Ok(Some(Slugged {
            value: value.to_owned(),
        }))
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

#[handler]
pub async fn show_article(#[route_param("article")] found: Slugged) -> Response {
    text(found.value)
}

#[tokio::test]
async fn a_route_param_argument_reads_the_parameter_it_names() {
    let router: Router = Router::new().get("/posts/{post}", show_post).into();
    router
        .prepare_bindings()
        .expect("`id` reads `post`, which the path declares");
    let addr = serve(router).await;
    assert_eq!(get(addr, "/posts/7").await, (200, "7".to_owned()));
}

#[tokio::test]
async fn a_route_param_argument_whose_parameter_is_missing_is_refused_at_startup() {
    let router: Router = Router::new().get("/posts/{id}", show_post).into();
    let error = refusal(&router);
    assert!(error.contains("post"), "{error}");
}

#[tokio::test]
async fn a_raw_identifier_reads_the_parameter_spelled_without_r_hash() {
    let router: Router = Router::new().get("/items/{type}", show_item).into();
    router
        .prepare_bindings()
        .expect("`r#type` reads `type`, which the path declares");
    let addr = serve(router).await;
    assert_eq!(get(addr, "/items/book").await, (200, "book".to_owned()));
}

#[tokio::test]
async fn a_renamed_optional_path_value_reads_its_parameter() {
    let router: Router = Router::new().get("/pages/{page?}", show_page).into();
    router.prepare_bindings().expect("the path declares `page`");
    let addr = serve(router).await;
    assert_eq!(get(addr, "/pages/3").await, (200, "3".to_owned()));
    assert_eq!(get(addr, "/pages").await, (200, "none".to_owned()));
}

#[tokio::test]
async fn a_renamed_bound_argument_binds_from_the_parameter_it_names() {
    let router: Router = Router::new()
        .get("/articles/{article}", show_article)
        .into();
    router
        .prepare_bindings()
        .expect("the path declares `article`");
    let addr = serve(router).await;
    assert_eq!(
        get(addr, "/articles/hello-world").await,
        (200, "hello-world".to_owned())
    );

    let misnamed: Router = Router::new().get("/articles/{found}", show_article).into();
    let error = refusal(&misnamed);
    assert!(error.contains("article"), "{error}");
}
