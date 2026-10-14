//! Retained search facts bind clicks to the server session and the shown position.
use super::derive::{SearchNote, database, note};
use sea_orm_migration::{MigrationTrait, SchemaManager};
use std::sync::Arc;
use suprnova::{DB, Search, SearchConfig, SearchDriver, SearchRegistry, Searchable, Viewer};

async fn setup() -> suprnova::testing::TestDatabase {
    let db = database().await;
    let connection = DB::connection().unwrap();
    let manager = SchemaManager::new(connection.inner());
    for migration in suprnova::search::migrations() {
        migration.up(&manager).await.unwrap();
    }
    suprnova::search::migrations::CreateSearchAnalytics
        .up(&manager)
        .await
        .unwrap();
    for migration in suprnova::recommendations::migrations() {
        migration.up(&manager).await.unwrap();
    }
    db
}
fn config(on: bool, viewer: bool) -> SearchConfig {
    SearchConfig {
        driver: SearchDriver::Collection,
        queue: false,
        analytics: on,
        analytics_viewer: viewer,
        ..Default::default()
    }
}
async fn index(model: &SearchNote) {
    let document = model.to_search_document().unwrap();
    use suprnova::queue::Job;
    suprnova::SearchIndexJob {
        index: document.index,
        key: document.key,
        source_version: document.source_version,
        op: suprnova::IndexOperation::Index,
        generation: None,
    }
    .handle()
    .await
    .unwrap();
}
async fn count(table: &str) -> i64 {
    DB::scalar(&format!("SELECT COUNT(*) FROM {table}"), [])
        .await
        .unwrap()
}
struct User;
impl suprnova::Authenticatable for User {
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
fn owner() -> Viewer {
    Viewer::Authenticated {
        id: "9".into(),
        user: Arc::new(User),
    }
}

#[tokio::test]
async fn analytics_off_writes_nothing_and_on_records_query_and_shown_keys() {
    let _db = setup().await;
    let slot = suprnova::session::new_session_slot_for_test();
    suprnova::session::session_scope_for_test(
        slot,
        SearchRegistry::isolated(async {
            SearchConfig::scope(config(false, false), async {
                let note = note("Searchable rust", true, 0).await;
                index(&note).await;
                assert!(
                    SearchNote::search("rust")
                        .get()
                        .await
                        .unwrap()
                        .impression
                        .is_none()
                );
                assert_eq!(count("search_queries").await, 0);
                assert_eq!(count("search_impressions").await, 0);
                assert_eq!(count("search_clicks").await, 0);
                assert_eq!(count("recommendation_events").await, 0);
                assert!(Search::click("unknown", "unknown", 0).await.is_err());
            })
            .await
            .unwrap();
            SearchConfig::scope(config(true, false), async {
                let result = SearchNote::search("rust")
                    .filter("enabled = true")
                    .get()
                    .await
                    .unwrap();
                assert!(result.impression.is_some());
                let row = DB::select("SELECT * FROM search_queries", [])
                    .await
                    .unwrap()
                    .remove(0);
                assert_eq!(row.get_string("query").unwrap(), "rust");
                assert_eq!(row.get_int("hits").unwrap(), 1);
                assert!(row.get("viewer_id").unwrap().is_null());
                assert!(row.get_string("filters").unwrap().contains("enabled"));
                let row = DB::select("SELECT * FROM search_impressions", [])
                    .await
                    .unwrap()
                    .remove(0);
                assert_ne!(row.get_string("session_hash").unwrap(), "test_session");
                let keys: serde_json::Value =
                    serde_json::from_str(&row.get_string("keys").unwrap()).unwrap();
                assert_eq!(keys[0]["position"], 0);
                assert_eq!(keys[0]["key"], result.hits[0].key);
            })
            .await
            .unwrap();
        }),
    )
    .await;
}

#[tokio::test]
async fn viewer_ids_require_both_opt_in_and_authenticated_viewer() {
    let _db = setup().await;
    SearchRegistry::isolated(async {
        for (enabled, viewer) in [(false, owner()), (true, Viewer::Guest), (true, owner())] {
            SearchConfig::scope(config(true, enabled), async {
                SearchNote::search("viewer")
                    .for_viewer(viewer)
                    .get()
                    .await
                    .unwrap();
            })
            .await
            .unwrap();
        }
        let rows = DB::select(
            "SELECT viewer_id FROM search_queries ORDER BY created_at, id",
            [],
        )
        .await
        .unwrap();
        assert!(rows[0].get("viewer_id").unwrap().is_null());
        assert!(rows[1].get("viewer_id").unwrap().is_null());
        assert_eq!(rows[2].get_string("viewer_id").unwrap(), "9");
        assert_eq!(count("search_impressions").await, 0);
    })
    .await;
}

#[tokio::test]
async fn clicks_reject_wrong_session_position_unknown_key_and_missing_impression() {
    let _db = setup().await;
    SearchConfig::scope(
        config(true, false),
        SearchRegistry::isolated(async {
            let note = note("Searchable rust", true, 0).await;
            index(&note).await;
            let slot = suprnova::session::new_session_slot_for_test();
            let result = suprnova::session::session_scope_for_test(
                slot.clone(),
                SearchNote::search("rust").get(),
            )
            .await
            .unwrap();
            let impression = result.impression.unwrap();
            let key = note.id.to_string();
            for (id, key, position) in [
                (&impression, key.as_str(), 1),
                (&impression, "unknown", 0),
                (&"unknown".to_string(), key.as_str(), 0),
            ] {
                assert!(
                    suprnova::session::session_scope_for_test(
                        slot.clone(),
                        Search::click(id, key, position)
                    )
                    .await
                    .is_err()
                );
            }
            let wrong = suprnova::session::new_session_slot_for_test();
            wrong.lock().unwrap().as_mut().unwrap().id = "different-session".into();
            assert!(
                suprnova::session::session_scope_for_test(
                    wrong,
                    Search::click(&impression, &key, 0)
                )
                .await
                .is_err()
            );
            assert_eq!(count("search_clicks").await, 0);
            suprnova::session::session_scope_for_test(
                slot.clone(),
                Search::click(&impression, &key, 0),
            )
            .await
            .unwrap();
            assert_eq!(count("search_clicks").await, 1);
            let report = suprnova::search::analytics::report(Some("test_search_notes"), 1)
                .await
                .unwrap();
            assert_eq!(report.click_positions.len(), 1);
            assert_eq!(report.click_positions[0].position, 0);
            assert_eq!(report.click_positions[0].count, 1);
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn suggestions_fold_case_order_frequency_and_exclude_expired_queries() {
    let _db = setup().await;
    SearchConfig::scope(
        config(true, false),
        SearchRegistry::isolated(async {
            for query in ["RÜST", "rüst", "rüst language", "unrelated", "ancient"] {
                SearchNote::search(query).get().await.unwrap();
            }
            DB::affecting_statement(
                "UPDATE search_queries SET created_at = ? WHERE query = ?",
                vec![0_i64.into(), "ancient".into()],
            )
            .await
            .unwrap();
            assert_eq!(
                Search::suggest("test_search_notes", "RÜ", 2).await.unwrap(),
                vec!["rüst", "rüst language"]
            );
            assert!(
                Search::suggest("test_search_notes", "anc", 10)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert!(Search::suggest("unknown-index", "rust", 10).await.is_err());
            SearchNote::search("other-surface").get().await.unwrap();
            DB::affecting_statement(
                r#"UPDATE search_queries SET "index"=? WHERE query=?"#,
                vec!["search_command_notes".into(), "other-surface".into()],
            )
            .await
            .unwrap();
            assert!(
                Search::suggest("test_search_notes", "other", 10)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(
                Search::suggest("search_command_notes", "other", 10)
                    .await
                    .unwrap(),
                vec!["other-surface"]
            );
            let report = suprnova::search::analytics::report(Some("test_search_notes"), 90)
                .await
                .unwrap();
            assert_eq!(report.top_queries[0].query, "rüst");
            assert_eq!(report.zero_result_queries.len(), 3);
            for args in [
                vec![
                    "search:analytics",
                    "--index=test_search_notes",
                    "--days",
                    "90",
                ],
                vec!["search:analytics-prune"],
            ] {
                suprnova::console::test(args)
                    .run()
                    .await
                    .assert_successful();
            }
            for args in [
                vec!["search:analytics", "--days=0"],
                vec!["search:analytics", "--index=unknown"],
                vec!["search:analytics-prune", "unknown"],
            ] {
                suprnova::console::test(args).run().await.assert_failed();
            }
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn prune_removes_expired_query_impression_and_click_rows_together() {
    let _db = setup().await;
    SearchConfig::scope(
        config(true, false),
        SearchRegistry::isolated(async {
            let note = note("Searchable rust", true, 0).await;
            index(&note).await;
            let slot = suprnova::session::new_session_slot_for_test();
            suprnova::session::session_scope_for_test(slot, async {
                let impression = SearchNote::search("rust")
                    .get()
                    .await
                    .unwrap()
                    .impression
                    .unwrap();
                Search::click(&impression, &note.id.to_string(), 0)
                    .await
                    .unwrap();
            })
            .await;
            for table in ["search_queries", "search_impressions", "search_clicks"] {
                DB::affecting_statement(
                    &format!("UPDATE {table} SET created_at = ?"),
                    vec![0_i64.into()],
                )
                .await
                .unwrap();
            }
            assert_eq!(suprnova::search::analytics::prune().await.unwrap(), 3);
            for table in ["search_queries", "search_impressions", "search_clicks"] {
                assert_eq!(count(table).await, 0);
            }
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn impressions_and_validated_clicks_forward_into_recommendations() {
    let _db = setup().await;
    SearchConfig::scope(config(true, false), SearchRegistry::isolated(async {
        let note = note("Searchable rust", true, 0).await; index(&note).await;
        suprnova::RecommendConfig::scope(suprnova::RecommendConfig { enabled: true, ..Default::default() }, async {
            let slot = suprnova::session::new_session_slot_for_test();
            suprnova::session::session_scope_for_test(slot, async {
                let impression = SearchNote::search("rust").get().await.unwrap().impression.unwrap();
                Search::click(&impression, &note.id.to_string(), 0).await.unwrap();
                let rows = DB::select("SELECT actor_kind,event_type,target_index,target_key FROM recommendation_events ORDER BY id", []).await.unwrap();
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0].get_string("event_type").unwrap(), "impression");
                assert_eq!(rows[1].get_string("event_type").unwrap(), "click");
                assert_eq!(rows[1].get_string("target_key").unwrap(), note.id.to_string());
                assert_eq!(rows[1].get_string("actor_kind").unwrap(), "guest");
            }).await;
        }).await.unwrap();
    })).await.unwrap();
}

#[tokio::test]
async fn suggestions_use_full_unicode_folding_and_literal_prefix_characters() {
    let _db = setup().await;
    SearchConfig::scope(
        config(true, false),
        SearchRegistry::isolated(async {
            for query in [
                "Straße",
                "STRASSE",
                "stranger",
                "Σ",
                "ς",
                "σ",
                "100%_safe",
                "100percent",
                "100%xsafe",
            ] {
                SearchNote::search(query).get().await.unwrap();
            }
            assert_eq!(
                Search::suggest("test_search_notes", "STRAẞ", 10)
                    .await
                    .unwrap(),
                vec!["strasse"]
            );
            assert_eq!(
                Search::suggest("test_search_notes", "ς", 10).await.unwrap(),
                vec!["σ"]
            );
            assert_eq!(
                Search::suggest("test_search_notes", "100%_", 10)
                    .await
                    .unwrap(),
                vec!["100%_safe"]
            );
        }),
    )
    .await
    .unwrap();
}

struct SessionScope(Arc<std::sync::Mutex<Option<suprnova::session::SessionData>>>);
#[async_trait::async_trait]
impl suprnova::Middleware for SessionScope {
    async fn handle(&self, request: suprnova::Request, next: suprnova::Next) -> suprnova::Response {
        suprnova::session::session_scope_for_test(self.0.clone(), next(request)).await
    }
}

#[tokio::test]
async fn analytics_endpoint_returns_a_session_impression_and_declared_hit_projection() {
    let _db = setup().await;
    SearchConfig::scope(
        config(true, false),
        SearchRegistry::isolated(async {
            let note = note("Rust <script>guide</script>", true, 0).await;
            index(&note).await;
            let slot = suprnova::session::new_session_slot_for_test();
            let router = Search::routes(&["test_search_notes"]).unwrap();
            let middleware = suprnova::MiddlewareRegistry::new().append(SessionScope(slot));
            let client = suprnova::testing::TestClient::new(router, middleware);
            let response = client
                .get("/search?index=test_search_notes&q=rust&facets=category&page=1&per_page=5")
                .header("Accept", "application/json")
                .send()
                .await;
            response.assert_ok();
            let body = response.json();
            let impression = body["impression"].as_str().unwrap();
            assert!(uuid::Uuid::parse_str(impression).is_ok());
            let hit = &body["hits"][0];
            assert_eq!(hit["key"], note.id.to_string());
            assert_eq!(
                hit["attributes"]["title"],
                serde_json::json!({"type": "text", "value": note.title})
            );
            assert!(hit["score"].is_number());
            assert!(hit["normalized_score"].is_number());
            assert!(
                hit["formatted"]["title"]
                    .as_str()
                    .unwrap()
                    .contains("&lt;script&gt;")
            );
            assert!(hit.get("model").is_none());
            for undeclared in ["id", "lat", "lng", "deleted_at"] {
                assert!(hit["attributes"].get(undeclared).is_none());
            }
            let rows = DB::select(
                "SELECT id FROM search_impressions WHERE id=?",
                vec![impression.to_owned().into()],
            )
            .await
            .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(count("search_queries").await, 1);
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn analytics_endpoint_requires_a_nonempty_server_session_before_recording() {
    let _db = setup().await;
    SearchConfig::scope(
        config(true, false),
        SearchRegistry::isolated(async {
            let note = note("Rust guide", true, 0).await;
            index(&note).await;
            let empty_slot = suprnova::session::new_session_slot_for_test();
            empty_slot.lock().unwrap().as_mut().unwrap().id.clear();
            for middleware in [
                suprnova::MiddlewareRegistry::new(),
                suprnova::MiddlewareRegistry::new().append(SessionScope(empty_slot)),
            ] {
                let router = Search::routes(&["test_search_notes"]).unwrap();
                let client = suprnova::testing::TestClient::new(router, middleware);
                client
                    .get("/search?index=test_search_notes&q=rust")
                    .header("Accept", "application/json")
                    .send()
                    .await
                    .assert_status(500);
                assert_eq!(count("search_queries").await, 0);
                assert_eq!(count("search_impressions").await, 0);
            }
        }),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn click_route_runs_session_validation_and_csrf_through_handle_request() {
    let _db = setup().await;
    SearchConfig::scope(config(true, false), SearchRegistry::isolated(async {
        let note = note("Searchable rust", true, 0).await; index(&note).await;
        let slot = suprnova::session::new_session_slot_for_test();
        let impression = suprnova::session::session_scope_for_test(slot.clone(), SearchNote::search("rust").get()).await.unwrap().impression.unwrap();
        let router = Search::routes(&["test_search_notes"]).unwrap();
        let middleware = suprnova::MiddlewareRegistry::new().append(SessionScope(slot.clone()));
        let client = suprnova::testing::TestClient::new(router, middleware);
        let valid = serde_json::json!({"impression": impression, "key": note.id.to_string(), "position": 0});
        let response = client.post("/search/click").header("Accept", "application/json").header("X-CSRF-TOKEN", "test_csrf_token").json(&valid).send().await;
        assert!((200..300).contains(&response.status()), "{}", response.json());
        for invalid in [serde_json::json!({"impression": impression, "key": "unknown", "position": 0}), serde_json::json!({"impression": impression, "key": note.id.to_string(), "position": 1})] {
            client.post("/search/click").header("Accept", "application/json").header("X-CSRF-TOKEN", "test_csrf_token").json(&invalid).send().await.assert_status(422);
        }
        slot.lock().unwrap().as_mut().unwrap().id = "different-session".into();
        client.post("/search/click").header("Accept", "application/json").header("X-CSRF-TOKEN", "test_csrf_token").json(&valid).send().await.assert_status(422);
        client.post("/search/click").header("Accept", "application/json").json(&valid).send().await.assert_status(419);
        assert_eq!(count("search_clicks").await, 1);
    })).await.unwrap();
}
