//! Phase 9A - Pinecone vector driver tests.
//!
//! Requires `--features vector-pinecone` because the driver is
//! feature-gated. The feature no longer gates a dependency - the driver
//! speaks Pinecone's REST API over the framework's existing `reqwest`
//! client - it just gates compilation of the module.

#![cfg(feature = "vector-pinecone")]

//!
//! Three layers:
//!
//! 1. **Pure-function tests** (always run) - metadata round-trips, vector
//!    encoding, match decoding, and the trait's short-circuit paths
//!    (empty inputs, k = 0, empty/zero-vector queries). None touches the
//!    network.
//!
//! 2. **Wire-contract tests** (always run) - drive the driver against a
//!    `wiremock` server standing in for Pinecone, and assert the exact
//!    method, path, headers and JSON body it puts on the wire, plus how
//!    it decodes the documented response shapes. These exist because the
//!    live tests below cannot run in CI: without them a typo in a field
//!    name (`topK` vs `top_k`) would ship silently.
//!
//!    They verify the driver against Pinecone's *documented* REST
//!    contract for API version `2025-04`. They cannot verify that the
//!    documentation matches the live service - only layer 3 does that.
//!
//! 3. **Integration tests** (`#[ignore]`) - drive a real Pinecone
//!    account. Require both:
//!    - `PINECONE_API_KEY` - your account's API key
//!    - `PINECONE_TEST_INDEX` - a pre-existing serverless index name
//!      (the driver doesn't auto-create indexes; see the module docs).
//!
//!    Each integration test uses a unique namespace (timestamp-tagged)
//!    and deletes it on the way out - your test index is reused but never
//!    polluted.
//!
//!    ```bash
//!    PINECONE_API_KEY=... PINECONE_TEST_INDEX=my-test-index \
//!        cargo test -p suprnova --features vector-pinecone \
//!        --test vector_pinecone -- --ignored
//!    ```

use serde_json::json;
use suprnova::vector::driver::VectorDriver;
use suprnova::vector::{DEFAULT_API_VERSION, PineconeMatch, PineconeVector};
use suprnova::{PineconeVectorDriver, VectorItem};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TEST_KEY: &str = "pcsk-test-key-never-leaves-this-process";
const TEST_INDEX: &str = "docs-index";

// ----------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------

/// A driver whose control plane points at a closed port. The short-circuit
/// tests must return before any request is attempted; if one ever stops
/// short-circuiting, this fails fast with connection-refused instead of
/// hanging for the full 30s request timeout.
fn unreachable_driver() -> PineconeVectorDriver {
    PineconeVectorDriver::from_api_key(TEST_KEY)
        .expect("driver builds without network")
        .with_control_plane("http://127.0.0.1:1")
}

/// A driver whose data plane is pinned to `server`, so no control-plane
/// round trip is needed. `with_index_host` deliberately honours the
/// scheme it is given, which is what lets a plain-HTTP fake stand in.
fn driver_against(server: &MockServer) -> PineconeVectorDriver {
    PineconeVectorDriver::from_api_key(TEST_KEY)
        .expect("driver builds")
        .with_index_host(TEST_INDEX, server.uri())
}

async fn only_request(server: &MockServer) -> wiremock::Request {
    let mut requests = server
        .received_requests()
        .await
        .expect("wiremock records requests");
    assert_eq!(
        requests.len(),
        1,
        "expected exactly one request, got {requests:#?}"
    );
    requests.remove(0)
}

// ----------------------------------------------------------------------
// Pure-function tests - metadata conversion
// ----------------------------------------------------------------------

#[test]
fn metadata_from_json_null_yields_none() {
    let got = PineconeVectorDriver::metadata_from_json(serde_json::Value::Null).unwrap();
    assert!(got.is_none());
}

#[test]
fn metadata_from_json_empty_object_yields_empty_map() {
    let got = PineconeVectorDriver::metadata_from_json(json!({})).unwrap();
    let map = got.expect("empty object is still Some");
    assert!(map.is_empty());
}

