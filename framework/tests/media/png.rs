#![cfg(feature = "media")]
//! Lossless PNG compression, colour metadata, and pipeline integration.

use std::time::{Duration, Instant};

use suprnova::{Image, PngEffort, recompress_png};

use crate::image_processing::{RED_PNG_1X1, decoded_rgba, photo_bmp};
use crate::images::png_with as with_chunks;

fn fixture(width: u32, height: u32, alpha: bool) -> Vec<u8> {
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&[
                x as u8,
                y as u8,
                (x / 8 + y / 8) as u8,
                if alpha { (x * 17 + y) as u8 } else { 255 },
            ]);
        }
    }
    let mut options = oxideav_png::EncodeOptions::default();
    options.filter_strategy = oxideav_png::FilterStrategy::Fixed(oxideav_png::FilterType::None);
    options.compression_level = Some(1);
    oxideav_png::encode_plane(
        width,
        height,
        oxideav_png::PngPixelFormat::Rgba,
        width as usize * 4,
        &pixels,
        None,
        &options,
    )
    .expect("fixture PNG")
}

fn chunk(png: &[u8], kind: &[u8; 4]) -> Option<Vec<u8>> {
    let mut pos = 8;
    loop {
        let (chunk, next) = oxideav_png::chunk::read_chunk(png, pos).expect("valid PNG chunk");
        if chunk.is_type(kind) {
            return Some(chunk.data.to_vec());
        }
        if chunk.is_type(b"IEND") {
            return None;
        }
        pos = next;
    }
}

#[tokio::test]
#[serial_test::serial]
async fn recompression_shrinks_and_preserves_every_pixel_and_transparency() {
    let source = with_chunks(
        &fixture(64, 48, true),
        &[
            (b"tEXt", b"Comment\0private upload".to_vec()),
            (
                b"zTXt",
                [
                    b"Comment\0\0".as_slice(),
                    &compcol::vec::compress_to_vec::<compcol::zlib::Zlib>(b"private text")
                        .expect("compressed text"),
                ]
                .concat(),
            ),
            (b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0private XMP".to_vec()),
            (b"tIME", vec![7, 234, 10, 22, 1, 2, 3]),
            (b"eXIf", b"private location".to_vec()),
        ],
    );
    let fast = recompress_png(&source, PngEffort::Fast).expect("fast recompression");
    let maximum = recompress_png(&source, PngEffort::Maximum).expect("maximum recompression");
    assert!(fast.len() < source.len());
    assert!(maximum.len() <= fast.len());
    let before = decoded_rgba(&source).await;
    assert!(before.2.chunks_exact(4).any(|pixel| pixel[3] != 255));
    for output in [&fast, &maximum] {
        assert_eq!(decoded_rgba(output).await, before);
        for kind in [b"tEXt", b"zTXt", b"iTXt", b"tIME", b"eXIf"] {
            assert!(chunk(output, kind).is_none(), "strips {kind:?}");
        }
    }
}

#[test]
#[serial_test::serial]
fn recompression_keeps_colour_chunks_and_the_icc_profile() {
    // sRGB profiles can be replaced by oxipng when stripping is enabled.
    let profile = moxcms::ColorProfile::new_srgb()
        .encode()
        .expect("sRGB profile");
    let mut iccp = b"fixture\0\0".to_vec();
    iccp.extend_from_slice(
        &compcol::vec::compress_to_vec::<compcol::zlib::Zlib>(&profile).expect("ICC zlib"),
    );
    let source = with_chunks(
        &fixture(16, 16, false),
        &[
            (b"iCCP", iccp),
            (b"gAMA", 45455_u32.to_be_bytes().to_vec()),
            (
                b"cHRM",
                [31270_u32, 32900, 64000, 33000, 30000, 60000, 15000, 6000]
                    .into_iter()
                    .flat_map(u32::to_be_bytes)
                    .collect(),
            ),
            (b"cICP", vec![1, 13, 0, 1]),
        ],
    );
    for effort in [PngEffort::Fast, PngEffort::Maximum] {
        let output = recompress_png(&source, effort).expect("profile recompression");
        let iccp = chunk(&output, b"iCCP").expect("ICC chunk stays");
        let start = iccp
            .iter()
            .position(|byte| *byte == 0)
            .expect("profile name terminator")
            + 2;
        let kept = compcol::vec::decompress_to_vec::<compcol::zlib::Zlib>(&iccp[start..])
            .expect("inflate ICC");
        assert_eq!(kept, profile);
        for kind in [b"gAMA", b"cHRM", b"cICP"] {
            assert_eq!(chunk(&output, kind), chunk(&source, kind));
        }
    }
    let srgb = with_chunks(RED_PNG_1X1, &[(b"sRGB", vec![0])]);
    assert_eq!(
        chunk(
            &recompress_png(&srgb, PngEffort::Fast).expect("sRGB recompression"),
            b"sRGB"
        ),
        Some(vec![0])
    );
}

#[tokio::test]
#[serial_test::serial]
async fn recompression_keeps_the_palette_and_its_transparency() {
    let mut source = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&4_u32.to_be_bytes());
    header.extend_from_slice(&1_u32.to_be_bytes());
    header.extend_from_slice(&[8, 3, 0, 0, 0]);
    oxideav_png::chunk::write_chunk(&mut source, b"IHDR", &header);
    oxideav_png::chunk::write_chunk(&mut source, b"PLTE", &[255, 0, 0, 0, 255, 0, 0, 0, 255]);
    oxideav_png::chunk::write_chunk(&mut source, b"tRNS", &[0, 128, 255]);
    let data = compcol::vec::compress_to_vec::<compcol::zlib::Zlib>(&[0, 0, 1, 2, 1])
        .expect("palette zlib");
    oxideav_png::chunk::write_chunk(&mut source, b"IDAT", &data);
    oxideav_png::chunk::write_chunk(&mut source, b"IEND", &[]);
    let before = decoded_rgba(&source).await;
    for effort in [PngEffort::Fast, PngEffort::Maximum] {
        let output = recompress_png(&source, effort).expect("palette recompression");
        assert_eq!(chunk(&output, b"PLTE"), chunk(&source, b"PLTE"));
        assert_eq!(decoded_rgba(&output).await, before);
    }
}

