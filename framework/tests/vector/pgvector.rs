#![cfg(feature = "vector-pgvector")]

//! Database-free contracts and disposable Postgres integration tests.
//! Integration tests share a database advisory lock because one checks
//! a missing extension. No test drops an extension with CASCADE.

use serde_json::json;
use sqlx::Connection;
use sqlx::postgres::{PgConnection, PgPoolOptions};
use std::process::Command;
use suprnova::{PgVectorDistance, PgVectorDriver, VectorDriver, VectorItem};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

#[test]
fn postgres_table_sql_matches_distance_and_quotes_identifiers() {
    for (distance, opclass) in [
        (PgVectorDistance::Cosine, "vector_cosine_ops"),
        (PgVectorDistance::Euclidean, "vector_l2_ops"),
        (PgVectorDistance::InnerProduct, "vector_ip_ops"),
    ] {
        let sql = PgVectorDriver::ensure_table_sql("Documents", 3, distance).unwrap();
        assert_eq!(
            sql,
            format!(
                "CREATE EXTENSION IF NOT EXISTS vector;\n\
             CREATE TABLE IF NOT EXISTS \"Documents\" (\n  \
             id VARCHAR(254) PRIMARY KEY,\n  \
             embedding vector(3) NOT NULL,\n  \
             metadata JSONB NULL\n\
             );\n\
             CREATE INDEX ON \"Documents\" USING hnsw (embedding {opclass});"
            )
        );
    }
}

#[test]
fn postgres_table_sql_rejects_invalid_dimension_and_store() {
    assert!(PgVectorDriver::ensure_table_sql("docs", 0, PgVectorDistance::Cosine).is_err());
    for name in ["", "9docs", "a\"; DROP TABLE docs;--", "a.b", "a-b", "café"] {
        assert!(PgVectorDriver::validate_store_name(name).is_err(), "{name}");
    }
    assert!(PgVectorDriver::validate_store_name(&"a".repeat(64)).is_err());
    assert!(PgVectorDriver::validate_store_name(&"a".repeat(63)).is_ok());
    assert_eq!(
        PgVectorDriver::validate_store_name("_Docs_9").unwrap(),
        "_Docs_9"
    );
}

#[test]
fn postgres_scores_preserve_cosine_and_inner_product_signs() {
    for (distance, raw, expected) in [
        (PgVectorDistance::Cosine, 0.0, 1.0),
        (PgVectorDistance::Cosine, 1.0, 0.0),
        (PgVectorDistance::Cosine, 2.0, -1.0),
        (PgVectorDistance::Euclidean, 0.0, 1.0),
        (PgVectorDistance::Euclidean, 3.0, 0.25),
        (PgVectorDistance::InnerProduct, -7.0, 7.0),
        (PgVectorDistance::InnerProduct, 2.0, -2.0),
    ] {
        assert_eq!(PgVectorDriver::score_from_distance(distance, raw), expected);
    }
}

#[test]
fn postgres_embedding_text_rejects_empty_and_non_finite_values() {
    assert_eq!(
        PgVectorDriver::embedding_to_vec_text(&[0.1, 0.2, -3.0]).unwrap(),
        "[0.1,0.2,-3]"
    );
    for values in [
        vec![],
        vec![f32::NAN],
        vec![f32::INFINITY],
        vec![f32::NEG_INFINITY],
    ] {
        assert!(PgVectorDriver::embedding_to_vec_text(&values).is_err());
    }
}

#[tokio::test]
async fn postgres_lazy_short_circuits_and_validation() {
    let driver = PgVectorDriver::from_url("postgres://user:pass@127.0.0.1:1/unused").unwrap();
    assert!(
        driver
            .similar("docs", vec![1.0], 0)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(driver.similar("docs", vec![], 10).await.unwrap().is_empty());
    driver.upsert("docs", vec![]).await.unwrap();
    driver.delete("docs", vec![]).await.unwrap();
    let error = driver
        .similar("docs", vec![0.0, -0.0], 1)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("zero-vector"));
    assert!(error.to_string().contains("docs"));
    assert!(driver.similar("docs", vec![f32::NAN], 1).await.is_err());
    assert!(driver.count("bad-name").await.is_err());
    assert!(
        driver
            .upsert("bad-name", vec![VectorItem::new("a", vec![1.0], json!({}))])
            .await
            .is_err()
    );
    assert!(driver.delete("bad-name", vec!["a".into()]).await.is_err());
    assert!(driver.similar("bad-name", vec![1.0], 1).await.is_err());
    assert!(
        driver
            .upsert("docs", vec![VectorItem::new("a", vec![1.0], json!([]))])
            .await
            .is_err()
    );
    assert!(
        driver
            .upsert("docs", vec![VectorItem::new("a", vec![], json!({}))])
            .await
            .is_err()
    );
}