#[test]
fn metadata_from_json_rejects_non_object_non_null() {
    let err = PineconeVectorDriver::metadata_from_json(json!("a bare string")).unwrap_err();
    assert!(
        format!("{err}").contains("JSON object or null"),
        "error names the constraint: {err}"
    );
}

#[test]
fn metadata_round_trip_preserves_every_json_kind() {
    let original = json!({
        "string_field": "hello",
        "bool_field": true,
        "number_field": 42.5,
        "null_field": null,
        "array_field": [1, "two", false],
        "nested": { "inner": "value" }
    });
    let map = PineconeVectorDriver::metadata_from_json(original.clone())
        .unwrap()
        .expect("object yields Some");
    let back = PineconeVectorDriver::metadata_to_json(Some(map));
    assert_eq!(
        back, original,
        "metadata is carried as JSON end to end, so nothing should be lost or coerced"
    );
}

#[test]
fn metadata_to_json_none_yields_null() {
    assert_eq!(
        PineconeVectorDriver::metadata_to_json(None),
        serde_json::Value::Null
    );
}

#[test]
fn metadata_to_json_empty_map_yields_empty_object() {
    let got = PineconeVectorDriver::metadata_to_json(Some(serde_json::Map::new()));
    assert_eq!(got, json!({}));
}

// ----------------------------------------------------------------------
// Pure-function tests - vector encode
// ----------------------------------------------------------------------

#[test]
fn build_vector_passes_id_through_unchanged() {
    let v = PineconeVectorDriver::build_vector(VectorItem::new(
        "anything-goes-as-an-id-✓",
        vec![1.0, 0.0],
        json!({}),
    ))
    .unwrap();
    assert_eq!(v.id, "anything-goes-as-an-id-✓");
}

#[test]
fn build_vector_includes_embedding_values() {
    let v =
        PineconeVectorDriver::build_vector(VectorItem::new("id", vec![1.0, 2.0, 3.0], json!({})))
            .unwrap();
    assert_eq!(v.values, vec![1.0, 2.0, 3.0]);
}

#[test]
fn build_vector_attaches_metadata_when_object() {
    let v = PineconeVectorDriver::build_vector(VectorItem::new(
        "id",
        vec![1.0],
        json!({"tag": "important"}),
    ))
    .unwrap();
    let metadata = v.metadata.expect("object metadata is Some");
    assert!(metadata.contains_key("tag"));
}

#[test]
fn build_vector_with_null_metadata_yields_none() {
    let v = PineconeVectorDriver::build_vector(VectorItem::new(
        "id",
        vec![1.0],
        serde_json::Value::Null,
    ))
    .unwrap();
    assert!(v.metadata.is_none());
}

#[test]
fn build_vector_rejects_non_object_metadata() {
    let err = PineconeVectorDriver::build_vector(VectorItem::new(
        "id",
        vec![1.0],
        json!("not an object"),
    ))
    .unwrap_err();
    assert!(format!("{err}").contains("JSON object or null"));
}

// ----------------------------------------------------------------------
// Pure-function tests - match decode
// ----------------------------------------------------------------------

#[test]
fn decode_match_passes_id_and_score_through() {
    let m = PineconeVectorDriver::decode_match(PineconeMatch {
        id: "my-id".to_string(),
        score: 0.93,
        metadata: None,
    });
    assert_eq!(m.id, "my-id");
    assert!((m.score - 0.93).abs() < 1e-6);
    assert_eq!(m.metadata, serde_json::Value::Null);
}

#[test]
fn decode_match_with_metadata_yields_object() {
    let mut metadata = serde_json::Map::new();
    metadata.insert("key".to_string(), json!("value"));
    let m = PineconeVectorDriver::decode_match(PineconeMatch {
        id: "id".to_string(),
        score: 0.5,
        metadata: Some(metadata),
    });
    assert_eq!(m.metadata["key"], "value");
}

