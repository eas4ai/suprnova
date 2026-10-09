//! The document head: title, meta tags, Open Graph, X cards, icons,
//! resource hints and JSON-LD schemas, Laravel Head's API.
//!
//! A page's head resolves from five layers, lowest first: the defaults
//! [`Head::defaults`] registers, a route group's metadata and a route's
//! (`with_head` on the route builders), what the request sets at run time
//! through the [`Head`] calls, and the metadata [`Head::errors`] registers
//! for an error status. A higher layer replaces a lower one field by field,
//! so a run-time title keeps the route's description.
//!
//! The resolved head reaches three places: [`Head::render_html`] for a
//! server-rendered view, [`Head::to_array`] as data, and every Inertia
//! response, which shares it as a `head` prop of rendered tags (with
//! Inertia's `serverHead` option the client keeps the document head in step
//! across visits) and writes it into the first visit's `<head>`.
//!
//! The request's layers live in the request's container scope, a
//! task-local, so concurrent requests never see each other's head. The
//! defaults, error metadata and Inertia settings belong to the application
//! container, so `TestContainer::fake()` isolates them.

mod builder;
mod render;
mod schema;

use std::sync::{Arc, Mutex, PoisonError, RwLock};

use serde_json::Value;

pub use builder::{
    ErrorPages, Feed, HeadBuilder, Hint, Icon, ImageType, LinkTag, Media, MetaTag, OgMedia, OgType,
    OpenGraph, Pwa, RobotsRule, TwitterCard, TwitterCardType,
};
pub use schema::Schema;

use builder::HeadData;
use render::Tag;

use super::share::{application_value, request_value, scoped_request_value};
use crate::{FrameworkError, Middleware, Next, Request, Response};

/// The prop an Inertia response carries the head under, unless
/// [`Head::inertia`] names another.
const DEFAULT_PROP: &str = "head";

/// The application's head settings: one per application container.
#[derive(Clone, Default)]
struct HeadConfig(Arc<RwLock<ConfigInner>>);

#[derive(Default)]
struct ConfigInner {
    defaults: HeadData,
    errors: ErrorPages,
    globals: HeadData,
    prop: Option<String>,
}

/// One request's head: one per container scope.
#[derive(Default)]
struct RequestHead(Mutex<RequestInner>);

#[derive(Default)]
struct RequestInner {
    /// Each route group's metadata, outermost first.
    groups: Vec<Arc<HeadData>>,
    /// The route's metadata.
    routes: Vec<Arc<HeadData>>,
    /// What the request set at run time.
    runtime: HeadData,
    /// The error status the response is rendered for.
    status: Option<u16>,
    /// The request's absolute URL, for a canonical link without one.
    url: Option<String>,
}

fn update_config(f: impl FnOnce(&mut ConfigInner)) {
    let config = application_value::<HeadConfig>(true).unwrap_or_default();
    f(&mut config.0.write().unwrap_or_else(PoisonError::into_inner));
}

/// Laravel's `Head` facade: the page's document head.
///
/// ```rust,no_run
/// use suprnova::Head;
///
/// // At boot: every page's defaults.
/// Head::defaults(|head| head
///     .title("Laravel")
///     .title_suffix(" - Laravel")
///     .description("Build something great."));
///
/// // In a handler: this page's title, rendered `About - Laravel`.
/// Head::title("About").description("Who we are.");
/// ```
///
/// The run-time calls (`Head::title`, `Head::description` and every other
/// [`HeadBuilder`] call) set the current request's metadata and return a
/// [`RuntimeHead`] to chain more. Outside a request they do nothing but log
/// a warning; [`Head::try_update`] returns the error instead.
pub struct Head;

impl Head {
    /// Register the defaults every page starts from, the lowest layer. A
    /// later call adds to the defaults already registered.
    pub fn defaults(f: impl FnOnce(HeadBuilder) -> HeadBuilder) {
        update_config(|config| {
            let defaults = std::mem::take(&mut config.defaults);
            config.defaults = f(HeadBuilder { data: defaults }).data;
        });
    }

    /// Register the metadata of error pages, the highest layer: it wins
    /// over every other layer when a response is rendered for an error
    /// status. The Inertia error page sets the status itself; call
    /// [`Head::status`] when you render an error page another way.
    ///
    /// ```rust,no_run
    /// use suprnova::Head;
    ///
    /// Head::errors(|errors| errors
    ///     .defaults(|head| head.robots("noindex, follow"))
    ///     .status(404, |head| head.title("Page Not Found")));
    /// ```
    pub fn errors(f: impl FnOnce(ErrorPages) -> ErrorPages) {
        update_config(|config| {
            let errors = std::mem::take(&mut config.errors);
            config.errors = f(errors);
        });
    }

