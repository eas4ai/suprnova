//! The scaffold database must exercise all mandatory search behavior by default.

use super::database_support::{Migrator, create, descriptor, document, engine, query, vectors};
use std::sync::Arc;
use suprnova::{
    DB, FrameworkError,
    search::{
        AttributeValue, Filter, GenerationId, LanguageConfig, SearchEngine, Total,
        testing::conformance,
    },
    testing::TestDatabase,
};

#[tokio::test]
async fn sqlite_database_obeys_the_shared_driver_contract_twice() -> Result<(), FrameworkError> {
    let _db = TestDatabase::fresh::<Migrator>().await?;
    vectors("database_sqlite_conformance");
    let driver: Arc<dyn SearchEngine> = Arc::new(engine());
    let options = conformance::Options {
        index_name: "database_sqlite_conformance",
        ..Default::default()
    };
    conformance::run(driver.clone(), options.clone()).await?;
    conformance::run(driver, options).await
}

#[tokio::test]
async fn sqlite_external_content_fts_and_triggers_follow_engine_writes()
-> Result<(), FrameworkError> {
    let _db = TestDatabase::fresh::<Migrator>().await?;
    let index = descriptor("database_sqlite_sync");
    let generation = create(&index).await?;
    let driver = engine();
    let ddl = DB::select_one(
        "SELECT sql FROM sqlite_master WHERE name = ?",
        vec!["search_database_sqlite_sync_native_fts".into()],
    )
    .await?
    .expect("FTS table")
    .get_string("sql")?;
    assert!(ddl.contains("fts5("));
    assert!(ddl.contains("content='search_database_sqlite_sync_native'"));
    assert!(ddl.contains("content_rowid='rowid'"));
    let triggers = DB::select(
        "SELECT name FROM sqlite_master WHERE type = ? AND tbl_name = ?",
        vec![
            "trigger".into(),
            "search_database_sqlite_sync_native".into(),
        ],
    )
    .await?;
    assert_eq!(triggers.len(), 3);
    let mut doc = document(&index, "a", "orchard", "garden");
    driver.index(&index, &generation, &[doc.clone()]).await?;
    assert_eq!(
        driver.count(&index, &generation, &query("orchard")).await?,
        Total::Exact(1)
    );
    doc.document.source_version = 2;
    doc.document
        .attributes
        .insert("title".into(), AttributeValue::Text("basket".into()));
    doc.content_hash = doc
        .document
        .content_hash(&index.index_settings().index_time.digest());
    driver.index(&index, &generation, &[doc]).await?;
    assert_eq!(
        driver.count(&index, &generation, &query("orchard")).await?,
        Total::Exact(0)
    );
    assert_eq!(
        driver.count(&index, &generation, &query("basket")).await?,
        Total::Exact(1)
    );
    driver
        .remove(&index, &generation, &[("a".into(), 3)])
        .await?;
    assert_eq!(
        driver.count(&index, &generation, &query("basket")).await?,
        Total::Exact(0)
    );
    Ok(())
}

#[tokio::test]
async fn sqlite_bm25_uses_declared_column_weights() -> Result<(), FrameworkError> {
    let _db = TestDatabase::fresh::<Migrator>().await?;
    let index = descriptor("database_sqlite_weights");
    let first = create(&index).await?;
    let second = GenerationId::new("reweighted")?;
    let driver = engine();
    let documents = [
        document(&index, "a", "orchard", "garden"),
        document(&index, "b", "garden", "orchard"),
    ];
    driver.index(&index, &first, &documents).await?;
    let initial = driver.query(&index, &first, &query("orchard")).await?;
    assert_eq!(initial.candidates[0].document.key, "a");
    assert!(initial.candidates[0].score > initial.candidates[1].score);
    let mut settings = index.index_settings();
    settings.index_time.searchable_attributes[0].weight = 1.0;
    settings.index_time.searchable_attributes[1].weight = 8.0;
    driver.create_generation(&index, &settings, &second).await?;
    let reweighted_documents = documents.map(|mut document| {
        document.content_hash = document
            .document
            .content_hash(&settings.index_time.digest());
        document
    });
    driver.index(&index, &second, &reweighted_documents).await?;
    let reweighted = driver.query(&index, &second, &query("orchard")).await?;
    assert_eq!(reweighted.candidates[0].document.key, "b");
    Ok(())
}

