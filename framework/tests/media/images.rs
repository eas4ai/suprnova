#![cfg(feature = "media")]
//! The image specification (`docs/spec/images.md`, IMG-001 to IMG-008):
//! orientation, metadata, driver choice, custom transformations, storage,
//! rotation backgrounds, the Laravel terminals, and the API's shape.
//!
//! Each test is named for the requirement it proves. The `magick` half of a
//! requirement runs the host's ImageMagick 7, so it is `#[ignore]`d for the
//! gate and run by the `images` mechanism; without the binary it fails,
//! never skips.
//!
//! Every test is `#[serial]`: several install a process-global
//! `ImageConfig` override, and a sibling running beside one would decode
//! under it.

use std::sync::Mutex;

use serial_test::serial;
use suprnova::{
    Color, Image, ImageConfig, ImageDriver, ImageDriverKind, ImagePipeline, ImagePixels,
    MagickCliDriver, OutputFormat, OxideAvImageDriver, Transformation,
};

use crate::image_processing::ConfigGuard;

// ───────────────────────── fixtures ─────────────────────────

const WIDTH: u32 = 7;
const HEIGHT: u32 = 5;

/// Packed RGBA in which every pixel has its own colour, so any permutation
/// of the pixels shows.
fn pattern(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for y in 0..height {
        for x in 0..width {
            out.extend_from_slice(&[
                (10 + x * 30) as u8,
                (5 + y * 40) as u8,
                ((x + y) * 15 + 3) as u8,
                255,
            ]);
        }
    }
    out
}

/// One colour, `width x height`.
fn flat(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    rgba.repeat((width * height) as usize)
}

fn png_of(width: u32, height: u32, rgba: Vec<u8>) -> Vec<u8> {
    oxideav_png::encode_plane(
        width,
        height,
        oxideav_png::PngPixelFormat::Rgba,
        width as usize * 4,
        &rgba,
        None,
        &oxideav_png::EncodeOptions::default(),
    )
    .expect("the fixture PNG encodes")
}

/// A one-channel greyscale PNG.
fn grey_png(width: u32, height: u32, level: u8) -> Vec<u8> {
    oxideav_png::encode_plane(
        width,
        height,
        oxideav_png::PngPixelFormat::Gray8,
        width as usize,
        &vec![level; (width * height) as usize],
        None,
        &oxideav_png::EncodeOptions::default(),
    )
    .expect("the fixture PNG encodes")
}

/// `source` re-encoded by the built-in driver, under the default config.
fn convert(source: &[u8], format: OutputFormat, quality: u8) -> Vec<u8> {
    OxideAvImageDriver::new()
        .process(
            source,
            &ImagePipeline {
                format: Some(format),
                quality,
                ..ImagePipeline::default()
            },
        )
        .expect("the fixture converts")
}

/// A big-endian TIFF holding an `Orientation` tag, and a GPS IFD with a
/// latitude reference when `gps` is set: what a phone writes, in small.
fn exif_tiff(orientation: Option<u16>, gps: bool) -> Vec<u8> {
    let mut entries: Vec<[u8; 12]> = Vec::new();
    if let Some(value) = orientation {
        let mut entry = [0u8; 12];
        entry[..2].copy_from_slice(&0x0112u16.to_be_bytes());
        entry[2..4].copy_from_slice(&3u16.to_be_bytes());
        entry[4..8].copy_from_slice(&1u32.to_be_bytes());
        entry[8..10].copy_from_slice(&value.to_be_bytes());
        entries.push(entry);
    }
    let ifd_len = 2 + 12 * (entries.len() + usize::from(gps)) + 4;
    let gps_offset = 8 + ifd_len;
    if gps {
        let mut entry = [0u8; 12];
        entry[..2].copy_from_slice(&0x8825u16.to_be_bytes());
        entry[2..4].copy_from_slice(&4u16.to_be_bytes());
        entry[4..8].copy_from_slice(&1u32.to_be_bytes());
        entry[8..12].copy_from_slice(&(gps_offset as u32).to_be_bytes());
        entries.push(entry);
    }
    let mut tiff = vec![b'M', b'M', 0, 42, 0, 0, 0, 8];
    tiff.extend_from_slice(&(entries.len() as u16).to_be_bytes());
    for entry in &entries {
        tiff.extend_from_slice(entry);
    }
    tiff.extend_from_slice(&[0, 0, 0, 0]);
    if gps {
        // GPSLatitudeRef, ASCII, two bytes: "N\0".
        tiff.extend_from_slice(&1u16.to_be_bytes());
        tiff.extend_from_slice(&1u16.to_be_bytes());
        tiff.extend_from_slice(&2u16.to_be_bytes());
        tiff.extend_from_slice(&2u32.to_be_bytes());
        tiff.extend_from_slice(b"N\0\0\0");
        tiff.extend_from_slice(&[0, 0, 0, 0]);
    }
    tiff
}

/// A JPEG segment, marker and body.
fn segment(marker: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![0xFF, marker];
    out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(body);
    out
}

/// `jpeg` with `segments` inserted after its start-of-image marker.
fn jpeg_with(jpeg: &[u8], segments: &[Vec<u8>]) -> Vec<u8> {
    let mut out = jpeg[..2].to_vec();
    for segment in segments {
        out.extend_from_slice(segment);
    }
    out.extend_from_slice(&jpeg[2..]);
    out
}

fn exif_segment(tiff: &[u8]) -> Vec<u8> {
    segment(0xE1, &[&b"Exif\0\0"[..], tiff].concat())
}

/// The APP2 segments of an ICC profile, in 60,000-byte chunks.
fn icc_segments(profile: &[u8]) -> Vec<Vec<u8>> {
    let chunks: Vec<&[u8]> = profile.chunks(60_000).collect();
    chunks
        .iter()
        .enumerate()
        .map(|(index, chunk)| {
            segment(
                0xE2,
                &[
                    &b"ICC_PROFILE\0"[..],
                    &[index as u8 + 1, chunks.len() as u8],
                    chunk,
                ]
                .concat(),
            )
        })
        .collect()
}

/// `png` with `chunks` inserted after its header chunk.
fn png_with(png: &[u8], chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    // Signature (8) and IHDR (12 + 13).
    let mut out = png[..33].to_vec();
    for (kind, data) in chunks {
        oxideav_png::chunk::write_chunk(&mut out, kind, data);
    }
    out.extend_from_slice(&png[33..]);
    out
}

/// An `iCCP` chunk's data for `profile`.
fn iccp(profile: &[u8]) -> Vec<u8> {
    let mut data = b"fixture\0\0".to_vec();
    data.extend_from_slice(
        &compcol::vec::compress_to_vec::<compcol::zlib::Zlib>(profile).expect("zlib"),
    );
    data
}

/// A `tEXt` chunk's data.
fn text(keyword: &str, value: &str) -> Vec<u8> {
    [keyword.as_bytes(), b"\0", value.as_bytes()].concat()
}

/// A RIFF chunk: fourcc, size, payload, padding.
fn riff_chunk(fourcc: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = fourcc.to_vec();
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    out
}

