//! Image preparation for static publication, before any object is uploaded.

use std::collections::{BTreeMap, HashSet};

use bytes::Bytes;
use html5ever::tokenizer::TagKind;
use sha2::{Digest, Sha256};

use super::assets::{
    attribute, escape, is_raster, is_vector_or_icon, local_path, parse_srcset, serialize_tag,
    set_attribute, tags, with_descriptor,
};
use crate::error::FrameworkError;
use crate::media::{Image, MagickCliDriver, OutputFormat, PngEffort};

/// Image settings for one publication.
#[derive(Debug, Clone)]
pub struct ImageOptions {
    /// Lossy encode quality. Publication requires `85..=100`.
    pub quality: u8,
    /// Also publish AVIF. Requires the configured `magick` image driver.
    pub avif: bool,
    /// Source paths whose PNG format the application requires.
    pub keep_png: HashSet<String>,
    /// Page URL used to resolve relative image and manifest references.
    pub document_path: String,
}

impl Default for ImageOptions {
    fn default() -> Self {
        Self {
            quality: 85,
            avif: false,
            keep_png: HashSet::new(),
            document_path: "/".into(),
        }
    }
}

/// One fingerprinted object, ready for the publication's upload sequence.
#[derive(Debug, Clone)]
pub struct ImageObject {
    /// Disk path, with no leading slash.
    pub path: String,
    /// Metadata-stripped image or rewritten web manifest.
    pub bytes: Bytes,
    /// The object's HTTP media type.
    pub content_type: String,
}

/// Rewritten document and every generated object it references.
#[derive(Debug)]
pub struct PreparedImages {
    /// UTF-8 HTML with format alternatives and stripped original fallbacks.
    pub html: Bytes,
    /// Fingerprinted objects, deduplicated by their content.
    pub assets: Vec<ImageObject>,
    /// Original image references mapped to their stripped fallback URL.
    /// Apply these exact replacements to Inertia page JSON strings too.
    pub replacements: BTreeMap<String, String>,
}

#[derive(Clone)]
struct Variants {
    original: String,
    webp: Option<String>,
    avif: Option<String>,
}

struct Preparation<'a> {
    source: &'a opendal::Operator,
    options: &'a ImageOptions,
    variants: BTreeMap<(String, bool), Variants>,
    objects: BTreeMap<String, ImageObject>,
    replacements: BTreeMap<String, String>,
}