#[tokio::test]
async fn sqlite_highlight_escapes_markup_and_snippet_uses_native_context()
-> Result<(), FrameworkError> {
    let _db = TestDatabase::fresh::<Migrator>().await?;
    let index = descriptor("database_sqlite_highlights");
    let generation = create(&index).await?;
    let driver = engine();
    driver
        .index(
            &index,
            &generation,
            &[document(
                &index,
                "a",
                "<script>orchard</script>",
                "one two three orchard four five six seven eight",
            )],
        )
        .await?;
    let mut q = query("orchard");
    q.highlights = true;
    let result = driver.query(&index, &generation, &q).await?;
    let highlighted = &result.candidates[0]
        .highlights
        .as_ref()
        .expect("native highlight")["title"];
    assert!(highlighted.contains("&lt;script&gt;<em>orchard</em>&lt;/script&gt;"));
    assert!(!highlighted.contains("<script>"));
    let row = DB::select_one("SELECT snippet(search_database_sqlite_highlights_native_fts, 1, ?, ?, ?, ?) AS snippet FROM search_database_sqlite_highlights_native_fts WHERE search_database_sqlite_highlights_native_fts MATCH ?", vec!["<em>".into(), "</em>".into(), "...".into(), 4i64.into(), "orchard".into()]).await?.expect("native snippet");
    let snippet = row.get_string("snippet")?;
    assert!(snippet.contains("<em>orchard</em>"));
    assert!(snippet.contains("..."));
    q.crop.insert("body".into(), 16);
    assert!(driver.capabilities().native_crop);
    let cropped_result = driver.query(&index, &generation, &q).await?;
    let cropped = &cropped_result.candidates[0]
        .highlights
        .as_ref()
        .expect("native crop")["body"];
    assert!(cropped.contains("<em>orchard</em>"));
    assert!(cropped.contains("..."));
    assert!(!cropped.contains("one two three orchard four five six seven eight"));
    let snippets = driver.snippets(&index, &generation, &q, "body", 4).await?;
    assert!(snippets["a"].contains("<em>orchard</em>"));
    assert!(snippets["a"].contains("..."));
    let snippets = driver.snippets(&index, &generation, &q, "title", 4).await?;
    assert!(snippets["a"].contains("&lt;script&gt;<em>orchard</em>&lt;/script&gt;"));
    assert!(!snippets["a"].contains("<script>"));
    assert!(
        driver
            .snippets(&index, &generation, &q, "body", 0)
            .await
            .is_err()
    );
    assert!(
        driver
            .snippets(&index, &generation, &q, "category", 4)
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn sqlite_fts5vocab_counts_documents_and_tracks_updates() -> Result<(), FrameworkError> {
    let _db = TestDatabase::fresh::<Migrator>().await?;
    let index = descriptor("database_sqlite_vocab");
    let generation = create(&index).await?;
    let driver = engine();
    driver
        .index(
            &index,
            &generation,
            &[
                document(&index, "a", "orchard orchard", "orchard"),
                document(&index, "b", "orchard", "garden"),
            ],
        )
        .await?;
    assert_eq!(
        driver
            .vocabulary(&index, &generation)
            .await?
            .words
            .get("orchard"),
        Some(&2)
    );
    let row = DB::select_one(
        "SELECT doc, cnt FROM search_database_sqlite_vocab_native_vocab WHERE term = ?",
        vec!["orchard".into()],
    )
    .await?
    .expect("fts5vocab row");
    assert_eq!(row.get_int("doc")?, 2);
    assert_eq!(row.get_int("cnt")?, 4);
    driver
        .remove(&index, &generation, &[("a".into(), 2)])
        .await?;
    assert_eq!(
        driver
            .vocabulary(&index, &generation)
            .await?
            .words
            .get("orchard"),
        Some(&1)
    );
    driver
        .remove(&index, &generation, &[("b".into(), 2)])
        .await?;
    assert!(
        !driver
            .vocabulary(&index, &generation)
            .await?
            .words
            .contains_key("orchard")
    );
    Ok(())
}

#[tokio::test]
async fn sqlite_porter_tokenizer_stems_while_unicode61_preserves_words()
-> Result<(), FrameworkError> {
    let _db = TestDatabase::fresh::<Migrator>().await?;
    let index = descriptor("database_sqlite_porter");
    let plain = create(&index).await?;
    let porter = GenerationId::new("porter")?;
    let mut settings = index.index_settings();
    settings.index_time.language = LanguageConfig::Sqlite {
        tokenizer: "porter unicode61".into(),
    };
    let driver = engine();
    driver.create_generation(&index, &settings, &porter).await?;
    let documents = [document(&index, "a", "running", "gardens")];
    driver.index(&index, &plain, &documents).await?;
    let porter_documents = documents.map(|mut document| {
        document.content_hash = document
            .document
            .content_hash(&settings.index_time.digest());
        document
    });
    driver.index(&index, &porter, &porter_documents).await?;
    assert_eq!(
        driver.count(&index, &plain, &query("run")).await?.value(),
        0
    );
    assert_eq!(
        driver.count(&index, &porter, &query("run")).await?.value(),
        1
    );
    assert_eq!(driver.read_settings(&index, &porter).await?, settings);
    Ok(())
}

#[tokio::test]
async fn sqlite_filter_values_stay_bound_and_identifiers_are_rejected() -> Result<(), FrameworkError>
{
    let _db = TestDatabase::fresh::<Migrator>().await?;
    let index = descriptor("database_sqlite_bound");
    let generation = create(&index).await?;
    let driver = engine();
    let hostile = "fruit' OR 1 = 1 --";
    let mut doc = document(&index, "a' OR 1 = 1 --", "orchard", "garden");
    doc.document
        .attributes
        .insert("category".into(), AttributeValue::Text(hostile.into()));
    doc.content_hash = doc
        .document
        .content_hash(&index.index_settings().index_time.digest());
    driver
        .index(
            &index,
            &generation,
            &[doc, document(&index, "b", "orchard", "garden")],
        )
        .await?;
    let mut q = query("orchard");
    q.filter = Some(Filter::Eq(
        "category".into(),
        AttributeValue::Text(hostile.into()),
    ));
    assert_eq!(
        driver
            .query(&index, &generation, &q)
            .await?
            .candidates
            .len(),
        1
    );
    q.filter = Some(Filter::Eq(
        "category".into(),
        AttributeValue::Text("' OR 1 = 1 --".into()),
    ));
    assert_eq!(driver.count(&index, &generation, &q).await?.value(), 0);
    let bad = GenerationId("native; DROP TABLE search_generations".into());
    assert!(
        driver
            .create_generation(&index, &index.index_settings(), &bad)
            .await
            .is_err()
    );
    let bad_index = descriptor("bad;index");
    assert!(
        driver
            .create_generation(&bad_index, &bad_index.index_settings(), &generation)
            .await
            .is_err()
    );
    let mut bad_attribute = descriptor("database_bad_attribute");
    bad_attribute.attributes[0].name = "title; DROP TABLE search_generations".into();
    assert!(
        driver
            .create_generation(&bad_attribute, &bad_attribute.index_settings(), &generation)
            .await
            .is_err()
    );
    let mut unsafe_language = index.index_settings();
    unsafe_language.index_time.language = LanguageConfig::Sqlite {
        tokenizer: "unicode61'; DROP TABLE search_generations; --".into(),
    };
    assert!(
        driver
            .create_generation(
                &index,
                &unsafe_language,
                &GenerationId::new("unsafe_language")?
            )
            .await
            .is_err()
    );
    assert_eq!(
        driver.read_settings(&index, &generation).await?,
        index.index_settings()
    );
    Ok(())
}

#[tokio::test]
async fn sqlite_expansion_preserves_phrase_alternatives_and_attribute_typo_exclusions()
-> Result<(), FrameworkError> {
    let _db = TestDatabase::fresh::<Migrator>().await?;
    let index = descriptor("database_sqlite_expansion");
    let generation = create(&index).await?;
    let driver = engine();
    driver
        .index(
            &index,
            &generation,
            &[
                document(&index, "a", "orchard", "garden"),
                document(&index, "b", "garden", "orchard"),
                document(&index, "c", "garden", "red apple"),
                document(&index, "d", "garden", "apple red"),
                document(&index, "e", "garden", "orchardist"),
            ],
        )
        .await?;
    let mut q = query("orchrad");
    q.expanded.terms[0].alternatives = vec!["orchrad".into(), "orchard".into()];
    q.expanded.terms[0].typo_alternatives = vec!["orchard".into()];
    q.expanded.terms[0].prefix = true;
    q.expanded.typo_disabled_attributes.insert("title".into());
    let result = driver.query(&index, &generation, &q).await?;
    assert_eq!(
        result
            .candidates
            .iter()
            .map(|hit| hit.document.key.as_str())
            .collect::<Vec<_>>(),
        ["b"]
    );
    let mut phrase = query("fruit");
    phrase.expanded.terms[0].alternatives = vec!["fruit".into(), "red apple".into()];
    let result = driver.query(&index, &generation, &phrase).await?;
    assert_eq!(
        result
            .candidates
            .iter()
            .map(|hit| hit.document.key.as_str())
            .collect::<Vec<_>>(),
        ["c"]
    );
    Ok(())
}

#[tokio::test]
async fn sqlite_settings_and_generation_switch_are_shared_by_engine_instances()
-> Result<(), FrameworkError> {
    let _db = TestDatabase::fresh::<Migrator>().await?;
    let index = descriptor("database_sqlite_generations");
    let first = create(&index).await?;
    let second = GenerationId::new("second")?;
    let driver = engine();
    driver
        .create_generation(&index, &index.index_settings(), &second)
        .await?;
    driver
        .switch_generation(&index, None, first.clone())
        .await?;
    assert_eq!(
        engine().current_generation(index.name).await?,
        Some(first.clone())
    );
    let mut settings = index.index_settings();
    settings.query.hits_cap = 17;
    driver.write_settings(&index, &first, &settings).await?;
    assert_eq!(engine().read_settings(&index, &first).await?, settings);
    let mut changed_index_settings = settings.clone();
    changed_index_settings.index_time.searchable_attributes[0].weight = 3.0;
    assert!(
        driver
            .write_settings(&index, &first, &changed_index_settings)
            .await
            .is_err()
    );
    assert_eq!(engine().read_settings(&index, &first).await?, settings);
    driver
        .switch_generation(&index, Some(first.clone()), second.clone())
        .await?;
    assert!(
        engine()
            .switch_generation(&index, Some(first.clone()), first.clone())
            .await
            .is_err()
    );
    assert_eq!(engine().current_generation(index.name).await?, Some(second));
    driver.drop_generation(&index, &first).await?;
    assert!(engine().read_settings(&index, &first).await.is_err());
    Ok(())
}

#[tokio::test]
async fn sqlite_tombstones_survive_new_instances_and_can_be_pruned() -> Result<(), FrameworkError> {
    let _db = TestDatabase::fresh::<Migrator>().await?;
    let index = descriptor("database_sqlite_tombstones");
    let generation = create(&index).await?;
    let doc = document(&index, "a", "orchard", "garden");
    engine()
        .index(&index, &generation, std::slice::from_ref(&doc))
        .await?;
    engine()
        .remove(&index, &generation, &[("a".into(), 2)])
        .await?;
    assert_eq!(
        engine()
            .index(&index, &generation, std::slice::from_ref(&doc))
            .await?
            .skipped_tombstoned,
        1
    );
    assert_eq!(engine().prune_tombstones(0).await?, 1);
    assert_eq!(
        engine().index(&index, &generation, &[doc]).await?.written,
        1
    );
    Ok(())
}

#[tokio::test]
async fn sqlite_facet_search_folds_case_and_diacritics_within_lexical_filters()
-> Result<(), FrameworkError> {
    let _db = TestDatabase::fresh::<Migrator>().await?;
    let index = descriptor("database_sqlite_facet_prefix");
    let generation = create(&index).await?;
    let driver = engine();
    let mut allowed = document(&index, "allowed", "orchard", "garden");
    allowed
        .document
        .attributes
        .insert("category".into(), AttributeValue::Text("Éclair".into()));
    allowed.content_hash = allowed
        .document
        .content_hash(&index.index_settings().index_time.digest());
    let mut outside_lexical = document(&index, "outside_lexical", "basket", "garden");
    outside_lexical
        .document
        .attributes
        .insert("category".into(), AttributeValue::Text("Éclair".into()));
    outside_lexical.content_hash = outside_lexical
        .document
        .content_hash(&index.index_settings().index_time.digest());
    let mut outside_filter = document(&index, "outside_filter", "orchard reserve", "garden");
    outside_filter
        .document
        .attributes
        .insert("category".into(), AttributeValue::Text("Éclair".into()));
    outside_filter.content_hash = outside_filter
        .document
        .content_hash(&index.index_settings().index_time.digest());
    driver
        .index(
            &index,
            &generation,
            &[allowed, outside_lexical, outside_filter],
        )
        .await?;
    let mut q = query("orchard");
    q.filter = Some(Filter::Ne(
        "title".into(),
        AttributeValue::Text("orchard reserve".into()),
    ));
    let facets = driver
        .facet_search(&index, &generation, &q, "category", "eC", 10)
        .await?;
    assert_eq!(facets["category"].len(), 1);
    assert_eq!(
        facets["category"][0].value,
        AttributeValue::Text("Éclair".into())
    );
    assert_eq!(facets["category"][0].count, 1);
    Ok(())
}

#[tokio::test]
async fn sqlite_semantic_vectors_and_removals_are_isolated_per_generation()
-> Result<(), FrameworkError> {
    let _db = TestDatabase::fresh::<Migrator>().await?;
    let index = descriptor("database_sqlite_vectors");
    vectors(index.name);
    let first = GenerationId::new("first")?;
    let second = GenerationId::new("second")?;
    let mut settings = index.index_settings();
    settings.index_time.dimensions = Some(3);
    let driver = engine();
    driver.create_generation(&index, &settings, &first).await?;
    driver.create_generation(&index, &settings, &second).await?;
    let mut first_doc = document(&index, "shared", "orchard", "garden");
    first_doc.dense = Some(vec![1.0, 0.0, 0.0]);
    let mut second_doc = document(&index, "shared", "basket", "garden");
    second_doc.dense = Some(vec![0.0, 1.0, 0.0]);
    first_doc.content_hash = first_doc
        .document
        .content_hash(&settings.index_time.digest());
    second_doc.content_hash = second_doc
        .document
        .content_hash(&settings.index_time.digest());
    driver.index(&index, &first, &[first_doc]).await?;
    driver.index(&index, &second, &[second_doc]).await?;
    let store = suprnova::vector::Vector::store(index.name)?;
    assert_eq!(store.count().await?, 2);
    let first_result = driver
        .semantic(&index, &first, &[1.0, 0.0, 0.0], None, 10)
        .await?;
    assert_eq!(first_result.candidates.len(), 1);
    assert!(first_result.candidates[0].score > 0.99);
    let second_result = driver
        .semantic(&index, &second, &[1.0, 0.0, 0.0], None, 10)
        .await?;
    assert_eq!(second_result.candidates.len(), 1);
    assert_eq!(second_result.candidates[0].score, 0.0);
    let private = Filter::Eq("category".into(), AttributeValue::Text("private".into()));
    assert!(
        driver
            .semantic(&index, &first, &[1.0, 0.0, 0.0], Some(&private), 10)
            .await?
            .candidates
            .is_empty()
    );
    driver
        .remove(&index, &first, &[("shared".into(), 2)])
        .await?;
    assert_eq!(store.count().await?, 1);
    assert_eq!(
        driver
            .semantic(&index, &second, &[0.0, 1.0, 0.0], None, 10)
            .await?
            .candidates
            .len(),
        1
    );
    driver
        .remove(&index, &second, &[("shared".into(), 2)])
        .await?;
    assert_eq!(store.count().await?, 0);
    Ok(())
}

#[tokio::test]
async fn sqlite_failed_batch_preserves_the_committed_dense_vector() -> Result<(), FrameworkError> {
    let db = TestDatabase::fresh::<Migrator>().await?;
    let index = descriptor("database_sqlite_vector_rollback");
    vectors(index.name);
    let generation = create(&index).await?;
    let driver = engine();
    let mut original = document(&index, "a", "orchard", "garden");
    original.dense = Some(vec![1.0, 0.0, 0.0]);
    driver
        .index(&index, &generation, &[original.clone()])
        .await?;
    db.execute_unprepared(
        "CREATE TRIGGER reject_failed_search_batch BEFORE INSERT ON search_database_sqlite_vector_rollback_native \
         BEGIN SELECT RAISE(ABORT, 'reject failed search batch'); END",
    ).await?;
    let mut update = document(&index, "a", "basket", "garden");
    update.document.source_version = 2;
    update.content_hash = update
        .document
        .content_hash(&index.index_settings().index_time.digest());
    update.dense = Some(vec![0.0, 1.0, 0.0]);
    let rejected = document(&index, "fail", "rejected", "garden");
    let error = driver
        .index(&index, &generation, &[update, rejected])
        .await
        .expect_err("the second insert must roll back the whole batch");
    assert!(error.to_string().contains("reject failed search batch"));
    assert_eq!(
        suprnova::vector::Vector::store(index.name)?.count().await?,
        1
    );
    let result = driver
        .semantic(&index, &generation, &[1.0, 0.0, 0.0], None, 10)
        .await?;
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0].document, original.document);
    assert!(result.candidates[0].score > 0.99);
    Ok(())
}

