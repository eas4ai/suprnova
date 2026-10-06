//! PFX-006 and PFX-004: a Live document behind a prefix names its action
//! endpoint and its assets under the root, and Live's reflected URLs and
//! redirect targets carry the root, so the browser neither calls the host
//! root nor refuses the reflection.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::json;
use suprnova::container::testing::{TestContainer, TestContainerGuard};
use suprnova::live::action::{OutcomeMetadata, action_result, route_intent, url_intent};
use suprnova::live::testing::{
    LiveSecurityCheck, prepare_live_router_for_test, record_live_security_pass_for_test,
};
use suprnova::live::{
    ActionOutcome, ActionResult, CanonicalValue, LiveBootstrapOptions, LiveComponent, LiveDocument,
    LiveMount, LiveRegistry, MountFlags, live,
};
use suprnova::view::{
    AssetSet, DocumentResponseIntent, TrustedHtml, TrustedMarkupReason, ViewName,
};
use suprnova::{
    App, HttpResponse, Middleware, MiddlewareRegistry, Next, Request, Response, Router, StatusCode,
    async_trait,
};

use crate::support::{self, PREFIX};

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

#[derive(LiveComponent)]
#[live(name = "tests.pfx-counter", view = "live/tests/public-counter.html")]
pub struct PfxCounter {
    #[public]
    count: u64,
}

#[live]
impl PfxCounter {
    #[action]
    pub fn go_to_receipt(&mut self) -> ActionOutcome {
        let parameters = CanonicalValue::Object(BTreeMap::from([(
            "receipt".to_owned(),
            CanonicalValue::String("42".to_owned()),
        )]));
        ActionOutcome::Redirect(
            route_intent("pfx.receipt", parameters).expect("a registered-route intent"),
        )
    }

    #[action]
    pub fn reflect_query(&mut self) -> ActionResult {
        let query = CanonicalValue::Object(BTreeMap::from([(
            "q".to_owned(),
            CanonicalValue::String("red shoes".to_owned()),
        )]));
        let metadata = OutcomeMetadata::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Some(url_intent(query).expect("a same-route URL intent")),
        )
        .expect("URL metadata");
        action_result::<Self>(ActionOutcome::NoRender, metadata).expect("a reflected result")
    }
}

#[suprnova::view(path = "live/tests/bootstrap-document.html")]
struct DocumentView<'a> {
    bootstrap: &'a TrustedHtml,
    first: &'a TrustedHtml,
    second: &'a TrustedHtml,
    third: &'a TrustedHtml,
}

/// Records the security facts the Live action route requires, on that
/// route only.
struct ActionFacts;

#[async_trait]
impl Middleware for ActionFacts {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        if request.path() != "/__live/action" {
            return next(request).await;
        }
        for (check, fact) in [
            (LiveSecurityCheck::Session, Some(b"session-42".as_slice())),
            (LiveSecurityCheck::Origin, None),
            (LiveSecurityCheck::Csrf, None),
            (
                LiveSecurityCheck::Principal,
                Some(b"principal-42".as_slice()),
            ),
            (LiveSecurityCheck::Tenant, Some(b"tenant-42".as_slice())),
            (LiveSecurityCheck::RateLimit, None),
        ] {
            if !record_live_security_pass_for_test(&mut request, check, fact) {
                return Err(HttpResponse::text("action facts rejected").status(500));
            }
        }
        next(request).await
    }
}

type DocumentFuture = Pin<Box<dyn Future<Output = Response> + Send>>;

fn empty_slot() -> TrustedHtml {
    TrustedHtml::framework_static(
        "",
        TrustedMarkupReason::new("unused island slot").expect("a reason"),
    )
    .expect("empty markup")
}

