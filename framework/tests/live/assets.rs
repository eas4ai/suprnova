//! Suprnova serves the exact reviewed Live artifacts and emits typed bootstrap markup.

use std::collections::BTreeMap;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::service::service_fn;
use hyper::{HeaderMap, Method, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use suprnova::container::testing::TestContainer;
use suprnova::live::assets::{ArtifactRole, LiveAssetCatalog, live_asset_catalog};
use suprnova::live::testing::prepare_live_router_for_test;
use suprnova::live::{
    CanonicalValue, EventPayloadMetadata, LiveBootstrapOptions, LiveComponent, LiveDocument,
    LiveDocumentErrorKind, LiveMount, LiveRegistry, MountFlags, MountedIsland, UploadPolicy,
    UploadReplacement, UploadScan, UploadType, live,
};
use suprnova::view::{
    AssetSet, DocumentResponseIntent, TrustedHtml, TrustedMarkupReason, ViewName,
};
use suprnova::{
    App, Crypt, EncryptionKey, HttpResponse, MiddlewareRegistry, Request, Router, handle_request,
};

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

fn ensure_crypt() {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
}

fn avatar_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(1024)
        .replacement(UploadReplacement::RetirePrevious)
        .accept(UploadType::Png)
        .scan(UploadScan::Disabled)
        .finalize_action("save_avatar")
        .build()
}

pub struct FeedUpdated;

impl EventPayloadMetadata for FeedUpdated {
    const NAME: &'static str = "feed.updated";
    const VERSION: u16 = 1;
}

#[derive(LiveComponent)]
#[live(
    name = "tests.bootstrap-counter",
    view = "live/tests/bootstrap-counter.html"
)]
pub struct BootstrapCounter {
    #[public]
    count: u64,
}

#[live]
impl BootstrapCounter {
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }
}

#[derive(LiveComponent)]
#[live(
    name = "tests.bootstrap-uploader",
    view = "live/tests/bootstrap-uploader.html"
)]
pub struct BootstrapUploader {
    #[model]
    #[upload(policy = avatar_policy)]
    avatar: String,
}

#[live]
impl BootstrapUploader {
    #[action]
    pub fn save_avatar(&mut self) {}
}

#[derive(LiveComponent)]
#[live(
    name = "tests.bootstrap-feed",
    view = "live/tests/bootstrap-feed.html",
    minimum_protocol_version = 2,
    streams(stream(name = "feed", topics("feed"), events(FeedUpdated)))
)]
pub struct BootstrapFeed {
    #[public]
    headline: String,
}

#[live]
impl BootstrapFeed {
    #[action]
    pub fn refresh(&mut self) {}
}

#[suprnova::view(path = "live/tests/bootstrap-document.html")]
struct BootstrapDocumentView<'a> {
    bootstrap: &'a TrustedHtml,
    first: &'a TrustedHtml,
    second: &'a TrustedHtml,
    third: &'a TrustedHtml,
}

fn empty_slot() -> TrustedHtml {
    TrustedHtml::framework_static(
        "",
        TrustedMarkupReason::new("unused island slot").expect("reason"),
    )
    .expect("empty markup")
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    fn text(&self) -> String {
        String::from_utf8(self.body.to_vec()).expect("UTF-8 body")
    }
}

async fn dispatch(
    router: Arc<Router>,
    method: Method,
    path: &str,
    headers: &[(&str, &str)],
) -> Reply {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept test request");
        let service = service_fn(move |request| {
            let router = Arc::clone(&router);
            async move {
                Ok::<_, std::convert::Infallible>(
                    handle_request(router, Arc::new(MiddlewareRegistry::new()), request).await,
                )
            }
        });
        let _ = hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .await;
    });
    let stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect to test listener");
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .expect("HTTP handshake");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let mut builder = hyper::Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = builder
        .body(Full::new(Bytes::new()))
        .expect("build request");
    let response = sender
        .send_request(request)
        .await
        .expect("send test request");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    Reply {
        status,
        headers,
        body,
    }
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn catalog() -> &'static LiveAssetCatalog {
    live_asset_catalog().expect("embedded artifacts validate")
}

