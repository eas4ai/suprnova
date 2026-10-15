use std::path::PathBuf;
use suprnova::search::{
    EmbedInput, Embedder, Precision, default_profile,
    embedder::fastembed::{
        FastembedConfig, FastembedEmbedder, ModelManifest, ModelSource, precision_files,
    },
};

#[test]
fn model_identifiers_are_typed_and_named_matching_is_case_insensitive() {
    assert!(
        matches!(ModelSource::parse("hf:owner/model@abc123").unwrap(),ModelSource::HuggingFace {repository,revision:Some(revision)} if repository=="owner/model"&&revision=="abc123")
    );
    assert!(
        matches!(ModelSource::parse("path:/tmp/model").unwrap(),ModelSource::Path(path) if path==std::path::Path::new("/tmp/model"))
    );
    assert!(matches!(
        ModelSource::parse("bgesmallenv15").unwrap(),
        ModelSource::Named(_)
    ));
    for invalid in [
        "hf:",
        "hf:../model",
        "hf:owner/model@",
        "path:",
        "no-such-model",
    ] {
        assert!(ModelSource::parse(invalid).is_err());
    }
}
#[test]
fn precision_graph_names_are_explicit() {
    assert_eq!(
        precision_files(Precision::Fp32),
        ("onnx/model.onnx", Some("onnx/model.onnx_data"))
    );
    assert_eq!(
        precision_files(Precision::Int8),
        ("onnx/model_quantized.onnx", None)
    );
    assert_eq!(precision_files(Precision::Q4), ("onnx/model_q4.onnx", None));
}
#[test]
fn manifests_record_all_files_and_refuse_a_changed_byte() {
    let root = tempfile::tempdir().unwrap();
    for name in ["model.onnx", "tokenizer.json"] {
        std::fs::write(root.path().join(name), b"tiny fixture").unwrap();
    }
    let manifest = ModelManifest::record(
        root.path(),
        "fixture/model",
        "abc123",
        &["model.onnx".into(), "tokenizer.json".into()],
    )
    .unwrap();
    assert_eq!(manifest.repository, "fixture/model");
    assert_eq!(manifest.revision, "abc123");
    assert_eq!(manifest.files.len(), 2);
    let digest = manifest.verify(root.path()).unwrap();
    assert_eq!(digest.len(), 64);
    assert!(root.path().join("manifest.json").exists());
    std::fs::write(root.path().join("model.onnx"), b"Tiny fixture").unwrap();
    assert!(
        manifest
            .verify(root.path())
            .unwrap_err()
            .to_string()
            .contains("model.onnx")
    );
}
#[test]
fn default_is_the_fp32_embeddinggemma_two_profile() {
    let config = FastembedConfig::default();
    assert_eq!(config.model, "hf:onnx-community/embeddinggemma-2-ONNX");
    assert_eq!(config.precision, Precision::Fp32);
    assert_eq!(config.batch, 64);
    assert_eq!(config.cache, PathBuf::from("storage/search/models"));
    let profile = default_profile();
    assert_eq!(profile.dimensions, 768);
    assert_eq!(profile.max_tokens, 8192);
    assert_eq!(
        profile
            .document_input(&EmbedInput {
                text: "rust".into(),
                ..Default::default()
            })
            .unwrap(),
        "title: none | text: rust"
    );
}
#[tokio::test]
#[ignore = "downloads EmbeddingGemma 2"]
async fn real_default_model_is_normalized_and_applies_document_template() {
    let started = std::time::Instant::now();
    let embedder = FastembedEmbedder::new(FastembedConfig::default())
        .await
        .unwrap();
    let input = EmbedInput {
        text: "Rust is a systems programming language.".into(),
        ..Default::default()
    };
    let document = embedder
        .embed_documents(std::slice::from_ref(&input))
        .await
        .unwrap()
        .remove(0);
    let query = embedder
        .embed_query("systems programming language")
        .await
        .unwrap();
    for vector in [&document, &query] {
        assert_eq!(vector.dense.len(), 768);
        assert!(
            (vector
                .dense
                .iter()
                .map(|x| f64::from(*x).powi(2))
                .sum::<f64>()
                - 1.)
                .abs()
                < 1e-5
        );
    }
    let mut profile = embedder.profile().clone();
    profile.query_prefix.clear();
    let prepared = profile.document_input(&input).unwrap();
    let manual = embedder
        .embed_prepared(&[prepared])
        .await
        .unwrap()
        .remove(0);
    assert_eq!(document, manual);
    let provenance = embedder.profile().provenance().unwrap();
    assert!(provenance.revision.is_some());
    assert!(provenance.manifest_sha256.is_some());
    eprintln!(
        "EmbeddingGemma 2 load and inference: {:?}; cached model bytes: {}",
        started.elapsed(),
        embedder.cached_bytes()
    );
}

