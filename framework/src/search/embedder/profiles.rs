//! Shipped profiles mirror model cards rather than relying on provider-specific defaults.

use super::{InputKinds, ModelFiles, ModelProfile, Pooling, Precision};
use crate::FrameworkError;

const RETRIEVAL_PREFIX: &str = "Represent this sentence for searching relevant passages: ";
const NAMES: &[&str] = &[
    "embeddinggemma-2",
    "embeddinggemma-300m",
    "bge-small-en-v1.5",
    "bge-base-en-v1.5",
    "bge-large-en-v1.5",
    "bge-m3",
    "multilingual-e5-small",
    "multilingual-e5-base",
    "multilingual-e5-large",
    "nomic-embed-text-v1.5",
    "gte-base-en-v1.5",
    "gte-large-en-v1.5",
    "mxbai-embed-large-v1",
    "snowflake-arctic-embed-xs",
    "snowflake-arctic-embed-s",
    "snowflake-arctic-embed-m",
    "snowflake-arctic-embed-l",
    "all-MiniLM-L6-v2",
    "all-MiniLM-L12-v2",
];

/// Names let configuration tools advertise supported profiles without loading model weights.
pub fn names() -> &'static [&'static str] {
    NAMES
}

/// The text ONNX export is the default; media capability requires a media graph profile.
/// Model card: <https://huggingface.co/google/embeddinggemma-2>.
pub fn default_profile() -> ModelProfile {
    let mut profile = text_profile(
        "onnx-community/embeddinggemma-2-ONNX",
        768,
        8192,
        Pooling::Mean,
    );
    profile.query_prefix = "task: search result | query: ".into();
    profile.document_template = "title: {title} | text: {text}".into();
    profile.truncation_dims = vec![512, 256, 128];
    profile.files.external_data = Some("onnx/model.onnx_data".into());
    profile
}

/// Repository IDs and short names resolve to the same preparation contract.
/// BGE-M3 omits instructions per its card. The public search configuration
/// specifies no prefix for Arctic and mxbai, overriding their cards'
/// retrieval instruction; applications can declare a custom profile to use it.
/// Sources: <https://huggingface.co/BAAI/bge-m3>,
/// <https://huggingface.co/Snowflake/snowflake-arctic-embed-s>,
/// <https://huggingface.co/mixedbread-ai/mxbai-embed-large-v1>.
/// Nomic's 8192-token limit requires a graph with the model card's RoPE scaling.
/// MiniLM limits follow each model's `sentence_bert_config.json`, including
/// L12-v2's 128-token limit rather than the shared overview table's 256.
pub fn profile(name: &str) -> Result<ModelProfile, FrameworkError> {
    let normalized = name
        .strip_prefix("hf:")
        .unwrap_or(name)
        .to_ascii_lowercase();
    let short = normalized.rsplit('/').next().unwrap_or(&normalized);
    let mut profile = match short {
        "embeddinggemma-2" | "embeddinggemma-2-onnx" => default_profile(),
        "embeddinggemma-300m" | "embeddinggemma-300m-onnx" => {
            let mut profile = text_profile("google/embeddinggemma-300m", 768, 2048, Pooling::Mean);
            profile.query_prefix = "task: search result | query: ".into();
            profile.document_template = "title: {title} | text: {text}".into();
            profile.truncation_dims = vec![512, 256, 128];
            profile
        }
        "bge-small-en-v1.5" => text_profile("BAAI/bge-small-en-v1.5", 384, 512, Pooling::Cls),
        "bge-base-en-v1.5" => text_profile("BAAI/bge-base-en-v1.5", 768, 512, Pooling::Cls),
        "bge-large-en-v1.5" => text_profile("BAAI/bge-large-en-v1.5", 1024, 512, Pooling::Cls),
        "bge-m3" => text_profile("BAAI/bge-m3", 1024, 8192, Pooling::Cls),
        "multilingual-e5-small" => {
            text_profile("intfloat/multilingual-e5-small", 384, 512, Pooling::Mean)
        }
        "multilingual-e5-base" => {
            text_profile("intfloat/multilingual-e5-base", 768, 512, Pooling::Mean)
        }
        "multilingual-e5-large" => {
            text_profile("intfloat/multilingual-e5-large", 1024, 512, Pooling::Mean)
        }
        "nomic-embed-text-v1.5" => {
            let mut profile =
                text_profile("nomic-ai/nomic-embed-text-v1.5", 768, 8192, Pooling::Mean);
            profile.truncation_dims = vec![512, 256, 128, 64];
            profile
        }
        "gte-base-en-v1.5" => text_profile("Alibaba-NLP/gte-base-en-v1.5", 768, 8192, Pooling::Cls),
        "gte-large-en-v1.5" => {
            text_profile("Alibaba-NLP/gte-large-en-v1.5", 1024, 8192, Pooling::Cls)
        }
        "mxbai-embed-large-v1" => {
            let mut profile = text_profile(
                "mixedbread-ai/mxbai-embed-large-v1",
                1024,
                512,
                Pooling::Cls,
            );
            profile.truncation_dims = vec![512];
            profile
        }
        "snowflake-arctic-embed-xs" => text_profile(
            "Snowflake/snowflake-arctic-embed-xs",
            384,
            512,
            Pooling::Cls,
        ),
        "snowflake-arctic-embed-s" => {
            text_profile("Snowflake/snowflake-arctic-embed-s", 384, 512, Pooling::Cls)
        }
        "snowflake-arctic-embed-m" => {
            text_profile("Snowflake/snowflake-arctic-embed-m", 768, 512, Pooling::Cls)
        }
        "snowflake-arctic-embed-l" => text_profile(
            "Snowflake/snowflake-arctic-embed-l",
            1024,
            512,
            Pooling::Cls,
        ),
        "all-minilm-l6-v2" => text_profile(
            "sentence-transformers/all-MiniLM-L6-v2",
            384,
            256,
            Pooling::Mean,
        ),
        "all-minilm-l12-v2" => text_profile(
            "sentence-transformers/all-MiniLM-L12-v2",
            384,
            128,
            Pooling::Mean,
        ),
        _ => {
            return Err(FrameworkError::param(format!(
                "SEARCH_EMBED_MODEL unknown profile {name}"
            )));
        }
    };
    if short.starts_with("bge-") && short != "bge-m3" {
        profile.query_prefix = RETRIEVAL_PREFIX.into();
    } else if short.starts_with("multilingual-e5-") {
        profile.query_prefix = "query: ".into();
        profile.document_template = "passage: {text}".into();
    } else if short == "nomic-embed-text-v1.5" {
        profile.query_prefix = "search_query: ".into();
        profile.document_template = "search_document: {text}".into();
    }
    profile.validate()?;
    Ok(profile)
}

/// Shared text defaults keep the fake and shipped profiles on the same validation boundary.
pub(crate) fn text_profile(
    id: &str,
    dimensions: usize,
    max_tokens: usize,
    pooling: Pooling,
) -> ModelProfile {
    ModelProfile {
        id: id.into(),
        revision: None,
        dimensions,
        max_tokens,
        query_prefix: String::new(),
        document_template: "{text}".into(),
        pooling,
        normalize: true,
        truncation_dims: vec![],
        inputs: InputKinds::default(),
        precision: Precision::Fp32,
        output_key: None,
        files: ModelFiles::default(),
    }
}
