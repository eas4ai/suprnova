//! The standing Qdrant service exercises the persistent search contract.
use std::sync::Arc;
use suprnova::{
    FrameworkError,
    search::{engine::qdrant::QdrantSearchEngine, testing::conformance},
};

async fn database() -> suprnova::database::testing::TestDatabase {
    let db = suprnova::database::testing::TestDatabase::sqlite_memory()
        .await
        .unwrap();
    for migration in suprnova::search::migrations() {
        migration
            .up(&sea_orm_migration::SchemaManager::new(db.conn()))
            .await
            .unwrap();
    }
    db
}
fn engine() -> QdrantSearchEngine {
    QdrantSearchEngine::from_url(
        &std::env::var("QDRANT_TEST_URL").expect("QDRANT_TEST_URL"),
        None,
        10,
    )
    .unwrap()
}
fn unique() -> &'static str {
    Box::leak(format!("qtest_{}", uuid::Uuid::new_v4().simple()).into_boxed_str())
}
#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn conformance_qdrant() -> Result<(), FrameworkError> {
    let _db = database().await;
    let engine = Arc::new(engine());
    let name = unique();
    let result = conformance::run(
        engine.clone(),
        conformance::Options {
            index_name: name,
            ..Default::default()
        },
    )
    .await;
    for generation in [
        "conformance_a",
        "conformance_b",
        "conformance_zero",
        "conformance_retired",
    ] {
        let collection = format!("search_{name}_{generation}");
        if engine
            .client()
            .expect("configured Qdrant client")
            .collection_exists(&collection)
            .await
            .unwrap()
        {
            engine
                .client()
                .expect("configured Qdrant client")
                .delete_collection(collection)
                .await
                .unwrap();
        }
    }
    result
}