#[test]
fn manifest_paths_cannot_escape_the_cache() {
    let root = tempfile::tempdir().unwrap();
    for name in ["../outside", "/tmp/outside"] {
        assert!(
            ModelManifest::record(root.path(), "fixture/model", "local", &[name.into()]).is_err()
        );
    }
}

#[cfg(feature = "testing")]
#[tokio::test]
async fn rerankers_register_lazily_for_builder_lookup() {
    use suprnova::search::{FastembedReranker, SearchRegistry};
    SearchRegistry::isolated(async {
        let cache = tempfile::tempdir().unwrap();
        FastembedReranker::register_supported(cache.path().into(), 2).unwrap();
        for name in [
            "BAAI/bge-reranker-base",
            "bge-reranker-base",
            "BAAI/bge-reranker-v2-m3",
            "jina-reranker-v1-turbo-en",
            "jina-reranker-v2-base-multilingual",
        ] {
            let reranker = SearchRegistry::reranker(name).unwrap();
            assert!(reranker.rerank("rust", &[]).await.unwrap().is_empty());
        }
        assert_eq!(std::fs::read_dir(cache.path()).unwrap().count(), 0);
    })
    .await;
}

fn tiny_model() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("onnx")).unwrap();
    std::fs::write(
        root.path().join("onnx/model_quantized.onnx"),
        include_bytes!("fixtures/text.onnx"),
    )
    .unwrap();
    for (name, bytes) in [
        (
            "tokenizer.json",
            include_bytes!("fixtures/tokenizer.json").as_slice(),
        ),
        (
            "config.json",
            include_bytes!("fixtures/config.json").as_slice(),
        ),
        (
            "tokenizer_config.json",
            include_bytes!("fixtures/tokenizer_config.json").as_slice(),
        ),
        (
            "special_tokens_map.json",
            include_bytes!("fixtures/special_tokens_map.json").as_slice(),
        ),
    ] {
        std::fs::write(root.path().join(name), bytes).unwrap();
    }
    root
}
#[tokio::test]
async fn dynamic_quantization_is_independent_of_document_batch_and_query_shape() {
    let root = tiny_model();
    let cache = tempfile::tempdir().unwrap();
    let mut profile = suprnova::search::FakeEmbedder::new(2)
        .unwrap()
        .profile()
        .clone();
    profile.output_key = Some("custom_output".into());
    let config = FastembedConfig {
        model: format!("path:{}", root.path().display()),
        cache: cache.path().into(),
        precision: Precision::Int8,
        batch: 64,
        profile: Some(profile),
        ..Default::default()
    };
    let embedder = FastembedEmbedder::new(config).await.unwrap();
    let documents = vec![
        EmbedInput {
            text: "low".into(),
            ..Default::default()
        },
        EmbedInput {
            text: "high".into(),
            ..Default::default()
        },
    ];
    let together = embedder.embed_documents(&documents).await.unwrap();
    let query = embedder.embed_query("low").await.unwrap();
    assert_eq!(together[0], query);
    assert_eq!(together[1], embedder.embed_query("high").await.unwrap());
}

