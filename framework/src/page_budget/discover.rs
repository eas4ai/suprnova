//! Parse a bounded reference graph without executing document code.

use crate::FrameworkError;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{BufferQueue, StartTag, Token, TokenSink, TokenSinkResult, Tokenizer};
use serde::Deserialize;
use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::rc::Rc;
use url::Url;

#[derive(Default)]
pub(super) struct References {
    pub links: Vec<Link>,
    pub alternatives: BTreeSet<String>,
    pub image_groups: Vec<Vec<String>>,
    pub work_done: usize,
}
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Link {
    pub url: Url,
    pub manifest_icon: bool,
    pub manifest: bool,
}
pub(super) struct Limits<'a> {
    pub resources: usize,
    pub tokens: usize,
    pub used_tokens: usize,
    pub known: &'a BTreeSet<String>,
}
struct Discovery {
    refs: References,
    known: BTreeSet<String>,
    max_resources: usize,
    max_tokens: usize,
    used_tokens: usize,
    error: Option<String>,
}
impl Discovery {
    fn new(limits: Limits<'_>) -> Self {
        Self {
            refs: References::default(),
            known: limits.known.clone(),
            max_resources: limits.resources,
            max_tokens: limits.tokens,
            used_tokens: limits.used_tokens,
            error: None,
        }
    }
    fn tick(&mut self) -> bool {
        if self.error.is_some() {
            return false;
        }
        if self.used_tokens.saturating_add(self.refs.work_done) >= self.max_tokens {
            self.error = Some(format!(
                "page budget discovery work limit of {} tokens exceeded after {} tokens",
                self.max_tokens,
                self.used_tokens.saturating_add(self.refs.work_done)
            ));
            return false;
        }
        self.refs.work_done += 1;
        true
    }
    fn json_tick<E: serde::de::Error>(&mut self) -> Result<(), E> {
        if self.tick() {
            Ok(())
        } else {
            Err(E::custom(
                self.error.as_deref().unwrap_or("discovery work limit"),
            ))
        }
    }
    fn reference(&mut self, base: &Url, raw: &str, manifest: bool, icon: bool) -> Option<String> {
        if !self.tick() {
            return None;
        }
        let url = resolve(base, raw)?;
        let key = url.to_string();
        if !self.known.contains(&key) {
            if self.known.len() >= self.max_resources {
                self.error = Some(format!(
                    "page budget resource limit exceeded: at most {} resources; asset {url}",
                    self.max_resources
                ));
                return None;
            }
            self.known.insert(key.clone());
        }
        if let Some(link) = self.refs.links.iter_mut().find(|link| link.url == url) {
            link.manifest |= manifest;
            link.manifest_icon |= icon;
        } else {
            self.refs.links.push(Link {
                url,
                manifest,
                manifest_icon: icon,
            });
        }
        Some(key)
    }
    fn group(&mut self, urls: impl IntoIterator<Item = String>) {
        let group: Vec<_> = urls
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if !group.is_empty() && !self.refs.image_groups.contains(&group) {
            self.refs.image_groups.push(group);
        }
    }
    fn finish(self, context: &str) -> Result<References, FrameworkError> {
        match self.error {
            Some(error) => Err(FrameworkError::bad_request(format!("{error}; {context}"))),
            None => Ok(self.refs),
        }
    }
}
fn resolve(base: &Url, raw: &str) -> Option<Url> {
    let value = raw.trim();
    if value.is_empty() || value.starts_with('#') {
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

// Raw candidates consume work even when they resolve to a duplicate or inline URL.
fn srcset(value: &str, mut candidate: impl FnMut(&str) -> bool) {
    let mut rest = value;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
        if rest.is_empty() {
            return;
        }
        let end = rest
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let token = &rest[..end];
        rest = &rest[end..];
        if !candidate(token.trim_end_matches(',')) {
            return;
        }
        if token.ends_with(',') {
            continue;
        }
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

struct Document {
    discovery: Discovery,
    base: Url,
    base_seen: bool,
    picture: Option<BTreeSet<String>>,
    style: bool,
    styles: StyleStream,
    input: Rc<BufferQueue>,
    html_tokens: usize,
}
impl Document {
    fn stop_on_error(&self) {
        if self.discovery.error.is_some() {
            // Clearing the shared input stops html5ever for character tokens too.
            // Its sink only permits Continue for those tokens.
            while self.input.pop_front().is_some() {}
        }
    }
    fn close_tag(&mut self, name: &str) {
        if name == "picture"
            && let Some(group) = self.picture.take()
        {
            self.discovery.group(group);
        }
        if name == "style" {
            self.styles.flush(&self.base, &mut self.discovery);
            self.style = false;
            self.styles = StyleStream::default();
        }
    }
    fn start_tag(&mut self, tag: html5ever::tokenizer::Tag) -> TokenSinkResult<()> {
        let name = tag.name.as_ref();
        let attrs: BTreeMap<_, _> = tag
            .attrs
            .iter()
            .map(|a| (a.name.local.to_string(), a.value.to_string()))
            .collect();
        let attr = |key: &str| attrs.get(key).map(String::as_str).unwrap_or("");
        if name == "base" && !self.base_seen && !attr("href").is_empty() {
            self.base_seen = true;
            if let Some(url) = resolve(&self.base, attr("href")) {
                self.base = url;
            }
        }
        if name == "picture" {
            self.picture = Some(BTreeSet::new());
        }
        let rel = |value: &str| {
            attr("rel")
                .split_ascii_whitespace()
                .any(|r| r.eq_ignore_ascii_case(value))
        };
        let linked = name == "link"
            && [
                "stylesheet",
                "icon",
                "apple-touch-icon",
                "manifest",
                "preload",
                "modulepreload",
            ]
            .iter()
            .any(|value| rel(value));
        let fields: &[&str] = match name {
            "video" => &["src", "poster"],
            "audio" | "script" | "img" | "source" => &["src"],
            "link" if linked => &["href"],
            _ => &[],
        };
        let manifest = linked && rel("manifest");
        let mut group = BTreeSet::new();
        for field in fields {
            if !attr(field).is_empty()
                && let Some(url) =
                    self.discovery
                        .reference(&self.base, attr(field), manifest, false)
            {
                group.insert(url);
            }
        }
        let set = match name {
            "img" | "source" => attr("srcset"),
            "link" if linked => attr("imagesrcset"),
            _ => "",
        };
        srcset(set, |raw| {
            if let Some(url) = self.discovery.reference(&self.base, raw, false, false) {
                group.insert(url);
            }
            self.discovery.error.is_none()
        });
        if matches!(name, "img" | "source") {
            if let Some(picture) = &mut self.picture {
                picture.extend(group);
            } else {
                self.discovery.group(group);
            }
        }
        if !attr("style").is_empty() {
            css_urls(
                &mut cssparser::Parser::new(attr("style")),
                &self.base,
                &mut self.discovery,
                false,
                None,
            );
        }
        match name {
            "style" => {
                self.style = true;
                TokenSinkResult::RawData(RawKind::Rawtext)
            }
            "script" => TokenSinkResult::RawData(RawKind::ScriptData),
            "textarea" | "title" => TokenSinkResult::RawData(RawKind::Rcdata),
            _ => TokenSinkResult::Continue,
        }
    }
}
struct Sink(RefCell<Document>);
impl TokenSink for Sink {
    type Handle = ();
    fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<()> {
        let mut doc = self.0.borrow_mut();
        doc.html_tokens += 1;
        if !doc.discovery.tick() {
            doc.stop_on_error();
            return TokenSinkResult::Continue;
        }
        let result = match token {
            Token::CharacterTokens(text) if doc.style => {
                let Document {
                    styles,
                    base,
                    discovery,
                    ..
                } = &mut *doc;
                styles.push(&text, base, discovery);
                TokenSinkResult::Continue
            }
            Token::TagToken(tag) if tag.kind == StartTag => doc.start_tag(tag),
            Token::TagToken(tag) => {
                doc.close_tag(tag.name.as_ref());
                TokenSinkResult::Continue
            }
            _ => TokenSinkResult::Continue,
        };
        doc.stop_on_error();
        result
    }
}

pub(super) fn html(
    base: &Url,
    text: &str,
    limits: Limits<'_>,
) -> Result<References, FrameworkError> {
    let input = Rc::new(BufferQueue::default());
    let document = Document {
        discovery: Discovery::new(limits),
        base: base.clone(),
        base_seen: false,
        picture: None,
        style: false,
        styles: StyleStream::default(),
        input: input.clone(),
        html_tokens: 0,
    };
    let tokenizer = Tokenizer::new(Sink(RefCell::new(document)), Default::default());
    input.push_back(text.into());
    let _ = tokenizer.feed(&input);
    if tokenizer.sink.0.borrow().discovery.error.is_none() {
        tokenizer.end();
    }
    let mut doc = tokenizer.sink.0.into_inner();
    if doc.discovery.error.is_none() {
        doc.styles.flush(&doc.base, &mut doc.discovery);
        if let Some(group) = doc.picture.take() {
            doc.discovery.group(group);
        }
    }
    doc.discovery
        .finish(&format!("HTML tokens processed: {}", doc.html_tokens))
}

// Keep only the unfinished CSS construct between HTML character chunks. Complete
// constructs enter cssparser immediately, so overflow stops before later markup.
#[derive(Default)]
struct StyleStream {
    pending: String,
    quote: Option<char>,
    escaped: bool,
    comment: bool,
    previous: Option<char>,
    parentheses: usize,
}
impl StyleStream {
    fn push(&mut self, text: &str, base: &Url, discovery: &mut Discovery) {
        for character in text.chars() {
            if discovery.error.is_some() {
                return;
            }
            self.pending.push(character);
            if self.comment {
                if self.previous == Some('*') && character == '/' {
                    self.comment = false;
                    self.previous = None;
                    continue;
                }
            } else if self.escaped {
                self.escaped = false;
            } else if character == '\\' {
                self.escaped = true;
            } else if let Some(quote) = self.quote {
                if character == quote {
                    self.quote = None;
                }
            } else if self.previous == Some('/') && character == '*' {
                self.comment = true;
            } else {
                match character {
                    '\'' | '"' => self.quote = Some(character),
                    '(' => self.parentheses = self.parentheses.saturating_add(1),
                    ')' => {
                        self.parentheses = self.parentheses.saturating_sub(1);
                        if self.parentheses == 0 {
                            self.flush(base, discovery);
                        }
                    }
                    ';' | '}' if self.parentheses == 0 => self.flush(base, discovery),
                    _ => {}
                }
            }
            self.previous = Some(character);
        }
    }
    fn flush(&mut self, base: &Url, discovery: &mut Discovery) {
        if !self.pending.is_empty() && discovery.error.is_none() {
            css_urls(
                &mut cssparser::Parser::new(&self.pending),
                base,
                discovery,
                false,
                None,
            );
        }
        self.pending.clear();
        self.previous = None;
    }
}
pub(super) fn css(
    base: &Url,
    text: &str,
    limits: Limits<'_>,
) -> Result<References, FrameworkError> {
    let mut discovery = Discovery::new(limits);
    css_urls(
        &mut cssparser::Parser::new(text),
        base,
        &mut discovery,
        false,
        None,
    );
    discovery.finish("stylesheet")
}
fn css_reference(
    base: &Url,
    raw: &str,
    discovery: &mut Discovery,
    group: &mut Option<&mut BTreeSet<String>>,
) {
    if let Some(url) = discovery.reference(base, raw, false, false)
        && let Some(group) = group
    {
        group.insert(url);
    }
}
fn css_urls(
    parser: &mut cssparser::Parser<'_>,
    base: &Url,
    discovery: &mut Discovery,
    image_set: bool,
    mut group: Option<&mut BTreeSet<String>>,
) {
    use cssparser::Token;
    let mut import = false;
    while discovery.error.is_none() {
        let Ok(token) = parser.next().cloned() else {
            break;
        };
        if !discovery.tick() {
            break;
        }
        match token {
            Token::AtKeyword(name) => import = name.eq_ignore_ascii_case("import"),
            Token::QuotedString(value) if import || image_set => {
                css_reference(base, &value, discovery, &mut group);
                import = false;
            }
            Token::UnquotedUrl(value) => {
                css_reference(base, &value, discovery, &mut group);
                import = false;
            }
            Token::Function(name) => {
                import = false;
                css_function(parser, &name, base, discovery, &mut group);
            }
            Token::CurlyBracketBlock | Token::ParenthesisBlock | Token::SquareBracketBlock => {
                let _: Result<(), cssparser::ParseError<()>> =
                    parser.parse_nested_block(|nested| {
                        css_urls(nested, base, discovery, false, None);
                        Ok(())
                    });
            }
            _ => {}
        }
    }
    // cssparser otherwise scans to the end of every enclosing block after
    // the callback returns. An empty parser stops that scan on collection failure.
    if discovery.error.is_some() {
        *parser = cssparser::Parser::new("");
    }
}

fn css_function(
    parser: &mut cssparser::Parser<'_>,
    name: &str,
    base: &Url,
    discovery: &mut Discovery,
    group: &mut Option<&mut BTreeSet<String>>,
) {
    if name.eq_ignore_ascii_case("url") {
        let _: Result<(), cssparser::ParseError<()>> = parser.parse_nested_block(|nested| {
            if let Ok(cssparser::Token::QuotedString(value)) = nested.next() {
                css_reference(base, value, discovery, group);
            }
            if discovery.error.is_some() {
                *nested = cssparser::Parser::new("");
            }
            Ok(())
        });
        return;
    }
    let image_set =
        name.eq_ignore_ascii_case("image-set") || name.eq_ignore_ascii_case("-webkit-image-set");
    let mut candidates = BTreeSet::new();
    let _: Result<(), cssparser::ParseError<()>> = parser.parse_nested_block(|nested| {
        css_urls(
            nested,
            base,
            discovery,
            image_set,
            if image_set {
                Some(&mut candidates)
            } else {
                None
            },
        );
        Ok(())
    });
    if image_set {
        discovery.group(candidates);
    }
}

#[derive(Default)]
struct Icon {
    src: Option<String>,
    sizes: Option<String>,
    mime: Option<String>,
}
struct Manifest<'a> {
    base: &'a Url,
    discovery: &'a mut Discovery,
    groups: BTreeMap<BTreeSet<String>, (bool, bool, Vec<String>)>,
}
impl Manifest<'_> {
    fn icon(&mut self, icon: Icon) {
        let Some(src) = icon.src else {
            return;
        };
        let Some(url) = self.discovery.reference(self.base, &src, false, true) else {
            return;
        };
        let mut sizes = BTreeSet::new();
        for size in icon.sizes.as_deref().unwrap_or("").split_ascii_whitespace() {
            if !self.discovery.tick() {
                return;
            }
            sizes.insert(size.to_owned());
        }
        if sizes.is_empty() {
            return;
        }
        let mime = icon.mime.as_deref().unwrap_or("");
        let group = self.groups.entry(sizes).or_default();
        let modern = matches!(mime, "image/webp" | "image/avif");
        group.0 |= modern;
        group.1 |= !modern
            && (matches!(mime, "image/png" | "image/jpeg" | "image/gif" | "image/bmp")
                || matches!(
                    Url::parse(&url)
                        .ok()
                        .as_ref()
                        .and_then(|url| url.path().rsplit('.').next()),
                    Some("png" | "jpg" | "jpeg" | "gif" | "bmp")
                ));
        if !group.2.contains(&url) {
            group.2.push(url);
        }
    }
}
// Typed fields and keys charge before deserialization. Ignored values use the
// recursive visitor below so extensions cannot hide unbounded parser work.
struct ScalarSeed<'a, T>(&'a mut Discovery, std::marker::PhantomData<T>);
fn scalar<T>(discovery: &mut Discovery) -> ScalarSeed<'_, T> {
    ScalarSeed(discovery, std::marker::PhantomData)
}
impl<'de, T: Deserialize<'de>> DeserializeSeed<'de> for ScalarSeed<'_, T> {
    type Value = T;
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<T, D::Error> {
        self.0.json_tick()?;
        T::deserialize(deserializer)
    }
}
struct MeteredIgnored<'a>(&'a mut Discovery);
impl<'de> DeserializeSeed<'de> for MeteredIgnored<'_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        self.0.json_tick()?;
        deserializer.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for MeteredIgnored<'_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a bounded JSON value")
    }
    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<(), S::Error> {
        while seq.next_element_seed(MeteredIgnored(self.0))?.is_some() {}
        Ok(())
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        while map.next_key_seed(MeteredIgnored(self.0))?.is_some() {
            map.next_value_seed(MeteredIgnored(self.0))?;
        }
        Ok(())
    }
}
struct IconFields<'a>(&'a mut Discovery);
impl<'de> DeserializeSeed<'de> for IconFields<'_> {
    type Value = Icon;
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Icon, D::Error> {
        // IconSeed already charges this object as an array element.
        deserializer.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for IconFields<'_> {
    type Value = Icon;
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a manifest icon object")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Icon, M::Error> {
        let mut icon = Icon::default();
        while let Some(key) = map.next_key_seed(scalar::<String>(self.0))? {
            match key.as_str() {
                "src" => icon.src = map.next_value_seed(scalar(self.0))?,
                "sizes" => icon.sizes = map.next_value_seed(scalar(self.0))?,
                "type" => icon.mime = map.next_value_seed(scalar(self.0))?,
                _ => map.next_value_seed(MeteredIgnored(self.0))?,
            }
        }
        Ok(icon)
    }
}
struct RootSeed<'a, 'b>(&'a mut Manifest<'b>);
impl<'de> DeserializeSeed<'de> for RootSeed<'_, '_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        self.0.discovery.json_tick()?;
        deserializer.deserialize_map(self)
    }
}
impl<'de> Visitor<'de> for RootSeed<'_, '_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a web app manifest object")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        while let Some(key) = map.next_key_seed(scalar::<String>(self.0.discovery))? {
            if key == "icons" {
                map.next_value_seed(IconsSeed(self.0))?;
            } else {
                map.next_value_seed(MeteredIgnored(self.0.discovery))?;
            }
        }
        Ok(())
    }
}
struct IconsSeed<'a, 'b>(&'a mut Manifest<'b>);
impl<'de> DeserializeSeed<'de> for IconsSeed<'_, '_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        self.0.discovery.json_tick()?;
        deserializer.deserialize_seq(self)
    }
}
impl<'de> Visitor<'de> for IconsSeed<'_, '_> {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an icon array")
    }
    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<(), S::Error> {
        let mut index = 1;
        while seq
            .next_element_seed(IconSeed {
                manifest: self.0,
                index,
            })?
            .is_some()
        {
            index += 1;
        }
        Ok(())
    }
}
struct IconSeed<'a, 'b> {
    manifest: &'a mut Manifest<'b>,
    index: usize,
}
impl<'de> DeserializeSeed<'de> for IconSeed<'_, '_> {
    type Value = ();
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        let discovery = &mut self.manifest.discovery;
        if !discovery.tick() {
            return Err(D::Error::custom(format!(
                "{}; icons processed: {}",
                discovery.error.as_deref().unwrap_or("discovery work limit"),
                self.index
            )));
        }
        let icon = IconFields(self.manifest.discovery).deserialize(deserializer)?;
        self.manifest.icon(icon);
        if let Some(error) = &self.manifest.discovery.error {
            return Err(D::Error::custom(format!(
                "{error}; icons processed: {}",
                self.index
            )));
        }
        Ok(())
    }
}
pub(super) fn manifest(
    base: &Url,
    bytes: &[u8],
    limits: Limits<'_>,
) -> Result<References, FrameworkError> {
    let mut discovery = Discovery::new(limits);
    let mut manifest = Manifest {
        base,
        discovery: &mut discovery,
        groups: BTreeMap::new(),
    };
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    RootSeed(&mut manifest)
        .deserialize(&mut deserializer)
        .and_then(|()| deserializer.end())
        .map_err(|error| {
            FrameworkError::bad_request(format!("page budget manifest {base}: {error}"))
        })?;
    for (_, (modern, fallback, urls)) in std::mem::take(&mut manifest.groups) {
        if modern && fallback {
            manifest.discovery.group(urls);
        }
    }
    discovery.finish("manifest")
}