// Environment cases run in child processes, so unrelated tests never see
// global environment mutation. The child still calls the public constructor.
fn env_child(
    case: &str,
    pgvector_url: Option<&str>,
    database_url: Option<&str>,
    store: Option<&str>,
) {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", "pgvector::postgres_env_child", "--nocapture"])
        .env("PGVECTOR_ENV_CASE", case)
        .env_remove("PGVECTOR_URL")
        .env_remove("DATABASE_URL")
        .env_remove("PGVECTOR_ENV_STORE");
    if let Some(url) = pgvector_url {
        cmd.env("PGVECTOR_URL", url);
    }
    if let Some(url) = database_url {
        cmd.env("DATABASE_URL", url);
    }
    if let Some(store) = store {
        cmd.env("PGVECTOR_ENV_STORE", store);
    }
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "child {case}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn postgres_env_child() {
    let Ok(case) = std::env::var("PGVECTOR_ENV_CASE") else {
        return;
    };
    match case.as_str() {
        "explicit" | "fallback" => {
            let driver = PgVectorDriver::from_env().unwrap();
            assert_eq!(
                driver.pool().connect_options().get_database(),
                Some("chosen")
            );
        }
        "wrong" | "missing" | "invalid" => {
            let error = PgVectorDriver::from_env()
                .err()
                .expect("must reject this environment")
                .to_string();
            assert!(error.contains("PGVECTOR_URL"), "{error}");
            if case != "invalid" {
                assert!(error.contains("DATABASE_URL"), "{error}");
            }
            assert!(!error.contains("secret-password"), "{error}");
        }
        "live" => {
            let driver = PgVectorDriver::from_env().unwrap();
            assert_eq!(
                driver
                    .count(&std::env::var("PGVECTOR_ENV_STORE").unwrap())
                    .await
                    .unwrap(),
                0
            );
        }
        _ => panic!("unknown child case"),
    }
}

#[test]
fn postgres_url_scheme_logic_and_environment_fallback() {
    let chosen = "postgres://user:pass@127.0.0.1:1/chosen";
    env_child(
        "explicit",
        Some(chosen),
        Some("mysql://user:pass@localhost/other"),
        None,
    );
    for url in [
        chosen,
        "postgresql://user:pass@127.0.0.1:1/chosen",
        "POSTGRES://user:pass@127.0.0.1:1/chosen",
    ] {
        env_child("fallback", Some("  "), Some(url), None);
    }
    env_child(
        "wrong",
        None,
        Some("mysql://user:secret-password@localhost/other"),
        None,
    );
    env_child("missing", None, None, None);
    env_child(
        "invalid",
        Some("mysql://user:secret-password@localhost/other"),
        Some(chosen),
        None,
    );
    env_child(
        "invalid",
        Some("postgres://user:secret-password@localhost:bad/chosen"),
        Some(chosen),
        None,
    );
}

struct TestStore {
    driver: PgVectorDriver,
    table: String,
    // A dedicated connection holds the lock across runtime pool queries.
    lock: PgConnection,
}

impl TestStore {
    async fn create(
        distance: PgVectorDistance,
    ) -> Result<Option<Self>, Box<dyn std::error::Error + Send + Sync>> {
        let Ok(url) = std::env::var("PG_TEST_URL") else {
            eprintln!("skipping Postgres integration test: PG_TEST_URL is unset");
            return Ok(None);
        };
        let mut lock = PgConnection::connect(&url).await?;
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(7_604_409_i64)
            .execute(&mut lock)
            .await?;
        let pool = PgPoolOptions::new()
            .max_connections(3)
            .connect(&url)
            .await?;
        let driver = PgVectorDriver::from_pool(pool).with_distance(distance);
        let table = format!("pgvec_{}", uuid::Uuid::new_v4().simple());
        let sql = driver.ensure_table_sql_for(&table, 3)?;
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(driver.pool())
            .await?;
        Ok(Some(Self {
            driver,
            table,
            lock,
        }))
    }

    async fn finish(mut self, result: TestResult) -> TestResult {
        let sql = format!("DROP TABLE IF EXISTS \"{}\"", self.table);
        let drop_result = sqlx::query(sqlx::AssertSqlSafe(sql))
            .execute(self.driver.pool())
            .await;
        sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(7_604_409_i64)
            .execute(&mut self.lock)
            .await?;
        self.driver.pool().close().await;
        drop_result?;
        result
    }
}

