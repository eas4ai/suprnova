//! Judged retrieval and warm latency use the same fixture and floors on every engine.
use std::{collections::BTreeMap, path::PathBuf};
use suprnova::{
    FrameworkError,
    search::{
        CollectionEngine,
        baseline::{
            Baseline, BaselineFloors, BudgetCheck, Fixture, Latency, ndcg_at_10, optimized_build,
        },
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
fn baseline_budget_policy_matches_build() {
    assert_eq!(optimized_build(), !cfg!(debug_assertions));
    assert_eq!(
        BudgetCheck::for_build(),
        if optimized_build() {
            BudgetCheck::Enforce
        } else {
            BudgetCheck::Report
        }
    );
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
fn baseline_rival_budgets_hand_computed() -> Result<(), FrameworkError> {
    use suprnova::search::baseline::{Measurement, Rival, RivalTrial};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("baseline.json");
    let mut floors = BaselineFloors::load(fixtures().join("baseline.json"))?;
    floors.calibrated = true;
    floors.calibrated_at = Some("2026-10-14T00:00:00Z".into());
    for (meili, typesense, expected) in [(4.0, 7.5, 15.0), (9.0, 2.0, 18.0), (3.0, 3.0, 6.0)] {
        let rival = |version: &str, p95_ms| Rival {
            version: Some(version.into()),
            settings: Some(serde_json::json!({"mode":"test"})),
            grid: vec![RivalTrial {
                settings: serde_json::json!({"mode":"test"}),
                ndcg_at_10: 0.8,
                p95_ms,
            }],
            lexical: Some(Measurement {
                ndcg_at_10: 0.8,
                queries: 300,
                latency: Latency {
                    p50_ms: 1.0,
                    p95_ms,
                    p99_ms: 10.0,
                },
            }),
        };
        floors.rivals.meilisearch = rival("1.test", meili);
        floors.rivals.typesense = rival("29.test", typesense);
        floors
            .p95_budgets_ms
            .values_mut()
            .for_each(|budget| *budget = expected);
        floors.write(&path)?;
        BaselineFloors::load(&path)?;
        let mut edited = floors.clone();
        *edited.p95_budgets_ms.get_mut("collection").unwrap() *= 1.01;
        std::fs::write(&path, serde_json::to_vec(&edited).unwrap()).unwrap();
        assert!(BaselineFloors::load(&path).is_err());
        *edited.p95_budgets_ms.get_mut("collection").unwrap() = expected * (1.0 + 1e-10);
        edited.write(&path)?;
        BaselineFloors::load(&path)?;
    }
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
    // The committed floors as they read before a calibration: rival scores
    // kept, the chosen settings and their grids not yet recorded.
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixtures().join("baseline.json")).unwrap()).unwrap();
    value["calibrated"] = serde_json::Value::Bool(false);
    for name in ["meilisearch", "typesense"] {
        let rival = value["rivals"][name].as_object_mut().unwrap();
        rival.remove("settings");
        rival.remove("grid");
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("baseline.json");
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let floors = BaselineFloors::load(&path)?;
    assert!(!floors.calibrated);
    assert!(floors.rivals.meilisearch.settings.is_none());
    assert!(floors.rivals.meilisearch.grid.is_empty());
    assert!(floors.rivals.typesense.settings.is_none());
    assert!(floors.rivals.typesense.grid.is_empty());
    let serialized = serde_json::to_value(&floors).unwrap();
    for name in ["meilisearch", "typesense"] {
        assert!(serialized["rivals"][name].get("settings").is_none());
    }
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
        BudgetCheck::for_build(),
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

async fn calibrate_mock_rivals(
    rejected_parameter: Option<(&str, &str)>,
    tied: bool,
) -> Result<BaselineFloors, FrameworkError> {
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
    for strategy in ["last", "frequency", "all"] {
        let hits = if strategy == "frequency" || (tied && strategy == "all") {
            serde_json::json!([{"id":"d"}])
        } else {
            serde_json::json!([])
        };
        Mock::given(method("POST"))
            .and(path_regex("^/indexes/[^/]+/search$"))
            .and(body_partial_json(serde_json::json!({
                "q":"query", "limit":10, "attributesToRetrieve":["id"],
                "matchingStrategy":strategy
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"hits":hits}))
                    .set_delay(std::time::Duration::from_millis(if strategy == "all" {
                        40
                    } else {
                        0
                    })),
            )
            .expect(2)
            .mount(&meili)
            .await;
    }
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
    for threshold in ["1", "10"] {
        for match_type in ["max_score", "sum_score"] {
            let rejected = rejected_parameter.is_some_and(|(parameter, value)| {
                (parameter == "drop_tokens_threshold" && value == threshold)
                    || (parameter == "text_match_type" && value == match_type)
            });
            let hits = if threshold == "10" && (match_type == "sum_score" || tied) {
                serde_json::json!([{"document":{"id":"d"}}])
            } else {
                serde_json::json!([])
            };
            Mock::given(method("GET"))
                .and(path_regex("^/collections/[^/]+/documents/search$"))
                .and(query_param("query_by", "title,text"))
                .and(query_param("q", "query"))
                .and(query_param("per_page", "10"))
                .and(query_param("include_fields", "id"))
                .and(query_param("drop_tokens_threshold", threshold))
                .and(query_param("text_match_type", match_type))
                .respond_with(if rejected {
                    // The response body must never be copied into the error.
                    ResponseTemplate::new(400).set_body_string("private server content")
                } else {
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"hits":hits}))
                        .set_delay(std::time::Duration::from_millis(
                            if match_type == "sum_score" { 40 } else { 0 },
                        ))
                })
                .expect(if rejected_parameter.is_none() {
                    2..=2
                } else {
                    0..=2
                })
                .mount(&typesense)
                .await;
        }
    }
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
    let before = std::fs::read(fixtures().join("baseline.json")).unwrap();
    let floors = BaselineFloors::load(fixtures().join("baseline.json"))?;
    let result = Calibration::new(
        &meili.uri(),
        "test-meili",
        &typesense.uri(),
        "test-typesense",
    )?
    .run(&fixture, &floors, 0.05)
    .await;
    meili.verify().await;
    typesense.verify().await;
    assert_eq!(
        std::fs::read(fixtures().join("baseline.json")).unwrap(),
        before
    );
    result
}

