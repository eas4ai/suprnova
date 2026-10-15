use std::sync::Arc;
use suprnova::search::{
    engine::{SearchEngine, collection::CollectionEngine},
    testing::conformance,
};
#[tokio::test]
async fn collection_obeys_the_shared_driver_contract() -> Result<(), suprnova::FrameworkError> {
    let engine: Arc<dyn SearchEngine> = Arc::new(CollectionEngine::new());
    conformance::run(engine.clone(), conformance::Options::default()).await?;
    conformance::run(engine, conformance::Options::default()).await
}
#[tokio::test]
async fn null_driver_reports_no_retrieval_capabilities() -> Result<(), suprnova::FrameworkError> {
    let e = suprnova::search::engine::null::NullEngine::new();
    assert!(!e.capabilities().semantic);
    assert!(!e.capabilities().facets);
    Ok(())
}

#[tokio::test]
async fn typo_expansion_respects_attributes_that_disable_corrections()
-> Result<(), suprnova::FrameworkError> {
    use std::collections::BTreeMap;
    use suprnova::search::{
        AttributeDecl, AttributeType, AttributeValue, EngineQuery, ExpandedQuery, ExpandedTerm,
        GenerationId, IndexDescriptor, IndexSettings, IndexedDocument, SearchDocument,
        SearchableDecl,
    };
    fn settings() -> IndexSettings {
        let mut s = IndexSettings::default();
        s.query
            .typo_tolerance
            .disable_on_attributes
            .insert("exact".into());
        s
    }
    let descriptor = IndexDescriptor {
        name: "attribute_typos",
        model_table: "things",
        key_column: "id",
        attributes: ["exact", "fuzzy"]
            .into_iter()
            .enumerate()
            .map(|(order, name)| AttributeDecl {
                name: name.into(),
                ty: AttributeType::Text,
                searchable: Some(SearchableDecl { weight: 1.0, order }),
                filterable: false,
                sortable: false,
                facet: false,
            })
            .collect(),
        embed_source: None,
        soft_delete: false,
        search_touches: vec![],
        settings,
        scope: None,
    };
    let engine = CollectionEngine::new();
    let generation = GenerationId::default();
    engine
        .create_generation(&descriptor, &descriptor.index_settings(), &generation)
        .await?;
    let documents = [("a", "exact"), ("b", "fuzzy")]
        .into_iter()
        .map(|(key, field)| {
            let document = SearchDocument {
                index: descriptor.name.into(),
                key: key.into(),
                source_version: 1,
                attributes: BTreeMap::from([
                    (
                        "exact".into(),
                        if field == "exact" {
                            AttributeValue::Text("apple".into())
                        } else {
                            AttributeValue::Null
                        },
                    ),
                    (
                        "fuzzy".into(),
                        if field == "fuzzy" {
                            AttributeValue::Text("apple".into())
                        } else {
                            AttributeValue::Null
                        },
                    ),
                ]),
                embed_text: None,
                title: None,
            };
            IndexedDocument {
                content_hash: document.content_hash(""),
                document,
                dense: None,
                sparse: None,
            }
        })
        .collect::<Vec<_>>();
    engine.index(&descriptor, &generation, &documents).await?;
    let q = EngineQuery {
        expanded: ExpandedQuery {
            terms: vec![ExpandedTerm {
                original: "applf".into(),
                alternatives: vec!["apple".into()],
                prefix: false,
                required: false,
                typo_alternatives: vec!["apple".into()],
            }],
            phrases: vec![],
            negatives: vec![],
            ..ExpandedQuery::default()
        },
        filter: None,
        sort: vec![],
        depth: 20,
        highlights: false,
        crop: Default::default(),
    };
    let result = engine.query(&descriptor, &generation, &q).await?;
    assert_eq!(result.candidates.len(), 1);
    assert_eq!(result.candidates[0].document.key, "b");
    Ok(())
}