/// A simple lossless WebP (one `VP8L` chunk) rebuilt in the extended
/// layout with `icc` before the image and `after` chunks after it.
fn webp_with(webp: &[u8], icc: Option<&[u8]>, after: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let size = u32::from_le_bytes(webp[16..20].try_into().unwrap()) as usize;
    let vp8l = &webp[12..20 + size + (size & 1)];
    let (width, height, alpha) = match &webp[12..16] {
        b"VP8L" => {
            let bits = u32::from_le_bytes(webp[21..25].try_into().unwrap());
            (
                (bits & 0x3FFF) + 1,
                ((bits >> 14) & 0x3FFF) + 1,
                bits & (1 << 28) != 0,
            )
        }
        b"VP8 " => (
            u32::from(u16::from_le_bytes([webp[26], webp[27]]) & 0x3FFF),
            u32::from(u16::from_le_bytes([webp[28], webp[29]]) & 0x3FFF),
            false,
        ),
        other => panic!("the fixture must be a simple WebP, not {other:?}"),
    };
    let mut flags = 0u8;
    if icc.is_some() {
        flags |= 0x20;
    }
    if alpha {
        flags |= 0x10;
    }
    for (fourcc, _) in after {
        match *fourcc {
            b"EXIF" => flags |= 0x08,
            b"XMP " => flags |= 0x04,
            _ => {}
        }
    }
    let mut header = [0u8; 10];
    header[0] = flags;
    header[4..7].copy_from_slice(&(width - 1).to_le_bytes()[..3]);
    header[7..10].copy_from_slice(&(height - 1).to_le_bytes()[..3]);
    let mut body = b"WEBP".to_vec();
    body.extend_from_slice(&riff_chunk(b"VP8X", &header));
    if let Some(profile) = icc {
        body.extend_from_slice(&riff_chunk(b"ICCP", profile));
    }
    body.extend_from_slice(vp8l);
    for (fourcc, data) in after {
        body.extend_from_slice(&riff_chunk(fourcc, data));
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// The JPEG, PNG and WebP forms of `rgba`, each carrying the EXIF
/// `Orientation` tag `orientation`.
fn oriented_sources(
    rgba: &[u8],
    width: u32,
    height: u32,
    orientation: u16,
) -> Vec<(&'static str, Vec<u8>)> {
    let png = png_of(width, height, rgba.to_vec());
    let tiff = exif_tiff(Some(orientation), false);
    let jpeg = convert(&png, OutputFormat::Jpeg, 100);
    let webp = convert(&png, OutputFormat::WebPLossless, 70);
    let lossy = convert(&png, OutputFormat::WebP, 90);
    vec![
        ("JPEG", jpeg_with(&jpeg, &[exif_segment(&tiff)])),
        ("PNG", png_with(&png, &[(b"eXIf", tiff.clone())])),
        ("WebP", webp_with(&webp, None, &[(b"EXIF", tiff.clone())])),
        ("lossy WebP", webp_with(&lossy, None, &[(b"EXIF", tiff)])),
    ]
}

/// A profile padded to a multiple of four bytes, its header's size field
/// updated: ICC requires the padding, and ImageMagick's lcms refuses a
/// profile without it, which moxcms's encoder leaves out.
fn padded(mut profile: Vec<u8>) -> Vec<u8> {
    profile.resize(profile.len().next_multiple_of(4), 0);
    let size = (profile.len() as u32).to_be_bytes();
    profile[..4].copy_from_slice(&size);
    profile
}

fn display_p3() -> Vec<u8> {
    padded(
        moxcms::ColorProfile::new_display_p3()
            .encode()
            .expect("moxcms encodes Display P3"),
    )
}

fn grey_gamma_profile() -> Vec<u8> {
    padded(
        moxcms::ColorProfile::new_gray_with_gamma(1.8)
            .encode()
            .expect("moxcms encodes a grey profile"),
    )
}

// ───────────────────────── reading output ─────────────────────────

/// Width, height and packed RGBA of a PNG.
fn png_pixels(png: &[u8]) -> (u32, u32, Vec<u8>) {
    let bitmap = oxideav_png::decode_rgba8(png).expect("the output PNG decodes");
    (bitmap.width, bitmap.height, bitmap.data)
}

/// The marker segments of a JPEG before its first scan.
fn jpeg_segments(jpeg: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut out = Vec::new();
    let mut pos = 2;
    while pos + 4 <= jpeg.len() {
        assert_eq!(jpeg[pos], 0xFF, "a marker at {pos}");
        let marker = jpeg[pos + 1];
        if marker == 0xDA {
            break;
        }
        let length = u16::from_be_bytes([jpeg[pos + 2], jpeg[pos + 3]]) as usize;
        out.push((marker, jpeg[pos + 4..pos + 2 + length].to_vec()));
        pos += 2 + length;
    }
    out
}

/// The chunks of a PNG, type and data.
fn png_chunks(png: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    let mut out = Vec::new();
    let mut pos = 8;
    while pos + 12 <= png.len() {
        let length = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
        let kind: [u8; 4] = png[pos + 4..pos + 8].try_into().unwrap();
        out.push((kind, png[pos + 8..pos + 8 + length].to_vec()));
        pos += 12 + length;
    }
    out
}

/// The top-level chunks of a WebP, fourcc and payload.
fn webp_chunks(webp: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    let mut out = Vec::new();
    let mut pos = 12;
    while pos + 8 <= webp.len() {
        let fourcc: [u8; 4] = webp[pos..pos + 4].try_into().unwrap();
        let size = u32::from_le_bytes(webp[pos + 4..pos + 8].try_into().unwrap()) as usize;
        out.push((
            fourcc,
            webp[pos + 8..(pos + 8 + size).min(webp.len())].to_vec(),
        ));
        pos += 8 + size + (size & 1);
    }
    out
}

/// The extensions of a GIF, as label and first sub-block.
fn gif_extensions(gif: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let table = |packed: u8| {
        if packed & 0x80 != 0 {
            3usize << ((packed & 7) + 1)
        } else {
            0
        }
    };
    let skip = |mut pos: usize| {
        loop {
            let size = gif[pos] as usize;
            pos += 1;
            if size == 0 {
                return pos;
            }
            pos += size;
        }
    };
    let mut out = Vec::new();
    let mut pos = 13 + table(gif[10]);
    loop {
        match gif[pos] {
            0x3B => return out,
            0x21 => {
                let size = gif[pos + 2] as usize;
                out.push((gif[pos + 1], gif[pos + 3..pos + 3 + size].to_vec()));
                pos = skip(pos + 2);
            }
            0x2C => pos = skip(pos + 10 + table(gif[pos + 9]) + 1),
            other => panic!("unexpected GIF block {other:#x}"),
        }
    }
}

/// The ICC profile in encoded output.
fn output_profile(format: OutputFormat, bytes: &[u8]) -> Option<Vec<u8>> {
    match format {
        OutputFormat::Jpeg => {
            let mut chunks: Vec<(u8, Vec<u8>)> = jpeg_segments(bytes)
                .into_iter()
                .filter(|(marker, body)| *marker == 0xE2 && body.starts_with(b"ICC_PROFILE\0"))
                .map(|(_, body)| (body[12], body[14..].to_vec()))
                .collect();
            chunks.sort();
            (!chunks.is_empty()).then(|| chunks.into_iter().flat_map(|(_, data)| data).collect())
        }
        OutputFormat::Png => png_chunks(bytes)
            .into_iter()
            .find(|(kind, _)| kind == b"iCCP")
            .map(|(_, data)| {
                let name = data.iter().position(|&b| b == 0).unwrap();
                compcol::vec::decompress_to_vec::<compcol::zlib::Zlib>(&data[name + 2..])
                    .expect("the iCCP inflates")
            }),
        OutputFormat::WebP | OutputFormat::WebPLossless => webp_chunks(bytes)
            .into_iter()
            .find(|(fourcc, _)| fourcc == b"ICCP")
            .map(|(_, data)| data),
        OutputFormat::Bmp => {
            let header = u32::from_le_bytes(bytes[14..18].try_into().unwrap());
            let cs = u32::from_le_bytes(bytes[14 + 56..14 + 60].try_into().unwrap());
            (header >= 124 && cs == 0x4D42_4544).then(|| {
                let offset =
                    14 + u32::from_le_bytes(bytes[14 + 112..14 + 116].try_into().unwrap()) as usize;
                let size =
                    u32::from_le_bytes(bytes[14 + 116..14 + 120].try_into().unwrap()) as usize;
                bytes[offset..offset + size].to_vec()
            })
        }
        // Only whether a GIF carries the ICC extension: its identifier.
        OutputFormat::Gif => gif_extensions(bytes)
            .into_iter()
            .find(|(label, first)| *label == 0xFF && first == b"ICCRGBG1012")
            .map(|(_, first)| first),
    }
}

/// The colour space of the pixels encoded output stores.
fn pixel_space(format: OutputFormat, bytes: &[u8]) -> [u8; 4] {
    match format {
        OutputFormat::Jpeg => {
            let components = jpeg_segments(bytes)
                .into_iter()
                .find(|(marker, _)| matches!(marker, 0xC0..=0xC3))
                .map(|(_, body)| body[5])
                .expect("a frame header");
            match components {
                1 => *b"GRAY",
                4 => *b"CMYK",
                _ => *b"RGB ",
            }
        }
        OutputFormat::Png => match bytes[25] {
            0 | 4 => *b"GRAY",
            _ => *b"RGB ",
        },
        _ => *b"RGB ",
    }
}

/// The tags in the first IFD of TIFF-structured EXIF.
fn exif_tags(tiff: &[u8]) -> Vec<u16> {
    let big = tiff.starts_with(b"MM");
    let u16_at = |at: usize| {
        let bytes = [tiff[at], tiff[at + 1]];
        if big {
            u16::from_be_bytes(bytes)
        } else {
            u16::from_le_bytes(bytes)
        }
    };
    let u32_at = |at: usize| {
        let bytes: [u8; 4] = tiff[at..at + 4].try_into().unwrap();
        if big {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        }
    };
    let ifd = u32_at(4) as usize;
    (0..u16_at(ifd) as usize)
        .map(|entry| u16_at(ifd + 2 + entry * 12))
        .collect()
}

/// The colour chunks of a PNG (`cHRM`, `gAMA`, `sRGB`, `cICP`), in file
/// order.
fn colour_chunks_of(png: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    png_chunks(png)
        .into_iter()
        .filter(|(kind, _)| matches!(kind, b"cHRM" | b"gAMA" | b"sRGB" | b"cICP"))
        .collect()
}

/// The EXIF in encoded output, as TIFF bytes.
fn output_exif(format: OutputFormat, bytes: &[u8]) -> Option<Vec<u8>> {
    match format {
        OutputFormat::Jpeg => jpeg_segments(bytes)
            .into_iter()
            .find(|(marker, body)| *marker == 0xE1 && body.starts_with(b"Exif\0\0"))
            .map(|(_, body)| body[6..].to_vec()),
        OutputFormat::Png => png_chunks(bytes)
            .into_iter()
            .find(|(kind, _)| kind == b"eXIf")
            .map(|(_, data)| data),
        OutputFormat::WebP | OutputFormat::WebPLossless => webp_chunks(bytes)
            .into_iter()
            .find(|(fourcc, _)| fourcc == b"EXIF")
            .map(|(_, data)| {
                data.strip_prefix(b"Exif\0\0")
                    .map(<[u8]>::to_vec)
                    .unwrap_or(data)
            }),
        _ => None,
    }
}

/// Every container in encoded output that IMG-002 forbids, by name. The
/// Orientation-only EXIF is reported as `orientation-only EXIF` so a
/// caller can allow exactly that.
fn metadata_found(format: OutputFormat, bytes: &[u8]) -> Vec<String> {
    let mut found = Vec::new();
    let exif = |tiff: &[u8], found: &mut Vec<String>| {
        if exif_tags(tiff) == [0x0112] {
            found.push("orientation-only EXIF".to_string());
        } else {
            found.push("EXIF".to_string());
        }
    };
    match format {
        OutputFormat::Jpeg => {
            for (marker, body) in jpeg_segments(bytes) {
                match marker {
                    0xE1 if body.starts_with(b"Exif\0\0") => exif(&body[6..], &mut found),
                    0xE1 => found.push("APP1 (XMP)".into()),
                    0xED => found.push("APP13 (IPTC)".into()),
                    0xFE => found.push("COM".into()),
                    _ => {}
                }
            }
        }
        OutputFormat::Png => {
            for (kind, data) in png_chunks(bytes) {
                match &kind {
                    b"eXIf" => exif(&data, &mut found),
                    b"tEXt" | b"zTXt" | b"iTXt" | b"tIME" => {
                        found.push(String::from_utf8_lossy(&kind).into_owned())
                    }
                    _ => {}
                }
            }
        }
        OutputFormat::WebP | OutputFormat::WebPLossless => {
            for (fourcc, data) in webp_chunks(bytes) {
                match &fourcc {
                    b"EXIF" => exif(data.strip_prefix(b"Exif\0\0").unwrap_or(&data), &mut found),
                    b"XMP " => found.push("XMP".into()),
                    _ => {}
                }
            }
        }
        OutputFormat::Gif => {
            for (label, first) in gif_extensions(bytes) {
                match label {
                    0xFE => found.push("GIF comment".into()),
                    0xFF if first != b"NETSCAPE2.0" => found.push(format!(
                        "GIF application {}",
                        String::from_utf8_lossy(&first)
                    )),
                    _ => {}
                }
            }
        }
        _ => {}
    }
    found
}

// ───────────────────────── drivers ─────────────────────────

fn oxideav() -> Box<dyn ImageDriver> {
    Box::new(OxideAvImageDriver::new())
}

fn magick() -> Box<dyn ImageDriver> {
    Box::new(MagickCliDriver::from_env())
}

fn pipeline(steps: Vec<Transformation>, format: OutputFormat) -> ImagePipeline {
    ImagePipeline {
        transformations: steps,
        format: Some(format),
        ..ImagePipeline::default()
    }
}

/// The config with orientation on decode turned off.
fn opt_out() -> ConfigGuard {
    let mut config = ImageConfig::default();
    config.auto_orient = false;
    ConfigGuard::set(config)
}

/// `rgba` turned the way EXIF orientation `tag` says to display it, built
/// from a clockwise quarter turn and the two mirrors (an implementation
/// independent of the drivers').
fn turned(rgba: &[u8], width: u32, height: u32, tag: u16) -> (u32, u32, Vec<u8>) {
    let px = |data: &[u8], w: u32, x: u32, y: u32| {
        let at = ((y * w + x) * 4) as usize;
        data[at..at + 4].to_vec()
    };
    let clockwise = |(w, h, data): (u32, u32, Vec<u8>)| {
        let mut out = Vec::new();
        for y in 0..w {
            for x in 0..h {
                out.extend(px(&data, w, y, h - 1 - x));
            }
        }
        (h, w, out)
    };
    let mirror = |(w, h, data): (u32, u32, Vec<u8>)| {
        let mut out = Vec::new();
        for y in 0..h {
            for x in 0..w {
                out.extend(px(&data, w, w - 1 - x, y));
            }
        }
        (w, h, out)
    };
    let flip = |(w, h, data): (u32, u32, Vec<u8>)| {
        let mut out = Vec::new();
        for y in 0..h {
            for x in 0..w {
                out.extend(px(&data, w, x, h - 1 - y));
            }
        }
        (w, h, out)
    };
    let source = (width, height, rgba.to_vec());
    match tag {
        1 => source,
        2 => mirror(source),
        3 => clockwise(clockwise(source)),
        4 => flip(source),
        5 => mirror(clockwise(source)),
        6 => clockwise(source),
        7 => flip(clockwise(source)),
        8 => clockwise(clockwise(clockwise(source))),
        _ => panic!("no orientation {tag}"),
    }
}

// ───────────────────────── IMG-001 ─────────────────────────

/// The lossless JPEG fixtures, colour and grey, each carrying the EXIF
/// `Orientation` tag `orientation`: a frame oxideav-mjpeg decodes rather
/// than zune-jpeg.
fn oriented_lossless_jpegs(orientation: u16) -> Vec<(&'static str, Vec<u8>)> {
    let tag = exif_segment(&exif_tiff(Some(orientation), false));
    vec![
        (
            "lossless JPEG",
            jpeg_with(
                include_bytes!("fixtures/jpeg/photo-lossless-rgb-35x21.jpg"),
                std::slice::from_ref(&tag),
            ),
        ),
        (
            "lossless grey JPEG",
            jpeg_with(
                include_bytes!("fixtures/jpeg/photo-lossless-grey-35x21.jpg"),
                &[tag],
            ),
        ),
    ]
}

/// Every orientation of a JPEG (baseline and lossless), a PNG and a WebP
/// comes out as the exact permutation of the pixels the driver decodes
/// with orientation off.
fn assert_every_orientation_is_applied(driver: &dyn ImageDriver) {
    for tag in 1..=8u16 {
        let sources = oriented_sources(&pattern(WIDTH, HEIGHT), WIDTH, HEIGHT, tag)
            .into_iter()
            .chain(oriented_lossless_jpegs(tag));
        for (name, source) in sources {
            let sensor = {
                let _config = opt_out();
                driver
                    .process(&source, &pipeline(vec![], OutputFormat::Png))
                    .unwrap_or_else(|e| panic!("{name} {tag}, unoriented: {e}"))
            };
            let upright = driver
                .process(&source, &pipeline(vec![], OutputFormat::Png))
                .unwrap_or_else(|e| panic!("{name} {tag}: {e}"));
            let (width, height, sensor) = png_pixels(&sensor);
            assert_eq!(
                png_pixels(&upright),
                turned(&sensor, width, height, tag),
                "{name} with orientation {tag} through {}",
                driver.name()
            );
        }
    }
}

#[test]
#[serial]
fn img_001_every_orientation_is_applied_by_oxideav() {
    assert_every_orientation_is_applied(oxideav().as_ref());
}

#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_001_every_orientation_is_applied_by_magick() {
    assert_every_orientation_is_applied(magick().as_ref());
}

#[test]
#[serial]
fn img_001_a_jpeg_with_two_exif_segments_is_oriented_by_the_last() {
    let rgba = pattern(WIDTH, HEIGHT);
    let jpeg = convert(&png_of(WIDTH, HEIGHT, rgba), OutputFormat::Jpeg, 100);
    let two = jpeg_with(
        &jpeg,
        &[
            exif_segment(&exif_tiff(Some(3), false)),
            exif_segment(&exif_tiff(Some(6), false)),
        ],
    );
    let driver = OxideAvImageDriver::new();
    let sensor = {
        let _config = opt_out();
        driver
            .process(&two, &pipeline(vec![], OutputFormat::Png))
            .unwrap()
    };
    let upright = driver
        .process(&two, &pipeline(vec![], OutputFormat::Png))
        .unwrap();
    let (width, height, sensor) = png_pixels(&sensor);
    assert_eq!(png_pixels(&upright), turned(&sensor, width, height, 6));
}

/// `orient()` applies the tag under the opt-out, and never turns an image
/// its decode already turned.
fn assert_orient_applies_the_tag_once(driver: &dyn ImageDriver) {
    for (name, source) in oriented_sources(&pattern(WIDTH, HEIGHT), WIDTH, HEIGHT, 6) {
        let upright = driver
            .process(&source, &pipeline(vec![], OutputFormat::Png))
            .unwrap();
        let oriented_twice = driver
            .process(
                &source,
                &pipeline(vec![Transformation::Orient], OutputFormat::Png),
            )
            .unwrap();
        assert_eq!(
            png_pixels(&oriented_twice),
            png_pixels(&upright),
            "{name}: orient() turned an image its decode already turned ({})",
            driver.name()
        );
        let _config = opt_out();
        let sensor = driver
            .process(&source, &pipeline(vec![], OutputFormat::Png))
            .unwrap();
        let oriented = driver
            .process(
                &source,
                &pipeline(vec![Transformation::Orient], OutputFormat::Png),
            )
            .unwrap();
        let (width, height, sensor) = png_pixels(&sensor);
        assert_eq!(
            png_pixels(&oriented),
            turned(&sensor, width, height, 6),
            "{name}: orient() under the opt-out ({})",
            driver.name()
        );
    }
}

#[test]
#[serial]
fn img_001_orient_applies_the_tag_once_under_oxideav() {
    assert_orient_applies_the_tag_once(oxideav().as_ref());
}

#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_001_orient_applies_the_tag_once_under_magick() {
    assert_orient_applies_the_tag_once(magick().as_ref());
}