#[tokio::test]
async fn baseline_calibration_uses_tasks_import_and_versions() -> Result<(), FrameworkError> {
    let floors = BaselineFloors::load(fixtures().join("baseline.json"))?;
    let calibrated = calibrate_mock_rivals(None, false).await?;
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
    let meili_p95 = calibrated
        .rivals
        .meilisearch
        .lexical
        .as_ref()
        .unwrap()
        .latency
        .p95_ms;
    let typesense_p95 = calibrated
        .rivals
        .typesense
        .lexical
        .as_ref()
        .unwrap()
        .latency
        .p95_ms;
    assert_eq!(
        calibrated.p95_budgets_ms.keys().collect::<Vec<_>>(),
        floors.p95_budgets_ms.keys().collect::<Vec<_>>()
    );
    for budget in calibrated.p95_budgets_ms.values() {
        assert_eq!(*budget, 2.0 * meili_p95.max(typesense_p95));
    }
    assert!(
        calibrated
            .budget_comment
            .contains(&format!("{meili_p95:.6}"))
    );
    assert!(
        calibrated
            .budget_comment
            .contains(&format!("{typesense_p95:.6}"))
    );
    assert!(calibrated.budget_comment.contains("1.test"));
    assert!(calibrated.budget_comment.contains("29.test"));
    assert!(
        calibrated
            .budget_comment
            .contains("two times the slower rival's p95")
    );
    assert!(
        calibrated
            .budget_comment
            .contains(&calibrated.calibrated_at.as_ref().unwrap()[..10])
    );
    assert!(!calibrated.budget_comment.contains("test-meili"));
    assert!(!calibrated.budget_comment.contains("test-typesense"));
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("baseline.json");
    calibrated.write(&output)?;
    assert_eq!(
        BaselineFloors::load(&output)?.p95_budgets_ms,
        calibrated.p95_budgets_ms
    );
    let serialized = serde_json::to_value(&calibrated).unwrap();
    assert_eq!(
        serde_json::to_value(BaselineFloors::load(&output)?).unwrap(),
        serialized
    );
    for (name, count) in [("meilisearch", 3), ("typesense", 4)] {
        let rival = &serialized["rivals"][name];
        assert_eq!(rival["grid"].as_array().unwrap().len(), count);
        assert!(
            calibrated
                .budget_comment
                .contains(&rival["settings"].to_string())
        );
        let trials = rival["grid"].as_array().unwrap();
        let chosen = trials
            .iter()
            .find(|trial| trial["settings"] == rival["settings"])
            .unwrap();
        assert_eq!(chosen["ndcg_at_10"], rival["lexical"]["ndcg_at_10"]);
        assert_eq!(chosen["p95_ms"], rival["lexical"]["latency"]["p95_ms"]);
    }
    assert_eq!(
        serialized["rivals"]["meilisearch"]["settings"],
        serde_json::json!({"matchingStrategy":"frequency", "limit":10, "attributesToRetrieve":["id"]})
    );
    assert_eq!(
        serialized["rivals"]["typesense"]["settings"],
        serde_json::json!({"drop_tokens_threshold":10, "text_match_type":"sum_score",
            "query_by":"title,text", "per_page":10, "include_fields":"id"})
    );
    let meili_grid = serialized["rivals"]["meilisearch"]["grid"]
        .as_array()
        .unwrap();
    for (trial, strategy) in meili_grid.iter().zip(["last", "frequency", "all"]) {
        assert_eq!(
            trial["settings"],
            serde_json::json!({
                "matchingStrategy":strategy, "limit":10, "attributesToRetrieve":["id"]
            })
        );
        assert_eq!(
            trial["ndcg_at_10"],
            if strategy == "frequency" { 1.0 } else { 0.0 }
        );
    }
    let typesense_grid = serialized["rivals"]["typesense"]["grid"]
        .as_array()
        .unwrap();
    for (trial, (threshold, match_type)) in typesense_grid.iter().zip([
        (1, "max_score"),
        (1, "sum_score"),
        (10, "max_score"),
        (10, "sum_score"),
    ]) {
        assert_eq!(
            trial["settings"],
            serde_json::json!({
                "drop_tokens_threshold":threshold, "text_match_type":match_type,
                "query_by":"title,text", "per_page":10, "include_fields":"id"
            })
        );
        assert_eq!(
            trial["ndcg_at_10"],
            if threshold == 10 && match_type == "sum_score" {
                1.0
            } else {
                0.0
            }
        );
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&calibrated.rivals).unwrap()
    );
    Ok(())
}