#[tokio::test]
async fn generation_and_descriptor_validation_cannot_be_bypassed()
-> Result<(), suprnova::FrameworkError> {
    use suprnova::search::{GenerationId, IndexDescriptor, IndexSettings};
    let descriptor = IndexDescriptor {
        name: "validation",
        model_table: "items",
        key_column: "id",
        attributes: vec![],
        embed_source: None,
        soft_delete: false,
        search_touches: vec![],
        settings: IndexSettings::default,
        scope: None,
    };
    let engine = CollectionEngine::new();
    let invalid = GenerationId("../../invalid".into());
    assert!(
        engine
            .create_generation(&descriptor, &IndexSettings::default(), &invalid)
            .await
            .is_err()
    );
    assert!(engine.read_settings(&descriptor, &invalid).await.is_err());
    let descriptor = IndexDescriptor {
        name: "invalid/name",
        ..descriptor
    };
    assert!(
        engine
            .create_generation(
                &descriptor,
                &IndexSettings::default(),
                &GenerationId::default()
            )
            .await
            .is_err()
    );
    Ok(())
}

fn typo_prefix_descriptor() -> suprnova::search::IndexDescriptor {
    use suprnova::search::{
        AttributeDecl, AttributeType, IndexDescriptor, IndexSettings, SearchableDecl,
    };
    IndexDescriptor {
        name: "typo_prefix",
        model_table: "items",
        key_column: "id",
        attributes: vec![AttributeDecl {
            name: "body".into(),
            ty: AttributeType::Text,
            searchable: Some(SearchableDecl {
                weight: 1.0,
                order: 0,
            }),
            filterable: false,
            sortable: false,
            facet: false,
        }],
        embed_source: None,
        soft_delete: false,
        search_touches: vec![],
        settings: IndexSettings::default,
        scope: None,
    }
}

#[tokio::test]
async fn typo_corrections_do_not_become_prefixes() -> Result<(), suprnova::FrameworkError> {
    use std::collections::BTreeMap;
    use suprnova::search::{
        AttributeValue, EngineQuery, GenerationId, IndexedDocument, QueryText, SearchDocument,
        expand,
    };
    let descriptor = typo_prefix_descriptor();
    let engine = CollectionEngine::new();
    let generation = GenerationId::default();
    let settings = descriptor.index_settings();
    engine
        .create_generation(&descriptor, &settings, &generation)
        .await?;
    let documents = [("a", "apple"), ("b", "applesauce")]
        .into_iter()
        .map(|(key, text)| {
            let document = SearchDocument {
                index: descriptor.name.into(),
                key: key.into(),
                source_version: 1,
                attributes: BTreeMap::from([("body".into(), AttributeValue::Text(text.into()))]),
                embed_text: None,
                title: None,
            };
            IndexedDocument {
                content_hash: document.content_hash(&settings.index_time.digest()),
                document,
                dense: None,
                sparse: None,
            }
        })
        .collect::<Vec<_>>();
    engine.index(&descriptor, &generation, &documents).await?;
    let vocabulary = engine.vocabulary(&descriptor, &generation).await?;
    let q = EngineQuery {
        expanded: expand(&QueryText::parse("applr"), &settings.query, &vocabulary, 64)?,
        filter: None,
        sort: vec![],
        depth: 20,
        highlights: false,
        crop: Default::default(),
    };
    let results = engine.query(&descriptor, &generation, &q).await?;
    assert_eq!(results.candidates.len(), 1);
    assert_eq!(results.candidates[0].document.key, "a");
    // A query warms the generation's field cache. Changed content and removals
    // must update both matching and the vocabulary used by the next snapshot.
    let mut updated = documents[1].clone();
    updated.document.source_version = 2;
    updated
        .document
        .attributes
        .insert("body".into(), AttributeValue::Text("apple apple".into()));
    updated.content_hash = updated.document.content_hash(&settings.index_time.digest());
    engine.index(&descriptor, &generation, &[updated]).await?;
    let results = engine.query(&descriptor, &generation, &q).await?;
    assert_eq!(results.candidates.len(), 2);
    assert_eq!(results.candidates[0].document.key, "b");
    engine
        .remove(&descriptor, &generation, &[("a".into(), 2)])
        .await?;
    let results = engine.query(&descriptor, &generation, &q).await?;
    assert_eq!(results.candidates.len(), 1);
    assert_eq!(results.candidates[0].document.key, "b");
    let vocabulary = engine.vocabulary(&descriptor, &generation).await?;
    assert_eq!(vocabulary.words.get("apple"), Some(&1));
    assert!(!vocabulary.words.contains_key("applesauce"));
    Ok(())
}

