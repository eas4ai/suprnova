use std::io::Read;
use suprnova::{preprocess_search_audio, preprocess_search_image};

fn reference(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/tests/search/fixtures/media/{name}.gz",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut output = Vec::new();
    flate2::read::GzDecoder::new(std::fs::File::open(path).unwrap())
        .read_to_end(&mut output)
        .unwrap();
    output
}

fn compare(name: &str, actual: &[f32]) {
    let metadata: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/media/reference.json")).unwrap();
    let tolerance = metadata["fixtures"][name]["absolute_tolerance"]
        .as_f64()
        .unwrap();
    let bytes = reference(name);
    let expected: Vec<_> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect();
    assert_eq!(actual.len(), expected.len(), "{name}");
    let maximum = actual
        .iter()
        .zip(expected)
        .map(|(a, b)| (*a as f64 - b as f64).abs())
        .fold(0.0, f64::max);
    assert!(
        maximum <= tolerance,
        "{name}: maximum error {maximum} exceeds {tolerance}"
    );
}

#[test]
fn rec_006_media_image_matches_reference() {
    for (budget, prefix) in [(280, "image"), (140, "video_frame")] {
        let output =
            preprocess_search_image(include_bytes!("fixtures/media/shapes.png"), budget).unwrap();
        compare(&format!("{prefix}_pixels"), &output.pixel_values);
        let positions: Vec<_> = reference(&format!("{prefix}_positions"))
            .as_chunks::<8>()
            .0
            .iter()
            .map(|b| i64::from_le_bytes(*b))
            .collect();
        assert_eq!(output.position_ids, positions);
        assert_eq!(
            output.soft_tokens as i64,
            i64::from_le_bytes(reference(&format!("{prefix}_tokens")).try_into().unwrap())
        );
    }
}

#[test]
fn rec_006_media_audio_matches_reference() {
    for (name, bytes) in [
        (
            "tones_16000",
            include_bytes!("fixtures/media/tones_16000.wav").as_slice(),
        ),
        (
            "tones_44100",
            include_bytes!("fixtures/media/tones_44100.wav").as_slice(),
        ),
    ] {
        let output = preprocess_search_audio(bytes).unwrap();
        compare(&format!("{name}_features"), &output.features);
        assert_eq!(output.mask, reference(&format!("{name}_mask")));
    }
}

#[test]
fn rec_006_media_malformed_inputs_fail() {
    for bytes in [b"".as_slice(), b"invalid", b"RIFF\0\0\0\0WAVE"] {
        assert!(preprocess_search_image(bytes, 280).is_err());
        assert!(preprocess_search_audio(bytes).is_err());
    }
    assert!(preprocess_search_image(include_bytes!("fixtures/media/shapes.png"), 0).is_err());
}

#[cfg(feature = "testing")]
#[tokio::test]
#[serial_test::serial(search_media_process)]
async fn rec_006_media_video_matches_reference() {
    let fake = suprnova::Process::fake();
    fake.when("*", |pending: &suprnova::PendingProcess| {
        let args = pending.arguments().unwrap();
        if args.iter().any(|arg| arg == "-hide_banner") {
            return suprnova::Process::result("")
                .error_output("Duration: 00:00:03.00, start: 0.0")
                .exit_code(1);
        }
        assert!(args.iter().any(|arg| arg == "matroska,webm,mov,avi,mpegts"));
        let pattern = std::path::Path::new(args.last().unwrap());
        for index in 1..=3 {
            std::fs::write(
                pattern.parent().unwrap().join(format!("{index:02}.png")),
                include_bytes!("fixtures/media/shapes.png"),
            )
            .unwrap();
        }
        suprnova::Process::result("")
    });
    let output = suprnova::preprocess_search_video(include_bytes!("fixtures/media/shapes.mkv"))
        .await
        .unwrap();
    assert_eq!(output.len(), 3);
    let pixels = output
        .iter()
        .flat_map(|frame| frame.pixel_values.iter().copied())
        .collect::<Vec<_>>();
    compare("video_pixels", &pixels);
    let positions = output
        .iter()
        .flat_map(|frame| frame.position_ids.iter().copied())
        .collect::<Vec<_>>();
    let expected = reference("video_positions")
        .as_chunks::<8>()
        .0
        .iter()
        .map(|bytes| i64::from_le_bytes(*bytes))
        .collect::<Vec<_>>();
    assert_eq!(positions, expected);
}

#[tokio::test]
#[serial_test::serial(search_media_process)]
async fn rec_006_media_video_malformed_input_fails() {
    assert!(suprnova::preprocess_search_video(b"invalid").await.is_err());
}

