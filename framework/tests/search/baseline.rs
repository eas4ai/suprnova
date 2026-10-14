//! Judged retrieval and warm latency use the same fixture and floors on every engine.
use std::{collections::BTreeMap, path::PathBuf};
use suprnova::{
    FrameworkError,
    search::{
        CollectionEngine,
        baseline::{Baseline, BaselineFloors, Fixture, Latency, ndcg_at_10},
    },
};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/search/fixtures")
}

#[test]
fn baseline_ndcg_hand_computed() -> Result<(), FrameworkError> {
    let qrels = BTreeMap::from([("a".into(), 3), ("b".into(), 2), ("c".into(), 1)]);
    let ideal = 3.0 + 2.0 / 3.0_f64.log2() + 0.5;
    let keys = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(ndcg_at_10(&keys(&["a", "b", "c"]), &qrels)?, 1.0);
    let expected = (1.0 + 3.0 / 3.0_f64.log2() + 1.0) / ideal;
    assert!((ndcg_at_10(&keys(&["c", "a", "b"]), &qrels)? - expected).abs() < 1e-12);
    assert_eq!(ndcg_at_10(&keys(&["missing"]), &qrels)?, 0.0);
    assert_eq!(ndcg_at_10(&[], &qrels)?, 0.0);
    assert_eq!(ndcg_at_10(&keys(&["a"]), &BTreeMap::new())?, 0.0);
    // Duplicate hits cannot gain relevance twice; their positions still consume rank.
    assert!((ndcg_at_10(&keys(&["a", "a", "b"]), &qrels)? - 4.0 / ideal).abs() < 1e-12);
    let mut below_cutoff = keys(&["x"; 10]);
    below_cutoff.push("a".into());
    assert_eq!(ndcg_at_10(&below_cutoff, &qrels)?, 0.0);
    assert!(ndcg_at_10(&[], &BTreeMap::from([("bad".into(), -1)])).is_err());
    Ok(())
}

#[test]
fn baseline_latency_nearest_rank() -> Result<(), FrameworkError> {
    let samples = (1..=100).map(f64::from).rev().collect::<Vec<_>>();
    let latency = Latency::from_samples(&samples)?;
    assert_eq!(
        (latency.p50_ms, latency.p95_ms, latency.p99_ms),
        (50.0, 95.0, 99.0)
    );
    assert!(Latency::from_samples(&[]).is_err());
    assert!(Latency::from_samples(&[f64::NAN]).is_err());
    assert!(Latency::from_samples(&[-1.0]).is_err());
    Ok(())
}

#[test]
fn baseline_fixture_counts_and_integrity() -> Result<(), FrameworkError> {
    let fixture = Fixture::load(fixtures())?;
    assert_eq!(fixture.corpus.len(), 5183);
    assert_eq!(fixture.queries.len(), 300);
    assert_eq!(
        fixture.qrels.values().map(BTreeMap::len).sum::<usize>(),
        339
    );
    for judgments in fixture.qrels.values() {
        for key in judgments.keys() {
            assert!(fixture.corpus.iter().any(|d| &d.id == key));
        }
    }
    assert!(fixture.license.contains("CC BY 4.0"));
    assert!(fixture.license.contains("ODC-By 1.0"));
    Ok(())
}

#[test]
fn baseline_fixture_rejects_broken_references_and_rows() -> Result<(), FrameworkError> {
    let directory = tempfile::tempdir().map_err(|e| FrameworkError::internal(e.to_string()))?;
    let put = |name: &str, text: &str| std::fs::write(directory.path().join(name), text).unwrap();
    put("LICENSE", "CC BY 4.0 and ODC-By 1.0");
    put(
        "corpus.jsonl",
        "{\"id\":\"d\",\"title\":\"title\",\"text\":\"body\"}\n",
    );
    put("queries.jsonl", "{\"id\":\"q\",\"text\":\"query\"}\n");
    put("qrels.tsv", "query_id\tdocument_id\trelevance\nq\td\t1\n");
    Fixture::load(directory.path())?;
    put(
        "qrels.tsv",
        "query_id\tdocument_id\trelevance\nq\tmissing\t1\n",
    );
    assert!(
        Fixture::load(directory.path())
            .unwrap_err()
            .to_string()
            .contains("qrels.tsv")
    );
    put("qrels.tsv", "query_id\tdocument_id\trelevance\nq\td\t-1\n");
    assert!(Fixture::load(directory.path()).is_err());
    put(
        "qrels.tsv",
        "query_id\tdocument_id\trelevance\nq\td\t1\nq\td\t1\n",
    );
    assert!(Fixture::load(directory.path()).is_err());
    put("qrels.tsv", "query_id\tdocument_id\trelevance\nq\td\t1\n");
    put(
        "queries.jsonl",
        "{\"id\":\"q\",\"text\":\"query\"}\n{\"id\":\"q\",\"text\":\"query\"}\n",
    );
    assert!(Fixture::load(directory.path()).is_err());
    put("queries.jsonl", "invalid JSON\n");
    assert!(
        Fixture::load(directory.path())
            .unwrap_err()
            .to_string()
            .contains("queries.jsonl")
    );
    Ok(())
}