// ----------------------------------------------------------------------
// Pure-function tests - short-circuits (no real network)
// ----------------------------------------------------------------------

#[tokio::test]
async fn upsert_with_empty_items_is_no_op() {
    unreachable_driver()
        .upsert("never-reached", vec![])
        .await
        .unwrap();
}

#[tokio::test]
async fn delete_with_empty_ids_is_no_op() {
    unreachable_driver()
        .delete("never-reached", Vec::<String>::new())
        .await
        .unwrap();
}

#[tokio::test]
async fn similar_with_k_zero_returns_empty_without_call() {
    let hits = unreachable_driver()
        .similar("never-reached", vec![1.0, 0.0], 0)
        .await
        .unwrap();
    assert!(hits.is_empty());
}

#[tokio::test]
async fn similar_with_empty_query_errors_locally() {
    let err = unreachable_driver()
        .similar("never-reached", Vec::<f32>::new(), 5)
        .await
        .unwrap_err();
    assert!(
        format!("{err}").contains("empty"),
        "error names the cause: {err}"
    );
}

#[tokio::test]
async fn similar_with_zero_vector_errors_locally() {
    let err = unreachable_driver()
        .similar("never-reached", vec![0.0, 0.0, 0.0], 5)
        .await
        .unwrap_err();
    assert!(
        format!("{err}").contains("zero-vector"),
        "error names the cause: {err}"
    );
}

#[tokio::test]
async fn upsert_with_zero_dim_first_item_errors_locally() {
    let err = unreachable_driver()
        .upsert(
            "never-reached",
            vec![VectorItem::new("a", Vec::<f32>::new(), json!({}))],
        )
        .await
        .unwrap_err();
    assert!(format!("{err}").contains("zero-length"));
}

// ----------------------------------------------------------------------
// Pure-function tests - builder
// ----------------------------------------------------------------------

#[test]
fn with_namespace_sets_the_namespace() {
    assert_eq!(
        unreachable_driver().with_namespace("custom-ns").namespace(),
        "custom-ns"
    );
}

#[test]
fn default_namespace_is_empty() {
    assert_eq!(unreachable_driver().namespace(), "");
}

/// The key must not reach a log through the back door of a `Debug` derive
/// on some struct that happens to hold a driver.
#[test]
fn debug_redacts_the_api_key() {
    let rendered = format!("{:?}", unreachable_driver());
    assert!(
        !rendered.contains(TEST_KEY),
        "Debug must not print the API key: {rendered}"
    );
    assert!(
        rendered.contains("redacted"),
        "…and should say so: {rendered}"
    );
}

// ----------------------------------------------------------------------
// Wire-contract tests - what actually goes on the wire
// ----------------------------------------------------------------------

/// Every request carries the API key and a pinned API version. Pinecone
/// rejects a request without either, and an unpinned version is how a
/// wire-shape change becomes a silent outage.
#[tokio::test]
async fn every_request_is_authenticated_and_version_pinned() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/describe_index_stats"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "namespaces": {} })))
        .mount(&server)
        .await;

    driver_against(&server).count(TEST_INDEX).await.unwrap();

    let request = only_request(&server).await;
    assert_eq!(
        request.headers.get("Api-Key").map(|v| v.to_str().unwrap()),
        Some(TEST_KEY),
        "Pinecone authenticates on the Api-Key header"
    );
    assert_eq!(
        request
            .headers
            .get("X-Pinecone-Api-Version")
            .map(|v| v.to_str().unwrap()),
        Some(DEFAULT_API_VERSION),
        "the REST API version must be pinned, not left to the server's default"
    );
}

