use suprnova::search::embedder::{
    EmbedInput, Embedder, MediaPart, SparseEmbedding, fake::FakeEmbedder, profiles,
    validate_embedding,
};

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