#[tokio::test]
async fn reduced_local_dimensions_pass_the_core_validation_boundary() {
    let root = tiny_model();
    let cache = tempfile::tempdir().unwrap();
    let mut profile = suprnova::search::FakeEmbedder::new(2)
        .unwrap()
        .profile()
        .clone();
    profile.output_key = Some("custom_output".into());
    profile.truncation_dims = vec![1];
    let config = FastembedConfig {
        model: format!("path:{}", root.path().display()),
        cache: cache.path().into(),
        precision: Precision::Int8,
        profile: Some(profile),
        dimensions: Some(1),
        ..Default::default()
    };
    let embedder = FastembedEmbedder::new(config).await.unwrap();
    assert_eq!(embedder.profile().dimensions, 1);
    let vector = embedder.embed_query("low").await.unwrap();
    suprnova::search::validate_embedding(vector.dense, embedder.profile(), None).unwrap();
}
#[test]
fn pinned_joint_sparse_files_load_with_external_tensors_and_refuse_corruption() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("sparse.onnx"),
        include_bytes!("fixtures/sparse.onnx"),
    )
    .unwrap();
    std::fs::write(
        root.path().join("sparse.onnx_data"),
        include_bytes!("fixtures/sparse.onnx_data"),
    )
    .unwrap();
    let manifest = ModelManifest::record(
        root.path(),
        "fixture/joint-sparse",
        "local",
        &["sparse.onnx".into(), "sparse.onnx_data".into()],
    )
    .unwrap();
    manifest.verify(root.path()).unwrap();
    let files = fastembed::TokenizerFiles {
        tokenizer_file: include_bytes!("fixtures/tokenizer.json").to_vec(),
        config_file: include_bytes!("fixtures/config.json").to_vec(),
        tokenizer_config_file: include_bytes!("fixtures/tokenizer_config.json").to_vec(),
        special_tokens_map_file: include_bytes!("fixtures/special_tokens_map.json").to_vec(),
    };
    let mut model = fastembed::Bgem3Embedding::try_new_from_path(
        root.path().join("sparse.onnx"),
        files,
        fastembed::InitOptionsUserDefined::new().with_intra_threads(2),
    )
    .unwrap();
    let output = model.embed(["middle high"], Some(1)).unwrap();
    assert_eq!(output.sparse.len(), 1);
    assert_eq!(output.sparse[0].indices, vec![17, 1000]);
    std::fs::write(root.path().join("sparse.onnx_data"), [0_u8; 4]).unwrap();
    assert!(
        manifest
            .verify(root.path())
            .unwrap_err()
            .to_string()
            .contains("sparse.onnx_data")
    );
}
#[cfg(feature = "testing")]
#[tokio::test]
async fn lexical_boot_installs_rerankers_without_replacing_application_registration() {
    use std::sync::Arc;
    use suprnova::search::{FastembedReranker, Reranker, Search, SearchConfig, SearchRegistry};
    SearchRegistry::isolated(async {
        let cache = tempfile::tempdir().unwrap();
        let custom: Arc<dyn Reranker> =
            Arc::new(FastembedReranker::new("bge-reranker-base", cache.path().into(), 2).unwrap());
        SearchRegistry::register_reranker("BAAI/bge-reranker-base", custom.clone()).unwrap();
        SearchConfig::scope(SearchConfig::default(), async {
            Search::boot().await.unwrap();
            Search::boot().await.unwrap();
            assert!(SearchRegistry::reranker("jina-reranker-v1-turbo-en").is_ok());
            assert!(Arc::ptr_eq(
                &custom,
                &SearchRegistry::reranker("BAAI/bge-reranker-base").unwrap()
            ));
        })
        .await
        .unwrap();
    })
    .await;
}

#[test]
fn a_generated_graph_requires_its_own_manifest_digest() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("graph.onnx"), b"original").unwrap();
    std::fs::write(root.path().join("derived.onnx"), b"derived graph").unwrap();
    let mut manifest = ModelManifest::record(
        root.path(),
        "fixture/model",
        "local",
        &["graph.onnx".into()],
    )
    .unwrap();
    manifest.generated_files.insert(
        "derived.onnx".into(),
        "graph.onnx with empty features, fixture-v1".into(),
    );
    std::fs::write(
        root.path().join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let error = ModelManifest::read(root.path())
        .unwrap()
        .verify(root.path())
        .unwrap_err()
        .to_string();
    assert!(error.contains("derived.onnx"));
}