    /// Name the prop an Inertia response carries the head under, in place
    /// of `head`, for an application whose pages already use a `head`
    /// prop. Point the client at it with `serverHead: '<prop>'`.
    pub fn inertia(prop: impl Into<String>) {
        update_config(|config| config.prop = Some(prop.into()));
    }

    /// Register tags written into the first Inertia visit's document only,
    /// without a `data-inertia` key and never into the `head` prop, so the
    /// client leaves them alone for the rest of the session: stable browser
    /// hints such as the viewport, the color scheme, icons and the
    /// manifest. Anything page-specific belongs in the other layers.
    pub fn inertia_globals(f: impl FnOnce(HeadBuilder) -> HeadBuilder) {
        update_config(|config| {
            let globals = std::mem::take(&mut config.globals);
            config.globals = f(HeadBuilder { data: globals }).data;
        });
    }

    /// Apply `f` to the current request's run-time metadata. Outside a
    /// request this logs a warning; [`Self::try_update`] returns the error.
    pub fn update(f: impl FnOnce(HeadBuilder) -> HeadBuilder) -> RuntimeHead {
        RuntimeHead::apply(f)
    }

    /// Apply `f` to the current request's run-time metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when no request is running: no container scope is
    /// active.
    pub fn try_update(f: impl FnOnce(HeadBuilder) -> HeadBuilder) -> Result<(), FrameworkError> {
        let state = request_value::<RequestHead>()?;
        let mut inner = state.0.lock().unwrap_or_else(PoisonError::into_inner);
        let runtime = std::mem::take(&mut inner.runtime);
        inner.runtime = f(HeadBuilder { data: runtime }).data;
        Ok(())
    }

    /// Render the current request's head for the error `status`, so the
    /// metadata [`Self::errors`] registered for it applies: for an error
    /// page you render yourself. The Inertia error page sets it already.
    pub fn status(status: u16) {
        match request_value::<RequestHead>() {
            Ok(state) => {
                state
                    .0
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .status = Some(status);
            }
            Err(error) => tracing::warn!(%error, "Head::status called outside a request"),
        }
    }

    /// The current request's resolved head as tags, each with its
    /// `data-inertia` key and every value HTML-escaped, for the `<head>` of
    /// a server-rendered view. Outside a request, the defaults alone.
    pub fn render_html() -> String {
        Resolved::current(None, None)
            .tags()
            .iter()
            .map(|tag| tag.html(true))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The current request's resolved head as data: one object per tag,
    /// with its `key`, `element`, `attributes` and `content` (a schema's
    /// content is its JSON-LD object).
    pub fn to_array() -> Value {
        render::to_array(&Resolved::current(None, None).tags())
    }

    /// Set the current request's `prev` and `next` links from a paginator.
    pub fn paginate<T>(paginator: &dyn crate::pagination::Paginated<T>) -> RuntimeHead {
        RuntimeHead::apply(|head| head.paginate(paginator))
    }

    /// Set the current request's `alternate` links, one per locale.
    pub fn alternates<L, H>(alternates: impl IntoIterator<Item = (L, H)>) -> RuntimeHead
    where
        L: Into<String>,
        H: Into<String>,
    {
        RuntimeHead::apply(|head| head.alternates(alternates))
    }
}

/// The current request's run-time head, returned by every run-time
/// [`Head`] call so more calls chain: `Head::title("About").description("...")`.
#[derive(Debug)]
pub struct RuntimeHead {
    _private: (),
}

impl RuntimeHead {
    fn apply(f: impl FnOnce(HeadBuilder) -> HeadBuilder) -> Self {
        if let Err(error) = Head::try_update(f) {
            tracing::warn!(%error, "a Head call outside a request was dropped");
        }
        Self { _private: () }
    }

    /// Set the `prev` and `next` links from a paginator.
    pub fn paginate<T>(self, paginator: &dyn crate::pagination::Paginated<T>) -> Self {
        Self::apply(|head| head.paginate(paginator))
    }