use std::{collections::BTreeMap, future::Future, pin::Pin};
use suprnova::search::{
    AttributeDecl, AttributeType, AttributeValue, EmbedInput, Embedder, EngineQuery, FacetOrder,
    FakeEmbedder, Filter, GenerationId, IndexDescriptor, IndexSettings, IndexedDocument,
    LanguageConfig, QueryText, SearchDocument, SearchEngine, SearchRegistry, SearchableDecl, Sort,
    SortDirection, SparseEmbedding, Total, expand,
};
fn settings() -> IndexSettings {
    IndexSettings::default()
}
fn descriptor() -> IndexDescriptor {
    IndexDescriptor {
        name: unique(),
        model_table: "qtest_rows",
        key_column: "id",
        attributes: vec![
            AttributeDecl {
                name: "text".into(),
                ty: AttributeType::Text,
                searchable: Some(SearchableDecl {
                    weight: 1.0,
                    order: 0,
                }),
                filterable: true,
                sortable: true,
                facet: false,
            },
            AttributeDecl {
                name: "category".into(),
                ty: AttributeType::Text,
                searchable: None,
                filterable: true,
                sortable: true,
                facet: true,
            },
            AttributeDecl {
                name: "number".into(),
                ty: AttributeType::Integer,
                searchable: None,
                filterable: true,
                sortable: true,
                facet: true,
            },
            AttributeDecl {
                name: "ratio".into(),
                ty: AttributeType::Float,
                searchable: None,
                filterable: true,
                sortable: true,
                facet: true,
            },
            AttributeDecl {
                name: "active".into(),
                ty: AttributeType::Bool,
                searchable: None,
                filterable: true,
                sortable: true,
                facet: true,
            },
            AttributeDecl {
                name: "created".into(),
                ty: AttributeType::DateTime,
                searchable: None,
                filterable: true,
                sortable: true,
                facet: true,
            },
            AttributeDecl {
                name: "tags".into(),
                ty: AttributeType::TextList,
                searchable: None,
                filterable: true,
                sortable: false,
                facet: true,
            },
            AttributeDecl {
                name: "location".into(),
                ty: AttributeType::Geo,
                searchable: None,
                filterable: true,
                sortable: true,
                facet: true,
            },
        ],
        embed_source: None,
        soft_delete: false,
        search_touches: vec![],
        settings,
        scope: None,
    }
}
fn document(index: &IndexDescriptor, key: &str, text: &str, n: i64) -> IndexedDocument {
    let doc = SearchDocument {
        index: index.name.into(),
        key: key.into(),
        source_version: n,
        attributes: BTreeMap::from([
            ("text".into(), AttributeValue::Text(text.into())),
            (
                "category".into(),
                AttributeValue::Text(if n % 2 == 0 { "Fruit" } else { "basket" }.into()),
            ),
            ("number".into(), AttributeValue::Integer(n)),
            ("ratio".into(), AttributeValue::Float(n as f64 / 2.0)),
            ("active".into(), AttributeValue::Bool(n % 2 == 0)),
            (
                "created".into(),
                AttributeValue::DateTime(
                    chrono::DateTime::parse_from_rfc3339(&format!("2026-01-0{n}T00:00:00Z"))
                        .unwrap()
                        .with_timezone(&chrono::Utc),
                ),
            ),
            (
                "tags".into(),
                AttributeValue::List(vec![
                    AttributeValue::Text("common".into()),
                    AttributeValue::Text(if n % 2 == 0 { "fruit" } else { "basket" }.into()),
                ]),
            ),
            (
                "location".into(),
                AttributeValue::Geo(suprnova::GeoPoint {
                    lat: n as f64,
                    lng: 0.0,
                }),
            ),
        ]),
        embed_text: None,
        title: None,
    };
    IndexedDocument {
        content_hash: doc.content_hash("test"),
        document: doc,
        dense: None,
        sparse: None,
    }
}
async fn query(
    engine: &QdrantSearchEngine,
    index: &IndexDescriptor,
    generation: &GenerationId,
    text: &str,
) -> EngineQuery {
    let settings = engine.read_settings(index, generation).await.unwrap();
    EngineQuery {
        expanded: expand(
            &QueryText::parse(text),
            &settings.query,
            &engine.vocabulary(index, generation).await.unwrap(),
            64,
        )
        .unwrap(),
        filter: None,
        sort: vec![],
        depth: 100,
        highlights: true,
        crop: BTreeMap::new(),
    }
}
type TestFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
async fn fixture(
    test: impl for<'a> FnOnce(
        &'a QdrantSearchEngine,
        &'a IndexDescriptor,
        &'a GenerationId,
    ) -> TestFuture<'a>,
) {
    use futures::FutureExt;
    let _db = database().await;
    let engine = engine();
    let index = descriptor();
    let generation = GenerationId::new("g1").unwrap();
    let outcome = std::panic::AssertUnwindSafe(test(&engine, &index, &generation))
        .catch_unwind()
        .await;
    let collections = engine
        .client()
        .expect("configured Qdrant client")
        .list_collections()
        .await
        .unwrap();
    for collection in collections.collections {
        if collection
            .name
            .starts_with(&format!("search_{}_", index.name))
        {
            engine
                .client()
                .expect("configured Qdrant client")
                .delete_collection(collection.name)
                .await
                .unwrap();
        }
    }
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn payload_indexes_and_language_options_qdrant() {
    fixture(|engine, index, generation| {
        Box::pin(async move {
            let mut settings = index.index_settings();
            settings.index_time.dimensions = Some(3);
            settings.index_time.language = LanguageConfig::Qdrant {
                tokenizer: "word".into(),
                stemmer: Some("english".into()),
                stop_words: vec!["the".into()],
                folding: true,
            };
            engine
                .create_generation(index, &settings, generation)
                .await
                .unwrap();
            assert_eq!(
                engine.read_settings(index, generation).await.unwrap(),
                settings
            );
            let collection = format!("search_{}_g1", index.name);
            let info = engine
                .client()
                .expect("configured Qdrant client")
                .collection_info(&collection)
                .await
                .unwrap()
                .result
                .unwrap();
            use qdrant_client::qdrant::{
                PayloadSchemaType as FieldType, payload_index_params::IndexParams,
            };
            for (attr, ty) in [
                ("text", FieldType::Keyword),
                ("category", FieldType::Keyword),
                ("number", FieldType::Integer),
                ("ratio", FieldType::Float),
                ("active", FieldType::Bool),
                ("created", FieldType::Datetime),
                ("location", FieldType::Geo),
                ("tags", FieldType::Keyword),
                ("__key", FieldType::Keyword),
                ("__source_version", FieldType::Integer),
                ("__text", FieldType::Text),
            ] {
                assert_eq!(info.payload_schema[attr].data_type, ty as i32, "{attr}");
            }
            let params = info.payload_schema["__text"]
                .params
                .as_ref()
                .unwrap()
                .index_params
                .as_ref()
                .unwrap();
            let IndexParams::TextIndexParams(params) = params else {
                panic!("text params");
            };
            assert_eq!(params.lowercase, Some(true));
            assert_eq!(params.ascii_folding, Some(true));
            assert_eq!(params.phrase_matching, Some(true));
            assert_eq!(params.stopwords.as_ref().unwrap().custom, ["the"]);
            assert!(params.stemmer.is_some());
            assert_eq!(
                engine
                    .client()
                    .expect("configured Qdrant client")
                    .config
                    .timeout,
                std::time::Duration::from_secs(10)
            );
            assert!(
                QdrantSearchEngine::from_url("http://127.0.0.1:6334", None, 0)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("SEARCH_QDRANT_TIMEOUT_SECS")
            );
            for tokenizer in ["whitespace", "prefix", "multilingual"] {
                let mut other = settings.clone();
                other.index_time.language = LanguageConfig::Qdrant {
                    tokenizer: tokenizer.into(),
                    stemmer: None,
                    stop_words: vec![],
                    folding: false,
                };
                let generation = GenerationId::new(format!("tok_{tokenizer}")).unwrap();
                engine
                    .create_generation(index, &other, &generation)
                    .await
                    .unwrap();
                assert_eq!(
                    engine.read_settings(index, &generation).await.unwrap(),
                    other
                );
            }
            let mut doc = document(index, "01", "The café is running", 1);
            doc.dense = Some(vec![1.0, 0.0, 0.0]);
            engine.index(index, generation, &[doc]).await.unwrap();
            for term in ["cafe", "run", "caf"] {
                assert_eq!(
                    engine
                        .count(
                            index,
                            generation,
                            &query(engine, index, generation, term).await
                        )
                        .await
                        .unwrap(),
                    Total::Exact(1)
                );
            }
            let point = engine
                .client()
                .expect("configured Qdrant client")
                .get_points(
                    qdrant_client::qdrant::GetPointsBuilder::new(
                        &collection,
                        vec![suprnova::vector::qdrant::QdrantVectorDriver::resolve_point_id("01")],
                    )
                    .with_payload(true),
                )
                .await
                .unwrap()
                .result
                .remove(0);
            let payload: serde_json::Value = qdrant_client::Payload::from(point.payload).into();
            assert_eq!(payload["__suprnova_id"], "01");
            assert_eq!(
                payload["location"],
                serde_json::json!({"lat":1.0,"lon":0.0})
            );
        })
    })
    .await;
}
#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn filter_operators_and_scope_qdrant() {
    fixture(|engine, index, generation| {
        Box::pin(async move {
            engine
                .create_generation(index, &index.index_settings(), generation)
                .await
                .unwrap();
            let mut missing = document(index, "missing", "apple", 4);
            missing
                .document
                .attributes
                .insert("number".into(), AttributeValue::Null);
            missing
                .document
                .attributes
                .insert("tags".into(), AttributeValue::List(vec![]));
            engine
                .index(
                    index,
                    generation,
                    &[
                        document(index, "a", "apple orchard", 1),
                        document(index, "b", "green apple", 2),
                        document(index, "c", "banana", 3),
                        missing,
                    ],
                )
                .await
                .unwrap();
            for (grammar, expected) in [
                ("number = 2", 1),
                ("number != 2", 3),
                ("number IN [1,2]", 2),
                ("number NOT IN [1,2]", 2),
                ("number > 1", 2),
                ("number >= 2", 2),
                ("number < 3", 2),
                ("number <= 2", 2),
                ("number 1 TO 2", 2),
                ("number IS NULL", 1),
                ("number IS NOT NULL", 3),
                ("tags IS NULL", 0),
                ("tags IS NOT NULL", 4),
                ("active = true", 2),
                ("active != true", 2),
                ("ratio = 1.0", 1),
                ("ratio 0.5 TO 1.0", 2),
                ("category IN ['basket']", 2),
                ("tags NOT IN ['basket']", 2),
                ("created = '2026-01-02T00:00:00Z'", 1),
                ("created > '2026-01-02T00:00:00Z'", 2),
                ("created < '2026-01-03T00:00:00Z'", 2),
                ("created <= '2026-01-02T00:00:00Z'", 2),
                (
                    "created '2026-01-02T00:00:00Z' TO '2026-01-03T00:00:00Z'",
                    2,
                ),
                (
                    "NOT (number = 1) AND (active = true OR category = 'basket')",
                    3,
                ),
                ("_geoRadius(1,0,1000)", 1),
            ] {
                let mut q = query(engine, index, generation, "").await;
                q.filter = Some(Filter::parse(grammar, index).unwrap());
                assert_eq!(
                    engine.count(index, generation, &q).await.unwrap().value(),
                    expected,
                    "{grammar}"
                );
            }
            for (filter, count) in [
                (Filter::And(vec![]), 4),
                (Filter::Or(vec![]), 0),
                (Filter::In("category".into(), vec![]), 0),
                (Filter::NotIn("category".into(), vec![]), 4),
            ] {
                let mut q = query(engine, index, generation, "").await;
                q.filter = Some(filter);
                assert_eq!(
                    engine.count(index, generation, &q).await.unwrap().value(),
                    count
                );
            }
            let mut q = query(engine, index, generation, "apple").await;
            q.filter = Some(Filter::Eq("active".into(), AttributeValue::Bool(true)));
            assert_eq!(
                engine.count(index, generation, &q).await.unwrap().value(),
                2
            );
            let result = engine.query(index, generation, &q).await.unwrap();
            assert!(
                result
                    .candidates
                    .iter()
                    .all(|c| q.filter.as_ref().unwrap().matches(&c.document))
            );
        })
    })
    .await;
}
#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn exact_facets_stats_and_list_prefix_qdrant() {
    fixture(|engine, index, generation| {
        Box::pin(async move {
            engine
                .create_generation(index, &index.index_settings(), generation)
                .await
                .unwrap();
            let mut first = document(index, "a", "apple orchard", 1);
            first.document.attributes.insert(
                "tags".into(),
                AttributeValue::List(
                    (0..150)
                        .map(|n| AttributeValue::Text(format!("other_{n}")))
                        .chain([
                            AttributeValue::Text("fruit".into()),
                            AttributeValue::Text("fruit".into()),
                        ])
                        .collect(),
                ),
            );
            engine
                .index(
                    index,
                    generation,
                    &[
                        first,
                        document(index, "b", "green apple", 2),
                        document(index, "c", "banana", 3),
                    ],
                )
                .await
                .unwrap();
            let mut q = query(engine, index, generation, "apple").await;
            q.depth = 1;
            let attrs =
                ["category", "number", "ratio", "active", "created", "tags"].map(str::to_owned);
            let facets = engine
                .facets(index, generation, &q, &attrs, 200, FacetOrder::Count)
                .await
                .unwrap();
            for (attr, values) in facets {
                for value in values {
                    let mut filtered = q.clone();
                    filtered.filter = Some(Filter::Eq(attr.clone(), value.value));
                    assert_eq!(
                        engine
                            .count(index, generation, &filtered)
                            .await
                            .unwrap()
                            .value(),
                        value.count,
                        "{attr}"
                    );
                }
            }
            let stats = engine
                .facet_stats(
                    index,
                    generation,
                    &q,
                    &["number".into(), "ratio".into(), "created".into()],
                )
                .await
                .unwrap();
            assert_eq!(stats["number"].min, AttributeValue::Integer(1));
            assert_eq!(stats["number"].max, AttributeValue::Integer(2));
            let found = engine
                .facet_search(index, generation, &q, "tags", "FR", 1)
                .await
                .unwrap();
            assert_eq!(found["tags"].len(), 1);
            assert_eq!(found["tags"][0].value, AttributeValue::Text("fruit".into()));
            assert_eq!(found["tags"][0].count, 2);
        })
    })
    .await;
}
#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn versions_tombstones_and_vocabulary_restart_qdrant() {
    fixture(|engine, index, generation| {
        Box::pin(async move {
            engine
                .create_generation(index, &index.index_settings(), generation)
                .await
                .unwrap();
            assert!(!engine.capabilities().native_crop);
            let conn = suprnova::DB::connection().unwrap();
            let manager = sea_orm_migration::SchemaManager::new(conn.inner());
            assert!(!manager.has_table("search_index_states").await.unwrap());
            engine.switch_generation(index, None, generation.clone()).await.unwrap();
            let database_engine = suprnova::search::DatabaseSearchEngine::new();
            assert_eq!(database_engine.current_generation(index.name).await.unwrap(), Some(generation.clone()));
            assert_eq!(database_engine.read_settings(index, generation).await.unwrap(), index.index_settings());
            let mut newest = document(index, "a", "newer orchard", 2);
            let invalid_key = document(index, &"k".repeat(256), "invalid key", 1);
            assert!(engine.index(index, generation, &[newest.clone(), invalid_key]).await.is_err());
            let invalid_word = document(index, "invalid", &"w".repeat(256), 1);
            assert!(engine.index(index, generation, &[newest.clone(), invalid_word]).await.is_err());
            assert!(engine.vocabulary(index, generation).await.unwrap().words.is_empty());
            assert!(engine.remove(index, generation, &[("k".repeat(256), 1)]).await.is_err());

            engine
                .index(index, generation, &[newest.clone()])
                .await
                .unwrap();
            assert_eq!(
                engine
                    .index(
                        index,
                        generation,
                        &[document(index, "a", "older orchard", 1)]
                    )
                    .await
                    .unwrap()
                    .skipped_older,
                1
            );
            let hashes = engine.stored_hashes(index, generation,
                &[newest.clone(), document(index, "missing", "absent", 1)]).await.unwrap();
            assert_eq!(hashes, BTreeMap::from([("a".into(), newest.content_hash.clone())]));
            use sea_orm::ConnectionTrait;
            let row = conn.inner().query_one_raw(sea_orm::Statement::from_sql_and_values(
                sea_orm::DbBackend::Sqlite,
                "SELECT document_frequency FROM search_vocabulary WHERE index_name=? AND generation=? AND word=?",
                [index.name.into(), generation.as_str().into(), "newer".into()],
            )).await.unwrap().unwrap();
            assert_eq!(row.try_get::<i64>("", "document_frequency").unwrap(), 1);
            newest.document.source_version = 4;
            assert_eq!(
                engine
                    .index(index, generation, &[newest.clone()])
                    .await
                    .unwrap()
                    .skipped_unchanged,
                1
            );
            assert_eq!(
                engine
                    .remove(index, generation, &[("a".into(), 3)])
                    .await
                    .unwrap()
                    .skipped_older,
                1
            );
            engine
                .remove(index, generation, &[("a".into(), 5)])
                .await
                .unwrap();
            let row = conn.inner().query_one_raw(sea_orm::Statement::from_sql_and_values(
                sea_orm::DbBackend::Sqlite,
                "SELECT source_version, removed_at FROM search_tombstones WHERE index_name=? AND generation=? AND document_key=?",
                [index.name.into(), generation.as_str().into(), "a".into()],
            )).await.unwrap().unwrap();
            assert_eq!(row.try_get::<i64>("", "source_version").unwrap(), 5);
            assert!(row.try_get::<i64>("", "removed_at").unwrap() <= chrono::Utc::now().timestamp());
            let restarted = crate::qdrant::engine();
            assert_eq!(
                restarted
                    .index(index, generation, &[newest.clone()])
                    .await
                    .unwrap()
                    .skipped_tombstoned,
                1
            );
            assert!(
                !restarted
                    .vocabulary(index, generation)
                    .await
                    .unwrap()
                    .words
                    .contains_key("newer")
            );
            // Removal fences use the same generation scope as the database driver.
            let other = GenerationId::new("g2").unwrap();
            engine
                .create_generation(index, &index.index_settings(), &other)
                .await
                .unwrap();
            assert_eq!(
                restarted
                    .index(index, &other, &[newest.clone()])
                    .await
                    .unwrap()
                    .written,
                1
            );
            newest.document.source_version = 6;
            assert_eq!(
                restarted
                    .index(index, generation, &[newest])
                    .await
                    .unwrap()
                    .written,
                1
            );
            assert_eq!(
                restarted.vocabulary(index, generation).await.unwrap().words["newer"],
                1
            );
        })
    })
    .await;
}
#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn generation_switch_drop_and_settings_drift_qdrant() {
    fixture(|engine, index, generation| {
        Box::pin(async move {
            let settings = index.index_settings();
            engine
                .create_generation(index, &settings, generation)
                .await
                .unwrap();
            engine
                .switch_generation(index, None, generation.clone())
                .await
                .unwrap();
            let next = GenerationId::new("g2").unwrap();
            engine
                .create_generation(index, &settings, &next)
                .await
                .unwrap();
            let restarted = crate::qdrant::engine();
            assert_eq!(
                restarted.current_generation(index.name).await.unwrap(),
                Some(generation.clone())
            );
            restarted
                .switch_generation(index, Some(generation.clone()), next.clone())
                .await
                .unwrap();
            assert!(
                engine
                    .switch_generation(index, Some(generation.clone()), generation.clone())
                    .await
                    .is_err()
            );
            assert!(engine.drop_generation(index, &next).await.is_err());
            engine.drop_generation(index, generation).await.unwrap();
            assert!(
                !engine
                    .client()
                    .expect("configured Qdrant client")
                    .collection_exists(format!("search_{}_g1", index.name))
                    .await
                    .unwrap()
            );
            let mut changed = settings.clone();
            changed.query.hits_cap = 27;
            engine.write_settings(index, &next, &changed).await.unwrap();
            assert_eq!(
                restarted.read_settings(index, &next).await.unwrap(),
                changed
            );
            changed.index_time.dimensions = Some(7);
            assert!(engine.write_settings(index, &next, &changed).await.is_err());
            engine
                .client()
                .expect("configured Qdrant client")
                .delete_field_index(
                    qdrant_client::qdrant::DeleteFieldIndexCollectionBuilder::new(
                        format!("search_{}_g2", index.name),
                        "__text",
                    )
                    .wait(true),
                )
                .await
                .unwrap();
            assert!(
                engine
                    .read_settings(index, &next)
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("__text")
            );
        })
    })
    .await;
}
#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn weighted_hybrid_and_semantic_fake_vectors_qdrant() {
    fixture(|engine, index, generation| {
        Box::pin(async move {
            let fake = FakeEmbedder::new(3).unwrap();
            let mut settings = index.index_settings();
            settings.index_time.dimensions = Some(3);
            engine
                .create_generation(index, &settings, generation)
                .await
                .unwrap();
            // "b" is one token longer than "a": BM25 saturation then orders the
            // lexical leg deterministically instead of leaving a tie to Qdrant.
            let mut docs = vec![
                document(index, "a", "apple orchard", 1),
                document(index, "b", "green apple basket", 2),
                document(index, "c", "purple banana", 3),
            ];
            for doc in &mut docs {
                doc.dense = Some(
                    fake.embed_documents(&[EmbedInput {
                        text: doc.document.attributes["text"].as_text().unwrap().into(),
                        title: None,
                        media: vec![],
                        document_template: None,
                    }])
                    .await
                    .unwrap()
                    .remove(0)
                    .dense,
                );
            }
            engine.index(index, generation, &docs).await.unwrap();
            let q = query(engine, index, generation, "apple").await;
            let vector = fake.embed_query("purple banana").await.unwrap().dense;
            // The native reference uses the same server BM25 options as the
            // lexical leg, so the assertion still checks Qdrant's RRF scores.
            use qdrant_client::qdrant as qb;
            let sparse = qb::Document {
                text: "apple".into(),
                model: "qdrant/bm25".into(),
                options: std::collections::HashMap::from([
                    ("tokenizer".into(), qb::Value::from("word")),
                    (
                        "stemmer".into(),
                        qb::Value::from(serde_json::json!({"type":"none"})),
                    ),
                    ("stopwords".into(), qb::Value::from(serde_json::json!({}))),
                    ("ascii_folding".into(), qb::Value::from(true)),
                    (
                        "avg_len".into(),
                        qb::Value::from(
                            suprnova::SearchConfig::current()
                                .unwrap()
                                .bm25_average_length as i64,
                        ),
                    ),
                ]),
            };
            let lexical = engine
                .hybrid(index, generation, &q, &vector, 0.0)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(lexical.total, Total::Exact(2));
            assert!(lexical.candidates.iter().all(|c| c.document.key != "c"));
            let dense = engine
                .hybrid(index, generation, &q, &vector, 1.0)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(dense.candidates[0].document.key, "c");
            let mixed = engine
                .hybrid(index, generation, &q, &vector, 0.7)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(mixed.candidates.len(), 3);
            let collection = format!("search_{}_g1", index.name);
            let native = engine
                .client()
                .expect("configured Qdrant client")
                .query(
                    qb::QueryPointsBuilder::new(&collection)
                        .prefetch(vec![
                            qb::PrefetchQueryBuilder::default()
                                .query(qb::Query::from(qb::VectorInput::from(sparse)))
                                .using("bm25_text")
                                .filter(engine.render_filter(index, generation, &q).await.unwrap())
                                .limit(100u64)
                                .build(),
                            qb::PrefetchQueryBuilder::default()
                                .query(vector.clone())
                                .using("dense")
                                .limit(100u64)
                                .build(),
                        ])
                        .query(qb::Query {
                            variant: Some(qb::query::Variant::Rrf(qb::Rrf {
                                k: Some(60),
                                weights: vec![0.3, 0.7],
                            })),
                        })
                        .limit(200)
                        .with_payload(true),
                )
                .await
                .unwrap();
            let native_scores = native
                .result
                .into_iter()
                .map(|p| {
                    let payload: serde_json::Value = qdrant_client::Payload::from(p.payload).into();
                    (payload["__key"].as_str().unwrap().to_owned(), p.score)
                })
                .collect::<BTreeMap<_, _>>();
            for candidate in mixed.candidates {
                assert!(
                    (candidate.score - f64::from(native_scores[&candidate.document.key])).abs()
                        < 1e-6
                );
            }
            let negative = query(engine, index, generation, "apple -banana").await;
            for ratio in [0.7, 1.0] {
                let result = engine
                    .hybrid(index, generation, &negative, &vector, ratio)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(result.candidates.len(), 2);
                assert!(
                    result
                        .candidates
                        .iter()
                        .all(|candidate| candidate.document.key != "c")
                );
            }
            let filter = Filter::Eq("active".into(), AttributeValue::Bool(true));
            let scoped = engine
                .semantic(index, generation, &vector, Some(&filter), 10)
                .await
                .unwrap();
            assert_eq!(scoped.candidates.len(), 1);
            assert_eq!(scoped.candidates[0].document.key, "b");
        })
    })
    .await;
}
struct SparseFake(FakeEmbedder);
#[async_trait::async_trait]
impl Embedder for SparseFake {
    fn profile(&self) -> &suprnova::search::ModelProfile {
        self.0.profile()
    }
    async fn embed_documents(
        &self,
        inputs: &[EmbedInput],
    ) -> Result<Vec<suprnova::search::Embedding>, FrameworkError> {
        self.0.embed_documents(inputs).await
    }
    async fn embed_query(&self, text: &str) -> Result<suprnova::search::Embedding, FrameworkError> {
        self.0.embed_query(text).await
    }
    async fn embed_sparse(&self, texts: &[String]) -> Result<Vec<SparseEmbedding>, FrameworkError> {
        Ok(texts
            .iter()
            .map(|text| {
                let mut values = BTreeMap::<u32, f32>::new();
                for word in suprnova::search::tokenize(text) {
                    let id = word
                        .bytes()
                        .fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(u32::from(b)));
                    *values.entry(id).or_default() += 1.0;
                }
                SparseEmbedding {
                    indices: values.keys().copied().collect(),
                    values: values.values().copied().collect(),
                }
            })
            .collect())
    }
}
#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn sparse_model_ranking_and_idf_qdrant() {
    SearchRegistry::isolated(async {
        fixture(|engine, index, generation| {
            Box::pin(async move {
                let provider = Arc::new(SparseFake(FakeEmbedder::new(3).unwrap()));
                SearchRegistry::register_embedder("sparse-test", provider.clone()).unwrap();
                let mut settings = index.index_settings();
                settings.index_time.sparse_model = Some("sparse-test".into());
                settings.index_time.dimensions = Some(3);
                engine
                    .create_generation(index, &settings, generation)
                    .await
                    .unwrap();
                let mut docs = vec![
                    document(index, "a", "apple orchard", 1),
                    document(index, "b", "apple apple orchard", 2),
                    document(index, "c", "banana", 3),
                ];
                for doc in &mut docs {
                    doc.sparse = Some(
                        provider
                            .embed_sparse(&[doc.document.attributes["text"]
                                .as_text()
                                .unwrap()
                                .into()])
                            .await
                            .unwrap()
                            .remove(0),
                    );
                    doc.dense = Some(
                        provider
                            .embed_query(doc.document.attributes["text"].as_text().unwrap())
                            .await
                            .unwrap()
                            .dense,
                    );
                }
                engine.index(index, generation, &docs).await.unwrap();
                let q = query(engine, index, generation, "apple").await;
                assert_eq!(
                    engine
                        .query(index, generation, &q)
                        .await
                        .unwrap()
                        .candidates[0]
                        .document
                        .key,
                    "b"
                );
                let vector = provider.embed_query("banana").await.unwrap().dense;
                assert_eq!(
                    engine
                        .hybrid(index, generation, &q, &vector, 0.5)
                        .await
                        .unwrap()
                        .unwrap()
                        .candidates
                        .len(),
                    3
                );
                let info = engine
                    .client()
                    .expect("configured Qdrant client")
                    .collection_info(format!("search_{}_g1", index.name))
                    .await
                    .unwrap()
                    .result
                    .unwrap();
                assert_eq!(
                    info.config
                        .unwrap()
                        .params
                        .unwrap()
                        .sparse_vectors_config
                        .unwrap()
                        .map["sparse"]
                        .modifier,
                    Some(qdrant_client::qdrant::Modifier::Idf as i32)
                );
                let mut changed = docs[..2].to_vec();
                changed[0].document.source_version = 10;
                for doc in &mut changed {
                    doc.document
                        .attributes
                        .insert("category".into(), AttributeValue::Text("same".into()));
                    doc.content_hash = doc.document.content_hash("updated");
                }
                engine.index(index, generation, &changed).await.unwrap();
                let mut sorted = q.clone();
                sorted.sort = vec![Sort::Attribute {
                    attribute: "category".into(),
                    direction: SortDirection::Asc,
                }];
                assert_eq!(
                    engine
                        .query(index, generation, &sorted)
                        .await
                        .unwrap()
                        .candidates[0]
                        .document
                        .key,
                    "b"
                );
                let mut no_sparse = document(index, "d", "apple", 4);
                no_sparse.dense = Some(vec![1.0, 0.0, 0.0]);
                assert!(engine.index(index, generation, &[no_sparse]).await.is_err());
            })
        })
        .await;
    })
    .await;
}
#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn lexical_no_embedder_prefix_phrase_and_depth_qdrant() {
    fixture(|engine, index, generation| {
        Box::pin(async move {
            engine
                .create_generation(index, &index.index_settings(), generation)
                .await
                .unwrap();
            engine
                .index(
                    index,
                    generation,
                    &[
                        document(index, "a", "red apple orchard", 1),
                        document(index, "b", "apple red orchard", 2),
                        document(index, "c", "green apple basket", 3),
                    ],
                )
                .await
                .unwrap();
            for (text, count) in [
                ("orch", 2),
                ("\"red apple\"", 1),
                ("apple -green", 2),
                ("applf", 3),
                ("apple orchard", 3),
            ] {
                assert_eq!(
                    engine
                        .count(
                            index,
                            generation,
                            &query(engine, index, generation, text).await
                        )
                        .await
                        .unwrap()
                        .value(),
                    count,
                    "{text}"
                );
            }
            let mut q = query(engine, index, generation, "").await;
            q.depth = 2;
            assert_eq!(
                engine
                    .query(index, generation, &q)
                    .await
                    .unwrap()
                    .candidates
                    .iter()
                    .map(|c| c.document.key.as_str())
                    .collect::<Vec<_>>(),
                ["c", "b"]
            );
            q.sort = vec![Sort::Attribute {
                attribute: "category".into(),
                direction: SortDirection::Asc,
            }];
            q.depth = 1;
            assert_eq!(
                engine
                    .query(index, generation, &q)
                    .await
                    .unwrap()
                    .candidates[0]
                    .document
                    .key,
                "b"
            );
            assert!(
                engine
                    .semantic(index, generation, &[1.0], None, 10)
                    .await
                    .is_err()
            );
            let info = engine
                .client()
                .expect("configured Qdrant client")
                .collection_info(format!("search_{}_g1", index.name))
                .await
                .unwrap()
                .result
                .unwrap();
            let qdrant_client::qdrant::vectors_config::Config::ParamsMap(params) = info
                .config
                .unwrap()
                .params
                .unwrap()
                .vectors_config
                .unwrap()
                .config
                .unwrap()
            else {
                panic!("named dense");
            };
            assert_eq!(params.map["dense"].size, 1);
        })
    })
    .await;
}