#[test]
fn baseline_uncalibrated_gate_is_closed() -> Result<(), FrameworkError> {
    let mut floors = BaselineFloors::load(fixtures().join("baseline.json"))?;
    floors.calibrated = false;
    assert!(floors.require_calibrated(false).is_err());
    floors.require_calibrated(true)?;
    Ok(())
}

async fn baseline_database() -> Result<suprnova::testing::TestContainerGuard, FrameworkError> {
    use sea_orm_migration::MigratorTrait;
    let guard = suprnova::testing::TestContainer::fake();
    // A complete fixture run can outlive SQLx's default 30-minute lifetime.
    let config = suprnova::DatabaseConfig::builder()
        .url("sqlite::memory:")
        .min_connections(1)
        .max_connections(1)
        .idle_timeout(0)
        .max_lifetime(0)
        .logging(false)
        .build();
    let connection = suprnova::DbConnection::connect(&config).await?;
    suprnova::testing::TestContainer::singleton(connection);
    super::database_support::Migrator::up(suprnova::DB::connection()?.inner(), None)
        .await
        .map_err(|error| {
            FrameworkError::database(format!("baseline metadata migrations: {error}"))
        })?;
    Ok(guard)
}

async fn record(
    engine: &str,
    driver: &dyn suprnova::search::SearchEngine,
    hybrid: bool,
) -> Result<(), FrameworkError> {
    let allow = std::env::var("SEARCH_BASELINE_ALLOW_UNCALIBRATED").as_deref() == Ok("1");
    BaselineFloors::load(fixtures().join("baseline.json"))?.require_calibrated(allow)?;
    let baseline = Baseline::load(fixtures())?;
    let provider = if hybrid {
        suprnova::search::Search::boot().await?;
        Some(suprnova::search::SearchRegistry::embedder_named(
            "search_baseline",
            suprnova::search::SearchConfig::current()?.embedder.as_str(),
        )?)
    } else {
        None
    };
    let result = baseline.run(engine, driver, provider.as_deref()).await?;
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/search-baseline");
    result.write(output.join(format!("{engine}.json")))?;
    if hybrid {
        result.write(output.join(format!("{engine}-hybrid.json")))?;
    }
    baseline.assert_record(
        &result,
        std::env::var("SEARCH_BASELINE_ALLOW_UNCALIBRATED").as_deref() == Ok("1"),
    )
}

#[tokio::test]
async fn baseline_collection() -> Result<(), FrameworkError> {
    record("collection", &CollectionEngine::new(), false).await
}

#[tokio::test]
async fn baseline_sqlite() -> Result<(), FrameworkError> {
    let _db = baseline_database().await?;
    record(
        "sqlite",
        &suprnova::search::DatabaseSearchEngine::new(),
        false,
    )
    .await
}

#[tokio::test]
#[ignore = "requires real SEARCH_EMBEDDER and its model cache or HTTP service"]
async fn baseline_collection_hybrid() -> Result<(), FrameworkError> {
    record("collection", &CollectionEngine::new(), true).await
}

#[tokio::test]
#[ignore = "requires real SEARCH_EMBEDDER and its model cache or HTTP service"]
async fn baseline_sqlite_hybrid() -> Result<(), FrameworkError> {
    let _db = baseline_database().await?;
    record(
        "sqlite",
        &suprnova::search::DatabaseSearchEngine::new(),
        true,
    )
    .await
}

