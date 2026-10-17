use super::{bounded, error};
use crate::FrameworkError;
use std::{f64::consts::PI, io::Cursor};

/// The frame mask must travel with the features to avoid embedding padded silence.
#[derive(Clone, Debug)]
pub struct AudioFeatures {
    /// Frame-major log-mel magnitudes have 128 bins per frame.
    pub features: Vec<f32>,
    /// One byte per frame identifies real waveform windows.
    pub mask: Vec<u8>,
}

/// Decode bounded PCM WAV and reproduce the shipped 16 kHz log-mel processor.
pub fn preprocess_search_audio(bytes: &[u8]) -> Result<AudioFeatures, FrameworkError> {
    bounded(bytes)?;
    let mut reader = hound::WavReader::new(Cursor::new(bytes)).map_err(error)?;
    let spec = reader.spec();
    if spec.channels == 0 || spec.channels > 8 || !(8000..=192000).contains(&spec.sample_rate) {
        return Err(error("WAV requires 1 to 8 channels and 8000 to 192000 Hz"));
    }
    if reader.duration() as u64 > spec.sample_rate as u64 * 30 {
        return Err(error("audio exceeds 30 seconds"));
    }
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .map(|v| v.map_err(error))
            .collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            if spec.bits_per_sample == 0 || spec.bits_per_sample > 32 {
                return Err(error("unsupported PCM bit depth"));
            }
            let divisor = (1_u64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|v| v.map(|n| n as f32 / divisor).map_err(error))
                .collect::<Result<_, _>>()?
        }
    };
    if samples.is_empty()
        || samples.iter().any(|v| !v.is_finite())
        || samples.len() % spec.channels as usize != 0
    {
        return Err(error("WAV samples are empty, nonfinite or incomplete"));
    }
    let mono: Vec<f32> = samples
        .chunks_exact(spec.channels as usize)
        .map(|frame| frame.iter().sum::<f32>() / spec.channels as f32)
        .collect();
    let waveform = resample(&mono, spec.sample_rate);
    extract(&waveform)
}

// Float64 avoids CPU-specific float32 convolution noise in quiet mel bins.
fn resample(samples: &[f32], rate: u32) -> Vec<f32> {
    if rate == 16000 {
        return samples.to_vec();
    }
    let mut divisor = rate;
    let mut other = 16000;
    while other != 0 {
        (divisor, other) = (other, divisor % other);
    }
    let original = rate / divisor;
    let target = 16000 / divisor;
    let base = original.min(target) as f64 * 0.99;
    let cutoff = base / original as f64;
    let width = (6.0 / cutoff).ceil() as isize;
    let length = (samples.len() as u64 * 16000).div_ceil(rate as u64) as usize;
    (0..length)
        .map(|index| {
            let phase = index % target as usize;
            let cycle = index / target as usize;
            let center = index as f64 * rate as f64 / 16000.0;
            let mut sum = 0.0_f64;
            for source in (center.floor() as isize - width)..=(center.ceil() as isize + width) {
                if source < 0 || source >= samples.len() as isize {
                    continue;
                }
                let offset = source - (cycle * original as usize) as isize;
                let mut t =
                    (-(phase as f64) / target as f64 + offset as f64 / original as f64) * base;
                t = t.clamp(-6.0, 6.0);
                let window = (t * PI / 6.0 / 2.0).cos().powi(2);
                t *= PI;
                let sinc = if t == 0.0 { 1.0 } else { t.sin() / t };
                sum += samples[source as usize] as f64 * (sinc * (window * cutoff));
            }
            sum as f32
        })
        .collect()
}

fn extract(waveform: &[f32]) -> Result<AudioFeatures, FrameworkError> {
    let padded = waveform.len().div_ceil(128) * 128;
    if padded < 161 {
        return Err(error("audio requires at least 161 padded samples"));
    }
    let count = (padded - 161) / 160 + 1;
    let mut features = Vec::with_capacity(count * 128);
    let mut mask = Vec::with_capacity(count);
    let window: Vec<f32> = (0..320)
        .map(|n| (0.5 - 0.5 * (2.0 * PI * n as f64 / 320.0).cos()) as f32)
        .collect();
    let max_mel = 2595.0 * (1.0_f64 + 8000.0 / 700.0).log10();
    let edges: Vec<_> = (0..130)
        .map(|n| 700.0 * (10_f64.powf(max_mel * n as f64 / 129.0 / 2595.0) - 1.0))
        .collect();
    let filters: Vec<Vec<f64>> = (0..128)
        .map(|m| {
            (0..257)
                .map(|bin| {
                    let frequency = bin as f64 * 31.25;
                    ((frequency - edges[m]) / (edges[m + 1] - edges[m]))
                        .min((edges[m + 2] - frequency) / (edges[m + 2] - edges[m + 1]))
                        .max(0.0)
                })
                .collect()
        })
        .collect();
    for frame in 0..count {
        let valid = frame * 160 + 160 < waveform.len();
        mask.push(u8::from(valid));
        let mut real = [0.0; 512];
        let mut imaginary = [0.0; 512];
        for n in 0..320 {
            let index = (frame * 160 + n) as isize - 160;
            real[n] = if index >= 0 {
                (waveform.get(index as usize).copied().unwrap_or(0.0) * window[n]) as f64
            } else {
                0.0
            };
        }
        fft(&mut real, &mut imaginary);
        let magnitude: Vec<_> = (0..257).map(|i| real[i].hypot(imaginary[i])).collect();
        for filter in &filters {
            let value = (magnitude
                .iter()
                .zip(filter)
                .map(|(a, b)| a * b)
                .sum::<f64>()
                + 0.001)
                .ln() as f32;
            features.push(if valid { value } else { 0.0 });
        }
    }
    Ok(AudioFeatures { features, mask })
}

fn fft(real: &mut [f64; 512], imaginary: &mut [f64; 512]) {
    for index in 0_usize..512 {
        let reversed = index.reverse_bits() >> (usize::BITS - 9);
        if index < reversed {
            real.swap(index, reversed);
        }
    }
    let mut size = 2;
    while size <= 512 {
        for start in (0..512).step_by(size) {
            for offset in 0..size / 2 {
                let angle = -2.0 * PI * offset as f64 / size as f64;
                let (sin, cos) = angle.sin_cos();
                let a = start + offset;
                let b = a + size / 2;
                let r = real[b] * cos - imaginary[b] * sin;
                let i = real[b] * sin + imaginary[b] * cos;
                real[b] = real[a] - r;
                imaginary[b] = imaginary[a] - i;
                real[a] += r;
                imaginary[a] += i;
            }
        }
        size *= 2;
    }
}
