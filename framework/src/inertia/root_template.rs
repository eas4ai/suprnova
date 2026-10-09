//! The application-owned root document of an Inertia first visit.
//!
//! Laravel applications own `app.blade.php` and place `@inertiaHead` and
//! `@inertia` in it, so their own meta tags, fonts, favicon and scripts
//! reach the first-load document. Here the application owns an Askama
//! template declared with [`inertia_root`](crate::inertia_root), and the
//! framework hands it its parts as values: [`InertiaRootParts`].
//!
//! The parts write themselves into the template's output while it renders.
//! The page JSON in particular is serialized straight into that output, so
//! a template visit copies the page no more often than the framework's own
//! document does, and the root template adds no size cap of its own.

use std::fmt;
use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::Value;

use crate::error::FrameworkError;
use crate::http::HttpResponse;
use crate::view::TemplateFailure;

use super::prop::InertiaRequestExt;

/// What `#[inertia_root]` generates for a template: write it, with the
/// parts placed, into the output.
type RenderFn = fn(&InertiaRootParts<'_>, &mut dyn fmt::Write) -> Result<(), TemplateFailure>;

/// The function of the request that picks a first visit's root document.
pub(crate) type RootTemplateChooser =
    Arc<dyn Fn(&dyn InertiaRequestExt) -> InertiaRootTemplate + Send + Sync>;

/// A root document an application declares with
/// [`inertia_root`](crate::inertia_root).
///
/// The attribute implements this. Write the attribute rather than the
/// impl: the attribute is what checks the template against the parts at
/// compile time, so a template that names a value the framework does not
/// supply fails the build instead of a request.
pub trait InertiaRoot {
    /// The declared type's name, which the error a failed render returns
    /// names.
    const NAME: &'static str;

    /// Writes the template into `output` with `parts` placed.
    fn render(
        parts: &InertiaRootParts<'_>,
        output: &mut dyn fmt::Write,
    ) -> Result<(), TemplateFailure>;
}

/// The document a first visit renders into: an application's root
/// template, or the framework's own document.
///
/// [`InertiaConfig::root_template`](crate::InertiaConfig::root_template)
/// takes one; build it with [`of`](Self::of).
#[derive(Clone, Copy)]
pub struct InertiaRootTemplate {
    application: Option<ApplicationTemplate>,
}

/// An application's template: its name, for errors, and its renderer.
#[derive(Clone, Copy)]
pub(crate) struct ApplicationTemplate {
    name: &'static str,
    render: RenderFn,
}

impl InertiaRootTemplate {
    /// The root template `T` declares with
    /// [`inertia_root`](crate::inertia_root).
    ///
    /// ```rust,ignore
    /// #[suprnova::inertia_root(path = "app.html")]
    /// pub struct AppDocument;
    ///
    /// let config = InertiaConfig::new()
    ///     .root_template(InertiaRootTemplate::of::<AppDocument>());
    /// ```
    pub fn of<T: InertiaRoot>() -> Self {
        Self {
            application: Some(ApplicationTemplate {
                name: T::NAME,
                render: T::render,
            }),
        }
    }

    /// The document the framework writes itself, the first visit's
    /// document when no root template is configured.
    pub const fn framework() -> Self {
        Self { application: None }
    }

    /// The application's template, or `None` for the framework's document.
    pub(crate) fn application(&self) -> Option<ApplicationTemplate> {
        self.application
    }
}

impl fmt::Debug for InertiaRootTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.application {
            Some(template) => write!(f, "InertiaRootTemplate({})", template.name),
            None => f.write_str("InertiaRootTemplate(framework)"),
        }
    }
}

