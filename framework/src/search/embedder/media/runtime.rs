//! Encoder outputs and placeholder tokens must agree before the text graph runs.

use super::{
    AudioFeatures, ImageFeatures, error, preprocess_search_audio, preprocess_search_image,
    preprocess_search_video,
};
use crate::{EmbedInput, Embedding, FrameworkError, ModelProfile, validate_embedding};
use ort::{session::Session, value::Tensor};
use std::path::Path;

pub(in crate::search::embedder) enum PreparedPart {
    Image(ImageFeatures),
    Audio(AudioFeatures),
    Video(Vec<ImageFeatures>),
}

pub(in crate::search::embedder) async fn prepare(
    input: &EmbedInput,
) -> Result<Vec<PreparedPart>, FrameworkError> {
    let mut parts = Vec::with_capacity(input.media.len());
    for part in &input.media {
        let kind = part
            .media_type
            .split('/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        parts.push(match kind.as_str() {
            "image" => {
                let bytes = part.data.clone();
                PreparedPart::Image(
                    tokio::task::spawn_blocking(move || preprocess_search_image(&bytes, 280))
                        .await
                        .map_err(error)??,
                )
            }
            "audio" => {
                let bytes = part.data.clone();
                PreparedPart::Audio(
                    tokio::task::spawn_blocking(move || preprocess_search_audio(&bytes))
                        .await
                        .map_err(error)??,
                )
            }
            "video" => PreparedPart::Video(preprocess_search_video(&part.data).await?),
            _ => return Err(error(format!("unsupported media type {}", part.media_type))),
        });
    }
    Ok(parts)
}

pub(in crate::search::embedder) struct MediaRuntime {
    vision: Session,
    audio: Session,
    main: Session,
    tokens: Tokens,
}

struct Tokens {
    image: i64,
    audio: i64,
    video: i64,
    boi: i64,
    eoi: i64,
    boa: i64,
    eoa: i64,
}

impl MediaRuntime {
    pub(in crate::search::embedder) fn load(root: &Path) -> Result<Self, FrameworkError> {
        verify_processor(root)?;
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.join("config.json")).map_err(error)?)
                .map_err(error)?;
        let token = |name: &str| {
            config[name]
                .as_i64()
                .filter(|v| *v >= 0)
                .ok_or_else(|| error(format!("config.json missing {name}")))
        };
        let tokens = Tokens {
            image: token("image_token_id")?,
            audio: token("audio_token_id")?,
            video: token("video_token_id")?,
            boi: token("boi_token_id")?,
            eoi: token("eoi_token_id")?,
            boa: token("boa_token_id")?,
            eoa: token("eoa_token_index")?,
        };
        Ok(Self {
            vision: session(root, "vision_encoder")?,
            audio: session(root, "audio_encoder")?,
            main: session(root, "model")?,
            tokens,
        })
    }

    pub(in crate::search::embedder) fn embed(
        &mut self,
        text: &str,
        parts: Vec<PreparedPart>,
        tokenizer: &tokenizers::Tokenizer,
        profile: &ModelProfile,
        dimensions: Option<usize>,
    ) -> Result<Embedding, FrameworkError> {
        let encoding = tokenizer.encode(text, true).map_err(error)?;
        let mut ids: Vec<i64> = encoding.get_ids().iter().map(|v| *v as i64).collect();
        let mut images = Vec::new();
        let mut audio = Vec::new();
        let mut video = Vec::new();
        for part in parts {
            match part {
                PreparedPart::Image(image) => {
                    let count = image.soft_tokens;
                    let values = self.vision(image)?;
                    append_features(&mut images, values, count)?;
                    placeholders(
                        &mut ids,
                        self.tokens.boi,
                        self.tokens.image,
                        self.tokens.eoi,
                        count,
                    );
                }
                PreparedPart::Audio(features) => {
                    let frames = features.mask.len();
                    let count = features.mask.iter().step_by(4).filter(|v| **v != 0).count();
                    let output = self.audio.run(ort::inputs![
                        "input_features" => Tensor::from_array(([1,frames,128],features.features)).map_err(error)?,
                        "input_features_mask" => Tensor::from_array(([1,frames],features.mask.into_iter().map(|v| v!=0).collect::<Vec<_>>())).map_err(error)?,
                    ]).map_err(error)?;
                    let (shape, values) = output["audio_features"]
                        .try_extract_tensor::<f32>()
                        .map_err(error)?;
                    if shape.len() != 2 || shape[1] != 512 {
                        return Err(error("audio encoder output must have width 512"));
                    }
                    append_features(&mut audio, values.to_vec(), count)?;
                    placeholders(
                        &mut ids,
                        self.tokens.boa,
                        self.tokens.audio,
                        self.tokens.eoa,
                        count,
                    );
                }
                PreparedPart::Video(frames) => {
                    for frame in frames {
                        let count = frame.soft_tokens;
                        let values = self.vision(frame)?;
                        append_features(&mut video, values, count)?;
                        placeholders(
                            &mut ids,
                            self.tokens.boi,
                            self.tokens.video,
                            self.tokens.eoi,
                            count,
                        );
                    }
                }
            }
        }
        if ids.len() > profile.max_tokens {
            return Err(error(format!(
                "model {} media and text exceed {} tokens",
                profile.id, profile.max_tokens
            )));
        }
        let length = ids.len();
        let output = self.main.run(ort::inputs![
            "input_ids" => Tensor::from_array(([1,length],ids)).map_err(error)?,
            "attention_mask" => Tensor::from_array(([1,length],vec![1_i64;length])).map_err(error)?,
            "image_features" => Tensor::from_array(([images.len()/512,512],images)).map_err(error)?,
            "audio_features" => Tensor::from_array(([audio.len()/512,512],audio)).map_err(error)?,
            "video_features" => Tensor::from_array(([video.len()/512,512],video)).map_err(error)?,
        ]).map_err(error)?;
        let (shape, values) = output["last_hidden_state"]
            .try_extract_tensor::<f32>()
            .map_err(error)?;
        if shape.as_ref() != [1, length as i64, 768] {
            return Err(error("main graph output shape differs"));
        }
        let mut pooled = vec![0.0_f32; 768];
        for row in values.as_chunks::<768>().0 {
            for (total, value) in pooled.iter_mut().zip(row) {
                *total += value / length as f32;
            }
        }
        validate_embedding(pooled, profile, dimensions)
    }

    fn vision(&mut self, image: ImageFeatures) -> Result<Vec<f32>, FrameworkError> {
        let patches = image.position_ids.len() / 2;
        let output = self.vision.run(ort::inputs![
            "pixel_values" => Tensor::from_array(([1,patches,768],image.pixel_values)).map_err(error)?,
            "pixel_position_ids" => Tensor::from_array(([1,patches,2],image.position_ids)).map_err(error)?,
        ]).map_err(error)?;
        let (shape, values) = output["image_features"]
            .try_extract_tensor::<f32>()
            .map_err(error)?;
        if shape.len() != 2 || shape[1] != 512 {
            return Err(error("vision encoder output must have width 512"));
        }
        Ok(values.to_vec())
    }
}

