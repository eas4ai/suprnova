//! Asset references share one HTML parser with publication image rewriting.

use crate::FrameworkError;
use html5ever::TokenizerResult;
use html5ever::tendril::SliceExt as _;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

/// Follow Vite entrypoints present in the document, including lazy chunks.
/// A missing manifest is permitted for applications without Vite. A present
/// malformed manifest, missing import key or invalid asset path is an error.
pub(crate) async fn vite_assets(
    source: &opendal::Operator,
    manifest_path: &str,
    referenced: &BTreeSet<String>,
) -> Result<BTreeSet<String>, FrameworkError> {
    let bytes = match source.read(manifest_path).await {
        Ok(bytes) => bytes.to_bytes(),
        Err(error) if error.kind() == opendal::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(error) => {
            return Err(FrameworkError::internal(format!(
                "publication Vite manifest `{manifest_path}`: {error}"
            )));
        }
    };
    let manifest: BTreeMap<String, serde_json::Value> =
        serde_json::from_slice(&bytes).map_err(|error| {
            FrameworkError::param(format!(
                "invalid publication Vite manifest `{manifest_path}`: {error}"
            ))
        })?;
    let build_root = manifest_path
        .split_once("/.vite/")
        .map(|(root, _)| root)
        .or_else(|| {
            manifest_path
                .rsplit_once('/')
                .map(|(root, _)| if root == ".vite" { "" } else { root })
        })
        .unwrap_or("")
        .trim_matches('/');
    let build_prefix = if build_root.is_empty() {
        String::new()
    } else {
        format!("{build_root}/")
    };
    let mut pending: Vec<_> = manifest
        .iter()
        .filter_map(|(key, entry)| {
            let file = entry.get("file").and_then(serde_json::Value::as_str)?;
            let relative = file.trim_start_matches('/');
            if !file.starts_with('/') && referenced.contains(&format!("{build_prefix}{relative}")) {
                Some((key.clone(), build_prefix.clone()))
            } else if referenced.contains(relative) {
                Some((key.clone(), String::new()))
            } else {
                None
            }
        })
        .collect();
    let mut visited = BTreeSet::new();
    let mut paths = BTreeSet::new();
    while let Some((key, prefix)) = pending.pop() {
        if !visited.insert((key.clone(), prefix.clone())) {
            continue;
        }
        let entry = manifest.get(&key).ok_or_else(|| {
            FrameworkError::param(format!(
                "publication Vite import `{key}` is absent from its manifest"
            ))
        })?;
        let file = entry
            .get("file")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                FrameworkError::param(format!("publication Vite entry `{key}` has no file"))
            })?;
        if let Some(path) = vite_path(file, &prefix)? {
            paths.insert(path);
        }
        for field in ["css", "assets", "imports", "dynamicImports"] {
            let Some(values) = entry.get(field) else {
                continue;
            };
            let values = values.as_array().ok_or_else(|| {
                FrameworkError::param(format!(
                    "publication Vite entry `{key}` field `{field}` is not an array"
                ))
            })?;
            for value in values {
                let value = value.as_str().ok_or_else(|| {
                    FrameworkError::param(format!(
                        "publication Vite entry `{key}` field `{field}` has a non-string path"
                    ))
                })?;
                if matches!(field, "imports" | "dynamicImports") {
                    pending.push((value.to_string(), prefix.clone()));
                } else if let Some(path) = vite_path(value, &prefix)? {
                    paths.insert(path);
                }
            }
        }
    }
    Ok(paths)
}

fn vite_path(value: &str, prefix: &str) -> Result<Option<String>, FrameworkError> {
    if value.starts_with('/') || url::Url::parse(value).is_ok() {
        local_path(value, "/")
    } else {
        local_path(&format!("{prefix}{value}"), "/")
    }
}