/// Under the opt-out a lossless source comes out with the very pixels it
/// went in with.
fn assert_the_opt_out_keeps_the_sensor_pixels(driver: &dyn ImageDriver) {
    let rgba = pattern(WIDTH, HEIGHT);
    let png = png_with(
        &png_of(WIDTH, HEIGHT, rgba.clone()),
        &[(b"eXIf", exif_tiff(Some(8), false))],
    );
    let _config = opt_out();
    let out = driver
        .process(&png, &pipeline(vec![], OutputFormat::Png))
        .unwrap();
    assert_eq!(png_pixels(&out), (WIDTH, HEIGHT, rgba), "{}", driver.name());
}

#[test]
#[serial]
fn img_001_the_opt_out_keeps_the_sensor_pixels_under_oxideav() {
    assert_the_opt_out_keeps_the_sensor_pixels(oxideav().as_ref());
}

#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_001_the_opt_out_keeps_the_sensor_pixels_under_magick() {
    assert_the_opt_out_keeps_the_sensor_pixels(magick().as_ref());
}

/// Runs alone in a child process (see `own_process`): it reads the
/// environment the child starts with.
#[test]
fn img_001_image_auto_orient_false_turns_orientation_off() {
    let output = {
        let _env = crate::env_lock::lock_env();
        crate::own_process::child_command(
            "images::img_001_image_auto_orient_false_turns_orientation_off_child",
        )
        .env("IMAGE_AUTO_ORIENT", "false")
        .output()
        .expect("the child runs")
    };
    crate::own_process::assert_child_passed(&output);
}

#[test]
fn img_001_image_auto_orient_false_turns_orientation_off_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let config = ImageConfig::from_env();
    assert!(
        !config.auto_orient,
        "IMAGE_AUTO_ORIENT=false must turn it off"
    );
    assert!(!suprnova::media::config().auto_orient);
    let rgba = pattern(WIDTH, HEIGHT);
    let png = png_with(
        &png_of(WIDTH, HEIGHT, rgba.clone()),
        &[(b"eXIf", exif_tiff(Some(6), false))],
    );
    let out = OxideAvImageDriver::new()
        .process(&png, &pipeline(vec![], OutputFormat::Png))
        .unwrap();
    assert_eq!(png_pixels(&out), (WIDTH, HEIGHT, rgba));
}

/// Under `driver`, `source` comes out turned by EXIF orientation `tag`:
/// the exact permutation of the pixels the driver decodes with orientation
/// off.
fn assert_oriented_by(driver: &dyn ImageDriver, label: &str, source: &[u8], tag: u16) {
    let sensor = {
        let _config = opt_out();
        driver
            .process(source, &pipeline(vec![], OutputFormat::Png))
            .unwrap_or_else(|e| panic!("{label}, unoriented, through {}: {e}", driver.name()))
    };
    let upright = driver
        .process(source, &pipeline(vec![], OutputFormat::Png))
        .unwrap_or_else(|e| panic!("{label} through {}: {e}", driver.name()));
    let (width, height, sensor) = png_pixels(&sensor);
    assert_eq!(
        png_pixels(&upright),
        turned(&sensor, width, height, tag),
        "{label} through {} must be turned by orientation {tag}",
        driver.name()
    );
}

/// A progressive JPEG with a second Exif segment between its scans, which
/// zune-jpeg keeps over the one in the header.
fn progressive_with_a_tag_between_scans() -> Vec<u8> {
    let jpeg = include_bytes!("fixtures/jpeg/photo-prog-420-35x21.jpg");
    let first = jpeg.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    let second = first
        + 2
        + jpeg[first + 2..]
            .windows(2)
            .position(|w| w == [0xFF, 0xDA])
            .expect("a second scan");
    let mut out = jpeg_with(&jpeg[..second], &[exif_segment(&exif_tiff(Some(3), false))]);
    out.extend_from_slice(&exif_segment(&exif_tiff(Some(6), false)));
    out.extend_from_slice(&jpeg[second..]);
    out
}

/// ImageMagick's own `Raw profile type exif` text chunk, holding EXIF with
/// orientation `tag`, which the framework does not read (it would have to
/// inflate a text chunk).
fn raw_profile_exif(tag: u16) -> Vec<u8> {
    let exif = [&b"Exif\0\0"[..], &exif_tiff(Some(tag), false)].concat();
    let hex: String = exif.iter().map(|byte| format!("{byte:02x}")).collect();
    let mut text = format!("\nexif\n{:8}\n", exif.len());
    for line in hex.as_bytes().chunks(72) {
        text.push_str(std::str::from_utf8(line).unwrap());
        text.push('\n');
    }
    [
        &b"Raw profile type exif\0\0"[..],
        &compcol::vec::compress_to_vec::<compcol::zlib::Zlib>(text.as_bytes()).unwrap(),
    ]
    .concat()
}

/// Both drivers read the orientation from the same places: the last Exif
/// segment of a JPEG, wherever it sits, and a PNG's `eXIf` chunk; not
/// ImageMagick's own text chunk or its `orNT` chunk.
fn assert_one_reader_orients(driver: &dyn ImageDriver) {
    let rgba = pattern(WIDTH, HEIGHT);
    let jpeg = convert(
        &png_of(WIDTH, HEIGHT, rgba.clone()),
        OutputFormat::Jpeg,
        100,
    );
    let two = jpeg_with(
        &jpeg,
        &[
            exif_segment(&exif_tiff(Some(3), false)),
            exif_segment(&exif_tiff(Some(6), false)),
        ],
    );
    assert_oriented_by(driver, "a JPEG with two Exif segments", &two, 6);
    assert_oriented_by(
        driver,
        "a progressive JPEG with a tag between its scans",
        &progressive_with_a_tag_between_scans(),
        6,
    );
    let png = png_of(WIDTH, HEIGHT, rgba);
    let text = png_with(&png, &[(b"zTXt", raw_profile_exif(6))]);
    assert_oriented_by(driver, "a PNG with EXIF only in a text chunk", &text, 1);
    let ornt = png_with(&png, &[(b"orNT", vec![6])]);
    assert_oriented_by(driver, "a PNG with an orNT chunk", &ornt, 1);
}

#[test]
#[serial]
fn img_001_oxideav_reads_the_orientation_from_the_shared_sources() {
    assert_one_reader_orients(oxideav().as_ref());
}

#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_001_magick_reads_the_orientation_from_the_shared_sources() {
    assert_one_reader_orients(magick().as_ref());
}

/// ImageMagick's names for the EXIF orientations 1 to 8, as `-orient`
/// takes them.
const MAGICK_ORIENTATIONS: [&str; 8] = [
    "TopLeft",
    "TopRight",
    "BottomRight",
    "BottomLeft",
    "LeftTop",
    "RightTop",
    "RightBottom",
    "LeftBottom",
];

/// A TIFF of `rgba`, a format the framework does not read, with the
/// orientation `tag` in its own IFD, written by the host ImageMagick.
fn magick_tiff(rgba: &[u8], width: u32, height: u32, tag: u16) -> Vec<u8> {
    host_magick(
        &[
            "png:-",
            "-orient",
            MAGICK_ORIENTATIONS[usize::from(tag) - 1],
            "-compress",
            "none",
            "tiff:-",
        ],
        &png_of(width, height, rgba.to_vec()),
    )
}

