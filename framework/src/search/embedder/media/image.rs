use super::{bounded, error};
use crate::FrameworkError;
use image::{ImageReader, RgbImage};
use std::io::Cursor;

/// Patch tensors retain padding positions so encoders can exclude synthetic pixels.
#[derive(Clone, Debug)]
pub struct ImageFeatures {
    /// RGB values are patch-major, with 768 values per patch.
    pub pixel_values: Vec<f32>,
    /// Each patch has an x,y position; padding uses -1 for both coordinates.
    pub position_ids: Vec<i64>,
    /// Real pooled tokens determine the placeholder count in the text graph.
    pub soft_tokens: usize,
}

/// Match the shipped processor's RGB conversion, patch grid and pixel rescaling.
pub fn preprocess_search_image(
    bytes: &[u8],
    budget: usize,
) -> Result<ImageFeatures, FrameworkError> {
    bounded(bytes)?;
    if ![70, 140, 280, 560, 1120].contains(&budget) {
        return Err(error("unsupported image soft token budget"));
    }
    let image = decode(bytes)?;
    let (width, height) = dimensions(image.width() as usize, image.height() as usize, budget)?;
    let rgb = resize(&image, width, height);
    let patch_width = width / 16;
    let patch_height = height / 16;
    let mut pixel_values = vec![0.0; budget * 9 * 768];
    let mut position_ids = vec![-1; budget * 9 * 2];
    for py in 0..patch_height {
        for px in 0..patch_width {
            let patch = py * patch_width + px;
            position_ids[patch * 2] = px as i64;
            position_ids[patch * 2 + 1] = py as i64;
            for y in 0..16 {
                for x in 0..16 {
                    for channel in 0..3 {
                        pixel_values[patch * 768 + (y * 16 + x) * 3 + channel] =
                            rgb[((py * 16 + y) * width + px * 16 + x) * 3 + channel] as f32 / 255.0;
                    }
                }
            }
        }
    }
    Ok(ImageFeatures {
        pixel_values,
        position_ids,
        soft_tokens: patch_width * patch_height / 9,
    })
}

fn decode(bytes: &[u8]) -> Result<RgbImage, FrameworkError> {
    if bytes.starts_with(&[0xff, 0xd8]) {
        use zune_jpeg::zune_core::{
            bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions,
        };
        let options = DecoderOptions::default()
            .set_max_width(8192)
            .set_max_height(8192)
            .jpeg_set_out_colorspace(ColorSpace::RGB);
        let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
        decoder.decode_headers().map_err(error)?;
        let (width, height) = decoder
            .dimensions()
            .ok_or_else(|| error("JPEG dimensions missing"))?;
        if width * height > 16_777_216 {
            return Err(error("image exceeds 16777216 pixels"));
        }
        return RgbImage::from_raw(
            width as u32,
            height as u32,
            decoder.decode().map_err(error)?,
        )
        .ok_or_else(|| error("JPEG RGB buffer length differs"));
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(error)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    Ok(reader.decode().map_err(error)?.to_rgb8())
}

fn dimensions(
    width: usize,
    height: usize,
    budget: usize,
) -> Result<(usize, usize), FrameworkError> {
    if width == 0 || height == 0 {
        return Err(error("empty image"));
    }
    let factor = ((budget * 9 * 256) as f64 / (width * height) as f64).sqrt();
    let mut w = (width as f64 * factor / 48.0).floor() as usize * 48;
    let mut h = (height as f64 * factor / 48.0).floor() as usize * 48;
    if h == 0 {
        h = 48;
        w = (width / height * 48).min(budget * 48);
    }
    if w == 0 {
        w = 48;
        h = (height / width * 48).min(budget * 48);
    }
    if w == 0 || h == 0 || w * h > budget * 9 * 256 {
        return Err(error("invalid image patch grid"));
    }
    Ok((w, h))
}

// Pillow rounds each separable pass to uint8. Keeping that rounding here
// avoids a different embedding for the same uploaded image.
fn resize(image: &RgbImage, width: usize, height: usize) -> Vec<u8> {
    let source_width = image.width() as usize;
    let source_height = image.height() as usize;
    let horizontal = coefficients(source_width, width);
    let vertical = coefficients(source_height, height);
    let mut intermediate = vec![0_u8; width * source_height * 3];
    for y in 0..source_height {
        for (x, (start, weights)) in horizontal.iter().enumerate() {
            for c in 0..3 {
                let sum = weights
                    .iter()
                    .enumerate()
                    .fold(1_i64 << 21, |sum, (i, weight)| {
                        sum + image.as_raw()[(y * source_width + start + i) * 3 + c] as i64 * weight
                    });
                intermediate[(y * width + x) * 3 + c] = (sum >> 22).clamp(0, 255) as u8;
            }
        }
    }
    let mut output = vec![0_u8; width * height * 3];
    for (y, (start, weights)) in vertical.iter().enumerate() {
        for x in 0..width {
            for c in 0..3 {
                let sum = weights
                    .iter()
                    .enumerate()
                    .fold(1_i64 << 21, |sum, (i, weight)| {
                        sum + intermediate[((start + i) * width + x) * 3 + c] as i64 * weight
                    });
                output[(y * width + x) * 3 + c] = (sum >> 22).clamp(0, 255) as u8;
            }
        }
    }
    output
}

fn coefficients(source: usize, target: usize) -> Vec<(usize, Vec<i64>)> {
    let scale = source as f64 / target as f64;
    let filter_scale = scale.max(1.0);
    let support = 2.0 * filter_scale;
    (0..target)
        .map(|out| {
            let center = (out as f64 + 0.5) * scale;
            let start = ((center - support + 0.5) as isize).max(0) as usize;
            let end = ((center + support + 0.5) as usize).min(source);
            let weights: Vec<_> = (start..end)
                .map(|i| cubic((i as f64 - center + 0.5) / filter_scale))
                .collect();
            let total: f64 = weights.iter().sum();
            (
                start,
                weights
                    .into_iter()
                    .map(|weight| (weight / total * (1_u64 << 22) as f64).round() as i64)
                    .collect(),
            )
        })
        .collect()
}

fn cubic(value: f64) -> f64 {
    let x = value.abs();
    if x < 1.0 {
        ((1.5 * x - 2.5) * x) * x + 1.0
    } else if x < 2.0 {
        ((-0.5 * x + 2.5) * x - 4.0) * x + 2.0
    } else {
        0.0
    }
}
