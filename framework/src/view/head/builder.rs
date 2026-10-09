//! The head builder and the values it takes.
//!
//! A [`HeadBuilder`] holds one layer of a page's head: the defaults, a
//! route group's metadata, a route's, what a request sets at run time, or
//! an error status's. Single-value fields replace; repeatable fields keep
//! every entry and update the one with the same key, the way Laravel Head
//! merges calls.

use serde_json::Value;

use super::schema::Schema;

/// Where a tag applies: a color scheme, an orientation, or any media query.
/// Laravel Head's `Media` enum, with a custom query as a variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Media {
    /// `(prefers-color-scheme: light)`.
    Light,
    /// `(prefers-color-scheme: dark)`.
    Dark,
    /// `(orientation: portrait)`.
    Portrait,
    /// `(orientation: landscape)`.
    Landscape,
    /// Any other media query, written as it is.
    Query(String),
}

impl Media {
    /// A custom media query, such as `(min-width: 600px)`.
    pub fn query(query: impl Into<String>) -> Self {
        Self::Query(query.into())
    }

    /// The media query the `media` attribute carries.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Light => "(prefers-color-scheme: light)",
            Self::Dark => "(prefers-color-scheme: dark)",
            Self::Portrait => "(orientation: portrait)",
            Self::Landscape => "(orientation: landscape)",
            Self::Query(query) => query,
        }
    }
}

/// The Open Graph object type (`og:type`). Laravel Head's `OgType`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OgType {
    /// `website`.
    Website,
    /// `article`.
    Article,
    /// `profile`.
    Profile,
    /// `book`.
    Book,
    /// `product`.
    Product,
    /// `music.song`.
    MusicSong,
    /// `music.album`.
    MusicAlbum,
    /// `video.movie`.
    VideoMovie,
    /// `video.episode`.
    VideoEpisode,
    /// `video.other`.
    VideoOther,
    /// Any other type, written as it is.
    Other(String),
}

impl OgType {
    /// The value `og:type` carries.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Website => "website",
            Self::Article => "article",
            Self::Profile => "profile",
            Self::Book => "book",
            Self::Product => "product",
            Self::MusicSong => "music.song",
            Self::MusicAlbum => "music.album",
            Self::VideoMovie => "video.movie",
            Self::VideoEpisode => "video.episode",
            Self::VideoOther => "video.other",
            Self::Other(kind) => kind,
        }
    }
}

/// An image MIME type, for the `type` of an icon or an Open Graph image.
/// Laravel Head's `ImageType`. Every call that takes one also takes the
/// MIME type as a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageType {
    /// `image/svg+xml`.
    Svg,
    /// `image/png`.
    Png,
    /// `image/jpeg`.
    Jpeg,
    /// `image/gif`.
    Gif,
    /// `image/webp`.
    Webp,
    /// `image/avif`.
    Avif,
    /// `image/x-icon`.
    Ico,
}

impl ImageType {
    /// The MIME type.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Svg => "image/svg+xml",
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Avif => "image/avif",
            Self::Ico => "image/x-icon",
        }
    }
}

impl From<ImageType> for String {
    fn from(kind: ImageType) -> Self {
        kind.as_str().to_string()
    }
}

/// The X (Twitter) card type (`twitter:card`). Laravel Head's
/// `TwitterCard` enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TwitterCardType {
    /// `summary`.
    Summary,
    /// `summary_large_image`.
    SummaryLargeImage,
    /// `app`.
    App,
    /// `player`.
    Player,
}

impl TwitterCardType {
    /// The value `twitter:card` carries.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::SummaryLargeImage => "summary_large_image",
            Self::App => "app",
            Self::Player => "player",
        }
    }
}

/// One robots directive. Laravel Head's `RobotsRule`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RobotsRule {
    /// `all`.
    All,
    /// `none`.
    None,
    /// `index`.
    Index,
    /// `noindex`.
    NoIndex,
    /// `follow`.
    Follow,
    /// `nofollow`.
    NoFollow,
    /// `noarchive`.
    NoArchive,
    /// `nosnippet`.
    NoSnippet,
    /// `noimageindex`.
    NoImageIndex,
    /// `notranslate`.
    NoTranslate,
}

impl RobotsRule {
    /// The directive.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::None => "none",
            Self::Index => "index",
            Self::NoIndex => "noindex",
            Self::Follow => "follow",
            Self::NoFollow => "nofollow",
            Self::NoArchive => "noarchive",
            Self::NoSnippet => "nosnippet",
            Self::NoImageIndex => "noimageindex",
            Self::NoTranslate => "notranslate",
        }
    }
}