/// Collect document-local assets after HTML parsing, excluding scripts' text.
/// Paths are decoded and resolved relative to the document URL.
pub(crate) fn referenced_assets(
    html: &[u8],
    document_path: &str,
) -> Result<BTreeSet<String>, FrameworkError> {
    let html = std::str::from_utf8(html).map_err(|error| {
        FrameworkError::param(format!("publication HTML is not UTF-8: {error}"))
    })?;
    let mut paths = BTreeSet::new();
    for (_, _, tag) in tags(html) {
        if tag.kind != TagKind::StartTag {
            continue;
        }
        let fields = reference_fields(&tag);
        for field in fields {
            if let Some(value) = attribute(&tag, field) {
                let urls = if field.ends_with("srcset") {
                    parse_srcset(value)
                } else {
                    vec![(value.into(), String::new())]
                };
                for (url, _) in urls {
                    if let Some(path) = local_path(&url, document_path)? {
                        paths.insert(path);
                    }
                }
            }
        }
    }
    Ok(paths)
}

pub(super) fn has_raster_data(html: &[u8]) -> Result<bool, FrameworkError> {
    let html = std::str::from_utf8(html).map_err(|error| {
        FrameworkError::param(format!("publication HTML is not UTF-8: {error}"))
    })?;
    Ok(tags(html).iter().any(|(_, _, tag)| {
        tag.kind == TagKind::StartTag
            && reference_fields(tag).iter().any(|field| {
                attribute(tag, field).is_some_and(|value| {
                    if field.ends_with("srcset") {
                        parse_srcset(value)
                            .iter()
                            .any(|(url, _)| raster_data_mime(url).is_some())
                    } else {
                        raster_data_mime(value).is_some()
                    }
                })
            })
    }))
}

fn reference_fields(tag: &HtmlTag) -> &'static [&'static str] {
    match tag.name.as_str() {
        "img" | "source" => &["src", "srcset"],
        "script" | "video" | "audio" | "track" | "embed" => &["src", "poster"],
        "link" if crate::page_budget::asset_link(attribute(tag, "rel").unwrap_or_default()) => {
            &["href", "imagesrcset"]
        }
        "object" => &["data"],
        _ => &[],
    }
}

pub(super) fn local_path(
    reference: &str,
    document_path: &str,
) -> Result<Option<String>, FrameworkError> {
    let reference = reference.trim();
    if reference.is_empty() || reference.starts_with('#') {
        return Ok(None);
    }
    let mut base = url::Url::parse(&crate::routing::url::app_url()).map_err(|error| {
        FrameworkError::param(format!("invalid publication application URL: {error}"))
    })?;
    base.set_path(&format!("/{}", document_path.trim_start_matches('/')));
    let resolved = base.join(reference).map_err(|error| {
        FrameworkError::param(format!("invalid publication asset URL: {error}"))
    })?;
    if resolved.origin() != base.origin() {
        return Ok(None);
    }
    let decoded = percent_encoding::percent_decode_str(resolved.path())
        .decode_utf8()
        .map_err(|error| {
            FrameworkError::param(format!("publication asset URL is not UTF-8: {error}"))
        })?;
    let path = decoded.trim_start_matches('/');
    if path.split('/').any(|part| part == ".." || part == ".") || path.contains(['\\', '\0']) {
        return Err(FrameworkError::param(
            "publication asset path contains traversal or an invalid character",
        ));
    }
    Ok((!path.is_empty()).then(|| path.to_string()))
}

// Only raster data URLs cross the image preparation boundary.
pub(super) fn raster_data_mime(reference: &str) -> Option<&str> {
    let (scheme, data) = reference.trim().split_once(':')?;
    if !scheme.eq_ignore_ascii_case("data") {
        return None;
    }
    let mime = data.split([';', ',']).next()?;
    [
        "image/png",
        "image/jpeg",
        "image/gif",
        "image/webp",
        "image/bmp",
    ]
    .iter()
    .any(|expected| mime.eq_ignore_ascii_case(expected))
    .then_some(mime)
}

