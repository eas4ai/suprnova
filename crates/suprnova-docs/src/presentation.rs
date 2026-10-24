//! Build layout data without choosing the application's view engine.

use crate::{Navigation, Page, Site, content, invalid, public_url, secure};
use suprnova::{CspNonce, FrameworkError, HttpResponse, Request};

/// Supplies escaped chrome and sanitized prose to the application's own layout.
///
/// Include `alternate_html` in the head and the other HTML fragments in the body.
/// Titles and identifiers are plain text; escape them before inserting into HTML.
pub struct DocsPage {
    /// Names the set so a layout can use its own brand treatment.
    pub site_title: String,
    /// Names the page for a document title or breadcrumb.
    pub title: String,
    /// Identifies the current release for application-specific chrome.
    pub version: String,
    /// Identifies the current translation for the document's lang attribute.
    pub language: String,
    /// Gives the canonical HTML address for the application's links.
    pub address: String,
    /// Holds sanitized output from the framework's Markdown pipeline.
    pub html: String,
    /// Preserves the declared navigation hierarchy while hiding refused pages.
    pub navigation_html: String,
    /// Offers admitted releases only; empty when the set declares one version.
    pub versions_html: String,
    /// Offers translations only when the set declares more than one language.
    pub languages_html: String,
    /// Provides a labelled search field, results and the nonced search script.
    pub search_html: String,
    /// Advertises the current address's Markdown twin in the layout's head.
    pub alternate_html: String,
    /// Lets the layout's own scripts share the response's nonce.
    pub nonce: String,
}

pub(super) async fn serve(
    site: &Site,
    req: &Request,
    index: usize,
    twin: bool,
    address: &str,
) -> Result<HttpResponse, FrameworkError> {
    let page = &site.pages[index];
    site.admit(req, page.private, &page.access).await?;
    let address = public_url(address);
    let link = format!("<{address}.md>; rel=\"alternate\"; type=\"text/markdown\"");
    if twin || accepts_markdown(req) {
        return Ok(secure(
            HttpResponse::markdown(&page.markdown)
                .header("X-Robots-Tag", "noindex")
                .header("Link", &link),
        ));
    }
    let visible = site
        .visible(req, &page.access.version, &page.access.language)
        .await?;
    let shared_nonce = CspNonce::current().or_else(|| req.csp_nonce());
    let nonce = shared_nonce
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    let navigation = site
        .navigation
        .get(&page.access.version)
        .ok_or_else(|| invalid("version navigation missing"))?;
    let context = DocsPage {
        site_title: site.manifest.title.clone(),
        title: page.title.clone(),
        version: page.access.version.clone(),
        language: page.access.language.clone(),
        address: public_url(&page.address),
        html: content::render_html(site, page)?,
        navigation_html: format!(
            "<nav aria-label=\"Documentation\">{}</nav>",
            navigation_html(navigation, &visible)
        ),
        versions_html: version_switch(site, req, page).await?,
        languages_html: language_switch(site, req, page).await?,
        search_html: search_html(
            &public_url(&format!(
                "{}/search.json",
                site.base(&page.access.version, &page.access.language)
            )),
            &nonce,
        ),
        alternate_html: format!(
            "<link rel=\"alternate\" type=\"text/markdown\" href=\"{}.md\">",
            escape(&address)
        ),
        nonce: nonce.clone(),
    };
    let mut response =
        HttpResponse::html((site.config.layout)(req, &context)?).header("Link", &link);
    if shared_nonce.is_none() {
        response = response.header(
            "Content-Security-Policy",
            format!(
                "default-src 'self'; script-src 'nonce-{nonce}'; object-src 'none'; base-uri 'none'"
            ),
        );
    }
    Ok(secure(response))
}

fn accepts_markdown(req: &Request) -> bool {
    req.header("accept")
        .unwrap_or_default()
        .split(',')
        .any(markdown_range)
}

