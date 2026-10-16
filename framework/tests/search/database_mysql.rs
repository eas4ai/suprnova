//! MySQL and MariaDB share behavioral checks while reporting server limits.

use super::database_support::{
    create, descriptor, document, engine, live_database, query, vectors,
};
use serial_test::serial;
use std::sync::Arc;
use suprnova::{
    DB, FrameworkError,
    search::{ExpandedQuery, SearchEngine, testing::conformance},
};

#[tokio::test]
#[serial(search_database_mysql)]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_database_obeys_the_shared_driver_contract() -> Result<(), FrameworkError> {
    let _guard = live_database("MYSQL_TEST_URL").await?;
    vectors("database_mysql_conformance");
    conformance::run(
        Arc::new(engine()),
        conformance::Options {
            index_name: "database_mysql_conformance",
            ..Default::default()
        },
    )
    .await
}

#[tokio::test]
#[serial(search_database_mysql)]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_weighted_match_boolean_expansion_vocabulary_and_server_limits()
-> Result<(), FrameworkError> {
    let _guard = live_database("MYSQL_TEST_URL").await?;
    let index = descriptor("database_mysql_native");
    let generation = create(&index).await?;
    let driver = engine();
    driver
        .index(
            &index,
            &generation,
            &[
                document(&index, "a", "orchard", "green garden"),
                document(&index, "b", "garden", "red orchard"),
                document(&index, "c", "garden", "green orchard basket"),
            ],
        )
        .await?;
    let result = driver.query(&index, &generation, &query("orchard")).await?;
    assert_eq!(result.candidates[0].document.key, "a");
    assert!(result.candidates[0].score > result.candidates[1].score);
    let mut q = query("orch");
    q.expanded.terms[0].prefix = true;
    q.expanded.negatives = vec!["basket".into()];
    assert_eq!(driver.count(&index, &generation, &q).await?.value(), 2);
    q.expanded = ExpandedQuery {
        phrases: vec![vec!["red".into(), "orchard".into()]],
        ..Default::default()
    };
    assert_eq!(
        driver.query(&index, &generation, &q).await?.candidates[0]
            .document
            .key,
        "b"
    );
    let vocabulary = driver.vocabulary(&index, &generation).await?;
    assert_eq!(vocabulary.words.get("orchard"), Some(&3));
    driver
        .remove(&index, &generation, &[("c".into(), 2)])
        .await?;
    assert_eq!(
        driver
            .vocabulary(&index, &generation)
            .await?
            .words
            .get("orchard"),
        Some(&2)
    );
    let row = DB::select_one("SELECT document_frequency FROM search_vocabulary WHERE index_name = ? AND generation = ? AND word = ?", vec![index.name.into(), generation.as_str().into(), "orchard".into()]).await?.expect("framework vocabulary row");
    assert_eq!(row.get_int("document_frequency")?, 2);
    let limits = driver.server_limits().await?;
    for name in [
        "innodb_ft_min_token_size",
        "ft_min_word_len",
        "innodb_ft_enable_stopword",
    ] {
        assert!(limits.contains_key(name), "missing {name}");
    }
    assert!(limits.keys().any(|name| name.contains("stopword_table")));
    driver.drop_generation(&index, &generation).await
}

#[tokio::test]
#[serial(search_database_mysql)]
#[ignore = "requires disposable MariaDB/MySQL at MYSQL_TEST_URL"]
async fn mysql_native_text_folds_case_and_accents_while_filters_and_facets_remain_exact()
-> Result<(), FrameworkError> {
    use suprnova::search::{AttributeValue, FacetOrder, Filter};

    let _guard = live_database("MYSQL_TEST_URL").await?;
    let index = descriptor("database_mysql_collation");
    let generation = create(&index).await?;
    let driver = engine();
    driver
        .index(
            &index,
            &generation,
            &[
                document(&index, "uppercase", "CAFÉ orchard", "garden"),
                document(&index, "accented", "Café", "garden"),
                document(&index, "lowercase", "cafe", "garden"),
            ],
        )
        .await?;
    let q = query("cafe");
    let result = driver.query(&index, &generation, &q).await?;
    assert_eq!(result.candidates.len(), 3);
    assert!(
        result
            .candidates
            .iter()
            .any(|candidate| candidate.document.key == "uppercase")
    );
    let facets = driver
        .facets(
            &index,
            &generation,
            &q,
            &["title".into()],
            10,
            FacetOrder::Count,
        )
        .await?;
    let labels = &facets["title"];
    assert_eq!(labels.len(), 3);
    for title in ["CAFÉ orchard", "Café", "cafe"] {
        assert!(
            labels
                .iter()
                .any(|value| value.value == AttributeValue::Text(title.into()) && value.count == 1)
        );
    }
    let mut filtered = q;
    filtered.filter = Some(Filter::Eq(
        "title".into(),
        AttributeValue::Text("cafe".into()),
    ));
    let exact = driver.query(&index, &generation, &filtered).await?;
    assert_eq!(exact.candidates.len(), 1);
    assert_eq!(exact.candidates[0].document.key, "lowercase");
    driver.drop_generation(&index, &generation).await
}