#[tokio::test]
async fn sqlite_geo_facets_preserve_coordinates_counts_and_prefix_labels()
-> Result<(), FrameworkError> {
    use suprnova::search::{AttributeDecl, AttributeType, FacetOrder, GeoPoint};

    let _db = TestDatabase::fresh::<Migrator>().await?;
    let mut index = descriptor("database_sqlite_geo_facets");
    index.attributes.push(AttributeDecl {
        name: "location".into(),
        ty: AttributeType::Geo,
        searchable: None,
        filterable: true,
        sortable: true,
        facet: true,
    });
    let generation = create(&index).await?;
    let driver = engine();
    let first = GeoPoint {
        lat: 40.0,
        lng: -73.0,
    };
    let second = GeoPoint {
        lat: 41.0,
        lng: -73.0,
    };
    let docs = [
        ("a", "orchard", first),
        ("b", "orchard", first),
        ("c", "orchard", second),
        ("d", "basket", first),
    ]
    .map(|(key, title, point)| {
        let mut doc = document(&index, key, title, "garden");
        doc.document
            .attributes
            .insert("location".into(), AttributeValue::Geo(point));
        doc.content_hash = doc
            .document
            .content_hash(&index.index_settings().index_time.digest());
        doc
    });
    driver.index(&index, &generation, &docs).await?;
    let q = query("orchard");
    let facets = driver
        .facets(
            &index,
            &generation,
            &q,
            &["location".into()],
            10,
            FacetOrder::Count,
        )
        .await?;
    let values = &facets["location"];
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].value, AttributeValue::Geo(first));
    assert_eq!(values[0].count, 2);
    assert_eq!(values[1].value, AttributeValue::Geo(second));
    assert_eq!(values[1].count, 1);
    let label = AttributeValue::Geo(first).facet_key();
    let prefix = label
        .split_once(',')
        .expect("coordinate label")
        .0
        .to_owned()
        + ",";
    let found = driver
        .facet_search(&index, &generation, &q, "location", &prefix, 10)
        .await?;
    assert_eq!(found["location"].len(), 1);
    assert_eq!(found["location"][0].value, AttributeValue::Geo(first));
    assert_eq!(found["location"][0].count, 2);
    Ok(())
}