fn document_handler(
    mount: &LiveMount<PfxCounter>,
) -> impl Fn(Request) -> DocumentFuture + Send + Sync + 'static {
    let mount = mount.clone();
    move |request: Request| -> DocumentFuture {
        let mount = mount.clone();
        Box::pin(async move {
            let result: Result<HttpResponse, String> = async {
                let mut document =
                    LiveDocument::from_request(&request).map_err(|error| error.to_string())?;
                let island = document
                    .mount(
                        &mount,
                        CanonicalValue::Object(BTreeMap::new()),
                        MountFlags::empty(),
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                let bootstrap = document
                    .bootstrap(LiveBootstrapOptions::esm())
                    .map_err(|error| error.to_string())?;
                let empty = empty_slot();
                document
                    .render(
                        ViewName::parse("live/tests/bootstrap-document.html")
                            .map_err(|error| error.to_string())?,
                        &DocumentView {
                            bootstrap: bootstrap.html(),
                            first: island.html(),
                            second: &empty,
                            third: &empty,
                        },
                        DocumentResponseIntent::html(StatusCode::OK)
                            .map_err(|error| error.to_string())?,
                        AssetSet::empty(),
                    )
                    .map_err(|error| error.to_string())
            }
            .await;
            result.map_err(|error| HttpResponse::text(error).status(500))
        })
    }
}

fn live_router(mount: &LiveMount<PfxCounter>) -> Router {
    let router: Router = Router::new()
        .get("/receipts/{receipt}", |_request: Request| async {
            Ok(HttpResponse::text("receipt"))
        })
        .name("pfx.receipt");
    let router: Router = router
        .get("/catalog/{section}", document_handler(mount))
        .into();
    let router = router
        .try_live()
        .expect("install the Live endpoint")
        .try_live_mount(mount)
        .expect("register the document mount");
    prepare_live_router_for_test(&router).expect("prepare the Live runtime");
    router
}

fn attribute<'html>(html: &'html str, name: &str) -> &'html str {
    let prefix = format!("{name}=\"");
    let start = html
        .find(&prefix)
        .map(|index| index + prefix.len())
        .unwrap_or_else(|| panic!("no {name} in {html}"));
    let tail = &html[start..];
    &tail[..tail.find('"').expect("a closing quote")]
}

fn config_endpoint(html: &str) -> String {
    let start = html
        .find("<script id=\"suprnova-live-config\" type=\"application/json\">")
        .expect("the configuration element");
    let rest = &html[start..];
    let open = rest.find('>').expect("an open tag") + 1;
    let close = rest.find("</script>").expect("a close tag");
    let config: serde_json::Value =
        serde_json::from_str(&rest[open..close]).expect("configuration JSON");
    config["endpoint"].as_str().expect("an endpoint").to_owned()
}

/// A fresh 16-byte identifier, base64url: every action needs its own
/// correlation id, idempotency key and browser nonce, or the second is a
/// replay.
fn fresh_id() -> String {
    static NEXT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(1);
    let byte = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    URL_SAFE_NO_PAD.encode([byte; 16])
}

fn action_body(snapshot: serde_json::Value, action: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "base_revision": "0",
        "child_parameters": null,
        "component": "tests.pfx-counter",
        "correlation_id": fresh_id(),
        "extensions": {
            "x_suprnova_framework_document_path_v1": "/browser-forged",
            "x_suprnova_live_document_key_v1": "catalog-counter",
        },
        "idempotency_key": fresh_id(),
        "model_proposals": {},
        "operations": [{"arguments": {}, "kind": "invoke_action", "name": action}],
        "protocol_version": 2,
        "runtime_contract_version": 2,
        "snapshot": {
            "browser_nonce": fresh_id(),
            "envelope": snapshot,
            "kind": "seed_promotion",
        },
        "snapshot_schema_version": 1,
    }))
    .expect("an action request")
}