#[tokio::test]
#[ignore = "requires disposable Postgres at PG_TEST_URL"]
async fn postgres_baseline() -> Result<(), FrameworkError> {
    let _db = super::database_support::live_database("PG_TEST_URL").await?;
    record(
        "postgres",
        &suprnova::search::DatabaseSearchEngine::new(),
        suprnova::search::SearchConfig::current()?.embedder
            != suprnova::search::SearchEmbedder::None,
    )
    .await
}

#[tokio::test]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_baseline() -> Result<(), FrameworkError> {
    let _db = super::database_support::live_database("MYSQL_TEST_URL").await?;
    record(
        "mysql",
        &suprnova::search::DatabaseSearchEngine::new(),
        suprnova::search::SearchConfig::current()?.embedder
            != suprnova::search::SearchEmbedder::None,
    )
    .await
}

#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn baseline_qdrant() -> Result<(), FrameworkError> {
    let _db = baseline_database().await?;
    let url = std::env::var("QDRANT_TEST_URL")
        .map_err(|_| FrameworkError::param("QDRANT_TEST_URL required"))?;
    let driver = suprnova::search::QdrantSearchEngine::from_url(&url, None, 30)?;
    record(
        "qdrant",
        &driver,
        suprnova::search::SearchConfig::current()?.embedder
            != suprnova::search::SearchEmbedder::None,
    )
    .await
}