/// A format the framework cannot read takes ImageMagick's own reading of
/// its orientation: every orientation is applied once and leaves no tag;
/// the opt-out keeps the sensor's pixels and carries the tag alone in JPEG,
/// PNG and WebP; and `orient()` applies it under the opt-out, after a
/// custom step too, where ImageMagick's reading (a TIFF's tag, which it
/// writes as the PNG `orNT` chunk) has to cross the PNG between two runs.
#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_001_magick_orients_a_format_the_framework_cannot_read() {
    suprnova::register_transformation("img-001-identity", Ok);
    let rgba = pattern(WIDTH, HEIGHT);
    let driver = magick();
    let carriers = [OutputFormat::Jpeg, OutputFormat::Png, OutputFormat::WebP];
    let custom_then_orient = || {
        pipeline(
            vec![
                Transformation::custom("img-001-identity"),
                Transformation::Orient,
            ],
            OutputFormat::Png,
        )
    };
    for tag in 1..=8u16 {
        let tiff = magick_tiff(&rgba, WIDTH, HEIGHT, tag);
        let label = format!("a TIFF with orientation {tag}");
        let upright = driver
            .process(&tiff, &pipeline(vec![], OutputFormat::Png))
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        assert_eq!(
            png_pixels(&upright),
            turned(&rgba, WIDTH, HEIGHT, tag),
            "{label} must be turned by its orientation"
        );
        let once = driver
            .process(&tiff, &custom_then_orient())
            .unwrap_or_else(|e| panic!("{label}, a custom step then orient(): {e}"));
        assert_eq!(
            png_pixels(&once),
            turned(&rgba, WIDTH, HEIGHT, tag),
            "{label}: orient() after a custom step turns an image decode oriented again"
        );
        for target in carriers {
            let out = driver.process(&tiff, &pipeline(vec![], target)).unwrap();
            assert_eq!(
                metadata_found(target, &out),
                Vec::<String>::new(),
                "{label} to {target:?}: a tag after orientation was applied"
            );
        }

        let _config = opt_out();
        let sensor = driver
            .process(&tiff, &pipeline(vec![], OutputFormat::Png))
            .unwrap_or_else(|e| panic!("{label}, opt-out: {e}"));
        assert_eq!(
            png_pixels(&sensor),
            (WIDTH, HEIGHT, rgba.clone()),
            "{label}: the opt-out must keep the sensor's pixels"
        );
        for target in carriers {
            let out = driver.process(&tiff, &pipeline(vec![], target)).unwrap();
            assert_eq!(
                metadata_found(target, &out),
                vec!["orientation-only EXIF".to_string()],
                "{label} to {target:?}, opt-out"
            );
            let exif = output_exif(target, &out).expect("the opt-out's EXIF");
            assert_eq!(
                image::metadata::Orientation::from_exif_chunk(&exif).map(|o| o.to_exif()),
                Some(tag as u8),
                "{label} to {target:?}, opt-out: the tag's value"
            );
        }
        let oriented = driver
            .process(
                &tiff,
                &pipeline(vec![Transformation::Orient], OutputFormat::Png),
            )
            .unwrap();
        assert_eq!(
            png_pixels(&oriented),
            turned(&rgba, WIDTH, HEIGHT, tag),
            "{label}: orient() under the opt-out"
        );
        assert_eq!(
            metadata_found(OutputFormat::Png, &oriented),
            Vec::<String>::new(),
            "{label}: a tag after orient()"
        );
        let after_custom = driver
            .process(&tiff, &custom_then_orient())
            .unwrap_or_else(|e| panic!("{label}, opt-out, a custom step then orient(): {e}"));
        assert_eq!(
            png_pixels(&after_custom),
            turned(&rgba, WIDTH, HEIGHT, tag),
            "{label}: orient() after a custom step under the opt-out"
        );
        assert_eq!(
            metadata_found(OutputFormat::Png, &after_custom),
            Vec::<String>::new(),
            "{label}: a tag after a custom step and orient()"
        );
        let custom_only = driver
            .process(
                &tiff,
                &pipeline(
                    vec![Transformation::custom("img-001-identity")],
                    OutputFormat::Png,
                ),
            )
            .unwrap_or_else(|e| panic!("{label}, opt-out, a custom step: {e}"));
        assert_eq!(
            png_pixels(&custom_only),
            (WIDTH, HEIGHT, rgba.clone()),
            "{label}: a custom step under the opt-out keeps the sensor's pixels"
        );
        let exif = output_exif(OutputFormat::Png, &custom_only).expect("the opt-out's EXIF");
        assert_eq!(
            image::metadata::Orientation::from_exif_chunk(&exif).map(|o| o.to_exif()),
            Some(tag as u8),
            "{label}: a custom step under the opt-out keeps the tag's value"
        );
    }
}

// ───────────────────────── IMG-002 ─────────────────────────

const XMP: &[u8] = b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF/></x:xmpmeta>";

/// A JPEG, a PNG and a WebP of `rgba`, each carrying EXIF with orientation
/// 6 and a GPS position, XMP, and the text metadata its format holds.
fn sources_with_metadata(rgba: &[u8], width: u32, height: u32) -> Vec<(&'static str, Vec<u8>)> {
    let png = png_of(width, height, rgba.to_vec());
    let tiff = exif_tiff(Some(6), true);
    let jpeg = jpeg_with(
        &convert(&png, OutputFormat::Jpeg, 100),
        &[
            exif_segment(&tiff),
            segment(
                0xE1,
                &[&b"http://ns.adobe.com/xap/1.0/\0"[..], XMP].concat(),
            ),
            segment(
                0xED,
                b"Photoshop 3.0\08BIM\x04\x04\0\0\0\0\0\x07\x1c\x02\x05\0\x02hi",
            ),
            segment(0xFE, b"a comment"),
        ],
    );
    let zipped = [
        &b"Description\0\0"[..],
        &compcol::vec::compress_to_vec::<compcol::zlib::Zlib>(b"compressed text").unwrap(),
    ]
    .concat();
    let png = png_with(
        &png,
        &[
            (b"eXIf", tiff.clone()),
            (b"tEXt", text("Comment", "a comment")),
            (b"zTXt", zipped),
            (b"iTXt", [&b"XML:com.adobe.xmp\0\0\0\0\0"[..], XMP].concat()),
            (b"tIME", vec![7, 234, 10, 5, 12, 0, 0]),
        ],
    );
    let webp = webp_with(
        &convert(
            &png_of(width, height, rgba.to_vec()),
            OutputFormat::WebPLossless,
            70,
        ),
        None,
        &[
            (b"EXIF", [&b"Exif\0\0"[..], &tiff].concat()),
            (b"XMP ", XMP.to_vec()),
        ],
    );
    vec![("JPEG", jpeg), ("PNG", png), ("WebP", webp)]
}

const TARGETS: [OutputFormat; 5] = [
    OutputFormat::Jpeg,
    OutputFormat::Png,
    OutputFormat::WebP,
    OutputFormat::Gif,
    OutputFormat::Bmp,
];

/// Nothing but the Orientation-only EXIF of the opt-out survives, and that
/// only where the output can hold EXIF and orientation was not applied.
fn assert_metadata_is_stripped(driver: &dyn ImageDriver) {
    let rgba = pattern(WIDTH, HEIGHT);
    for (name, source) in sources_with_metadata(&rgba, WIDTH, HEIGHT) {
        for target in TARGETS {
            let label = format!("{name} to {target:?} through {}", driver.name());
            let oriented = driver.process(&source, &pipeline(vec![], target)).unwrap();
            assert_eq!(
                metadata_found(target, &oriented),
                Vec::<String>::new(),
                "{label}"
            );

            let _config = opt_out();
            let kept = driver.process(&source, &pipeline(vec![], target)).unwrap();
            let holds_exif = matches!(
                target,
                OutputFormat::Jpeg | OutputFormat::Png | OutputFormat::WebP
            );
            let expected: Vec<String> = if holds_exif {
                vec!["orientation-only EXIF".into()]
            } else {
                Vec::new()
            };
            assert_eq!(metadata_found(target, &kept), expected, "{label}, opt-out");
            if holds_exif {
                let tiff = output_exif(target, &kept).unwrap();
                assert_eq!(
                    image::metadata::Orientation::from_exif_chunk(&tiff).map(|o| o.to_exif()),
                    Some(6),
                    "{label}, opt-out keeps the tag's value"
                );
            }
            let turned = driver
                .process(&source, &pipeline(vec![Transformation::Orient], target))
                .unwrap();
            assert_eq!(
                metadata_found(target, &turned),
                Vec::<String>::new(),
                "{label}, opt-out with orient()"
            );
        }
    }
}

#[test]
#[serial]
fn img_002_oxideav_output_keeps_no_exif_xmp_iptc_or_text() {
    assert_metadata_is_stripped(oxideav().as_ref());
}

#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_002_magick_output_keeps_no_exif_xmp_iptc_or_text() {
    assert_metadata_is_stripped(magick().as_ref());
}

/// A Display P3 `rgba` image in each format that holds a profile.
fn sources_with_profile(
    rgba: &[u8],
    width: u32,
    height: u32,
    profile: &[u8],
) -> Vec<(&'static str, Vec<u8>)> {
    let png = png_of(width, height, rgba.to_vec());
    // Rendering intent 4: LCS_GM_IMAGES, perceptual, the ICC default.
    let bmp = oxideav_bmp::encode(
        &oxideav_bmp::BmpImage::new(
            width,
            height,
            oxideav_bmp::PixelFormat::Rgba,
            vec![oxideav_bmp::Plane::new(width as usize * 4, rgba.to_vec())],
        )
        .expect("the fixture BMP image")
        .with_metadata(oxideav_bmp::Metadata::new().with_icc(profile.to_vec())),
        &oxideav_bmp::EncodeOptions::default().with_rendering_intent(4),
    )
    .expect("the fixture BMP encodes");
    vec![
        (
            "JPEG",
            jpeg_with(
                &convert(&png, OutputFormat::Jpeg, 100),
                &icc_segments(profile),
            ),
        ),
        ("PNG", png_with(&png, &[(b"iCCP", iccp(profile))])),
        (
            "WebP",
            webp_with(
                &convert(&png, OutputFormat::WebPLossless, 70),
                Some(profile),
                &[],
            ),
        ),
        ("BMP", bmp),
    ]
}

/// A profile that describes the output's pixels is carried byte for byte
/// into every format that holds one, and none ever describes pixels of
/// another colour space.
fn assert_a_matching_profile_is_carried(driver: &dyn ImageDriver) {
    let p3 = display_p3();
    for (name, source) in sources_with_profile(&pattern(WIDTH, HEIGHT), WIDTH, HEIGHT, &p3) {
        for target in [
            OutputFormat::Jpeg,
            OutputFormat::Png,
            OutputFormat::WebP,
            OutputFormat::Bmp,
        ] {
            let out = driver.process(&source, &pipeline(vec![], target)).unwrap();
            let profile = output_profile(target, &out);
            assert!(
                profile.as_deref() == Some(&p3[..]),
                "{name} to {target:?} through {}: the profile is not the source's",
                driver.name()
            );
            assert_eq!(pixel_space(target, &out), *b"RGB ", "{name} to {target:?}");
        }
    }
}

#[test]
#[serial]
fn img_002_oxideav_carries_a_matching_profile() {
    assert_a_matching_profile_is_carried(oxideav().as_ref());
}

#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_002_magick_carries_a_matching_profile() {
    assert_a_matching_profile_is_carried(magick().as_ref());
}

/// The sRGB form of `level` in a grey profile.
fn grey_in_srgb(profile: &[u8], level: u8) -> u8 {
    let source = moxcms::ColorProfile::new_from_slice(profile).unwrap();
    let transform = source
        .create_transform_8bit(
            moxcms::Layout::Gray,
            &moxcms::ColorProfile::new_srgb(),
            moxcms::Layout::Rgb,
            moxcms::TransformOptions::default(),
        )
        .unwrap();
    let mut rgb = [0u8; 3];
    transform.transform(&[level], &mut rgb).unwrap();
    rgb[0]
}

