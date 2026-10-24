//! Route real tool handlers through the same request-local set and gate.

use crate::{Site, invalid, public_url, secure};
use std::sync::Arc;
use suprnova::{
    FrameworkError, HttpResponse, Response, Router, handler, tool, tool_input, tool_output,
};

struct ToolContext {
    site: Arc<Site>,
    pages: Vec<usize>,
}
tokio::task_local! { static CONTEXT: ToolContext; }

/// Gives all three read tools scalar, validated arguments for GET and MCP callers.
#[tool_input]
struct DocsQuery {
    /// Selects a release; omission uses the manifest's default version.
    #[serde(default)]
    #[validate(length(max = 128))]
    version: Option<String>,
    /// Selects a translation; omission uses the first language.
    #[serde(default)]
    #[validate(length(max = 128))]
    language: Option<String>,
    /// Selects a navigation path for read_page, without a Markdown suffix.
    #[serde(default)]
    #[validate(length(max = 512))]
    path: Option<String>,
    /// Searches titles, headings and text, with all words required.
    #[serde(default)]
    #[validate(length(max = 200))]
    query: Option<String>,
}

/// Makes the original Markdown and its addresses available as structured MCP data.
#[tool_output]
struct DocsResult {
    /// Preserves Markdown source instead of rendering it for the agent.
    markdown: String,
    /// Identifies each selected page without asking an agent to parse Markdown links.
    pages: Vec<DocsAddress>,
}

/// Keeps both representations discoverable in a tool result.
#[tool_output]
struct DocsAddress {
    /// Names the page for an agent's references.
    title: String,
    /// Points to the canonical HTML address in its version and language.
    address: String,
    /// Points to the source for a client that wants to fetch it separately.
    markdown_address: String,
}

pub(super) fn mount(router: Router, site: Arc<Site>) -> Result<Router, FrameworkError> {
    let prefix = site.config.prefix.clone();
    let router = attach(
        router.try_get(&format!("{prefix}/_tools/search_docs"), search_docs)?,
        &site,
    );
    let router = attach(
        router.try_get(&format!("{prefix}/_tools/read_page"), read_page)?,
        &site,
    );
    Ok(attach(
        router.try_get(&format!("{prefix}/_tools/list_pages"), list_pages)?,
        &site,
    ))
}

fn attach(builder: suprnova::routing::RouteBuilder, site: &Arc<Site>) -> Router {
    let shared = Arc::clone(site);
    builder
        .middleware_boxed(Arc::new(move |req, next| {
            let site = Arc::clone(&shared);
            Box::pin(async move {
                let input: DocsQuery = req
                    .query_into()
                    .map_err(|e| secure(HttpResponse::from(e)))?;
                let version = input
                    .version
                    .as_deref()
                    .unwrap_or(&site.manifest.default_version);
                let language = input
                    .language
                    .as_deref()
                    .unwrap_or(&site.manifest.languages[0]);
                let visible = site
                    .visible(&req, version, language)
                    .await
                    .map_err(|e| secure(HttpResponse::from(e)))?;
                let addresses: std::collections::HashSet<_> =
                    visible.iter().map(|page| page.address.as_str()).collect();
                let pages = site
                    .pages
                    .iter()
                    .enumerate()
                    .filter(|(_, page)| addresses.contains(page.address.as_str()))
                    .map(|(index, _)| index)
                    .collect();
                CONTEXT.scope(ToolContext { site, pages }, next(req)).await
            })
        }))
        .into()
}

#[tool(name = "search_docs", description = "Search documentation titles, headings and text. Returns Markdown and page addresses. Optional version and language select the set slice.", callers = [public], input = DocsQuery, output = DocsResult, annotations(read_only, untrusted_content))]
#[handler]
async fn search_docs(input: DocsQuery) -> Response {
    answer(&input, Mode::Search).map_err(|e| secure(HttpResponse::from(e)))
}

#[tool(name = "read_page", description = "Read one documentation page as Markdown, with its address. Supply path without .md; version and language are optional.", callers = [public], input = DocsQuery, output = DocsResult, annotations(read_only, untrusted_content))]
#[handler]
async fn read_page(input: DocsQuery) -> Response {
    answer(&input, Mode::Read).map_err(|e| secure(HttpResponse::from(e)))
}

#[tool(name = "list_pages", description = "List admitted documentation pages in navigation order as Markdown links. Optional version and language select the set slice.", callers = [public], input = DocsQuery, output = DocsResult, annotations(read_only, untrusted_content))]
#[handler]
async fn list_pages(input: DocsQuery) -> Response {
    answer(&input, Mode::List).map_err(|e| secure(HttpResponse::from(e)))
}

#[derive(Clone, Copy)]
enum Mode {
    Search,
    Read,
    List,
}

fn answer(input: &DocsQuery, mode: Mode) -> Result<HttpResponse, FrameworkError> {
    CONTEXT
        .try_with(|context| {
            let pages: Vec<_> = context
                .pages
                .iter()
                .map(|i| &context.site.pages[*i])
                .collect();
            let selected: Vec<_> = match mode {
                Mode::Read => {
                    let path = input
                        .path
                        .as_deref()
                        .ok_or_else(|| FrameworkError::bad_request("read_page needs path"))?;
                    vec![
                        *pages
                            .iter()
                            .find(|p| p.access.path.as_deref() == Some(path))
                            .ok_or_else(|| FrameworkError::not_found("documentation not found"))?,
                    ]
                }
                Mode::List => pages,
                Mode::Search => {
                    let query = input.query.as_deref().unwrap_or_default().to_lowercase();
                    let words: Vec<_> = query.split_whitespace().collect();
                    pages
                        .into_iter()
                        .filter(|p| {
                            let text =
                                format!("{} {}", p.title, p.rendered.plain_text).to_lowercase();
                            !words.is_empty() && words.iter().all(|word| text.contains(word))
                        })
                        .take(20)
                        .collect()
                }
            };
            let markdown = if matches!(mode, Mode::List) {
                selected
                    .iter()
                    .map(|p| format!("- [{}]({}.md)\n", p.title, public_url(&p.address)))
                    .collect()
            } else {
                selected
                    .iter()
                    .map(|p| p.markdown.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n")
            };
            let result = DocsResult {
                markdown,
                pages: selected
                    .iter()
                    .map(|p| DocsAddress {
                        title: p.title.clone(),
                        address: public_url(&p.address),
                        markdown_address: format!("{}.md", public_url(&p.address)),
                    })
                    .collect(),
            };
            Ok(secure(HttpResponse::json(
                serde_json::to_value(result).map_err(invalid)?,
            )))
        })
        .map_err(|_| invalid("docs tool has no mounted set"))?
}