#[tokio::test]
async fn baseline_calibration_uses_tasks_import_and_versions() -> Result<(), FrameworkError> {
    use suprnova::search::baseline::Calibration;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_partial_json, header, method, path_regex, query_param},
    };
    let meili = MockServer::start().await;
    let typesense = MockServer::start().await;
    for (path, verb, body) in [
        ("^/indexes$", "POST", serde_json::json!({"primaryKey":"id"})),
        (
            "^/indexes/[^/]+/settings$",
            "PATCH",
            serde_json::json!({"searchableAttributes":["title","text"]}),
        ),
    ] {
        Mock::given(method(verb))
            .and(path_regex(path))
            .and(header("Authorization", "Bearer test-meili"))
            .and(body_partial_json(body))
            .respond_with(
                ResponseTemplate::new(202).set_body_json(serde_json::json!({"taskUid":1})),
            )
            .expect(1)
            .mount(&meili)
            .await;
    }
    Mock::given(method("POST"))
        .and(path_regex("^/indexes/[^/]+/documents$"))
        .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({"taskUid":1})))
        .expect(1)
        .mount(&meili)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("^/tasks/1$"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"status":"succeeded"})),
        )
        .expect(4)
        .mount(&meili)
        .await;
    Mock::given(method("POST"))
        .and(path_regex("^/indexes/[^/]+/search$"))
        .and(body_partial_json(
            serde_json::json!({"q":"query","limit":10}),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"hits":[{"id":"d"}]})),
        )
        .expect(2)
        .mount(&meili)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("^/version$"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"pkgVersion":"1.test"})),
        )
        .expect(1)
        .mount(&meili)
        .await;
    Mock::given(method("DELETE"))
        .and(path_regex("^/indexes/[^/]+$"))
        .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({"taskUid":1})))
        .expect(1)
        .mount(&meili)
        .await;
    Mock::given(method("POST")).and(path_regex("^/collections$"))
        .and(header("X-TYPESENSE-API-KEY", "test-typesense"))
        .and(body_partial_json(serde_json::json!({"fields":[{"name":"title","type":"string"},{"name":"text","type":"string"}]})))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({}))).expect(1).mount(&typesense).await;
    Mock::given(method("POST"))
        .and(path_regex("^/collections/[^/]+/documents/import$"))
        .and(query_param("action", "create"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"success\":true}\n"))
        .expect(1)
        .mount(&typesense)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("^/collections/[^/]+/documents/search$"))
        .and(query_param("query_by", "title,text"))
        .and(query_param("q", "query"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"hits":[{"document":{"id":"d"}}]})),
        )
        .expect(2)
        .mount(&typesense)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("^/debug$"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"version":"29.test"})),
        )
        .expect(1)
        .mount(&typesense)
        .await;
    Mock::given(method("DELETE"))
        .and(path_regex("^/collections/[^/]+$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .expect(1)
        .mount(&typesense)
        .await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("corpus.jsonl"),
        "{\"id\":\"d\",\"title\":\"title\",\"text\":\"body\"}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("queries.jsonl"),
        "{\"id\":\"q\",\"text\":\"query\"}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("qrels.tsv"),
        "query_id\tdocument_id\trelevance\nq\td\t1\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("LICENSE"), "CC BY 4.0 and ODC-By 1.0").unwrap();
    let fixture = Fixture::load(dir.path())?;
    let floors = BaselineFloors::load(fixtures().join("baseline.json"))?;
    let calibrated = Calibration::new(
        &meili.uri(),
        "test-meili",
        &typesense.uri(),
        "test-typesense",
    )?
    .run(&fixture, &floors, 0.05)
    .await?;
    assert!(calibrated.calibrated);
    assert_eq!(
        calibrated.rivals.meilisearch.version.as_deref(),
        Some("1.test")
    );
    assert_eq!(
        calibrated.rivals.typesense.version.as_deref(),
        Some("29.test")
    );
    assert_eq!(
        calibrated
            .rivals
            .meilisearch
            .lexical
            .as_ref()
            .unwrap()
            .ndcg_at_10,
        1.0
    );
    assert_eq!(
        calibrated
            .rivals
            .typesense
            .lexical
            .as_ref()
            .unwrap()
            .ndcg_at_10,
        1.0
    );
    assert_eq!(calibrated.p95_budgets_ms, floors.p95_budgets_ms);
    meili.verify().await;
    typesense.verify().await;
    Ok(())
}

#[tokio::test]
async fn baseline_assertions_enforce_floors_budgets_and_identity() -> Result<(), FrameworkError> {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("corpus.jsonl"),
        "{\"id\":\"d\",\"title\":\"title\",\"text\":\"body\"}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("queries.jsonl"),
        "{\"id\":\"q\",\"text\":\"body\"}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("qrels.tsv"),
        "query_id\tdocument_id\trelevance\nq\td\t1\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("LICENSE"), "CC BY 4.0 and ODC-By 1.0").unwrap();
    let fixture = Fixture::load(dir.path())?;
    let mut floors = BaselineFloors::load(fixtures().join("baseline.json"))?;
    floors.calibrated = true;
    floors.fixture_sha256 = fixture.sha256;
    floors.calibrated_at = Some("2026-10-14T00:00:00Z".into());
    let measurement = suprnova::search::baseline::Measurement {
        ndcg_at_10: 0.8,
        queries: 1,
        latency: Latency::from_samples(&[1.0])?,
    };
    floors.rivals.meilisearch = suprnova::search::baseline::Rival {
        version: Some("test-meili".into()),
        lexical: Some(measurement.clone()),
    };
    let mut weaker = measurement;
    weaker.ndcg_at_10 = 0.5;
    floors.rivals.typesense = suprnova::search::baseline::Rival {
        version: Some("test-typesense".into()),
        lexical: Some(weaker),
    };
    floors.p95_budgets_ms.insert("collection".into(), 1000.0);
    floors.write(dir.path().join("baseline.json"))?;
    let baseline = Baseline::load(dir.path())?;
    let mut result = baseline
        .run("collection", &CollectionEngine::new(), None)
        .await?;
    assert_eq!(result.lexical.ndcg_at_10, 1.0);
    baseline.assert_record(&result, false)?;
    let scope = result.latency_scope.clone();
    result.latency_scope = "different timing boundary".into();
    assert!(baseline.assert_record(&result, false).is_err());
    result.latency_scope = scope;
    result.vocabulary_prepare_ms = -1.0;
    assert!(baseline.assert_record(&result, false).is_err());
    result.vocabulary_prepare_ms = 0.0;
    let _db = baseline_database().await?;
    let sqlite = suprnova::search::DatabaseSearchEngine::new();
    for _ in 0..2 {
        let sqlite_record = baseline.run("sqlite", &sqlite, None).await?;
        assert_eq!(sqlite_record.lexical.ndcg_at_10, 1.0);
        baseline.assert_record(&sqlite_record, false)?;
    }
    let tables = suprnova::DB::select(
        "SELECT name FROM sqlite_master WHERE name LIKE ?",
        vec!["search_search_baseline_%".into()],
    )
    .await?;
    assert!(
        tables.is_empty(),
        "baseline generations must clean up their physical tables"
    );
    result.lexical.ndcg_at_10 = 0.74;
    assert!(
        baseline
            .assert_record(&result, false)
            .unwrap_err()
            .to_string()
            .contains("nDCG@10")
    );
    result.lexical.ndcg_at_10 = 0.8;
    result.hybrid = Some(result.lexical.clone());
    result.model_profile = Some(suprnova::search::default_profile());
    result.model_provenance = Some(result.model_profile.as_ref().unwrap().provenance()?);
    assert!(
        baseline
            .assert_record(&result, false)
            .unwrap_err()
            .to_string()
            .contains("hybrid")
    );
    result.hybrid.as_mut().unwrap().ndcg_at_10 = 0.81;
    baseline.assert_record(&result, false)?;
    result.hybrid = None;
    result.lexical.latency = Latency::from_samples(&[1001.0])?;
    assert!(
        baseline
            .assert_record(&result, false)
            .unwrap_err()
            .to_string()
            .contains("p95")
    );
    result.lexical.latency = Latency::from_samples(&[1.0])?;
    result.fixture_sha256 = "0".repeat(64);
    assert!(baseline.assert_record(&result, false).is_err());
    std::fs::write(
        dir.path().join("queries.jsonl"),
        "{\"id\":\"q\",\"text\":\"changed\"}\n",
    )
    .unwrap();
    assert!(Baseline::load(dir.path()).is_err());
    Ok(())
}

#[tokio::test]
async fn baseline_calibration_failed_task_cleans_up() -> Result<(), FrameworkError> {
    use suprnova::search::baseline::Calibration;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path_regex},
    };
    let meili = MockServer::start().await;
    let typesense = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex("^/indexes$"))
        .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({"taskUid":1})))
        .mount(&meili)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("^/tasks/1$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"status":"failed","error":{"message":"sensitive server content"}}),
        ))
        .mount(&meili)
        .await;
    Mock::given(method("DELETE"))
        .and(path_regex("^/indexes/[^/]+$"))
        .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({"taskUid":2})))
        .expect(1)
        .mount(&meili)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("^/tasks/2$"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"status":"succeeded"})),
        )
        .mount(&meili)
        .await;
    let fixture = Fixture::load(fixtures())?;
    let floors = BaselineFloors::load(fixtures().join("baseline.json"))?;
    let error = Calibration::new(
        &meili.uri(),
        "test-meili",
        &typesense.uri(),
        "test-typesense",
    )?
    .run(&fixture, &floors, 0.05)
    .await
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("Meilisearch task 1"));
    assert!(!message.contains("sensitive server content"));
    assert!(!message.contains("test-meili"));
    assert!(typesense.received_requests().await.unwrap().is_empty());
    assert!(!floors.calibrated);
    meili.verify().await;
    Ok(())
}

