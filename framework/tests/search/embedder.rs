use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use suprnova::search::embedder::{
    EmbedInput, Embedder, MediaPart, SparseEmbedding, fake::FakeEmbedder, profiles,
    validate_embedding,
};
use suprnova::{FrameworkError, LazyEmbedder};

#[cfg(feature = "testing")]
#[tokio::test]
async fn lazy_registration_keeps_the_shipped_profile_until_resolution_and_loads_once() {
    use suprnova::SearchRegistry;

    SearchRegistry::isolated(async {
        let mut shipped = FakeEmbedder::new(8).unwrap().profile().clone();
        shipped.revision = None;
        let mut resolved = shipped.clone();
        resolved.revision = Some("resolved-artifact".into());
        let model: Arc<dyn Embedder> =
            Arc::new(FakeEmbedder::with_profile(resolved.clone()).unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let lazy = Arc::new(
            LazyEmbedder::new(shipped.clone(), {
                let calls = calls.clone();
                move || {
                    let model = model.clone();
                    let calls = calls.clone();
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        tokio::task::yield_now().await;
                        Ok(model)
                    }
                }
            })
            .unwrap(),
        );
        SearchRegistry::register_embedder("local", lazy.clone()).unwrap();
        assert!(lazy.loaded().is_none());
        assert_eq!(lazy.profile(), &shipped);
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let registered = SearchRegistry::embedder_named("lazy", "local").unwrap();
        let (first, concurrent) =
            tokio::join!(registered.resolve_profile(), lazy.resolve_profile());
        assert_eq!(first.unwrap(), resolved);
        assert_eq!(concurrent.unwrap(), resolved);
        assert_eq!(lazy.resolve_profile().await.unwrap(), resolved);
        assert_eq!(lazy.profile(), &resolved);
        assert!(lazy.loaded().is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    })
    .await;
}

#[tokio::test]
async fn lazy_load_errors_are_retried_by_later_embedding_calls() {
    let fake = FakeEmbedder::new(8).unwrap();
    let profile = fake.profile().clone();
    let expected = fake.embed_query("retry").await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let lazy = LazyEmbedder::new(profile.clone(), {
        let calls = calls.clone();
        move || {
            let attempt = calls.fetch_add(1, Ordering::SeqCst);
            let profile = profile.clone();
            async move {
                if attempt == 0 {
                    return Err(FrameworkError::param("local model temporarily unavailable"));
                }
                Ok(Arc::new(FakeEmbedder::with_profile(profile)?) as Arc<dyn Embedder>)
            }
        }
    })
    .unwrap();
    let error = lazy.resolve_profile().await.unwrap_err().to_string();
    assert!(
        error.contains("local model temporarily unavailable"),
        "{error}"
    );
    assert!(lazy.loaded().is_none());
    assert_eq!(lazy.embed_query("retry").await.unwrap(), expected);
    assert_eq!(lazy.embed_query("retry").await.unwrap(), expected);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn lazy_refuses_another_model_or_dimensions_before_every_embedding_operation() {
    let shipped = FakeEmbedder::new(8).unwrap().profile().clone();
    for (id, dimensions) in [("other-model", 8), ("fake-hash", 16)] {
        let mut resolved = shipped.clone();
        resolved.id = id.into();
        resolved.dimensions = dimensions;
        let model: Arc<dyn Embedder> = Arc::new(FakeEmbedder::with_profile(resolved).unwrap());
        let lazy = LazyEmbedder::new(shipped.clone(), move || {
            let model = model.clone();
            async move { Ok(model) }
        })
        .unwrap();
        let inputs = [EmbedInput {
            text: "refused".into(),
            ..Default::default()
        }];
        let texts = ["refused".into()];
        let errors = [
            lazy.resolve_profile().await.unwrap_err(),
            lazy.embed_documents(&inputs).await.unwrap_err(),
            lazy.embed_query("refused").await.unwrap_err(),
            lazy.embed_sparse_query("refused").await.unwrap_err(),
            lazy.embed_sparse(&texts).await.unwrap_err(),
            lazy.embed_documents_metered(&inputs).await.unwrap_err(),
            lazy.embed_query_metered("refused").await.unwrap_err(),
        ];
        for error in errors {
            let error = error.to_string();
            assert!(error.contains("fake-hash"), "{error}");
            assert!(error.contains(id), "{error}");
            assert!(error.contains("8 dimensions"), "{error}");
            assert!(
                error.contains(&format!("{dimensions} dimensions")),
                "{error}"
            );
        }
        assert!(lazy.loaded().is_none());
        assert_eq!(lazy.profile(), &shipped);
    }
}

#[test]
fn lazy_validates_the_shipped_profile_without_loading() {
    let mut invalid = FakeEmbedder::new(8).unwrap().profile().clone();
    invalid.dimensions = 0;
    assert!(
        LazyEmbedder::new(invalid, || async {
            panic!("an invalid profile must not invoke the loader")
        })
        .is_err()
    );
}

#[tokio::test]
async fn lazy_delegates_dense_sparse_and_metered_operations_to_the_loaded_provider() {
    use suprnova::{Embedding, EmbeddingUsage, Metered, ModelProfile};

    struct Provider(FakeEmbedder);
    #[async_trait::async_trait]
    impl Embedder for Provider {
        fn profile(&self) -> &ModelProfile {
            self.0.profile()
        }
        async fn embed_documents(
            &self,
            inputs: &[EmbedInput],
        ) -> Result<Vec<Embedding>, FrameworkError> {
            self.0.embed_documents(inputs).await
        }
        async fn embed_query(&self, text: &str) -> Result<Embedding, FrameworkError> {
            self.0.embed_query(text).await
        }
        async fn embed_sparse(
            &self,
            texts: &[String],
        ) -> Result<Vec<SparseEmbedding>, FrameworkError> {
            Ok(texts
                .iter()
                .map(|_| SparseEmbedding {
                    indices: vec![1],
                    values: vec![0.5],
                })
                .collect())
        }
        async fn embed_sparse_query(&self, _: &str) -> Result<SparseEmbedding, FrameworkError> {
            Ok(SparseEmbedding {
                indices: vec![2],
                values: vec![0.75],
            })
        }
        async fn embed_documents_metered(
            &self,
            inputs: &[EmbedInput],
        ) -> Result<Metered<Vec<Embedding>>, FrameworkError> {
            Ok(Metered {
                value: self.0.embed_documents(inputs).await?,
                usage: Some(EmbeddingUsage {
                    prompt_tokens: Some(9),
                    total_tokens: Some(9),
                }),
            })
        }
        async fn embed_query_metered(
            &self,
            text: &str,
        ) -> Result<Metered<Embedding>, FrameworkError> {
            Ok(Metered {
                value: self.0.embed_query(text).await?,
                usage: Some(EmbeddingUsage {
                    prompt_tokens: Some(7),
                    total_tokens: Some(7),
                }),
            })
        }
    }

    for operation in 0..6 {
        let fake = FakeEmbedder::new(8).unwrap();
        let expected = fake.embed_query("delegated").await.unwrap();
        let model: Arc<dyn Embedder> = Arc::new(Provider(fake));
        let lazy = LazyEmbedder::new(model.profile().clone(), move || {
            let model = model.clone();
            async move { Ok(model) }
        })
        .unwrap();
        let inputs = [EmbedInput {
            text: "delegated".into(),
            ..Default::default()
        }];
        assert!(lazy.loaded().is_none());
        match operation {
            0 => assert_eq!(lazy.embed_documents(&inputs).await.unwrap(), vec![expected]),
            1 => assert_eq!(lazy.embed_query("delegated").await.unwrap(), expected),
            2 => assert_eq!(
                lazy.embed_sparse(&["delegated".into()]).await.unwrap(),
                vec![SparseEmbedding {
                    indices: vec![1],
                    values: vec![0.5]
                }]
            ),
            3 => assert_eq!(
                lazy.embed_sparse_query("delegated").await.unwrap(),
                SparseEmbedding {
                    indices: vec![2],
                    values: vec![0.75]
                }
            ),
            4 => {
                let result = lazy.embed_documents_metered(&inputs).await.unwrap();
                assert_eq!(result.value, vec![expected]);
                assert_eq!(result.usage.unwrap().total_tokens, Some(9));
            }
            _ => {
                let result = lazy.embed_query_metered("delegated").await.unwrap();
                assert_eq!(result.value, expected);
                assert_eq!(result.usage.unwrap().total_tokens, Some(7));
            }
        }
        assert!(lazy.loaded().is_some());
    }
}

#[tokio::test]
async fn equal_texts_produce_unit_cosine_and_configured_dimensions() {
    let embedder = FakeEmbedder::new(16).unwrap();
    let query = embedder.embed_query("rust search").await.unwrap();
    let docs = embedder
        .embed_documents(&[EmbedInput {
            text: "rust search".into(),
            ..Default::default()
        }])
        .await
        .unwrap();
    assert_eq!(query, docs[0]);
    assert_eq!(query.dense.len(), 16);
    let cosine: f32 = query
        .dense
        .iter()
        .zip(&docs[0].dense)
        .map(|(a, b)| a * b)
        .sum();
    assert!((cosine - 1.0).abs() < 1e-6);
    assert!(FakeEmbedder::new(0).is_err());
    assert!(embedder.embed_sparse(&["text".into()]).await.is_err());
}

#[test]
fn default_profile_is_embeddinggemma_two_with_its_retrieval_preparation() {
    let profile = profiles::default_profile();
    assert_eq!(profile.dimensions, 768);
    assert_eq!(profile.max_tokens, 8192);
    assert_eq!(
        profile.query_input("rust").unwrap(),
        "task: search result | query: rust"
    );
    assert_eq!(
        profile
            .document_input(&EmbedInput {
                text: "body {title}".into(),
                ..Default::default()
            })
            .unwrap(),
        "title: none | text: body {title}"
    );
    assert_eq!(profile.truncation_dims, [512, 256, 128]);
}

#[test]
fn minilm_profiles_use_their_model_configuration_token_limits() {
    let l6 = profiles::profile("all-MiniLM-L6-v2").unwrap();
    let l12 = profiles::profile("all-MiniLM-L12-v2").unwrap();
    assert_eq!(l6.dimensions, 384);
    assert_eq!(l6.max_tokens, 256);
    assert_eq!(l12.dimensions, 384);
    assert_eq!(l12.max_tokens, 128);
}

#[test]
fn named_profiles_validate_and_apply_each_family_preparation() {
    for name in profiles::names() {
        profiles::profile(name).unwrap().validate().unwrap();
    }
    let input = EmbedInput {
        text: "body".into(),
        ..Default::default()
    };
    assert_eq!(
        profiles::profile("multilingual-e5-small")
            .unwrap()
            .document_input(&input)
            .unwrap(),
        "passage: body"
    );
    assert_eq!(
        profiles::profile("nomic-embed-text-v1.5")
            .unwrap()
            .query_input("q")
            .unwrap(),
        "search_query: q"
    );
    assert_eq!(
        profiles::profile("bge-m3")
            .unwrap()
            .query_input("q")
            .unwrap(),
        "q"
    );
    assert!(profiles::profile("unknown").is_err());
}

#[test]
fn invalid_vectors_fail_before_storage_and_truncation_renormalizes() {
    let mut profile = FakeEmbedder::new(4).unwrap().profile().clone();
    profile.truncation_dims = vec![2];
    let embedding = validate_embedding(vec![3.0, 4.0, 9.0, 9.0], &profile, Some(2)).unwrap();
    assert_eq!(embedding.dense, vec![0.6, 0.8]);
    for vector in [
        vec![1.0],
        vec![1.0, f32::NAN, 1.0, 1.0],
        vec![f32::INFINITY; 4],
        vec![0.0; 4],
    ] {
        assert!(validate_embedding(vector, &profile, None).is_err());
    }
    assert!(validate_embedding(vec![1.0; 4], &profile, Some(3)).is_err());
}

#[test]
fn media_and_malformed_templates_fail_with_profile_context() {
    let mut profile = profiles::default_profile();
    assert!(
        profile
            .document_input(&EmbedInput {
                media: vec![MediaPart {
                    media_type: "image/png".into(),
                    data: vec![1]
                }],
                ..Default::default()
            })
            .is_err()
    );
    profile.document_template = "{unknown}".into();
    assert!(profile.query_input("text").is_err());
    assert!(
        SparseEmbedding {
            indices: vec![2, 1],
            values: vec![1.0, 2.0]
        }
        .validate()
        .is_err()
    );
    SparseEmbedding {
        indices: vec![1, 2],
        values: vec![1.0, 2.0],
    }
    .validate()
    .unwrap();
}

#[tokio::test]
async fn fake_applies_profiles_before_hashing() {
    let mut profile = FakeEmbedder::new(8).unwrap().profile().clone();
    profile.query_prefix = "query: ".into();
    profile.document_template = "passage: {text}".into();
    let prepared = FakeEmbedder::with_profile(profile).unwrap();
    let plain = FakeEmbedder::new(8).unwrap();
    assert_eq!(
        prepared.embed_query("text").await.unwrap(),
        plain.embed_query("query: text").await.unwrap()
    );
    assert_eq!(
        prepared
            .embed_documents(&[EmbedInput {
                text: "text".into(),
                ..Default::default()
            }])
            .await
            .unwrap()[0],
        plain.embed_query("passage: text").await.unwrap()
    );
}

#[test]
fn index_template_overrides_profile_template_without_reinterpreting_user_braces() {
    let profile = profiles::default_profile();
    let input = EmbedInput {
        text: "body {title}".into(),
        title: Some("Heading".into()),
        document_template: Some("{title}: {text}".into()),
        ..Default::default()
    };
    assert_eq!(
        profile.document_input(&input).unwrap(),
        "Heading: body {title}"
    );
    let invalid = EmbedInput {
        document_template: Some("{unknown}".into()),
        ..input
    };
    assert!(profile.document_input(&invalid).is_err());
}

#[test]
fn native_vectors_keep_magnitude_only_when_profile_disables_normalization() {
    let mut profile = FakeEmbedder::new(4).unwrap().profile().clone();
    profile.normalize = false;
    profile.truncation_dims = vec![2];
    assert_eq!(
        validate_embedding(vec![3.0, 4.0, 0.0, 0.0], &profile, None)
            .unwrap()
            .dense,
        [3.0, 4.0, 0.0, 0.0]
    );
    assert_eq!(
        validate_embedding(vec![3.0, 4.0, 0.0, 0.0], &profile, Some(2))
            .unwrap()
            .dense,
        [0.6, 0.8]
    );
    assert!(
        SparseEmbedding {
            indices: vec![1, 1],
            values: vec![1.0, 1.0]
        }
        .validate()
        .is_err()
    );
    assert!(
        SparseEmbedding {
            indices: vec![1],
            values: vec![]
        }
        .validate()
        .is_err()
    );
}

#[tokio::test]
async fn fake_truncates_its_scalar_tokens_after_applying_preparation() {
    let mut profile = FakeEmbedder::new(8).unwrap().profile().clone();
    profile.max_tokens = 3;
    let fake = FakeEmbedder::with_profile(profile).unwrap();
    assert_eq!(
        fake.embed_query("abé remainder").await.unwrap(),
        fake.embed_query("abé").await.unwrap()
    );
    let input = EmbedInput {
        text: "abé remainder".into(),
        ..Default::default()
    };
    assert_eq!(
        fake.embed_documents(&[input]).await.unwrap()[0],
        fake.embed_query("abé").await.unwrap()
    );
}

#[cfg(feature = "testing")]
#[tokio::test]
async fn boot_none_keeps_lexical_only_error_and_existing_provider_is_retained() {
    use std::sync::Arc;
    use suprnova::search::{Search, SearchConfig, SearchEmbedder, SearchRegistry};
    SearchRegistry::isolated(async {
        SearchConfig::scope(SearchConfig::default(), async {
            Search::boot().await.unwrap();
            assert!(
                SearchRegistry::embedder_named("boot", "none")
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("SEARCH_EMBEDDER")
            );
        })
        .await
        .unwrap();
        let provider = Arc::new(suprnova::search::FakeEmbedder::new(4).unwrap());
        SearchRegistry::register_embedder("http", provider.clone()).unwrap();
        SearchConfig::scope(
            SearchConfig {
                embedder: SearchEmbedder::Http,
                ..Default::default()
            },
            async {
                Search::boot().await.unwrap();
                Search::boot().await.unwrap();
                let registered = SearchRegistry::embedder_named("boot", "http").unwrap();
                assert_eq!(registered.profile().dimensions, 4);
            },
        )
        .await
        .unwrap();
    })
    .await;
}

#[cfg(all(feature = "testing", not(feature = "search-fastembed")))]
#[tokio::test]
async fn fastembed_boot_names_the_required_feature_when_it_is_disabled() {
    use suprnova::search::{Search, SearchConfig, SearchEmbedder, SearchRegistry};
    SearchRegistry::isolated(async {
        SearchConfig::scope(
            SearchConfig {
                embedder: SearchEmbedder::Fastembed,
                ..Default::default()
            },
            async {
                let error = Search::boot().await.unwrap_err().to_string();
                assert!(error.contains("SEARCH_EMBEDDER"));
                assert!(error.contains("search-fastembed"));
            },
        )
        .await
        .unwrap();
    })
    .await;
}