/// The Open Graph values that are not media: `og(...)`'s named arguments
/// in Laravel Head. Unset values fall back: the title and description to
/// the document's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpenGraph {
    pub(super) kind: Option<OgType>,
    pub(super) title: Option<String>,
    pub(super) description: Option<String>,
    pub(super) url: Option<String>,
    pub(super) site_name: Option<String>,
    pub(super) locale: Option<String>,
    pub(super) image: Option<String>,
}

impl OpenGraph {
    /// No values yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// `og:type`.
    pub fn kind(mut self, kind: OgType) -> Self {
        self.kind = Some(kind);
        self
    }

    /// `og:title`, when it differs from the document title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// `og:description`, when it differs from the document description.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// `og:url`.
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// `og:site_name`.
    pub fn site_name(mut self, site_name: impl Into<String>) -> Self {
        self.site_name = Some(site_name.into());
        self
    }

    /// `og:locale`.
    pub fn locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = Some(locale.into());
        self
    }

    /// One image with no other attributes, written to the same list
    /// [`HeadBuilder::og_image`] adds to.
    pub fn image(mut self, url: impl Into<String>) -> Self {
        self.image = Some(url.into());
        self
    }

    fn overlay(&mut self, higher: &Self) {
        overlay_option(&mut self.kind, &higher.kind);
        overlay_option(&mut self.title, &higher.title);
        overlay_option(&mut self.description, &higher.description);
        overlay_option(&mut self.url, &higher.url);
        overlay_option(&mut self.site_name, &higher.site_name);
        overlay_option(&mut self.locale, &higher.locale);
    }
}

/// One Open Graph image, video or audio entry, or an X card image. The URL
/// is its key: adding the same URL again updates the earlier entry.
#[derive(Clone, Debug, PartialEq)]
pub struct OgMedia {
    pub(super) url: String,
    pub(super) secure_url: Option<String>,
    pub(super) kind: Option<String>,
    pub(super) width: Option<u32>,
    pub(super) height: Option<u32>,
    pub(super) alt: Option<String>,
}

impl OgMedia {
    /// An entry for `url`.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            secure_url: None,
            kind: None,
            width: None,
            height: None,
            alt: None,
        }
    }

    /// `:secure_url`, the HTTPS address of the same file.
    pub fn secure_url(mut self, url: impl Into<String>) -> Self {
        self.secure_url = Some(url.into());
        self
    }

    /// `:type`, the MIME type: an [`ImageType`] or a string.
    pub fn kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = Some(kind.into());
        self
    }

    /// `:width`, in pixels.
    pub fn width(mut self, width: u32) -> Self {
        self.width = Some(width);
        self
    }

    /// `:height`, in pixels.
    pub fn height(mut self, height: u32) -> Self {
        self.height = Some(height);
        self
    }

    /// `:alt`, the text that describes an image.
    pub fn alt(mut self, alt: impl Into<String>) -> Self {
        self.alt = Some(alt.into());
        self
    }
}

impl From<&str> for OgMedia {
    fn from(url: &str) -> Self {
        Self::new(url)
    }
}

impl From<String> for OgMedia {
    fn from(url: String) -> Self {
        Self::new(url)
    }
}

/// The X (Twitter) card values: `twitter(...)`'s named arguments in
/// Laravel Head. Setting one in any layer turns the card on; the title,
/// description and image fall back to the document's and the first Open
/// Graph image.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TwitterCard {
    pub(super) card: Option<TwitterCardType>,
    pub(super) site: Option<String>,
    pub(super) creator: Option<String>,
    pub(super) title: Option<String>,
    pub(super) description: Option<String>,
}

impl TwitterCard {
    /// No values yet: the card still renders, from the page's values.
    pub fn new() -> Self {
        Self::default()
    }

    /// `twitter:card`.
    pub fn card(mut self, card: TwitterCardType) -> Self {
        self.card = Some(card);
        self
    }

    /// `twitter:site`, the site's account.
    pub fn site(mut self, site: impl Into<String>) -> Self {
        self.site = Some(site.into());
        self
    }

    /// `twitter:creator`, the author's account.
    pub fn creator(mut self, creator: impl Into<String>) -> Self {
        self.creator = Some(creator.into());
        self
    }

    /// `twitter:title`, when it differs from the document title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// `twitter:description`, when it differs from the document
    /// description.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    fn overlay(&mut self, higher: &Self) {
        overlay_option(&mut self.card, &higher.card);
        overlay_option(&mut self.site, &higher.site);
        overlay_option(&mut self.creator, &higher.creator);
        overlay_option(&mut self.title, &higher.title);
        overlay_option(&mut self.description, &higher.description);
    }
}

/// An icon link: `icon`, `apple-touch-icon` or `apple-touch-startup-image`.
/// The address is its key.
#[derive(Clone, Debug, PartialEq)]
pub struct Icon {
    pub(super) href: String,
    pub(super) kind: Option<String>,
    pub(super) sizes: Option<String>,
    pub(super) media: Option<Media>,
}

