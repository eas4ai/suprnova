//! Session-free Inertia renders for explicitly marked public pages.

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use indexmap::IndexMap;
use sha2::{Digest, Sha256};

use super::{CacheableInertiaProps, Prop};
use crate::{App, FrameworkError, HttpResponse, Inertia, Router};

/// A typed declaration is independent of the route's explicit publication marker.
#[derive(Clone)]
pub(crate) struct PublicPage {
    fields: BTreeSet<String>,
    refusal: Option<String>,
}

/// Both representations come from one set of resolved values.
#[derive(Clone, Debug)]
pub(crate) struct PublicPageCapture {
    /// Route pattern whose declaration authorized this capture.
    pub(crate) route: String,
    /// Trusted first-visit document before downstream middleware rewrites.
    pub(crate) html: Bytes,
    /// Page JSON resolved together with the document.
    pub(crate) json: Bytes,
    /// Digest checked against the final response body before publication.
    pub(crate) digest: [u8; 32],
    /// CSP hash sources derived only from the trusted document.
    pub(crate) script_hashes: Vec<String>,
}

struct PublicationScope {
    route: String,
    capture: Mutex<Option<PublicPageCapture>>,
    refusal: Mutex<Option<String>>,
}

tokio::task_local! {
    static PUBLICATION: Arc<PublicationScope>;
}

impl Inertia {
    /// Declares the props of a public page without marking its route for publication.
    ///
    /// Per-request fields remain a per-route boot refusal. A refusal does not stop
    /// the application's listener from opening.
    pub fn public_page<P: CacheableInertiaProps>(
        route: &str,
        component: &str,
    ) -> Result<(), FrameworkError> {
        let fields: BTreeSet<String> = P::PUBLIC.iter().map(|name| (*name).to_owned()).collect();
        let refusal = if !P::PER_REQUEST.is_empty() {
            Some(format!(
                "per-request props are not public: {}",
                P::PER_REQUEST.join(", ")
            ))
        } else {
            fields
                .iter()
                .find(|name| private_key(name))
                .map(|name| format!("credential or private prop `{name}` is not public"))
        };
        crate::lock::write(
            &App::inertia_registry().public_pages,
            "Inertia public pages",
        )?
        .insert(
            (route.to_owned(), component.to_owned()),
            PublicPage { fields, refusal },
        );
        Ok(())
    }

    /// Lists the named shared props a session-free public render may resolve.
    ///
    /// Register ordinary named shares first. Authentication, errors, flash and
    /// credential shares always refuse publication, even if this list names them.
    pub fn public_shared(keys: &[&str]) -> Result<(), FrameworkError> {
        for key in keys {
            if private_key(key) {
                return Err(FrameworkError::internal(format!(
                    "Inertia shared prop `{key}` contains private or credential data"
                )));
            }
        }
        let registry = App::inertia_registry();
        let mut public =
            crate::lock::write(&registry.public_shared, "Inertia public shared props")?;
        public.extend(keys.iter().map(|key| (*key).to_owned()));
        Ok(())
    }
}

fn private_key(key: &str) -> bool {
    matches!(
        key.split('.').next().unwrap_or(key),
        "auth"
            | "errors"
            | "flash"
            | "csrf"
            | "csrf_token"
            | "csrfToken"
            | "xsrfToken"
            | "credentials"
    )
}

/// Whether this task is rendering an explicitly marked public page.
pub(crate) fn active() -> bool {
    PUBLICATION.try_with(|_| ()).is_ok()
}

/// The matched route pattern used in refusal diagnostics.
pub(crate) fn route() -> Option<String> {
    PUBLICATION.try_with(|scope| scope.route.clone()).ok()
}

/// The caller keeps the collector outside this scope to inspect handler reads.
pub(crate) async fn scope<F: Future>(
    route: String,
    future: F,
) -> (F::Output, Option<PublicPageCapture>, Option<String>) {
    let state = Arc::new(PublicationScope {
        route,
        capture: Mutex::new(None),
        refusal: Mutex::new(None),
    });
    let output = PUBLICATION.scope(state.clone(), future).await;
    let capture = state
        .capture
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take();
    let refusal = state
        .refusal
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take();
    (output, capture, refusal)
}

