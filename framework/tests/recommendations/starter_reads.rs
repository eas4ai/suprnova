use super::support::{Note, setup};
use std::{cell::RefCell, collections::BTreeMap, sync::LazyLock};
use suprnova::{
    Actor, AttributeValue, DB, EventType, FrameworkError, GenerateRun, IndexedDocument, Recommend,
    RecommendationEvent, RecommendationTarget, SearchDocument, SearchRegistry, Searchable,
    SearchableEntry, Viewer,
};

const INDEX: &str = "counted_recommendation_notes";
tokio::task_local! {
    static DOCUMENTS: BTreeMap<String, SearchDocument>;
    static LOADS: RefCell<Vec<String>>;
}

fn descriptor() -> &'static suprnova::IndexDescriptor {
    static DESCRIPTOR: LazyLock<suprnova::IndexDescriptor> = LazyLock::new(|| {
        let source = Note::descriptor();
        suprnova::IndexDescriptor {
            name: INDEX,
            model_table: source.model_table,
            key_column: source.key_column,
            attributes: source.attributes.clone(),
            embed_source: source.embed_source.clone(),
            soft_delete: source.soft_delete,
            search_touches: source.search_touches.clone(),
            settings: source.settings,
            scope: source.scope,
            scope_fields: source.scope_fields.clone(),
        }
    });
    &DESCRIPTOR
}

struct Counted;
suprnova::inventory::submit! {
    SearchableEntry {
        index: INDEX,
        descriptor,
        type_id: || std::any::TypeId::of::<Counted>(),
        model_name: || "CountedRecommendationNote",
        connection: || None,
        search: |_, _, _| Box::pin(async { Err(FrameworkError::internal("unused test search")) }),
        import_all: || Box::pin(async { Ok(()) }),
        load_document: |key| {
            let document = DOCUMENTS.try_with(|docs| docs.get(key).cloned()).ok().flatten();
            Box::pin(async move { Ok(document) })
        },
        load_documents: |keys| {
            let _ = LOADS.try_with(|loads| loads.borrow_mut().extend(keys.iter().cloned()));
            let documents = DOCUMENTS.try_with(|docs| keys.iter().filter_map(|key| docs.get(key).cloned()).collect()).unwrap_or_default();
            Box::pin(async move { Ok(documents) })
        },
        evaluate_scope: |viewer, document| {
            document.attributes.get("public") == Some(&AttributeValue::Bool(true))
                || document.attributes.get("owner") == Some(&AttributeValue::Text(viewer.id().unwrap_or("").into()))
        },
        document: |_| Err(FrameworkError::internal("unused test projection")),
        should_index: |_| Ok(true),
        keeps_version: false,
        keep: |_, _| {},
    }
}

fn documents() -> BTreeMap<String, SearchDocument> {
    (0..201)
        .map(|id| {
            let key = format!("{id:04}");
            (
                key.clone(),
                SearchDocument {
                    index: INDEX.into(),
                    key,
                    source_version: 1,
                    title: Some(format!("Note {id}")),
                    embed_text: None,
                    attributes: BTreeMap::from([
                        ("title".into(), AttributeValue::Text(format!("Note {id}"))),
                        ("category".into(), AttributeValue::Text("rust".into())),
                        ("public".into(), AttributeValue::Bool(true)),
                        ("owner".into(), AttributeValue::Text("alice".into())),
                        ("published_at".into(), AttributeValue::Integer(id)),
                    ]),
                },
            )
        })
        .collect()
}

async fn generate() {
    let engine = SearchRegistry::engine_for(INDEX).unwrap();
    let generation = SearchRegistry::generation_for(INDEX).await.unwrap();
    let indexed: Vec<_> = DOCUMENTS.with(|docs| {
        docs.values()
            .map(|document| IndexedDocument {
                document: document.clone(),
                content_hash: document.key.clone(),
                dense: None,
                sparse: None,
            })
            .collect()
    });
    engine
        .index(descriptor(), &generation, &indexed)
        .await
        .unwrap();
    let event = RecommendationEvent::new(
        EventType::Like,
        RecommendationTarget {
            index: INDEX.into(),
            key: "0000".into(),
        },
        Some(Actor::User("alice".into())),
        "home",
        "seed",
    )
    .await
    .unwrap();
    Recommend::record(event).await.unwrap();
    Recommend::generate(GenerateRun::default()).await.unwrap();
    LOADS.with(|loads| loads.borrow_mut().clear());
}

#[tokio::test]
async fn small_limit_hydrates_candidates_and_shared_targets_only() {
    setup(DOCUMENTS.scope(
        documents(),
        LOADS.scope(RefCell::new(Vec::new()), async {
            generate().await;
            let rows = Recommend::for_user("alice")
                .limit(1)
                .for_viewer(Viewer::Guest)
                .get()
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].target.key, "0001");
            assert_eq!(rows[0].score, 1400);
            assert_eq!(rows[0].explanation.shared_target_titles, ["Note 0"]);
            LOADS.with(|loads| {
                let loads = loads.borrow();
                assert!(
                    loads.len() < 32,
                    "hydrated {} documents for one result",
                    loads.len()
                );
                assert!(loads.contains(&"0000".into()));
                assert!(loads.contains(&"0001".into()));
                assert!(loads.iter().all(|key| key.as_str() <= "0030"));
            });
            let all = Recommend::for_user("alice")
                .limit(200)
                .for_viewer(Viewer::Guest)
                .get()
                .await
                .unwrap();
            assert_eq!(
                all.iter()
                    .map(|row| row.target.key.clone())
                    .collect::<Vec<_>>(),
                (1..201).map(|id| format!("{id:04}")).collect::<Vec<_>>()
            );
            assert!(
                all.iter()
                    .all(|row| row.score == 1400 && row.explanation.fallback_tier == "affinity")
            );
        }),
    ))
    .await;
}

#[tokio::test]
async fn ranked_pages_refill_after_live_scope_changes_and_search_removal() {
    let mut docs = documents();
    setup(DOCUMENTS.scope(
        docs.clone(),
        LOADS.scope(RefCell::new(Vec::new()), async move {
            generate().await;
            for id in 1..=30 {
                docs.get_mut(&format!("{id:04}"))
                    .unwrap()
                    .attributes
                    .insert("public".into(), AttributeValue::Bool(false));
            }
            let engine = SearchRegistry::engine_for(INDEX).unwrap();
            let generation = SearchRegistry::generation_for(INDEX).await.unwrap();
            engine
                .remove(descriptor(), &generation, &[("0031".into(), i64::MAX)])
                .await
                .unwrap();
            DOCUMENTS
                .scope(docs, async {
                    let rows = Recommend::for_user("alice")
                        .limit(2)
                        .for_viewer(Viewer::Guest)
                        .get()
                        .await
                        .unwrap();
                    assert_eq!(
                        rows.iter()
                            .map(|row| row.target.key.as_str())
                            .collect::<Vec<_>>(),
                        ["0032", "0033"]
                    );
                    assert_eq!(rows.iter().map(|row| row.rank).collect::<Vec<_>>(), [1, 2]);
                    assert!(rows.iter().all(|row| row.run_id == rows[0].run_id));
                })
                .await;
            assert_eq!(
                DB::scalar::<i64>(
                    "SELECT COUNT(*) FROM starter_recommendations WHERE actor_id = ?",
                    vec!["alice".into()]
                )
                .await
                .unwrap(),
                200
            );
        }),
    ))
    .await;
}