fn asset_path(file: &str) -> String {
    format!("/__live/v1/assets/{}/{file}", catalog().identity())
}

#[tokio::test]
async fn artifact_routes_serve_exact_reviewed_bytes_with_validators() {
    ensure_crypt();
    let _container = TestContainer::fake();
    App::init();
    let router = Arc::new(Router::new().try_live().expect("install Live routes"));
    let catalog = catalog();

    for role in ArtifactRole::ALL {
        let artifact = catalog.artifact(role);
        let path = asset_path(artifact.file());
        let reply = dispatch(Arc::clone(&router), Method::GET, &path, &[]).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}");
        assert_eq!(&reply.body[..], artifact.bytes(), "{path} bytes");
        assert_eq!(
            reply.header("content-type"),
            Some("text/javascript; charset=utf-8")
        );
        assert_eq!(
            reply.header("content-length"),
            Some(artifact.bytes().len().to_string().as_str())
        );
        assert_eq!(
            reply.header("cache-control"),
            Some("public, max-age=31536000, immutable")
        );
        let etag = format!("\"{}\"", artifact.sha256_hex());
        assert_eq!(reply.header("etag"), Some(etag.as_str()));
        assert_eq!(reply.header("x-content-type-options"), Some("nosniff"));
        assert_eq!(hex(&Sha256::digest(&reply.body)), artifact.sha256_hex());

        let head = dispatch(Arc::clone(&router), Method::HEAD, &path, &[]).await;
        assert_eq!(head.status, StatusCode::OK);
        assert!(head.body.is_empty());
        assert_eq!(head.header("etag"), Some(etag.as_str()));
        assert_eq!(
            head.header("content-length"),
            Some(artifact.bytes().len().to_string().as_str())
        );
        assert_eq!(
            head.header("content-type"),
            Some("text/javascript; charset=utf-8")
        );

        let unchanged = dispatch(
            Arc::clone(&router),
            Method::GET,
            &path,
            &[("if-none-match", etag.as_str())],
        )
        .await;
        assert_eq!(unchanged.status, StatusCode::NOT_MODIFIED);
        assert!(unchanged.body.is_empty());
        assert_eq!(unchanged.header("etag"), Some(etag.as_str()));
        assert_eq!(
            unchanged.header("cache-control"),
            Some("public, max-age=31536000, immutable")
        );

        let stale = dispatch(
            Arc::clone(&router),
            Method::GET,
            &path,
            &[("if-none-match", "\"0000\"")],
        )
        .await;
        assert_eq!(stale.status, StatusCode::OK);
        assert_eq!(&stale.body[..], artifact.bytes());
    }

    let manifest = dispatch(
        Arc::clone(&router),
        Method::GET,
        &asset_path("suprnova-live.assets.json"),
        &[],
    )
    .await;
    assert_eq!(manifest.status, StatusCode::OK);
    assert_eq!(
        manifest.header("content-type"),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(&manifest.body[..], catalog.manifest_bytes());
    assert!(manifest.header("etag").is_some());
    assert_eq!(manifest.header("x-content-type-options"), Some("nosniff"));

    for boot in catalog.boot_scripts() {
        let reply = dispatch(
            Arc::clone(&router),
            Method::GET,
            &asset_path(boot.file()),
            &[],
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", boot.file());
        assert_eq!(&reply.body[..], boot.bytes());
        assert_eq!(
            reply.header("content-type"),
            Some("text/javascript; charset=utf-8")
        );
        assert_eq!(
            reply.header("cache-control"),
            Some("public, max-age=31536000, immutable")
        );
        assert_eq!(hex(&Sha256::digest(&reply.body)), boot.sha256_hex());
        assert!(!reply.text().contains("eval"));
    }
}

#[tokio::test]
async fn artifact_routes_are_a_closed_immutable_namespace() {
    ensure_crypt();
    let _container = TestContainer::fake();
    App::init();
    let router = Arc::new(Router::new().try_live().expect("install Live routes"));
    let identity = catalog().identity().to_owned();

    let rejected = [
        format!("/__live/v1/assets/{identity}/index.d.ts"),
        format!("/__live/v1/assets/{identity}/missing.js"),
        format!("/__live/v1/assets/{identity}/..%2Fsuprnova-live.esm.js"),
        format!("/__live/v1/assets/{identity}/suprnova-live.esm.js?v=1"),
        format!("/__live/v1/assets/{identity}/SUPRNOVA-LIVE.ESM.JS"),
        "/__live/v1/assets/stale-identity/suprnova-live.esm.js".to_owned(),
        "/__live/v1/assets/suprnova-live.esm.js".to_owned(),
    ];
    for path in rejected {
        let reply = dispatch(Arc::clone(&router), Method::GET, &path, &[]).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND, "{path}");
        assert!(reply.body.is_empty(), "{path} leaks a body");
        assert_eq!(reply.header("cache-control"), Some("no-store"), "{path}");
    }

    let path = asset_path("suprnova-live.esm.js");
    for method in [Method::POST, Method::PUT, Method::DELETE, Method::OPTIONS] {
        let reply = dispatch(Arc::clone(&router), method.clone(), &path, &[]).await;
        assert_eq!(reply.status, StatusCode::METHOD_NOT_ALLOWED, "{method}");
        assert_eq!(reply.header("allow"), Some("GET, HEAD"));
        assert!(reply.body.is_empty());
    }
}