#[tokio::test]
async fn upsert_posts_pinecones_documented_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/vectors/upsert"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "upsertedCount": 2 })))
        .mount(&server)
        .await;

    driver_against(&server)
        .with_namespace("tenant-a")
        .upsert(
            TEST_INDEX,
            vec![
                VectorItem::new("a", vec![1.0, 0.0], json!({ "tag": "x" })),
                VectorItem::new("b", vec![0.0, 1.0], serde_json::Value::Null),
            ],
        )
        .await
        .expect("upsert succeeds");

    let body: serde_json::Value = only_request(&server).await.body_json().expect("json body");
    assert_eq!(body["namespace"], "tenant-a");
    assert_eq!(body["vectors"][0]["id"], "a");
    assert_eq!(body["vectors"][0]["values"], json!([1.0, 0.0]));
    assert_eq!(body["vectors"][0]["metadata"]["tag"], "x");
    assert!(
        body["vectors"][1].get("metadata").is_none(),
        "null metadata must be omitted, not sent as JSON null: {body}"
    );
}

/// The field names Pinecone's `/query` endpoint expects are camelCase.
/// Getting one wrong is a 400 in production and invisible in a unit test
/// that never serializes.
#[tokio::test]
async fn similar_posts_camel_case_query_fields_and_decodes_matches() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "matches": [
                { "id": "perfect", "score": 1.0, "metadata": { "title": "Hello" } },
                { "id": "close", "score": 0.8 }
            ],
            "namespace": "",
            "usage": { "readUnits": 5 }
        })))
        .mount(&server)
        .await;

    let hits = driver_against(&server)
        .similar(TEST_INDEX, vec![1.0, 0.0], 2)
        .await
        .expect("query succeeds");

    let body: serde_json::Value = only_request(&server).await.body_json().expect("json body");
    assert_eq!(body["topK"], 2);
    assert_eq!(body["includeMetadata"], true);
    assert_eq!(body["includeValues"], false);
    assert_eq!(body["vector"], json!([1.0, 0.0]));
    assert_eq!(body["namespace"], "");

    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, "perfect");
    assert_eq!(hits[0].metadata["title"], "Hello");
    assert_eq!(
        hits[1].metadata,
        serde_json::Value::Null,
        "a hit Pinecone returns without metadata decodes to null, not an error"
    );
}

#[tokio::test]
async fn delete_posts_the_id_list_and_namespace() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/vectors/delete"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;

    driver_against(&server)
        .with_namespace("tenant-b")
        .delete(TEST_INDEX, vec!["toss".to_string(), "also".to_string()])
        .await
        .expect("delete succeeds");

    let body: serde_json::Value = only_request(&server).await.body_json().expect("json body");
    assert_eq!(body["ids"], json!(["toss", "also"]));
    assert_eq!(body["namespace"], "tenant-b");
}

/// `count` reads a per-namespace summary. The default namespace lives
/// under an empty-string key, and a namespace that has never been written
/// to is absent rather than zero - so "missing" must mean 0, not an error.
#[tokio::test]
async fn count_reads_this_drivers_namespace_out_of_the_stats_map() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/describe_index_stats"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "namespaces": {
                "": { "vectorCount": 50 },
                "tenant-a": { "vectorCount": 7 }
            },
            "dimension": 2,
            "totalVectorCount": 57
        })))
        .mount(&server)
        .await;

    assert_eq!(
        driver_against(&server).count(TEST_INDEX).await.unwrap(),
        50,
        "the default namespace is keyed by the empty string"
    );
    assert_eq!(
        driver_against(&server)
            .with_namespace("tenant-a")
            .count(TEST_INDEX)
            .await
            .unwrap(),
        7
    );
    assert_eq!(
        driver_against(&server)
            .with_namespace("never-written")
            .count(TEST_INDEX)
            .await
            .unwrap(),
        0,
        "an absent namespace holds no vectors; it is not an error"
    );
}