/// A grey profile goes with the pixels it describes, or the pixels are
/// converted from it to sRGB and it goes: never a grey profile on RGB
/// pixels, never a profile kept with its colour chunks dropped, and never
/// a colour chunk kept after its profile is converted away.
fn assert_a_grey_profile_never_describes_rgb(driver: &dyn ImageDriver) {
    let grey = grey_gamma_profile();
    let level = 100;
    let colour_chunks = vec![
        (*b"gAMA", 55_555u32.to_be_bytes().to_vec()),
        (*b"cHRM", chrm()),
        // BT.709 primaries, the sRGB transfer function, RGB, full range.
        (*b"cICP", vec![1, 13, 0, 1]),
    ];
    let mut chunks: Vec<(&[u8; 4], Vec<u8>)> = colour_chunks
        .iter()
        .map(|(kind, data)| (kind, data.clone()))
        .collect();
    chunks.push((b"iCCP", iccp(&grey)));
    let source = png_with(&grey_png(8, 8, level), &chunks);
    let converted = grey_in_srgb(&grey, level);
    // With and without a custom step, which under `magick` takes the image
    // through Rust as RGBA.
    suprnova::register_transformation("img-002-identity", Ok);
    let pipelines = [vec![], vec![Transformation::custom("img-002-identity")]];
    for (target, steps) in [
        OutputFormat::Jpeg,
        OutputFormat::Png,
        OutputFormat::WebPLossless,
        OutputFormat::Bmp,
    ]
    .into_iter()
    .flat_map(|target| pipelines.iter().map(move |steps| (target, steps)))
    {
        let label = format!("{target:?} through {}, steps {steps:?}", driver.name());
        let out = driver
            .process(&source, &pipeline(steps.clone(), target))
            .unwrap();
        let pixels = driver
            .process(&out, &pipeline(vec![], OutputFormat::Png))
            .map(|png| png_pixels(&png).2)
            .unwrap();
        match output_profile(target, &out) {
            Some(profile) => {
                assert_eq!(profile, grey, "{label}");
                assert_eq!(
                    pixel_space(target, &out),
                    *b"GRAY",
                    "{label}: a grey profile on RGB"
                );
                if target == OutputFormat::Png {
                    assert_eq!(
                        colour_chunks_of(&out),
                        colour_chunks,
                        "{label}: the profile stayed but its colour chunks did not"
                    );
                }
            }
            None => {
                let tolerance = if target == OutputFormat::Jpeg { 2 } else { 1 };
                for pixel in pixels.chunks(4) {
                    for &channel in &pixel[..3] {
                        assert!(
                            channel.abs_diff(converted) <= tolerance,
                            "{label}: {channel} is not the sRGB form {converted} of level {level}"
                        );
                    }
                }
                if target == OutputFormat::Png {
                    assert_eq!(
                        colour_chunks_of(&out),
                        Vec::new(),
                        "{label}: colour chunks kept after their profile was converted away"
                    );
                }
            }
        }
    }
}

#[test]
#[serial]
fn img_002_oxideav_never_puts_a_grey_profile_on_rgb() {
    assert_a_grey_profile_never_describes_rgb(oxideav().as_ref());
}

#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_002_magick_never_puts_a_grey_profile_on_rgb() {
    assert_a_grey_profile_never_describes_rgb(magick().as_ref());
}

/// An RGB profile on an all-grey image stays with RGB pixels: ImageMagick
/// would write such an image as greyscale and keep the RGB profile.
#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_002_magick_keeps_an_rgb_profile_on_grey_pixels_consistent() {
    let p3 = display_p3();
    let rgba = flat(8, 8, [120, 120, 120, 255]);
    for (name, source) in sources_with_profile(&rgba, 8, 8, &p3) {
        for target in [OutputFormat::Jpeg, OutputFormat::Png] {
            let out = magick()
                .process(&source, &pipeline(vec![], target))
                .unwrap();
            let profile = output_profile(target, &out);
            assert!(
                profile.as_deref() == Some(&p3[..]),
                "{name} to {target:?}: the profile was dropped or changed"
            );
            assert_eq!(pixel_space(target, &out), *b"RGB ", "{name} to {target:?}");
        }
    }
}

/// GIF holds no profile here, so a flat Display P3 image comes out as its
/// sRGB conversion.
fn assert_gif_output_is_srgb(driver: &dyn ImageDriver) {
    let p3 = display_p3();
    let colour = [200u8, 60, 40];
    let transform = moxcms::ColorProfile::new_from_slice(&p3)
        .unwrap()
        .create_transform_8bit(
            moxcms::Layout::Rgb,
            &moxcms::ColorProfile::new_srgb(),
            moxcms::Layout::Rgb,
            moxcms::TransformOptions::default(),
        )
        .unwrap();
    let rgba = flat(16, 16, [colour[0], colour[1], colour[2], 255]);
    for (name, source) in sources_with_profile(&rgba, 16, 16, &p3) {
        // The colour the driver decodes, which a lossy JPEG moves a level
        // or two from the one it was written with. A PNG keeps the profile,
        // so its pixels are the decoded ones, unconverted.
        let decoded = driver
            .process(&source, &pipeline(vec![], OutputFormat::Png))
            .unwrap();
        assert_eq!(
            output_profile(OutputFormat::Png, &decoded).as_deref(),
            Some(&p3[..])
        );
        let decoded = pixel_at(&decoded, 8, 8);
        let mut expected = [0u8; 3];
        transform.transform(&decoded[..3], &mut expected).unwrap();

        let gif = driver
            .process(&source, &pipeline(vec![], OutputFormat::Gif))
            .unwrap();
        assert_eq!(output_profile(OutputFormat::Gif, &gif), None, "{name}");
        let png = OxideAvImageDriver::new()
            .process(&gif, &pipeline(vec![], OutputFormat::Png))
            .unwrap();
        for pixel in png_pixels(&png).2.chunks(4) {
            for (channel, want) in pixel[..3].iter().zip(expected) {
                assert!(
                    channel.abs_diff(want) <= 1,
                    "{name} through {}: {:?} is not the sRGB form {expected:?} of P3 {:?}",
                    driver.name(),
                    &pixel[..3],
                    &decoded[..3]
                );
            }
        }
    }
}

#[test]
#[serial]
fn img_002_oxideav_gif_output_is_srgb() {
    assert_gif_output_is_srgb(oxideav().as_ref());
}

#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_002_magick_gif_output_is_srgb() {
    assert_gif_output_is_srgb(magick().as_ref());
}

/// A `cHRM` chunk's data: a white point and three primaries.
fn chrm() -> Vec<u8> {
    [
        31_270u32, 32_900, 64_000, 33_000, 30_000, 60_000, 15_000, 6_000,
    ]
    .iter()
    .flat_map(|value| value.to_be_bytes())
    .collect()
}

/// A `cICP` chunk's data: Display P3 primaries (12), the sRGB transfer
/// function (13), RGB (matrix 0), full range.
const P3_CICP: [u8; 4] = [12, 13, 0, 1];

/// A PNG's colour chunks travel with its profile into PNG output, byte for
/// byte, and alone when it has no profile.
fn assert_png_colour_chunks_are_kept(driver: &dyn ImageDriver) {
    let p3 = display_p3();
    let rgba = pattern(WIDTH, HEIGHT);
    let gama = 45_455u32.to_be_bytes().to_vec();
    let chrm = chrm();
    let cicp = P3_CICP.to_vec();
    let with_profile = png_with(
        &png_of(WIDTH, HEIGHT, rgba.clone()),
        &[
            (b"cHRM", chrm.clone()),
            (b"gAMA", gama.clone()),
            (b"cICP", cicp.clone()),
            (b"iCCP", iccp(&p3)),
        ],
    );
    let alone = png_with(&png_of(WIDTH, HEIGHT, rgba), &[(b"sRGB", vec![0])]);
    let out = driver
        .process(&with_profile, &pipeline(vec![], OutputFormat::Png))
        .unwrap();
    assert_eq!(
        output_profile(OutputFormat::Png, &out).as_deref(),
        Some(&p3[..])
    );
    assert_eq!(
        colour_chunks_of(&out),
        vec![(*b"cHRM", chrm), (*b"gAMA", gama), (*b"cICP", cicp)],
        "{}",
        driver.name()
    );
    let out = driver
        .process(&alone, &pipeline(vec![], OutputFormat::Png))
        .unwrap();
    assert_eq!(
        colour_chunks_of(&out),
        vec![(*b"sRGB", vec![0])],
        "{}",
        driver.name()
    );
}

#[test]
#[serial]
fn img_002_oxideav_keeps_png_colour_chunks_with_their_profile() {
    assert_png_colour_chunks_are_kept(oxideav().as_ref());
}

#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_002_magick_keeps_png_colour_chunks_with_their_profile() {
    assert_png_colour_chunks_are_kept(magick().as_ref());
}

/// Run the host ImageMagick with `args` on `input` over stdin, and return
/// what it writes to stdout.
fn host_magick(args: &[&str], input: &[u8]) -> Vec<u8> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(MagickCliDriver::from_env().binary())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the host ImageMagick runs");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input)
        .expect("the input goes in");
    let output = child.wait_with_output().expect("ImageMagick finishes");
    assert!(output.status.success(), "ImageMagick runs {args:?}");
    output.stdout
}

/// Decode `jpeg` with the host ImageMagick, whose JPEG decoder is libjpeg,
/// the decoder browsers use too; packed RGBA.
fn libjpeg_pixels(jpeg: &[u8]) -> (u32, u32, Vec<u8>) {
    png_pixels(&host_magick(&["jpeg:-", "-depth", "8", "png:-"], jpeg))
}

/// A JPEG the built-in driver writes decodes to the same colours in the
/// framework's decoder and in libjpeg. It used to carry RGB samples behind
/// a JFIF header, which libjpeg reads as YCbCr: a flat (200, 60, 40) came
/// back as (77, 255, 80).
#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_002_a_jpeg_decodes_to_the_same_colours_in_libjpeg() {
    let colour = [200u8, 60, 40];
    let flat_png = png_of(16, 16, flat(16, 16, [colour[0], colour[1], colour[2], 255]));
    let pattern_png = png_of(WIDTH, HEIGHT, pattern(WIDTH, HEIGHT));
    let driver = OxideAvImageDriver::new();
    for (name, source, against_source) in
        [("flat", flat_png, true), ("pattern", pattern_png, false)]
    {
        let jpeg = driver
            .process(&source, &pipeline(vec![], OutputFormat::Jpeg))
            .expect("the driver writes a JPEG");
        let own = png_pixels(
            &driver
                .process(&jpeg, &pipeline(vec![], OutputFormat::Png))
                .expect("the framework decodes its own JPEG"),
        );
        let libjpeg = libjpeg_pixels(&jpeg);
        assert_eq!((own.0, own.1), (libjpeg.0, libjpeg.1), "{name}: sizes");
        for (index, (a, b)) in own.2.chunks(4).zip(libjpeg.2.chunks(4)).enumerate() {
            assert!(
                close([a[0], a[1], a[2], a[3]], [b[0], b[1], b[2]], 4),
                "{name} pixel {index}: the framework reads {a:?}, libjpeg reads {b:?}"
            );
            if against_source {
                for (decoded, label) in [(a, "the framework"), (b, "libjpeg")] {
                    assert!(
                        close([decoded[0], decoded[1], decoded[2], 255], colour, 6),
                        "{name} pixel {index}: {label} reads {decoded:?} for {colour:?}"
                    );
                }
            }
        }
    }
}

/// `profile` with its header's size field changed to `size`.
fn misdeclared(mut profile: Vec<u8>, size: u32) -> Vec<u8> {
    profile[..4].copy_from_slice(&size.to_be_bytes());
    profile
}

/// A profile whose header gives another size than its length is not an ICC
/// profile, and is not carried.
fn assert_a_misdeclared_profile_is_not_carried(driver: &dyn ImageDriver) {
    let p3 = display_p3();
    let wrong = misdeclared(p3.clone(), p3.len() as u32 + 4);
    for (name, source) in sources_with_profile(&pattern(WIDTH, HEIGHT), WIDTH, HEIGHT, &wrong) {
        for target in [OutputFormat::Jpeg, OutputFormat::Png, OutputFormat::WebP] {
            let label = format!("{name} to {target:?} through {}", driver.name());
            match driver.process(&source, &pipeline(vec![], target)) {
                Ok(out) => assert_eq!(output_profile(target, &out), None, "{label}"),
                // ImageMagick's PNG reader, libpng, refuses a PNG whose
                // iCCP profile does not match its header: not carried
                // either.
                Err(e) if driver.name() == "magick" && name == "PNG" => {
                    assert!(e.to_string().contains("iCCP"), "{label}: {e}");
                }
                Err(e) => panic!("{label}: {e}"),
            }
        }
    }
}

#[test]
#[serial]
fn img_002_oxideav_does_not_carry_a_profile_its_header_misdescribes() {
    assert_a_misdeclared_profile_is_not_carried(oxideav().as_ref());
}