#[tokio::test]
async fn baseline_calibration_ties_choose_lower_p95() -> Result<(), FrameworkError> {
    let calibrated = calibrate_mock_rivals(None, true).await?;
    let serialized = serde_json::to_value(calibrated).unwrap();
    for name in ["meilisearch", "typesense"] {
        let rival = &serialized["rivals"][name];
        assert_eq!(
            rival["grid"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|trial| trial["ndcg_at_10"].as_f64() == Some(1.0))
                .count(),
            2
        );
        let best = rival["grid"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|trial| trial["ndcg_at_10"].as_f64() == Some(1.0))
            .min_by(|a, b| {
                a["p95_ms"]
                    .as_f64()
                    .unwrap()
                    .total_cmp(&b["p95_ms"].as_f64().unwrap())
            })
            .unwrap();
        assert_eq!(rival["settings"], best["settings"]);
        assert_eq!(rival["lexical"]["latency"]["p95_ms"], best["p95_ms"]);
    }
    Ok(())
}

#[tokio::test]
async fn baseline_calibration_rejected_typesense_parameter_cleans_up_both_indexes() {
    for rejected in [
        ("drop_tokens_threshold", "10"),
        ("text_match_type", "sum_score"),
    ] {
        let error = calibrate_mock_rivals(Some(rejected), false)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("HTTP 400"), "{error}");
        assert!(error.contains(rejected.0), "{error}");
        assert!(!error.contains("private server content"));
        assert!(!error.contains("test-typesense"));
        assert!(!error.contains("test-meili"));
    }
}

