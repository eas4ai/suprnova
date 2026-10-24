# Documentation sites

`suprnova-docs` serves a Markdown tree inside your application. The same pages
feed the browser, Markdown twins, `llms.txt`, browser search and three read tools
on your existing [agents endpoint](agents.md). Content is read and indexed at
boot, so deployment publishes a consistent snapshot without a search service.

## Install and mount

Add the plugin beside your framework dependency. Keep both on the same release
and source so requests and routers have one Rust crate identity. Like the
payment adapters, the plugin is distributed from the framework repository by
Git, not crates.io. Pin both dependencies to the same Git URL and the same tag
or revision that includes this plugin. Do not mix a Git framework with a local
plugin, or two different revisions of the framework.

Inside this workspace, `app/Cargo.toml` uses these local paths:

```toml
[dependencies]
suprnova = { path = "../framework", default-features = false, features = ["agents"] }
suprnova-docs = { path = "../crates/suprnova-docs" }
```

Mount before preparing the tool registry. The callback returns a complete HTML
document, so it can call your view engine or use an application-owned template.
The dogfood version lives in `app/src/docs.rs` and is mounted by
`app/src/agents.rs` when the application serves its routes.

```rust
use suprnova::{AgentsConfig, FrameworkError, Router};
use suprnova_docs::{DocsConfig, DocsSite};

fn routes() -> Result<Router, FrameworkError> {
    let config = DocsConfig::new("docs", "/docs", |_, page| {
        Ok(format!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">{}</head><body><header>Flight manual{}{}</header><aside>{}{}</aside><main>{}</main></body></html>",
            page.alternate_html, page.versions_html, page.languages_html,
            page.navigation_html, page.search_html, page.html,
        ))
    })?;
    DocsSite::mount(Router::new(), config)?.try_agents(AgentsConfig::new())
}
```

`DocsPage` supplies sanitized `html` from `MarkdownRenderer`, escaped HTML
fragments for navigation, version links, language links and search, and an
`alternate_html` link for the head. `title`, `site_title`, `version`, `language`
and `address` are plain values; escape them if you insert them into HTML.
`nonce` lets your own script tags share the response's nonce. Include the search
fragment once per page. The layout callback also receives the current request.

`DocsConfig::new` accepts an absolute mount prefix such as `/help`, or `/` for
the application root. One set may be mounted per router: the three agent tool
names belong to that set. Conflicting routes fail mounting instead of replacing
another handler. Malformed JSON, missing pages, duplicate or reserved paths,
invalid versions and symlinks outside the content folder fail at boot.

The prefix is relative to the application. Every generated address includes the
request's public root from `suprnova::url::root()`. With
`APP_URL=https://example.org/billing`, a set mounted at `/docs` serves public
links under `/billing/docs`. A trusted proxy's `X-Forwarded-Prefix` replaces
that root for its request; outside a request, the path in `APP_URL` supplies it.
Navigation, version and language switches, HTML content links, assets, Markdown
discovery, corpus index links, search and tool addresses all follow this rule.
The examples below assume the application is served at the host root.

## Folder layout

A single-language set looks like this:

```text
docs/
  docs.json
  v2/
    navigation.json
    index.md
    guides/
      install.md
    crew.md
    _assets/
      orbit.svg
  v1/
    navigation.json
    index.md
```

Pages are UTF-8 Markdown. Navigation defines the pages served and their order;
unlisted Markdown files are not routes. Page paths have no extension and use
letters, digits, hyphens, underscores, dots and slash-separated folders. Empty
segments, `.` and `..`, URL escapes and query characters are refused. Versions
and language codes use the same characters without slashes. `_assets`,
`_tools`, `search.json`, `llms.txt` and `llms-full.txt` are reserved page names.
Changes on disk become visible when the application boots again.

## Manifest and navigation

`docs/docs.json` names the set, ordered versions, default version and languages.
Every version needs a folder and at least one page. The default language is the
first language. Unknown manifest and navigation keys are errors.

```json
{
  "title": "Flight manual",
  "versions": [{ "id": "v2" }, { "id": "v1" }],
  "default_version": "v2",
  "languages": ["en"],
  "private": false
}
```

Each version's `navigation.json` is an array of nodes. A node has a title and a
page path, children, or both. A private group makes its descendants private.
Depth-first order defines both the navigation and the corpus indexes.

```json
[
  { "title": "Welcome", "path": "index" },
  {
    "title": "Guides",
    "children": [{ "title": "Install", "path": "guides/install" }]
  },
  { "title": "Crew briefing", "path": "crew", "private": true }
]
```

Use Markdown links such as `[Install](guides/install.md)` or
`[Welcome](../index.md#overview)`. HTML links resolve within the current version
and language; the `.md` suffix is removed while query strings and fragments
stay intact. Leading-slash content links resolve from that version's root.
External URLs and fragment links stay as written. Images in `_assets` resolve
to `/docs/v2/_assets/...`, shared by that version's languages. Assets are
snapshotted at boot and served with their MIME type. Symlinks in assets are
refused. Keep confidential assets in a private version or set; a page's privacy
does not make a shared version asset private.

## Versions and languages

`/docs/v2/index` is the canonical versioned page. `/docs/index` aliases the
default version and `/docs` serves its first navigation page. `/docs/v2` serves
that version's first page. A set with one version emits no version switch; one
language emits no language switch. A switch links to the same page when that
page is admitted in the target slice, otherwise its first admitted page.