#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn structural_lists_ordered_scalars_and_large_integers_qdrant() {
    fixture(|engine, index, generation| {
        Box::pin(async move {
            engine
                .create_generation(index, &index.index_settings(), generation)
                .await
                .unwrap();
            let mut first = document(index, "a", "apple", 1);
            first.document.attributes.insert(
                "number".into(),
                AttributeValue::Integer(9_007_199_254_740_993),
            );
            let mut second = document(index, "b", "banana", 2);
            second.document.attributes.insert(
                "number".into(),
                AttributeValue::Integer(9_007_199_254_740_994),
            );
            engine
                .index(index, generation, &[first, second])
                .await
                .unwrap();
            for (filter, expected) in [
                (
                    Filter::Eq(
                        "tags".into(),
                        AttributeValue::List(vec![
                            AttributeValue::Text("common".into()),
                            AttributeValue::Text("basket".into()),
                        ]),
                    ),
                    1,
                ),
                (
                    Filter::Gt(
                        "number".into(),
                        AttributeValue::Integer(9_007_199_254_740_993),
                    ),
                    1,
                ),
                (
                    Filter::Gte(
                        "number".into(),
                        AttributeValue::Integer(9_007_199_254_740_994),
                    ),
                    1,
                ),
                (
                    Filter::Lt(
                        "number".into(),
                        AttributeValue::Integer(9_007_199_254_740_994),
                    ),
                    1,
                ),
                (
                    Filter::Between(
                        "number".into(),
                        AttributeValue::Integer(9_007_199_254_740_993),
                        AttributeValue::Integer(9_007_199_254_740_994),
                    ),
                    2,
                ),
                (
                    Filter::Gt("text".into(), AttributeValue::Text("apple".into())),
                    1,
                ),
                (
                    Filter::Lt("text".into(), AttributeValue::Text("banana".into())),
                    1,
                ),
                (
                    Filter::Between(
                        "text".into(),
                        AttributeValue::Text("apple".into()),
                        AttributeValue::Text("banana".into()),
                    ),
                    2,
                ),
                (Filter::Gt("active".into(), AttributeValue::Bool(false)), 1),
                (Filter::Lte("active".into(), AttributeValue::Bool(false)), 1),
            ] {
                let mut q = query(engine, index, generation, "").await;
                q.filter = Some(filter);
                assert_eq!(
                    engine.count(index, generation, &q).await.unwrap().value(),
                    expected,
                    "{:?}",
                    q.filter
                );
            }
        })
    })
    .await;
}
#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn concurrent_instances_preserve_newest_write_and_removal_qdrant() {
    fixture(|engine, index, generation| {
        Box::pin(async move {
            engine
                .create_generation(index, &index.index_settings(), generation)
                .await
                .unwrap();
            let other = crate::qdrant::engine();
            let mut newest = document(index, "a", "newest orchard", 2);
            newest.document.source_version = 10;
            let older = document(index, "a", "old orchard", 1);
            let new_batch = [newest.clone()];
            let old_batch = [older];
            let (new, old) = tokio::join!(
                engine.index(index, generation, &new_batch),
                other.index(index, generation, &old_batch)
            );
            new.unwrap();
            old.unwrap();
            let q = query(engine, index, generation, "").await;
            let result = engine.query(index, generation, &q).await.unwrap();
            assert_eq!(result.candidates[0].document.source_version, 10);
            let newer = document(index, "b", "restored orchard", 4);
            let newer_batch = [newer];
            let removals = [("b".into(), 3)];
            let (write, remove) = tokio::join!(
                engine.index(index, generation, &newer_batch),
                other.remove(index, generation, &removals)
            );
            write.unwrap();
            remove.unwrap();
            let result = engine.query(index, generation, &q).await.unwrap();
            assert!(
                result
                    .candidates
                    .iter()
                    .any(|c| c.document.key == "b" && c.document.source_version == 4)
            );
            let final_batch = [newest];
            let final_removals = [("a".into(), 11)];
            let (write, remove) = tokio::join!(
                engine.index(index, generation, &final_batch),
                other.remove(index, generation, &final_removals)
            );
            write.unwrap();
            remove.unwrap();
            assert!(
                !engine
                    .query(index, generation, &q)
                    .await
                    .unwrap()
                    .candidates
                    .iter()
                    .any(|c| c.document.key == "a")
            );
        })
    })
    .await;
}
#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn sparse_prefix_and_synonym_field_boundaries_qdrant() {
    SearchRegistry::isolated(async {
        fixture(|engine, index, generation| {
            Box::pin(async move {
                let provider = Arc::new(SparseFake(FakeEmbedder::new(3).unwrap()));
                SearchRegistry::register_embedder("sparse-prefix", provider.clone()).unwrap();
                let mut index = IndexDescriptor {
                    name: index.name,
                    model_table: index.model_table,
                    key_column: index.key_column,
                    attributes: index.attributes.clone(),
                    embed_source: None,
                    soft_delete: false,
                    search_touches: vec![],
                    settings,
                    scope: None,
                };
                index
                    .attributes
                    .iter_mut()
                    .find(|a| a.name == "category")
                    .unwrap()
                    .searchable = Some(SearchableDecl {
                    weight: 1.0,
                    order: 1,
                });
                let mut settings = index.index_settings();
                settings.index_time.sparse_model = Some("sparse-prefix".into());
                settings.index_time.dimensions = Some(3);
                settings.query.synonyms = vec![suprnova::search::Synonym {
                    terms: vec!["nyc".into(), "new york".into()],
                    one_way: true,
                }];
                engine
                    .create_generation(&index, &settings, generation)
                    .await
                    .unwrap();
                let mut docs = vec![
                    document(&index, "a", "apple", 1),
                    document(&index, "b", "new", 2),
                    document(&index, "c", "new york", 3),
                ];
                docs[1]
                    .document
                    .attributes
                    .insert("category".into(), AttributeValue::Text("york".into()));
                for doc in &mut docs {
                    doc.sparse = Some(
                        provider
                            .embed_sparse(&[doc.document.attributes["text"]
                                .as_text()
                                .unwrap()
                                .into()])
                            .await
                            .unwrap()
                            .remove(0),
                    );
                    doc.dense = Some(vec![1.0, 0.0, 0.0]);
                }
                engine.index(&index, generation, &docs).await.unwrap();
                for (text, key) in [("app", "a"), ("nyc", "c")] {
                    let q = query(engine, &index, generation, text).await;
                    let result = engine.query(&index, generation, &q).await.unwrap();
                    assert_eq!(result.total, Total::Exact(1));
                    assert_eq!(result.candidates.len(), 1);
                    assert_eq!(result.candidates[0].document.key, key);
                    let result = engine
                        .hybrid(&index, generation, &q, &[1.0, 0.0, 0.0], 0.2)
                        .await
                        .unwrap()
                        .unwrap();
                    assert!(result.candidates.iter().any(|c| c.document.key == key));
                }
            })
        })
        .await;
    })
    .await;
}