    /// Set the `alternate` links, one per locale.
    pub fn alternates<L, H>(self, alternates: impl IntoIterator<Item = (L, H)>) -> Self
    where
        L: Into<String>,
        H: Into<String>,
    {
        Self::apply(|head| head.alternates(alternates))
    }
}

/// The run-time calls, on [`Head`] to start and on [`RuntimeHead`] to
/// chain, each the [`HeadBuilder`] call of the same name applied to the
/// current request's run-time metadata.
macro_rules! runtime_calls {
    ($( $(#[$doc:meta])* fn $name:ident($($arg:ident: $ty:ty),*); )*) => {
        impl Head {
            $(
                $(#[$doc])*
                ///
                /// Sets the current request's run-time metadata; see
                /// [`HeadBuilder`] for the call.
                pub fn $name($($arg: $ty),*) -> RuntimeHead {
                    RuntimeHead::apply(move |head| head.$name($($arg),*))
                }
            )*
        }

        impl RuntimeHead {
            $(
                $(#[$doc])*
                pub fn $name(self, $($arg: $ty),*) -> Self {
                    Self::apply(move |head| head.$name($($arg),*))
                }
            )*
        }
    };
}

runtime_calls! {
    /// The page title, which takes the prefix and suffix of the lower layers.
    fn title(title: impl Into<String>);
    /// A title that takes no inherited prefix or suffix.
    fn exact_title(title: impl Into<String>);
    /// The text written before a higher layer's title.
    fn title_prefix(prefix: impl Into<String>);
    /// The text written after a higher layer's title.
    fn title_suffix(suffix: impl Into<String>);
    /// The `description` meta tag.
    fn description(description: impl Into<String>);
    /// A canonical link to the request's URL, over `https`.
    fn canonical();
    /// A canonical link to `url`, over `https`.
    fn canonical_url(url: impl Into<String>);
    /// Keep the canonical URL's own scheme instead of `https`.
    fn canonical_keep_scheme();
    /// The `robots` directives.
    fn robots(directives: impl Into<String>);
    /// The `robots` directives from a list.
    fn robots_rules(rules: impl IntoIterator<Item = RobotsRule>);
    /// `robots` `all`.
    fn searchable_by_robots();
    /// `robots` `none`.
    fn hidden_from_robots();
    /// Apply `then` when `condition` holds.
    fn when(condition: bool, then: impl FnOnce(HeadBuilder) -> HeadBuilder);
    /// Apply `then` unless `condition` holds.
    fn unless(condition: bool, then: impl FnOnce(HeadBuilder) -> HeadBuilder);
    /// The Open Graph values.
    fn og(og: OpenGraph);
    /// An Open Graph image.
    fn og_image(image: impl Into<OgMedia>);
    /// An Open Graph video.
    fn og_video(video: impl Into<OgMedia>);
    /// An Open Graph audio file.
    fn og_audio(audio: impl Into<OgMedia>);
    /// The X (Twitter) card values.
    fn twitter(twitter: TwitterCard);
    /// The X card image.
    fn twitter_image(image: impl Into<OgMedia>);
    /// A `theme-color`, for every media or under one.
    fn theme_color(color: impl Into<String>, media: impl Into<Option<Media>>);
    /// `application-name`.
    fn application_name(name: impl Into<String>);
    /// `color-scheme`.
    fn color_scheme(scheme: impl Into<String>);
    /// `referrer`.
    fn referrer(policy: impl Into<String>);
    /// `viewport`.
    fn viewport(viewport: impl Into<String>);
    /// `apple-mobile-web-app-title`.
    fn apple_web_app_title(title: impl Into<String>);
    /// The web app capable tags.
    fn web_app_capable();
    /// `apple-mobile-web-app-status-bar-style`.
    fn apple_web_app_status_bar_style(style: impl Into<String>);
    /// An `icon` link.
    fn icon(icon: impl Into<Icon>);
    /// An `icon` link.
    fn favicon(icon: impl Into<Icon>);
    /// An `apple-touch-icon` link.
    fn apple_touch_icon(icon: impl Into<Icon>);
    /// An `apple-touch-startup-image` link.
    fn apple_touch_startup_image(image: impl Into<Icon>);
    /// The `mask-icon` link and its color.
    fn mask_icon(href: impl Into<String>, color: impl Into<String>);
    /// The web app `manifest` link.
    fn manifest(href: impl Into<String>);
    /// The tags an installable web app needs.
    fn pwa(pwa: Pwa);
    /// A `preload` hint.
    fn preload(hint: impl Into<Hint>);
    /// A `prefetch` hint.
    fn prefetch(hint: impl Into<Hint>);
    /// A `preconnect` hint.
    fn preconnect(hint: impl Into<Hint>);
    /// A `dns-prefetch` hint.
    fn dns_prefetch(hint: impl Into<Hint>);
    /// The `prev` link.
    fn prev_page(href: impl Into<String>);
    /// The `next` link.
    fn next_page(href: impl Into<String>);
    /// A feed discovery link.
    fn feed(feed: Feed);
    /// A custom `<meta>` tag.
    fn meta(name: impl Into<String>, content: impl Into<String>);
    /// A custom `<meta>` tag under a media query.
    fn meta_for(name: impl Into<String>, content: impl Into<String>, media: Media);
    /// A custom `<meta>` tag with every option.
    fn meta_tag(meta: MetaTag);
    /// A custom `<link>` tag.
    fn link(rel: impl Into<String>, href: impl Into<String>);
    /// A custom `<link>` tag with attributes.
    fn link_tag(link: LinkTag);
    /// A JSON-LD schema.
    fn schema(schema: impl Into<Schema>);
}

/// The head one response resolves.
struct Resolved {
    data: HeadData,
    /// Whether any layer set anything.
    set: bool,
    request_url: Option<String>,
    globals: HeadData,
    prop: String,
}

impl Resolved {
    /// The current request's head. `status` is the error status the
    /// response is rendered for, ahead of [`Head::status`]'s; `url` gives
    /// the request's URL, ahead of the one a route layer recorded, and is
    /// called only when a canonical link needs it.
    fn current(status: Option<u16>, url: Option<&dyn Fn() -> String>) -> Self {
        let config = application_value::<HeadConfig>(false);
        let config = config
            .as_ref()
            .map(|config| config.0.read().unwrap_or_else(PoisonError::into_inner));
        let request = scoped_request_value::<RequestHead>();
        let request = request
            .as_ref()
            .map(|state| state.0.lock().unwrap_or_else(PoisonError::into_inner));

        let mut layers: Vec<&HeadData> = Vec::new();
        if let Some(config) = &config {
            layers.push(&config.defaults);
        }
        if let Some(request) = &request {
            layers.extend(request.groups.iter().map(Arc::as_ref));
            layers.extend(request.routes.iter().map(Arc::as_ref));
            layers.push(&request.runtime);
        }
        let status = status.or_else(|| request.as_ref().and_then(|request| request.status));
        if let (Some(status), Some(config)) = (status.filter(|status| *status >= 400), &config) {
            layers.push(&config.errors.defaults);
            if let Some(layer) = config.errors.statuses.get(&status) {
                layers.push(layer);
            }
        }

        let mut data = HeadData::default();
        let mut set = false;
        for layer in layers {
            if !layer.is_empty() {
                set = true;
                data.overlay(layer);
            }
        }
        let globals = config
            .as_ref()
            .map(|config| config.globals.clone())
            .unwrap_or_default();
        let needs_url = [&data, &globals].iter().any(|layer| {
            layer
                .canonical
                .as_ref()
                .is_some_and(|canonical| canonical.url.is_none())
        });
        let request_url = needs_url
            .then(|| {
                url.map(|url| url())
                    .or_else(|| request.as_ref().and_then(|request| request.url.clone()))
            })
            .flatten();
        if needs_url && request_url.is_none() {
            tracing::warn!(
                "Head::canonical() names the request's URL, which is not known outside an \
                 Inertia render or a route that declares with_head; give the URL with \
                 canonical_url"
            );
        }
        Self {
            data,
            set,
            request_url,
            globals,
            prop: config
                .as_ref()
                .and_then(|config| config.prop.clone())
                .unwrap_or_else(|| DEFAULT_PROP.to_string()),
        }
    }

    fn tags(&self) -> Vec<Tag> {
        render::tags(&self.data, self.request_url.as_deref())
    }
}

/// The head an Inertia response carries: the page-managed tags for the
/// `head` prop and the first visit, and the globals for the first visit
/// only.
pub(crate) struct InertiaHead {
    prop: String,
    tags: Vec<Tag>,
    globals: Vec<Tag>,
}

impl InertiaHead {
    /// The head of the Inertia response being built, rendered for the
    /// error `status` when there is one. `url` gives the request's absolute
    /// URL when a canonical link needs it. `None` when no layer sets
    /// anything and no globals are registered, so the page object of an
    /// application that does not use `Head` stays as it is.
    pub(crate) fn for_response(url: &dyn Fn() -> String, status: Option<u16>) -> Option<Self> {
        let resolved = Resolved::current(status, Some(url));
        if !resolved.set && resolved.globals.is_empty() {
            return None;
        }
        Some(Self {
            tags: if resolved.set {
                resolved.tags()
            } else {
                Vec::new()
            },
            globals: render::tags(&resolved.globals, resolved.request_url.as_deref()),
            prop: resolved.prop,
        })
    }

    /// The prop the head goes under.
    pub(crate) fn prop(&self) -> &str {
        &self.prop
    }

    /// The prop's value: each page-managed tag rendered with its
    /// `data-inertia` key. `None` when there are none.
    pub(crate) fn prop_value(&self) -> Option<Value> {
        (!self.tags.is_empty()).then(|| {
            Value::Array(
                self.tags
                    .iter()
                    .map(|tag| Value::String(tag.html(true)))
                    .collect(),
            )
        })
    }

    /// Whether the head sets the title, so the document writes no default.
    pub(crate) fn has_title(&self) -> bool {
        self.tags.iter().chain(&self.globals).any(Tag::is_title)
    }

    /// Whether the head sets the viewport, so the framework's own document
    /// writes none of its own.
    pub(crate) fn has_viewport(&self) -> bool {
        self.tags.iter().chain(&self.globals).any(Tag::is_viewport)
    }

    /// The first visit's head tags: the globals without a key, then each
    /// page-managed tag with its key, leaving out a tag whose key the SSR
    /// head already carries, and the title when the SSR head has one, so
    /// the document holds each element once.
    pub(crate) fn first_visit_html(&self, ssr_head: &str) -> String {
        let ssr_keys = inertia_keys(ssr_head);
        let ssr_title = crate::inertia::contains_title_element(ssr_head);
        let mut html = String::new();
        for tag in &self.globals {
            if ssr_title && tag.is_title() {
                continue;
            }
            html.push_str(&tag.html(false));
            html.push('\n');
        }
        for tag in &self.tags {
            if ssr_keys.contains(&tag.key) || (ssr_title && tag.is_title()) {
                continue;
            }
            html.push_str(&tag.html(true));
            html.push('\n');
        }
        html
    }
}

/// The `data-inertia` keys the elements of `head` carry, as written.
fn inertia_keys(head: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut rest = head;
    while let Some(at) = rest.find("data-inertia=") {
        rest = &rest[at + "data-inertia=".len()..];
        let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            continue;
        };
        rest = &rest[1..];
        if let Some(end) = rest.find(quote) {
            keys.push(unescape_attribute(&rest[..end]));
            rest = &rest[end + 1..];
        }
    }
    keys
}

/// Undo the attribute escaping the head writes, so a key read back from
/// the SSR head compares with the key it was written from.
fn unescape_attribute(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Which layer a [`HeadLayer`] fills.
#[derive(Clone, Copy)]
enum LayerKind {
    Group,
    Route,
}

/// The route middleware `with_head` adds: it records a route group's or a
/// route's metadata, and the request's URL, in the request's head before
/// the handler runs.
pub(crate) struct HeadLayer {
    kind: LayerKind,
    data: Arc<HeadData>,
}

impl HeadLayer {
    /// The layer of a route group.
    pub(crate) fn group(f: impl FnOnce(HeadBuilder) -> HeadBuilder) -> Self {
        Self {
            kind: LayerKind::Group,
            data: Arc::new(f(HeadBuilder::new()).data),
        }
    }

    /// The layer of a route.
    pub(crate) fn route(f: impl FnOnce(HeadBuilder) -> HeadBuilder) -> Self {
        Self {
            kind: LayerKind::Route,
            data: Arc::new(f(HeadBuilder::new()).data),
        }
    }
}

#[async_trait::async_trait]
impl Middleware for HeadLayer {
    async fn handle(&self, request: Request, next: Next) -> Response {
        match scoped_request_value::<RequestHead>() {
            Some(state) => {
                let mut inner = state.0.lock().unwrap_or_else(PoisonError::into_inner);
                match self.kind {
                    LayerKind::Group => inner.groups.push(Arc::clone(&self.data)),
                    LayerKind::Route => inner.routes.push(Arc::clone(&self.data)),
                }
                if inner.url.is_none() {
                    inner.url = Some(crate::routing::url::current(&request));
                }
            }
            None => tracing::warn!("route head metadata outside a container scope was dropped"),
        }
        next(request).await
    }
}