pub(super) fn is_raster(path: &str) -> bool {
    path.rsplit('.').next().is_some_and(|extension| {
        matches!(
            extension.to_ascii_lowercase().as_str(),
            "png"
                | "jpg"
                | "jpeg"
                | "webp"
                | "gif"
                | "bmp"
                | "avif"
                | "tif"
                | "tiff"
                | "heic"
                | "heif"
        )
    })
}

pub(super) fn is_vector_or_icon(path: &str) -> bool {
    path.rsplit('.')
        .next()
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "svg" | "ico"))
}

pub(super) fn parse_srcset(value: &str) -> Vec<(String, String)> {
    let mut rest = value;
    let mut candidates = Vec::new();
    loop {
        rest = rest.trim_start_matches(|ch: char| ch.is_ascii_whitespace() || ch == ',');
        if rest.is_empty() {
            break;
        }
        let end = rest
            .find(|ch: char| ch.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let url = &rest[..end];
        rest = &rest[end..];
        if url.ends_with(',') {
            candidates.push((url.trim_end_matches(',').into(), String::new()));
        } else {
            let end = rest.find(',').unwrap_or(rest.len());
            candidates.push((url.into(), rest[..end].trim().into()));
            rest = &rest[end..];
        }
    }
    candidates
}

#[cfg(feature = "media")]
pub(super) fn with_descriptor(url: &str, descriptor: &str) -> String {
    if descriptor.is_empty() {
        url.into()
    } else {
        format!("{url} {descriptor}")
    }
}

// Tokenizer tendrils are not Send. Drop them in the synchronous parsing
// pass so no parser-owned value survives across storage or codec awaits.
pub(super) struct HtmlTag {
    pub(super) name: String,
    pub(super) kind: TagKind,
    pub(super) attrs: Vec<(String, String)>,
}

pub(super) fn attribute<'a>(tag: &'a HtmlTag, name: &str) -> Option<&'a str> {
    tag.attrs
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

pub(super) fn set_attribute(tag: &mut HtmlTag, name: &str, value: &str) {
    if let Some((_, attribute)) = tag.attrs.iter_mut().find(|(key, _)| key == name) {
        *attribute = value.to_string();
    }
}

pub(super) fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

pub(super) fn serialize_tag(tag: &HtmlTag) -> String {
    let mut output = format!("<{}", tag.name);
    for (name, value) in &tag.attrs {
        output.push_str(&format!(" {name}=\"{}\"", escape(value)));
    }
    output.push('>');
    output
}

#[derive(Default)]
struct TagSink {
    tokens: RefCell<Vec<(usize, Token)>>,
    end: Cell<usize>,
}

impl TokenSink for TagSink {
    type Handle = ();
    fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<()> {
        let transition = match &token {
            Token::TagToken(tag) if tag.kind == TagKind::StartTag => match tag.name.as_ref() {
                "title" | "textarea" => TokenSinkResult::RawData(RawKind::Rcdata),
                "style" | "xmp" | "iframe" | "noembed" | "noframes" | "noscript" => {
                    TokenSinkResult::RawData(RawKind::Rawtext)
                }
                "script" => TokenSinkResult::RawData(RawKind::ScriptData),
                "plaintext" => TokenSinkResult::Plaintext,
                _ => TokenSinkResult::Continue,
            },
            _ => TokenSinkResult::Continue,
        };
        if matches!(
            token,
            Token::TagToken(_) | Token::CommentToken(_) | Token::DoctypeToken(_)
        ) {
            self.tokens.borrow_mut().push((self.end.get(), token));
        }
        transition
    }
}