impl Preparation<'_> {
    fn object(&mut self, bytes: Vec<u8>, extension: &str, content_type: &str) -> String {
        let digest = hex::encode(Sha256::digest(&bytes));
        let path = format!("static/images/{digest}.{extension}");
        self.objects.entry(path.clone()).or_insert(ImageObject {
            path: path.clone(),
            bytes: bytes.into(),
            content_type: content_type.into(),
        });
        format!("/{path}")
    }

    async fn read(&self, path: &str) -> Result<Bytes, FrameworkError> {
        let metadata = self.source.stat(path).await.map_err(|error| {
            FrameworkError::internal(format!("publication image `{path}`: {error}"))
        })?;
        if metadata.content_length() > crate::media::config().max_alloc_bytes {
            return Err(FrameworkError::param(format!(
                "publication image `{path}` exceeds IMAGE_MAX_ALLOC_BYTES"
            )));
        }
        self.source
            .read(path)
            .await
            .map(|buffer| buffer.to_bytes())
            .map_err(|error| {
                FrameworkError::internal(format!("publication image `{path}`: {error}"))
            })
    }

    async fn image(&mut self, path: &str, keep: bool) -> Result<Variants, FrameworkError> {
        let keep = keep
            || self.options.keep_png.contains(path)
            || self.options.keep_png.contains(&format!("/{path}"));
        let key = (path.to_string(), keep);
        if let Some(variants) = self.variants.get(&key) {
            return Ok(variants.clone());
        }
        let bytes = self.read(path).await?;
        let mime = infer::get(&bytes)
            .map(|kind| kind.mime_type())
            .ok_or_else(|| {
                FrameworkError::param(format!("publication image `{path}` has an unknown format"))
            })?;
        let original_format = match mime {
            "image/png" => OutputFormat::Png,
            "image/jpeg" => OutputFormat::Jpeg,
            "image/webp" => OutputFormat::WebP,
            "image/gif" => OutputFormat::Gif,
            "image/bmp" | "image/x-ms-bmp" => OutputFormat::Bmp,
            _ => {
                return Err(FrameworkError::param(format!(
                    "publication image `{path}` cannot retain its original format `{mime}`"
                )));
            }
        };
        let pipeline = Image::from_bytes(bytes).quality(self.options.quality);
        let original = if original_format == OutputFormat::Png && keep {
            pipeline
                .clone()
                .to_png()
                .recompress_png(PngEffort::Maximum)
                .to_bytes()
                .await?
        } else {
            pipeline
                .clone()
                .to_format(original_format)
                .to_bytes()
                .await?
        };
        let original = self.object(
            original,
            original_format.extension(),
            original_format.mime_type(),
        );
        let (webp, avif) = if original_format == OutputFormat::Png && keep {
            (None, None)
        } else {
            let webp = pipeline.clone().to_webp().to_bytes().await?;
            let webp = Some(self.object(webp, "webp", "image/webp"));
            let avif = if self.options.avif {
                let png = pipeline.to_png().to_bytes().await?;
                let quality = self.options.quality;
                let bytes = tokio::task::spawn_blocking(move || {
                    MagickCliDriver::from_env().publication_avif(&png, quality)
                })
                .await
                .map_err(|error| {
                    FrameworkError::internal(format!("publication AVIF worker failed: {error}"))
                })??;
                Some(self.object(bytes, "avif", "image/avif"))
            } else {
                None
            };
            (webp, avif)
        };
        let variants = Variants {
            original,
            webp,
            avif,
        };
        self.replacements
            .insert(path.to_string(), variants.original.clone());
        self.replacements
            .insert(format!("/{path}"), variants.original.clone());
        self.variants.insert(key, variants.clone());
        Ok(variants)
    }

    async fn manifest(&mut self, path: &str) -> Result<String, FrameworkError> {
        let bytes = self.read(path).await?;
        let mut manifest: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
            FrameworkError::param(format!(
                "publication manifest `{path}` is invalid JSON: {error}"
            ))
        })?;
        if let Some(icons) = manifest
            .get_mut("icons")
            .and_then(serde_json::Value::as_array_mut)
        {
            for icon in icons {
                if let Some(src) = icon.get("src").and_then(serde_json::Value::as_str)
                    && let Some(icon_path) = local_path(src, &format!("/{path}"))?
                    && is_raster(&icon_path)
                {
                    let variants = self.image(&icon_path, true).await?;
                    icon["src"] = variants.original.into();
                }
            }
        }
        let bytes = serde_json::to_vec(&manifest).map_err(|error| {
            FrameworkError::internal(format!(
                "publication manifest serialization failed: {error}"
            ))
        })?;
        Ok(self.object(bytes, "webmanifest", "application/manifest+json"))
    }

    async fn candidates(
        &mut self,
        value: &str,
        keep: bool,
        srcset: bool,
    ) -> Result<(String, Option<String>, Option<String>), FrameworkError> {
        let candidates = if srcset {
            parse_srcset(value)
        } else {
            vec![(value.to_string(), String::new())]
        };
        let mut original = Vec::new();
        let mut webp = Vec::new();
        let mut avif = Vec::new();
        let mut all_webp = true;
        let mut all_avif = true;
        for (url, descriptor) in candidates {
            let variants = if let Some(path) = local_path(&url, &self.options.document_path)?
                && !is_vector_or_icon(&path)
            {
                Some(self.image(&path, keep).await?)
            } else {
                None
            };
            if let Some(variants) = variants {
                self.replacements
                    .insert(url.clone(), variants.original.clone());
                original.push(with_descriptor(&variants.original, &descriptor));
                if let Some(url) = variants.webp {
                    webp.push(with_descriptor(&url, &descriptor));
                } else {
                    all_webp = false;
                }
                if let Some(url) = variants.avif {
                    avif.push(with_descriptor(&url, &descriptor));
                } else {
                    all_avif = false;
                }
            } else {
                original.push(with_descriptor(&url, &descriptor));
                all_webp = false;
                all_avif = false;
            }
        }
        let original = original.join(", ");
        if original != value {
            self.replacements.insert(value.to_owned(), original.clone());
        }
        Ok((
            original,
            (all_webp && !webp.is_empty()).then(|| webp.join(", ")),
            (all_avif && !avif.is_empty()).then(|| avif.join(", ")),
        ))
    }
}