#[cfg(feature = "testing")]
mod declarations {
    use std::sync::Arc;
    use suprnova::{FakeEmbedder, SearchConfig, SearchDriver, SearchRegistry, Searchable};

    fn media_settings() -> suprnova::IndexSettings {
        let mut settings = suprnova::IndexSettings::default();
        settings.index_time.embedder = Some("media-test".into());
        settings
    }

    #[suprnova::model(table = "media_test_clips", timestamps = false)]
    #[derive(Searchable)]
    #[searchable(index = "media_test_clips", settings = "media_settings")]
    pub struct Clip {
        pub id: i64,
        pub caption: String,
        #[searchable(media(kind = "audio", disk = "media-test"))]
        pub sound: String,
        #[searchable(media(kind = "image", disk = "media-test"))]
        pub poster: String,
    }

    #[tokio::test]
    async fn rec_006_media_text_profile_fails_sync_naming_index_and_model() {
        SearchConfig::scope(
            SearchConfig {
                driver: SearchDriver::Collection,
                ..Default::default()
            },
            SearchRegistry::isolated(async {
                let profile = suprnova::default_profile();
                let model = profile.id.clone();
                SearchRegistry::register_embedder(
                    "media-test",
                    Arc::new(FakeEmbedder::with_profile(profile).unwrap()),
                )
                .unwrap();
                let error = suprnova::search::generations::sync_settings("media_test_clips")
                    .await
                    .unwrap_err()
                    .to_string();
                assert!(
                    error.contains("media_test_clips") && error.contains(&model),
                    "{error}"
                );
            }),
        )
        .await
        .unwrap();
    }

    #[cfg(feature = "filesystem")]
    #[tokio::test]
    async fn rec_006_media_storage_parts_reach_embedder_in_order_and_are_bounded() {
        use suprnova::{EmbedInput, Embedder, Embedding, FrameworkError, MediaPart, ModelProfile};
        struct Recording {
            profile: ModelProfile,
            seen: std::sync::Mutex<Vec<MediaPart>>,
        }
        #[suprnova::async_trait]
        impl Embedder for Recording {
            fn profile(&self) -> &ModelProfile {
                &self.profile
            }
            async fn embed_documents(
                &self,
                inputs: &[EmbedInput],
            ) -> Result<Vec<Embedding>, FrameworkError> {
                self.profile.document_input(&inputs[0])?;
                *self.seen.lock().unwrap() = inputs[0].media.clone();
                FakeEmbedder::with_profile(self.profile.clone())?
                    .embed_documents(inputs)
                    .await
            }
            async fn embed_query(&self, text: &str) -> Result<Embedding, FrameworkError> {
                FakeEmbedder::with_profile(self.profile.clone())?
                    .embed_query(text)
                    .await
            }
        }
        let _storage = suprnova::Storage::fake();
        suprnova::Storage::register_memory("media-test");
        let disk = suprnova::Storage::disk("media-test").unwrap();
        disk.write("sound.wav", b"sound".to_vec()).await.unwrap();
        disk.write("poster.png", b"poster".to_vec()).await.unwrap();
        let mut attributes = std::collections::BTreeMap::new();
        attributes.insert(
            "sound".into(),
            suprnova::AttributeValue::Text("sound.wav".into()),
        );
        attributes.insert(
            "poster".into(),
            suprnova::AttributeValue::Text("poster.png".into()),
        );
        let document = suprnova::SearchDocument {
            index: "media_test_clips".into(),
            key: "1".into(),
            source_version: 0,
            attributes,
            embed_text: Some("caption".into()),
            title: None,
        };
        let sources = Clip::descriptor().index_settings().index_time.embed_media;
        let media = suprnova::load_search_media(&document, &sources, 16)
            .await
            .unwrap();
        assert!(
            suprnova::load_search_media(&document, &sources, 4)
                .await
                .is_err()
        );
        let mut profile = suprnova::default_profile();
        profile.inputs.audio = true;
        profile.inputs.image = true;
        let recording = Arc::new(Recording {
            profile,
            seen: Default::default(),
        });
        recording
            .embed_documents(&[EmbedInput {
                text: "caption".into(),
                media,
                ..Default::default()
            }])
            .await
            .unwrap();
        let _db = super::super::derive::database().await;
        use sea_orm::ConnectionTrait;
        suprnova::DB::connection().unwrap().inner().execute_unprepared(
            "CREATE TABLE media_test_clips (id INTEGER PRIMARY KEY, caption TEXT NOT NULL, sound TEXT NOT NULL, poster TEXT NOT NULL); INSERT INTO media_test_clips VALUES (1, 'caption', 'sound.wav', 'poster.png')"
        ).await.unwrap();
        recording.seen.lock().unwrap().clear();
        SearchConfig::scope(
            SearchConfig {
                driver: SearchDriver::Collection,
                queue: false,
                ..Default::default()
            },
            SearchRegistry::isolated(async {
                SearchRegistry::register_embedder("media-test", recording.clone()).unwrap();
                suprnova::searchable::<Clip>(&["1".into()]).await.unwrap();
            }),
        )
        .await
        .unwrap();
        let seen = recording.seen.lock().unwrap();
        assert_eq!(
            seen.iter()
                .map(|part| part.media_type.as_str())
                .collect::<Vec<_>>(),
            ["audio/wav", "image/png"]
        );
        assert_eq!(seen[0].data, b"sound");
        assert_eq!(seen[1].data, b"poster");
    }

