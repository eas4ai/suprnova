//! Shared driver checks exercise behavior rather than implementation-specific ranking.
/// Conformance is reusable by external drivers without depending on a test runner.
pub mod conformance {
    use crate::{
        FrameworkError,
        search::{
            document::{AttributeType, AttributeValue, GeoPoint, SearchDocument},
            engine::{GenerationId, IndexedDocument, SearchEngine},
            expansion::{ExpandedQuery, expand},
            query::{EngineQuery, Filter, QueryText, Sort, SortDirection},
            registry::{AttributeDecl, IndexDescriptor, SearchableDecl},
            settings::{FacetOrder, IndexSettings},
        },
    };
    use std::{collections::BTreeMap, sync::Arc};
    /// Drivers may disable unsupported optional capabilities while mandatory checks remain.
    #[derive(Debug, Clone)]
    pub struct Options {
        /// An isolated name prevents conformance from writing application indexes.
        pub index_name: &'static str,
        /// Native dense retrieval can be tested without an external embedding provider.
        pub test_semantic: bool,
        /// Geo checks are optional when the engine reports no geo support.
        pub test_geo: bool,
    }
    impl Default for Options {
        fn default() -> Self {
            Self {
                index_name: "search_conformance",
                test_semantic: true,
                test_geo: true,
            }
        }
    }
    fn check(condition: bool, behavior: &str) -> Result<(), FrameworkError> {
        if condition {
            Ok(())
        } else {
            Err(FrameworkError::param(format!(
                "search engine conformance: {behavior}"
            )))
        }
    }
    fn settings() -> IndexSettings {
        let mut settings = IndexSettings::default();
        settings.index_time.dimensions = Some(3);
        settings
    }
    fn descriptor(name: &'static str) -> IndexDescriptor {
        IndexDescriptor {
            name,
            model_table: "search_conformance_rows",
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
                    sortable: false,
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
                    sortable: false,
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
                    facet: false,
                },
            ],
            embed_source: None,
            soft_delete: false,
            search_touches: vec![],
            settings,
            scope: None,
        }
    }
    fn document(
        index: &IndexDescriptor,
        key: &str,
        text: &str,
        number: Option<i64>,
        category: &str,
        active: bool,
    ) -> Result<IndexedDocument, FrameworkError> {
        let date = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .map_err(|e| FrameworkError::param(format!("conformance date: {e}")))?
            .with_timezone(&chrono::Utc);
        let mut attributes = BTreeMap::from([
            ("text".into(), AttributeValue::Text(text.into())),
            ("category".into(), AttributeValue::Text(category.into())),
            ("active".into(), AttributeValue::Bool(active)),
            ("created".into(), AttributeValue::DateTime(date)),
            (
                "ratio".into(),
                AttributeValue::Float(number.unwrap_or(0) as f64 / 2.0),
            ),
            (
                "tags".into(),
                AttributeValue::List(vec![
                    AttributeValue::Text(category.into()),
                    AttributeValue::Text("common".into()),
                    AttributeValue::Text(category.into()),
                ]),
            ),
            (
                "location".into(),
                AttributeValue::Geo(GeoPoint {
                    lat: number.unwrap_or(0) as f64,
                    lng: 0.0,
                }),
            ),
        ]);
        attributes.insert(
            "number".into(),
            number
                .map(AttributeValue::Integer)
                .unwrap_or(AttributeValue::Null),
        );
        let document = SearchDocument {
            index: index.name.into(),
            key: key.into(),
            source_version: 1,
            attributes,
            embed_text: None,
            title: None,
        };
        Ok(IndexedDocument {
            content_hash: document.content_hash(&index.index_settings().index_time.digest()),
            document,
            dense: Some(vec![
                if active { 1.0 } else { 0.0 },
                if active { 0.0 } else { 1.0 },
                0.0,
            ]),
            sparse: None,
        })
    }
    async fn query(
        engine: &Arc<dyn SearchEngine>,
        index: &IndexDescriptor,
        generation: &GenerationId,
        text: &str,
        filter: Option<Filter>,
    ) -> Result<EngineQuery, FrameworkError> {
        let vocabulary = engine.vocabulary(index, generation).await?;
        let settings = engine.read_settings(index, generation).await?;
        Ok(EngineQuery {
            expanded: expand(&QueryText::parse(text), &settings.query, &vocabulary, 64)?,
            filter,
            sort: vec![],
            depth: 100,
            highlights: false,
        })
    }
    /// A fresh corpus verifies every mandatory engine behavior using isolated generations.
    ///
    /// The fixture uses three-dimensional dense vectors when semantic checks are enabled.
    /// Configure any backing vector store for those dimensions before running the suite.
    /// The index must be isolated. Its previous current generation is restored on success;
    /// a fresh index retains an empty active generation because active generations cannot be dropped.
    pub async fn run(
        engine: Arc<dyn SearchEngine>,
        options: Options,
    ) -> Result<(), FrameworkError> {
        let index = descriptor(options.index_name);
        let first = GenerationId::new("conformance_a")?;
        let second = GenerationId::new("conformance_b")?;
        let settings = index.index_settings();
        let original_generation = engine.current_generation(index.name).await?;
        engine.create_generation(&index, &settings, &first).await?;
        check(
            engine.read_settings(&index, &first).await? == settings,
            "settings round-trip differs",
        )?;
        let mut query_settings = settings.clone();
        query_settings.query.hits_cap = 37;
        engine
            .write_settings(&index, &first, &query_settings)
            .await?;
        check(
            engine.read_settings(&index, &first).await? == query_settings,
            "query-time settings write was not readable",
        )?;
        engine.write_settings(&index, &first, &settings).await?;
        engine
            .switch_generation(&index, original_generation.clone(), first.clone())
            .await?;
        let mut corpus = vec![
            document(&index, "a", "red apple orchard", Some(1), "fruit", true)?,
            document(&index, "b", "green apple orchard", Some(2), "fruit", false)?,
            document(&index, "c", "apple red basket", Some(3), "basket", true)?,
            document(&index, "d", "purple banana", None, "other", false)?,
        ];
        if !options.test_semantic {
            for document in &mut corpus {
                document.dense = None;
            }
        }
        check(
            engine.index(&index, &first, &corpus).await?.written == 4,
            "fixed corpus writes were not acknowledged",
        )?;
        let zero_generation = GenerationId::new("conformance_zero")?;
        engine
            .create_generation(&index, &settings, &zero_generation)
            .await?;
        let mut zero_documents = Vec::new();
        for (key, value) in [("positive_zero", 0.0), ("negative_zero", -0.0)] {
            let mut document = corpus[0].clone();
            document.document.key = key.into();
            document
                .document
                .attributes
                .insert("ratio".into(), AttributeValue::Float(value));
            document.content_hash = document
                .document
                .content_hash(&settings.index_time.digest());
            zero_documents.push(document);
        }
        engine
            .index(&index, &zero_generation, &zero_documents)
            .await?;
        let zeros_query = query(&engine, &index, &zero_generation, "", None).await?;
        let zeros = engine
            .facets(
                &index,
                &zero_generation,
                &zeros_query,
                &["ratio".into()],
                10,
                FacetOrder::Count,
            )
            .await?;
        check(
            zeros
                .get("ratio")
                .is_some_and(|values| values.len() == 1 && values[0].count == 2),
            "positive and negative zero facet counts are split",
        )?;
        let mut zero_filter = zeros_query;
        zero_filter.filter = Some(Filter::Eq("ratio".into(), AttributeValue::Float(0.0)));
        check(
            engine
                .count(&index, &zero_generation, &zero_filter)
                .await?
                .value()
                == 2,
            "zero facet count disagrees with equal-value filtered total",
        )?;
        engine.drop_generation(&index, &zero_generation).await?;
        let empty = query(&engine, &index, &first, "", None).await?;
        check(
            engine.count(&index, &first, &empty).await?.value() == 4,
            "lexical total is not exact",
        )?;
        let phrase = query(&engine, &index, &first, "\"red apple\"", None).await?;
        check(
            engine
                .query(&index, &first, &phrase)
                .await?
                .candidates
                .iter()
                .map(|d| d.document.key.as_str())
                .collect::<Vec<_>>()
                == ["a"],
            "phrase order or adjacency was ignored",
        )?;
        let prefix = query(&engine, &index, &first, "orch", None).await?;
        check(
            engine.count(&index, &first, &prefix).await?.value() == 2,
            "last-word prefix was ignored",
        )?;
        let negative = query(&engine, &index, &first, "apple -green", None).await?;
        check(
            engine.count(&index, &first, &negative).await?.value() == 2,
            "negative word was ignored",
        )?;
        let filters = [
            ("number = 2", 1),
            ("number != 2", 3),
            ("number IN [1, 2]", 2),
            ("number NOT IN [1, 2]", 2),
            ("number > 1", 2),
            ("number >= 2", 2),
            ("number < 3", 2),
            ("number <= 2", 2),
            ("number 1 TO 2", 2),
            ("number IS NULL", 1),
            ("number IS NOT NULL", 3),
            ("active = true", 2),
            ("ratio >= 1.0", 2),
            ("category = 'fruit'", 2),
            ("tags IN ['fruit']", 2),
            ("created >= '2026-01-01T00:00:00Z'", 4),
            (
                "NOT (number = 1) AND (active = true OR category = 'fruit')",
                2,
            ),
        ];
        for (grammar, count) in filters {
            let q = query(
                &engine,
                &index,
                &first,
                "",
                Some(Filter::parse(grammar, &index)?),
            )
            .await?;
            check(
                engine.count(&index, &first, &q).await?.value() == count,
                &format!("filter {grammar} was not applied"),
            )?;
        }
        if options.test_geo {
            check(engine.capabilities().geo, "geo requested but unsupported")?;
            let q = query(
                &engine,
                &index,
                &first,
                "",
                Some(Filter::parse("_geoRadius(1, 0, 1000)", &index)?),
            )
            .await?;
            check(
                engine.count(&index, &first, &q).await?.value() == 1,
                "geo radius was ignored",
            )?;
        }
        let lexical = query(&engine, &index, &first, "apple", None).await?;
        let facets = engine
            .facets(
                &index,
                &first,
                &lexical,
                &["category".into()],
                100,
                FacetOrder::Count,
            )
            .await?;
        let values = facets
            .get("category")
            .ok_or_else(|| FrameworkError::param("conformance: category facets missing"))?;
        for value in values {
            let mut filtered = lexical.clone();
            filtered.filter = Some(Filter::Eq("category".into(), value.value.clone()));
            check(
                engine.count(&index, &first, &filtered).await?.value() == value.count,
                "facet count differs from filtered lexical total",
            )?;
        }
        let capped = engine
            .facets(
                &index,
                &first,
                &lexical,
                &["category".into()],
                1,
                FacetOrder::Alphabetical,
            )
            .await?;
        check(
            capped
                .get("category")
                .and_then(|v| v.first())
                .is_some_and(|v| v.value == AttributeValue::Text("fruit".into()) && v.count == 2),
            "facet cap did not retain the most frequent value",
        )?;
        let alphabetical = engine
            .facets(
                &index,
                &first,
                &lexical,
                &["category".into()],
                2,
                FacetOrder::Alphabetical,
            )
            .await?;
        check(
            alphabetical.get("category").is_some_and(|values| {
                values
                    .iter()
                    .map(|v| v.value.facet_key())
                    .collect::<Vec<_>>()
                    == ["basket", "fruit"]
                    && values[0].count == 1
                    && values[1].count == 2
            }),
            "facet alphabetical presentation did not follow frequency-based selection",
        )?;
        let stats = engine
            .facet_stats(
                &index,
                &first,
                &lexical,
                &["number".into(), "created".into(), "ratio".into()],
            )
            .await?;
        check(
            stats.get("number").is_some_and(|s| {
                s.min == AttributeValue::Integer(1) && s.max == AttributeValue::Integer(3)
            }),
            "facet min/max differs from complete lexical matches",
        )?;
        check(
            stats.get("ratio").is_some_and(|s| {
                s.min == AttributeValue::Float(0.5) && s.max == AttributeValue::Float(1.5)
            }),
            "float facet extrema differ from lexical matches",
        )?;
        check(
            stats.get("created").is_some_and(|s| {
                Some(&s.min) == corpus[0].document.attributes.get("created") && s.max == s.min
            }),
            "datetime facet extrema differ from lexical matches",
        )?;
        let list_facets = engine
            .facets(
                &index,
                &first,
                &lexical,
                &["tags".into()],
                100,
                FacetOrder::Count,
            )
            .await?;
        check(
            list_facets.get("tags").is_some_and(|values| {
                values
                    .iter()
                    .any(|v| v.value == AttributeValue::Text("fruit".into()) && v.count == 2)
                    && values
                        .iter()
                        .any(|v| v.value == AttributeValue::Text("common".into()) && v.count == 3)
            }),
            "list facets count repeated elements more than once per document",
        )?;
        let found = engine
            .facet_search(&index, &first, &lexical, "category", "fr", 10)
            .await?;
        check(
            found
                .get("category")
                .is_some_and(|v| v.len() == 1 && v[0].count == 2),
            "facet type-ahead count differs",
        )?;
        let mut sorted = empty.clone();
        sorted.sort = vec![Sort::Attribute {
            attribute: "number".into(),
            direction: SortDirection::Desc,
        }];
        let ordered = engine.query(&index, &first, &sorted).await?;
        check(
            ordered
                .candidates
                .iter()
                .map(|c| c.document.key.as_str())
                .collect::<Vec<_>>()
                == ["c", "b", "a", "d"],
            "sort does not place missing values last",
        )?;
        let keys = engine
            .query(&index, &first, &empty)
            .await?
            .candidates
            .into_iter()
            .map(|c| c.document.key)
            .collect::<Vec<_>>();
        let mut limited = empty.clone();
        limited.depth = 2;
        let limited_keys = engine
            .query(&index, &first, &limited)
            .await?
            .candidates
            .into_iter()
            .map(|candidate| candidate.document.key)
            .collect::<Vec<_>>();
        check(
            limited_keys == keys.iter().take(2).cloned().collect::<Vec<_>>(),
            "candidate-depth limit changed ranked prefix order",
        )?;
        let first_page = keys.iter().take(2).collect::<Vec<_>>();
        let second_page = keys.iter().skip(2).take(2).collect::<Vec<_>>();
        check(
            first_page.iter().all(|key| !second_page.contains(key)),
            "consecutive ranked slices overlap",
        )?;
        if options.test_semantic {
            check(
                engine.capabilities().semantic,
                "semantic requested but unsupported",
            )?;
            let semantic = engine
                .semantic(
                    &index,
                    &first,
                    &[1.0, 0.0, 0.0],
                    Some(&Filter::Eq(
                        "category".into(),
                        AttributeValue::Text("fruit".into()),
                    )),
                    10,
                )
                .await?;
            check(
                semantic
                    .candidates
                    .first()
                    .is_some_and(|c| c.document.key == "a" && c.score > 0.99),
                "semantic cosine or filtering failed",
            )?;
        }
        let vocabulary = engine.vocabulary(&index, &first).await?;
        check(
            vocabulary.words.get("apple") == Some(&3),
            "vocabulary does not contain document frequencies",
        )?;
        let mut newer = corpus[0].clone();
        newer.document.source_version = 2;
        newer
            .document
            .attributes
            .insert("text".into(), AttributeValue::Text("newer orchard".into()));
        newer.content_hash = newer
            .document
            .content_hash(&index.index_settings().index_time.digest());
        check(
            engine
                .index(&index, &first, &[newer.clone()])
                .await?
                .written
                == 1,
            "newer write did not apply",
        )?;
        check(
            engine
                .index(&index, &first, &[corpus[0].clone()])
                .await?
                .skipped_older
                == 1,
            "older source version overwrote a newer one",
        )?;
        newer.document.source_version = 4;
        check(
            engine
                .index(&index, &first, &[newer.clone()])
                .await?
                .skipped_unchanged
                == 1,
            "identical content was not skipped",
        )?;
        check(
            engine
                .remove(&index, &first, &[("a".into(), 3)])
                .await?
                .skipped_older
                == 1,
            "unchanged content did not advance the source-version watermark",
        )?;
        engine.remove(&index, &first, &[("a".into(), 5)]).await?;
        check(
            engine
                .index(&index, &first, &[newer])
                .await?
                .skipped_tombstoned
                == 1,
            "a reordered write resurrected a removal",
        )?;
        check(
            !engine
                .vocabulary(&index, &first)
                .await?
                .words
                .contains_key("newer"),
            "removed document left a zero-frequency vocabulary term",
        )?;
        check(
            engine
                .remove(&index, &first, &[("a".into(), 4)])
                .await?
                .skipped_older
                == 1,
            "older removal weakened an existing tombstone",
        )?;
        let mut equal_version = corpus[0].clone();
        equal_version.document.source_version = 5;
        check(
            engine
                .index(&index, &first, &[equal_version.clone()])
                .await?
                .skipped_tombstoned
                == 1,
            "equal source version resurrected a tombstoned document",
        )?;
        equal_version.document.source_version = 6;
        check(
            engine
                .index(&index, &first, &[equal_version])
                .await?
                .written
                == 1,
            "newer source version could not replace a tombstone",
        )?;
        check(
            engine
                .query(&index, &first, &empty)
                .await?
                .candidates
                .iter()
                .any(|candidate| {
                    candidate.document.key == "a" && candidate.document.source_version == 6
                }),
            "newer write source version was not readable",
        )?;
        engine.create_generation(&index, &settings, &second).await?;
        check(
            engine.count(&index, &second, &empty).await?.value() == 0,
            "new generation contains old documents",
        )?;
        engine.index(&index, &second, &corpus).await?;
        engine
            .switch_generation(&index, Some(first.clone()), second.clone())
            .await?;
        check(
            engine.current_generation(index.name).await? == Some(second.clone()),
            "generation switch did not become current",
        )?;
        check(
            engine.drop_generation(&index, &second).await.is_err(),
            "active generation could be dropped",
        )?;
        engine.drop_generation(&index, &first).await?;
        let retirement = match original_generation {
            Some(generation) => generation,
            None => {
                let generation = GenerationId::new("conformance_retired")?;
                engine
                    .create_generation(&index, &settings, &generation)
                    .await?;
                generation
            }
        };
        engine
            .switch_generation(&index, Some(second.clone()), retirement)
            .await?;
        engine.drop_generation(&index, &second).await?;
        Ok(())
    }
    /// An empty expanded plan is useful for isolated count or facet checks.
    pub fn empty_query() -> EngineQuery {
        EngineQuery {
            expanded: ExpandedQuery {
                terms: vec![],
                phrases: vec![],
                negatives: vec![],
                ..ExpandedQuery::default()
            },
            filter: None,
            sort: vec![],
            depth: 100,
            highlights: false,
        }
    }
}