#[tokio::test]
async fn sqlite_colliding_generation_names_do_not_drop_existing_tables()
-> Result<(), FrameworkError> {
    let _db = TestDatabase::fresh::<Migrator>().await?;
    let first = descriptor("database_collision_a");
    let first_generation = GenerationId::new("b")?;
    let second = descriptor("database_collision");
    let second_generation = GenerationId::new("a_b")?;
    let driver = engine();
    driver
        .create_generation(&first, &first.index_settings(), &first_generation)
        .await?;
    driver
        .index(
            &first,
            &first_generation,
            &[document(&first, "a", "orchard", "garden")],
        )
        .await?;
    assert!(
        driver
            .create_generation(&second, &second.index_settings(), &second_generation)
            .await
            .is_err()
    );
    assert_eq!(
        driver.read_settings(&first, &first_generation).await?,
        first.index_settings()
    );
    assert_eq!(
        driver
            .count(&first, &first_generation, &query("orchard"))
            .await?,
        Total::Exact(1)
    );
    assert_eq!(
        driver
            .query(&first, &first_generation, &query("orchard"))
            .await?
            .candidates[0]
            .document
            .key,
        "a"
    );
    assert_eq!(
        driver
            .vocabulary(&first, &first_generation)
            .await?
            .words
            .get("orchard"),
        Some(&1)
    );
    Ok(())
}