/// The values the framework hands a root template, each placed with
/// `{{ name }}`.
///
/// `title`, `head` and `body` are markup and write themselves unescaped;
/// `lang`, `csrf_token`, `nonce` and the `view` data are plain values the
/// template escapes like any other. The framework builds this; a template
/// only reads it.
#[non_exhaustive]
pub struct InertiaRootParts<'a> {
    /// The `<title>` element, from the response's title or
    /// [`InertiaConfig::default_title`](crate::InertiaConfig::default_title).
    /// Empty when the SSR head or the document head [`Head`](crate::Head)
    /// resolved carries a title of its own, since a document shows its
    /// first title only. A template that writes its own `<title>` leaves
    /// this out.
    pub title: InertiaRootTitle<'a>,
    /// The `csrf-token` meta tag, the document head [`Head`](crate::Head)
    /// resolved, the SSR head when the SSR server rendered the page, and
    /// the Vite tags, in that order.
    pub head: InertiaRootHead<'a>,
    /// The page data element and the mount element, or the SSR body.
    pub body: InertiaRootBody<'a>,
    /// The document's language: the locale in effect for the request.
    pub lang: &'a str,
    /// The session's CSRF token, empty outside a session.
    pub csrf_token: &'a str,
    /// The request's CSP nonce. `None`: no nonce policy supplies one
    /// (RDOC-005).
    pub nonce: Option<&'a str>,
    /// Whether the SSR server rendered this response, for a template that
    /// places fallback head content when it did not.
    pub ssr: bool,
    /// The response's view data, which reaches this template and never the
    /// page: `{% if let Some(v) = view.get("key") %}`.
    pub view: &'a InertiaViewData,
}

/// Values one response hands its root template and never its page props,
/// set with [`InertiaResponse::with_view_data`](crate::InertiaResponse::with_view_data).
///
/// For what the first-load HTML must carry without running JavaScript,
/// such as the meta tags a link preview reads. An Inertia visit's JSON and
/// the page data never include it. A value is any JSON value, as Laravel's
/// `withViewData` takes any PHP value.
#[derive(Clone, Debug, Default)]
pub struct InertiaViewData {
    values: IndexMap<String, Value>,
}

impl InertiaViewData {
    /// The value set under `key`, or `None`.
    pub fn get(&self, key: &str) -> Option<InertiaViewValue<'_>> {
        self.values.get(key).map(InertiaViewValue)
    }

    /// Sets `key` to `value`, replacing an earlier value.
    pub(crate) fn insert(&mut self, key: String, value: Value) {
        self.values.insert(key, value);
    }
}

/// One view data value as a template places it with `{{ value }}`: a
/// string as itself, any other value as its JSON, escaped by the template
/// like any other value.
///
/// A string displays without its JSON quotes because a meta tag's content
/// is the text, not a JSON literal of it.
#[derive(Clone, Copy, Debug)]
pub struct InertiaViewValue<'a>(&'a Value);

impl fmt::Display for InertiaViewValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Value::String(text) => f.write_str(text),
            other => fmt::Display::fmt(other, f),
        }
    }
}

/// What [`render`] needs from the response to build the parts.
pub(super) struct RootInputs<'a> {
    pub(super) page: &'a Value,
    pub(super) title: Option<&'a str>,
    pub(super) csrf_token: &'a str,
    pub(super) head_tags: &'a str,
    pub(super) ssr_head: &'a str,
    pub(super) ssr_body: Option<&'a str>,
    pub(super) assets: &'a str,
    pub(super) lang: &'a str,
    pub(super) mount_id: &'a str,
    pub(super) view: &'a InertiaViewData,
}

/// Renders the first visit through `template`.
///
/// The template writes into one buffer the response then owns. A template
/// that fails returns an error and the buffer is dropped, so no part of
/// the document is ever sent.
pub(super) fn render(
    template: ApplicationTemplate,
    inputs: RootInputs<'_>,
) -> Result<HttpResponse, FrameworkError> {
    let parts = InertiaRootParts {
        title: InertiaRootTitle {
            title: inputs.title,
        },
        head: InertiaRootHead {
            csrf_token: inputs.csrf_token,
            head_tags: inputs.head_tags,
            ssr_head: inputs.ssr_head,
            assets: inputs.assets,
        },
        body: match inputs.ssr_body {
            Some(ssr_body) => InertiaRootBody {
                content: BodyContent::Ssr(ssr_body),
            },
            None => InertiaRootBody {
                content: BodyContent::Page {
                    page: inputs.page,
                    mount_id: inputs.mount_id,
                },
            },
        },
        lang: inputs.lang,
        csrf_token: inputs.csrf_token,
        nonce: None,
        ssr: inputs.ssr_body.is_some(),
        view: inputs.view,
    };
    let mut html = String::new();
    (template.render)(&parts, &mut html).map_err(|failure| {
        FrameworkError::internal(format!(
            "the Inertia root template `{}` failed to render: {failure:?}",
            template.name
        ))
    })?;
    Ok(HttpResponse::html(html).header("Vary", "X-Inertia"))
}