fn config_json(html: &str) -> Value {
    let start = html
        .find("<script id=\"suprnova-live-config\" type=\"application/json\">")
        .expect("one configuration element");
    let rest = &html[start..];
    let open = rest.find('>').expect("open") + 1;
    let close = rest.find("</script>").expect("close");
    let text = &rest[open..close];
    assert_eq!(
        html.matches("suprnova-live-config").count(),
        1,
        "the configuration element is emitted exactly once"
    );
    serde_json::from_str(text).expect("configuration JSON")
}

fn tag_order(html: &str, needles: &[&str]) {
    let mut last = 0;
    for needle in needles {
        let position = html
            .find(needle)
            .unwrap_or_else(|| panic!("{needle} in {html}"));
        assert!(position > last, "{needle} is out of order");
        last = position;
    }
}

fn shop_router(counter_twice: bool, options: LiveBootstrapOptions) -> Router {
    App::singleton(
        LiveRegistry::builder()
            .register::<BootstrapCounter>()
            .expect("register counter")
            .register::<BootstrapUploader>()
            .expect("register uploader")
            .register::<BootstrapFeed>()
            .expect("register feed")
            .build(),
    );
    let counter = LiveMount::<BootstrapCounter>::public_seed("/shop", "counter", "shop-counter")
        .expect("declare counter");
    let counter_again =
        LiveMount::<BootstrapCounter>::public_seed("/shop", "counter-again", "shop-counter-again")
            .expect("declare second counter");
    let uploader =
        LiveMount::<BootstrapUploader>::public_seed("/shop", "uploader", "shop-uploader")
            .expect("declare uploader");
    let feed = LiveMount::<BootstrapFeed>::public_seed("/shop", "feed", "shop-feed")
        .expect("declare feed");
    let handler_counter = counter.clone();
    let handler_counter_again = counter_again.clone();
    let handler_uploader = uploader.clone();
    let handler_feed = feed.clone();
    let router: Router = Router::new()
        .get("/shop", move |request: Request| {
            let counter = handler_counter.clone();
            let counter_again = handler_counter_again.clone();
            let uploader = handler_uploader.clone();
            let feed = handler_feed.clone();
            let options = options.clone();
            async move {
                let result: Result<HttpResponse, String> = async {
                    let mut document = LiveDocument::from_request(&request)
                        .map_err(|error| format!("from_request: {error}"))?;
                    let first = document
                        .mount(
                            &counter,
                            CanonicalValue::Object(BTreeMap::new()),
                            MountFlags::empty(),
                        )
                        .await
                        .map_err(|error| format!("mount counter: {error}"))?;
                    let empty = empty_slot();
                    let mut islands: Vec<MountedIsland> = Vec::new();
                    if counter_twice {
                        islands.push(
                            document
                                .mount(
                                    &counter_again,
                                    CanonicalValue::Object(BTreeMap::new()),
                                    MountFlags::empty(),
                                )
                                .await
                                .map_err(|error| format!("mount counter again: {error}"))?,
                        );
                    } else {
                        islands.push(
                            document
                                .mount(
                                    &uploader,
                                    CanonicalValue::Object(BTreeMap::new()),
                                    MountFlags::empty(),
                                )
                                .await
                                .map_err(|error| format!("mount uploader: {error}"))?,
                        );
                        islands.push(
                            document
                                .mount(
                                    &feed,
                                    CanonicalValue::Object(BTreeMap::new()),
                                    MountFlags::empty(),
                                )
                                .await
                                .map_err(|error| format!("mount feed: {error}"))?,
                        );
                    }
                    let second = islands.first().map_or(&empty, MountedIsland::html);
                    let third = islands.get(1).map_or(&empty, MountedIsland::html);
                    let bootstrap = document
                        .bootstrap(options)
                        .map_err(|error| format!("bootstrap: {error}"))?;
                    document
                        .render(
                            ViewName::parse("live/tests/bootstrap-document.html")
                                .expect("view identity"),
                            &BootstrapDocumentView {
                                bootstrap: bootstrap.html(),
                                first: first.html(),
                                second,
                                third,
                            },
                            DocumentResponseIntent::html(StatusCode::OK).expect("response intent"),
                            AssetSet::empty(),
                        )
                        .map_err(|error| format!("render: {error}"))
                }
                .await;
                result.map_err(|error| {
                    HttpResponse::text(format!("Live document failed: {error}")).status(500)
                })
            }
        })
        .into();
    router
        .try_live()
        .expect("install Live routes")
        .try_live_mount(&counter)
        .expect("register counter")
        .try_live_mount(&counter_again)
        .expect("register second counter")
        .try_live_mount(&uploader)
        .expect("register uploader")
        .try_live_mount(&feed)
        .expect("register feed")
}