#[test]
#[serial_test::serial]
fn an_optimal_png_never_grows_and_metadata_never_returns() {
    let optimal = recompress_png(RED_PNG_1X1, PngEffort::Maximum).expect("optimize tiny PNG");
    for effort in [PngEffort::Fast, PngEffort::Maximum] {
        assert!(recompress_png(&optimal, effort).expect("optimal PNG").len() <= optimal.len());
        let tagged = with_chunks(&optimal, &[(b"tEXt", b"GPS\0private".to_vec())]);
        let output = recompress_png(&tagged, effort).expect("strip tiny PNG");
        assert!(output.len() <= tagged.len());
        assert!(chunk(&output, b"tEXt").is_none());
    }
}

#[test]
#[serial_test::serial]
fn recompression_refuses_other_formats_and_malformed_png() {
    let error = recompress_png(&photo_bmp(), PngEffort::Fast)
        .expect_err("BMP is not PNG")
        .to_string();
    assert!(error.contains("image/bmp"), "{error}");
    assert!(recompress_png(b"not an image", PngEffort::Fast).is_err());
    assert!(recompress_png(&RED_PNG_1X1[..40], PngEffort::Fast).is_err());
}

#[tokio::test]
#[serial_test::serial]
async fn pipeline_recompression_runs_after_transforms_and_refuses_non_png_output() {
    let image = Image::from_bytes(fixture(64, 48, true))
        .to_png()
        .resize(32, 24);
    let plain = image.clone().to_bytes().await.expect("plain pipeline");
    for effort in [PngEffort::Fast, PngEffort::Maximum] {
        let output = image
            .clone()
            .recompress_png(effort)
            .quality(1)
            .to_bytes()
            .await
            .expect("recompressed pipeline");
        assert_eq!(output, recompress_png(&plain, effort).expect("same helper"));
        assert_eq!(decoded_rgba(&output).await, decoded_rgba(&plain).await);
    }
    let inferred = Image::from_bytes(RED_PNG_1X1)
        .recompress_png(PngEffort::Fast)
        .to_bytes()
        .await
        .expect("inferred PNG");
    assert!(inferred.starts_with(b"\x89PNG"));
    let error = image
        .to_jpeg()
        .recompress_png(PngEffort::Fast)
        .to_bytes()
        .await
        .expect_err("JPEG output refused")
        .to_string();
    assert!(error.contains("image/jpeg"), "{error}");
}

#[test]
#[serial_test::serial]
fn maximum_recompression_of_512_square_finishes_within_timeout() {
    let source = fixture(512, 512, false);
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let started = Instant::now();
        let output = recompress_png(&source, PngEffort::Maximum);
        send.send((output, started.elapsed(), source.len()))
            .expect("send timing");
    });
    let (output, elapsed, original_len) = receive
        .recv_timeout(Duration::from_secs(120))
        .expect("maximum finishes within 120 seconds");
    assert!(output.expect("maximum PNG").len() <= original_len);
    eprintln!("Maximum 512x512 opaque PNG: {elapsed:?}");
}

#[tokio::test]
#[serial_test::serial]
async fn pipeline_recompression_keeps_the_orientation_opt_out() {
    let mut config = suprnova::ImageConfig::default();
    config.auto_orient = false;
    let _guard = crate::image_processing::ConfigGuard::set(config);
    let tiff = vec![
        b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
    ];
    let source = with_chunks(&fixture(7, 5, false), &[(b"eXIf", tiff)]);
    let image = Image::from_bytes(source.clone()).to_png();
    let plain = image.clone().to_bytes().await.expect("orientation opt-out");
    let output = image
        .recompress_png(PngEffort::Fast)
        .to_bytes()
        .await
        .expect("recompressed opt-out");
    assert!(chunk(&plain, b"eXIf").is_some());
    assert_eq!(chunk(&output, b"eXIf"), chunk(&plain, b"eXIf"));
    assert_eq!(decoded_rgba(&output).await, decoded_rgba(&plain).await);
    let raw = recompress_png(&source, PngEffort::Fast).expect("raw recompression");
    assert!(chunk(&raw, b"eXIf").is_none());
    assert_eq!(decoded_rgba(&raw).await, decoded_rgba(&source).await);
}