For more than one language, place each translation under its language code:

```text
docs/v2/navigation.json
docs/v2/en/index.md
docs/v2/en/guides/install.md
docs/v2/fr/index.md
docs/v2/fr/guides/install.md
docs/v2/_assets/orbit.svg
```

Set `"languages": ["en", "fr"]`. Both translations use the version's navigation
and every listed page must exist in each language. Their URLs become
`/docs/v2/en/index` and `/docs/v2/fr/index`. The version root and its indexes
select the first language. Default-version aliases include `/docs/en/index`.
Navigation labels are shared; translate the page bodies and titles in Markdown.

## Private pages

A set has `"private": true` at the manifest root; a version has
`{ "id": "v1", "private": true }`; a navigation page or group has
`"private": true`. Marks inherit, and private content is denied unless the
application supplies a gate. The gate can inspect the request and `DocsAccess`
(version, language and optional page path), or use [authorization](authorization.md).
A path of `None` checks a whole set/version for indexes or shared assets.

```rust
use suprnova::{Auth, FrameworkError};
use suprnova_docs::DocsConfig;

fn config() -> Result<DocsConfig, FrameworkError> {
    DocsConfig::new("docs", "/docs", |_, page| {
        Ok(format!("<!doctype html><html><head>{}</head><body>{}{}{}{}{}</body></html>",
            page.alternate_html, page.navigation_html, page.versions_html,
            page.languages_html, page.html, page.search_html))
    })?.with_gate(|_, _| Box::pin(async {
        Ok(Auth::user().await?.is_some())
    }))
}
```

The dogfood set uses this sign-in gate for `crew`. Refused private pages answer
404, disappear from navigation, search, indexes and tool results, and cannot be
read through a default-version alias or Markdown negotiation. A gate failure
fails the request. Responses are `private, no-store`; do not put a public cache
in front of a gated set. MCP invokes the gate as the verified caller's user,
not by forwarding a bearer token to an HTTP page.

## Markdown twins and corpus indexes

Each page has a `.md` twin, and `Accept: text/markdown` also selects Markdown.
A media range with `q=0` does not select it. Twins return the original source,
`text/markdown; charset=utf-8` and `X-Robots-Tag: noindex`. HTML advertises the
twin through `Link` and the `alternate_html` fragment. Responses vary on
`Accept`, `Cookie` and `Authorization`. The external `suprnova-markdown` wrapper
is not a workspace dependency; both use the framework's content pipeline.

```bash
curl http://localhost:3000/docs/v2/launch.md
curl -H 'Accept: text/markdown' http://localhost:3000/docs/v2/launch
curl http://localhost:3000/docs/llms.txt
curl http://localhost:3000/docs/llms-full.txt
```

`llms.txt` has a set heading and one Markdown link per admitted page.
`llms-full.txt` concatenates exactly those pages' original Markdown in navigation
order, with no added headers or separators. Both exist at the set
root for the default version, at every version root, and at each language root
when the set declares multiple languages. For example, `/docs/v1/llms.txt` and
`/docs/v2/fr/llms-full.txt` select those slices.

## Browser search

Each slice serves `search.json`, containing title, HTML and Markdown addresses,
headings and plain text per admitted page. The small dependency-free script in
`DocsPage::search_html` loads this index and matches all entered words against
titles, headings and text. It displays up to twenty links using DOM text nodes.
No search process, JavaScript package or external request is needed.

Register [security headers](security-headers.md) so the script shares the
response's CSP nonce. Without that middleware the plugin creates a nonce and a
local CSP for its HTML response. Search JSON, Markdown and assets carry no
inline script. The boot index is shared; visibility is checked on every request
before any index is serialized.

## Agent tools

Mounting registers routed `#[tool]` handlers. Your existing MCP endpoint lists
`search_docs`, `read_page` and `list_pages`; the plugin does not create an
endpoint. All three accept optional `version` and `language` strings.

| Tool | Arguments | Answer |
| --- | --- | --- |
| `search_docs` | `query`, up to 200 characters | Up to twenty matches, with Markdown bodies and HTML addresses |
| `read_page` | `path`, without `.md` | One original Markdown body with its HTML address |
| `list_pages` | No additional arguments | Markdown links in navigation order |

For example, call `read_page` with `{"path":"launch","version":"v2"}` or
`search_docs` with `{"query":"orbital alignment"}`. An empty search has no hits.
Read routes also live under `/docs/_tools/` and run the same gate. Tool handlers return JSON for the existing MCP adapter. Its structured result
has `markdown` (original source, or the listing) and `pages` (title, canonical
HTML address and Markdown address). The content is Markdown, never rendered
HTML, and carries the read-only and untrusted-content annotations: documentation is data for an agent to read.

### Why Suprnova diverges

LaRecipe includes Ask AI, an agent skill, write tools, database sources and an
editing workflow. This plugin serves the agreed read-only filesystem set. It
uses Suprnova's existing Markdown renderer, application layout and MCP endpoint.
It adds no AI provider, authoring skill, write tool, separate server or frontend
build. Source edits follow the application's normal review and deployment.

## Next

Read [Agents](agents.md) to configure MCP authentication and
[Security Headers](security-headers.md) to set your browser policy.
