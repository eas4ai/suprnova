//! The preview pages: one document per component of the `{namespace}`
//! library, each mounting the component as a public seed beside its
//! stylesheet and script.
//!
//! Nothing of the library is copied into this application. The namespace's
//! asset route serves each stylesheet and script straight from
//! `../components/`; `askama.toml` makes `../components/` a template root, and
//! `templates/{namespace}-ui/<component>/<view>.html` includes the library's view
//! from there; `src/live/{namespace_module}/mod.rs` compiles each Rust file by
//! `#[path]`. A stylesheet or script edit shows on the next reload; a view or
//! Rust edit shows once the preview is rebuilt.
//!
//! Give each new component a page: add its view stub and its `#[path]` line,
//! declare its mount in `routes()` and route its path to `page`, as the
//! counter's are. The page itself is `templates/_preview/page.html`, a name no
//! component directory can take.

use std::collections::BTreeMap;

use suprnova::live::{
    CanonicalValue, ComponentContract, LiveBootstrapOptions, LiveDocument, LiveMount, MountFlags,
};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{FrameworkError, HttpResponse, Request, Response, Router, StatusCode};

use crate::live::{namespace_module}::counter::Counter;

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

/// Where the counter's preview renders.
pub const COUNTER_PATH: &str = "/preview/counter";

/// The document every preview page renders: one island, with the
/// component's stylesheets and scripts.
#[suprnova::view(path = "_preview/page.html")]
struct Page<'a> {
    title: &'a str,
    stylesheets: &'a [&'a str],
    scripts: &'a [&'a str],
    bootstrap: &'a TrustedHtml,
    island: &'a TrustedHtml,
}

/// What one preview page shows besides its island.
#[derive(Clone, Copy)]
struct Preview {
    title: &'static str,
    stylesheets: &'static [&'static str],
    scripts: &'static [&'static str],
}

const COUNTER: Preview = Preview {
    title: "Counter",
    stylesheets: &["/{namespace}-ui/counter/counter.css"],
    scripts: &["/{namespace}-ui/counter/counter.js"],
};

/// Serves the library's stylesheets and scripts at `/{namespace}-ui/`, as an
/// application serves them once it installs the library, and installs every
/// preview page with its island.
pub fn routes(router: Router) -> Result<Router, FrameworkError> {
    let router =
        router.try_live_ui_assets_for_from("{namespace}", suprnova::base_path("../components"))?;
    let counter = LiveMount::<Counter>::public_seed(COUNTER_PATH, "counter", "{namespace}-counter")?;
    let mount = counter.clone();
    let router: Router = router
        .get(COUNTER_PATH, move |request: Request| {
            let mount = mount.clone();
            async move { page(request, &mount, COUNTER).await }
        })
        .into();
    router.try_live_mount(&counter)
}

/// Renders one component's preview page.
async fn page<C: ComponentContract>(
    request: Request,
    mount: &LiveMount<C>,
    preview: Preview,
) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let mut document = LiveDocument::from_request(&request)?;
        let island = document
            .mount(mount, CanonicalValue::Object(BTreeMap::new()), MountFlags::empty())
            .await?;
        let bootstrap = document.bootstrap(LiveBootstrapOptions::esm())?;
        document
            .render(
                ViewName::parse("_preview/page.html")
                    .map_err(|_| FrameworkError::internal("the preview page's view name"))?,
                &Page {
                    title: preview.title,
                    stylesheets: preview.stylesheets,
                    scripts: preview.scripts,
                    bootstrap: bootstrap.html(),
                    island: island.html(),
                },
                DocumentResponseIntent::html(StatusCode::OK)
                    .map_err(|_| FrameworkError::internal("the preview page's response"))?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(|error| {
        // The detail goes to the log; the page says where to look.
        tracing::warn!(error = %error, "a preview page failed");
        HttpResponse::text("The preview page failed; the server log says why.").status(500)
    })
}
