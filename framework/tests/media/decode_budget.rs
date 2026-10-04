#![cfg(feature = "media")]
//! Input whose header passes the decode limits but whose data asks for more:
//! PNG pixel data that inflates past what its header declares, GIF frames
//! beyond the first, and file sources whose reported size is not their real
//! size. The header gate cannot see any of these, so each test proves the
//! refusal or the bound holds while the data is read and decoded.
//!
//! `#[serial]` for the same reason as `image_processing`: the limit tests
//! install a process-global `ImageConfig` override.

use oxideav_gif::{Block, DisposalMethod, Frame, GifImage, GraphicControl, Rgb, Version};
use oxideav_png::{PngEncoderOptions, PngImage, PngPixelFormat};
use suprnova::{Image, ImageConfig, OutputFormat};

use crate::image_processing::ConfigGuard;

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// The chunk payloads of type `kind` in a PNG file, in file order.
fn png_chunks(png: &[u8], kind: &[u8; 4]) -> Vec<Vec<u8>> {
    let mut found = Vec::new();
    let mut pos = PNG_SIGNATURE.len();
    while pos < png.len() {
        let (chunk, next) = oxideav_png::chunk::read_chunk(png, pos).expect("a well-formed chunk");
        if chunk.is_type(kind) {
            found.push(chunk.data.to_vec());
        }
        pos = next;
    }
    found
}

/// A PNG whose header declares 1x1 RGBA but whose IDAT is the pixel data of a
/// `width x height` RGBA image: small on disk, small to the header gate, and
/// `height * (1 + width * 4)` bytes once inflated.
fn png_declaring_one_pixel(width: u32, height: u32) -> Vec<u8> {
    let stride = width as usize * 4;
    let large = oxideav_png::encode_png_image(&PngImage {
        width,
        height,
        pixel_format: PngPixelFormat::Rgba,
        stride,
        data: vec![0u8; stride * height as usize],
        palette: Vec::new(),
    })
    .expect("the large image encodes");
    let idat = png_chunks(&large, b"IDAT").concat();

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&1u32.to_be_bytes());
    ihdr.extend_from_slice(&1u32.to_be_bytes());
    // Bit depth 8, colour type 6 (RGBA), compression, filter, no interlace.
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);

    let mut png = PNG_SIGNATURE.to_vec();
    oxideav_png::chunk::write_chunk(&mut png, b"IHDR", &ihdr);
    oxideav_png::chunk::write_chunk(&mut png, b"IDAT", &idat);
    oxideav_png::chunk::write_chunk(&mut png, b"IEND", &[]);
    png
}

