//! BIND-010: a unit-only enum derives `RouteBinding`. Each variant binds
//! from its `#[route(value = "...")]`, else its snake-case name, matched
//! exactly; any other value answers 404, never the route's `missing()`
//! response, with a body that names the enum and not the value.

use suprnova::http::{HttpResponse, text};
use suprnova::{Request, Response, RouteBinding, Router, handler, route};

use super::{get, message, serve};

#[derive(Debug, Clone, Copy, PartialEq, suprnova::RouteBinding)]
pub enum EnCategory {
    Fruits,
    PantryStaples,
    #[route(value = "veg")]
    Vegetables,
}

#[handler]
pub async fn show(category: EnCategory) -> Response {
    text(format!("{category:?}"))
}

async fn never(_request: Request) -> Response {
    Ok(HttpResponse::text("missing ran").status(302))
}

fn router() -> Router {
    Router::new()
        .get("/categories/{category}", show)
        .missing(never)
        .name("en.categories.show")
}

#[tokio::test]
async fn bind_010_a_variant_binds_from_its_value_or_snake_case_name() {
    let addr = serve(router()).await;
    assert_eq!(
        get(addr, "/categories/fruits").await,
        (200, "Fruits".to_owned())
    );
    assert_eq!(
        get(addr, "/categories/pantry_staples").await,
        (200, "PantryStaples".to_owned())
    );
    assert_eq!(
        get(addr, "/categories/veg").await,
        (200, "Vegetables".to_owned())
    );
    assert_eq!(
        get(addr, "/categories/vegetables").await.0,
        404,
        "a variant with a value binds from that value alone"
    );
}

#[tokio::test]
async fn bind_010_matching_is_exact_and_case_sensitive() {
    let addr = serve(router()).await;
    for value in ["Fruits", "FRUITS", "fruit", "pantry-staples"] {
        assert_eq!(
            get(addr, &format!("/categories/{value}")).await.0,
            404,
            "{value}"
        );
    }
}

#[tokio::test]
async fn bind_010_a_miss_answers_404_without_missing_and_never_repeats_the_value() {
    let addr = serve(router()).await;
    let (status, body) = get(addr, "/categories/sweets").await;
    assert_eq!(status, 404, "the route's missing() must not run: {body}");
    assert_eq!(message(&body), "EnCategory not found");
    assert!(!body.contains("sweets"), "{body}");
}

#[test]
fn bind_010_route_fills_a_parameter_with_the_variant_string() {
    let _router = router();
    assert_eq!(EnCategory::PantryStaples.route_key(), "pantry_staples");
    assert_eq!(
        route("en.categories.show", EnCategory::Vegetables).as_deref(),
        Some("/categories/veg")
    );
}