#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn builder_index_job_highlights_and_native_fusion_qdrant() {
    use super::derive::{SearchNote, database, note};
    use futures::FutureExt;
    use suprnova::{SearchConfig, SearchDriver, Searchable, queue::Job};
    let db = database().await;
    for migration in suprnova::search::migrations() {
        migration
            .up(&sea_orm_migration::SchemaManager::new(db.conn()))
            .await
            .unwrap();
    }
    let engine = Arc::new(crate::qdrant::engine());
    let descriptor = SearchNote::descriptor();
    let generation =
        GenerationId::new(format!("builder_{}", uuid::Uuid::new_v4().simple())).unwrap();
    let outcome = std::panic::AssertUnwindSafe(SearchConfig::scope(
        SearchConfig {
            driver: SearchDriver::Collection,
            queue: false,
            ..Default::default()
        },
        SearchRegistry::isolated(async {
            let public = note("Rust public <script>", true, 0).await;
            let private = note("Rust private", true, 9).await;
            SearchRegistry::register_engine("qdrant", engine.clone()).unwrap();
            SearchRegistry::set_index_driver(descriptor.name, "qdrant").unwrap();
            SearchRegistry::register_embedder(
                "qdrant-dense",
                Arc::new(FakeEmbedder::new(3).unwrap()),
            )
            .unwrap();
            SearchRegistry::register_embedder(
                "qdrant-sparse",
                Arc::new(SparseFake(FakeEmbedder::new(3).unwrap())),
            )
            .unwrap();
            let mut settings = descriptor.index_settings();
            settings.index_time.embedder = Some("qdrant-dense".into());
            settings.index_time.sparse_model = Some("qdrant-sparse".into());
            settings.index_time.dimensions = Some(3);
            engine
                .create_generation(descriptor, &settings, &generation)
                .await
                .unwrap();
            engine
                .switch_generation(descriptor, None, generation.clone())
                .await
                .unwrap();
            for model in [public, private] {
                let doc = model.to_search_document().unwrap();
                suprnova::SearchIndexJob {
                    index: doc.index,
                    key: doc.key,
                    source_version: doc.source_version,
                    op: suprnova::IndexOperation::Index,
                    generation: None,
                }
                .handle()
                .await
                .unwrap();
            }
            let result = SearchNote::search("pubic")
                .hybrid(0.7)
                .highlight(["title"])
                .facets(["category"])
                .get()
                .await
                .unwrap();
            assert_eq!(result.engine, "qdrant");
            assert_eq!(result.hits.len(), 1);
            assert_eq!(result.facets["category"][0].count, 1);
            assert!(result.hits[0].formatted["title"].contains("<em>public</em>"));
            assert!(result.hits[0].formatted["title"].contains("&lt;script&gt;"));
            assert!(result.hits[0].engine_score > 0.0 && result.hits[0].engine_score < 1.0);
            assert_eq!(result.hits[0].model.owner_id, 0);
        }),
    ))
    .catch_unwind()
    .await;
    let collection = format!("search_{}_{}", descriptor.name, generation.as_str());
    if engine
        .client()
        .expect("configured Qdrant client")
        .collection_exists(&collection)
        .await
        .unwrap()
    {
        engine
            .client()
            .expect("configured Qdrant client")
            .delete_collection(collection)
            .await
            .unwrap();
    }
    match outcome {
        Ok(result) => result.unwrap(),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn native_ties_keep_document_key_prefix_qdrant() {
    SearchRegistry::isolated(async {
        fixture(|engine, index, generation| {
            Box::pin(async move {
                let provider = Arc::new(SparseFake(FakeEmbedder::new(3).unwrap()));
                SearchRegistry::register_embedder("tie-sparse", provider.clone()).unwrap();
                let mut settings = index.index_settings();
                settings.index_time.dimensions = Some(3);
                settings.index_time.sparse_model = Some("tie-sparse".into());
                engine
                    .create_generation(index, &settings, generation)
                    .await
                    .unwrap();
                let sparse = provider
                    .embed_sparse(&["apple".into()])
                    .await
                    .unwrap()
                    .remove(0);
                let docs = (0..20)
                    .map(|n| {
                        let mut doc = document(index, &format!("key_{n:03}"), "apple", 1);
                        doc.dense = Some(vec![1.0, 0.0, 0.0]);
                        doc.sparse = Some(sparse.clone());
                        doc
                    })
                    .collect::<Vec<_>>();
                engine.index(index, generation, &docs).await.unwrap();
                for depth in [1, 2, 7] {
                    let expected = (0..depth)
                        .map(|n| format!("key_{n:03}"))
                        .collect::<Vec<_>>();
                    let semantic = engine
                        .semantic(index, generation, &[1.0, 0.0, 0.0], None, depth)
                        .await
                        .unwrap();
                    assert_eq!(
                        semantic
                            .candidates
                            .iter()
                            .map(|c| c.document.key.clone())
                            .collect::<Vec<_>>(),
                        expected,
                        "semantic depth {depth}"
                    );
                    let mut q = query(engine, index, generation, "apple").await;
                    q.depth = depth;
                    let lexical = engine.query(index, generation, &q).await.unwrap();
                    assert_eq!(
                        lexical
                            .candidates
                            .iter()
                            .map(|c| c.document.key.clone())
                            .collect::<Vec<_>>(),
                        expected,
                        "sparse depth {depth}"
                    );
                }
                let signed = docs
                    .into_iter()
                    .enumerate()
                    .map(|(n, mut doc)| {
                        doc.document.source_version = 2;
                        doc.content_hash = doc.document.content_hash("signed");
                        doc.sparse = Some(if n < 10 {
                            SparseEmbedding {
                                indices: sparse.indices.clone(),
                                values: vec![-1.0],
                            }
                        } else {
                            SparseEmbedding {
                                indices: vec![],
                                values: vec![],
                            }
                        });
                        doc
                    })
                    .collect::<Vec<_>>();
                engine.index(index, generation, &signed).await.unwrap();
                let mut q = query(engine, index, generation, "apple").await;
                q.depth = 1;
                assert_eq!(
                    engine
                        .query(index, generation, &q)
                        .await
                        .unwrap()
                        .candidates[0]
                        .document
                        .key,
                    "key_010"
                );
            })
        })
        .await;
    })
    .await;
}

#[tokio::test]
async fn shared_generation_metadata_does_not_require_qdrant() -> Result<(), FrameworkError> {
    use sea_orm::ConnectionTrait;
    let db = database().await;
    let index = descriptor();
    let database_engine = suprnova::search::DatabaseSearchEngine::new();
    let qdrant = QdrantSearchEngine::from_url("http://127.0.0.1:1", None, 1)?;
    let first = GenerationId::new("g1")?;
    let second = GenerationId::new("g2")?;
    assert_eq!(qdrant.current_generation(index.name).await?, None);
    database_engine
        .create_generation(&index, &index.index_settings(), &first)
        .await?;
    database_engine
        .create_generation(&index, &index.index_settings(), &second)
        .await?;
    assert_eq!(qdrant.current_generation(index.name).await?, None);
    database_engine
        .switch_generation(&index, None, first.clone())
        .await?;
    assert_eq!(
        qdrant.current_generation(index.name).await?,
        Some(first.clone())
    );
    database_engine
        .switch_generation(&index, Some(first.clone()), second.clone())
        .await?;
    assert_eq!(qdrant.current_generation(index.name).await?, Some(second));
    database_engine.drop_generation(&index, &first).await?;
    assert!(
        !sea_orm_migration::SchemaManager::new(db.conn())
            .has_table("search_index_states")
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?
    );
    db.conn().execute_raw(sea_orm::Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "UPDATE search_generations SET state='active' WHERE index_name=? AND generation='__control'",
        [index.name.into()],
    )).await.map_err(|e| FrameworkError::database(e.to_string()))?;
    assert!(qdrant.current_generation(index.name).await.is_err());
    assert!(
        database_engine
            .current_generation(index.name)
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn bm25_ranks_coverage_and_term_frequency_qdrant() {
    fixture(|engine, index, generation| {
        Box::pin(async move {
            engine
                .create_generation(index, &index.index_settings(), generation)
                .await
                .unwrap();
            let docs = [
                ("both", "apple orchard filler filler"),
                ("repeated", "apple apple filler filler"),
                ("once", "apple filler filler filler"),
                ("unrelated", "banana filler filler filler"),
            ]
            .into_iter()
            .map(|(key, text)| document(index, key, text, 1))
            .collect::<Vec<_>>();
            engine.index(index, generation, &docs).await.unwrap();
            let q = query(engine, index, generation, "apple orchard").await;
            let hits = engine.query(index, generation, &q).await.unwrap();
            assert_eq!(hits.total.value(), 3);
            assert_eq!(
                hits.candidates
                    .iter()
                    .map(|c| c.document.key.as_str())
                    .collect::<Vec<_>>(),
                ["both", "repeated", "once"]
            );
            assert!(hits.candidates[0].score > hits.candidates[1].score);
            assert!(hits.candidates[1].score > hits.candidates[2].score);
            let mut shallow = q.clone();
            shallow.depth = 1;
            assert_eq!(
                engine
                    .query(index, generation, &shallow)
                    .await
                    .unwrap()
                    .candidates[0]
                    .document
                    .key,
                "both"
            );
        })
    })
    .await;
}

#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn native_bm25_attribute_weights_qdrant() {
    fixture(|engine, index, _| {
        Box::pin(async move {
            let mut index = IndexDescriptor {
                name: index.name,
                ..descriptor()
            };
            index
                .attributes
                .iter_mut()
                .find(|a| a.name == "category")
                .unwrap()
                .searchable = Some(SearchableDecl {
                weight: 1.0,
                order: 1,
            });
            for (slug, title_weight, body_weight, first) in
                [("title", 5.0, 1.0, "title"), ("body", 1.0, 5.0, "body")]
            {
                let generation = GenerationId::new(slug).unwrap();
                let mut settings = index.index_settings();
                settings.index_time.searchable_attributes[0].weight = title_weight;
                settings.index_time.searchable_attributes[1].weight = body_weight;
                engine
                    .create_generation(&index, &settings, &generation)
                    .await
                    .unwrap();
                let mut docs = vec![
                    document(&index, "title", "apple filler", 1),
                    document(&index, "body", "other filler", 2),
                ];
                for (doc, body) in docs.iter_mut().zip(["other filler", "apple filler"]) {
                    doc.document
                        .attributes
                        .insert("category".into(), AttributeValue::Text(body.into()));
                }
                engine.index(&index, &generation, &docs).await.unwrap();
                let mut q = query(engine, &index, &generation, "apple").await;
                let hits = engine.query(&index, &generation, &q).await.unwrap();
                assert_eq!(hits.candidates.len(), 2);
                assert_eq!(hits.candidates[0].document.key, first);
                assert!(hits.candidates[0].score > hits.candidates[1].score);
                q.depth = 1;
                assert_eq!(
                    engine
                        .query(&index, &generation, &q)
                        .await
                        .unwrap()
                        .candidates[0]
                        .document
                        .key,
                    first
                );
            }
        })
    })
    .await;
}

#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn native_bm25_text_schema_qdrant() {
    fixture(|engine, index, generation| {
        Box::pin(async move {
            let mut index = IndexDescriptor {
                name: index.name,
                ..descriptor()
            };
            index
                .attributes
                .iter_mut()
                .find(|a| a.name == "category")
                .unwrap()
                .searchable = Some(SearchableDecl {
                weight: 2.0,
                order: 1,
            });
            for (slug, model) in [
                (generation.clone(), None),
                (GenerationId::new("explicit").unwrap(), Some("bm25".into())),
            ] {
                let mut settings = index.index_settings();
                settings.index_time.sparse_model = model;
                engine
                    .create_generation(&index, &settings, &slug)
                    .await
                    .unwrap();
                engine
                    .index(&index, &slug, &[document(&index, "a", "apple", 1)])
                    .await
                    .unwrap();
                let collection = format!("search_{}_{}", index.name, slug.as_str());
                let info = engine
                    .client()
                    .unwrap()
                    .collection_info(&collection)
                    .await
                    .unwrap()
                    .result
                    .unwrap();
                let sparse = info
                    .config
                    .unwrap()
                    .params
                    .unwrap()
                    .sparse_vectors_config
                    .unwrap()
                    .map;
                assert_eq!(sparse.len(), 2);
                assert!(!sparse.contains_key("sparse"));
                for name in ["bm25_text", "bm25_category"] {
                    assert_eq!(
                        sparse[name].modifier,
                        Some(qdrant_client::qdrant::Modifier::Idf as i32)
                    );
                }
                let points = engine
                    .client()
                    .unwrap()
                    .scroll(
                        qdrant_client::qdrant::ScrollPointsBuilder::new(&collection)
                            .with_payload(true)
                            .with_vectors(true),
                    )
                    .await
                    .unwrap()
                    .result;
                assert_eq!(points.len(), 1);
                assert!(!points[0].payload.contains_key("__sparse_indices"));
                assert!(!info.payload_schema.contains_key("__sparse_indices"));
            }
        })
    })
    .await;
}