async fn served() -> (std::net::SocketAddr, TestContainerGuard) {
    support::ensure_crypt();
    support::install("http://localhost");
    let container = TestContainer::fake();
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<PfxCounter>()
            .expect("register the counter")
            .build(),
    );
    let mount =
        LiveMount::<PfxCounter>::public_seed("/catalog/{section}", "counter", "catalog-counter")
            .expect("declare the mount");
    let address = support::serve(
        live_router(&mount),
        MiddlewareRegistry::new().append(ActionFacts),
    )
    .await;
    (address, container)
}

#[tokio::test]
async fn pfx_006_a_live_document_names_its_endpoint_and_assets_under_the_root() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_006_a_live_document_names_its_endpoint_and_assets_under_the_root",
    )
    .await
    {
        return;
    }
    let (address, _container) = served().await;

    let document = support::get_prefixed(address, "/catalog/books").await;
    assert_eq!(document.status, 200, "{}", document.body);
    assert_eq!(config_endpoint(&document.body), "/billing/__live/action");
    let script = attribute(&document.body, "<script type=\"module\" src");
    assert!(script.starts_with("/billing/__live/assets/"), "{script}");
    let preload = attribute(&document.body, "<link rel=\"modulepreload\" href");
    assert!(preload.starts_with("/billing/__live/assets/"), "{preload}");

    // The proxy strips the root; the framework serves what is left.
    let asset =
        support::get_prefixed(address, script.strip_prefix(PREFIX).expect("the root")).await;
    assert_eq!(asset.status, 200, "{script}");

    let at_host_root = support::get(address, "/catalog/books", &[]).await;
    assert_eq!(config_endpoint(&at_host_root.body), "/__live/action");
    let script = attribute(&at_host_root.body, "<script type=\"module\" src");
    assert!(script.starts_with("/__live/assets/"), "{script}");
}

/// The snapshot a document's island carries.
fn snapshot_of(document: &str) -> serde_json::Value {
    let encoded = attribute(document, "data-suprnova-live-snapshot");
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .expect("a base64url snapshot");
    serde_json::from_slice(&bytes).expect("a JSON snapshot")
}

async fn act(address: std::net::SocketAddr, prefixed: bool, action: &str) -> serde_json::Value {
    let path = "/catalog/books";
    let document = if prefixed {
        support::get_prefixed(address, path).await
    } else {
        support::get(address, path, &[]).await
    };
    let body = action_body(snapshot_of(&document.body), action);
    let mut headers: Vec<(&str, &[u8])> = vec![(
        "content-type",
        b"application/vnd.suprnova.live+json; charset=utf-8; version=2",
    )];
    if prefixed {
        headers.push(("x-forwarded-prefix", PREFIX.as_bytes()));
    }
    let reply = support::send(address, "POST", "/__live/action", &headers, &body).await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    reply.json()
}

#[tokio::test]
async fn pfx_006_a_reflected_url_carries_the_root() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_006_a_reflected_url_carries_the_root",
    )
    .await
    {
        return;
    }
    let (address, _container) = served().await;
    let behind = act(address, true, "reflect_query").await;
    assert_eq!(
        behind["url_intent"],
        json!({"kind": "reflected", "target": "/billing/catalog/books?q=red+shoes"})
    );
    let at_host_root = act(address, false, "reflect_query").await;
    assert_eq!(
        at_host_root["url_intent"],
        json!({"kind": "reflected", "target": "/catalog/books?q=red+shoes"})
    );
}

#[tokio::test]
async fn pfx_004_a_live_redirect_target_carries_the_root() {
    if crate::own_process_async::delegate(
        module_path!(),
        "pfx_004_a_live_redirect_target_carries_the_root",
    )
    .await
    {
        return;
    }
    let (address, _container) = served().await;
    let behind = act(address, true, "go_to_receipt").await.to_string();
    assert!(behind.contains("\"/billing/receipts/42\""), "{behind}");
    let at_host_root = act(address, false, "go_to_receipt").await.to_string();
    assert!(at_host_root.contains("\"/receipts/42\""), "{at_host_root}");
}