/// The control plane resolves an index host, and the driver must reach it
/// over TLS regardless of what the response says - a scheme taken from a
/// response body is a scheme an attacker who can answer for the control
/// plane gets to choose.
#[tokio::test]
async fn index_host_resolves_through_the_control_plane_and_forces_https() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/indexes/docs-index"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "name": "docs-index",
            "dimension": 2,
            "metric": "cosine",
            "host": "http://docs-index-abc123.svc.aped-1234.pinecone.io",
            "status": { "ready": true, "state": "Ready" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let driver = PineconeVectorDriver::from_api_key(TEST_KEY)
        .unwrap()
        .with_control_plane(server.uri());

    let host = driver.index_host(TEST_INDEX).await.expect("resolves");
    assert_eq!(
        host.as_str(),
        "https://docs-index-abc123.svc.aped-1234.pinecone.io"
    );

    // Resolved once, then cached: `.expect(1)` above is verified when the
    // server drops, so a second control-plane call would fail this test.
    let again = driver.index_host(TEST_INDEX).await.expect("cached");
    assert_eq!(again.as_str(), host.as_str());
}

#[tokio::test]
async fn a_host_pinned_by_the_operator_skips_the_control_plane_entirely() {
    let control = MockServer::start().await;
    let data = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/describe_index_stats"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "namespaces": {} })))
        .mount(&data)
        .await;

    PineconeVectorDriver::from_api_key(TEST_KEY)
        .unwrap()
        .with_control_plane(control.uri())
        .with_index_host(TEST_INDEX, data.uri())
        .count(TEST_INDEX)
        .await
        .unwrap();

    assert!(
        control
            .received_requests()
            .await
            .expect("recorded")
            .is_empty(),
        "pinning a host must remove the control-plane round trip, not just prefer it"
    );
}

/// An error must name the status and carry a slice of the body - and must
/// never carry the API key, which is the one thing in scope that would be
/// catastrophic in a log.
#[tokio::test]
async fn a_failed_call_surfaces_status_and_body_but_never_the_api_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/query"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "code": 3,
            "message": "Vector dimension 2 does not match the dimension of the index 1024"
        })))
        .mount(&server)
        .await;

    let err = driver_against(&server)
        .similar(TEST_INDEX, vec![1.0, 0.0], 1)
        .await
        .unwrap_err();
    let rendered = format!("{err}");

    assert!(
        rendered.contains("400"),
        "the status must be named: {rendered}"
    );
    assert!(
        rendered.contains("does not match the dimension"),
        "the provider's explanation must survive: {rendered}"
    );
    assert!(
        !rendered.contains(TEST_KEY),
        "the API key must never reach an error message: {rendered}"
    );
}

/// A non-2xx response must not be parsed as a result. Pinecone answering
/// `{"matches": []}` with a 500 means "we failed", not "no hits".
#[tokio::test]
async fn a_server_error_is_not_decoded_as_an_empty_result() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/query"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({ "matches": [] })))
        .mount(&server)
        .await;

    let result = driver_against(&server)
        .similar(TEST_INDEX, vec![1.0, 0.0], 1)
        .await;
    assert!(
        result.is_err(),
        "a 500 whose body happens to parse must still be an error, got {result:?}"
    );
}

/// The trapdoor is the reason dropping the SDK doesn't cost power users
/// anything: it reaches any endpoint, with their own request and response
/// types, over the driver's authenticated and host-resolved transport.
#[tokio::test]
async fn the_data_plane_trapdoor_reaches_an_endpoint_the_trait_does_not_cover() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/vectors/fetch_by_metadata"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "vectors": [{ "id": "doc-1", "values": [1.0, 0.0] }]
        })))
        .mount(&server)
        .await;

    #[derive(serde::Deserialize)]
    struct FetchResponse {
        vectors: Vec<PineconeVector>,
    }

    let response: FetchResponse = driver_against(&server)
        .data_plane_post(
            TEST_INDEX,
            "/vectors/fetch_by_metadata",
            &json!({ "filter": { "genre": { "$eq": "comedy" } }, "limit": 2 }),
        )
        .await
        .expect("trapdoor call succeeds");

    assert_eq!(response.vectors[0].id, "doc-1");

    let body: serde_json::Value = only_request(&server).await.body_json().expect("json body");
    assert_eq!(body["filter"]["genre"]["$eq"], "comedy");
}