#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn native_bm25_stemming_qdrant() {
    fixture(|engine, index, generation| {
        Box::pin(async move {
            let mut settings = index.index_settings();
            settings.index_time.language = LanguageConfig::Qdrant {
                tokenizer: "word".into(),
                stemmer: Some("english".into()),
                stop_words: vec![],
                folding: true,
            };
            engine
                .create_generation(index, &settings, generation)
                .await
                .unwrap();
            engine
                .index(index, generation, &[document(index, "a", "runs", 1)])
                .await
                .unwrap();
            let q = query(engine, index, generation, "running").await;
            let hits = engine.query(index, generation, &q).await.unwrap();
            assert_eq!(hits.candidates.len(), 1);
            assert!(
                hits.candidates[0].score > 0.0,
                "stemmed text match must receive native BM25 relevance"
            );
        })
    })
    .await;
}

#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn native_bm25_cyrillic_qdrant() {
    fixture(|engine, index, generation| {
        Box::pin(async move {
            engine
                .create_generation(index, &index.index_settings(), generation)
                .await
                .unwrap();
            engine
                .index(
                    index,
                    generation,
                    &[document(index, "a", "космос планета", 1)],
                )
                .await
                .unwrap();
            let mut q = query(engine, index, generation, "космос").await;
            // Engine callers supply text in the engine's script. The builder also
            // needs to preserve it when it expands queries for Qdrant.
            q.expanded.terms[0].original = "космос".into();
            q.expanded.terms[0].alternatives = vec!["космос".into()];
            let hits = engine.query(index, generation, &q).await.unwrap();
            assert_eq!(hits.candidates.len(), 1);
            assert!(
                hits.candidates[0].score > 0.0,
                "Cyrillic text must retain its script for ranking"
            );
            assert!(
                engine
                    .vocabulary(index, generation)
                    .await
                    .unwrap()
                    .words
                    .contains_key("космос"),
                "prefix and typo words must preserve Cyrillic too"
            );
        })
    })
    .await;
}