/// Remembers the cause before a handler converts its error to a redacted HTTP 500.
pub(crate) fn refusal(cause: impl std::fmt::Display) -> FrameworkError {
    let message = format!(
        "Public page `{}` refused: {cause}",
        route().as_deref().unwrap_or("unknown")
    );
    let _ = PUBLICATION.try_with(|scope| {
        *scope
            .refusal
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(message.clone());
    });
    FrameworkError::internal(message)
}

/// Only declared named shares are evaluated. Credential providers never run.
pub(crate) fn shares(
    component: &str,
    props: &IndexMap<String, Prop>,
    has_providers: bool,
) -> Result<Vec<(String, Prop)>, FrameworkError> {
    let registry = App::inertia_registry();
    let route = route().ok_or_else(|| refusal("publication scope is missing"))?;
    let pages = crate::lock::read(&registry.public_pages, "Inertia public pages")?;
    let page = pages.get(&(route, component.to_owned())).ok_or_else(|| {
        refusal(format!(
            "component `{component}` has no typed public declaration"
        ))
    })?;
    if let Some(cause) = &page.refusal {
        return Err(refusal(cause));
    }
    if has_providers {
        return Err(refusal(
            "opaque page prop provider cannot prove its public fields",
        ));
    }
    if registry.trait_provider()?.is_some()
        || !registry.shared_providers()?.is_empty()
        || registry
            .installed_config()
            .is_some_and(|config| config.hooks.is_some())
    {
        return Err(refusal(
            "opaque shared prop provider or middleware hook is not listed public",
        ));
    }
    for key in props.keys() {
        let root = key.split('.').next().unwrap_or(key);
        if private_key(root) || !page.fields.contains(root) {
            return Err(refusal(format!("undeclared or private page prop `{root}`")));
        }
    }
    let public = crate::lock::read(&registry.public_shared, "Inertia public shared props")?;
    let mut shares = Vec::new();
    for (key, prop) in registry.snapshot_static()? {
        if private_key(&key) {
            return Err(refusal(format!(
                "private shared prop `{key}` cannot be public"
            )));
        }
        if !public.contains(&key) {
            return Err(refusal(format!("unlisted shared prop `{key}`")));
        }
        shares.push((key, prop));
    }
    Ok(shares)
}

/// Static declarations are checked per route; runtime refusals add observed causes.
pub(crate) fn boot_refusals(router: &Router) -> Result<Vec<(String, String)>, FrameworkError> {
    let registry = App::inertia_registry();
    let pages = crate::lock::read(&registry.public_pages, "Inertia public pages")?;
    let public = crate::lock::read(&registry.public_shared, "Inertia public shared props")?;
    let shares = registry.snapshot_static()?;
    let mut refusals = Vec::new();
    for route in router.public_page_patterns() {
        let declared: Vec<_> = pages
            .iter()
            .filter(|((pattern, _), _)| pattern == &route)
            .collect();
        if declared.is_empty() {
            refusals.push((
                route.clone(),
                "missing typed public page declaration".to_owned(),
            ));
        }
        for (_, page) in declared {
            if let Some(cause) = &page.refusal {
                refusals.push((route.clone(), cause.clone()));
            }
        }
        if registry.trait_provider()?.is_some()
            || !registry.shared_providers()?.is_empty()
            || registry
                .installed_config()
                .is_some_and(|config| config.hooks.is_some())
            || crate::middleware::get_global_middleware()
                .iter()
                .any(crate::middleware::inertia_hooks_of)
            || router
                .get_route_middleware(&hyper::Method::GET, &route)
                .iter()
                .any(crate::middleware::inertia_hooks_of)
        {
            refusals.push((
                route.clone(),
                "opaque shared prop provider or middleware hook is not listed public".to_owned(),
            ));
        }
        for (key, _) in &shares {
            if private_key(key) || !public.contains(key) {
                refusals.push((route.clone(), format!("unlisted shared prop `{key}`")));
            }
        }
    }
    Ok(refusals)
}