async fn render_shop(counter_twice: bool, options: LiveBootstrapOptions) -> String {
    ensure_crypt();
    let _container = TestContainer::fake();
    App::init();
    let router = shop_router(counter_twice, options);
    prepare_live_router_for_test(&router).expect("prepare runtime");
    let reply = dispatch(Arc::new(router), Method::GET, "/shop", &[]).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    reply.text()
}

#[tokio::test]
async fn esm_bootstrap_emits_config_preload_optional_roles_and_boot_in_order() {
    let html = render_shop(false, LiveBootstrapOptions::esm()).await;
    let catalog = catalog();
    let identity = catalog.identity();
    let config = config_json(&html);
    assert_eq!(
        config,
        json!({
            "asset_identity": identity,
            "credentials": "same-origin",
            "endpoint": "/__live/v1/action",
            "max_parallel_per_island": 1,
            "max_queued_per_island": 8,
            "max_response_bytes": 1_048_576,
            "protocol": { "maximum": 2, "minimum": 1 },
            "request_timeout_ms": 5_000,
            "runtime_contract_version": 1,
        })
    );
    let config_text = {
        let start = html.find("type=\"application/json\">").expect("config") + 24;
        let end = html[start..].find("</script>").expect("close") + start;
        html[start..end].to_owned()
    };
    assert_eq!(
        config_text,
        serde_json::to_string(&config).expect("canonical form"),
        "the configuration is canonical JSON with sorted keys"
    );

    let core = catalog.artifact(ArtifactRole::CoreEsm);
    let uploads = catalog.artifact(ArtifactRole::UploadsEsm);
    let async_role = catalog.artifact(ArtifactRole::AsyncEsm);
    let boot = catalog.boot_script_for(suprnova::live::LiveBootstrapStrategy::Esm, true);
    assert_eq!(boot.file(), "suprnova-live.boot.async.esm.js");
    let core_link = format!(
        "<link rel=\"modulepreload\" href=\"/__live/v1/assets/{identity}/suprnova-live.esm.js\" integrity=\"{}\" crossorigin=\"anonymous\">",
        core.sri()
    );
    let uploads_tag = format!(
        "<script type=\"module\" src=\"/__live/v1/assets/{identity}/suprnova-live.uploads.esm.js\" integrity=\"{}\" crossorigin=\"anonymous\"></script>",
        uploads.sri()
    );
    let async_tag = format!(
        "<script type=\"module\" src=\"/__live/v1/assets/{identity}/suprnova-live.async.esm.js\" integrity=\"{}\" crossorigin=\"anonymous\"></script>",
        async_role.sri()
    );
    let boot_tag = format!(
        "<script type=\"module\" src=\"/__live/v1/assets/{identity}/{}\" integrity=\"{}\" crossorigin=\"anonymous\"></script>",
        boot.file(),
        boot.sri()
    );
    tag_order(
        &html,
        &[
            "suprnova-live-config",
            &core_link,
            &uploads_tag,
            &async_tag,
            &boot_tag,
        ],
    );
    assert_eq!(html.matches("suprnova-live.esm.js").count(), 1);
    assert_eq!(html.matches(boot.file()).count(), 1);
    assert!(
        !html.contains("suprnova-live.boot.esm.js"),
        "a document with the async role boots through the configuring script"
    );
    assert!(!html.contains("stimulus"), "Stimulus loads only on request");
    assert!(
        !html.contains(".classic.js"),
        "one delivery form per document"
    );
    assert!(
        !html.contains("<script>") && !html.contains("<script type=\"module\">"),
        "no inline executable script: {html}"
    );
    assert!(html.contains("data-suprnova-live-island"));
}