#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn native_bm25_language_neutral_qdrant() {
    fixture(|engine, index, generation| {
        Box::pin(async move {
            engine
                .create_generation(index, &index.index_settings(), generation)
                .await
                .unwrap();
            engine
                .index(index, generation, &[document(index, "a", "the", 1)])
                .await
                .unwrap();
            let q = query(engine, index, generation, "the").await;
            let hits = engine.query(index, generation, &q).await.unwrap();
            assert_eq!(hits.candidates.len(), 1);
            assert!(
                hits.candidates[0].score > 0.0,
                "default BM25 must not remove English stop words"
            );
            use qdrant_client::qdrant as qb;
            let native = engine
                .client()
                .unwrap()
                .query(
                    qb::QueryPointsBuilder::new(format!("search_{}_g1", index.name))
                        .query(qb::Query::from(qb::VectorInput::from(qb::Document {
                            text: "the".into(),
                            model: "qdrant/bm25".into(),
                            options: std::collections::HashMap::from([
                                ("tokenizer".into(), qb::Value::from("word")),
                                (
                                    "stemmer".into(),
                                    qb::Value::from(serde_json::json!({"type":"none"})),
                                ),
                                ("stopwords".into(), qb::Value::from(serde_json::json!({}))),
                                ("ascii_folding".into(), qb::Value::from(true)),
                                (
                                    "avg_len".into(),
                                    qb::Value::from(
                                        suprnova::SearchConfig::current()
                                            .unwrap()
                                            .bm25_average_length
                                            as i64,
                                    ),
                                ),
                            ]),
                        })))
                        .using("bm25_text")
                        .limit(10),
                )
                .await
                .unwrap();
            assert_eq!(native.result.len(), 1);
            assert_eq!(hits.candidates[0].score, f64::from(native.result[0].score));
        })
    })
    .await;
}