#[tokio::test]
async fn sqlite_dense_delivery_retries_after_a_store_registration_failure()
-> Result<(), FrameworkError> {
    let _db = TestDatabase::fresh::<Migrator>().await?;
    let index = descriptor("database_sqlite_vector_retry");
    let generation = create(&index).await?;
    let mut doc = document(&index, "a", "orchard", "garden");
    doc.dense = Some(vec![1.0, 0.0, 0.0]);
    let error = engine()
        .index(&index, &generation, &[doc.clone()])
        .await
        .expect_err("an unregistered vector store must fail visibly");
    assert!(error.to_string().contains(index.name));
    vectors(index.name);
    let fresh = engine();
    let outcome = fresh.index(&index, &generation, &[doc]).await?;
    assert_eq!(outcome.skipped_unchanged, 1);
    let result = fresh
        .semantic(&index, &generation, &[1.0, 0.0, 0.0], None, 10)
        .await?;
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0].document.key, "a");
    assert!(result.candidates[0].score > 0.99);
    assert_eq!(
        suprnova::vector::Vector::store(index.name)?.count().await?,
        1
    );
    Ok(())
}

#[tokio::test]
async fn sqlite_searchable_lists_preserve_raw_text_token_boundaries() -> Result<(), FrameworkError>
{
    use suprnova::search::{AttributeDecl, AttributeType, SearchableDecl};

    let _db = TestDatabase::fresh::<Migrator>().await?;
    let mut index = descriptor("database_sqlite_text_lists");
    index.attributes.push(AttributeDecl {
        name: "tags".into(),
        ty: AttributeType::TextList,
        searchable: Some(SearchableDecl {
            weight: 1.0,
            order: 2,
        }),
        filterable: true,
        sortable: false,
        facet: true,
    });
    let generation = create(&index).await?;
    let driver = engine();
    let mut doc = document(&index, "a", "orchard", "garden");
    let tags = AttributeValue::List(vec![
        AttributeValue::Text("café\nbasket".into()),
        AttributeValue::Text("quoted words".into()),
    ]);
    doc.document.attributes.insert("tags".into(), tags.clone());
    doc.content_hash = doc
        .document
        .content_hash(&index.index_settings().index_time.digest());
    driver.index(&index, &generation, &[doc]).await?;
    let mut q = query("basket");
    q.highlights = true;
    let result = driver.query(&index, &generation, &q).await?;
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0].document.attributes["tags"], tags);
    let highlighted = result.candidates[0]
        .highlights
        .as_ref()
        .expect("native list highlight");
    assert!(highlighted["tags"].contains("<em>basket</em>"));
    assert!(!highlighted.contains_key("tags_text"));
    let snippets = driver.snippets(&index, &generation, &q, "tags", 4).await?;
    assert!(snippets["a"].contains("<em>basket</em>"));
    let vocabulary = driver.vocabulary(&index, &generation).await?;
    assert_eq!(vocabulary.words.get("basket"), Some(&1));
    assert!(!vocabulary.words.contains_key("nbasket"));
    let mut phrase = query("cafe");
    phrase.expanded.terms.clear();
    phrase.expanded.phrases = vec![vec!["cafe".into(), "basket".into()]];
    assert_eq!(
        driver.count(&index, &generation, &phrase).await?,
        Total::Exact(1)
    );
    let mut filtered = query("basket");
    filtered.filter = Some(Filter::Eq(
        "tags".into(),
        AttributeValue::Text("café\nbasket".into()),
    ));
    assert_eq!(
        driver.count(&index, &generation, &filtered).await?,
        Total::Exact(1)
    );
    driver
        .remove(&index, &generation, &[("a".into(), 2)])
        .await?;
    assert_eq!(
        driver.count(&index, &generation, &q).await?,
        Total::Exact(0)
    );
    assert!(
        !driver
            .vocabulary(&index, &generation)
            .await?
            .words
            .contains_key("basket")
    );
    Ok(())
}