impl Icon {
    /// An icon at `href`.
    pub fn new(href: impl Into<String>) -> Self {
        Self {
            href: href.into(),
            kind: None,
            sizes: None,
            media: None,
        }
    }

    /// The `type`: an [`ImageType`] or a MIME type string.
    pub fn kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = Some(kind.into());
        self
    }

    /// The `sizes`, such as `32x32`.
    pub fn sizes(mut self, sizes: impl Into<String>) -> Self {
        self.sizes = Some(sizes.into());
        self
    }

    /// The `media` the icon applies under.
    pub fn media(mut self, media: Media) -> Self {
        self.media = Some(media);
        self
    }
}

impl From<&str> for Icon {
    fn from(href: &str) -> Self {
        Self::new(href)
    }
}

impl From<String> for Icon {
    fn from(href: String) -> Self {
        Self::new(href)
    }
}

/// A resource hint: `preload`, `prefetch`, `preconnect` or `dns-prefetch`.
/// The address is its key.
#[derive(Clone, Debug, PartialEq)]
pub struct Hint {
    pub(super) href: String,
    pub(super) as_kind: Option<String>,
    pub(super) kind: Option<String>,
    pub(super) crossorigin: bool,
}

impl Hint {
    /// A hint for `href`.
    pub fn new(href: impl Into<String>) -> Self {
        Self {
            href: href.into(),
            as_kind: None,
            kind: None,
            crossorigin: false,
        }
    }

    /// The `as` attribute: what the resource is (`font`, `image`,
    /// `script`, `style`). A browser ignores a preload without it.
    pub fn as_kind(mut self, as_kind: impl Into<String>) -> Self {
        self.as_kind = Some(as_kind.into());
        self
    }

    /// The `type` attribute, the resource's MIME type.
    pub fn kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = Some(kind.into());
        self
    }

    /// Add `crossorigin`, which a font preload needs even from the same
    /// origin.
    pub fn crossorigin(mut self) -> Self {
        self.crossorigin = true;
        self
    }
}

impl From<&str> for Hint {
    fn from(href: &str) -> Self {
        Self::new(href)
    }
}

impl From<String> for Hint {
    fn from(href: String) -> Self {
        Self::new(href)
    }
}

/// A feed discovery link. The address is its key.
#[derive(Clone, Debug, PartialEq)]
pub struct Feed {
    pub(super) href: String,
    pub(super) kind: &'static str,
    pub(super) title: Option<String>,
}

impl Feed {
    /// An RSS feed at `href`.
    pub fn rss(href: impl Into<String>) -> Self {
        Self {
            href: href.into(),
            kind: "application/rss+xml",
            title: None,
        }
    }

    /// An Atom feed at `href`.
    pub fn atom(href: impl Into<String>) -> Self {
        Self {
            href: href.into(),
            kind: "application/atom+xml",
            title: None,
        }
    }

    /// The feed's `title`.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
}

/// A custom `<meta>` tag. Its name and media are its key.
#[derive(Clone, Debug, PartialEq)]
pub struct MetaTag {
    pub(super) name: String,
    pub(super) content: String,
    pub(super) media: Option<Media>,
    pub(super) property: Option<bool>,
}

impl MetaTag {
    /// `name` with `content`. The tag writes `property` for an `og:` or
    /// `article:` name and `name` otherwise, unless
    /// [`Self::property`] says which.
    pub fn new(name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            content: content.into(),
            media: None,
            property: None,
        }
    }

    /// The `media` the tag applies under.
    pub fn media(mut self, media: Media) -> Self {
        self.media = Some(media);
        self
    }

    /// Write `property` (`true`) or `name` (`false`) whatever the name is.
    pub fn property(mut self, property: bool) -> Self {
        self.property = Some(property);
        self
    }

    pub(super) fn uses_property(&self) -> bool {
        self.property
            .unwrap_or_else(|| self.name.starts_with("og:") || self.name.starts_with("article:"))
    }
}

/// A custom `<link>` tag. Its `rel` and address are its key.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkTag {
    pub(super) rel: String,
    pub(super) href: String,
    pub(super) attributes: Vec<(String, String)>,
}

impl LinkTag {
    /// A `rel` link to `href`.
    pub fn new(rel: impl Into<String>, href: impl Into<String>) -> Self {
        Self {
            rel: rel.into(),
            href: href.into(),
            attributes: Vec::new(),
        }
    }

    /// Add an attribute, such as `type` or `title`. A later one with the
    /// same name replaces it.
    pub fn attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let name = name.into();
        let value = value.into();
        match self.attributes.iter_mut().find(|(key, _)| *key == name) {
            Some(slot) => slot.1 = value,
            None => self.attributes.push((name, value)),
        }
        self
    }
}