#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn native_bm25_sum_and_zero_depth_qdrant() {
    fixture(|engine, index, generation| {
        Box::pin(async move {
            let mut index = IndexDescriptor {
                name: index.name,
                ..descriptor()
            };
            index
                .attributes
                .iter_mut()
                .find(|a| a.name == "category")
                .unwrap()
                .searchable = Some(SearchableDecl {
                weight: 1.0,
                order: 1,
            });
            engine
                .create_generation(&index, &index.index_settings(), generation)
                .await
                .unwrap();
            let mut docs = Vec::new();
            for n in 0..40 {
                let key = format!("zero_{n:02}");
                let mut doc = document(&index, &key, "applesauce", 1);
                doc.document.source_version = 100 + n;
                doc.document
                    .attributes
                    .insert("category".into(), AttributeValue::Text("filler".into()));
                docs.push(doc);
            }
            // Two stronger hits per attribute keep the summed winner outside
            // each attribute's depth-one lookahead.
            for (key, title, body) in [
                (
                    "title_1",
                    "apple apple apple apple",
                    "filler filler filler filler",
                ),
                (
                    "title_2",
                    "apple apple apple apple",
                    "filler filler filler filler",
                ),
                (
                    "body_1",
                    "filler filler filler filler",
                    "apple apple apple apple",
                ),
                (
                    "body_2",
                    "filler filler filler filler",
                    "apple apple apple apple",
                ),
                (
                    "sum",
                    "apple filler filler filler",
                    "apple filler filler filler",
                ),
            ] {
                let mut doc = document(&index, key, title, 1);
                doc.document
                    .attributes
                    .insert("category".into(), AttributeValue::Text(body.into()));
                docs.push(doc);
            }
            engine.index(&index, generation, &docs).await.unwrap();
            let mut q = query(engine, &index, generation, "apple").await;
            let full = engine.query(&index, generation, &q).await.unwrap();
            assert_eq!(full.candidates.len(), 45);
            assert_eq!(full.candidates[0].document.key, "sum");
            assert!(full.candidates[..5].iter().all(|c| c.score > 0.0));
            assert!(full.candidates[5..].iter().all(|c| c.score == 0.0));
            for depth in [1, 2, 3, 4, 10, 25] {
                q.depth = depth;
                let page = engine.query(&index, generation, &q).await.unwrap();
                assert_eq!(
                    page.candidates
                        .iter()
                        .map(|c| (&c.document.key, c.score))
                        .collect::<Vec<_>>(),
                    full.candidates[..depth]
                        .iter()
                        .map(|c| (&c.document.key, c.score))
                        .collect::<Vec<_>>(),
                    "depth {depth}"
                );
            }
        })
    })
    .await;
}

#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn native_bm25_language_options_qdrant() {
    use qdrant_client::qdrant as qb;
    suprnova::SearchConfig::scope(
        suprnova::SearchConfig { bm25_average_length: 64, ..Default::default() },
        fixture(|engine, index, _| {
        Box::pin(async move {
            let configs = [
                (LanguageConfig::Server, serde_json::json!({"tokenizer":"word", "stemmer":{"type":"none"}, "stopwords":{}, "ascii_folding":true})),
                (LanguageConfig::Qdrant { tokenizer:"word".into(), stemmer:None, stop_words:vec![], folding:true },
                    serde_json::json!({"tokenizer":"word", "stemmer":{"type":"none"}, "stopwords":{}, "ascii_folding":true})),
                (LanguageConfig::Qdrant { tokenizer:"whitespace".into(), stemmer:Some("english".into()), stop_words:vec!["filler".into()], folding:false },
                    serde_json::json!({"tokenizer":"whitespace", "stemmer":{"type":"snowball", "language":"english"}, "stopwords":{"custom":["filler"]}, "ascii_folding":false})),
                (LanguageConfig::Qdrant { tokenizer:"multilingual".into(), stemmer:None, stop_words:vec![], folding:false },
                    serde_json::json!({"tokenizer":"multilingual", "stemmer":{"type":"none"}, "stopwords":{}, "ascii_folding":false})),
                (LanguageConfig::Qdrant { tokenizer:"prefix".into(), stemmer:None, stop_words:vec![], folding:true },
                    serde_json::json!({"tokenizer":"prefix", "stemmer":{"type":"none"}, "stopwords":{}, "ascii_folding":true})),
            ];
            for (n, (language, mut options)) in configs.into_iter().enumerate() {
                let generation = GenerationId::new(format!("options{n}")).unwrap();
                let mut settings = index.index_settings();
                settings.index_time.language = language;
                engine.create_generation(index, &settings, &generation).await.unwrap();
                engine.index(index, &generation, &[document(index, "a", "runs café filler the", 1)]).await.unwrap();
                let collection = format!("search_{}_{}", index.name, generation.as_str());
                options["avg_len"] = serde_json::json!(suprnova::SearchConfig::current().unwrap().bm25_average_length);
                let options = options.as_object().unwrap().iter().map(|(k,v)| (k.clone(), qb::Value::from(v.clone()))).collect();
                let input = qb::Document { text:"runs café filler the".into(), model:"qdrant/bm25".into(), options };
                engine.client().unwrap().upsert_points(qb::UpsertPointsBuilder::new(&collection,
                    vec![qb::PointStruct::new(42_u64, qb::NamedVectors { vectors:std::collections::HashMap::from([("bm25_text".into(), input.clone().into())]) },
                        qdrant_client::Payload::default())]).wait(true)).await.unwrap();
                let points = engine.client().unwrap().scroll(qb::ScrollPointsBuilder::new(&collection).with_payload(true).with_vectors(true)).await.unwrap().result;
                assert_eq!(points.len(), 2);
                let vectors = points.iter().map(|p| {
                    let qb::vectors_output::VectorsOptions::Vectors(v) = p.vectors.as_ref().unwrap().vectors_options.as_ref().unwrap() else { panic!("named vectors") };
                    v.vectors["bm25_text"].clone()
                }).collect::<Vec<_>>();
                assert_eq!(vectors[0], vectors[1], "document options {:?}", settings.index_time.language);
                let q = query(engine, index, &generation, "running the cafe").await;
                let actual = engine.query(index, &generation, &q).await.unwrap();
                let text = q.expanded.terms.iter().flat_map(|t| t.alternatives.iter().chain(std::iter::once(&t.original)))
                    .chain(q.expanded.phrases.iter().flatten()).cloned().collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>().join(" ");
                let native = engine.client().unwrap().query(qb::QueryPointsBuilder::new(&collection)
                    .query(qb::Query::from(qb::VectorInput::from(qb::Document { text, ..input })))
                    .using("bm25_text").filter(engine.render_filter(index, &generation, &q).await.unwrap()).limit(10)).await.unwrap();
                assert_eq!(actual.candidates.len(), 1);
                assert_eq!(native.result.len(), 1);
                assert!(actual.candidates[0].score > 0.0);
                assert_eq!(actual.candidates[0].score, f64::from(native.result[0].score), "query options {:?}", settings.index_time.language);
            }
            for language in [LanguageConfig::Postgres { configuration:"english".into() }, LanguageConfig::Sqlite { tokenizer:"unicode61".into() }] {
                let mut settings = index.index_settings();
                settings.index_time.language = language;
                assert!(engine.create_generation(index, &settings, &GenerationId::new("invalid").unwrap()).await.is_err());
            }
        })
    })).await.unwrap();
}