#[tokio::test]
async fn sqlite_unchanged_content_can_repair_a_missing_dense_vector() -> Result<(), FrameworkError>
{
    let _db = TestDatabase::fresh::<Migrator>().await?;
    let index = descriptor("database_sqlite_vector_repair");
    vectors(index.name);
    let generation = create(&index).await?;
    let driver = engine();
    let mut doc = document(&index, "a", "orchard", "garden");
    driver.index(&index, &generation, &[doc.clone()]).await?;
    doc.dense = Some(vec![1.0, 0.0, 0.0]);
    let outcome = driver.index(&index, &generation, &[doc]).await?;
    assert_eq!(outcome.skipped_unchanged, 1);
    let result = driver
        .semantic(&index, &generation, &[1.0, 0.0, 0.0], None, 10)
        .await?;
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0].document.key, "a");
    assert!(result.candidates[0].score > 0.99);
    Ok(())
}

#[tokio::test]
async fn sqlite_semantic_filters_before_taking_the_cosine_top() -> Result<(), FrameworkError> {
    let _db = TestDatabase::fresh::<Migrator>().await?;
    let index = descriptor("database_sqlite_filtered_vectors");
    vectors(index.name);
    let generation = GenerationId::new("filtered")?;
    let mut settings = index.index_settings();
    settings.index_time.dimensions = Some(3);
    let driver = engine();
    driver
        .create_generation(&index, &settings, &generation)
        .await?;
    // Four neighbours closer to the query than the one document the filter admits.
    let mut documents = Vec::new();
    for (key, y) in [
        ("near0", 0.0),
        ("near1", 0.1),
        ("near2", 0.2),
        ("near3", 0.3),
    ] {
        let mut doc = document(&index, key, "excluded", "orchard");
        doc.dense = Some(vec![1.0, y, 0.0]);
        documents.push(doc);
    }
    let mut eligible = document(&index, "far", "eligible", "orchard");
    eligible.dense = Some(vec![1.0, 1.0, 0.0]);
    documents.push(eligible);
    for doc in &mut documents {
        doc.content_hash = doc.document.content_hash(&settings.index_time.digest());
    }
    driver.index(&index, &generation, &documents).await?;
    let admitted = Filter::Eq("title".into(), AttributeValue::Text("eligible".into()));
    let result = driver
        .semantic(&index, &generation, &[1.0, 0.0, 0.0], Some(&admitted), 1)
        .await?;
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0].document.key, "far");
    Ok(())
}