fn markdown_range(range: &str) -> bool {
    let mut parts = range.trim().split(';');
    if !parts
        .next()
        .is_some_and(|media| media.trim().eq_ignore_ascii_case("text/markdown"))
    {
        return false;
    }
    for parameter in parts {
        let Some((name, value)) = parameter.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("q")
            && !value
                .trim()
                .parse::<f32>()
                .is_ok_and(|quality| quality > 0.0 && quality <= 1.0)
        {
            return false;
        }
    }
    true
}

fn navigation_html(nodes: &[Navigation], visible: &[&Page]) -> String {
    let mut html = String::new();
    for node in nodes {
        let page = node.path.as_ref().and_then(|path| {
            visible
                .iter()
                .find(|p| p.access.path.as_ref() == Some(path))
        });
        let children = navigation_html(&node.children, visible);
        if page.is_none() && children.is_empty() {
            continue;
        }
        html.push_str("<li>");
        if let Some(page) = page {
            html.push_str(&format!(
                "<a href=\"{}\">{}</a>",
                escape(&public_url(&page.address)),
                escape(&node.title)
            ));
        } else if node.path.is_none() {
            html.push_str(&escape(&node.title));
        }
        html.push_str(&children);
        html.push_str("</li>");
    }
    if html.is_empty() {
        html
    } else {
        format!("<ul>{html}</ul>")
    }
}

async fn version_switch(site: &Site, req: &Request, page: &Page) -> Result<String, FrameworkError> {
    if site.manifest.versions.len() == 1 {
        return Ok(String::new());
    }
    let mut links = String::new();
    for version in &site.manifest.versions {
        let access = crate::DocsAccess {
            version: version.id.clone(),
            language: page.access.language.clone(),
            path: None,
        };
        if !site
            .allowed(req, site.manifest.private || version.private, &access)
            .await?
        {
            continue;
        }
        let pages = site
            .visible(req, &version.id, &page.access.language)
            .await?;
        if let Some(target) = equivalent(&pages, page) {
            links.push_str(&switch_link(
                target,
                &version.id,
                version.id == page.access.version,
            ));
        }
    }
    Ok(format!("<nav aria-label=\"Version\">{links}</nav>"))
}

async fn language_switch(
    site: &Site,
    req: &Request,
    page: &Page,
) -> Result<String, FrameworkError> {
    if site.manifest.languages.len() == 1 {
        return Ok(String::new());
    }
    let version = site
        .manifest
        .versions
        .iter()
        .find(|v| v.id == page.access.version)
        .ok_or_else(|| invalid("page version missing"))?;
    let mut links = String::new();
    for language in &site.manifest.languages {
        let access = crate::DocsAccess {
            version: version.id.clone(),
            language: language.clone(),
            path: None,
        };
        if !site
            .allowed(req, site.manifest.private || version.private, &access)
            .await?
        {
            continue;
        }
        let pages = site.visible(req, &page.access.version, language).await?;
        if let Some(target) = equivalent(&pages, page) {
            links.push_str(&switch_link(
                target,
                language,
                language == &page.access.language,
            ));
        }
    }
    Ok(format!("<nav aria-label=\"Language\">{links}</nav>"))
}

fn switch_link(page: &Page, label: &str, active: bool) -> String {
    format!(
        "<a href=\"{}\"{}>{}</a> ",
        escape(&public_url(&page.address)),
        if active { " aria-current=\"true\"" } else { "" },
        escape(label)
    )
}

fn equivalent<'a>(pages: &[&'a Page], current: &Page) -> Option<&'a Page> {
    pages
        .iter()
        .copied()
        .find(|p| p.access.path == current.access.path)
        .or_else(|| pages.first().copied())
}

fn search_html(address: &str, nonce: &str) -> String {
    format!(
        "<section data-docs-search data-index=\"{}\"><label>Search documentation <input type=\"search\" maxlength=\"200\" autocomplete=\"off\"></label><p role=\"status\" aria-live=\"polite\"></p><ul></ul></section><script nonce=\"{}\">{}</script>",
        escape(address),
        escape(nonce),
        include_str!("search.js")
    )
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