    #[cfg(feature = "filesystem")]
    #[tokio::test]
    async fn rec_006_media_a_file_replaced_at_the_same_path_is_embedded_again() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use suprnova::{EmbedInput, Embedder, Embedding, FrameworkError, MediaPart, ModelProfile};
        struct Recording {
            profile: ModelProfile,
            calls: AtomicUsize,
            seen: std::sync::Mutex<Vec<MediaPart>>,
        }
        #[suprnova::async_trait]
        impl Embedder for Recording {
            fn profile(&self) -> &ModelProfile {
                &self.profile
            }
            async fn embed_documents(
                &self,
                inputs: &[EmbedInput],
            ) -> Result<Vec<Embedding>, FrameworkError> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                *self.seen.lock().unwrap() = inputs[0].media.clone();
                FakeEmbedder::with_profile(self.profile.clone())?
                    .embed_documents(inputs)
                    .await
            }
            async fn embed_query(&self, text: &str) -> Result<Embedding, FrameworkError> {
                FakeEmbedder::with_profile(self.profile.clone())?
                    .embed_query(text)
                    .await
            }
        }
        let _storage = suprnova::Storage::fake();
        suprnova::Storage::register_memory("media-test");
        let disk = suprnova::Storage::disk("media-test").unwrap();
        disk.write("sound.wav", b"first take".to_vec())
            .await
            .unwrap();
        disk.write("poster.png", b"poster".to_vec()).await.unwrap();
        let mut profile = suprnova::default_profile();
        profile.inputs.audio = true;
        profile.inputs.image = true;
        let recording = Arc::new(Recording {
            profile,
            calls: AtomicUsize::new(0),
            seen: Default::default(),
        });
        let _db = super::super::derive::database().await;
        use sea_orm::ConnectionTrait;
        suprnova::DB::connection().unwrap().inner().execute_unprepared(
            "CREATE TABLE media_test_clips (id INTEGER PRIMARY KEY, caption TEXT NOT NULL, sound TEXT NOT NULL, poster TEXT NOT NULL); INSERT INTO media_test_clips VALUES (1, 'caption', 'sound.wav', 'poster.png')"
        ).await.unwrap();
        SearchConfig::scope(
            SearchConfig {
                driver: SearchDriver::Collection,
                queue: false,
                ..Default::default()
            },
            SearchRegistry::isolated(async {
                SearchRegistry::register_embedder("media-test", recording.clone()).unwrap();
                suprnova::searchable::<Clip>(&["1".into()]).await.unwrap();
                assert_eq!(recording.calls.load(Ordering::SeqCst), 1);
                // An unchanged row over unchanged files still skips the embedder.
                suprnova::searchable::<Clip>(&["1".into()]).await.unwrap();
                assert_eq!(recording.calls.load(Ordering::SeqCst), 1);
                // The row and its paths stay the same while the stored bytes change.
                disk.write("sound.wav", b"second take".to_vec())
                    .await
                    .unwrap();
                suprnova::searchable::<Clip>(&["1".into()]).await.unwrap();
                assert_eq!(recording.calls.load(Ordering::SeqCst), 2);
                assert_eq!(recording.seen.lock().unwrap()[0].data, b"second take");
            }),
        )
        .await
        .unwrap();
    }

    #[test]
    fn rec_006_media_declarations_preserve_order() {
        let parts = Clip::descriptor().index_settings().index_time.embed_media;
        assert_eq!(
            parts
                .iter()
                .map(|part| part.field.as_str())
                .collect::<Vec<_>>(),
            ["sound", "poster"]
        );
    }
}

