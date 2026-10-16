//! The HTTP adapter exercises the endpoint's index and projection boundaries.
use super::derive::{database, note};
use http_body_util::BodyExt;
use std::sync::Arc;
use suprnova::{MiddlewareRegistry, Router, Search, SearchConfig, SearchDriver, SearchRegistry};

use super::common;

async fn request(path: &str) -> (u16, serde_json::Value) {
    request_as(path, None).await
}

struct Owner;
impl suprnova::Authenticatable for Owner {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn into_arc_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }
    fn get_auth_identifier(&self) -> String {
        "9".into()
    }
}
struct LoginAsOwner;
#[async_trait::async_trait]
impl suprnova::Middleware for LoginAsOwner {
    async fn handle(&self, request: suprnova::Request, next: suprnova::Next) -> suprnova::Response {
        suprnova::Auth::set_user(Arc::new(Owner));
        next(request).await
    }
}

async fn request_as(path: &str, owner: Option<LoginAsOwner>) -> (u16, serde_json::Value) {
    let router = Search::routes(&["test_search_notes"]).unwrap();
    let middleware = match owner {
        Some(owner) => MiddlewareRegistry::new().append(owner),
        None => MiddlewareRegistry::new(),
    };
    let response = suprnova::handle_request(
        Arc::new(router),
        Arc::new(middleware),
        common::incoming_get_request(path, &[("Accept", "application/json")]).await,
    )
    .await;
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn unnamed_index_is_a_non_disclosing_not_found() {
    let (status, body) = request("/search?index=secret_index&q=Rust").await;
    assert_eq!(status, 404);
    assert!(!body.to_string().contains("secret_index"));
}

#[tokio::test]
async fn invalid_attributes_name_the_parameter_and_attribute() {
    for (parameter, value, attribute) in [
        ("filter", "password%20%3D%20'secret'", "password"),
        ("filter", "title%20%3D%20'Rust'", "title"),
        ("facets", "title", "title"),
        ("sort", "body:asc", "body"),
    ] {
        let (status, body) = request(&format!(
            "/search?index=test_search_notes&{parameter}={value}"
        ))
        .await;
        assert_eq!(status, 422, "{body}");
        assert!(body["errors"][parameter].to_string().contains(attribute));
    }
}

#[tokio::test]
async fn guest_scope_and_document_projection_survive_http_serialization() {
    let _db = database().await;
    SearchConfig::scope(
        SearchConfig {
            driver: SearchDriver::Collection,
            queue: false,
            ..Default::default()
        },
        SearchRegistry::isolated(async {
            let public = note("Search <script>guide</script>", true, 0).await;
            let private = note("Search private", true, 9).await;
            for model in [&public, &private] {
                use suprnova::Searchable;
                let document = model.to_search_document().unwrap();
                suprnova::queue::Job::handle(suprnova::SearchIndexJob {
                    index: document.index,
                    key: document.key,
                    source_version: document.source_version,
                    op: suprnova::IndexOperation::Index,
                    generation: None,
                })
                .await
                .unwrap();
            }
            let (status, body) = request(
                "/search?index=test_search_notes&q=guide&facets=category&page=1&per_page=5",
            )
            .await;
            assert_eq!(status, 200, "{body}");
            assert_eq!(body["hits"].as_array().unwrap().len(), 1);
            assert_eq!(body["hits"][0]["key"], public.id.to_string());
            assert_eq!(body["total"], 1);
            assert_eq!(body["total_estimated"], false);
            assert_eq!(body["page"], 1);
            assert_eq!(body["per_page"], 5);
            let hit = &body["hits"][0];
            assert!(hit.get("model").is_none());
            for hidden in ["id", "lat", "lng", "deleted_at"] {
                assert!(hit["attributes"].get(hidden).is_none());
            }
            assert!(
                hit["formatted"]["title"]
                    .as_str()
                    .unwrap()
                    .contains("&lt;script&gt;<em>guide</em>&lt;/script&gt;")
            );
            assert!(hit["normalized_score"].as_f64().unwrap() <= 1.0);
            assert!(body.get("processing_time_ms").is_some());
            assert!(body.get("impression").is_none());
            let (status, body) = request_as(
                "/search?index=test_search_notes&q=private",
                Some(LoginAsOwner),
            )
            .await;
            assert_eq!(status, 200, "{body}");
            assert_eq!(body["hits"][0]["key"], private.id.to_string());
            assert_eq!(body["total"], 1);
            // A configured semantic default must retain the core total's accuracy.
            use suprnova::Searchable;
            SearchRegistry::register_embedder(
                "fake",
                Arc::new(suprnova::FakeEmbedder::new(8).unwrap()),
            )
            .unwrap();
            let descriptor = super::derive::SearchNote::descriptor();
            let engine = SearchRegistry::engine_for(descriptor.name).unwrap();
            let previous = SearchRegistry::generation_for(descriptor.name)
                .await
                .unwrap();
            let mut settings = engine.read_settings(descriptor, &previous).await.unwrap();
            settings.index_time.embedder = Some("fake".into());
            settings.index_time.dimensions = Some(8);
            settings.query.hybrid_ratio = 1.0;
            let generation = suprnova::GenerationId::new("endpoint_semantic").unwrap();
            engine
                .create_generation(descriptor, &settings, &generation)
                .await
                .unwrap();
            engine
                .switch_generation(descriptor, Some(previous), generation)
                .await
                .unwrap();
            let document = public.to_search_document().unwrap();
            suprnova::queue::Job::handle(suprnova::SearchIndexJob {
                index: document.index,
                key: document.key,
                source_version: document.source_version,
                op: suprnova::IndexOperation::Index,
                generation: None,
            })
            .await
            .unwrap();
            let (status, body) = request("/search?index=test_search_notes&q=guide").await;
            assert_eq!(status, 200, "{body}");
            assert_eq!(body["total"], 1);
            assert_eq!(body["total_estimated"], true);
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn pagination_requires_positive_bounded_numbers() {
    for (parameter, value) in [
        ("page", "0"),
        ("page", "bad"),
        ("per_page", "0"),
        ("per_page", "1001"),
        ("page", "18446744073709551615"),
    ] {
        let (status, body) = request(&format!(
            "/search?index=test_search_notes&{parameter}={value}"
        ))
        .await;
        assert_eq!(status, 422, "{body}");
        assert!(body["errors"].get(parameter).is_some(), "{body}");
    }
}

#[test]
fn route_installation_rejects_unregistered_indexes() {
    assert!(
        Router::new()
            .try_search_endpoint(&["absent_index"])
            .is_err()
    );
}

#[tokio::test]
async fn an_oversized_query_is_a_validation_error_naming_q() {
    let q = vec!["word"; 1025].join("%20");
    let (status, body) = request(&format!("/search?index=test_search_notes&q={q}")).await;
    assert_eq!(status, 422, "{body}");
    assert!(body["errors"]["q"].to_string().contains("1024"), "{body}");
}