async fn ranked(
    distance: PgVectorDistance,
    expected_ids: &[&str],
    expected_scores: &[f32],
) -> TestResult {
    let Some(test) = TestStore::create(distance).await? else {
        return Ok(());
    };
    let result = async {
        test.driver
            .upsert(
                &test.table,
                vec![
                    VectorItem::new("aligned", vec![1.0, 0.0, 0.0], json!({"title": "first"})),
                    VectorItem::new("orthogonal", vec![0.0, 1.0, 0.0], json!(null)),
                    VectorItem::new("opposite", vec![-1.0, 0.0, 0.0], json!({})),
                ],
            )
            .await?;
        let hits = test
            .driver
            .similar(&test.table, vec![1.0, 0.0, 0.0], 10)
            .await?;
        assert_eq!(hits.len(), 3);
        for ((hit, id), score) in hits.iter().zip(expected_ids).zip(expected_scores) {
            assert_eq!(hit.id, *id);
            assert!(
                (hit.score - score).abs() < 1e-5,
                "{}: {} != {score}",
                hit.id,
                hit.score
            );
        }
        assert_eq!(hits[0].metadata, json!({"title": "first"}));
        assert_eq!(hits[1].metadata, json!(null));
        assert_eq!(
            test.driver
                .similar(&test.table, vec![1.0, 0.0, 0.0], 1)
                .await?
                .len(),
            1
        );
        Ok(())
    }
    .await;
    test.finish(result).await
}

#[tokio::test]
#[ignore = "requires disposable Postgres with pgvector at PG_TEST_URL"]
async fn postgres_cosine_returns_best_first() -> TestResult {
    ranked(
        PgVectorDistance::Cosine,
        &["aligned", "orthogonal", "opposite"],
        &[1.0, 0.0, -1.0],
    )
    .await
}

#[tokio::test]
#[ignore = "requires disposable Postgres with pgvector at PG_TEST_URL"]
async fn postgres_euclidean_returns_best_first() -> TestResult {
    ranked(
        PgVectorDistance::Euclidean,
        &["aligned", "orthogonal", "opposite"],
        &[1.0, 1.0 / (1.0 + 2.0_f32.sqrt()), 1.0 / 3.0],
    )
    .await
}

#[tokio::test]
#[ignore = "requires disposable Postgres with pgvector at PG_TEST_URL"]
async fn postgres_inner_product_returns_best_first() -> TestResult {
    ranked(
        PgVectorDistance::InnerProduct,
        &["aligned", "orthogonal", "opposite"],
        &[1.0, 0.0, -1.0],
    )
    .await
}

#[tokio::test]
#[ignore = "requires disposable Postgres with pgvector at PG_TEST_URL"]
async fn postgres_upsert_replaces_by_id() -> TestResult {
    let Some(test) = TestStore::create(PgVectorDistance::Cosine).await? else {
        return Ok(());
    };
    let result = async {
        test.driver
            .upsert(
                &test.table,
                vec![VectorItem::new(
                    "doc",
                    vec![0.0, 1.0, 0.0],
                    json!({"old": true}),
                )],
            )
            .await?;
        test.driver
            .upsert(
                &test.table,
                vec![VectorItem::new(
                    "doc",
                    vec![1.0, 0.0, 0.0],
                    json!({"new": true}),
                )],
            )
            .await?;
        assert_eq!(test.driver.count(&test.table).await?, 1);
        let hits = test
            .driver
            .similar(&test.table, vec![1.0, 0.0, 0.0], 1)
            .await?;
        assert_eq!(hits[0].id, "doc");
        assert_eq!(hits[0].score, 1.0);
        assert_eq!(hits[0].metadata, json!({"new": true}));
        test.driver
            .upsert(
                &test.table,
                vec![
                    VectorItem::new("doc", vec![0.0, 1.0, 0.0], json!({"old": true})),
                    VectorItem::new("doc", vec![1.0, 0.0, 0.0], json!({"last": true})),
                ],
            )
            .await?;
        let hits = test
            .driver
            .similar(&test.table, vec![1.0, 0.0, 0.0], 1)
            .await?;
        assert_eq!(hits[0].metadata, json!({"last": true}));
        assert_eq!(test.driver.count(&test.table).await?, 1);
        Ok(())
    }
    .await;
    test.finish(result).await
}

#[tokio::test]
#[ignore = "requires disposable Postgres with pgvector at PG_TEST_URL"]
async fn postgres_batches_upsert_delete_and_count() -> TestResult {
    let Some(test) = TestStore::create(PgVectorDistance::Cosine).await? else {
        return Ok(());
    };
    let result = async {
        assert_eq!(test.driver.count(&test.table).await?, 0);
        test.driver
            .upsert(
                &test.table,
                (0..1001)
                    .map(|i| VectorItem::new(format!("doc-{i}"), vec![1.0, 0.0, 0.0], json!({})))
                    .collect(),
            )
            .await?;
        assert_eq!(test.driver.count(&test.table).await?, 1001);
        test.driver
            .delete(&test.table, vec!["doc-0".into(), "unknown".into()])
            .await?;
        assert_eq!(test.driver.count(&test.table).await?, 1000);
        test.driver
            .delete(&test.table, (0..1001).map(|i| format!("doc-{i}")).collect())
            .await?;
        assert_eq!(test.driver.count(&test.table).await?, 0);
        Ok(())
    }
    .await;
    test.finish(result).await
}