#[cfg(feature = "search-fastembed")]
#[tokio::test]
#[ignore = "requires the external EmbeddingGemma 2 media cache and ffmpeg"]
async fn real_media_profile_embeds_image_audio_and_video() {
    use suprnova::{EmbedInput, Embedder, FastembedConfig, FastembedEmbedder, MediaPart};
    let cache = std::env::var("SEARCH_MODEL_CACHE").expect("SEARCH_MODEL_CACHE is required");
    let embedder = FastembedEmbedder::new(FastembedConfig {
        model: format!("path:{cache}/source"),
        cache: cache.into(),
        profile: Some(suprnova::embeddinggemma_media_profile().unwrap()),
        ..Default::default()
    })
    .await
    .unwrap();
    let input = |kind: &str, bytes: &[u8]| EmbedInput {
        text: String::new(),
        media: vec![MediaPart {
            media_type: kind.into(),
            data: bytes.to_vec(),
        }],
        ..Default::default()
    };
    let image = input("image/png", include_bytes!("fixtures/media/shapes.png"));
    let audio = input(
        "audio/wav",
        include_bytes!("fixtures/media/tones_16000.wav"),
    );
    let shifted = input(
        "audio/wav",
        include_bytes!("fixtures/media/tones_shifted.wav"),
    );
    let other_audio = input(
        "audio/wav",
        include_bytes!("fixtures/media/tones_other.wav"),
    );
    let video = input(
        "video/x-matroska",
        include_bytes!("fixtures/media/shapes.mkv"),
    );
    let unrelated = input("image/png", include_bytes!("fixtures/media/unrelated.png"));
    let inputs = [
        image.clone(),
        audio.clone(),
        shifted,
        other_audio,
        video,
        unrelated,
    ];
    let vectors = embedder.embed_documents(&inputs).await.unwrap();
    for vector in &vectors {
        assert_eq!(vector.dense.len(), 768);
        assert!(vector.dense.iter().all(|v| v.is_finite()));
        let norm = vector.dense.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm {norm}");
    }
    let repeated = embedder.embed_documents(&inputs).await.unwrap();
    for (first, second) in vectors.iter().zip(&repeated) {
        assert_eq!(first.dense, second.dense);
    }
    let text = embedder
        .embed_query("A red rectangle and a blue circle on a white background.")
        .await
        .unwrap();
    let other = embedder
        .embed_query("A financial report about interest rates and bond markets.")
        .await
        .unwrap();
    let dot = |a: &suprnova::Embedding, b: &suprnova::Embedding| {
        a.dense
            .iter()
            .zip(&b.dense)
            .map(|(a, b)| a * b)
            .sum::<f32>()
    };
    println!(
        "image similarity: shapes={} unrelated={}",
        dot(&vectors[0], &text),
        dot(&vectors[0], &other)
    );
    println!(
        "audio similarity: shifted={} different={}",
        dot(&vectors[1], &vectors[2]),
        dot(&vectors[1], &vectors[3])
    );
    println!(
        "video similarity: frames={} unrelated={}",
        dot(&vectors[4], &vectors[0]),
        dot(&vectors[4], &vectors[5])
    );
    assert!(dot(&vectors[0], &text) > dot(&vectors[0], &other));
    assert!(dot(&vectors[1], &vectors[2]) > dot(&vectors[1], &vectors[3]));
    assert!(dot(&vectors[4], &vectors[0]) > dot(&vectors[4], &vectors[5]));
}

#[test]
fn rec_006_media_profile_accepts_all_kinds() {
    let profile = suprnova::embeddinggemma_media_profile().unwrap();
    assert!(profile.inputs.image && profile.inputs.audio && profile.inputs.video);
    assert!(!suprnova::default_profile().inputs.audio);
}

#[tokio::test]
#[serial_test::serial(search_media_process)]
async fn rec_006_media_missing_video_binary_names_configuration() {
    let error = suprnova::preprocess_search_video_with_binary(
        include_bytes!("fixtures/media/shapes.mkv"),
        "/does-not-exist/lane-media-ffmpeg",
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("SEARCH_FFMPEG_BINARY"));
}

#[test]
fn rec_006_media_image_codecs_match_reference() {
    for (name, bytes) in [
        (
            "jpg_pixels",
            include_bytes!("fixtures/media/shapes.jpg").as_slice(),
        ),
        (
            "webp_pixels",
            include_bytes!("fixtures/media/shapes.webp").as_slice(),
        ),
    ] {
        let output = preprocess_search_image(bytes, 280).unwrap();
        compare(name, &output.pixel_values);
    }
}
