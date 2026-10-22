//! Parse references without executing scripts or visiting external resources.

use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{BufferQueue, StartTag, Token, TokenSink, TokenSinkResult, Tokenizer};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use url::Url;

#[derive(Default)]
pub(super) struct References {
    pub links: Vec<Link>,
    pub alternatives: BTreeSet<String>,
    pub image_groups: Vec<Vec<String>>,
}

#[derive(Clone)]
pub(super) struct Link {
    pub url: Url,
    pub manifest_icon: bool,
    pub manifest: bool,
}

#[derive(Default)]
struct Document {
    links: Vec<String>,
    base: Option<String>,
    manifests: BTreeSet<String>,
    picture: Option<Vec<String>>,
    groups: Vec<Vec<String>>,
    style: bool,
    css: String,
}

struct Sink(RefCell<Document>);

impl TokenSink for Sink {
    type Handle = ();
    fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<()> {
        let mut doc = self.0.borrow_mut();
        match token {
            Token::CharacterTokens(text) if doc.style => doc.css.push_str(&text),
            Token::TagToken(tag) => {
                let name = tag.name.as_ref();
                if tag.kind != StartTag {
                    if name == "picture"
                        && let Some(group) = doc.picture.take()
                    {
                        doc.groups.push(group);
                    }
                    if name == "style" {
                        doc.style = false;
                    }
                    return TokenSinkResult::Continue;
                }
                let attrs: BTreeMap<_, _> = tag
                    .attrs
                    .iter()
                    .map(|a| (a.name.local.to_string(), a.value.to_string()))
                    .collect();
                let attr = |key: &str| attrs.get(key).map(String::as_str).unwrap_or("");
                if name == "base" && doc.base.is_none() && !attr("href").is_empty() {
                    doc.base = Some(attr("href").into());
                }
                if name == "picture" {
                    doc.picture = Some(Vec::new());
                }
                let mut urls = Vec::new();
                match name {
                    "script" => {
                        urls.push(attr("src").to_owned());
                    }
                    "img" | "source" => {
                        urls.push(attr("src").to_owned());
                        urls.extend(srcset(attr("srcset")));
                    }
                    "link"
                        if attr("rel").split_ascii_whitespace().any(|r| {
                            [
                                "stylesheet",
                                "icon",
                                "apple-touch-icon",
                                "manifest",
                                "preload",
                                "modulepreload",
                            ]
                            .iter()
                            .any(|s| r.eq_ignore_ascii_case(s))
                        }) =>
                    {
                        if attr("rel")
                            .split_ascii_whitespace()
                            .any(|r| r.eq_ignore_ascii_case("manifest"))
                        {
                            doc.manifests.insert(attr("href").to_owned());
                        }
                        urls.push(attr("href").to_owned());
                        urls.extend(srcset(attr("imagesrcset")));
                    }
                    _ => {}
                }
                urls.retain(|s| !s.is_empty());
                if matches!(name, "img" | "source") {
                    if let Some(group) = &mut doc.picture {
                        group.extend(urls.clone());
                    } else {
                        doc.groups.push(urls.clone());
                    }
                }
                doc.links.extend(urls);
                if name == "style" {
                    doc.style = true;
                    return TokenSinkResult::RawData(RawKind::Rawtext);
                }
                if name == "script" {
                    return TokenSinkResult::RawData(RawKind::ScriptData);
                }
                if matches!(name, "textarea" | "title") {
                    return TokenSinkResult::RawData(RawKind::Rcdata);
                }
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

// Data URLs can contain commas. Consume their URL before splitting descriptors.
fn srcset(value: &str) -> Vec<String> {
    let mut rest = value;
    let mut urls = Vec::new();
    while !rest.is_empty() {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
        if rest.is_empty() {
            break;
        }
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let token = &rest[..end];
        if token.starts_with("data:") {
            rest = &rest[end..];
        } else {
            let end = token.find(',').unwrap_or(token.len());
            urls.push(token[..end].to_owned());
            rest = &rest[end..];
            if rest.starts_with(',') {
                continue;
            }
        }
        rest = rest.split_once(',').map_or("", |(_, tail)| tail);
    }
    urls
}

fn resolve(base: &Url, value: &str) -> Option<Url> {
    if value.trim().is_empty() || value.trim_start().starts_with('#') {
        return None;
    }
    let mut url = base.join(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    url.set_fragment(None);
    Some(url)
}

pub(super) fn html(base: &Url, text: &str) -> References {
    let tokenizer = Tokenizer::new(Sink(RefCell::new(Document::default())), Default::default());
    let buffer = BufferQueue::default();
    buffer.push_back(text.into());
    let _ = tokenizer.feed(&buffer);
    tokenizer.end();
    let doc = tokenizer.sink.0.into_inner();
    let base = doc
        .base
        .as_deref()
        .and_then(|b| resolve(base, b))
        .unwrap_or_else(|| base.clone());
    let mut result = References::default();
    result.links.extend(doc.links.iter().filter_map(|s| {
        resolve(&base, s).map(|url| Link {
            url,
            manifest_icon: false,
            manifest: doc.manifests.contains(s),
        })
    }));
    for group in doc.groups.into_iter().chain(doc.picture) {
        let urls: Vec<_> = group.iter().filter_map(|s| resolve(&base, s)).collect();
        result
            .image_groups
            .push(urls.iter().map(Url::to_string).collect());
        let modern = urls.iter().any(|u| modern(u.path()));
        let fallback = urls.iter().any(|u| legacy(u.path()));
        if modern && fallback {
            result
                .alternatives
                .extend(urls.into_iter().map(|u| u.to_string()));
        }
    }
    result.links.extend(css(&base, &doc.css).links);
    result
}

fn modern(path: &str) -> bool {
    matches!(path.rsplit('.').next(), Some("webp" | "avif"))
}
fn legacy(path: &str) -> bool {
    matches!(
        path.rsplit('.').next(),
        Some("png" | "jpg" | "jpeg" | "gif" | "bmp")
    )
}

pub(super) fn css(base: &Url, text: &str) -> References {
    let mut parser = cssparser::Parser::new(text);
    let mut urls = Vec::new();
    css_urls(&mut parser, &mut urls);
    References {
        links: urls
            .iter()
            .filter_map(|s| resolve(base, s))
            .map(|url| Link {
                url,
                manifest_icon: false,
                manifest: false,
            })
            .collect(),
        alternatives: BTreeSet::new(),
        image_groups: Vec::new(),
    }
}

fn css_urls<'i>(parser: &mut cssparser::Parser<'i>, urls: &mut Vec<String>) {
    use cssparser::Token;
    let mut import = false;
    while let Ok(token) = parser.next().cloned() {
        match token {
            Token::AtKeyword(name) => import = name.eq_ignore_ascii_case("import"),
            Token::QuotedString(value) if import => {
                urls.push(value.to_string());
                import = false;
            }
            Token::UnquotedUrl(value) => {
                urls.push(value.to_string());
                import = false;
            }
            Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                import = false;
                let _: Result<(), cssparser::ParseError<()>> =
                    parser.parse_nested_block(|nested| {
                        if let Ok(Token::QuotedString(value)) = nested.next() {
                            urls.push(value.to_string());
                        }
                        Ok(())
                    });
            }
            Token::Function(_)
            | Token::CurlyBracketBlock
            | Token::ParenthesisBlock
            | Token::SquareBracketBlock => {
                let _: Result<(), cssparser::ParseError<()>> =
                    parser.parse_nested_block(|nested| {
                        css_urls(nested, urls);
                        Ok(())
                    });
            }
            _ => {}
        }
    }
}

pub(super) fn manifest(base: &Url, bytes: &[u8]) -> Result<References, crate::FrameworkError> {
    let json: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| {
        crate::FrameworkError::bad_request(format!("page budget manifest {base}: {e}"))
    })?;
    let mut result = References::default();
    let mut groups: BTreeMap<BTreeSet<String>, (bool, bool, Vec<String>)> = BTreeMap::new();
    for icon in json
        .get("icons")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        let Some(url) = icon
            .get("src")
            .and_then(|v| v.as_str())
            .and_then(|src| resolve(base, src))
        else {
            continue;
        };
        let sizes = icon
            .get("sizes")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split_ascii_whitespace()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let mime = icon.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if !sizes.is_empty() {
            let group = groups.entry(sizes).or_default();
            group.0 |= matches!(mime, "image/webp" | "image/avif");
            group.1 |= !matches!(mime, "image/webp" | "image/avif")
                && (matches!(mime, "image/png" | "image/jpeg" | "image/gif" | "image/bmp")
                    || legacy(url.path()));
            group.2.push(url.to_string());
        }
        result.links.push(Link {
            url,
            manifest_icon: true,
            manifest: false,
        });
    }
    // Manifest alternatives describe the same icon size and declare a modern type.
    for (_, (modern, fallback, urls)) in groups {
        if modern && fallback {
            result.image_groups.push(urls);
        }
    }
    Ok(result)
}