#[tokio::test]
async fn repeated_islands_and_core_only_documents_do_not_duplicate_roles() {
    let html = render_shop(true, LiveBootstrapOptions::esm()).await;
    let boot = catalog().boot_script(suprnova::live::LiveBootstrapStrategy::Esm);
    assert_eq!(boot.file(), "suprnova-live.boot.esm.js");
    assert_eq!(html.matches("data-suprnova-live-island").count(), 2);
    assert_eq!(html.matches("suprnova-live.esm.js").count(), 1);
    assert_eq!(html.matches(boot.file()).count(), 1);
    assert!(!html.contains("uploads"), "no island needs the upload role");
    assert!(!html.contains("async"), "no island needs the async role");
}

#[tokio::test]
async fn classic_bootstrap_orders_optional_roles_before_core_and_boot() {
    let html = render_shop(
        false,
        LiveBootstrapOptions::classic()
            .with_stimulus()
            .with_nonce("test-nonce"),
    )
    .await;
    let catalog = catalog();
    let identity = catalog.identity();
    let sri = |role| catalog.artifact(role).sri().to_owned();
    let boot = catalog.boot_script(suprnova::live::LiveBootstrapStrategy::Classic);
    tag_order(
        &html,
        &[
            "suprnova-live-config",
            &format!(
                "<link rel=\"preload\" as=\"script\" href=\"/__live/v1/assets/{identity}/suprnova-live.classic.js\" integrity=\"{}\" crossorigin=\"anonymous\">",
                sri(ArtifactRole::CoreClassic)
            ),
            &format!(
                "<script defer src=\"/__live/v1/assets/{identity}/suprnova-live.stimulus.classic.js\" integrity=\"{}\" crossorigin=\"anonymous\" nonce=\"test-nonce\"></script>",
                sri(ArtifactRole::StimulusClassic)
            ),
            &format!(
                "<script defer src=\"/__live/v1/assets/{identity}/suprnova-live.uploads.classic.js\" integrity=\"{}\" crossorigin=\"anonymous\" nonce=\"test-nonce\"></script>",
                sri(ArtifactRole::UploadsClassic)
            ),
            &format!(
                "<script defer src=\"/__live/v1/assets/{identity}/suprnova-live.async.classic.js\" integrity=\"{}\" crossorigin=\"anonymous\" nonce=\"test-nonce\"></script>",
                sri(ArtifactRole::AsyncClassic)
            ),
            &format!(
                "<script defer src=\"/__live/v1/assets/{identity}/suprnova-live.classic.js\" integrity=\"{}\" crossorigin=\"anonymous\" nonce=\"test-nonce\"></script>",
                sri(ArtifactRole::CoreClassic)
            ),
            &format!(
                "<script defer src=\"/__live/v1/assets/{identity}/{}\" integrity=\"{}\" crossorigin=\"anonymous\" nonce=\"test-nonce\"></script>",
                boot.file(),
                boot.sri()
            ),
        ],
    );
    assert!(!html.contains(".esm.js"), "one delivery form per document");
    assert_eq!(html.matches("suprnova-live.classic.js").count(), 2);
}