#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_002_magick_does_not_carry_a_profile_its_header_misdescribes() {
    assert_a_misdeclared_profile_is_not_carried(magick().as_ref());
}

/// `profile` claiming the CMYK colour space.
fn cmyk_class(mut profile: Vec<u8>) -> Vec<u8> {
    profile[16..20].copy_from_slice(b"CMYK");
    profile
}

/// A CMYK profile cannot describe RGB pixels, so RGB output never carries
/// one.
#[test]
#[serial]
fn img_002_oxideav_does_not_carry_a_cmyk_profile_on_rgb_pixels() {
    let cmyk = cmyk_class(display_p3());
    let rgba = pattern(WIDTH, HEIGHT);
    let png = png_with(
        &png_of(WIDTH, HEIGHT, rgba.clone()),
        &[(b"iCCP", iccp(&cmyk))],
    );
    for target in [
        OutputFormat::Jpeg,
        OutputFormat::Png,
        OutputFormat::WebP,
        OutputFormat::Bmp,
    ] {
        let out = oxideav().process(&png, &pipeline(vec![], target)).unwrap();
        assert_eq!(output_profile(target, &out), None, "{target:?}");
    }
    let out = oxideav()
        .process(&png, &pipeline(vec![], OutputFormat::Png))
        .unwrap();
    assert_eq!(
        png_pixels(&out),
        (WIDTH, HEIGHT, rgba),
        "nothing to convert"
    );
}

#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_002_magick_does_not_carry_a_cmyk_profile_on_rgb_pixels() {
    use std::process::Command;
    let made = Command::new(MagickCliDriver::from_env().binary())
        .args(["-size", "8x8", "xc:#c83c28", "-colorspace", "CMYK", "jpg:-"])
        .output()
        .expect("ImageMagick writes a CMYK JPEG");
    assert!(made.status.success());
    let cmyk = cmyk_class(display_p3());
    let source = jpeg_with(&made.stdout, &icc_segments(&cmyk));
    for target in [OutputFormat::Png, OutputFormat::WebP, OutputFormat::Bmp] {
        let out = magick()
            .process(&source, &pipeline(vec![], target))
            .unwrap();
        assert_eq!(output_profile(target, &out), None, "{target:?}");
        assert_eq!(pixel_space(target, &out), *b"RGB ", "{target:?}");
    }
}

/// Under the opt-out, `orient()` after a custom step turns the pixels the
/// step returned: under `magick` the turn comes after the image went
/// through Rust between two runs.
fn assert_orient_after_a_custom_step(driver: &dyn ImageDriver) {
    suprnova::register_transformation("img-001-identity", Ok);
    for (name, source) in oriented_sources(&pattern(WIDTH, HEIGHT), WIDTH, HEIGHT, 6) {
        let _config = opt_out();
        let sensor = driver
            .process(&source, &pipeline(vec![], OutputFormat::Png))
            .unwrap();
        let out = driver
            .process(
                &source,
                &pipeline(
                    vec![
                        Transformation::custom("img-001-identity"),
                        Transformation::Orient,
                    ],
                    OutputFormat::Png,
                ),
            )
            .unwrap();
        let (width, height, sensor) = png_pixels(&sensor);
        assert_eq!(
            png_pixels(&out),
            turned(&sensor, width, height, 6),
            "{name} through {}",
            driver.name()
        );
        assert_eq!(
            metadata_found(OutputFormat::Png, &out),
            Vec::<String>::new(),
            "{name}"
        );
    }
}

#[test]
#[serial]
fn img_001_orient_after_a_custom_step_under_oxideav() {
    assert_orient_after_a_custom_step(oxideav().as_ref());
}

#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_001_orient_after_a_custom_step_under_magick() {
    assert_orient_after_a_custom_step(magick().as_ref());
}

/// Both drivers report the size an image has once turned, for a source that
/// still carries its tag, without turning it.
#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_001_both_drivers_report_a_tagged_source_at_its_turned_size() {
    for (name, source) in oriented_sources(&pattern(WIDTH, HEIGHT), WIDTH, HEIGHT, 6) {
        for driver in [oxideav(), magick()] {
            assert_eq!(
                driver.dimensions(&source).unwrap(),
                (HEIGHT, WIDTH),
                "{name} through {}",
                driver.name()
            );
        }
    }
    // A format only `magick` reads takes ImageMagick's reading of the tag,
    // the one its decode applies.
    let tiff = magick_tiff(&pattern(WIDTH, HEIGHT), WIDTH, HEIGHT, 6);
    assert_eq!(
        magick().dimensions(&tiff).unwrap(),
        (HEIGHT, WIDTH),
        "a TIFF with orientation 6 through magick"
    );
    let _config = opt_out();
    assert_eq!(
        magick().dimensions(&tiff).unwrap(),
        (WIDTH, HEIGHT),
        "a TIFF with orientation 6 through magick, opt-out"
    );
}

/// How many times `img-004-counted` ran.
static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A custom step runs once, even when `magick` runs the end of the pipeline
/// again to keep a profile consistent with its pixels: an all-grey image
/// with an RGB profile, written as PNG, which ImageMagick writes as grey.
#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_004_a_custom_step_runs_once_when_magick_runs_again() {
    suprnova::register_transformation("img-004-counted", |pixels| {
        RUNS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(pixels)
    });
    let p3 = display_p3();
    let grey = png_with(
        &png_of(8, 8, flat(8, 8, [120, 120, 120, 255])),
        &[(b"iCCP", iccp(&p3))],
    );
    RUNS.store(0, std::sync::atomic::Ordering::SeqCst);
    let out = magick()
        .process(
            &grey,
            &pipeline(
                vec![Transformation::custom("img-004-counted")],
                OutputFormat::Png,
            ),
        )
        .unwrap();
    assert_eq!(RUNS.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        output_profile(OutputFormat::Png, &out).as_deref(),
        Some(&p3[..])
    );
    assert_eq!(pixel_space(OutputFormat::Png, &out), *b"RGB ");
}

// ───────────────────────── IMG-003 ─────────────────────────

const MISSING_MAGICK: &str = "suprnova-img-003-no-such-magick";

/// Runs alone in a child process (see `own_process`): the child names a
/// missing ImageMagick binary, and the facade's drivers are built once per
/// process.
#[test]
fn img_003_an_image_can_choose_its_driver() {
    let output = {
        let _env = crate::env_lock::lock_env();
        crate::own_process::child_command("images::img_003_an_image_can_choose_its_driver_child")
            .env("IMAGE_MAGICK_BINARY", MISSING_MAGICK)
            .env_remove("IMAGE_DRIVER")
            .output()
            .expect("the child runs")
    };
    crate::own_process::assert_child_passed(&output);
}

#[tokio::test]
async fn img_003_an_image_can_choose_its_driver_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let png = png_of(WIDTH, HEIGHT, pattern(WIDTH, HEIGHT));
    let image = Image::from_bytes(png.clone()).resize(3, 2).to_png();
    let err = image
        .clone()
        .using(ImageDriverKind::Magick)
        .to_bytes()
        .await
        .expect_err("the image chose magick, whose binary is missing");
    assert!(err.to_string().contains(MISSING_MAGICK), "got: {err}");
    // The same image without the choice, a clone of it, and another image
    // all still run the process default.
    image.clone().to_bytes().await.expect("the default driver");
    Image::from_bytes(png.clone())
        .to_bytes()
        .await
        .expect("another image");
    assert_eq!(suprnova::media::default_driver().unwrap().name(), "oxideav");
    // Choosing the default by name works too.
    image
        .using(ImageDriverKind::OxideAv)
        .to_bytes()
        .await
        .expect("an image that chose oxideav");
}

/// Runs alone in a child process (see `own_process`): the process default
/// is `magick`, naming a missing binary, so only an image that chose
/// `oxideav` can succeed.
#[test]
fn img_003_an_image_can_choose_oxideav_over_a_magick_default() {
    let output = {
        let _env = crate::env_lock::lock_env();
        crate::own_process::child_command(
            "images::img_003_an_image_can_choose_oxideav_over_a_magick_default_child",
        )
        .env("IMAGE_DRIVER", "magick")
        .env("IMAGE_MAGICK_BINARY", MISSING_MAGICK)
        .output()
        .expect("the child runs")
    };
    crate::own_process::assert_child_passed(&output);
}

#[tokio::test]
async fn img_003_an_image_can_choose_oxideav_over_a_magick_default_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let png = png_of(WIDTH, HEIGHT, pattern(WIDTH, HEIGHT));
    let image = Image::from_bytes(png.clone()).resize(3, 2).to_png();
    let err = image
        .clone()
        .to_bytes()
        .await
        .expect_err("the default is magick, whose binary is missing");
    assert!(err.to_string().contains(MISSING_MAGICK), "got: {err}");
    let chosen = image
        .clone()
        .using(ImageDriverKind::OxideAv)
        .to_bytes()
        .await
        .expect("an image that chose oxideav");
    let by_oxideav = OxideAvImageDriver::new()
        .process(
            &png,
            &pipeline(
                vec![Transformation::Resize {
                    width: 3,
                    height: 2,
                }],
                OutputFormat::Png,
            ),
        )
        .unwrap();
    assert_eq!(chosen, by_oxideav, "the image that chose oxideav ran it");
    // The choice changed neither the process default nor the same image
    // without it.
    assert_eq!(suprnova::media::default_driver().unwrap().name(), "magick");
    image
        .to_bytes()
        .await
        .expect_err("the image without the choice still runs magick");
}

/// What [`CustomDefault`] answers for every image, so the output shows
/// which driver ran.
const CUSTOM_OUTPUT: &[u8] = b"img-003-custom-default";

/// A driver an application installs with `set_default_driver`.
struct CustomDefault;

impl ImageDriver for CustomDefault {
    fn process(
        &self,
        _contents: &[u8],
        _pipeline: &ImagePipeline,
    ) -> Result<Vec<u8>, suprnova::FrameworkError> {
        Ok(CUSTOM_OUTPUT.to_vec())
    }

    fn dimensions(&self, _contents: &[u8]) -> Result<(u32, u32), suprnova::FrameworkError> {
        Ok((1, 1))
    }

    fn dominant_color(&self, _contents: &[u8]) -> Result<String, suprnova::FrameworkError> {
        Ok("#000000".to_owned())
    }

    fn name(&self) -> &'static str {
        "img-003-custom"
    }
}

/// The image the custom-default tests process, with [`CustomDefault`]
/// installed as the process default. An image without a choice runs it.
async fn under_a_custom_default() -> (Vec<u8>, Image) {
    suprnova::media::set_default_driver(Box::new(CustomDefault))
        .expect("the first driver of the process");
    let png = png_of(WIDTH, HEIGHT, pattern(WIDTH, HEIGHT));
    let image = Image::from_bytes(png.clone()).resize(3, 2).to_png();
    assert_eq!(
        image.clone().to_bytes().await.expect("the custom default"),
        CUSTOM_OUTPUT
    );
    (png, image)
}

/// The image's choice changed neither the process default nor the same
/// image without it.
async fn the_custom_default_still_runs(image: Image) {
    assert_eq!(
        suprnova::media::default_driver().unwrap().name(),
        "img-003-custom"
    );
    assert_eq!(
        image.to_bytes().await.expect("the custom default"),
        CUSTOM_OUTPUT
    );
}

/// Runs alone in a child process (see `own_process`): the child installs a
/// custom default driver, which a process does once, and names a missing
/// ImageMagick binary.
#[test]
fn img_003_an_image_choice_overrides_a_custom_default() {
    let output = {
        let _env = crate::env_lock::lock_env();
        crate::own_process::child_command(
            "images::img_003_an_image_choice_overrides_a_custom_default_child",
        )
        .env("IMAGE_MAGICK_BINARY", MISSING_MAGICK)
        .env_remove("IMAGE_DRIVER")
        .output()
        .expect("the child runs")
    };
    crate::own_process::assert_child_passed(&output);
}