#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn native_bm25_builder_cyrillic_qdrant() {
    use super::derive::{SearchNote, database, note};
    use futures::FutureExt;
    use suprnova::{SearchConfig, SearchDriver, Searchable, queue::Job};
    let db = database().await;
    for migration in suprnova::search::migrations() {
        migration
            .up(&sea_orm_migration::SchemaManager::new(db.conn()))
            .await
            .unwrap();
    }
    let engine = Arc::new(crate::qdrant::engine());
    let descriptor = SearchNote::descriptor();
    let generation =
        GenerationId::new(format!("native_{}", uuid::Uuid::new_v4().simple())).unwrap();
    let outcome = std::panic::AssertUnwindSafe(SearchConfig::scope(
        SearchConfig {
            driver: SearchDriver::Collection,
            queue: false,
            bm25_average_length: 128,
            ..Default::default()
        },
        SearchRegistry::isolated(async {
            let model = note("космос планета café", true, 0).await;
            SearchRegistry::register_engine("qdrant", engine.clone()).unwrap();
            SearchRegistry::set_index_driver(descriptor.name, "qdrant").unwrap();
            let settings = descriptor.index_settings();
            engine
                .create_generation(descriptor, &settings, &generation)
                .await
                .unwrap();
            engine
                .switch_generation(descriptor, None, generation.clone())
                .await
                .unwrap();
            let doc = model.to_search_document().unwrap();
            suprnova::SearchIndexJob {
                index: doc.index,
                key: doc.key,
                source_version: doc.source_version,
                op: suprnova::IndexOperation::Index,
                generation: None,
            }
            .handle()
            .await
            .unwrap();
            for text in ["космос", "косм", "космсо", "\"космос планета\"", "cafe"]
            {
                let hits = SearchNote::search(text)
                    .highlight(["title"])
                    .get()
                    .await
                    .unwrap();
                assert_eq!(hits.hits.len(), 1, "{text}");
                if text != "косм" {
                    assert!(hits.hits[0].engine_score > 0.0, "{text}");
                }
                assert!(hits.hits[0].formatted["title"].contains("<em>"), "{text}");
            }
            assert_eq!(
                SearchNote::search("планета -космос")
                    .get()
                    .await
                    .unwrap()
                    .hits
                    .len(),
                0
            );
        }),
    ))
    .catch_unwind()
    .await;
    engine
        .client()
        .unwrap()
        .delete_collection(format!(
            "search_{}_{}",
            descriptor.name,
            generation.as_str()
        ))
        .await
        .unwrap();
    match outcome {
        Ok(result) => result.unwrap(),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn native_bm25_threshold_bound_qdrant() {
    fixture(|engine, index, generation| {
        Box::pin(async move {
            let mut index = IndexDescriptor {
                name: index.name,
                ..descriptor()
            };
            index
                .attributes
                .iter_mut()
                .find(|a| a.name == "category")
                .unwrap()
                .searchable = Some(SearchableDecl {
                weight: 2.0,
                order: 1,
            });
            let mut settings = index.index_settings();
            settings.index_time.dimensions = Some(3);
            engine
                .create_generation(&index, &settings, generation)
                .await
                .unwrap();
            let mut docs = Vec::new();
            for n in 0..400 {
                let key = format!("doc_{n:03}");
                let text = if n < 4 {
                    format!("common {}", "common ".repeat(32 - n * 6))
                } else {
                    format!("common {}", "filler ".repeat(128))
                };
                let mut doc = document(&index, &key, &text, 1);
                doc.document
                    .attributes
                    .insert("category".into(), AttributeValue::Text(text));
                doc.dense = Some(vec![1.0, 0.0, 0.0]);
                docs.push(doc);
            }
            engine.index(&index, generation, &docs).await.unwrap();
            let mut q = query(engine, &index, generation, "common").await;
            q.depth = 3;
            suprnova::search::testing::take_qdrant_attribute_limit(engine).unwrap();
            let actual = engine.query(&index, generation, &q).await.unwrap();
            assert_eq!(actual.total.value(), 400);
            assert_eq!(
                actual
                    .candidates
                    .iter()
                    .map(|c| c.document.key.as_str())
                    .collect::<Vec<_>>(),
                ["doc_000", "doc_001", "doc_002"]
            );
            let limit = suprnova::search::testing::take_qdrant_attribute_limit(engine).unwrap();
            eprintln!("lexical largest attribute limit: {limit}");
            let lexical_limit = limit;
            // At ratio zero hybrid uses the same exact lexical order.
            let lexical = engine
                .hybrid(&index, generation, &q, &[1.0, 0.0, 0.0], 0.0)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                lexical
                    .candidates
                    .iter()
                    .map(|c| (&c.document.key, c.score))
                    .collect::<Vec<_>>(),
                actual
                    .candidates
                    .iter()
                    .map(|c| (&c.document.key, c.score))
                    .collect::<Vec<_>>()
            );
            let hybrid = engine
                .hybrid(&index, generation, &q, &[1.0, 0.0, 0.0], 0.01)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                hybrid.candidates[..3]
                    .iter()
                    .map(|c| c.document.key.as_str())
                    .collect::<Vec<_>>(),
                ["doc_000", "doc_001", "doc_002"]
            );
            let limit = suprnova::search::testing::take_qdrant_attribute_limit(engine).unwrap();
            eprintln!("hybrid largest attribute limit: {limit}");
            assert!(
                limit < 100,
                "hybrid attribute limit {limit} should stay well below 400"
            );
            assert!(
                lexical_limit < 100,
                "attribute limit {lexical_limit} should stay well below 400"
            );
        })
    })
    .await;
}

#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn missing_searchable_attribute_writes_nothing_qdrant() {
    fixture(|engine, index, generation| {
        Box::pin(async move {
            engine
                .create_generation(index, &index.index_settings(), generation)
                .await
                .unwrap();
            let valid = document(index, "valid", "apple", 1);
            let mut missing = document(index, "missing", "apple", 1);
            missing.document.attributes.remove("text");
            let docs = [valid, missing];
            let error = engine.index(index, generation, &docs).await.unwrap_err();
            assert!(
                error.to_string().contains("text") && error.to_string().contains("missing"),
                "{error}"
            );
            let count = engine
                .client()
                .unwrap()
                .count(
                    qdrant_client::qdrant::CountPointsBuilder::new(format!(
                        "search_{}_g1",
                        index.name
                    ))
                    .exact(true),
                )
                .await
                .unwrap()
                .result
                .unwrap()
                .count;
            assert_eq!(
                count, 0,
                "the valid document preceding the invalid one must not be written"
            );
            assert!(
                engine
                    .stored_hashes(index, generation, &docs)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert!(
                engine
                    .vocabulary(index, generation)
                    .await
                    .unwrap()
                    .words
                    .is_empty()
            );
        })
    })
    .await;
}

#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn native_bm25_threshold_fractional_ties_qdrant() {
    fixture(|engine, index, _| {
        Box::pin(async move {
            for (n, weights) in [
                [0.1, 0.2, 0.3],
                [0.3, 0.7, 1.1],
                [1.1, 1.3, 1.7],
                [0.9, 0.1, 0.9],
            ]
            .into_iter()
            .enumerate()
            {
                let mut index = IndexDescriptor {
                    name: index.name,
                    ..descriptor()
                };
                for (order, (name, weight)) in ["text", "category", "tags"]
                    .into_iter()
                    .zip(weights)
                    .enumerate()
                {
                    index
                        .attributes
                        .iter_mut()
                        .find(|a| a.name == name)
                        .unwrap()
                        .searchable = Some(SearchableDecl { weight, order });
                }
                let generation = GenerationId::new(format!("ties{n}")).unwrap();
                engine
                    .create_generation(&index, &index.index_settings(), &generation)
                    .await
                    .unwrap();
                let docs = (0..30)
                    .map(|n| {
                        let mut doc = document(&index, &format!("doc_{n:03}"), "common", 1);
                        doc.document
                            .attributes
                            .insert("category".into(), AttributeValue::Text("common".into()));
                        doc.document.attributes.insert(
                            "tags".into(),
                            AttributeValue::List(vec![AttributeValue::Text("common".into())]),
                        );
                        doc
                    })
                    .collect::<Vec<_>>();
                engine.index(&index, &generation, &docs).await.unwrap();
                let mut q = query(engine, &index, &generation, "common").await;
                q.depth = 3;
                let result = engine.query(&index, &generation, &q).await.unwrap();
                assert_eq!(
                    result
                        .candidates
                        .iter()
                        .map(|c| c.document.key.as_str())
                        .collect::<Vec<_>>(),
                    ["doc_000", "doc_001", "doc_002"],
                    "weights {weights:?}"
                );
                assert!(
                    result
                        .candidates
                        .iter()
                        .all(|c| c.score == result.candidates[0].score)
                );
            }
        })
    })
    .await;
}

#[tokio::test]
#[ignore = "requires Qdrant at QDRANT_TEST_URL"]
async fn native_bm25_threshold_rounded_zero_boundary_qdrant() {
    fixture(|engine, index, generation| {
        Box::pin(async move {
            let mut index = IndexDescriptor {
                name: index.name,
                ..descriptor()
            };
            for (order, name) in ["text", "category"].into_iter().enumerate() {
                index
                    .attributes
                    .iter_mut()
                    .find(|a| a.name == name)
                    .unwrap()
                    .searchable = Some(SearchableDecl {
                    weight: f32::from_bits(1),
                    order,
                });
            }
            engine
                .create_generation(&index, &index.index_settings(), generation)
                .await
                .unwrap();
            let mut docs = (0..30)
                .map(|n| {
                    let mut doc = document(&index, &format!("doc_{n:03}"), "common", 1);
                    doc.document
                        .attributes
                        .insert("category".into(), AttributeValue::Text("common".into()));
                    doc
                })
                .collect::<Vec<_>>();
            let mut zero = document(&index, "a_zero", "commonplace", 1);
            zero.document
                .attributes
                .insert("category".into(), AttributeValue::Text("filler".into()));
            docs.push(zero);
            engine.index(&index, generation, &docs).await.unwrap();
            let mut q = query(engine, &index, generation, "common").await;
            q.depth = 3;
            let result = engine.query(&index, generation, &q).await.unwrap();
            assert!(result.candidates.iter().all(|c| c.score == 0.0));
            assert_eq!(
                result
                    .candidates
                    .iter()
                    .map(|c| c.document.key.as_str())
                    .collect::<Vec<_>>(),
                ["a_zero", "doc_000", "doc_001"]
            );
        })
    })
    .await;
}