/// A store name reaches a URL path. A crafted one must not walk out of
/// `/indexes/` into some other control-plane endpoint.
#[tokio::test]
async fn a_crafted_store_name_cannot_reach_another_control_plane_endpoint() {
    let server = MockServer::start().await;
    // The only mounted route is the one a traversal would be aiming for.
    Mock::given(method("GET"))
        .and(path("/api-keys"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "host": "leaked" })))
        .expect(0)
        .mount(&server)
        .await;

    let driver = PineconeVectorDriver::from_api_key(TEST_KEY)
        .unwrap()
        .with_control_plane(server.uri());

    // Unmatched by any mock, so this 404s - the point is *which* path was
    // requested, which `.expect(0)` above asserts on drop.
    let _ = driver.index_host("../api-keys").await;

    let requested = only_request(&server).await;
    assert_eq!(
        requested.url.path(),
        "/indexes/..%2Fapi-keys",
        "the store name must stay inside its path segment"
    );
}

// ----------------------------------------------------------------------
// Integration tests - env-gated, require a real Pinecone account
// ----------------------------------------------------------------------

fn unique_namespace(tag: &str) -> String {
    format!(
        "p9a_t3_{tag}_{}",
        std::time::UNIX_EPOCH.elapsed().unwrap().as_nanos()
    )
}

fn pinecone_env_or_skip(test_name: &str) -> Option<(String, String)> {
    match (
        std::env::var("PINECONE_API_KEY").ok(),
        std::env::var("PINECONE_TEST_INDEX").ok(),
    ) {
        (Some(a), Some(i)) => Some((a, i)),
        _ => {
            eprintln!(
                "[{test_name}] skipping: set PINECONE_API_KEY and PINECONE_TEST_INDEX to run"
            );
            None
        }
    }
}

async fn delete_namespace(driver: &PineconeVectorDriver, index_name: &str, namespace: &str) {
    let _: Result<serde_json::Value, _> = driver
        .data_plane_post(
            index_name,
            "/vectors/delete",
            &json!({ "deleteAll": true, "namespace": namespace }),
        )
        .await;
}

#[tokio::test]
#[ignore = "requires PINECONE_API_KEY and PINECONE_TEST_INDEX"]
async fn integration_upsert_and_count_roundtrip() {
    let Some((key, index_name)) = pinecone_env_or_skip("upsert_and_count_roundtrip") else {
        return;
    };
    let ns = unique_namespace("count");
    let driver = PineconeVectorDriver::from_api_key(&key)
        .unwrap()
        .with_namespace(&ns);

    // Use 4-dim vectors - common for small test indexes. If the user's
    // index has a different dim, Pinecone rejects this and the assertion
    // surfaces its message.
    driver
        .upsert(
            &index_name,
            vec![
                VectorItem::new("a", vec![1.0, 0.0, 0.0, 0.0], json!({"tag": 1})),
                VectorItem::new("b", vec![0.0, 1.0, 0.0, 0.0], json!({"tag": 2})),
            ],
        )
        .await
        .expect("upsert succeeds; if your index dim != 4 this is the failure surface");

    // Pinecone's stats endpoint is eventually consistent; poll briefly so
    // flakes don't fail the test.
    let mut count = 0;
    for _ in 0..10 {
        count = driver.count(&index_name).await.unwrap();
        if count >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert_eq!(count, 2, "two vectors should be present in the namespace");

    delete_namespace(&driver, &index_name, &ns).await;
}

#[tokio::test]
#[ignore = "requires PINECONE_API_KEY and PINECONE_TEST_INDEX"]
async fn integration_similar_returns_top_k_descending() {
    let Some((key, index_name)) = pinecone_env_or_skip("similar_top_k") else {
        return;
    };
    let ns = unique_namespace("topk");
    let driver = PineconeVectorDriver::from_api_key(&key)
        .unwrap()
        .with_namespace(&ns);

    driver
        .upsert(
            &index_name,
            vec![
                VectorItem::new("perfect", vec![1.0, 0.0, 0.0, 0.0], json!({})),
                VectorItem::new("orthogonal", vec![0.0, 1.0, 0.0, 0.0], json!({})),
                VectorItem::new("close", vec![0.9, 0.1, 0.0, 0.0], json!({})),
            ],
        )
        .await
        .unwrap();

    let mut hits = vec![];
    for _ in 0..10 {
        hits = driver
            .similar(&index_name, vec![1.0, 0.0, 0.0, 0.0], 3)
            .await
            .unwrap();
        if hits.len() == 3 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0].id, "perfect");
    assert!(hits[0].score >= hits[1].score);
    assert!(hits[1].score >= hits[2].score);

    delete_namespace(&driver, &index_name, &ns).await;
}

