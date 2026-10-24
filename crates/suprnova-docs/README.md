# suprnova-docs

Mount a versioned Markdown documentation set inside a Suprnova application.
The application supplies its layout and private-content gate. One boot snapshot
serves HTML, Markdown twins, `llms.txt`, `llms-full.txt`, browser search and the
`search_docs`, `read_page` and `list_pages` tools on the existing MCP endpoint.

```rust
use suprnova::{AgentsConfig, FrameworkError, Router};
use suprnova_docs::{DocsConfig, DocsSite};

fn router() -> Result<Router, FrameworkError> {
    let config = DocsConfig::new("docs", "/docs", |_, page| Ok(format!(
        "<!doctype html><html><head>{}</head><body>{}{}{}{}{}</body></html>",
        page.alternate_html, page.navigation_html, page.versions_html,
        page.languages_html, page.html, page.search_html,
    )))?;
    DocsSite::mount(Router::new(), config)?.try_agents(AgentsConfig::new())
}
```

Use the same version and source for the framework and plugin. The plugin's
framework dependency disables default features and enables `agents`; the
application chooses database, filesystem and other features. The external
`suprnova-markdown` crate is not needed: this plugin uses the same
`MarkdownRenderer` directly.

See [the manual](../../manual/docs-site.md) for the manifest and navigation
schema, folder layout, languages, privacy, CSP and tool arguments. The fixture
sets under `tests/fixtures` are complete examples; `app/docs` is the dogfood set.
Run the contract tests with `cargo nextest run -p suprnova-docs` and the browser
search behavior checks with `bun test crates/suprnova-docs/tests/browser_search.test.js`
from the workspace root.