#[tokio::test]
#[ignore = "requires disposable Postgres with pgvector at PG_TEST_URL"]
async fn postgres_dimension_mismatch_rolls_back_previous_batches() -> TestResult {
    let Some(test) = TestStore::create(PgVectorDistance::Cosine).await? else {
        return Ok(());
    };
    let result = async {
        let mut items: Vec<_> = (0..500)
            .map(|i| VectorItem::new(format!("doc-{i}"), vec![1.0, 0.0, 0.0], json!({})))
            .collect();
        items.push(VectorItem::new("wrong", vec![1.0, 0.0], json!({})));
        let error = test
            .driver
            .upsert(&test.table, items)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(&test.table), "{error}");
        assert!(error.contains("expected 3 dimensions, not 2"), "{error}");
        assert_eq!(test.driver.count(&test.table).await?, 0);
        test.driver
            .upsert(
                &test.table,
                vec![VectorItem::new("valid", vec![1.0, 0.0, 0.0], json!({}))],
            )
            .await?;
        let error = test
            .driver
            .similar(&test.table, vec![1.0, 0.0], 1)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(&test.table), "{error}");
        assert!(error.contains("dimensions"), "{error}");
        Ok(())
    }
    .await;
    test.finish(result).await
}

#[tokio::test]
#[ignore = "requires disposable Postgres with pgvector at PG_TEST_URL"]
async fn postgres_zero_query_is_rejected_only_for_cosine() -> TestResult {
    let Some(test) = TestStore::create(PgVectorDistance::Cosine).await? else {
        return Ok(());
    };
    let result = async {
        let error = test
            .driver
            .similar(&test.table, vec![0.0; 3], 1)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("zero-vector"));
        test.driver
            .upsert(
                &test.table,
                vec![VectorItem::new("doc", vec![1.0, 0.0, 0.0], json!({}))],
            )
            .await?;
        for distance in [PgVectorDistance::Euclidean, PgVectorDistance::InnerProduct] {
            let driver =
                PgVectorDriver::from_pool(test.driver.pool().clone()).with_distance(distance);
            let hits = driver.similar(&test.table, vec![0.0; 3], 1).await?;
            assert_eq!(hits.len(), 1);
            assert_eq!(
                hits[0].score,
                if distance == PgVectorDistance::Euclidean {
                    0.5
                } else {
                    0.0
                }
            );
        }
        Ok(())
    }
    .await;
    test.finish(result).await
}

#[tokio::test]
#[ignore = "requires disposable Postgres with pgvector at PG_TEST_URL"]
async fn postgres_from_env_precedence() -> TestResult {
    let Some(test) = TestStore::create(PgVectorDistance::Cosine).await? else {
        return Ok(());
    };
    let url = std::env::var("PG_TEST_URL")?;
    env_child(
        "live",
        Some(&url),
        Some("postgres://user:pass@127.0.0.1:1/unreachable"),
        Some(&test.table),
    );
    env_child("live", None, Some(&url), Some(&test.table));
    test.finish(Ok(())).await
}

#[tokio::test]
#[ignore = "requires disposable Postgres with pgvector at PG_TEST_URL"]
async fn postgres_missing_extension_error_text() -> TestResult {
    let Some(test) = TestStore::create(PgVectorDistance::Cosine).await? else {
        return Ok(());
    };
    let sql = format!("DROP TABLE \"{}\"", test.table);
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .execute(test.driver.pool())
        .await?;
    if let Err(error) = sqlx::query("DROP EXTENSION vector")
        .execute(test.driver.pool())
        .await
    {
        // Insufficient privilege or dependent tables: never use CASCADE.
        if matches!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("42501" | "2BP01")
        ) {
            eprintln!(
                "skipping postgres_missing_extension_error_text: role cannot drop vector extension or other tables depend on it ({error})"
            );
            return test.finish(Ok(())).await;
        }
        return test.finish(Err(error.into())).await;
    }
    let outcome = test.driver.count(&test.table).await;
    // Restore before assertions, so a failed assertion cannot leave vector absent.
    sqlx::query("CREATE EXTENSION vector")
        .execute(test.driver.pool())
        .await?;
    let error = outcome.unwrap_err().to_string();
    assert!(error.contains("CREATE EXTENSION vector"), "{error}");
    assert!(error.contains("pgvector package"), "{error}");
    test.finish(Ok(())).await
}
