//! The dogfood documentation set uses the same plugin an application installs.

use suprnova::{Auth, FrameworkError, Router};
use suprnova_docs::{DocsConfig, DocsSite};

/// Mounts the flight manual with an application-owned layout and sign-in gate.
///
/// Both browser sessions and verified MCP callers reach `Auth::user`, so the
/// private crew briefing follows the same policy on every representation.
pub fn mount(router: Router) -> Result<Router, FrameworkError> {
    let config = DocsConfig::new(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs"),
        "/docs",
        |_, page| {
            let title = page.title.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
            Ok(format!(
                "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{title} | Suprnova</title>{}</head><body data-docs-layout><header><a href=\"/\">Suprnova</a><p>Flight manual</p>{}{}</header><aside>{}{}</aside><main>{}</main></body></html>",
                page.alternate_html, page.versions_html, page.languages_html,
                page.navigation_html, page.search_html, page.html,
            ))
        },
    )?.with_gate(|_, _| Box::pin(async { Ok(Auth::user().await?.is_some()) }))?;
    DocsSite::mount(router, config)
}