/// Prepare local document images before switching a publication live.
///
/// Recognizes HTML image elements, responsive candidates, picture sources,
/// icon links and web-manifest icons. Remote and data URLs stay unchanged.
/// The source operator is rooted at the application's public directory.
/// Returns errors for unreadable or malformed local images, invalid paths,
/// quality below 85 or AVIF without the configured Magick driver.
pub async fn prepare(
    html: &[u8],
    source: &opendal::Operator,
    options: &ImageOptions,
) -> Result<PreparedImages, FrameworkError> {
    if !(85..=100).contains(&options.quality) {
        return Err(FrameworkError::param(
            "publication image quality must be 85..=100",
        ));
    }
    if options.avif && crate::media::default_driver()?.name() != "magick" {
        return Err(FrameworkError::param(
            "publication AVIF requires IMAGE_DRIVER=magick and an ImageMagick AVIF encoder",
        ));
    }
    let html = std::str::from_utf8(html).map_err(|error| {
        FrameworkError::param(format!("publication HTML is not UTF-8: {error}"))
    })?;
    let tags = tags(html);
    let mut preparation = Preparation {
        source,
        options,
        variants: BTreeMap::new(),
        objects: BTreeMap::new(),
        replacements: BTreeMap::new(),
    };
    let mut output = String::with_capacity(html.len());
    let mut cursor = 0;
    let mut picture_depth = 0usize;
    for (start, end, mut tag) in tags {
        if tag.name == "picture" {
            if tag.kind == TagKind::StartTag {
                picture_depth += 1;
            } else {
                picture_depth = picture_depth.saturating_sub(1);
            }
        }
        if tag.kind != TagKind::StartTag {
            continue;
        }
        let name = tag.name.to_string();
        let keep = attribute(&tag, "data-static-keep-png").is_some();
        let mut sources = String::new();
        let mut changed = false;
        if name == "img" || name == "source" && picture_depth > 0 {
            let mut alternatives = None;
            for field in ["src", "srcset"] {
                if let Some(value) = attribute(&tag, field).map(str::to_string) {
                    let (original, webp, avif) = preparation
                        .candidates(&value, keep, field == "srcset")
                        .await?;
                    set_attribute(&mut tag, field, &original);
                    if field == "srcset" || webp.is_some() || avif.is_some() {
                        alternatives = Some((webp, avif));
                    }
                    changed |= original != value;
                }
            }
            if let Some((webp, avif)) = alternatives {
                for (mime, value) in [("image/avif", avif), ("image/webp", webp)] {
                    if let Some(value) = value {
                        sources.push_str(&format!(
                            "<source type=\"{mime}\" srcset=\"{}\"",
                            escape(&value)
                        ));
                        for field in ["media", "sizes"] {
                            if let Some(value) = attribute(&tag, field) {
                                sources.push_str(&format!(" {field}=\"{}\"", escape(value)));
                            }
                        }
                        sources.push('>');
                    }
                }
            }
        } else if name == "link" {
            let rel = attribute(&tag, "rel")
                .unwrap_or_default()
                .to_ascii_lowercase();
            if let Some(href) = attribute(&tag, "href")
                && let Some(path) = local_path(href, &options.document_path)?
            {
                let replacement = if rel.split_ascii_whitespace().any(|part| part == "manifest") {
                    Some(preparation.manifest(&path).await?)
                } else if rel
                    .split_ascii_whitespace()
                    .any(|part| part == "icon" || part == "apple-touch-icon")
                    && is_raster(&path)
                {
                    Some(preparation.image(&path, true).await?.original)
                } else {
                    None
                };
                if let Some(replacement) = replacement {
                    set_attribute(&mut tag, "href", &replacement);
                    changed = true;
                }
            }
        }
        if changed {
            output.push_str(&html[cursor..start]);
            let wrap = name == "img" && picture_depth == 0 && !sources.is_empty();
            if wrap {
                output.push_str("<picture>");
            }
            output.push_str(&sources);
            output.push_str(&serialize_tag(&tag));
            if wrap {
                output.push_str("</picture>");
            }
            cursor = end;
        }
    }
    output.push_str(&html[cursor..]);
    Ok(PreparedImages {
        html: output.into(),
        assets: preparation.objects.into_values().collect(),
        replacements: preparation.replacements,
    })
}