#[test]
fn baseline_calibration_rejects_unsafe_destinations_and_empty_keys() {
    use suprnova::search::baseline::Calibration;
    for url in [
        "ftp://localhost/",
        "http://user:password@localhost/",
        "http://localhost/?key=secret",
        "http://localhost/#secret",
        "bad-url",
    ] {
        assert!(Calibration::new(url, "test", "http://localhost/", "test").is_err());
    }
    assert!(Calibration::new("http://localhost/", "", "http://localhost/", "test").is_err());
    assert!(
        Calibration::new(
            "http://localhost/",
            "test",
            "http://localhost/",
            "test\ninvalid"
        )
        .is_err()
    );
}

#[tokio::test]
async fn baseline_calibration_refuses_redirects_without_forwarding_keys()
-> Result<(), FrameworkError> {
    use suprnova::search::baseline::Calibration;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path_regex},
    };
    let meili = MockServer::start().await;
    let other = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex("^/indexes$"))
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("Location", format!("{}/indexes", other.uri())),
        )
        .expect(1)
        .mount(&meili)
        .await;
    Mock::given(method("DELETE"))
        .and(path_regex("^/indexes/[^/]+$"))
        .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({"taskUid":1})))
        .expect(1)
        .mount(&meili)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("^/tasks/1$"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"status":"succeeded"})),
        )
        .mount(&meili)
        .await;
    let fixture = Fixture::load(fixtures())?;
    let floors = BaselineFloors::load(fixtures().join("baseline.json"))?;
    let error = Calibration::new(
        &meili.uri(),
        "test-private-key",
        &other.uri(),
        "test-typesense",
    )?
    .run(&fixture, &floors, 0.05)
    .await
    .unwrap_err();
    assert!(error.to_string().contains("HTTP 307"));
    assert!(!error.to_string().contains("test-private-key"));
    assert!(other.received_requests().await.unwrap().is_empty());
    meili.verify().await;
    Ok(())
}
