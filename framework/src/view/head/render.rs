//! Resolved head metadata as tags: HTML with every value escaped, and the
//! same tags as data.

use serde_json::{Map, Value};

use super::builder::{HeadData, Hint, Icon, OgMedia, attribute_name_problem};
use crate::inertia::escape_html_attr;

/// What an element holds between its tags.
enum Content {
    /// Text, HTML-escaped when written.
    Text(String),
    /// A JSON-LD object, written with `<`, `>` and `&` escaped so it cannot
    /// end its script element.
    Json(Value),
}

/// One element of the head, with the key that identifies it across
/// visits.
pub(super) struct Tag {
    /// The `data-inertia` key, stable for the element's role: `title`,
    /// `og:image:0`, `meta:format-detection`.
    pub(super) key: String,
    element: &'static str,
    /// Attributes in order; `None` is an attribute without a value.
    attributes: Vec<(String, Option<String>)>,
    content: Option<Content>,
}

impl Tag {
    fn new(key: impl Into<String>, element: &'static str) -> Self {
        Self {
            key: key.into(),
            element,
            attributes: Vec::new(),
            content: None,
        }
    }

    fn attr(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.push((name.into(), Some(value.into())));
        self
    }

    fn attr_opt(self, name: &str, value: Option<impl Into<String>>) -> Self {
        match value {
            Some(value) => self.attr(name, value),
            None => self,
        }
    }

    fn flag(mut self, name: &str) -> Self {
        self.attributes.push((name.to_string(), None));
        self
    }

    fn meta(key: impl Into<String>, name: &str, content: impl Into<String>) -> Self {
        Self::new(key, "meta")
            .attr("name", name)
            .attr("content", content)
    }

    fn property(key: impl Into<String>, property: &str, content: impl Into<String>) -> Self {
        Self::new(key, "meta")
            .attr("property", property)
            .attr("content", content)
    }

    fn link(key: impl Into<String>, rel: &str, href: impl Into<String>) -> Self {
        Self::new(key, "link").attr("rel", rel).attr("href", href)
    }

    /// Whether this is the `<title>` element.
    pub(super) fn is_title(&self) -> bool {
        self.element == "title"
    }

    /// Whether this is the `viewport` meta tag.
    pub(super) fn is_viewport(&self) -> bool {
        self.element == "meta"
            && self
                .attributes
                .iter()
                .any(|(name, value)| name == "name" && value.as_deref() == Some("viewport"))
    }

    /// The attributes that are written. A name is written as it is, not
    /// escaped, so one that is not a valid HTML attribute name is left out
    /// here as well as where the application supplies it: a name that ends
    /// the tag must never reach the page or the data.
    fn written_attributes(&self) -> impl Iterator<Item = &(String, Option<String>)> {
        self.attributes
            .iter()
            .filter(|(name, _)| attribute_name_problem(name).is_none())
    }

    /// The element as HTML, with its `data-inertia` key when `keyed`.
    pub(super) fn html(&self, keyed: bool) -> String {
        let mut html = format!("<{}", self.element);
        if keyed {
            html.push_str(" data-inertia=\"");
            html.push_str(&escape_html_attr(&self.key));
            html.push('"');
        }
        for (name, value) in self.written_attributes() {
            html.push(' ');
            html.push_str(name);
            if let Some(value) = value {
                html.push_str("=\"");
                html.push_str(&escape_html_attr(value));
                html.push('"');
            }
        }
        html.push('>');
        match &self.content {
            None => {}
            Some(Content::Text(text)) => {
                html.push_str(&escape_html_text(text));
                html.push_str(&format!("</{}>", self.element));
            }
            Some(Content::Json(value)) => {
                html.push_str(&script_safe_json(value));
                html.push_str(&format!("</{}>", self.element));
            }
        }
        html
    }

    /// The element as data.
    fn data(&self) -> Value {
        let mut object = Map::new();
        object.insert("key".to_string(), Value::String(self.key.clone()));
        object.insert(
            "element".to_string(),
            Value::String(self.element.to_string()),
        );
        let attributes: Map<String, Value> = self
            .written_attributes()
            .map(|(name, value)| {
                (
                    name.clone(),
                    value.clone().map_or(Value::Bool(true), Value::String),
                )
            })
            .collect();
        object.insert("attributes".to_string(), Value::Object(attributes));
        match &self.content {
            None => {}
            Some(Content::Text(text)) => {
                object.insert("content".to_string(), Value::String(text.clone()));
            }
            Some(Content::Json(value)) => {
                object.insert("content".to_string(), value.clone());
            }
        }
        Value::Object(object)
    }
}