/// Captures both representations and rejects nonce-bearing document markup.
pub(crate) fn capture(response: &HttpResponse, json: &[u8]) -> Result<(), FrameworkError> {
    let hashes =
        crate::security_headers::trusted_script_hashes(response.body()).map_err(refusal)?;
    PUBLICATION
        .try_with(|scope| {
            let capture = PublicPageCapture {
                route: scope.route.clone(),
                html: Bytes::copy_from_slice(response.body()),
                json: Bytes::copy_from_slice(json),
                digest: Sha256::digest(response.body()).into(),
                script_hashes: hashes,
            };
            *scope
                .capture
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = Some(capture);
        })
        .map_err(|_| refusal("publication scope is missing"))
}

/// The policy hashes trusted render bytes, never a later middleware rewrite.
pub(crate) fn script_hashes() -> Vec<String> {
    PUBLICATION
        .try_with(|scope| {
            scope
                .capture
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .as_ref()
                .map(|capture| capture.script_hashes.clone())
                .unwrap_or_default()
        })
        .unwrap_or_default()
}

/// Keeps the embedded hydration page and published JSON on the same image URLs.
pub(crate) fn rewrite_public_image_urls(
    html: &Bytes,
    json: &Bytes,
    replacements: &std::collections::BTreeMap<String, String>,
) -> Result<(Bytes, Bytes), FrameworkError> {
    if replacements.is_empty() {
        return Ok((html.clone(), json.clone()));
    }
    let mut page: serde_json::Value = serde_json::from_slice(json).map_err(|error| {
        FrameworkError::internal(format!("Public image page JSON is invalid: {error}"))
    })?;
    let mut original = Vec::new();
    serde_json::to_writer(super::SlashEscaping(&mut original), &page).map_err(|error| {
        FrameworkError::internal(format!("Public image page encoding failed: {error}"))
    })?;
    fn rewrite(
        value: &mut serde_json::Value,
        replacements: &std::collections::BTreeMap<String, String>,
    ) -> bool {
        match value {
            serde_json::Value::String(value) => match replacements.get(value) {
                Some(replacement) => {
                    value.clone_from(replacement);
                    true
                }
                None => false,
            },
            serde_json::Value::Array(values) => values.iter_mut().fold(false, |changed, value| {
                rewrite(value, replacements) || changed
            }),
            serde_json::Value::Object(values) => {
                values.values_mut().fold(false, |changed, value| {
                    rewrite(value, replacements) || changed
                })
            }
            _ => false,
        }
    }
    if !rewrite(&mut page, replacements) {
        return Ok((html.clone(), json.clone()));
    }
    let mut matches = html
        .windows(original.len())
        .enumerate()
        .filter_map(|(at, bytes)| (bytes == original).then_some(at));
    let at = matches.next().ok_or_else(|| {
        FrameworkError::internal("Public image rewrite could not find its embedded page JSON")
    })?;
    if matches.next().is_some() {
        return Err(FrameworkError::internal(
            "Public image rewrite found ambiguous embedded page JSON",
        ));
    }
    let json = serde_json::to_vec(&page).map_err(|error| {
        FrameworkError::internal(format!("Public image page encoding failed: {error}"))
    })?;
    let mut embedded = Vec::new();
    serde_json::to_writer(super::SlashEscaping(&mut embedded), &page).map_err(|error| {
        FrameworkError::internal(format!("Public image page encoding failed: {error}"))
    })?;
    let mut document = Vec::with_capacity(html.len() - original.len() + embedded.len());
    document.extend_from_slice(&html[..at]);
    document.extend_from_slice(&embedded);
    document.extend_from_slice(&html[at + original.len()..]);
    Ok((Bytes::from(document), Bytes::from(json)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InertiaRequestExt, InertiaResponse};

    struct PublicRequest;
    impl InertiaRequestExt for PublicRequest {
        fn path(&self) -> &str {
            "/public"
        }
        fn header(&self, _: &str) -> Option<&str> {
            None
        }
    }

    struct PublicProps;
    impl CacheableInertiaProps for PublicProps {
        const PUBLIC: &'static [&'static str] = &["title"];
        const PER_REQUEST: &'static [&'static str] = &[];
        fn cache_props(&self) -> Result<IndexMap<String, Prop>, FrameworkError> {
            Ok(IndexMap::new())
        }
    }

    #[test]
    fn pub_007_rewritten_images_keep_hydration_json_and_script_escaping()
    -> Result<(), FrameworkError> {
        let page = serde_json::json!({"component":"Images", "props":{"nested":[{"image":"/old.png","text":"</script><script>bad()</script>"}]},"url":"/images","version":"v1"});
        let json = Bytes::from(
            serde_json::to_vec(&page)
                .map_err(|error| FrameworkError::internal(error.to_string()))?,
        );
        let mut embedded = Vec::new();
        serde_json::to_writer(super::super::SlashEscaping(&mut embedded), &page)
            .map_err(|error| FrameworkError::internal(error.to_string()))?;
        let html = Bytes::from(format!(
            "<script data-page=\"custom\" type=\"application/json\">{}</script><div id=\"custom\"></div>",
            String::from_utf8_lossy(&embedded)
        ));
        let (same_html, same_json) = rewrite_public_image_urls(&html, &json, &Default::default())?;
        assert_eq!(same_html.as_ptr(), html.as_ptr());
        assert_eq!(same_json.as_ptr(), json.as_ptr());
        let replacements = std::collections::BTreeMap::from([(
            "/old.png".to_owned(),
            "/_pub/assets/image.original.png".to_owned(),
        )]);
        let (html, json) = rewrite_public_image_urls(&html, &json, &replacements)?;
        let page: serde_json::Value = serde_json::from_slice(&json)
            .map_err(|error| FrameworkError::internal(error.to_string()))?;
        assert_eq!(
            page["props"]["nested"][0]["image"],
            "/_pub/assets/image.original.png"
        );
        let html = String::from_utf8_lossy(&html);
        assert!(html.contains("image.original.png"));
        assert!(!html.contains("<script>bad()"));
        assert!(!html.contains("old.png"));
        Ok(())
    }

    #[tokio::test]
    async fn pub_001_one_render_captures_html_json_without_private_shares()
    -> Result<(), FrameworkError> {
        let _container = crate::testing::TestContainer::fake();
        Inertia::public_page::<PublicProps>("/public", "Public")?;
        Inertia::share("app", "Public app")?;
        Inertia::public_shared(&["app"])?;
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let resolved = calls.clone();
        let (response, capture, _) = scope(
            "/public".to_owned(),
            InertiaResponse::new("Public")
                .lazy("title", move || {
                    let resolved = resolved.clone();
                    async move {
                        resolved.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok::<_, FrameworkError>("hello")
                    }
                })
                .resolve(&PublicRequest),
        )
        .await;
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        let response = response?;
        let capture =
            capture.ok_or_else(|| FrameworkError::internal("expected publication capture"))?;
        assert_eq!(response.body(), capture.html);
        let json: serde_json::Value = serde_json::from_slice(&capture.json)
            .map_err(|error| FrameworkError::internal(error.to_string()))?;
        assert_eq!(json["props"]["title"], "hello");
        assert_eq!(json["props"]["app"], "Public app");
        assert!(json["props"].get("auth").is_none());
        assert!(json["props"].get("errors").is_none());
        assert!(json.get("flash").is_none());
        assert!(!String::from_utf8_lossy(&capture.html).contains("must-not-leak"));
        Ok(())
    }

    #[tokio::test]
    async fn pub_001_unlisted_share_refuses_with_route_and_cause() -> Result<(), FrameworkError> {
        let _container = crate::testing::TestContainer::fake();
        Inertia::public_page::<PublicProps>("/public", "Public")?;
        Inertia::share("private_settings", "hidden")?;
        let (response, capture, _) = scope(
            "/public".to_owned(),
            InertiaResponse::new("Public").resolve(&PublicRequest),
        )
        .await;
        let error = response
            .err()
            .ok_or_else(|| FrameworkError::internal("expected refusal"))?
            .to_string();
        assert!(
            error.contains("/public") && error.contains("private_settings"),
            "{error}"
        );
        assert!(capture.is_none());
        Ok(())
    }
}