#[test]
fn baseline_calibrated_load_requires_best_grid_trial() -> Result<(), FrameworkError> {
    let floors = BaselineFloors::load(fixtures().join("baseline.json"))?;
    let mut value = serde_json::to_value(floors).unwrap();
    value["calibrated"] = serde_json::json!(true);
    for name in ["meilisearch", "typesense"] {
        let rival = &mut value["rivals"][name];
        let score = rival["lexical"]["ndcg_at_10"].clone();
        rival["settings"] = serde_json::json!({"mode":"best"});
        rival["grid"] = serde_json::json!([
            {"settings":{"mode":"weaker"},"ndcg_at_10":0.1,"p95_ms":1.0},
            {"settings":{"mode":"best"},"ndcg_at_10":score,"p95_ms":rival["lexical"]["latency"]["p95_ms"]}
        ]);
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("baseline.json");
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    BaselineFloors::load(&path)?;
    for name in ["meilisearch", "typesense"] {
        for edit in [
            "weaker",
            "missing_settings",
            "missing_grid",
            "empty_grid",
            "invalid_trial",
            "invalid_p95",
        ] {
            let mut edited = value.clone();
            match edit {
                "weaker" => {
                    edited["rivals"][name]["lexical"]["ndcg_at_10"] = serde_json::json!(0.1)
                }
                "missing_settings" | "missing_grid" => {
                    let field = if edit == "missing_settings" {
                        "settings"
                    } else {
                        "grid"
                    };
                    edited["rivals"][name]
                        .as_object_mut()
                        .unwrap()
                        .remove(field);
                }
                "empty_grid" => edited["rivals"][name]["grid"] = serde_json::json!([]),
                "invalid_p95" => {
                    edited["rivals"][name]["grid"][0]["p95_ms"] = serde_json::json!(-1.0)
                }
                _ => edited["rivals"][name]["grid"][0]["ndcg_at_10"] = serde_json::json!(2.0),
            }
            std::fs::write(&path, serde_json::to_vec(&edited).unwrap()).unwrap();
            assert!(BaselineFloors::load(&path).is_err(), "{name}: {edit}");
            edited["calibrated"] = serde_json::json!(false);
            std::fs::write(&path, serde_json::to_vec(&edited).unwrap()).unwrap();
            BaselineFloors::load(&path)?;
        }
        let score = value["rivals"][name]["lexical"]["ndcg_at_10"]
            .as_f64()
            .unwrap();
        let mut rounded = value.clone();
        rounded["rivals"][name]["lexical"]["ndcg_at_10"] = serde_json::json!(score * (1.0 + 1e-10));
        std::fs::write(&path, serde_json::to_vec(&rounded).unwrap()).unwrap();
        BaselineFloors::load(&path)?;
    }
    let mut zero = value;
    for name in ["meilisearch", "typesense"] {
        zero["rivals"][name]["lexical"]["ndcg_at_10"] = serde_json::json!(0.0);
        for trial in zero["rivals"][name]["grid"].as_array_mut().unwrap() {
            trial["ndcg_at_10"] = serde_json::json!(0.0);
        }
    }
    std::fs::write(&path, serde_json::to_vec(&zero).unwrap()).unwrap();
    BaselineFloors::load(&path)?;
    zero["rivals"]["meilisearch"]["lexical"]["ndcg_at_10"] = serde_json::json!(1e-12);
    std::fs::write(&path, serde_json::to_vec(&zero).unwrap()).unwrap();
    assert!(BaselineFloors::load(&path).is_err());
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
        settings: Some(serde_json::json!({"mode":"test"})),
        grid: vec![suprnova::search::baseline::RivalTrial {
            settings: serde_json::json!({"mode":"test"}),
            ndcg_at_10: 0.8,
            p95_ms: 200.0,
        }],
        lexical: Some(measurement.clone()),
    };
    let mut weaker = measurement;
    weaker.ndcg_at_10 = 0.5;
    floors.rivals.typesense = suprnova::search::baseline::Rival {
        version: Some("test-typesense".into()),
        settings: Some(serde_json::json!({"mode":"test"})),
        grid: vec![suprnova::search::baseline::RivalTrial {
            settings: serde_json::json!({"mode":"test"}),
            ndcg_at_10: 0.5,
            p95_ms: 500.0,
        }],
        lexical: Some(weaker),
    };
    // Relevance and speed are independent: Typesense is weaker but slower.
    floors.rivals.meilisearch.lexical.as_mut().unwrap().latency = Latency {
        p50_ms: 100.0,
        p95_ms: 200.0,
        p99_ms: 300.0,
    };
    floors.rivals.typesense.lexical.as_mut().unwrap().latency = Latency {
        p50_ms: 250.0,
        p95_ms: 500.0,
        p99_ms: 600.0,
    };
    floors
        .p95_budgets_ms
        .values_mut()
        .for_each(|budget| *budget = 1000.0);
    let mut edited = floors.clone();
    edited.p95_budgets_ms.insert("collection".into(), 1200.0);
    std::fs::write(
        dir.path().join("baseline.json"),
        serde_json::to_vec(&edited).unwrap(),
    )
    .unwrap();
    assert!(Baseline::load(dir.path()).is_err());
    floors.write(dir.path().join("baseline.json"))?;
    let baseline = Baseline::load(dir.path())?;
    let mut result = baseline
        .run("collection", &CollectionEngine::new(), None)
        .await?;
    assert_eq!(result.lexical.ndcg_at_10, 1.0);
    baseline.assert_record(&result, false, BudgetCheck::Enforce)?;
    assert!(result.query_inference.is_none());
    let scope = result.latency_scope.clone();
    assert_eq!(
        scope,
        "warm engine query: first page of 10 hits at the builder's depth; parsing, expansion, retrieval and fusion; query inference timed separately; fixed vocabulary; no hydration"
    );
    result.latency_scope = "warm engine query: parsing, expansion, retrieval, query inference and fusion; fixed vocabulary; no hydration".into();
    assert!(
        baseline
            .assert_record(&result, false, BudgetCheck::Report)
            .is_err()
    );
    result.latency_scope = "different timing boundary".into();
    assert!(
        baseline
            .assert_record(&result, false, BudgetCheck::Enforce)
            .is_err()
    );
    result.latency_scope = scope;
    let mut old_scope = floors.clone();
    old_scope.latency_scope = "warm engine query: parsing, expansion, retrieval, query inference and fusion; fixed vocabulary; no hydration".into();
    std::fs::write(
        dir.path().join("baseline.json"),
        serde_json::to_vec(&old_scope).unwrap(),
    )
    .unwrap();
    assert!(BaselineFloors::load(dir.path().join("baseline.json")).is_err());
    floors.write(dir.path().join("baseline.json"))?;
    result.vocabulary_prepare_ms = -1.0;
    assert!(
        baseline
            .assert_record(&result, false, BudgetCheck::Enforce)
            .is_err()
    );
    result.vocabulary_prepare_ms = 0.0;
    let _db = baseline_database().await?;
    let sqlite = suprnova::search::DatabaseSearchEngine::new();
    for _ in 0..2 {
        let sqlite_record = baseline.run("sqlite", &sqlite, None).await?;
        assert_eq!(sqlite_record.lexical.ndcg_at_10, 1.0);
        baseline.assert_record(&sqlite_record, false, BudgetCheck::for_build())?;
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
            .assert_record(&result, false, BudgetCheck::Enforce)
            .unwrap_err()
            .to_string()
            .contains("nDCG@10")
    );
    assert!(
        baseline
            .assert_record(&result, false, BudgetCheck::Report)
            .unwrap_err()
            .to_string()
            .contains("nDCG@10")
    );
    result.lexical.ndcg_at_10 = 0.8;
    result.query_inference = Some(Latency::from_samples(&[1.0])?);
    assert!(
        baseline
            .assert_record(&result, false, BudgetCheck::Report)
            .unwrap_err()
            .to_string()
            .contains("query_inference")
    );
    result.query_inference = None;
    result.hybrid = Some(result.lexical.clone());
    result.model_profile = Some(suprnova::search::default_profile());
    result.model_provenance = Some(result.model_profile.as_ref().unwrap().provenance()?);
    assert!(
        baseline
            .assert_record(&result, false, BudgetCheck::Report)
            .unwrap_err()
            .to_string()
            .contains("query_inference")
    );
    result.query_inference = Some(Latency::from_samples(&[2000.0])?);
    assert!(
        baseline
            .assert_record(&result, false, BudgetCheck::Enforce)
            .unwrap_err()
            .to_string()
            .contains("hybrid")
    );
    assert!(
        baseline
            .assert_record(&result, false, BudgetCheck::Report)
            .unwrap_err()
            .to_string()
            .contains("nDCG@10")
    );
    result.hybrid.as_mut().unwrap().ndcg_at_10 = 0.81;
    baseline.assert_record(&result, false, BudgetCheck::Enforce)?;
    result.query_inference.as_mut().unwrap().p95_ms = f64::NAN;
    assert!(
        baseline
            .assert_record(&result, false, BudgetCheck::Report)
            .is_err()
    );
    result.query_inference = Some(Latency::from_samples(&[2000.0])?);
    result.hybrid.as_mut().unwrap().latency = Latency::from_samples(&[1001.0])?;
    assert!(
        baseline
            .assert_record(&result, false, BudgetCheck::Enforce)
            .unwrap_err()
            .to_string()
            .contains("hybrid p95")
    );
    baseline.assert_record(&result, false, BudgetCheck::Report)?;
    result.hybrid = None;
    result.query_inference = None;
    result.lexical.latency = Latency::from_samples(&[1001.0])?;
    assert!(
        baseline
            .assert_record(&result, false, BudgetCheck::Enforce)
            .unwrap_err()
            .to_string()
            .contains("p95")
    );
    baseline.assert_record(&result, false, BudgetCheck::Report)?;
    result.lexical.latency.p50_ms = f64::NAN;
    assert!(
        baseline
            .assert_record(&result, false, BudgetCheck::Report)
            .is_err()
    );
    // A delayed HTTP provider proves that inference stays outside both engine samples.
    // This exercises the hybrid runner without downloading a local model.
    use suprnova::search::embedder::http::{HttpEmbedder, HttpEmbedderConfig};
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    let provider = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"data":[{"index":0,"embedding":[1.0,0.0]}]}))
                .set_delay(std::time::Duration::from_millis(150)),
        )
        .expect(3) // One document batch, then warm and measured hybrid queries.
        .mount(&provider)
        .await;
    let embedder = HttpEmbedder::new(HttpEmbedderConfig {
        base_url: provider.uri(),
        model: "baseline-timing-test".into(),
        dimensions: Some(2),
        ..Default::default()
    })
    .await?;
    let timed = baseline
        .run("collection", &CollectionEngine::new(), Some(&embedder))
        .await?;
    let inference = timed.query_inference.as_ref().unwrap();
    assert!(inference.p50_ms >= 150.0);
    assert!(timed.lexical.latency.p95_ms < inference.p50_ms);
    assert!(timed.hybrid.as_ref().unwrap().latency.p95_ms < inference.p50_ms);
    assert_eq!(timed.lexical.queries, 1);
    assert_eq!(timed.hybrid.as_ref().unwrap().queries, 1);
    baseline.assert_record(&timed, false, BudgetCheck::Enforce)?;
    provider.verify().await;
    result.lexical.latency = Latency::from_samples(&[1.0])?;
    result.fixture_sha256 = "0".repeat(64);
    assert!(
        baseline
            .assert_record(&result, false, BudgetCheck::Enforce)
            .is_err()
    );
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
    let committed = fixtures().join("baseline.json");
    let before = std::fs::read(&committed).map_err(|e| FrameworkError::internal(e.to_string()))?;
    let floors = BaselineFloors::load(&committed)?;
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
    // A failed calibration leaves the committed floors exactly as they were.
    assert_eq!(
        std::fs::read(&committed).map_err(|e| FrameworkError::internal(e.to_string()))?,
        before
    );
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