#[tokio::test]
async fn expired_tombstones_do_not_reject_a_new_removal() -> Result<(), suprnova::FrameworkError> {
    use suprnova::search::{GenerationId, IndexDescriptor, IndexSettings};
    let descriptor = IndexDescriptor {
        name: "expired_removal",
        model_table: "items",
        key_column: "id",
        attributes: vec![],
        embed_source: None,
        soft_delete: false,
        search_touches: vec![],
        settings: IndexSettings::default,
        scope: None,
    };
    let engine = CollectionEngine::with_tombstone_hours(0)?;
    let generation = GenerationId::default();
    engine
        .create_generation(&descriptor, &IndexSettings::default(), &generation)
        .await?;
    engine
        .remove(&descriptor, &generation, &[("a".into(), 5)])
        .await?;
    let outcome = engine
        .remove(&descriptor, &generation, &[("a".into(), 4)])
        .await?;
    assert_eq!(outcome.written, 1);
    assert_eq!(outcome.skipped_older, 0);
    Ok(())
}

#[tokio::test]
async fn word_disjunction_keeps_phrases_and_exclusions_required()
-> Result<(), suprnova::FrameworkError> {
    use std::collections::BTreeMap;
    use suprnova::search::{
        AttributeValue, EngineQuery, GenerationId, IndexedDocument, QueryText, SearchDocument,
        expand,
    };
    let descriptor = typo_prefix_descriptor();
    let engine = CollectionEngine::new();
    let generation = GenerationId::default();
    let settings = descriptor.index_settings();
    engine
        .create_generation(&descriptor, &settings, &generation)
        .await?;
    let documents = [
        ("both", "apple pie fresh baked"),
        ("one", "apple tart fresh baked"),
        ("phrase", "tart crust fresh baked"),
        ("reversed", "apple pie baked fresh"),
        ("negative", "apple pie fresh baked green"),
    ]
    .into_iter()
    .map(|(key, text)| {
        let document = SearchDocument {
            index: descriptor.name.into(),
            key: key.into(),
            source_version: 1,
            attributes: BTreeMap::from([("body".into(), AttributeValue::Text(text.into()))]),
            embed_text: None,
            title: None,
        };
        IndexedDocument {
            content_hash: document.content_hash(&settings.index_time.digest()),
            document,
            dense: None,
            sparse: None,
        }
    })
    .collect::<Vec<_>>();
    engine.index(&descriptor, &generation, &documents).await?;
    let vocabulary = engine.vocabulary(&descriptor, &generation).await?;
    for (text, expected) in [
        (r#"apple pie -green "fresh baked""#, vec!["both", "one"]),
        (r#""fresh baked" -green"#, vec!["both", "one", "phrase"]),
        ("-green", vec!["both", "one", "phrase", "reversed"]),
    ] {
        let q = EngineQuery {
            expanded: expand(&QueryText::parse(text), &settings.query, &vocabulary, 64)?,
            filter: None,
            sort: vec![],
            depth: 100,
            highlights: false,
            crop: Default::default(),
        };
        let result = engine.query(&descriptor, &generation, &q).await?;
        assert_eq!(
            result
                .candidates
                .iter()
                .map(|hit| hit.document.key.as_str())
                .collect::<Vec<_>>(),
            expected,
            "{text}"
        );
    }
    Ok(())
}