#[tokio::test]
#[ignore = "requires PINECONE_API_KEY and PINECONE_TEST_INDEX"]
async fn integration_delete_removes_points_by_id() {
    let Some((key, index_name)) = pinecone_env_or_skip("delete_by_id") else {
        return;
    };
    let ns = unique_namespace("del");
    let driver = PineconeVectorDriver::from_api_key(&key)
        .unwrap()
        .with_namespace(&ns);

    driver
        .upsert(
            &index_name,
            vec![
                VectorItem::new("keep", vec![1.0, 0.0, 0.0, 0.0], json!({})),
                VectorItem::new("toss", vec![0.0, 1.0, 0.0, 0.0], json!({})),
            ],
        )
        .await
        .unwrap();
    driver
        .delete(&index_name, vec!["toss".to_string()])
        .await
        .unwrap();

    let mut count = 99;
    for _ in 0..10 {
        count = driver.count(&index_name).await.unwrap();
        if count <= 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert_eq!(count, 1);

    delete_namespace(&driver, &index_name, &ns).await;
}

#[tokio::test]
#[ignore = "requires PINECONE_API_KEY and PINECONE_TEST_INDEX"]
async fn integration_metadata_roundtrips_through_pinecone() {
    let Some((key, index_name)) = pinecone_env_or_skip("metadata_roundtrip") else {
        return;
    };
    let ns = unique_namespace("meta");
    let driver = PineconeVectorDriver::from_api_key(&key)
        .unwrap()
        .with_namespace(&ns);

    driver
        .upsert(
            &index_name,
            vec![VectorItem::new(
                "doc-1",
                vec![1.0, 0.0, 0.0, 0.0],
                json!({ "title": "Hello", "score_field": 4.5, "active": true }),
            )],
        )
        .await
        .unwrap();

    let mut hits = vec![];
    for _ in 0..10 {
        hits = driver
            .similar(&index_name, vec![1.0, 0.0, 0.0, 0.0], 1)
            .await
            .unwrap();
        if !hits.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "doc-1");
    assert_eq!(hits[0].metadata["title"], "Hello");
    assert_eq!(hits[0].metadata["score_field"], 4.5);
    assert_eq!(hits[0].metadata["active"], true);

    delete_namespace(&driver, &index_name, &ns).await;
}

/// The whole point of the REST rewrite: the driver reaches a live account
/// with no `pinecone-sdk`, no `tonic`, and no `rustls-webpki 0.102` in the
/// build. Cheapest possible live proof that auth and the version header
/// are actually accepted, independent of any index's dimension.
#[tokio::test]
#[ignore = "requires PINECONE_API_KEY and PINECONE_TEST_INDEX"]
async fn integration_control_plane_resolves_a_real_index_host() {
    let Some((key, index_name)) = pinecone_env_or_skip("control_plane_host") else {
        return;
    };
    let driver = PineconeVectorDriver::from_api_key(&key).unwrap();
    let host = driver
        .index_host(&index_name)
        .await
        .expect("describe_index reaches Pinecone and returns a host");
    assert!(
        host.starts_with("https://"),
        "the data plane must be reached over TLS, got {host}"
    );
}