/// The tags an installable web app needs. Laravel Head's `pwa(...)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Pwa {
    name: String,
    manifest: Option<String>,
    theme_color: Option<String>,
    apple_touch_icon: Option<String>,
    apple_web_app_status_bar_style: Option<String>,
}

impl Pwa {
    /// An app called `name`: the application name and the Apple web app
    /// title, and web app capable.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            manifest: None,
            theme_color: None,
            apple_touch_icon: None,
            apple_web_app_status_bar_style: None,
        }
    }

    /// The web app manifest link.
    pub fn manifest(mut self, href: impl Into<String>) -> Self {
        self.manifest = Some(href.into());
        self
    }

    /// The theme color.
    pub fn theme_color(mut self, color: impl Into<String>) -> Self {
        self.theme_color = Some(color.into());
        self
    }

    /// The Apple touch icon.
    pub fn apple_touch_icon(mut self, href: impl Into<String>) -> Self {
        self.apple_touch_icon = Some(href.into());
        self
    }

    /// The Apple status bar style, such as `black`.
    pub fn apple_web_app_status_bar_style(mut self, style: impl Into<String>) -> Self {
        self.apple_web_app_status_bar_style = Some(style.into());
        self
    }
}

/// A canonical URL: the request's own, or one given.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Canonical {
    /// `None` for the request's URL.
    pub(super) url: Option<String>,
    pub(super) force_https: bool,
}

/// One layer of head metadata. Every field is unset until a call sets it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct HeadData {
    pub(super) title: Option<String>,
    pub(super) title_prefix: Option<String>,
    pub(super) title_suffix: Option<String>,
    /// The title ignores the prefix and suffix of the layers below.
    pub(super) exact_title: bool,
    pub(super) description: Option<String>,
    pub(super) canonical: Option<Canonical>,
    pub(super) robots: Option<String>,
    pub(super) og: OpenGraph,
    pub(super) og_images: Vec<OgMedia>,
    pub(super) og_videos: Vec<OgMedia>,
    pub(super) og_audios: Vec<OgMedia>,
    pub(super) twitter: Option<TwitterCard>,
    pub(super) twitter_image: Option<OgMedia>,
    pub(super) theme_colors: Vec<(String, Option<Media>)>,
    pub(super) application_name: Option<String>,
    pub(super) color_scheme: Option<String>,
    pub(super) referrer: Option<String>,
    pub(super) viewport: Option<String>,
    pub(super) apple_web_app_title: Option<String>,
    pub(super) web_app_capable: Option<bool>,
    pub(super) apple_web_app_status_bar_style: Option<String>,
    pub(super) icons: Vec<Icon>,
    pub(super) apple_touch_icons: Vec<Icon>,
    pub(super) apple_touch_startup_images: Vec<Icon>,
    pub(super) mask_icon: Option<(String, Option<String>)>,
    pub(super) manifest: Option<String>,
    pub(super) preloads: Vec<Hint>,
    pub(super) prefetches: Vec<Hint>,
    pub(super) preconnects: Vec<Hint>,
    pub(super) dns_prefetches: Vec<Hint>,
    pub(super) prev: Option<String>,
    pub(super) next: Option<String>,
    pub(super) alternates: Vec<(String, String)>,
    pub(super) feeds: Vec<Feed>,
    pub(super) metas: Vec<MetaTag>,
    pub(super) links: Vec<LinkTag>,
    pub(super) schemas: Vec<Schema>,
}