fn append_features(
    target: &mut Vec<f32>,
    values: Vec<f32>,
    count: usize,
) -> Result<(), FrameworkError> {
    if count == 0 || values.len() != count * 512 || values.iter().any(|v| !v.is_finite()) {
        return Err(error(
            "encoder rows differ from media placeholder count or contain nonfinite values",
        ));
    }
    target.extend(values);
    Ok(())
}

fn placeholders(ids: &mut Vec<i64>, begin: i64, token: i64, end: i64, count: usize) {
    ids.push(begin);
    ids.extend(std::iter::repeat_n(token, count));
    ids.push(end);
}

fn session(root: &Path, name: &str) -> Result<Session, FrameworkError> {
    Session::builder()
        .map_err(error)?
        .with_intra_threads(4)
        .map_err(error)?
        .commit_from_file(root.join(format!("onnx/{name}.onnx")))
        .map_err(error)
}

fn verify_processor(root: &Path) -> Result<(), FrameworkError> {
    let config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("processor_config.json")).map_err(error)?)
            .map_err(error)?;
    for (pointer, expected) in [
        (
            "/processor_class",
            serde_json::json!("EmbeddingGemma2Processor"),
        ),
        ("/image_processor/patch_size", serde_json::json!(16)),
        ("/image_processor/pooling_kernel_size", serde_json::json!(3)),
        ("/image_processor/max_soft_tokens", serde_json::json!(280)),
        ("/image_processor/resample", serde_json::json!(3)),
        ("/image_processor/do_normalize", serde_json::json!(false)),
        ("/feature_extractor/sampling_rate", serde_json::json!(16000)),
        ("/feature_extractor/feature_size", serde_json::json!(128)),
        ("/feature_extractor/frame_length", serde_json::json!(320)),
        ("/feature_extractor/hop_length", serde_json::json!(160)),
        ("/feature_extractor/fft_length", serde_json::json!(512)),
        ("/feature_extractor/preemphasis", serde_json::json!(0.0)),
        ("/feature_extractor/dither", serde_json::json!(0.0)),
        ("/feature_extractor/mel_floor", serde_json::json!(0.001)),
        ("/video_processor/fps", serde_json::json!(1)),
        ("/video_processor/max_frames", serde_json::json!(32)),
        ("/video_processor/max_soft_tokens", serde_json::json!(140)),
    ] {
        if config.pointer(pointer) != Some(&expected) {
            return Err(error(format!(
                "unsupported processor_config.json {pointer}"
            )));
        }
    }
    for (section, budget) in [("image_processor", 280), ("video_processor", 140)] {
        let values = &config[section];
        if values["patch_size"] != 16
            || values["pooling_kernel_size"] != 3
            || values["max_soft_tokens"] != budget
            || values["do_normalize"] != false
            || values["resample"] != 3
            || values["rescale_factor"].as_f64() != Some(1.0 / 255.0)
        {
            return Err(error(format!(
                "unsupported processor_config.json {section}"
            )));
        }
    }
    let audio = &config["feature_extractor"];
    if audio["min_frequency"].as_f64() != Some(0.0)
        || audio["max_frequency"].as_f64() != Some(8000.0)
        || audio["input_scale_factor"].as_f64() != Some(1.0)
        || audio["per_bin_mean"] != serde_json::Value::Null
        || audio["per_bin_stddev"] != serde_json::Value::Null
    {
        return Err(error(
            "unsupported processor_config.json audio scaling or frequency range",
        ));
    }
    Ok(())
}