#[tokio::test]
#[serial_test::serial]
async fn a_png_whose_pixel_data_inflates_past_its_header_is_refused() {
    // 512 x (1 + 1024 * 4) bytes: about 2 MiB behind a header that declares
    // five. The file itself is a few kilobytes.
    let png = png_declaring_one_pixel(1024, 512);
    assert!(png.len() < 64 * 1024, "the fixture is {} bytes", png.len());

    let err = Image::from_bytes(png)
        .to_format(OutputFormat::Bmp)
        .to_bytes()
        .await
        .expect_err("pixel data past the declared size must be refused");
    let message = err.to_string();
    assert!(message.contains("inflates past"), "got: {message}");
    assert!(
        message.contains("1x1"),
        "the error names the declared size: {message}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn interlaced_palette_and_sixteen_bit_pngs_still_decode() {
    // The inflate bound is computed from the header for every colour type,
    // bit depth and interlace mode, so each one must still decode at exactly
    // its declared size. Odd sizes make Adam7 skip passes and leave partial
    // ones, which is where a wrong pass size would show.
    let formats: [(PngPixelFormat, usize); 8] = [
        (PngPixelFormat::Gray8, 1),
        (PngPixelFormat::Gray16Le, 2),
        (PngPixelFormat::Rgb24, 3),
        (PngPixelFormat::Rgb48Le, 6),
        (PngPixelFormat::Pal8, 1),
        (PngPixelFormat::Ya8, 2),
        (PngPixelFormat::Rgba, 4),
        (PngPixelFormat::Rgba64Le, 8),
    ];
    for (pixel_format, bytes_per_pixel) in formats {
        for interlace in [false, true] {
            for (width, height) in [(1u32, 1u32), (3, 3), (13, 7), (9, 17)] {
                let stride = width as usize * bytes_per_pixel;
                let palette = if pixel_format == PngPixelFormat::Pal8 {
                    vec![255, 0, 0]
                } else {
                    Vec::new()
                };
                let png = oxideav_png::encode_png_image_with_options(
                    &PngImage {
                        width,
                        height,
                        pixel_format,
                        stride,
                        data: vec![0u8; stride * height as usize],
                        palette,
                    },
                    &PngEncoderOptions {
                        interlace,
                        ..PngEncoderOptions::default()
                    },
                )
                .expect("the fixture encodes");
                let dimensions = Image::from_bytes(png).dimensions().await.unwrap_or_else(|e| {
                    panic!(
                        "{pixel_format:?} {width}x{height} (interlaced: {interlace}) must decode: {e}"
                    )
                });
                assert_eq!(dimensions, (width, height));
            }
        }
    }
}

/// A four-colour GIF89a on a `width x height` logical screen.
fn gif(width: u16, height: u16, frames: Vec<Frame>) -> Vec<u8> {
    oxideav_gif::encode(&GifImage {
        version: Version::Gif89a,
        screen_width: width,
        screen_height: height,
        color_resolution: 7,
        global_palette_sorted: false,
        background_index: 0,
        pixel_aspect_ratio: 0,
        global_palette: Some(vec![
            Rgb::new(255, 0, 0),
            Rgb::new(0, 255, 0),
            Rgb::new(0, 0, 255),
            Rgb::new(0, 0, 0),
        ]),
        blocks: frames.into_iter().map(Block::Image).collect(),
    })
    .expect("the GIF encodes")
}

/// A solid frame of palette entry `index`, kept on the canvas after it shows.
fn frame(left: u16, top: u16, width: u16, height: u16, index: u8) -> Frame {
    Frame {
        left,
        top,
        width,
        height,
        local_palette: None,
        palette_sorted: false,
        interlaced: false,
        indices: vec![index; usize::from(width) * usize::from(height)],
        graphic_control: Some(GraphicControl {
            disposal: DisposalMethod::Keep,
            user_input: false,
            transparent_index: None,
            delay_centis: 10,
        }),
    }
}

#[tokio::test]
#[serial_test::serial]
async fn an_animated_gif_decodes_as_its_first_frame() {
    // A red first frame, then fifty green 1x1 frames over its corner. All
    // frames together need 51 canvases; the budget below holds five, and the
    // pipeline only ever uses the first one.
    let mut frames = vec![frame(0, 0, 256, 256, 0)];
    frames.extend((0..50).map(|_| frame(0, 0, 1, 1, 1)));
    let animation = gif(256, 256, frames);
    let _config = ConfigGuard::set(ImageConfig {
        max_alloc_bytes: 5 * 256 * 256 * 4,
        ..ImageConfig::default()
    });

    let image = Image::from_bytes(animation);
    assert_eq!(
        image
            .clone()
            .dimensions()
            .await
            .expect("the first frame decodes"),
        (256, 256)
    );
    assert_eq!(
        image
            .dominant_color()
            .await
            .expect("the first frame decodes"),
        "#ff0000",
        "the first frame, not a later one, is the image"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn a_gif_whose_first_frame_escapes_its_screen_is_refused() {
    // The logical screen passes the gate at 16x16; the frame inside it is
    // 64x64, and its pixels would be decoded before the frame is placed.
    let malformed = gif(16, 16, vec![frame(0, 0, 64, 64, 0)]);
    let err = Image::from_bytes(malformed)
        .dimensions()
        .await
        .expect_err("a frame larger than its screen must be refused");
    let message = err.to_string();
    assert!(message.contains("logical screen"), "got: {message}");
    assert!(message.contains("64x64"), "got: {message}");
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial]
async fn a_file_source_is_read_no_further_than_the_cap() {
    use std::io::Write;
    use std::time::Duration;

    // A FIFO reports a size of zero, so the size check before the read passes
    // it, and only the read itself can hold the cap. The writer counts what
    // the reader accepted: a reader that stops at the cap closes the pipe and
    // the writer's next write fails.
    const CAP: u64 = 64 * 1024;
    const OFFERED: usize = 16 * 1024 * 1024;

    let dir = tempfile::tempdir().expect("a directory");
    let fifo = dir.path().join("endless.png");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("run mkfifo");
    assert!(made.success(), "mkfifo failed");

    let (sent_tx, sent_rx) = std::sync::mpsc::channel();
    let writer_path = fifo.clone();
    std::thread::spawn(move || {
        let mut sent = 0usize;
        if let Ok(mut pipe) = std::fs::OpenOptions::new().write(true).open(&writer_path) {
            let mut chunk = vec![0u8; 64 * 1024];
            chunk[..PNG_SIGNATURE.len()].copy_from_slice(PNG_SIGNATURE);
            while sent < OFFERED {
                match pipe.write(&chunk) {
                    Ok(written) => sent += written,
                    Err(_) => break,
                }
            }
        }
        let _ = sent_tx.send(sent);
    });

    let _config = ConfigGuard::set(ImageConfig {
        max_alloc_bytes: CAP,
        ..ImageConfig::default()
    });
    let err = Image::from_path(&fifo)
        .to_bytes()
        .await
        .expect_err("the source is larger than the cap");
    assert!(err.to_string().contains("limit"), "got: {err}");

    let sent = sent_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("the writer finishes once the reader is gone");
    assert!(
        sent < OFFERED,
        "the reader took all {sent} bytes offered instead of stopping past the {CAP}-byte cap"
    );
}

#[cfg(feature = "testing")]
#[tokio::test]
#[serial_test::serial]
async fn a_stored_file_over_the_cap_is_refused() {
    use suprnova::DiskExt;

    let _guard = suprnova::Storage::fake();
    suprnova::Storage::register_memory("budget");
    let disk = suprnova::Storage::disk("budget").expect("disk");
    let mut oversized = PNG_SIGNATURE.to_vec();
    oversized.resize(256 * 1024, 0);
    disk.put("big.png", oversized).await.expect("seed");

    let _config = ConfigGuard::set(ImageConfig {
        max_alloc_bytes: 64 * 1024,
        ..ImageConfig::default()
    });
    let err = Image::from_disk("budget", "big.png")
        .to_bytes()
        .await
        .expect_err("the stored file is larger than the cap");
    assert!(err.to_string().contains("limit"), "got: {err}");
}

/// A storage layer whose `stat` reports no content length, as a service that
/// omits it does. OpenDAL then reports a size of zero.
#[cfg(feature = "testing")]
mod unknown_length {
    use std::sync::Arc;

    use opendal::raw::{
        Layer, OpCopier, OpCopy, OpCreateDir, OpList, OpPresign, OpRead, OpRename, OpStat, OpWrite,
        RpCreateDir, RpPresign, RpRename, RpStat, Service, ServiceInfo, Servicer, oio,
    };
    use opendal::{Capability, Metadata, OperationContext, Result};

    #[derive(Debug, Clone, Copy)]
    pub(super) struct UnknownLength;

    impl Layer for UnknownLength {
        fn apply_service(&self, inner: Servicer) -> Servicer {
            Arc::new(UnknownLengthService { inner })
        }
    }

    #[derive(Debug)]
    struct UnknownLengthService {
        inner: Servicer,
    }

    impl Service for UnknownLengthService {
        type Reader = oio::Reader;
        type Writer = oio::Writer;
        type Lister = oio::Lister;
        type Deleter = oio::Deleter;
        type Copier = oio::Copier;

        fn info(&self) -> ServiceInfo {
            self.inner.info()
        }

        fn capability(&self) -> Capability {
            self.inner.capability()
        }

        async fn create_dir(
            &self,
            ctx: &OperationContext,
            path: &str,
            args: OpCreateDir,
        ) -> Result<RpCreateDir> {
            self.inner.create_dir(ctx, path, args).await
        }

        async fn stat(&self, ctx: &OperationContext, path: &str, args: OpStat) -> Result<RpStat> {
            let mode = self
                .inner
                .stat(ctx, path, args)
                .await?
                .into_metadata()
                .mode();
            Ok(RpStat::new(Metadata::new(mode)))
        }

        fn read(&self, ctx: &OperationContext, path: &str, args: OpRead) -> Result<Self::Reader> {
            self.inner.read(ctx, path, args)
        }

        fn write(&self, ctx: &OperationContext, path: &str, args: OpWrite) -> Result<Self::Writer> {
            self.inner.write(ctx, path, args)
        }

        fn delete(&self, ctx: &OperationContext) -> Result<Self::Deleter> {
            self.inner.delete(ctx)
        }

        fn list(&self, ctx: &OperationContext, path: &str, args: OpList) -> Result<Self::Lister> {
            self.inner.list(ctx, path, args)
        }

        fn copy(
            &self,
            ctx: &OperationContext,
            from: &str,
            to: &str,
            args: OpCopy,
            opts: OpCopier,
        ) -> Result<Self::Copier> {
            self.inner.copy(ctx, from, to, args, opts)
        }

        async fn rename(
            &self,
            ctx: &OperationContext,
            from: &str,
            to: &str,
            args: OpRename,
        ) -> Result<RpRename> {
            self.inner.rename(ctx, from, to, args).await
        }

        async fn presign(
            &self,
            ctx: &OperationContext,
            path: &str,
            args: OpPresign,
        ) -> Result<RpPresign> {
            self.inner.presign(ctx, path, args).await
        }
    }
}

#[cfg(feature = "testing")]
#[tokio::test]
#[serial_test::serial]
async fn a_stored_file_of_unknown_length_is_read_up_to_the_cap() {
    use suprnova::DiskExt;

    let _guard = suprnova::Storage::fake();
    suprnova::Storage::register_memory_with("unknown-length", |operator| {
        operator.layer(unknown_length::UnknownLength)
    });
    let disk = suprnova::Storage::disk("unknown-length").expect("disk");
    let png = Image::from_bytes(crate::image_processing::RED_PNG_1X1)
        .resize(6, 4)
        .to_format(OutputFormat::Png)
        .to_bytes()
        .await
        .expect("fixture build");
    disk.put("image.png", png).await.expect("seed");
    assert_eq!(
        disk.size("image.png").await.expect("stat"),
        0,
        "the fixture only means something if the size is unknown"
    );

    // A size of zero is "unknown", not "empty": the object still reads.
    let dimensions = Image::from_disk("unknown-length", "image.png")
        .dimensions()
        .await
        .expect("an object whose length is unknown still decodes");
    assert_eq!(dimensions, (6, 4));

    // And the read still stops at the cap.
    let mut oversized = PNG_SIGNATURE.to_vec();
    oversized.resize(256 * 1024, 0);
    disk.put("big.png", oversized).await.expect("seed");
    let _config = ConfigGuard::set(ImageConfig {
        max_alloc_bytes: 64 * 1024,
        ..ImageConfig::default()
    });
    let err = Image::from_disk("unknown-length", "big.png")
        .to_bytes()
        .await
        .expect_err("the stored file is larger than the cap");
    assert!(err.to_string().contains("limit"), "got: {err}");
}