impl HeadData {
    /// Whether no call set anything.
    pub(super) fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Lay `higher` over this layer, field by field: a value `higher` sets
    /// wins, a repeatable entry `higher` adds updates the entry with the
    /// same key, and `higher`'s Open Graph media of one kind replaces this
    /// layer's media of that kind, so a page's image is not merged with a
    /// default one.
    ///
    /// The title is laid over as Laravel Head resolves it: `higher`'s
    /// title takes this layer's prefix and suffix unless it is exact, and
    /// the result keeps the prefix and suffix for a layer above. The
    /// merged title is marked exact, since its affixes are applied.
    pub(super) fn overlay(&mut self, higher: &Self) {
        if let Some(title) = &higher.title {
            self.title = Some(if higher.exact_title {
                title.clone()
            } else {
                format!(
                    "{}{title}{}",
                    self.title_prefix.as_deref().unwrap_or_default(),
                    self.title_suffix.as_deref().unwrap_or_default()
                )
            });
        }
        self.exact_title = self.title.is_some();
        overlay_option(&mut self.title_prefix, &higher.title_prefix);
        overlay_option(&mut self.title_suffix, &higher.title_suffix);
        overlay_option(&mut self.description, &higher.description);
        overlay_option(&mut self.canonical, &higher.canonical);
        overlay_option(&mut self.robots, &higher.robots);
        self.og.overlay(&higher.og);
        for (mine, theirs) in [
            (&mut self.og_images, &higher.og_images),
            (&mut self.og_videos, &higher.og_videos),
            (&mut self.og_audios, &higher.og_audios),
        ] {
            if !theirs.is_empty() {
                mine.clone_from(theirs);
            }
        }
        match (&mut self.twitter, &higher.twitter) {
            (Some(mine), Some(theirs)) => mine.overlay(theirs),
            (mine @ None, Some(theirs)) => *mine = Some(theirs.clone()),
            (_, None) => {}
        }
        overlay_option(&mut self.twitter_image, &higher.twitter_image);
        for (color, media) in &higher.theme_colors {
            upsert(
                &mut self.theme_colors,
                (color.clone(), media.clone()),
                |a, b| a.1 == b.1,
            );
        }
        overlay_option(&mut self.application_name, &higher.application_name);
        overlay_option(&mut self.color_scheme, &higher.color_scheme);
        overlay_option(&mut self.referrer, &higher.referrer);
        overlay_option(&mut self.viewport, &higher.viewport);
        overlay_option(&mut self.apple_web_app_title, &higher.apple_web_app_title);
        overlay_option(&mut self.web_app_capable, &higher.web_app_capable);
        overlay_option(
            &mut self.apple_web_app_status_bar_style,
            &higher.apple_web_app_status_bar_style,
        );
        for (mine, theirs) in [
            (&mut self.icons, &higher.icons),
            (&mut self.apple_touch_icons, &higher.apple_touch_icons),
            (
                &mut self.apple_touch_startup_images,
                &higher.apple_touch_startup_images,
            ),
        ] {
            for icon in theirs {
                upsert(mine, icon.clone(), |a, b| a.href == b.href);
            }
        }
        overlay_option(&mut self.mask_icon, &higher.mask_icon);
        overlay_option(&mut self.manifest, &higher.manifest);
        for (mine, theirs) in [
            (&mut self.preloads, &higher.preloads),
            (&mut self.prefetches, &higher.prefetches),
            (&mut self.preconnects, &higher.preconnects),
            (&mut self.dns_prefetches, &higher.dns_prefetches),
        ] {
            for hint in theirs {
                upsert(mine, hint.clone(), |a, b| a.href == b.href);
            }
        }
        overlay_option(&mut self.prev, &higher.prev);
        overlay_option(&mut self.next, &higher.next);
        for alternate in &higher.alternates {
            upsert(&mut self.alternates, alternate.clone(), |a, b| a.0 == b.0);
        }
        for feed in &higher.feeds {
            upsert(&mut self.feeds, feed.clone(), |a, b| a.href == b.href);
        }
        for meta in &higher.metas {
            upsert(&mut self.metas, meta.clone(), |a, b| {
                a.name == b.name && a.media == b.media
            });
        }
        for link in &higher.links {
            upsert(&mut self.links, link.clone(), |a, b| {
                a.rel == b.rel && a.href == b.href
            });
        }
        self.schemas.extend(higher.schemas.iter().cloned());
    }
}

fn overlay_option<T: Clone>(mine: &mut Option<T>, theirs: &Option<T>) {
    if theirs.is_some() {
        mine.clone_from(theirs);
    }
}

/// Replace the entry `same` matches, in its place, or append `item`.
fn upsert<T>(list: &mut Vec<T>, item: T, same: impl Fn(&T, &T) -> bool) {
    match list.iter_mut().find(|entry| same(entry, &item)) {
        Some(entry) => *entry = item,
        None => list.push(item),
    }
}

/// One layer of a page's document head, built with chained calls.
///
/// [`Head::defaults`](crate::Head::defaults), `with_head` on a route or a
/// group, [`Head::errors`](crate::Head::errors) and
/// [`Head::inertia_globals`](crate::Head::inertia_globals) hand you one to
/// fill; the run-time calls on [`Head`](crate::Head) fill the request's.
/// A single-value call replaces an earlier one; a repeatable call adds an
/// entry and updates the earlier one with the same key (an Open Graph
/// image's URL, an icon's address, a meta tag's name and media).
///
/// ```rust
/// use suprnova::head::{HeadBuilder, Media, OgMedia};
///
/// let head = HeadBuilder::new()
///     .title("Laravel")
///     .title_suffix(" - Laravel")
///     .description("Build something great.")
///     .theme_color("#ffffff", Media::Light)
///     .og_image(OgMedia::new("/cover.jpg").alt("Cover"));
/// assert!(head.render_html().contains("<title data-inertia=\"title\">Laravel</title>"));
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeadBuilder {
    pub(super) data: HeadData,
}

impl HeadBuilder {
    /// An empty layer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Lay `higher` over this layer, field by field, as `Head` resolves
    /// its layers: what `higher` sets wins, and `higher`'s title takes
    /// this layer's prefix and suffix unless it is exact.
    pub fn merge(mut self, higher: HeadBuilder) -> Self {
        self.data.overlay(&higher.data);
        self
    }

