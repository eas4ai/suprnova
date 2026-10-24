//! Serve one Markdown tree to readers and agents inside the application's layout.
//!
//! Mounting reads and validates the set once. Request-time authorization filters
//! every representation, so a boot-time index cannot bypass the application's gate.
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

mod content;
mod presentation;
mod tools;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use serde::Deserialize;
use suprnova::{FrameworkError, HttpResponse, Request, Router};

pub use presentation::DocsPage;

/// Lets an application's asynchronous authorization use the current request.
pub type GateFuture<'a> = Pin<Box<dyn Future<Output = Result<bool, FrameworkError>> + Send + 'a>>;
type Gate = dyn for<'a> Fn(&'a Request, &'a DocsAccess) -> GateFuture<'a> + Send + Sync;
type Layout = dyn Fn(&Request, &DocsPage) -> Result<String, FrameworkError> + Send + Sync;

/// Identifies the protected content so the application can apply its own policy.
#[derive(Clone, Debug)]
pub struct DocsAccess {
    /// Selects the version whose visibility is being checked.
    pub version: String,
    /// Selects the language whose page is being checked.
    pub language: String,
    /// Identifies a page, or is absent for a version's shared assets and indexes.
    pub path: Option<String>,
}

/// Keeps filesystem, layout and authorization choices with the application.
pub struct DocsConfig {
    root: PathBuf,
    prefix: String,
    layout: Arc<Layout>,
    gate: Arc<Gate>,
}

impl DocsConfig {
    /// Validates a mount prefix and accepts the application's complete HTML layout.
    ///
    /// `root` usually names `<application root>/docs`. Empty or `/` mounts at
    /// the application root. Private content is denied until a gate is supplied.
    pub fn new(
        root: impl AsRef<Path>,
        prefix: &str,
        layout: impl Fn(&Request, &DocsPage) -> Result<String, FrameworkError> + Send + Sync + 'static,
    ) -> Result<Self, FrameworkError> {
        let prefix = prefix.trim_end_matches('/');
        if !prefix.is_empty() {
            let Some(path) = prefix.strip_prefix('/') else {
                return Err(invalid("the mount prefix must start with /"));
            };
            content::validate_path(path)?;
        }
        Ok(Self {
            root: root.as_ref().to_owned(),
            prefix: prefix.to_owned(),
            layout: Arc::new(layout),
            gate: Arc::new(|_, _| Box::pin(async { Ok(false) })),
        })
    }