/// `text` escaped for element content.
fn escape_html_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `value` as JSON that cannot end a script element or open a comment:
/// `<`, `>` and `&` only occur inside JSON strings, where their `\u`
/// escapes mean the same characters.
fn script_safe_json(value: &Value) -> String {
    value
        .to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

/// The tags as data, in order.
pub(super) fn to_array(tags: &[Tag]) -> Value {
    Value::Array(tags.iter().map(Tag::data).collect())
}

/// `url` with `https` in place of `http`.
fn force_https(url: String) -> String {
    match url.strip_prefix("http://") {
        Some(rest) => format!("https://{rest}"),
        None => url,
    }
}

/// The tags of `data`, in a fixed order. `request_url` is the absolute
/// URL a canonical link without an address names.
pub(super) fn tags(data: &HeadData, request_url: Option<&str>) -> Vec<Tag> {
    let mut tags = Vec::new();
    if let Some(title) = &data.title {
        let mut tag = Tag::new("title", "title");
        tag.content = Some(Content::Text(title.clone()));
        tags.push(tag);
    }
    if let Some(description) = &data.description {
        tags.push(Tag::meta("description", "description", description));
    }
    if let Some(canonical) = &data.canonical {
        let url = match &canonical.url {
            Some(url) => Some(crate::routing::url::to(url)),
            None => request_url.map(str::to_string),
        };
        // Without a URL there is no link: the facade logs that, and a
        // builder rendered alone has no request to name.
        if let Some(url) = url {
            let url = if canonical.force_https {
                force_https(url)
            } else {
                url
            };
            tags.push(Tag::link("canonical", "canonical", url));
        }
    }
    if let Some(robots) = &data.robots {
        tags.push(Tag::meta("robots", "robots", robots));
    }
    for (color, media) in &data.theme_colors {
        let key = match media {
            Some(media) => format!("theme-color:{}", media.as_str()),
            None => "theme-color".to_string(),
        };
        tags.push(
            Tag::meta(key, "theme-color", color)
                .attr_opt("media", media.as_ref().map(|media| media.as_str())),
        );
    }
    for (name, value) in [
        ("application-name", &data.application_name),
        ("color-scheme", &data.color_scheme),
        ("referrer", &data.referrer),
        ("viewport", &data.viewport),
        ("apple-mobile-web-app-title", &data.apple_web_app_title),
    ] {
        if let Some(value) = value {
            tags.push(Tag::meta(name, name, value));
        }
    }
    if data.web_app_capable == Some(true) {
        for name in ["mobile-web-app-capable", "apple-mobile-web-app-capable"] {
            tags.push(Tag::meta(name, name, "yes"));
        }
    }
    if let Some(style) = &data.apple_web_app_status_bar_style {
        let name = "apple-mobile-web-app-status-bar-style";
        tags.push(Tag::meta(name, name, style));
    }
    for (rel, icons) in [
        ("icon", &data.icons),
        ("apple-touch-icon", &data.apple_touch_icons),
        (
            "apple-touch-startup-image",
            &data.apple_touch_startup_images,
        ),
    ] {
        for icon in icons {
            tags.push(icon_tag(rel, icon));
        }
    }
    if let Some((href, color)) = &data.mask_icon {
        tags.push(Tag::link("mask-icon", "mask-icon", href).attr_opt("color", color.clone()));
    }
    if let Some(manifest) = &data.manifest {
        tags.push(Tag::link("manifest", "manifest", manifest));
    }
    open_graph_tags(data, &mut tags);
    twitter_tags(data, &mut tags);
    for (rel, hints) in [
        ("preconnect", &data.preconnects),
        ("dns-prefetch", &data.dns_prefetches),
        ("preload", &data.preloads),
        ("prefetch", &data.prefetches),
    ] {
        for hint in hints {
            tags.push(hint_tag(rel, hint));
        }
    }
    if let Some(prev) = &data.prev {
        tags.push(Tag::link("prev", "prev", prev));
    }
    if let Some(next) = &data.next {
        tags.push(Tag::link("next", "next", next));
    }
    for (hreflang, href) in &data.alternates {
        tags.push(
            Tag::new(format!("alternate:{hreflang}"), "link")
                .attr("rel", "alternate")
                .attr("hreflang", hreflang)
                .attr("href", href),
        );
    }
    for feed in &data.feeds {
        tags.push(
            Tag::new(format!("feed:{}", feed.href), "link")
                .attr("rel", "alternate")
                .attr("type", feed.kind)
                .attr_opt("title", feed.title.clone())
                .attr("href", &feed.href),
        );
    }
    for meta in &data.metas {
        let key = match &meta.media {
            Some(media) => format!("meta:{}:{}", meta.name, media.as_str()),
            None => format!("meta:{}", meta.name),
        };
        let attribute = if meta.uses_property() {
            "property"
        } else {
            "name"
        };
        tags.push(
            Tag::new(key, "meta")
                .attr(attribute, &meta.name)
                .attr("content", &meta.content)
                .attr_opt("media", meta.media.as_ref().map(|media| media.as_str())),
        );
    }
    for link in &data.links {
        let mut tag = Tag::link(
            format!("link:{}:{}", link.rel, link.href),
            &link.rel,
            &link.href,
        );
        for (name, value) in &link.attributes {
            tag = tag.attr(name.clone(), value);
        }
        tags.push(tag);
    }
    for (index, schema) in data.schemas.iter().enumerate() {
        let mut tag =
            Tag::new(format!("schema:{index}"), "script").attr("type", "application/ld+json");
        tag.content = Some(Content::Json(schema.to_value()));
        tags.push(tag);
    }
    tags
}

fn icon_tag(rel: &str, icon: &Icon) -> Tag {
    Tag::link(format!("{rel}:{}", icon.href), rel, &icon.href)
        .attr_opt("type", icon.kind.clone())
        .attr_opt("sizes", icon.sizes.clone())
        .attr_opt("media", icon.media.as_ref().map(|media| media.as_str()))
}

fn hint_tag(rel: &str, hint: &Hint) -> Tag {
    let tag = Tag::link(format!("{rel}:{}", hint.href), rel, &hint.href)
        .attr_opt("as", hint.as_kind.clone())
        .attr_opt("type", hint.kind.clone());
    if hint.crossorigin {
        tag.flag("crossorigin")
    } else {
        tag
    }
}

/// The Open Graph tags, when any Open Graph value or media is set. The
/// document title and description fill a missing `og:title` and
/// `og:description`.
fn open_graph_tags(data: &HeadData, tags: &mut Vec<Tag>) {
    let og = &data.og;
    let has_values = og.kind.is_some()
        || og.title.is_some()
        || og.description.is_some()
        || og.url.is_some()
        || og.site_name.is_some()
        || og.locale.is_some();
    if !has_values
        && data.og_images.is_empty()
        && data.og_videos.is_empty()
        && data.og_audios.is_empty()
    {
        return;
    }
    let title = og.title.as_ref().or(data.title.as_ref());
    let description = og.description.as_ref().or(data.description.as_ref());
    for (property, value) in [
        (
            "og:type",
            og.kind.as_ref().map(|kind| kind.as_str().to_string()),
        ),
        ("og:title", title.cloned()),
        ("og:description", description.cloned()),
        ("og:url", og.url.clone()),
        ("og:site_name", og.site_name.clone()),
        ("og:locale", og.locale.clone()),
    ] {
        if let Some(value) = value {
            tags.push(Tag::property(property, property, value));
        }
    }
    for (kind, media) in [
        ("og:image", &data.og_images),
        ("og:video", &data.og_videos),
        ("og:audio", &data.og_audios),
    ] {
        for (index, entry) in media.iter().enumerate() {
            media_tags(kind, index, entry, tags);
        }
    }
}

/// One Open Graph media entry: the URL, then each attribute it sets.
fn media_tags(kind: &str, index: usize, entry: &OgMedia, tags: &mut Vec<Tag>) {
    let key = format!("{kind}:{index}");
    tags.push(Tag::property(key.clone(), kind, &entry.url));
    let attributes = [
        ("secure_url", entry.secure_url.clone()),
        ("type", entry.kind.clone()),
        ("width", entry.width.map(|width| width.to_string())),
        ("height", entry.height.map(|height| height.to_string())),
        ("alt", entry.alt.clone()),
    ];
    for (attribute, value) in attributes {
        if let Some(value) = value {
            let property = format!("{kind}:{attribute}");
            tags.push(Tag::property(
                format!("{key}:{attribute}"),
                &property,
                value,
            ));
        }
    }
}

/// The X card tags, when a layer turned the card on. The title, the
/// description and the image fall back to the document's and the first
/// Open Graph image.
fn twitter_tags(data: &HeadData, tags: &mut Vec<Tag>) {
    let Some(twitter) = &data.twitter else {
        return;
    };
    let image = data.twitter_image.as_ref().or(data.og_images.first());
    let fields = [
        (
            "twitter:card",
            twitter.card.map(|card| card.as_str().to_string()),
        ),
        ("twitter:site", twitter.site.clone()),
        ("twitter:creator", twitter.creator.clone()),
        (
            "twitter:title",
            twitter.title.clone().or_else(|| data.title.clone()),
        ),
        (
            "twitter:description",
            twitter
                .description
                .clone()
                .or_else(|| data.description.clone()),
        ),
        ("twitter:image", image.map(|image| image.url.clone())),
        (
            "twitter:image:alt",
            image.and_then(|image| image.alt.clone()),
        ),
    ];
    for (name, value) in fields {
        if let Some(value) = value {
            tags.push(Tag::meta(name, name, value));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_leaves_out_an_attribute_name_that_could_end_it() {
        let tag = Tag::new("link:x", "link")
            .attr("rel", "alternate")
            .attr("x><script>alert(1)</script><link x", "v")
            .flag("a b")
            .attr("hreflang", "fr");
        let html = tag.html(true);
        assert_eq!(
            html,
            "<link data-inertia=\"link:x\" rel=\"alternate\" hreflang=\"fr\">"
        );
        let data = tag.data();
        let attributes = data["attributes"].as_object().expect("the attributes");
        assert_eq!(attributes.len(), 2, "{data}");
        assert!(attributes.contains_key("rel") && attributes.contains_key("hreflang"));
    }
}