    /// Render this layer alone as tags, each with its `data-inertia` key
    /// and every value HTML-escaped. A canonical URL that names no address
    /// needs the request's URL, which only the request's head has; it is
    /// left out here.
    pub fn render_html(&self) -> String {
        super::render::tags(&self.data, None)
            .iter()
            .map(|tag| tag.html(true))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// This layer alone as data: one object per tag, with its key, element,
    /// attributes and content.
    pub fn to_array(&self) -> Value {
        super::render::to_array(&super::render::tags(&self.data, None))
    }

    /// The page title. A higher layer's title takes the prefix and suffix
    /// of the layers below it; this layer's own title does not take its
    /// own.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.data.title = Some(title.into());
        self.data.exact_title = false;
        self
    }

    /// A title that takes no inherited prefix or suffix.
    pub fn exact_title(mut self, title: impl Into<String>) -> Self {
        self.data.title = Some(title.into());
        self.data.exact_title = true;
        self
    }

    /// The text written before a higher layer's title.
    pub fn title_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.data.title_prefix = Some(prefix.into());
        self
    }

    /// The text written after a higher layer's title, such as ` - Laravel`.
    pub fn title_suffix(mut self, suffix: impl Into<String>) -> Self {
        self.data.title_suffix = Some(suffix.into());
        self
    }

    /// The `description` meta tag, which also fills a missing Open Graph
    /// and X description.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.data.description = Some(description.into());
        self
    }

    /// A canonical link to the request's URL, over `https`.
    pub fn canonical(mut self) -> Self {
        self.data.canonical = Some(Canonical {
            url: None,
            force_https: true,
        });
        self
    }

    /// A canonical link to `url`, over `https`. A path is made absolute on
    /// the application's origin and public root.
    pub fn canonical_url(mut self, url: impl Into<String>) -> Self {
        self.data.canonical = Some(Canonical {
            url: Some(url.into()),
            force_https: true,
        });
        self
    }

    /// Keep the canonical URL's own scheme instead of `https`. Laravel
    /// Head's `forceHttps: false`.
    pub fn canonical_keep_scheme(mut self) -> Self {
        if let Some(canonical) = &mut self.data.canonical {
            canonical.force_https = false;
        }
        self
    }

    /// The `robots` directives, as written, such as `noindex, nofollow`.
    pub fn robots(mut self, directives: impl Into<String>) -> Self {
        self.data.robots = Some(directives.into());
        self
    }

    /// The `robots` directives from a list, comma-separated.
    pub fn robots_rules(mut self, rules: impl IntoIterator<Item = RobotsRule>) -> Self {
        self.data.robots = Some(
            rules
                .into_iter()
                .map(RobotsRule::as_str)
                .collect::<Vec<_>>()
                .join(", "),
        );
        self
    }

    /// `robots` `all`.
    pub fn searchable_by_robots(self) -> Self {
        self.robots("all")
    }

    /// `robots` `none`.
    pub fn hidden_from_robots(self) -> Self {
        self.robots("none")
    }

    /// Apply `then` when `condition` holds.
    pub fn when(self, condition: bool, then: impl FnOnce(Self) -> Self) -> Self {
        if condition { then(self) } else { self }
    }

    /// Apply `then` unless `condition` holds.
    pub fn unless(self, condition: bool, then: impl FnOnce(Self) -> Self) -> Self {
        self.when(!condition, then)
    }

    /// The Open Graph values, laid over earlier ones field by field.
    pub fn og(mut self, og: OpenGraph) -> Self {
        if let Some(image) = &og.image {
            upsert(
                &mut self.data.og_images,
                OgMedia::new(image.clone()),
                |a, b| a.url == b.url,
            );
        }
        let mut og = og;
        og.image = None;
        self.data.og.overlay(&og);
        self
    }

    /// Add an Open Graph image; the same URL again updates its entry.
    pub fn og_image(mut self, image: impl Into<OgMedia>) -> Self {
        upsert(&mut self.data.og_images, image.into(), |a, b| {
            a.url == b.url
        });
        self
    }

    /// Add an Open Graph video; the same URL again updates its entry.
    pub fn og_video(mut self, video: impl Into<OgMedia>) -> Self {
        upsert(&mut self.data.og_videos, video.into(), |a, b| {
            a.url == b.url
        });
        self
    }

    /// Add an Open Graph audio file; the same URL again updates its entry.
    pub fn og_audio(mut self, audio: impl Into<OgMedia>) -> Self {
        upsert(&mut self.data.og_audios, audio.into(), |a, b| {
            a.url == b.url
        });
        self
    }

    /// Turn the X (Twitter) card on with these values, laid over earlier
    /// ones field by field.
    pub fn twitter(mut self, twitter: TwitterCard) -> Self {
        match &mut self.data.twitter {
            Some(current) => current.overlay(&twitter),
            None => self.data.twitter = Some(twitter),
        }
        self
    }

    /// The X card image, in place of the first Open Graph image.
    pub fn twitter_image(mut self, image: impl Into<OgMedia>) -> Self {
        self.data.twitter_image = Some(image.into());
        self
    }

    /// A `theme-color`, for every media (`None`) or under one. The media is
    /// its key.
    pub fn theme_color(
        mut self,
        color: impl Into<String>,
        media: impl Into<Option<Media>>,
    ) -> Self {
        upsert(
            &mut self.data.theme_colors,
            (color.into(), media.into()),
            |a, b| a.1 == b.1,
        );
        self
    }

    /// `application-name`.
    pub fn application_name(mut self, name: impl Into<String>) -> Self {
        self.data.application_name = Some(name.into());
        self
    }

    /// `color-scheme`, such as `light dark`.
    pub fn color_scheme(mut self, scheme: impl Into<String>) -> Self {
        self.data.color_scheme = Some(scheme.into());
        self
    }

    /// `referrer`, the referrer policy.
    pub fn referrer(mut self, policy: impl Into<String>) -> Self {
        self.data.referrer = Some(policy.into());
        self
    }

    /// `viewport`. The framework's own Inertia document writes no viewport
    /// of its own while one is set here.
    pub fn viewport(mut self, viewport: impl Into<String>) -> Self {
        self.data.viewport = Some(viewport.into());
        self
    }

    /// `apple-mobile-web-app-title`.
    pub fn apple_web_app_title(mut self, title: impl Into<String>) -> Self {
        self.data.apple_web_app_title = Some(title.into());
        self
    }

    /// `mobile-web-app-capable` and `apple-mobile-web-app-capable` `yes`,
    /// so the page opens as a standalone app from the home screen.
    pub fn web_app_capable(mut self) -> Self {
        self.data.web_app_capable = Some(true);
        self
    }

    /// `apple-mobile-web-app-status-bar-style`, such as `black`.
    pub fn apple_web_app_status_bar_style(mut self, style: impl Into<String>) -> Self {
        self.data.apple_web_app_status_bar_style = Some(style.into());
        self
    }

    /// An `icon` link; the same address again updates its entry.
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        upsert(&mut self.data.icons, icon.into(), |a, b| a.href == b.href);
        self
    }

    /// The same as [`Self::icon`].
    pub fn favicon(self, icon: impl Into<Icon>) -> Self {
        self.icon(icon)
    }

    /// An `apple-touch-icon` link.
    pub fn apple_touch_icon(mut self, icon: impl Into<Icon>) -> Self {
        upsert(&mut self.data.apple_touch_icons, icon.into(), |a, b| {
            a.href == b.href
        });
        self
    }

    /// An `apple-touch-startup-image` link.
    pub fn apple_touch_startup_image(mut self, image: impl Into<Icon>) -> Self {
        upsert(
            &mut self.data.apple_touch_startup_images,
            image.into(),
            |a, b| a.href == b.href,
        );
        self
    }

    /// The Safari pinned-tab `mask-icon` and its color.
    pub fn mask_icon(mut self, href: impl Into<String>, color: impl Into<String>) -> Self {
        self.data.mask_icon = Some((href.into(), Some(color.into())));
        self
    }

    /// The web app `manifest` link.
    pub fn manifest(mut self, href: impl Into<String>) -> Self {
        self.data.manifest = Some(href.into());
        self
    }

    /// The tags an installable web app needs: the application name, the
    /// Apple web app title, web app capable, and the manifest, theme color,
    /// Apple touch icon and status bar style the [`Pwa`] names. Creating
    /// the manifest and the service worker stays the application's work.
    pub fn pwa(self, pwa: Pwa) -> Self {
        let mut head = self
            .application_name(pwa.name.clone())
            .apple_web_app_title(pwa.name)
            .web_app_capable();
        if let Some(manifest) = pwa.manifest {
            head = head.manifest(manifest);
        }
        if let Some(color) = pwa.theme_color {
            head = head.theme_color(color, None);
        }
        if let Some(icon) = pwa.apple_touch_icon {
            head = head.apple_touch_icon(icon);
        }
        if let Some(style) = pwa.apple_web_app_status_bar_style {
            head = head.apple_web_app_status_bar_style(style);
        }
        head
    }

    /// A `preload` hint; give it [`Hint::as_kind`], since a browser ignores
    /// a preload without `as`.
    pub fn preload(mut self, hint: impl Into<Hint>) -> Self {
        upsert(&mut self.data.preloads, hint.into(), |a, b| {
            a.href == b.href
        });
        self
    }

    /// A `prefetch` hint.
    pub fn prefetch(mut self, hint: impl Into<Hint>) -> Self {
        upsert(&mut self.data.prefetches, hint.into(), |a, b| {
            a.href == b.href
        });
        self
    }

    /// A `preconnect` hint.
    pub fn preconnect(mut self, hint: impl Into<Hint>) -> Self {
        upsert(&mut self.data.preconnects, hint.into(), |a, b| {
            a.href == b.href
        });
        self
    }

    /// A `dns-prefetch` hint.
    pub fn dns_prefetch(mut self, hint: impl Into<Hint>) -> Self {
        upsert(&mut self.data.dns_prefetches, hint.into(), |a, b| {
            a.href == b.href
        });
        self
    }

    /// The `prev` and `next` links of a paginated page, from the
    /// paginator's own links.
    pub fn paginate<T>(self, paginator: &dyn crate::pagination::Paginated<T>) -> Self {
        let mut head = self;
        for (rel, href) in paginator.links_iter() {
            head = match rel {
                "prev" => head.prev_page(href),
                "next" => head.next_page(href),
                _ => head,
            };
        }
        head
    }

    /// The `prev` link: the previous page of a paginated list.
    pub fn prev_page(mut self, href: impl Into<String>) -> Self {
        self.data.prev = Some(href.into());
        self
    }

    /// The `next` link: the next page of a paginated list.
    pub fn next_page(mut self, href: impl Into<String>) -> Self {
        self.data.next = Some(href.into());
        self
    }

    /// One `alternate` link per locale (`hreflang`), such as `en`, `fr` or
    /// `x-default`; the same locale again updates its entry.
    pub fn alternates<L, H>(mut self, alternates: impl IntoIterator<Item = (L, H)>) -> Self
    where
        L: Into<String>,
        H: Into<String>,
    {
        for (hreflang, href) in alternates {
            upsert(
                &mut self.data.alternates,
                (hreflang.into(), href.into()),
                |a, b| a.0 == b.0,
            );
        }
        self
    }

    /// A feed discovery link.
    pub fn feed(mut self, feed: Feed) -> Self {
        upsert(&mut self.data.feeds, feed, |a, b| a.href == b.href);
        self
    }

    /// A custom `<meta>` tag: `property` for an `og:` or `article:` name,
    /// `name` otherwise.
    pub fn meta(self, name: impl Into<String>, content: impl Into<String>) -> Self {
        self.meta_tag(MetaTag::new(name, content))
    }

    /// A custom `<meta>` tag under a media query.
    pub fn meta_for(
        self,
        name: impl Into<String>,
        content: impl Into<String>,
        media: Media,
    ) -> Self {
        self.meta_tag(MetaTag::new(name, content).media(media))
    }

    /// A custom `<meta>` tag with every option; its name and media are its
    /// key.
    pub fn meta_tag(mut self, meta: MetaTag) -> Self {
        upsert(&mut self.data.metas, meta, |a, b| {
            a.name == b.name && a.media == b.media
        });
        self
    }

    /// A custom `<link>` tag.
    pub fn link(self, rel: impl Into<String>, href: impl Into<String>) -> Self {
        self.link_tag(LinkTag::new(rel, href))
    }

    /// A custom `<link>` tag with attributes; its `rel` and address are its
    /// key.
    pub fn link_tag(mut self, link: LinkTag) -> Self {
        upsert(&mut self.data.links, link, |a, b| {
            a.rel == b.rel && a.href == b.href
        });
        self
    }

    /// A JSON-LD schema, written as its own
    /// `<script type="application/ld+json">` element.
    pub fn schema(mut self, schema: impl Into<Schema>) -> Self {
        self.data.schemas.push(schema.into());
        self
    }
}

/// Error page metadata, per status and for every error status. Laravel
/// Head's `ErrorPages`, which [`Head::errors`](crate::Head::errors) hands
/// you.
#[derive(Clone, Debug, Default)]
pub struct ErrorPages {
    pub(super) defaults: HeadData,
    pub(super) statuses: std::collections::BTreeMap<u16, HeadData>,
}

impl ErrorPages {
    /// Metadata for every error page, under a status's own.
    pub fn defaults(mut self, f: impl FnOnce(HeadBuilder) -> HeadBuilder) -> Self {
        let defaults = std::mem::take(&mut self.defaults);
        self.defaults = f(HeadBuilder { data: defaults }).data;
        self
    }

    /// Metadata for the error page of `status`, such as `404`.
    pub fn status(mut self, status: u16, f: impl FnOnce(HeadBuilder) -> HeadBuilder) -> Self {
        let current = self.statuses.remove(&status).unwrap_or_default();
        self.statuses
            .insert(status, f(HeadBuilder { data: current }).data);
        self
    }
}