pub(super) fn tags(html: &str) -> Vec<(usize, usize, HtmlTag)> {
    let tokenizer = Tokenizer::new(TagSink::default(), Default::default());
    let queue = BufferQueue::default();
    let mut end = 0;
    for part in html.split_inclusive('>') {
        end += part.len();
        tokenizer.sink.end.set(end);
        queue.push_back(part.to_tendril());
        while tokenizer.feed(&queue) != TokenizerResult::Done {}
    }
    tokenizer.end();
    let mut floor = 0;
    let mut tags = Vec::new();
    for (end, token) in tokenizer.sink.tokens.into_inner() {
        if let Token::TagToken(tag) = token {
            let prefix = if tag.kind == TagKind::EndTag {
                format!("</{}", tag.name)
            } else {
                format!("<{}", tag.name)
            };
            if let Some(start) = html[floor..end]
                .match_indices('<')
                .map(|(offset, _)| floor + offset)
                .find(|start| {
                    html[*start..end]
                        .get(..prefix.len())
                        .is_some_and(|part| part.eq_ignore_ascii_case(&prefix))
                        && html[*start + prefix.len()..end]
                            .chars()
                            .next()
                            .is_some_and(|ch| ch.is_ascii_whitespace() || ch == '>' || ch == '/')
                })
            {
                tags.push((
                    start,
                    end,
                    HtmlTag {
                        name: tag.name.to_string(),
                        kind: tag.kind,
                        attrs: tag
                            .attrs
                            .into_iter()
                            .map(|attribute| {
                                (
                                    attribute.name.local.to_string(),
                                    attribute.value.to_string(),
                                )
                            })
                            .collect(),
                    },
                ));
            }
        }
        floor = end;
    }
    tags
}

/// Rewrites only parsed asset attributes; executable script text remains trusted and unchanged.
pub(crate) fn version_assets(
    html: &bytes::Bytes,
    json: &bytes::Bytes,
    document: &str,
    paths: &BTreeMap<String, String>,
) -> Result<(bytes::Bytes, bytes::Bytes), FrameworkError> {
    let replace = |value: &str| -> Result<Option<String>, FrameworkError> {
        let Some(path) = local_path(value, document)? else {
            return Ok(None);
        };
        Ok(paths.get(&path).map(|target| {
            let suffix = value.find(['?', '#']).map(|at| &value[at..]).unwrap_or("");
            format!("{target}{suffix}")
        }))
    };
    fn strings(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::String(value) => out.push(value.clone()),
            serde_json::Value::Array(values) => values.iter().for_each(|v| strings(v, out)),
            serde_json::Value::Object(values) => values.values().for_each(|v| strings(v, out)),
            _ => {}
        }
    }
    let mut values = Vec::new();
    strings(
        &serde_json::from_slice(json)
            .map_err(|e| FrameworkError::internal(format!("publication JSON: {e}")))?,
        &mut values,
    );
    let mut replacements = BTreeMap::new();
    for value in values {
        if let Some(target) = replace(&value)? {
            replacements.insert(value, target);
        }
    }
    let (html, json) =
        crate::inertia::publication::rewrite_public_image_urls(html, json, &replacements)?;
    let text = std::str::from_utf8(&html).map_err(|e| FrameworkError::internal(e.to_string()))?;
    let mut output = String::new();
    let mut cursor = 0;
    for (start, end, mut tag) in tags(text) {
        if tag.kind != TagKind::StartTag {
            continue;
        }
        let fields = reference_fields(&tag);
        let mut changed = false;
        for field in fields {
            if let Some(value) = attribute(&tag, field) {
                let target = if field.ends_with("srcset") {
                    let mut candidates = Vec::new();
                    for (url, descriptor) in parse_srcset(value) {
                        candidates.push(
                            format!("{} {}", replace(&url)?.unwrap_or(url), descriptor)
                                .trim()
                                .to_string(),
                        );
                    }
                    candidates.join(", ")
                } else {
                    replace(value)?.unwrap_or_else(|| value.to_string())
                };
                changed |= target != value;
                set_attribute(&mut tag, field, &target);
            }
        }
        if changed {
            output.push_str(&text[cursor..start]);
            output.push_str(&serialize_tag(&tag));
            cursor = end;
        }
    }
    output.push_str(&text[cursor..]);
    Ok((bytes::Bytes::from(output), json))
}

#[cfg(test)]
mod tests {
    use super::{referenced_assets, vite_assets};
    use std::collections::BTreeSet;