    /// Supplies the same authorization decision to HTML, Markdown, search and tools.
    ///
    /// The gate runs only for private content and may use request-scoped `Auth`.
    /// A gate error fails the request rather than publishing a partial index.
    pub fn with_gate(
        mut self,
        gate: impl for<'a> Fn(&'a Request, &'a DocsAccess) -> GateFuture<'a> + Send + Sync + 'static,
    ) -> Result<Self, FrameworkError> {
        self.gate = Arc::new(gate);
        Ok(self)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    title: String,
    versions: Vec<Version>,
    default_version: String,
    languages: Vec<String>,
    #[serde(default)]
    private: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Version {
    id: String,
    #[serde(default)]
    private: bool,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Navigation {
    title: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    private: bool,
    #[serde(default)]
    children: Vec<Navigation>,
}

struct Page {
    access: DocsAccess,
    title: String,
    address: String,
    markdown: String,
    rendered: suprnova::RenderedMarkdown,
    private: bool,
}

struct Asset {
    version: String,
    address: String,
    bytes: bytes::Bytes,
    mime: String,
    private: bool,
}

struct Site {
    config: DocsConfig,
    manifest: Manifest,
    pages: Vec<Page>,
    navigation: std::collections::HashMap<String, Vec<Navigation>>,
    assets: Vec<Asset>,
}

/// Mounts a validated set without owning a server, layout engine or agent endpoint.
pub struct DocsSite;

impl DocsSite {
    /// Builds the index at boot and registers pages, twins, assets and three tools.
    ///
    /// Mount before `Router::try_agents` and before preparing the tool registry.
    /// Each router accepts one docs set because the three tool names are fixed.
    /// Malformed manifests, missing pages, duplicate paths and escaped filesystem
    /// paths fail boot instead of producing a partly working site.
    pub fn mount(mut router: Router, config: DocsConfig) -> Result<Router, FrameworkError> {
        let site = Arc::new(content::load(config)?);
        for (index, page) in site.pages.iter().enumerate() {
            router = mount_page(router, &page.address, &site, index)?;
        }
        for version in &site.manifest.versions {
            for language in &site.manifest.languages {
                router = mount_slice(
                    router,
                    &site,
                    &version.id,
                    language,
                    &site.base(&version.id, language),
                )?;
            }
            if site.manifest.languages.len() > 1 {
                router = mount_slice(
                    router,
                    &site,
                    &version.id,
                    &site.manifest.languages[0],
                    &format!("{}/{}", site.config.prefix, version.id),
                )?;
            }
        }
        for (index, asset) in site.assets.iter().enumerate() {
            let shared = Arc::clone(&site);
            router = router
                .try_get(&asset.address, move |req| {
                    let site = Arc::clone(&shared);
                    async move {
                        let asset = &site.assets[index];
                        let access = DocsAccess {
                            version: asset.version.clone(),
                            language: site.manifest.languages[0].clone(),
                            path: None,
                        };
                        site.admit(&req, asset.private, &access)
                            .await
                            .map_err(|e| secure(HttpResponse::from(e)))?;
                        Ok(secure(HttpResponse::bytes(
                            asset.bytes.clone(),
                            &asset.mime,
                        )))
                    }
                })?
                .into();
        }
        tools::mount(router, site)
    }
}

fn mount_slice(
    mut router: Router,
    site: &Arc<Site>,
    version: &str,
    language: &str,
    base: &str,
) -> Result<Router, FrameworkError> {
    let first = site
        .pages
        .iter()
        .position(|page| page.access.version == version && page.access.language == language)
        .ok_or_else(|| invalid("each version and language needs a page"))?;
    router = mount_page(router, base, site, first)?;
    for kind in ["llms.txt", "llms-full.txt", "search.json"] {
        router = mount_index(router, &format!("{base}/{kind}"), site, version, language)?;
    }
    Ok(router)
}

fn mount_page(
    mut router: Router,
    address: &str,
    site: &Arc<Site>,
    index: usize,
) -> Result<Router, FrameworkError> {
    for address in site.addresses(&site.pages[index].access.version, address) {
        // An empty root is represented by / in the HTTP router.
        let address = if address.is_empty() {
            "/".to_owned()
        } else {
            address
        };
        for (path, twin) in [(address.clone(), false), (format!("{address}.md"), true)] {
            let site = Arc::clone(site);
            let address = address.clone();
            router = router
                .try_get(&path, move |req| {
                    let site = Arc::clone(&site);
                    let address = address.clone();
                    async move {
                        presentation::serve(&site, &req, index, twin, &address)
                            .await
                            .map_err(|e| secure(HttpResponse::from(e)))
                    }
                })?
                .into();
        }
    }
    Ok(router)
}

fn mount_index(
    mut router: Router,
    address: &str,
    site: &Arc<Site>,
    version: &str,
    language: &str,
) -> Result<Router, FrameworkError> {
    let kind = address
        .rsplit('/')
        .next()
        .ok_or_else(|| invalid("index address missing"))?
        .to_owned();
    for address in site.addresses(version, address) {
        let site = Arc::clone(site);
        let version = version.to_owned();
        let language = language.to_owned();
        let kind = kind.clone();
        router = router
            .try_get(&address, move |req| {
                let site = Arc::clone(&site);
                let (version, language, kind) = (version.clone(), language.clone(), kind.clone());
                async move {
                    index_response(&site, &req, &version, &language, &kind)
                        .await
                        .map_err(|e| secure(HttpResponse::from(e)))
                }
            })?
            .into();
    }
    Ok(router)
}

async fn index_response(
    site: &Site,
    req: &Request,
    version: &str,
    language: &str,
    kind: &str,
) -> Result<HttpResponse, FrameworkError> {
    let pages = site.visible(req, version, language).await?;
    let response = match kind {
        "search.json" => HttpResponse::json(serde_json::Value::Array(pages.iter().map(|p| serde_json::json!({
            "title": p.title, "address": p.address, "markdown_address": format!("{}.md", p.address),
            "headings": p.rendered.headings, "text": p.rendered.plain_text,
        })).collect())),
        "llms-full.txt" => HttpResponse::markdown(pages.iter().map(|p| p.markdown.as_str()).collect::<String>()),
        _ => HttpResponse::markdown(format!("# {}\n\n{}", site.manifest.title, pages.iter().map(|p| format!("- [{}]({}.md)\n", p.title, p.address)).collect::<String>())),
    };
    Ok(secure(response))
}

impl Site {
    fn addresses(&self, version: &str, canonical: &str) -> Vec<String> {
        let mut addresses = vec![canonical.to_owned()];
        if version == self.manifest.default_version {
            let stem = format!("{}/{version}", self.config.prefix);
            if let Some(suffix) = canonical.strip_prefix(&stem) {
                addresses.push(format!("{}{suffix}", self.config.prefix));
            }
        }
        addresses
    }

    fn base(&self, version: &str, language: &str) -> String {
        if self.manifest.languages.len() == 1 {
            format!("{}/{version}", self.config.prefix)
        } else {
            format!("{}/{version}/{language}", self.config.prefix)
        }
    }

    async fn allowed(
        &self,
        req: &Request,
        private: bool,
        access: &DocsAccess,
    ) -> Result<bool, FrameworkError> {
        if private {
            (self.config.gate)(req, access).await
        } else {
            Ok(true)
        }
    }

    async fn admit(
        &self,
        req: &Request,
        private: bool,
        access: &DocsAccess,
    ) -> Result<(), FrameworkError> {
        if self.allowed(req, private, access).await? {
            Ok(())
        } else {
            Err(FrameworkError::not_found("documentation not found"))
        }
    }

    async fn visible<'a>(
        &'a self,
        req: &Request,
        version: &str,
        language: &str,
    ) -> Result<Vec<&'a Page>, FrameworkError> {
        let version = self
            .manifest
            .versions
            .iter()
            .find(|v| v.id == version)
            .ok_or_else(|| FrameworkError::not_found("documentation not found"))?;
        if !self.manifest.languages.iter().any(|l| l == language) {
            return Err(FrameworkError::not_found("documentation not found"));
        }
        self.admit(
            req,
            self.manifest.private || version.private,
            &DocsAccess {
                version: version.id.clone(),
                language: language.to_owned(),
                path: None,
            },
        )
        .await?;
        let mut pages = Vec::new();
        for page in &self.pages {
            if page.access.version == version.id
                && page.access.language == language
                && self.allowed(req, page.private, &page.access).await?
            {
                pages.push(page);
            }
        }
        Ok(pages)
    }
}

fn secure(response: HttpResponse) -> HttpResponse {
    response
        .header("Cache-Control", "private, no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("Vary", "Accept, Cookie, Authorization")
}

fn invalid(message: impl std::fmt::Display) -> FrameworkError {
    FrameworkError::internal(format!("documentation site: {message}"))
}