#[test]
#[serial_test::serial]
fn recompression_checks_configured_limits_before_optimization() {
    let mut config = suprnova::ImageConfig::default();
    config.max_dimension = 1;
    let _guard = crate::image_processing::ConfigGuard::set(config);
    let error = recompress_png(&fixture(16, 16, false), PngEffort::Fast)
        .expect_err("over the dimension cap")
        .to_string();
    assert!(error.contains("IMAGE_MAX_DIMENSION"), "{error}");
}

#[test]
#[serial_test::serial]
fn malformed_animation_chunks_return_errors_without_panicking() {
    for (kind, lengths) in [
        (b"fcTL", vec![0, 3, 25, 27]),
        (b"fdAT", vec![0, 3]),
        (b"acTL", vec![0, 7, 9]),
    ] {
        for length in lengths {
            let source = with_chunks(RED_PNG_1X1, &[(kind, vec![0; length])]);
            let result = std::panic::catch_unwind(|| recompress_png(&source, PngEffort::Fast));
            assert!(result.is_ok(), "{kind:?} length {length} must not panic");
            assert!(
                result.expect("no panic").is_err(),
                "malformed {kind:?} length {length}"
            );
        }
    }
}

#[test]
#[serial_test::serial]
fn animation_frames_must_fit_the_canvas_before_optimization() {
    for (width, height, x, y) in [
        (0_u32, 1_u32, 0_u32, 0_u32),
        (2, 1, 0, 0),
        (1, 2, 0, 0),
        (1, 1, 1, 0),
        (1, 1, 0, 1),
        (1, 1, u32::MAX, 0),
        (u32::MAX, u32::MAX, 0, 0),
    ] {
        let mut control = Vec::new();
        for number in [0, width, height, x, y] {
            control.extend_from_slice(&number.to_be_bytes());
        }
        control.extend_from_slice(&[0, 1, 0, 10, 0, 0]);
        let source = with_chunks(RED_PNG_1X1, &[(b"fcTL", control)]);
        let error = recompress_png(&source, PngEffort::Fast)
            .expect_err("invalid frame geometry")
            .to_string();
        assert!(error.contains("frame") || error.contains("fcTL"), "{error}");
    }
}

#[test]
#[serial_test::serial]
fn recompression_preserves_animation_frames_and_delays() {
    use oxideav_png::apng::{Actl, Fctl};
    let first = Fctl::new(0, 1, 1).with_delay(1, 10).to_bytes().to_vec();
    let source = with_chunks(
        RED_PNG_1X1,
        &[
            (b"acTL", Actl::new(2, 0).to_bytes().to_vec()),
            (b"fcTL", first),
        ],
    );
    let mut animation = source[..source.len() - 12].to_vec();
    oxideav_png::chunk::write_chunk(
        &mut animation,
        b"fcTL",
        &Fctl::new(1, 1, 1).with_delay(2, 10).to_bytes(),
    );
    let mut second = 2_u32.to_be_bytes().to_vec();
    second.extend_from_slice(
        &compcol::vec::compress_to_vec::<compcol::zlib::Zlib>(&[0, 0, 255, 0])
            .expect("green frame zlib"),
    );
    oxideav_png::chunk::write_chunk(&mut animation, b"fdAT", &second);
    oxideav_png::chunk::write_chunk(&mut animation, b"IEND", &[]);
    let decode = |bytes: &[u8]| {
        oxideav_png::decode_all(bytes)
            .expect("decode animation")
            .into_iter()
            .map(|frame| (frame.delay, frame.image.to_rgba8()))
            .collect::<Vec<_>>()
    };
    let before = decode(&animation);
    assert_eq!(before.len(), 2);
    assert_ne!(before[0].1, before[1].1);
    for effort in [PngEffort::Fast, PngEffort::Maximum] {
        let output = recompress_png(&animation, effort).expect("recompress animation");
        assert_eq!(decode(&output), before);
        assert!(output.len() <= animation.len());
    }
}

#[test]
#[serial_test::serial]
fn duplicate_image_headers_are_refused_before_optimization() {
    let source = with_chunks(RED_PNG_1X1, &[(b"IHDR", RED_PNG_1X1[16..29].to_vec())]);
    let error = recompress_png(&source, PngEffort::Fast)
        .expect_err("duplicate IHDR is not a valid PNG")
        .to_string();
    assert!(error.contains("IHDR"), "{error}");
}