#[tokio::test]
async fn bootstrap_fails_closed_on_repetition_and_late_mounts() {
    ensure_crypt();
    let _container = TestContainer::fake();
    App::init();
    App::singleton(
        LiveRegistry::builder()
            .register::<BootstrapCounter>()
            .expect("register counter")
            .build(),
    );
    let counter = LiveMount::<BootstrapCounter>::public_seed("/shop", "counter", "shop-counter")
        .expect("declare counter");
    let late = LiveMount::<BootstrapCounter>::public_seed("/shop", "late", "shop-late")
        .expect("declare late counter");
    let handler_counter = counter.clone();
    let handler_late = late.clone();
    let router: Router = Router::new()
        .get("/shop", move |request: Request| {
            let counter = handler_counter.clone();
            let late = handler_late.clone();
            async move {
                let mut document = LiveDocument::from_request(&request).map_err(|error| {
                    HttpResponse::text(format!("unprepared: {error}")).status(500)
                })?;
                document
                    .mount(
                        &counter,
                        CanonicalValue::Object(BTreeMap::new()),
                        MountFlags::empty(),
                    )
                    .await
                    .map_err(|error| HttpResponse::text(format!("mount: {error}")).status(500))?;
                let first = document.bootstrap(LiveBootstrapOptions::esm());
                let repeated = document.bootstrap(LiveBootstrapOptions::esm());
                let late_mount = document
                    .mount(
                        &late,
                        CanonicalValue::Object(BTreeMap::new()),
                        MountFlags::empty(),
                    )
                    .await;
                let report = format!(
                    "{}|{}|{}",
                    first.is_ok(),
                    repeated
                        .err()
                        .map(|error| format!("{:?}", error.kind()))
                        .unwrap_or_default(),
                    late_mount
                        .err()
                        .map(|error| format!("{:?}", error.kind()))
                        .unwrap_or_default(),
                );
                Ok::<_, HttpResponse>(HttpResponse::text(report))
            }
        })
        .into();
    let router = router
        .try_live()
        .expect("install Live routes")
        .try_live_mount(&counter)
        .expect("register counter")
        .try_live_mount(&late)
        .expect("register late counter");
    prepare_live_router_for_test(&router).expect("prepare runtime");
    let reply = dispatch(Arc::new(router), Method::GET, "/shop", &[]).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(
        reply.text(),
        format!(
            "true|{:?}|{:?}",
            LiveDocumentErrorKind::BootstrapRepeated,
            LiveDocumentErrorKind::MountAfterBootstrap
        )
    );
}

#[tokio::test]
async fn a_document_without_islands_still_boots_only_the_core() {
    ensure_crypt();
    let _container = TestContainer::fake();
    App::init();
    App::singleton(LiveRegistry::builder().build());
    let router: Router = Router::new()
        .get("/empty", move |request: Request| async move {
            let mut document = LiveDocument::from_request(&request)
                .map_err(|_| HttpResponse::text("unprepared").status(500))?;
            let bootstrap = document
                .bootstrap(LiveBootstrapOptions::esm())
                .map_err(|_| HttpResponse::text("bootstrap").status(500))?;
            let roles = bootstrap
                .roles()
                .iter()
                .map(|role| role.as_str())
                .collect::<Vec<_>>()
                .join(",");
            Ok::<_, HttpResponse>(HttpResponse::text(roles))
        })
        .into();
    let router = router.try_live().expect("install Live routes");
    let router = router
        .try_live_document("/empty")
        .expect("declare a Live document without mounts");
    prepare_live_router_for_test(&router).expect("prepare runtime");
    let reply = dispatch(Arc::new(router), Method::GET, "/empty", &[]).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert_eq!(reply.text(), "core-esm");
}