#[tokio::test]
async fn img_003_an_image_choice_overrides_a_custom_default_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let (png, image) = under_a_custom_default().await;
    let chosen = image
        .clone()
        .using(ImageDriverKind::OxideAv)
        .to_bytes()
        .await
        .expect("an image that chose oxideav");
    let by_oxideav = OxideAvImageDriver::new()
        .process(
            &png,
            &pipeline(
                vec![Transformation::Resize {
                    width: 3,
                    height: 2,
                }],
                OutputFormat::Png,
            ),
        )
        .unwrap();
    assert_eq!(chosen, by_oxideav, "the image that chose oxideav ran it");
    let err = image
        .clone()
        .using(ImageDriverKind::Magick)
        .to_bytes()
        .await
        .expect_err("the image chose magick, whose binary is missing");
    assert!(err.to_string().contains(MISSING_MAGICK), "got: {err}");
    the_custom_default_still_runs(image).await;
}

/// Runs alone in a child process (see `own_process`): the child installs a
/// custom default driver, which a process does once, and runs the host's
/// ImageMagick.
#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_003_an_image_can_choose_the_host_magick_over_a_custom_default() {
    let output = {
        let _env = crate::env_lock::lock_env();
        crate::own_process::child_command(
            "images::img_003_an_image_can_choose_the_host_magick_over_a_custom_default_child",
        )
        .env_remove("IMAGE_MAGICK_BINARY")
        .env_remove("IMAGE_DRIVER")
        .output()
        .expect("the child runs")
    };
    crate::own_process::assert_child_passed(&output);
}

#[tokio::test]
#[ignore = "requires a host ImageMagick 7 binary"]
async fn img_003_an_image_can_choose_the_host_magick_over_a_custom_default_child() {
    if !crate::own_process::is_child() {
        return;
    }
    let (png, image) = under_a_custom_default().await;
    let chosen = image
        .clone()
        .using(ImageDriverKind::Magick)
        .to_bytes()
        .await
        .expect("an image that chose the host magick");
    let by_magick = magick()
        .process(
            &png,
            &pipeline(
                vec![Transformation::Resize {
                    width: 3,
                    height: 2,
                }],
                OutputFormat::Png,
            ),
        )
        .unwrap();
    assert_eq!(chosen, by_magick, "the image that chose magick ran it");
    the_custom_default_still_runs(image).await;
}

// ───────────────────────── IMG-004 ─────────────────────────

/// What a probe transformation saw: the size and pixels it received.
static SEEN: Mutex<Option<(u32, u32, Vec<u8>)>> = Mutex::new(None);

/// Register `img-004-invert`: it records what it receives and inverts the
/// colour channels.
fn register_probe() {
    suprnova::register_transformation("img-004-invert", |mut pixels: ImagePixels| {
        *SEEN.lock().unwrap() = Some((pixels.width(), pixels.height(), pixels.pixels().to_vec()));
        for pixel in pixels.pixels_mut().chunks_mut(4) {
            for channel in &mut pixel[..3] {
                *channel = !*channel;
            }
        }
        Ok(pixels)
    });
}

/// A custom step runs at its place: it receives the pixels the steps before
/// it produce, and the steps after it work on what it returns.
fn assert_a_custom_step_runs_at_its_place(driver: &dyn ImageDriver) {
    register_probe();
    let source = png_of(WIDTH, HEIGHT, pattern(WIDTH, HEIGHT));
    let before = vec![Transformation::Resize {
        width: 4,
        height: 2,
    }];
    let prefix = driver
        .process(&source, &pipeline(before.clone(), OutputFormat::Png))
        .unwrap();
    let (width, height, expected_in) = png_pixels(&prefix);
    let mut steps = before;
    steps.push(Transformation::custom("img-004-invert"));
    steps.push(Transformation::FlipHorizontally);
    let out = driver
        .process(&source, &pipeline(steps, OutputFormat::Png))
        .unwrap();
    let (seen_width, seen_height, seen) = SEEN.lock().unwrap().take().expect("the step ran");
    assert_eq!(
        (seen_width, seen_height),
        (width, height),
        "{}",
        driver.name()
    );
    assert_eq!(
        seen,
        expected_in,
        "{}: the step saw other pixels",
        driver.name()
    );
    let inverted: Vec<u8> = expected_in
        .chunks(4)
        .flat_map(|pixel| [!pixel[0], !pixel[1], !pixel[2], pixel[3]])
        .collect();
    assert_eq!(
        png_pixels(&out),
        turned(&inverted, width, height, 2),
        "{}: the flip did not follow the step",
        driver.name()
    );
}

#[test]
#[serial]
fn img_004_a_custom_step_runs_at_its_place_under_oxideav() {
    assert_a_custom_step_runs_at_its_place(oxideav().as_ref());
}

#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_004_a_custom_step_runs_at_its_place_under_magick() {
    assert_a_custom_step_runs_at_its_place(magick().as_ref());
}

fn assert_an_unregistered_step_fails_naming_it(driver: &dyn ImageDriver) {
    let source = png_of(WIDTH, HEIGHT, pattern(WIDTH, HEIGHT));
    let err = driver
        .process(
            &source,
            &pipeline(
                vec![Transformation::custom("img-004-never-registered")],
                OutputFormat::Png,
            ),
        )
        .expect_err("nothing is registered under this name");
    assert!(
        err.to_string().contains("img-004-never-registered"),
        "got: {err}"
    );
}

#[test]
#[serial]
fn img_004_an_unregistered_step_fails_naming_it_under_oxideav() {
    assert_an_unregistered_step_fails_naming_it(oxideav().as_ref());
}

#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_004_an_unregistered_step_fails_naming_it_under_magick() {
    assert_an_unregistered_step_fails_naming_it(magick().as_ref());
}

/// `pixels` widened to twice its width, the new right half transparent.
fn widened(pixels: &ImagePixels) -> Result<ImagePixels, suprnova::FrameworkError> {
    let (width, height) = (pixels.width() as usize, pixels.height() as usize);
    let mut out = Vec::with_capacity(width * height * 8);
    for row in pixels.pixels().chunks(width * 4) {
        out.extend_from_slice(row);
        out.resize(out.len() + width * 4, 0);
    }
    ImagePixels::new(pixels.width() * 2, height as u32, out)
}

/// A custom step that changes the size hands the steps after it the image
/// it returned, and one whose result is past `IMAGE_MAX_DIMENSION` is
/// refused with the error that limit gives.
fn assert_a_custom_step_can_change_the_size(driver: &dyn ImageDriver) {
    suprnova::register_transformation("img-004-widen", |pixels| widened(&pixels));
    let rgba = pattern(WIDTH, HEIGHT);
    let source = png_of(WIDTH, HEIGHT, rgba.clone());
    let steps = vec![
        Transformation::custom("img-004-widen"),
        Transformation::FlipHorizontally,
    ];
    let out = driver
        .process(&source, &pipeline(steps.clone(), OutputFormat::Png))
        .unwrap_or_else(|e| panic!("{}: {e}", driver.name()));
    let wide = widened(&ImagePixels::new(WIDTH, HEIGHT, rgba).unwrap()).unwrap();
    assert_eq!(
        png_pixels(&out),
        turned(wide.pixels(), WIDTH * 2, HEIGHT, 2),
        "{}: the flip did not work on the widened image",
        driver.name()
    );

    let mut config = ImageConfig::default();
    config.max_dimension = WIDTH * 2 - 1;
    let _config = ConfigGuard::set(config);
    let err = driver
        .process(&source, &pipeline(steps, OutputFormat::Png))
        .expect_err("the widened image is past the limit");
    let message = err.to_string();
    assert!(
        message.contains(&format!("{}x{HEIGHT}", WIDTH * 2))
            && message.contains("IMAGE_MAX_DIMENSION"),
        "{}: got {message}",
        driver.name()
    );
}

#[test]
#[serial]
fn img_004_a_custom_step_can_change_the_size_within_the_limits_under_oxideav() {
    assert_a_custom_step_can_change_the_size(oxideav().as_ref());
}

#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_004_a_custom_step_can_change_the_size_within_the_limits_under_magick() {
    assert_a_custom_step_can_change_the_size(magick().as_ref());
}

/// Runs alone in a child process (see `own_process`): it points the
/// temporary directories at directories of its own.
#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_004_a_magick_custom_step_adds_no_argument_and_no_file() {
    let scratch = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("img-004-magick");
    let _ = std::fs::remove_dir_all(&scratch);
    let rust_tmp = scratch.join("rust-tmp");
    let magick_tmp = scratch.join("magick-tmp");
    std::fs::create_dir_all(&rust_tmp).unwrap();
    std::fs::create_dir_all(&magick_tmp).unwrap();
    let output = {
        let _env = crate::env_lock::lock_env();
        crate::own_process::child_command(
            "images::img_004_a_magick_custom_step_adds_no_argument_and_no_file_child",
        )
        .env("TMPDIR", &rust_tmp)
        .env("MAGICK_TEMPORARY_PATH", &magick_tmp)
        .env("IMG_004_SCRATCH", &scratch)
        .output()
        .expect("the child runs")
    };
    crate::own_process::assert_child_passed(&output);
}

#[test]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_004_a_magick_custom_step_adds_no_argument_and_no_file_child() {
    use std::os::unix::fs::PermissionsExt;
    if !crate::own_process::is_child() {
        return;
    }
    let scratch = std::path::PathBuf::from(std::env::var("IMG_004_SCRATCH").unwrap());
    let rust_tmp = std::env::temp_dir();
    let magick_tmp = std::path::PathBuf::from(std::env::var("MAGICK_TEMPORARY_PATH").unwrap());
    // A stand-in binary that records every argument it is given, and what
    // both temporary directories hold as it starts, and runs the real
    // ImageMagick with them.
    let log = scratch.join("argv.log");
    let listing = scratch.join("listing.log");
    let wrapper = scratch.join("magick-recording");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{log}'\nprintf -- '--end--\\n' >> '{log}'\n\
             ls -A \"$TMPDIR\" \"$MAGICK_TEMPORARY_PATH\" | grep -v -e ':$' -e '^$' >> '{listing}'\n\
             exec magick \"$@\"\n",
            log = log.display(),
            listing = listing.display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    suprnova::register_transformation("img-004-argv-probe", Ok);

    let p3 = display_p3();
    let source = jpeg_with(
        &convert(
            &png_of(WIDTH, HEIGHT, pattern(WIDTH, HEIGHT)),
            OutputFormat::Jpeg,
            100,
        ),
        &[
            exif_segment(&exif_tiff(Some(6), true)),
            icc_segments(&p3).remove(0),
        ],
    );
    let mut config = ImageConfig::default();
    config.auto_orient = false;
    suprnova::media::set_config_for_tests(Some(config));
    let out = MagickCliDriver::new(wrapper.to_string_lossy())
        .process(
            &source,
            &pipeline(
                vec![
                    Transformation::Grayscale,
                    Transformation::custom("img-004-argv-probe"),
                    Transformation::FlipVertically,
                ],
                OutputFormat::Jpeg,
            ),
        )
        .expect("the pipeline runs");

    let argv = std::fs::read_to_string(&log).unwrap();
    let runs: Vec<&str> = argv
        .split("--end--\n")
        .filter(|run| !run.is_empty())
        .collect();
    assert_eq!(
        runs.len(),
        2,
        "one custom step splits the pipeline in two runs: {argv}"
    );
    assert!(
        !argv.contains("img-004"),
        "a custom step reached the arguments: {argv}"
    );
    assert!(
        runs[1].lines().any(|arg| arg == "png:-[0]"),
        "the second run reads stdin: {argv}"
    );
    // No run reads or writes a file: the image goes in over stdin and out
    // over stdout, so no argument names a path.
    assert!(
        argv.lines().all(|arg| !arg.contains('/')),
        "an argument names a file: {argv}"
    );
    // The driver put no file in either temporary directory before a run
    // started, and none is left in either once the pipeline is done.
    // (ImageMagick keeps its own copy of stdin in its directory while a run
    // reads it, and removes it as the run ends.)
    assert_eq!(
        std::fs::read_to_string(&listing).unwrap(),
        "",
        "a temporary directory held a file as a run started"
    );
    for dir in [&rust_tmp, &magick_tmp] {
        assert_eq!(
            std::fs::read_dir(dir).unwrap().count(),
            0,
            "{} holds a file after the pipeline",
            dir.display()
        );
    }
    // The pipeline with a custom step ends as one without: the opt-out's
    // Orientation-only EXIF is there, and nothing else of the metadata.
    assert_eq!(
        metadata_found(OutputFormat::Jpeg, &out),
        vec!["orientation-only EXIF".to_string()]
    );
}

