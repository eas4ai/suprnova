//! Parse references without executing scripts or visiting external resources.

use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{BufferQueue, StartTag, Token, TokenSink, TokenSinkResult, Tokenizer};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use url::Url;

#[derive(Default)]
pub(super) struct References {
    pub links: Vec<Link>,
    pub alternatives: BTreeSet<String>,
    pub image_groups: Vec<Vec<String>>,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Link {
    pub url: Url,
    pub manifest_icon: bool,
    pub manifest: bool,
}

// Each parser shares an overflow flag so it stops retaining references at the limit.
#[derive(Clone, Default)]
struct Limit {
    maximum: usize,
    exceeded: Rc<Cell<bool>>,
}
impl Limit {
    fn new(maximum: usize) -> Self {
        Self {
            maximum,
            exceeded: Rc::default(),
        }
    }
    fn push<T: PartialEq>(&self, values: &mut Vec<T>, value: T) {
        if self.exceeded.get() || values.contains(&value) {
            return;
        }
        if values.len() >= self.maximum {
            self.exceeded.set(true);
        } else {
            values.push(value);
        }
    }
    fn check(&self) -> Result<(), crate::FrameworkError> {
        if self.exceeded.get() {
            Err(crate::FrameworkError::bad_request(format!(
                "page budget discovery exceeds {} resources",
                self.maximum
            )))
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
struct Document {
    limit: Limit,
    links: Vec<String>,
    base: Option<String>,
    manifests: BTreeSet<String>,
    picture: Option<Vec<String>>,
    groups: Vec<Vec<String>>,
    style: bool,
    css: String,
}

impl Document {
    fn close_tag(&mut self, name: &str) {
        if name == "picture"
            && let Some(group) = self.picture.take()
        {
            self.limit.push(&mut self.groups, group);
        }
        if name == "style" {
            self.style = false;
        }
    }
}

fn tag_urls(name: &str, attrs: &BTreeMap<String, String>, limit: &Limit) -> Vec<String> {
    let attr = |key: &str| attrs.get(key).map(String::as_str).unwrap_or("");
    let fields: &[&str] = match name {
        "video" => &["src", "poster"],
        "audio" | "script" | "img" | "source" => &["src"],
        "link"
            if attr("rel").split_ascii_whitespace().any(|rel| {
                [
                    "stylesheet",
                    "icon",
                    "apple-touch-icon",
                    "manifest",
                    "preload",
                    "modulepreload",
                ]
                .iter()
                .any(|candidate| rel.eq_ignore_ascii_case(candidate))
            }) =>
        {
            &["href"]
        }
        _ => &[],
    };
    let mut urls = Vec::new();
    for field in fields {
        if !attr(field).is_empty() {
            limit.push(&mut urls, attr(field).to_owned());
        }
    }
    match name {
        "img" | "source" => srcset(attr("srcset"), &mut urls, limit),
        "link" if !fields.is_empty() => srcset(attr("imagesrcset"), &mut urls, limit),
        _ => {}
    }
    urls
}

struct Sink(RefCell<Document>);

impl TokenSink for Sink {
    type Handle = ();
    fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<()> {
        let mut doc = self.0.borrow_mut();
        let limit = doc.limit.clone();
        if limit.exceeded.get() {
            return TokenSinkResult::Continue;
        }
        match token {
            Token::CharacterTokens(text) if doc.style => doc.css.push_str(&text),
            Token::TagToken(tag) => {
                let name = tag.name.as_ref();
                if tag.kind != StartTag {
                    doc.close_tag(name);
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
                let mut urls = tag_urls(name, &attrs, &limit);
                if name == "link"
                    && attr("rel")
                        .split_ascii_whitespace()
                        .any(|rel| rel.eq_ignore_ascii_case("manifest"))
                {
                    doc.manifests.insert(attr("href").to_owned());
                }
                urls.retain(|s| !s.is_empty());
                if matches!(name, "img" | "source") {
                    if let Some(group) = &mut doc.picture {
                        for url in &urls {
                            limit.push(group, url.clone());
                        }
                    } else {
                        limit.push(&mut doc.groups, urls.clone());
                    }
                }
                for url in urls {
                    limit.push(&mut doc.links, url);
                }
                doc.css.push_str(attr("style"));
                doc.css.push(';');
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

// HTML URL tokens may contain commas. Only trailing URL commas end a candidate.
fn srcset(value: &str, urls: &mut Vec<String>, limit: &Limit) {
    let mut rest = value;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
        if rest.is_empty() {
            break;
        }
        let end = rest
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let token = &rest[..end];
        rest = &rest[end..];
        limit.push(urls, token.trim_end_matches(',').to_owned());
        if limit.exceeded.get() {
            break;
        }
        if token.ends_with(',') {
            continue;
        }
        // Commas in descriptor parentheses do not terminate a candidate.
        let mut parentheses = 0usize;
        let mut consumed = rest.len();
        for (index, character) in rest.char_indices() {
            match character {
                '(' => parentheses = parentheses.saturating_add(1),
                ')' => parentheses = parentheses.saturating_sub(1),
                ',' if parentheses == 0 => {
                    consumed = index + 1;
                    break;
                }
                _ => {}
            }
        }
        rest = &rest[consumed..];
    }
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

pub(super) fn html(
    base: &Url,
    text: &str,
    maximum: usize,
) -> Result<References, crate::FrameworkError> {
    let limit = Limit::new(maximum);
    let document = Document {
        limit: limit.clone(),
        ..Document::default()
    };
    let tokenizer = Tokenizer::new(Sink(RefCell::new(document)), Default::default());
    let buffer = BufferQueue::default();
    buffer.push_back(text.into());
    let _ = tokenizer.feed(&buffer);
    tokenizer.end();
    limit.check()?;
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
    for link in css(&base, &doc.css, maximum)?.links {
        limit.push(&mut result.links, link);
    }
    limit.check()?;
    Ok(result)
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

pub(super) fn css(
    base: &Url,
    text: &str,
    maximum: usize,
) -> Result<References, crate::FrameworkError> {
    let limit = Limit::new(maximum);
    let mut parser = cssparser::Parser::new(text);
    let mut urls = Vec::new();
    css_urls(&mut parser, &mut urls, false, &limit);
    limit.check()?;
    Ok(References {
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
    })
}

fn css_urls<'i>(
    parser: &mut cssparser::Parser<'i>,
    urls: &mut Vec<String>,
    image_set: bool,
    limit: &Limit,
) {
    use cssparser::Token;
    let mut import = false;
    while !limit.exceeded.get() {
        let Ok(token) = parser.next().cloned() else {
            break;
        };
        match token {
            Token::AtKeyword(name) => import = name.eq_ignore_ascii_case("import"),
            Token::QuotedString(value) if import || image_set => {
                limit.push(urls, value.to_string());
                import = false;
            }
            Token::UnquotedUrl(value) => {
                limit.push(urls, value.to_string());
                import = false;
            }
            Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                import = false;
                let _: Result<(), cssparser::ParseError<()>> =
                    parser.parse_nested_block(|nested| {
                        if let Ok(Token::QuotedString(value)) = nested.next() {
                            limit.push(urls, value.to_string());
                        }
                        Ok(())
                    });
            }
            Token::Function(name) => {
                let image_set = name.eq_ignore_ascii_case("image-set")
                    || name.eq_ignore_ascii_case("-webkit-image-set");
                let _: Result<(), cssparser::ParseError<()>> =
                    parser.parse_nested_block(|nested| {
                        css_urls(nested, urls, image_set, limit);
                        Ok(())
                    });
            }
            Token::CurlyBracketBlock | Token::ParenthesisBlock | Token::SquareBracketBlock => {
                let _: Result<(), cssparser::ParseError<()>> =
                    parser.parse_nested_block(|nested| {
                        css_urls(nested, urls, false, limit);
                        Ok(())
                    });
            }
            _ => {}
        }
    }
}

pub(super) fn manifest(
    base: &Url,
    bytes: &[u8],
    maximum: usize,
) -> Result<References, crate::FrameworkError> {
    let json: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| {
        crate::FrameworkError::bad_request(format!("page budget manifest {base}: {e}"))
    })?;
    let limit = Limit::new(maximum);
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
            if groups.len() >= maximum && !groups.contains_key(&sizes) {
                return Err(crate::FrameworkError::bad_request(
                    "page budget manifest exceeds resource groups",
                ));
            }
            let group = groups.entry(sizes).or_default();
            group.0 |= matches!(mime, "image/webp" | "image/avif");
            group.1 |= !matches!(mime, "image/webp" | "image/avif")
                && (matches!(mime, "image/png" | "image/jpeg" | "image/gif" | "image/bmp")
                    || legacy(url.path()));
            limit.push(&mut group.2, url.to_string());
        }
        limit.push(
            &mut result.links,
            Link {
                url,
                manifest_icon: true,
                manifest: false,
            },
        );
        limit.check()?;
    }
    // Manifest alternatives describe the same icon size and declare a modern type.
    for (_, (modern, fallback, urls)) in groups {
        if modern && fallback {
            result.image_groups.push(urls);
        }
    }
    Ok(result)
}