    #[test]
    fn pub_007_asset_parser_ignores_fake_markup_and_decodes_real_attributes() {
        let paths = referenced_assets(br#"<script>const x = '<img src="/fake.png">';</script><!-- <img src='/fake.png'> --><link rel='stylesheet' href='/app.123.css'><img src='../images/a&amp;b.png' srcset='/small.png 1x, /large.png 2x'><script src='/app.123.js'></script>"#, "/nested/page").expect("parsed assets");
        assert_eq!(
            paths.into_iter().collect::<Vec<_>>(),
            [
                "app.123.css",
                "app.123.js",
                "images/a&b.png",
                "large.png",
                "small.png"
            ]
        );
    }
    #[tokio::test]
    async fn pub_002_vite_assets_follow_css_static_and_dynamic_imports() {
        let source =
            opendal::Operator::new(opendal::services::Memory::default()).expect("memory source");
        source.write("assets/.vite/manifest.json", br#"{
          "entry.ts":{"file":"assets/app-01234567.js","css":["assets/app-01234567.css"],"assets":["assets/font-01234567.woff2"],"imports":["vendor.ts"],"dynamicImports":["lazy.ts"]},
          "vendor.ts":{"file":"assets/vendor-01234567.js","imports":["entry.ts"]},
          "lazy.ts":{"file":"assets/lazy-01234567.js","css":["assets/lazy-01234567.css"]},
          "unused.ts":{"file":"assets/unused-01234567.js"}
        }"#.to_vec()).await.expect("manifest");
        let paths = vite_assets(
            &source,
            "assets/.vite/manifest.json",
            &BTreeSet::from(["assets/app-01234567.js".into()]),
        )
        .await
        .expect("Vite closure");
        assert_eq!(
            paths.into_iter().collect::<Vec<_>>(),
            [
                "assets/app-01234567.css",
                "assets/app-01234567.js",
                "assets/font-01234567.woff2",
                "assets/lazy-01234567.css",
                "assets/lazy-01234567.js",
                "assets/vendor-01234567.js"
            ]
        );
    }

    #[tokio::test]
    async fn pub_002_vite_missing_manifest_is_optional_but_broken_import_fails() {
        let source =
            opendal::Operator::new(opendal::services::Memory::default()).expect("memory source");
        let referenced = BTreeSet::from(["assets/app-01234567.js".into()]);
        assert!(
            vite_assets(&source, "assets/.vite/manifest.json", &referenced)
                .await
                .expect("no Vite")
                .is_empty()
        );
        source
            .write(
                "assets/.vite/manifest.json",
                br#"{"entry.ts":{"file":"assets/app-01234567.js","imports":["missing.ts"]}}"#
                    .to_vec(),
            )
            .await
            .expect("broken manifest");
        let error = vite_assets(&source, "assets/.vite/manifest.json", &referenced)
            .await
            .expect_err("missing imported chunk key");
        assert!(error.to_string().contains("missing.ts"));
    }
    #[tokio::test]
    async fn pub_002_vite_assets_resolve_build_relative_paths_from_public_root() {
        let source =
            opendal::Operator::new(opendal::services::Memory::default()).expect("memory source");
        source.write("assets/.vite/manifest.json", br#"{
          "entry.ts":{"file":"app-01234567.js","css":["app-01234567.css"],"dynamicImports":["lazy.ts"]},
          "lazy.ts":{"file":"chunks/lazy-01234567.js","assets":["font-01234567.woff2"]}
        }"#.to_vec()).await.expect("manifest");
        let referenced = referenced_assets(b"<script src='/assets/app-01234567.js'></script>", "/")
            .expect("document assets");
        let paths = vite_assets(&source, "assets/.vite/manifest.json", &referenced)
            .await
            .expect("build-relative Vite closure");
        assert_eq!(
            paths.into_iter().collect::<Vec<_>>(),
            [
                "assets/app-01234567.css",
                "assets/app-01234567.js",
                "assets/chunks/lazy-01234567.js",
                "assets/font-01234567.woff2"
            ]
        );
    }
}