/// A custom step does not cost a pipeline its profile, or the opt-out's
/// orientation tag.
fn assert_a_custom_step_keeps_the_profile_and_the_tag(driver: &dyn ImageDriver) {
    suprnova::register_transformation("img-004-identity", Ok);
    let p3 = display_p3();
    let rgba = pattern(WIDTH, HEIGHT);
    let png = png_of(WIDTH, HEIGHT, rgba);
    let source = jpeg_with(
        &convert(&png, OutputFormat::Jpeg, 100),
        &[
            exif_segment(&exif_tiff(Some(6), false)),
            icc_segments(&p3).remove(0),
        ],
    );
    let _config = opt_out();
    for target in [OutputFormat::Jpeg, OutputFormat::Png, OutputFormat::WebP] {
        let out = driver
            .process(
                &source,
                &pipeline(vec![Transformation::custom("img-004-identity")], target),
            )
            .unwrap();
        assert_eq!(
            output_profile(target, &out).as_deref(),
            Some(&p3[..]),
            "{target:?} through {}",
            driver.name()
        );
        assert_eq!(
            metadata_found(target, &out),
            vec!["orientation-only EXIF".to_string()],
            "{target:?} through {}",
            driver.name()
        );
    }
}

#[test]
#[serial]
fn img_004_a_custom_step_keeps_the_profile_and_the_tag_under_oxideav() {
    assert_a_custom_step_keeps_the_profile_and_the_tag(oxideav().as_ref());
}

#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_004_a_custom_step_keeps_the_profile_and_the_tag_under_magick() {
    assert_a_custom_step_keeps_the_profile_and_the_tag(magick().as_ref());
}

// ───────────────────────── IMG-005 ─────────────────────────

fn is_generated(name: &str, extension: &str) -> bool {
    let Some(stem) = name.strip_suffix(&format!(".{extension}")) else {
        return false;
    };
    stem.len() == 40 && stem.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

#[cfg(feature = "testing")]
#[tokio::test]
#[serial]
async fn img_005_store_and_store_as_return_the_path_they_wrote() {
    use suprnova::DiskExt;
    let _guard = suprnova::Storage::fake();
    suprnova::Storage::register_memory("img-005");
    let disk = suprnova::Storage::disk("img-005").unwrap();
    let source = png_of(WIDTH, HEIGHT, pattern(WIDTH, HEIGHT));
    let image = Image::from_bytes(source.clone()).resize(3, 2);

    let first = image
        .clone()
        .to_webp()
        .store("avatars/", Some("img-005"))
        .await
        .unwrap();
    let second = image
        .clone()
        .to_webp()
        .store("/avatars", Some("img-005"))
        .await
        .unwrap();
    for path in [&first, &second] {
        let name = path.strip_prefix("avatars/").expect("in the directory");
        assert!(is_generated(name, "webp"), "{path}");
    }
    assert_ne!(first, second, "two images in one directory got one name");
    let expected = image.clone().to_webp().to_bytes().await.unwrap();
    assert_eq!(disk.get(&first).await.unwrap(), expected);

    // The source format's extension when the pipeline does not convert.
    let kept = image.clone().store("", Some("img-005")).await.unwrap();
    assert!(is_generated(&kept, "png"), "{kept}");

    let named = image
        .clone()
        .store_as("avatars", "me.png", Some("img-005"))
        .await
        .unwrap();
    assert_eq!(named, "avatars/me.png");
    assert_eq!(
        disk.get("avatars/me.png").await.unwrap(),
        image.to_bytes().await.unwrap()
    );
}

/// The compile-fail cases: today's `store(disk, path)` (IMG-005), a
/// struct literal of `ImageConfig` and an exhaustive match on
/// `Transformation` (IMG-008) must not compile outside the crate.
#[test]
fn img_005_and_img_008_the_breaking_forms_do_not_compile() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/media/compile_fail/*.rs");
}

// ───────────────────────── IMG-006 ─────────────────────────

/// The pixel at `(x, y)` of a PNG.
fn pixel_at(png: &[u8], x: u32, y: u32) -> [u8; 4] {
    let (width, _, data) = png_pixels(png);
    let at = ((y * width + x) * 4) as usize;
    data[at..at + 4].try_into().unwrap()
}

/// `out` read back as a PNG, through the built-in driver.
fn as_png(out: &[u8]) -> Vec<u8> {
    OxideAvImageDriver::new()
        .process(out, &pipeline(vec![], OutputFormat::Png))
        .unwrap()
}

fn close(actual: [u8; 4], want: [u8; 3], tolerance: u8) -> bool {
    actual[..3]
        .iter()
        .zip(want)
        .all(|(channel, want)| channel.abs_diff(want) <= tolerance)
}

/// Rotation corners and flattened transparency take the format's default,
/// or the colour the caller named.
fn assert_rotation_backgrounds(driver: &dyn ImageDriver) {
    let name = driver.name();
    let red = png_of(64, 64, flat(64, 64, [220, 20, 20, 255]));
    // Left half transparent, right half opaque red.
    let mut half = flat(64, 64, [220, 20, 20, 255]);
    for y in 0..64 {
        for x in 0..32 {
            half[(y * 64 + x) * 4 + 3] = 0;
        }
    }
    let half = png_of(64, 64, half);
    let turn = |background: Option<Color>| Transformation::Rotate {
        degrees: 45.0,
        background,
    };

    for (target, tolerance) in [(OutputFormat::Jpeg, 8), (OutputFormat::Gif, 0)] {
        let out = as_png(
            &driver
                .process(&red, &pipeline(vec![turn(None)], target))
                .unwrap(),
        );
        assert!(
            close(pixel_at(&out, 0, 0), [255, 255, 255], tolerance),
            "{target:?} corner through {name}: {:?}",
            pixel_at(&out, 0, 0)
        );
        let out = as_png(&driver.process(&half, &pipeline(vec![], target)).unwrap());
        assert!(
            close(pixel_at(&out, 4, 32), [255, 255, 255], tolerance),
            "{target:?} flattened through {name}: {:?}",
            pixel_at(&out, 4, 32)
        );
        let blue = Some(Color::rgb(0, 0, 255));
        let out = as_png(
            &driver
                .process(&red, &pipeline(vec![turn(blue)], target))
                .unwrap(),
        );
        assert!(
            close(pixel_at(&out, 0, 0), [0, 0, 255], tolerance),
            "{target:?} named corner through {name}: {:?}",
            pixel_at(&out, 0, 0)
        );
        let quarter = Transformation::Rotate {
            degrees: 90.0,
            background: blue,
        };
        let out = as_png(
            &driver
                .process(&half, &pipeline(vec![quarter], target))
                .unwrap(),
        );
        // A quarter turn moves the transparent left half to the top.
        assert!(
            close(pixel_at(&out, 32, 4), [0, 0, 255], tolerance),
            "{target:?} flattened onto the named colour through {name}: {:?}",
            pixel_at(&out, 32, 4)
        );
    }
    // An opaque WebP is written lossy, so its named corner is blue within
    // that loss; the transparent default keeps it lossless.
    for (target, tolerance) in [
        (OutputFormat::Png, 0),
        (OutputFormat::WebP, 24),
        (OutputFormat::Bmp, 0),
    ] {
        let out = as_png(
            &driver
                .process(&red, &pipeline(vec![turn(None)], target))
                .unwrap(),
        );
        assert_eq!(
            pixel_at(&out, 0, 0)[3],
            0,
            "{target:?} corner through {name}"
        );
        let named = Some(Color::rgb(0, 0, 255));
        let out = as_png(
            &driver
                .process(&red, &pipeline(vec![turn(named)], target))
                .unwrap(),
        );
        let corner = pixel_at(&out, 0, 0);
        assert!(
            close(corner, [0, 0, 255], tolerance) && corner[3] == 255,
            "{target:?} named corner through {name}: {corner:?}"
        );
    }
}

#[test]
#[serial]
fn img_006_oxideav_fills_corners_and_flattens_by_format() {
    assert_rotation_backgrounds(oxideav().as_ref());
}

#[test]
#[serial]
#[ignore = "requires a host ImageMagick 7 binary"]
fn img_006_magick_fills_corners_and_flattens_by_format() {
    assert_rotation_backgrounds(magick().as_ref());
}

#[tokio::test]
#[serial]
async fn img_006_the_facade_takes_a_background_colour() {
    let red = png_of(16, 16, flat(16, 16, [220, 20, 20, 255]));
    let out = Image::from_bytes(red)
        .rotate_with_background(45.0, Color::from_hex("#00ff00").unwrap())
        .to_png()
        .to_bytes()
        .await
        .unwrap();
    assert_eq!(pixel_at(&out, 0, 0), [0, 255, 0, 255]);
}

// ───────────────────────── IMG-007 ─────────────────────────

#[tokio::test]
#[serial]
async fn img_007_the_terminals_agree_with_to_bytes_and_dimensions() {
    use base64::Engine as _;
    let image = Image::from_bytes(png_of(WIDTH, HEIGHT, pattern(WIDTH, HEIGHT))).resize(5, 3);
    let bytes = image.clone().to_bytes().await.unwrap();
    let encoded = image.clone().to_base64().await.unwrap();
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .unwrap(),
        bytes
    );
    let mime = image.clone().mime_type().await.unwrap();
    assert_eq!(
        image.clone().to_data_uri().await.unwrap(),
        format!("data:{mime};base64,{encoded}")
    );
    let (width, height) = image.clone().dimensions().await.unwrap();
    assert_eq!(image.clone().width().await.unwrap(), width);
    assert_eq!(image.clone().height().await.unwrap(), height);

    // The aliases mirror on the axis Laravel's do.
    let flip = as_png(&image.clone().flip().to_png().to_bytes().await.unwrap());
    let vertically = as_png(
        &image
            .clone()
            .flip_vertically()
            .to_png()
            .to_bytes()
            .await
            .unwrap(),
    );
    let flop = as_png(&image.clone().flop().to_png().to_bytes().await.unwrap());
    let horizontally = as_png(
        &image
            .clone()
            .flip_horizontally()
            .to_png()
            .to_bytes()
            .await
            .unwrap(),
    );
    assert_eq!(flip, vertically);
    assert_eq!(flop, horizontally);
    assert_ne!(flip, flop);
}

// ───────────────────────── IMG-008 ─────────────────────────

#[test]
fn img_008_the_manual_documents_the_image_api() {
    let images = include_str!("../../../manual/images.md");
    let env_vars = include_str!("../../../manual/env-vars.md");
    let prose = images.split_whitespace().collect::<Vec<_>>().join(" ");
    for topic in [
        "## Orientation",
        "## Metadata",
        "IMAGE_AUTO_ORIENT",
        "There is no argument position user input can reach.",
    ] {
        assert!(prose.contains(topic), "manual/images.md lacks {topic:?}");
    }
    for method in [
        "orient()",
        "using(",
        "transform(",
        "register_transformation",
        ".store(",
        "store_as(",
        "rotate_with_background(",
        "to_base64()",
        "to_data_uri()",
        "width()",
        "height()",
        "to_webp()",
        "to_jpg()",
        "to_jpeg()",
        "to_png()",
        "to_gif()",
        "to_bmp()",
        "optimize(",
        "flip()",
        "flop()",
    ] {
        assert!(images.contains(method), "manual/images.md lacks {method:?}");
    }
    assert!(
        env_vars.contains("`IMAGE_AUTO_ORIENT`"),
        "manual/env-vars.md lacks IMAGE_AUTO_ORIENT"
    );
}