/// The `<title>` part: the title element, or nothing when the SSR head
/// carries one.
pub struct InertiaRootTitle<'a> {
    title: Option<&'a str>,
}

impl fmt::Display for InertiaRootTitle<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(title) = self.title {
            f.write_str("<title>")?;
            write_escaped(f, title, false)?;
            f.write_str("</title>")?;
        }
        Ok(())
    }
}

/// The head part: the `csrf-token` meta tag, the document head `Head`
/// resolved, the SSR head and the Vite tags.
pub struct InertiaRootHead<'a> {
    csrf_token: &'a str,
    head_tags: &'a str,
    ssr_head: &'a str,
    assets: &'a str,
}

impl fmt::Display for InertiaRootHead<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<meta name=\"csrf-token\" content=\"")?;
        write_escaped(f, self.csrf_token, true)?;
        f.write_str("\">\n")?;
        f.write_str(self.head_tags)?;
        if !self.ssr_head.is_empty() {
            f.write_str(self.ssr_head)?;
            f.write_str("\n")?;
        }
        f.write_str(self.assets)
    }
}

/// The body part: the page data element and the mount element, or the
/// SSR body, which carries both.
pub struct InertiaRootBody<'a> {
    content: BodyContent<'a>,
}

enum BodyContent<'a> {
    Page { page: &'a Value, mount_id: &'a str },
    Ssr(&'a str),
}

impl fmt::Display for InertiaRootBody<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.content {
            BodyContent::Ssr(body) => f.write_str(body),
            BodyContent::Page { page, mount_id } => {
                f.write_str("<script type=\"application/json\" data-page=\"")?;
                write_escaped(f, mount_id, true)?;
                f.write_str("\">")?;
                serde_json::to_writer(EscapedPageJson(f), page).map_err(|_| fmt::Error)?;
                f.write_str("</script>\n<div id=\"")?;
                write_escaped(f, mount_id, true)?;
                f.write_str("\"></div>")
            }
        }
    }
}

// The three markup parts write HTML on purpose: placed bare, Askama writes
// them as they are rather than escaping them as text.
impl askama::filters::HtmlSafe for InertiaRootTitle<'_> {}
impl askama::filters::HtmlSafe for InertiaRootHead<'_> {}
impl askama::filters::HtmlSafe for InertiaRootBody<'_> {}

/// Writes page JSON into a template's output with every `/` written as
/// `\/` and `<` and `>` as `\u003c` and `\u003e`, the escaping the
/// framework's own document applies: `/` keeps a `</script>` inside a
/// string from closing the page data element, and `<` and `>` keep a
/// `<!--` or a `<script` from putting the HTML tokenizer into the escaped
/// script states, where the real end tag no longer closes it (Laravel's
/// `JSON_HEX_TAG`).
///
/// `serde_json` writes whole UTF-8 fragments (a string's text between the
/// characters it escapes, or ASCII), and the three characters are ASCII,
/// so the split keeps every fragment valid text. A write that was not
/// would fail the render rather than write something else.
struct EscapedPageJson<'f, 'b>(&'f mut fmt::Formatter<'b>);

impl std::io::Write for EscapedPageJson<'_, '_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let text = std::str::from_utf8(bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        let mut rest = text;
        while let Some(at) = rest.find(['/', '<', '>']) {
            let escape = match rest.as_bytes()[at] {
                b'<' => "\\u003c",
                b'>' => "\\u003e",
                _ => "\\/",
            };
            self.0
                .write_str(&rest[..at])
                .map_err(std::io::Error::other)?;
            self.0.write_str(escape).map_err(std::io::Error::other)?;
            rest = &rest[at + 1..];
        }
        self.0.write_str(rest).map_err(std::io::Error::other)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Writes `text` HTML-escaped: `&`, `<` and `>` always, and the quotes
/// too when it goes into an attribute value. The same entities the
/// framework's own document writes.
fn write_escaped(f: &mut fmt::Formatter<'_>, text: &str, attribute: bool) -> fmt::Result {
    let mut rest = text;
    while let Some(at) =
        rest.find(|c: char| matches!(c, '&' | '<' | '>') || (attribute && matches!(c, '"' | '\'')))
    {
        f.write_str(&rest[..at])?;
        f.write_str(match rest.as_bytes()[at] {
            b'&' => "&amp;",
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'"' => "&quot;",
            _ => "&#x27;",
        })?;
        rest = &rest[at + 1..];
    }
    f.write_str(rest)
}
