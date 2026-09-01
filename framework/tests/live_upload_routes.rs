use std::any::Any;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Method;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use suprnova::auth::Authenticatable;
use suprnova::container::testing::TestContainer;
use suprnova::live::testing::{
    LiveSecurityCheck, inspect_configured_upload_residue_for_test,
    inspect_upload_mount_authority_for_test, prepare_live_router_for_test,
    record_live_security_pass_for_test,
};
use suprnova::live::{
    BoundedHeaders, CanonicalValue, DirectPartReference, DirectTransferInstruction, DurableUpload,
    DurableUploadId, FailedFinalize, FinalizeRequest, FinalizeToken, LiveComponent, LiveDocument,
    LiveMount, LiveRegistry, LiveUploadHost, MountFlags, PreparedFinalize, ScanDisposition,
    ScanInput, TransferMethod, TrustedProviderOrigin, TrustedProviderUrl, UnixMillis,
    UploadFinalizer, UploadFuture, UploadLimitConfig, UploadLimits, UploadPart, UploadPolicy,
    UploadReplacement, UploadScan, UploadScanFailure, UploadScanner, UploadType, live,
};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{
    App, Auth, Crypt, EncryptionKey, FrameworkError, Gate, HttpResponse, Middleware,
    MiddlewareRegistry, Next, Request, Response, Router, StatusCode, async_trait, handle_request,
};
use suprnova_live_test_support::DirectProviderConformanceAdapter;

const LIVE_UPLOAD_PATH: &str = "/__live/v1/upload";
static SAVE_ACTION_CALLS: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
struct TestUploadFinalizer {
    durable: Mutex<HashMap<String, DurableUpload>>,
    fail_commit_remaining: AtomicUsize,
    commit_calls: AtomicUsize,
}

impl TestUploadFinalizer {
    fn fail_one_commit() -> Self {
        Self {
            durable: Mutex::new(HashMap::new()),
            fail_commit_remaining: AtomicUsize::new(1),
            commit_calls: AtomicUsize::new(0),
        }
    }
}

impl UploadFinalizer for TestUploadFinalizer {
    fn prepare<'a>(
        &'a self,
        request: FinalizeRequest<'a>,
    ) -> UploadFuture<'a, Result<PreparedFinalize, suprnova::live::UploadError>> {
        Box::pin(async move {
            let token = FinalizeToken::parse(&format!(
                "test-finalize:{}",
                request.idempotency_key().as_str()
            ))?;
            Ok(PreparedFinalize::new(&request, token))
        })
    }

    fn commit<'a>(
        &'a self,
        prepared: PreparedFinalize,
    ) -> UploadFuture<'a, Result<DurableUpload, suprnova::live::UploadError>> {
        Box::pin(async move {
            self.commit_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_commit_remaining.swap(0, Ordering::SeqCst) != 0 {
                return Err(suprnova::live::UploadError::new(
                    suprnova::live::UploadErrorKind::ProviderUnavailable,
                ));
            }
            let key = prepared.token().as_str().to_owned();
            let durable = DurableUpload::new(
                &prepared,
                DurableUploadId::parse(&format!("durable:{key}"))?,
            );
            self.durable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(key, durable.clone());
            Ok(durable)
        })
    }

    fn compensate<'a>(
        &'a self,
        failed: FailedFinalize,
    ) -> UploadFuture<'a, Result<(), suprnova::live::UploadError>> {
        Box::pin(async move {
            self.durable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(failed.prepared().token().as_str());
            Ok(())
        })
    }

    fn reconcile<'a>(
        &'a self,
        request: FinalizeRequest<'a>,
    ) -> UploadFuture<'a, Result<Option<DurableUpload>, suprnova::live::UploadError>> {
        Box::pin(async move {
            Ok(self
                .durable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&format!(
                    "test-finalize:{}",
                    request.idempotency_key().as_str()
                ))
                .cloned())
        })
    }
}

struct BlockingUploadFinalizer {
    inner: TestUploadFinalizer,
    entered: tokio::sync::Semaphore,
    release: tokio::sync::Semaphore,
}

impl Default for BlockingUploadFinalizer {
    fn default() -> Self {
        Self {
            inner: TestUploadFinalizer::default(),
            entered: tokio::sync::Semaphore::new(0),
            release: tokio::sync::Semaphore::new(0),
        }
    }
}

impl BlockingUploadFinalizer {
    async fn wait_until_commit(&self) {
        self.entered
            .acquire()
            .await
            .expect("blocking finalizer remains open")
            .forget();
    }

    fn release_commit(&self) {
        self.release.add_permits(1);
    }
}

impl UploadFinalizer for BlockingUploadFinalizer {
    fn prepare<'a>(
        &'a self,
        request: FinalizeRequest<'a>,
    ) -> UploadFuture<'a, Result<PreparedFinalize, suprnova::live::UploadError>> {
        self.inner.prepare(request)
    }

    fn commit<'a>(
        &'a self,
        prepared: PreparedFinalize,
    ) -> UploadFuture<'a, Result<DurableUpload, suprnova::live::UploadError>> {
        Box::pin(async move {
            self.entered.add_permits(1);
            self.release
                .acquire()
                .await
                .expect("blocking finalizer remains open")
                .forget();
            self.inner.commit(prepared).await
        })
    }

    fn compensate<'a>(
        &'a self,
        failed: FailedFinalize,
    ) -> UploadFuture<'a, Result<(), suprnova::live::UploadError>> {
        self.inner.compensate(failed)
    }

    fn reconcile<'a>(
        &'a self,
        request: FinalizeRequest<'a>,
    ) -> UploadFuture<'a, Result<Option<DurableUpload>, suprnova::live::UploadError>> {
        self.inner.reconcile(request)
    }
}

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

#[suprnova::view(path = "live/public-document.html")]
struct UploadDocumentView<'a> {
    island: &'a TrustedHtml,
}

fn route_upload_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(1024)
        .accept(UploadType::Png)
        .scan(UploadScan::Disabled)
        .finalize_action("save_avatar")
        .build()
}

fn required_scan_upload_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(1024)
        .accept(UploadType::Png)
        .scan(UploadScan::Required {
            on_timeout: UploadScanFailure::Reject,
            on_unavailable: UploadScanFailure::Reject,
        })
        .finalize_action("save_avatar")
        .build()
}

fn retry_scan_upload_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(1024)
        .accept(UploadType::Png)
        .scan(UploadScan::Required {
            on_timeout: UploadScanFailure::Retry,
            on_unavailable: UploadScanFailure::Retry,
        })
        .finalize_action("save_avatar")
        .build()
}

struct UnavailableUploadScanner;

impl UploadScanner for UnavailableUploadScanner {
    fn scan<'a>(
        &'a self,
        _input: ScanInput<'a>,
    ) -> UploadFuture<'a, Result<ScanDisposition, suprnova::live::UploadError>> {
        Box::pin(async { Ok(ScanDisposition::Unavailable) })
    }
}

fn large_upload_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(300 * 1024)
        .accept(UploadType::Png)
        .scan(UploadScan::Disabled)
        .finalize_action("save_avatar")
        .build()
}

fn preserved_upload_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(1024)
        .replacement(UploadReplacement::PreservePrevious)
        .accept(UploadType::Png)
        .scan(UploadScan::Disabled)
        .finalize_action("save_avatar")
        .build()
}

fn aggregate_upload_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(16)
        .maximum_file_bytes(64 * 1024 * 1024)
        .replacement(UploadReplacement::PreservePrevious)
        .accept(UploadType::Png)
        .scan(UploadScan::Disabled)
        .finalize_action("save_avatar")
        .build()
}

#[derive(LiveComponent)]
#[live(
    name = "tests.upload-route-component",
    view = "live/tests/upload-policy.html"
)]
pub struct UploadRouteComponent {
    #[model]
    #[upload(policy = route_upload_policy)]
    avatar: String,
    #[model]
    #[upload(policy = required_scan_upload_policy)]
    scanned_avatar: String,
    #[model]
    #[upload(policy = retry_scan_upload_policy)]
    retry_scanned_avatar: String,
    #[model]
    #[upload(policy = large_upload_policy)]
    large_avatar: String,
    #[model]
    #[upload(policy = preserved_upload_policy)]
    preserved_avatar: String,
    #[model]
    #[upload(policy = aggregate_upload_policy)]
    aggregate_avatar: Vec<String>,
}

#[live]
impl UploadRouteComponent {
    #[mount]
    pub fn mount() -> Self {
        Self {
            avatar: String::new(),
            scanned_avatar: String::new(),
            retry_scanned_avatar: String::new(),
            large_avatar: String::new(),
            preserved_avatar: String::new(),
            aggregate_avatar: Vec::new(),
        }
    }

    #[action]
    pub fn save_avatar(&mut self) {
        SAVE_ACTION_CALLS.fetch_add(1, Ordering::SeqCst);
    }
}

struct StrictUploadFacts;

struct UploadFactsWithoutCurrentAuth;

struct CancelArrival {
    entered: Arc<tokio::sync::Semaphore>,
}

struct UploadTestPrincipal(String);

impl Authenticatable for UploadTestPrincipal {
    fn get_auth_identifier(&self) -> String {
        self.0.clone()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_arc_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

#[async_trait]
impl Middleware for StrictUploadFacts {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        let session = request
            .header("x-test-upload-session")
            .unwrap_or("upload-session")
            .to_owned();
        let principal = request
            .header("x-test-upload-principal")
            .unwrap_or("upload-principal")
            .to_owned();
        let tenant = request
            .header("x-test-upload-tenant")
            .unwrap_or("upload-tenant")
            .to_owned();
        if request.header("x-test-upload-no-auth") != Some("1") {
            Auth::set_user(Arc::new(UploadTestPrincipal(principal.clone())));
        }
        record_upload_facts_with(&mut request, &session, &principal, &tenant);
        next(request).await
    }
}

#[async_trait]
impl Middleware for UploadFactsWithoutCurrentAuth {
    async fn handle(&self, mut request: Request, next: Next) -> Response {
        record_upload_facts(&mut request);
        next(request).await
    }
}

#[async_trait]
impl Middleware for CancelArrival {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if request.header("x-test-upload-cancel-arrival") == Some("1") {
            self.entered.add_permits(1);
        }
        next(request).await
    }
}

fn record_upload_facts(request: &mut Request) {
    record_upload_facts_with(
        request,
        "upload-session",
        "upload-principal",
        "upload-tenant",
    );
}

fn record_upload_facts_with(request: &mut Request, session: &str, principal: &str, tenant: &str) {
    for (check, fact) in [
        (LiveSecurityCheck::Session, Some(session.as_bytes())),
        (LiveSecurityCheck::Origin, None),
        (LiveSecurityCheck::Csrf, None),
        (LiveSecurityCheck::Principal, Some(principal.as_bytes())),
        (LiveSecurityCheck::Tenant, Some(tenant.as_bytes())),
        (LiveSecurityCheck::RateLimit, None),
    ] {
        assert!(
            record_live_security_pass_for_test(request, check, fact),
            "upload test facts rejected"
        );
    }
}

fn ensure_crypt() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| Crypt::init(EncryptionKey::generate()));
}

async fn dispatch_one(
    router: Router,
    request: hyper::Request<Full<Bytes>>,
) -> (hyper::StatusCode, hyper::HeaderMap, Bytes) {
    dispatch_shared(
        Arc::new(router),
        Arc::new(MiddlewareRegistry::new()),
        request,
    )
    .await
}

async fn dispatch_shared(
    router: Arc<Router>,
    middleware: Arc<MiddlewareRegistry>,
    request: hyper::Request<Full<Bytes>>,
) -> (hyper::StatusCode, hyper::HeaderMap, Bytes) {
    ensure_crypt();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept test request");
        let service = service_fn(move |request| {
            let router = Arc::clone(&router);
            let middleware = Arc::clone(&middleware);
            async move {
                Ok::<_, std::convert::Infallible>(handle_request(router, middleware, request).await)
            }
        });
        let _ = hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .await;
    });

    let stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect test request");
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .expect("HTTP handshake");
    tokio::spawn(async move {
        let _ = connection.await;
    });
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
        .expect("collect response body")
        .to_bytes();
    (status, headers, body)
}

fn semantic_router_and_runtime() -> (Arc<Router>, suprnova::live::LiveRuntime) {
    semantic_router_and_runtime_with_host(
        LiveUploadHost::new().with_finalizer(Arc::new(TestUploadFinalizer::default())),
    )
}

fn semantic_router_and_runtime_with_host(
    upload_host: LiveUploadHost,
) -> (Arc<Router>, suprnova::live::LiveRuntime) {
    App::init();
    App::singleton(upload_host);
    App::singleton(
        LiveRegistry::builder()
            .register::<UploadRouteComponent>()
            .expect("register route upload component")
            .build(),
    );
    for field in [
        "avatar",
        "scanned_avatar",
        "retry_scanned_avatar",
        "large_avatar",
        "preserved_avatar",
        "aggregate_avatar",
    ] {
        for control in [
            "Create",
            "Reacquire",
            "Status",
            "Queue",
            "BeginTransfer",
            "PutChunk",
            "Complete",
            "Accept",
            "BeginFinalize",
            "CommitFinalize",
            "Cancel",
            "Reject",
            "Expire",
            "Fail",
        ] {
            Gate::define::<String, String>(
                &format!("live:tests.upload-route-component.upload.{field}.{control}"),
                |_, _| true,
            );
        }
    }
    let mount = LiveMount::<UploadRouteComponent>::public_seed(
        "/upload-fixture",
        "avatar-slot",
        "avatar-document",
    )
    .expect("declare route upload mount");
    let handler_mount = mount.clone();
    let router: Router = Router::new()
        .get("/upload-fixture", move |request: Request| {
            let mount = handler_mount.clone();
            async move {
                let result: Result<HttpResponse, FrameworkError> = async {
                    let mut document = LiveDocument::from_request(&request)?;
                    let island = document
                        .mount(
                            &mount,
                            CanonicalValue::Object(BTreeMap::new()),
                            MountFlags::empty(),
                        )
                        .await?;
                    document
                        .render(
                            ViewName::parse("live/public-document.html")
                                .map_err(|_| FrameworkError::internal("test view identity"))?,
                            &UploadDocumentView {
                                island: island.html(),
                            },
                            DocumentResponseIntent::html(StatusCode::OK)
                                .map_err(|_| FrameworkError::internal("test response intent"))?,
                            AssetSet::empty(),
                        )
                        .map_err(FrameworkError::from)
                }
                .await;
                result.map_err(|_| HttpResponse::text("Live document failed").status(500))
            }
        })
        .into();
    let router: Router = router
        .try_live()
        .expect("install Live routes")
        .try_live_upload_reacquisition("/account/uploads/{handle}/reacquire")
        .expect("register upload reacquisition route")
        .middleware(StrictUploadFacts)
        .into();
    let router = router
        .try_live_mount(&mount)
        .expect("register route upload mount");
    let runtime = prepare_live_router_for_test(&router).expect("prepare route upload runtime");
    (Arc::new(router), runtime)
}

fn html_attribute<'a>(document: &'a str, name: &str) -> &'a str {
    let marker = format!("{name}=\"");
    let start = document.find(&marker).expect("Live attribute") + marker.len();
    let rest = &document[start..];
    let end = rest.find('"').expect("Live attribute terminator");
    &rest[..end]
}

fn upload_action_request(
    seed: &Value,
    handle: &str,
    browser_nonce: &str,
    correlation_id: &str,
    idempotency_key: &str,
) -> hyper::Request<Full<Bytes>> {
    let body = serde_json::to_vec(&json!({
        "base_revision": "0",
        "component": "tests.upload-route-component",
        "correlation_id": correlation_id,
        "extensions": {
            "x_suprnova_live_document_key_v1": "avatar-document",
        },
        "idempotency_key": idempotency_key,
        "model_proposals": {"avatar": handle},
        "operations": [
            {"field": "avatar", "kind": "sync_model"},
            {"arguments": {}, "kind": "invoke_action", "name": "save_avatar"}
        ],
        "protocol_version": 1,
        "runtime_contract_version": 1,
        "snapshot": {
            "browser_nonce": browser_nonce,
            "envelope": seed,
            "kind": "seed_promotion",
        },
        "snapshot_schema_version": 1,
    }))
    .expect("encode upload action request");
    hyper::Request::builder()
        .method(Method::POST)
        .uri("/__live/v1/action")
        .header(
            "content-type",
            "application/vnd.suprnova.live+json; charset=utf-8; version=1",
        )
        .body(Full::new(Bytes::from(body)))
        .expect("build upload action request")
}

fn semantic_router() -> Arc<Router> {
    semantic_router_and_runtime().0
}

fn control_request(value: Value, grant: Option<&str>) -> hyper::Request<Full<Bytes>> {
    let mut builder = hyper::Request::builder()
        .method(Method::POST)
        .uri(LIVE_UPLOAD_PATH)
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .header("x-suprnova-live", "upload-v1");
    if let Some(grant) = grant {
        builder = builder.header("authorization", format!("SuprnovaUpload {grant}"));
    }
    builder
        .body(Full::new(Bytes::from(
            serde_json::to_vec(&value).expect("encode upload control"),
        )))
        .expect("build upload control request")
}

#[allow(
    clippy::too_many_arguments,
    reason = "the upload data request keeps every conditional transfer fact explicit"
)]
fn chunk_request(
    handle: &str,
    grant: &str,
    expected_revision: u64,
    idempotency_key: &str,
    chunk_index: u32,
    offset: u64,
    bytes: Bytes,
    checksum: &str,
) -> hyper::Request<Full<Bytes>> {
    hyper::Request::builder()
        .method(Method::POST)
        .uri(LIVE_UPLOAD_PATH)
        .header("authorization", format!("SuprnovaUpload {grant}"))
        .header("content-type", "application/octet-stream")
        .header("x-suprnova-live", "upload-v1")
        .header("x-suprnova-upload-checksum", checksum)
        .header("x-suprnova-upload-chunk", chunk_index)
        .header("x-suprnova-upload-handle", handle)
        .header("x-suprnova-upload-idempotency", idempotency_key)
        .header("x-suprnova-upload-offset", offset)
        .header("x-suprnova-upload-operation", "put_chunk")
        .header("x-suprnova-upload-revision", expected_revision)
        .body(Full::new(bytes))
        .expect("build upload chunk request")
}

async fn send_control(
    router: Arc<Router>,
    middleware: Arc<MiddlewareRegistry>,
    value: Value,
    grant: Option<&str>,
) -> (hyper::StatusCode, hyper::HeaderMap, Value) {
    let (status, headers, body) =
        dispatch_shared(router, middleware, control_request(value, grant)).await;
    let value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, headers, value)
}

async fn create_ready_avatar(
    router: Arc<Router>,
    middleware: Arc<MiddlewareRegistry>,
    id: &str,
) -> (String, String, String) {
    let bytes =
        Bytes::from_static(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01");
    let checksum = hex::encode(Sha256::digest(&bytes));
    let (status, _, created) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        json!({
            "field": "avatar",
            "file": {
                "lastModified": 1,
                "name": format!("{id}.png"),
                "size": bytes.len(),
                "type": "image/png"
            },
            "idempotency_key": format!("create-{id}"),
            "island": {
                "component": "tests.upload-route-component",
                "documentKey": "avatar-document",
                "slot": "avatar-slot"
            },
            "operation": "create",
            "protocol_version": 1
        }),
        None,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::CREATED, "{created}");
    let handle = created["handle"]
        .as_str()
        .expect("ready upload handle")
        .to_owned();
    let grant = created["grant"]
        .as_str()
        .expect("ready upload grant")
        .to_owned();
    let request = chunk_request(
        &handle,
        &grant,
        1,
        &format!("put-{id}-0"),
        0,
        0,
        bytes,
        &checksum,
    );
    let (status, _, body) =
        dispatch_shared(Arc::clone(&router), Arc::clone(&middleware), request).await;
    let chunk: Value = serde_json::from_slice(&body).expect("ready chunk response");
    assert_eq!(status, hyper::StatusCode::OK, "{chunk}");
    assert_eq!(chunk["revision"], "4");
    let (status, _, completed) = send_control(
        router,
        middleware,
        json!({
            "expected_revision": "4",
            "handle": handle,
            "idempotency_key": format!("complete-{id}"),
            "operation": "complete",
            "protocol_version": 1,
            "whole_checksum": checksum
        }),
        Some(&grant),
    )
    .await;
    assert_eq!(status, hyper::StatusCode::OK, "{completed}");
    assert_eq!(completed["state"], "ready");
    assert_eq!(completed["revision"], "6");
    (handle, grant, checksum)
}

async fn send_reacquire(
    router: Arc<Router>,
    handle: &str,
) -> (hyper::StatusCode, hyper::HeaderMap, Value) {
    send_reacquire_as(router, handle, handle, None, None, None, false).await
}

#[allow(
    clippy::too_many_arguments,
    reason = "the hostile route helper keeps each independently bound request fact explicit"
)]
async fn send_reacquire_as(
    router: Arc<Router>,
    route_handle: &str,
    body_handle: &str,
    session: Option<&str>,
    principal: Option<&str>,
    tenant: Option<&str>,
    no_auth: bool,
) -> (hyper::StatusCode, hyper::HeaderMap, Value) {
    let mut builder = hyper::Request::builder()
        .method(Method::POST)
        .uri(format!("/account/uploads/{route_handle}/reacquire"))
        .header("content-type", "application/json")
        .header("accept", "application/json");
    if let Some(session) = session {
        builder = builder.header("x-test-upload-session", session);
    }
    if let Some(principal) = principal {
        builder = builder.header("x-test-upload-principal", principal);
    }
    if let Some(tenant) = tenant {
        builder = builder.header("x-test-upload-tenant", tenant);
    }
    if no_auth {
        builder = builder.header("x-test-upload-no-auth", "1");
    }
    let request = builder
        .body(Full::new(Bytes::from(
            serde_json::to_vec(&json!({
                "handle": body_handle,
                "operation": "reacquire",
                "protocol_version": 1
            }))
            .expect("encode upload reacquisition"),
        )))
        .expect("build upload reacquisition request");
    let (status, headers, body) =
        dispatch_shared(router, Arc::new(MiddlewareRegistry::new()), request).await;
    let value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, headers, value)
}

#[test]
fn live_installation_registers_the_upload_control_and_data_route() {
    let router = Router::new()
        .try_live()
        .expect("install the complete Live route namespace");

    assert!(
        router
            .match_route(&Method::POST, LIVE_UPLOAD_PATH)
            .is_some(),
        "the browser's fixed upload endpoint must be part of the atomic Live installation"
    );
    assert!(
        router.match_route(&Method::GET, LIVE_UPLOAD_PATH).is_some(),
        "non-POST upload requests must reach the closed typed method response"
    );
}

#[test]
fn upload_reacquisition_is_an_explicit_application_route_outside_the_reserved_namespace() {
    let router: Router = Router::new()
        .try_live()
        .expect("install the reserved Live namespace")
        .try_live_upload_reacquisition("/account/uploads/{handle}/reacquire")
        .expect("register the explicit application reacquisition route")
        .middleware(StrictUploadFacts)
        .into();

    assert!(
        router
            .match_route(
                &Method::POST,
                "/account/uploads/00000000-0000-4000-8000-000000000000/reacquire",
            )
            .is_some(),
        "the application-selected route must be registered explicitly",
    );
    assert!(
        router
            .match_route(
                &Method::POST,
                "/__live/v1/uploads/00000000-0000-4000-8000-000000000000/reacquire",
            )
            .is_none(),
        "Live installation must not invent a reserved reacquisition route",
    );
    assert!(
        Router::new()
            .match_route(
                &Method::POST,
                "/account/uploads/00000000-0000-4000-8000-000000000000/reacquire",
            )
            .is_none(),
        "the application route must not exist until the developer registers it",
    );
    assert!(
        router
            .match_route(
                &Method::GET,
                "/account/uploads/00000000-0000-4000-8000-000000000000/reacquire",
            )
            .is_none(),
        "reacquisition is POST-only",
    );
    assert!(
        Router::new()
            .try_live_upload_reacquisition("/__live/v1/uploads/{handle}/reacquire")
            .is_err(),
        "an application reacquisition route cannot enter the reserved namespace",
    );
    assert!(
        Router::new()
            .try_live_upload_reacquisition("/account/uploads/reacquire")
            .is_err(),
        "the route must expose exactly one opaque handle parameter",
    );
    let duplicate: Router = Router::new()
        .try_live_upload_reacquisition("/account/uploads/{handle}/reacquire")
        .expect("register first reacquisition route")
        .into();
    assert!(
        duplicate
            .try_live_upload_reacquisition("/account/uploads/{handle}/reacquire")
            .is_err(),
        "duplicate reacquisition routes must preserve normal route conflicts",
    );
}

#[tokio::test]
async fn non_post_upload_requests_return_a_closed_empty_method_response() {
    let request = hyper::Request::builder()
        .method(Method::GET)
        .uri(LIVE_UPLOAD_PATH)
        .body(Full::new(Bytes::new()))
        .expect("build request");
    let (status, headers, body) = dispatch_one(
        Router::new().try_live().expect("install Live routes"),
        request,
    )
    .await;

    assert_eq!(status, hyper::StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(headers.get("allow").expect("Allow header"), "POST");
    assert_eq!(
        headers.get("cache-control").expect("cache policy"),
        "no-store"
    );
    assert_eq!(headers.get("content-length").expect("body length"), "0");
    assert!(body.is_empty());
}

#[tokio::test]
async fn upload_control_rejects_the_wrong_media_before_runtime_or_provider_work() {
    let request = hyper::Request::builder()
        .method(Method::POST)
        .uri(LIVE_UPLOAD_PATH)
        .header("content-type", "text/plain")
        .header("x-suprnova-live", "upload-v1")
        .body(Full::new(Bytes::from_static(b"{}")))
        .expect("build request");
    let (status, headers, body) = dispatch_one(
        Router::new().try_live().expect("install Live routes"),
        request,
    )
    .await;

    assert_eq!(status, hyper::StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(
        headers.get("cache-control").expect("cache policy"),
        "no-store"
    );
    assert_eq!(headers.get("content-length").expect("body length"), "0");
    assert!(body.is_empty());
}

#[tokio::test]
async fn upload_control_enforces_its_sixteen_kibibyte_body_cap() {
    let request = hyper::Request::builder()
        .method(Method::POST)
        .uri(LIVE_UPLOAD_PATH)
        .header("content-type", "application/json")
        .header("x-suprnova-live", "upload-v1")
        .body(Full::new(Bytes::from(vec![b'x'; 16 * 1024 + 1])))
        .expect("build request");
    let (status, headers, body) = dispatch_one(
        Router::new().try_live().expect("install Live routes"),
        request,
    )
    .await;

    assert_eq!(status, hyper::StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(headers.get("content-length").expect("body length"), "0");
    assert!(body.is_empty());
}

#[tokio::test]
async fn malformed_upload_control_json_fails_before_runtime_or_provider_work() {
    let request = hyper::Request::builder()
        .method(Method::POST)
        .uri(LIVE_UPLOAD_PATH)
        .header("content-type", "application/json")
        .header("x-suprnova-live", "upload-v1")
        .body(Full::new(Bytes::from_static(b"{")))
        .expect("build request");
    let (status, headers, body) = dispatch_one(
        Router::new().try_live().expect("install Live routes"),
        request,
    )
    .await;

    assert_eq!(status, hyper::StatusCode::BAD_REQUEST);
    assert_eq!(headers.get("content-length").expect("body length"), "0");
    assert!(body.is_empty());
}

#[tokio::test]
#[serial_test::serial]
async fn upload_control_and_data_route_drives_the_engine_state_machine() {
    ensure_crypt();
    let _container = TestContainer::fake();
    let (router, runtime) = semantic_router_and_runtime();
    let middleware = Arc::new(MiddlewareRegistry::new().append(StrictUploadFacts));
    let bytes =
        Bytes::from_static(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01");
    let checksum = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();

    let (status, headers, created) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        json!({
            "field": "avatar",
            "file": {
                "lastModified": 1,
                "name": "avatar.png",
                "size": bytes.len(),
                "type": "image/png"
            },
            "idempotency_key": "create-avatar",
            "island": {
                "component": "tests.upload-route-component",
                "documentKey": "avatar-document",
                "slot": "avatar-slot"
            },
            "operation": "create",
            "protocol_version": 1
        }),
        None,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::CREATED, "{created}");
    assert_eq!(
        headers.get("cache-control").expect("cache policy"),
        "no-store"
    );
    let handle = created["handle"].as_str().expect("created handle");
    let grant = created["grant"].as_str().expect("created grant");
    assert_eq!(created["state"], "queued");
    assert_eq!(created["revision"], "1");

    let wrong_chunk_request = hyper::Request::builder()
        .method(Method::POST)
        .uri(LIVE_UPLOAD_PATH)
        .header("authorization", format!("SuprnovaUpload {grant}"))
        .header("content-type", "application/octet-stream")
        .header("x-suprnova-live", "upload-v1")
        .header("x-suprnova-upload-checksum", &checksum)
        .header("x-suprnova-upload-chunk", "1")
        .header("x-suprnova-upload-offset", "0")
        .header("x-suprnova-upload-handle", handle)
        .header("x-suprnova-upload-idempotency", "put-avatar-wrong-index")
        .header("x-suprnova-upload-operation", "put_chunk")
        .header("x-suprnova-upload-revision", "1")
        .body(Full::new(bytes.clone()))
        .expect("build out-of-order upload chunk request");
    let (status, _, wrong_chunk_body) = dispatch_shared(
        Arc::clone(&router),
        Arc::clone(&middleware),
        wrong_chunk_request,
    )
    .await;
    assert_eq!(
        status,
        hyper::StatusCode::CONFLICT,
        "{}",
        String::from_utf8_lossy(&wrong_chunk_body),
    );
    let wrong_checksum_request = hyper::Request::builder()
        .method(Method::POST)
        .uri(LIVE_UPLOAD_PATH)
        .header("authorization", format!("SuprnovaUpload {grant}"))
        .header("content-type", "application/octet-stream")
        .header("x-suprnova-live", "upload-v1")
        .header("x-suprnova-upload-checksum", "0".repeat(64))
        .header("x-suprnova-upload-chunk", "0")
        .header("x-suprnova-upload-offset", "0")
        .header("x-suprnova-upload-handle", handle)
        .header("x-suprnova-upload-idempotency", "put-avatar-wrong-checksum")
        .header("x-suprnova-upload-operation", "put_chunk")
        .header("x-suprnova-upload-revision", "1")
        .body(Full::new(bytes.clone()))
        .expect("build checksum-mismatched upload chunk request");
    let (status, _, wrong_checksum_body) = dispatch_shared(
        Arc::clone(&router),
        Arc::clone(&middleware),
        wrong_checksum_request,
    )
    .await;
    assert_eq!(
        status,
        hyper::StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        String::from_utf8_lossy(&wrong_checksum_body),
    );
    let (status, _, unchanged) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        json!({
            "handle": handle,
            "operation": "status",
            "protocol_version": 1
        }),
        Some(grant),
    )
    .await;
    assert_eq!(status, hyper::StatusCode::OK, "{unchanged}");
    assert_eq!(unchanged["state"], "queued");
    assert_eq!(unchanged["revision"], "1");

    let chunk_request = hyper::Request::builder()
        .method(Method::POST)
        .uri(LIVE_UPLOAD_PATH)
        .header("authorization", format!("SuprnovaUpload {grant}"))
        .header("content-type", "application/octet-stream")
        .header("x-suprnova-live", "upload-v1")
        .header("x-suprnova-upload-checksum", &checksum)
        .header("x-suprnova-upload-chunk", "0")
        .header("x-suprnova-upload-offset", "0")
        .header("x-suprnova-upload-handle", handle)
        .header("x-suprnova-upload-idempotency", "put-avatar-0")
        .header("x-suprnova-upload-operation", "put_chunk")
        .header("x-suprnova-upload-revision", "1")
        .body(Full::new(bytes.clone()))
        .expect("build upload chunk request");
    let (status, _, chunk_body) =
        dispatch_shared(Arc::clone(&router), Arc::clone(&middleware), chunk_request).await;
    let chunk: Value = serde_json::from_slice(&chunk_body).expect("chunk response JSON");
    assert_eq!(status, hyper::StatusCode::OK, "{chunk}");
    assert_eq!(chunk["state"], "transferring");
    assert_eq!(chunk["revision"], "4");
    assert!(
        chunk.get("nextChunkIndex").is_none(),
        "chunk responses must retain the production browser response shape"
    );

    let other_handle = "00000000-0000-4000-8000-000000000001";
    for (label, result) in [
        (
            "route/body substitution",
            send_reacquire_as(
                Arc::clone(&router),
                handle,
                other_handle,
                None,
                None,
                None,
                false,
            )
            .await,
        ),
        (
            "cross-session",
            send_reacquire_as(
                Arc::clone(&router),
                handle,
                handle,
                Some("other-session"),
                None,
                None,
                false,
            )
            .await,
        ),
        (
            "cross-principal",
            send_reacquire_as(
                Arc::clone(&router),
                handle,
                handle,
                None,
                Some("other-principal"),
                None,
                false,
            )
            .await,
        ),
        (
            "cross-tenant",
            send_reacquire_as(
                Arc::clone(&router),
                handle,
                handle,
                None,
                None,
                Some("other-tenant"),
                false,
            )
            .await,
        ),
        (
            "missing current authentication",
            send_reacquire_as(Arc::clone(&router), handle, handle, None, None, None, true).await,
        ),
    ] {
        let (status, _, body) = result;
        assert_eq!(status, hyper::StatusCode::FORBIDDEN, "{label}: {body}");
        let encoded = body.to_string();
        assert!(
            !encoded.contains(handle),
            "{label} exposed the upload handle"
        );
        assert!(
            !encoded.contains(grant),
            "{label} exposed the transfer grant"
        );
        assert_eq!(body.as_object().map(serde_json::Map::len), Some(1));
    }

    let (status, headers, reacquired) = send_reacquire(Arc::clone(&router), handle).await;
    assert_eq!(status, hyper::StatusCode::OK, "{reacquired}");
    assert_eq!(
        headers.get("cache-control").expect("cache policy"),
        "no-store"
    );
    assert_eq!(reacquired["fileIdentity"]["lastModified"], 1);
    assert_eq!(reacquired["fileIdentity"]["name"], "avatar.png");
    assert_eq!(reacquired["fileIdentity"]["size"], bytes.len());
    assert_eq!(reacquired["fileIdentity"]["type"], "image/png");
    assert_eq!(reacquired["uploadedBytes"], bytes.len());
    assert_eq!(reacquired["nextChunkIndex"], 1);
    assert_eq!(reacquired["revision"], "4");
    assert_eq!(reacquired["state"], "transferring");
    let grant = reacquired["grant"]
        .as_str()
        .expect("reacquired transfer grant");

    let (status, _, inspected) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        json!({
            "handle": handle,
            "operation": "status",
            "protocol_version": 1
        }),
        Some(grant),
    )
    .await;
    assert_eq!(status, hyper::StatusCode::OK, "{inspected}");
    assert_eq!(inspected["revision"], "4");
    assert_eq!(inspected["nextChunkIndex"], 1);

    let (status, _, completed) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        json!({
            "expected_revision": "4",
            "handle": handle,
            "idempotency_key": "complete-avatar",
            "operation": "complete",
            "protocol_version": 1,
            "whole_checksum": checksum
        }),
        Some(grant),
    )
    .await;
    assert_eq!(status, hyper::StatusCode::OK, "{completed}");
    assert_eq!(completed["state"], "ready");
    assert_eq!(completed["revision"], "6");

    let (status, _, replayed_completion) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        json!({
            "expected_revision": "4",
            "handle": handle,
            "idempotency_key": "complete-avatar",
            "operation": "complete",
            "protocol_version": 1,
            "whole_checksum": checksum
        }),
        Some(grant),
    )
    .await;
    assert_eq!(status, hyper::StatusCode::OK, "{replayed_completion}");
    assert_eq!(replayed_completion["state"], "ready");
    assert_eq!(replayed_completion["revision"], "6");

    let document_request = hyper::Request::builder()
        .uri("/upload-fixture")
        .body(Full::new(Bytes::new()))
        .expect("build upload document request");
    let (status, _, document_body) = dispatch_shared(
        Arc::clone(&router),
        Arc::clone(&middleware),
        document_request,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::OK);
    let document = std::str::from_utf8(&document_body).expect("upload document UTF-8");
    let encoded_seed = html_attribute(document, "data-suprnova-live-snapshot");
    let seed = URL_SAFE_NO_PAD
        .decode(encoded_seed)
        .expect("decode upload seed snapshot");
    let seed: Value = serde_json::from_slice(&seed).expect("parse upload seed snapshot");

    SAVE_ACTION_CALLS.store(0, Ordering::SeqCst);
    let forged = upload_action_request(
        &seed,
        "00000000-0000-4000-8000-000000000001",
        "ICEiIyQlJicoKSorLC0uLw",
        "AAECAwQFBgcICQoLDA0ODw",
        "EBESExQVFhcYGRobHB0eHw",
    );
    let (status, _, forged_body) =
        dispatch_shared(Arc::clone(&router), Arc::clone(&middleware), forged).await;
    assert_ne!(
        status,
        hyper::StatusCode::OK,
        "a syntactically valid but unready handle reached the action: {}",
        String::from_utf8_lossy(&forged_body),
    );
    assert_eq!(
        SAVE_ACTION_CALLS.load(Ordering::SeqCst),
        0,
        "upload proposal authority must reject before the action body",
    );

    let ready = upload_action_request(
        &seed,
        handle,
        "MDEyMzQ1Njc4OTo7PD0-Pw",
        "QEFCQ0RFRkdISUpLTE1OTw",
        "ICEiIyQlJicoKSorLC0uLw",
    );
    let (status, _, ready_body) =
        dispatch_shared(Arc::clone(&router), Arc::clone(&middleware), ready).await;
    assert_eq!(
        status,
        hyper::StatusCode::OK,
        "the exact Ready handle was rejected: {}",
        String::from_utf8_lossy(&ready_body),
    );
    assert_eq!(SAVE_ACTION_CALLS.load(Ordering::SeqCst), 1);

    let (status, _, finalized) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        json!({
            "handle": handle,
            "operation": "status",
            "protocol_version": 1
        }),
        Some(grant),
    )
    .await;
    assert_eq!(status, hyper::StatusCode::OK, "{finalized}");
    assert_eq!(finalized["state"], "finalized");
    assert_eq!(finalized["revision"], "8");

    let (status, _, terminal) = send_reacquire(Arc::clone(&router), handle).await;
    assert_eq!(status, hyper::StatusCode::BAD_REQUEST, "{terminal}");
    let encoded = terminal.to_string();
    assert!(!encoded.contains(handle));
    assert!(!encoded.contains(grant));
    assert_eq!(terminal.as_object().map(serde_json::Map::len), Some(1));

    let (status, _, cancel_created) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        json!({
            "field": "avatar",
            "file": {
                "lastModified": 2,
                "name": "cancel.png",
                "size": bytes.len(),
                "type": "image/png"
            },
            "idempotency_key": "create-cancel",
            "island": {
                "component": "tests.upload-route-component",
                "documentKey": "avatar-document",
                "slot": "avatar-slot"
            },
            "operation": "create",
            "protocol_version": 1
        }),
        None,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::CREATED, "{cancel_created}");
    let cancel_handle = cancel_created["handle"].as_str().expect("cancel handle");
    let cancel_grant = cancel_created["grant"].as_str().expect("cancel grant");
    let (status, _, canceled) = send_control(
        router,
        middleware,
        json!({
            "expected_revision": "1",
            "handle": cancel_handle,
            "idempotency_key": "cancel-avatar",
            "operation": "cancel",
            "protocol_version": 1
        }),
        Some(cancel_grant),
    )
    .await;
    assert_eq!(status, hyper::StatusCode::OK, "{canceled}");
    assert_eq!(canceled["state"], "canceled");
    assert_eq!(canceled["revision"], "2");
    let authority = inspect_upload_mount_authority_for_test(
        &runtime,
        "tests.upload-route-component",
        "avatar-slot",
        "avatar-document",
        Some(b"upload-session"),
        Some(b"upload-principal"),
        Some(b"upload-tenant"),
    )
    .expect("derive canceled upload authority");
    let cleanup = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let residue = inspect_configured_upload_residue_for_test(
                &runtime,
                &authority,
                "avatar",
                "create-cancel",
            )
            .await
            .expect("inspect automatic upload cleanup");
            if residue.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        cleanup.is_ok(),
        "production cleanup runner did not reclaim residue"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn configured_direct_provider_issues_reports_replays_and_validates_constrained_parts() {
    ensure_crypt();
    let _container = TestContainer::fake();
    let origin = TrustedProviderOrigin::parse("https://uploads.example.test")
        .expect("trusted direct origin");
    let direct = Arc::new(
        DirectProviderConformanceAdapter::new(
            UploadLimits::new(UploadLimitConfig::reference()).expect("reference upload limits"),
            origin.clone(),
        )
        .expect("direct conformance provider"),
    );
    let upload_host = LiveUploadHost::new()
        .with_direct_provider(direct.clone())
        .with_finalizer(Arc::new(TestUploadFinalizer::default()));
    let (router, _) = semantic_router_and_runtime_with_host(upload_host);
    let middleware = Arc::new(MiddlewareRegistry::new().append(StrictUploadFacts));
    let bytes =
        Bytes::from_static(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01");
    let checksum = hex::encode(Sha256::digest(&bytes));

    let (status, _, created) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        json!({
            "field": "avatar",
            "file": {
                "lastModified": 1,
                "name": "direct-avatar.png",
                "size": bytes.len(),
                "type": "image/png"
            },
            "idempotency_key": "create-direct-avatar",
            "island": {
                "component": "tests.upload-route-component",
                "documentKey": "avatar-document",
                "slot": "avatar-slot"
            },
            "mode": "direct",
            "operation": "create",
            "protocol_version": 1
        }),
        None,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::CREATED, "{created}");
    assert_eq!(created["mode"], "direct");
    let handle = created["handle"].as_str().expect("direct handle");
    let grant = created["grant"].as_str().expect("direct grant");
    let wire = created["instruction"]
        .as_object()
        .expect("one direct instruction");
    assert_eq!(wire["method"], "PUT");
    assert!(wire.get("credentials").is_none());

    let headers = wire["headers"]
        .as_object()
        .expect("direct headers")
        .iter()
        .map(|(name, value)| {
            (
                name.clone(),
                value.as_str().expect("direct header value").to_owned(),
            )
        })
        .collect::<Vec<_>>();
    let header_refs = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let expires_at = UnixMillis::new(wire["expires_at"].as_u64().expect("instruction expiry"));
    let instruction = DirectTransferInstruction::new(
        TransferMethod::Put,
        TrustedProviderUrl::parse(wire["url"].as_str().expect("direct URL"), &origin)
            .expect("trusted direct URL"),
        BoundedHeaders::parse(&header_refs).expect("bounded direct headers"),
        UploadPart::new(
            u32::try_from(wire["part"].as_u64().expect("direct part")).expect("u32 part"),
            wire["offset"].as_u64().expect("direct offset"),
            wire["maximum_bytes"]
                .as_u64()
                .expect("direct maximum bytes"),
        )
        .expect("direct part range"),
        DirectPartReference::parse(wire["reference"].as_str().expect("direct reference"))
            .expect("direct reference"),
        UnixMillis::new(expires_at.get().saturating_sub(1)),
        expires_at,
        usize::try_from(
            wire["maximum_bytes"]
                .as_u64()
                .expect("direct maximum bytes"),
        )
        .expect("direct byte ceiling"),
    )
    .expect("reconstruct constrained direct instruction");
    direct
        .store_part_for_test(
            &instruction,
            &bytes,
            UnixMillis::new(expires_at.get().saturating_sub(1)),
        )
        .expect("provider stores exact instructed part");

    let report = json!({
        "expected_revision": "1",
        "handle": handle,
        "idempotency_key": "report-direct-avatar-0",
        "operation": "report_direct_part",
        "part": 0,
        "protocol_version": 1,
        "reference": wire["reference"]
    });
    let (status, _, reported) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        report.clone(),
        Some(grant),
    )
    .await;
    assert_eq!(status, hyper::StatusCode::OK, "{reported}");
    assert_eq!(reported["uploadedBytes"], bytes.len());
    assert_eq!(reported["nextChunkIndex"], 1);
    assert_eq!(reported["revision"], "4");

    let (status, _, replayed) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        report,
        Some(grant),
    )
    .await;
    assert_eq!(status, hyper::StatusCode::OK, "{replayed}");
    assert_eq!(replayed["revision"], "4");

    let (status, _, completed) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        json!({
            "expected_revision": "4",
            "handle": handle,
            "idempotency_key": "complete-direct-avatar",
            "operation": "complete",
            "protocol_version": 1,
            "whole_checksum": checksum
        }),
        Some(grant),
    )
    .await;
    assert_eq!(status, hyper::StatusCode::OK, "{completed}");
    assert_eq!(completed["state"], "ready");
    assert_eq!(completed["revision"], "6");

    let (status, _, cancel_created) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        json!({
            "field": "avatar",
            "file": {
                "lastModified": 2,
                "name": "direct-cancel.png",
                "size": 8,
                "type": "image/png"
            },
            "idempotency_key": "create-direct-cancel",
            "island": {
                "component": "tests.upload-route-component",
                "documentKey": "avatar-document",
                "slot": "avatar-slot"
            },
            "mode": "direct",
            "operation": "create",
            "protocol_version": 1
        }),
        None,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::CREATED, "{cancel_created}");
    let cancel_handle = cancel_created["handle"]
        .as_str()
        .expect("cancel direct handle");
    let cancel_grant = cancel_created["grant"]
        .as_str()
        .expect("cancel direct grant");
    let (status, _, canceled) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        json!({
            "expected_revision": "1",
            "handle": cancel_handle,
            "idempotency_key": "cancel-direct-upload",
            "operation": "cancel",
            "protocol_version": 1
        }),
        Some(cancel_grant),
    )
    .await;
    assert_eq!(status, hyper::StatusCode::OK, "{canceled}");
    assert_eq!(canceled["state"], "canceled");
}

#[tokio::test]
#[serial_test::serial]
async fn retry_scanner_keeps_verification_pending_and_exact_completion_replayable() {
    ensure_crypt();
    let _container = TestContainer::fake();
    let upload_host = LiveUploadHost::new()
        .with_scanner(Arc::new(UnavailableUploadScanner))
        .with_finalizer(Arc::new(TestUploadFinalizer::default()));
    let (router, _) = semantic_router_and_runtime_with_host(upload_host);
    let middleware = Arc::new(MiddlewareRegistry::new().append(StrictUploadFacts));
    let bytes =
        Bytes::from_static(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01");
    let checksum = hex::encode(Sha256::digest(&bytes));
    let (status, _, created) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        json!({
            "field": "retry_scanned_avatar",
            "file": {
                "lastModified": 1,
                "name": "retry-scanned-avatar.png",
                "size": bytes.len(),
                "type": "image/png"
            },
            "idempotency_key": "create-retry-scanned-avatar",
            "island": {
                "component": "tests.upload-route-component",
                "documentKey": "avatar-document",
                "slot": "avatar-slot"
            },
            "operation": "create",
            "protocol_version": 1
        }),
        None,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::CREATED, "{created}");
    let handle = created["handle"].as_str().expect("retry upload handle");
    let grant = created["grant"].as_str().expect("retry upload grant");
    let request = chunk_request(
        handle,
        grant,
        1,
        "put-retry-scanned-avatar-0",
        0,
        0,
        bytes,
        &checksum,
    );
    let (status, _, body) =
        dispatch_shared(Arc::clone(&router), Arc::clone(&middleware), request).await;
    let chunk: Value = serde_json::from_slice(&body).expect("retry chunk response");
    assert_eq!(status, hyper::StatusCode::OK, "{chunk}");
    assert_eq!(chunk["revision"], "4");

    let complete = json!({
        "expected_revision": "4",
        "handle": handle,
        "idempotency_key": "complete-retry-scanned-avatar",
        "operation": "complete",
        "protocol_version": 1,
        "whole_checksum": checksum
    });
    for attempt in 0..2 {
        let (status, _, pending) = send_control(
            Arc::clone(&router),
            Arc::clone(&middleware),
            complete.clone(),
            Some(grant),
        )
        .await;
        assert_eq!(
            status,
            hyper::StatusCode::SERVICE_UNAVAILABLE,
            "attempt {attempt}: {pending}"
        );
        assert_eq!(pending["state"], "verifying");
        assert_eq!(pending["revision"], "5");
    }
}

#[tokio::test]
#[serial_test::serial]
async fn transient_finalize_failure_reconciles_before_returning_the_committed_action() {
    ensure_crypt();
    let _container = TestContainer::fake();
    let finalizer = Arc::new(TestUploadFinalizer::fail_one_commit());
    let upload_host = LiveUploadHost::new().with_finalizer(finalizer.clone());
    let (router, _) = semantic_router_and_runtime_with_host(upload_host);
    let middleware = Arc::new(MiddlewareRegistry::new().append(StrictUploadFacts));
    let (handle, grant, _) = create_ready_avatar(
        Arc::clone(&router),
        Arc::clone(&middleware),
        "finalize-retry-avatar",
    )
    .await;

    let document_request = hyper::Request::builder()
        .uri("/upload-fixture")
        .body(Full::new(Bytes::new()))
        .expect("build upload document request");
    let (status, _, document_body) = dispatch_shared(
        Arc::clone(&router),
        Arc::clone(&middleware),
        document_request,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::OK);
    let document = std::str::from_utf8(&document_body).expect("upload document UTF-8");
    let seed = URL_SAFE_NO_PAD
        .decode(html_attribute(document, "data-suprnova-live-snapshot"))
        .expect("decode upload seed snapshot");
    let seed: Value = serde_json::from_slice(&seed).expect("parse upload seed snapshot");

    SAVE_ACTION_CALLS.store(0, Ordering::SeqCst);
    let first = upload_action_request(
        &seed,
        &handle,
        "YGFiY2RlZmdoaWprbG1ubw",
        "cHFyc3R1dnd4eXp7fH1-fw",
        "gIGCg4SFhoeIiYqLjI2Ojw",
    );
    let (status, _, action_body) =
        dispatch_shared(Arc::clone(&router), Arc::clone(&middleware), first).await;
    assert_eq!(
        status,
        hyper::StatusCode::OK,
        "bounded reconciliation lost the committed action response: {}",
        String::from_utf8_lossy(&action_body)
    );
    assert_eq!(SAVE_ACTION_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(finalizer.commit_calls.load(Ordering::SeqCst), 2);

    let (status, _, finalized) = send_control(
        router,
        middleware,
        json!({
            "handle": handle,
            "operation": "status",
            "protocol_version": 1
        }),
        Some(&grant),
    )
    .await;
    assert_eq!(status, hyper::StatusCode::OK, "{finalized}");
    assert_eq!(finalized["state"], "finalized");
    assert_eq!(finalized["revision"], "8");
}

#[tokio::test]
#[serial_test::serial]
async fn cancellation_waits_for_finalization_and_cannot_retire_the_committed_upload() {
    ensure_crypt();
    let _container = TestContainer::fake();
    let finalizer = Arc::new(BlockingUploadFinalizer::default());
    let upload_host = LiveUploadHost::new().with_finalizer(finalizer.clone());
    let (router, _) = semantic_router_and_runtime_with_host(upload_host);
    let cancel_arrival = Arc::new(tokio::sync::Semaphore::new(0));
    let middleware = Arc::new(MiddlewareRegistry::new().append(StrictUploadFacts).append(
        CancelArrival {
            entered: Arc::clone(&cancel_arrival),
        },
    ));
    let (handle, grant, _) = create_ready_avatar(
        Arc::clone(&router),
        Arc::clone(&middleware),
        "cancel-finalize-race-avatar",
    )
    .await;

    let document_request = hyper::Request::builder()
        .uri("/upload-fixture")
        .body(Full::new(Bytes::new()))
        .expect("build upload document request");
    let (status, _, document_body) = dispatch_shared(
        Arc::clone(&router),
        Arc::clone(&middleware),
        document_request,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::OK);
    let document = std::str::from_utf8(&document_body).expect("upload document UTF-8");
    let seed = URL_SAFE_NO_PAD
        .decode(html_attribute(document, "data-suprnova-live-snapshot"))
        .expect("decode upload seed snapshot");
    let seed: Value = serde_json::from_slice(&seed).expect("parse upload seed snapshot");

    SAVE_ACTION_CALLS.store(0, Ordering::SeqCst);
    let action_router = Arc::clone(&router);
    let action_middleware = Arc::clone(&middleware);
    let action_handle = handle.clone();
    let action = tokio::spawn(async move {
        dispatch_shared(
            action_router,
            action_middleware,
            upload_action_request(
                &seed,
                &action_handle,
                "kJGSk5SVlpeYmZqbnJ2enw",
                "oKGio6SlpqeoqaqrrK2urw",
                "sLGys7S1tre4ubq7vL2-vw",
            ),
        )
        .await
    });
    finalizer.wait_until_commit().await;

    let mut cancel_request = control_request(
        json!({
            "expected_revision": "6",
            "handle": handle,
            "idempotency_key": "cancel-during-finalize",
            "operation": "cancel",
            "protocol_version": 1
        }),
        Some(&grant),
    );
    cancel_request.headers_mut().insert(
        "x-test-upload-cancel-arrival",
        hyper::header::HeaderValue::from_static("1"),
    );
    let cancel_router = Arc::clone(&router);
    let cancel_middleware = Arc::clone(&middleware);
    let cancel = tokio::spawn(async move {
        dispatch_shared(cancel_router, cancel_middleware, cancel_request).await
    });
    cancel_arrival
        .acquire()
        .await
        .expect("cancel arrival barrier remains open")
        .forget();
    tokio::task::yield_now().await;
    assert!(
        !cancel.is_finished(),
        "cancel completed while finalization held the upload operation lock"
    );

    finalizer.release_commit();
    let (action_status, _, action_body) = action.await.expect("join finalizing action");
    assert_eq!(
        action_status,
        hyper::StatusCode::OK,
        "finalizing action failed: {}",
        String::from_utf8_lossy(&action_body)
    );
    let (cancel_status, _, cancel_body) = cancel.await.expect("join racing cancellation");
    assert_eq!(
        cancel_status,
        hyper::StatusCode::CONFLICT,
        "racing cancellation was not rejected: {}",
        String::from_utf8_lossy(&cancel_body)
    );
    assert_eq!(SAVE_ACTION_CALLS.load(Ordering::SeqCst), 1);

    let (status, _, finalized) = send_control(
        router,
        middleware,
        json!({
            "handle": handle,
            "operation": "status",
            "protocol_version": 1
        }),
        Some(&grant),
    )
    .await;
    assert_eq!(status, hyper::StatusCode::OK, "{finalized}");
    assert_eq!(finalized["state"], "finalized");
    assert_eq!(finalized["revision"], "8");
}

#[tokio::test]
#[serial_test::serial]
async fn large_chunks_and_exact_retries_preserve_revision_and_provider_order() {
    ensure_crypt();
    let _container = TestContainer::fake();
    let router = semantic_router();
    let middleware = Arc::new(MiddlewareRegistry::new().append(StrictUploadFacts));
    let mut first = vec![0_u8; 128 * 1024];
    first[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    let first = Bytes::from(first);
    let second = Bytes::from(vec![1_u8; 128 * 1024]);
    let first_checksum = hex::encode(Sha256::digest(&first));
    let second_checksum = hex::encode(Sha256::digest(&second));

    let (status, _, created) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        json!({
            "field": "large_avatar",
            "file": {
                "lastModified": 1,
                "name": "large.png",
                "size": first.len() + second.len(),
                "type": "image/png"
            },
            "idempotency_key": "create-large-avatar",
            "island": {
                "component": "tests.upload-route-component",
                "documentKey": "avatar-document",
                "slot": "avatar-slot"
            },
            "operation": "create",
            "protocol_version": 1
        }),
        None,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::CREATED, "{created}");
    let handle = created["handle"].as_str().expect("created handle");
    let grant = created["grant"].as_str().expect("created grant");

    let send = |revision, key, index, offset, bytes: Bytes, checksum: &str| {
        dispatch_shared(
            Arc::clone(&router),
            Arc::clone(&middleware),
            chunk_request(handle, grant, revision, key, index, offset, bytes, checksum),
        )
    };
    let mut oversized = vec![0_u8; 256 * 1024 + 1];
    oversized[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    let oversized = Bytes::from(oversized);
    let oversized_checksum = hex::encode(Sha256::digest(&oversized));
    let (status, _, _) = send(
        1,
        "put-large-avatar-oversized",
        0,
        0,
        oversized,
        &oversized_checksum,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::PAYLOAD_TOO_LARGE);
    let (status, _, untouched) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        json!({
            "handle": handle,
            "operation": "status",
            "protocol_version": 1
        }),
        Some(grant),
    )
    .await;
    assert_eq!(status, hyper::StatusCode::OK, "{untouched}");
    assert_eq!(untouched["revision"], "1");
    assert_eq!(untouched["nextChunkIndex"], 0);

    let (status, _, body) = send(
        1,
        "put-large-avatar-0",
        0,
        0,
        first.clone(),
        &first_checksum,
    )
    .await;
    let accepted: Value = serde_json::from_slice(&body).expect("first chunk response");
    assert_eq!(status, hyper::StatusCode::OK, "{accepted}");
    assert_eq!(accepted["revision"], "4");

    let (status, _, body) = send(
        1,
        "put-large-avatar-0",
        0,
        0,
        first.clone(),
        &first_checksum,
    )
    .await;
    let replay: Value = serde_json::from_slice(&body).expect("retry response");
    assert_eq!(status, hyper::StatusCode::OK, "{replay}");
    assert_eq!(replay["revision"], "4");

    let (status, _, _) = send(
        3,
        "put-large-avatar-stale-1",
        1,
        first.len() as u64,
        second.clone(),
        &second_checksum,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::CONFLICT);

    let (status, _, inspected) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        json!({
            "handle": handle,
            "operation": "status",
            "protocol_version": 1
        }),
        Some(grant),
    )
    .await;
    assert_eq!(status, hyper::StatusCode::OK, "{inspected}");
    assert_eq!(inspected["revision"], "4");
    assert_eq!(inspected["nextChunkIndex"], 1);

    let (status, _, _) = send(
        4,
        "put-large-avatar-wrong-offset",
        1,
        1,
        second.clone(),
        &second_checksum,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::CONFLICT);

    let (status, _, body) = send(
        4,
        "put-large-avatar-1",
        1,
        first.len() as u64,
        second,
        &second_checksum,
    )
    .await;
    let accepted: Value = serde_json::from_slice(&body).expect("second chunk response");
    assert_eq!(status, hyper::StatusCode::OK, "{accepted}");
    assert_eq!(accepted["revision"], "5");
}

#[tokio::test]
#[serial_test::serial]
async fn authored_file_count_and_replacement_policy_are_atomic() {
    ensure_crypt();
    let _container = TestContainer::fake();
    let (router, runtime) = semantic_router_and_runtime();
    let middleware = Arc::new(MiddlewareRegistry::new().append(StrictUploadFacts));
    let create = |field: &str, key: &str| {
        json!({
            "field": field,
            "file": {
                "lastModified": 1,
                "name": "policy.png",
                "size": 512,
                "type": "image/png"
            },
            "idempotency_key": key,
            "island": {
                "component": "tests.upload-route-component",
                "documentKey": "avatar-document",
                "slot": "avatar-slot"
            },
            "operation": "create",
            "protocol_version": 1
        })
    };

    let (status, _, first) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        create("preserved_avatar", "preserve-first"),
        None,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::CREATED, "{first}");
    let (status, _, rejected) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        create("preserved_avatar", "preserve-second"),
        None,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::BAD_REQUEST, "{rejected}");
    assert_eq!(rejected["error"], "upload_file_count_exceeded");

    let (status, _, old) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        create("avatar", "retire-old"),
        None,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::CREATED, "{old}");
    let (status, _, replacement) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        create("avatar", "retire-new"),
        None,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::CREATED, "{replacement}");

    let authority = inspect_upload_mount_authority_for_test(
        &runtime,
        "tests.upload-route-component",
        "avatar-slot",
        "avatar-document",
        Some(b"upload-session"),
        Some(b"upload-principal"),
        Some(b"upload-tenant"),
    )
    .expect("derive replacement authority");
    let cleanup = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let residue = inspect_configured_upload_residue_for_test(
                &runtime,
                &authority,
                "avatar",
                "retire-old",
            )
            .await
            .expect("inspect retired upload residue");
            if residue.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        cleanup.is_ok(),
        "retired upload was not reclaimed automatically"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn aggregate_declared_bytes_are_reserved_per_scope_before_provider_work() {
    ensure_crypt();
    let _container = TestContainer::fake();
    let router = semantic_router();
    let middleware = Arc::new(MiddlewareRegistry::new().append(StrictUploadFacts));
    for index in 0..4 {
        let (status, _, created) = send_control(
            Arc::clone(&router),
            Arc::clone(&middleware),
            json!({
                "field": "aggregate_avatar",
                "file": {
                    "lastModified": index,
                    "name": format!("aggregate-{index}.png"),
                    "size": 64 * 1024 * 1024_u64,
                    "type": "image/png"
                },
                "idempotency_key": format!("aggregate-{index}"),
                "island": {
                    "component": "tests.upload-route-component",
                    "documentKey": "avatar-document",
                    "slot": "avatar-slot"
                },
                "operation": "create",
                "protocol_version": 1
            }),
            None,
        )
        .await;
        assert_eq!(status, hyper::StatusCode::CREATED, "{created}");
    }
    let (status, _, exhausted) = send_control(
        router,
        middleware,
        json!({
            "field": "aggregate_avatar",
            "file": {
                "lastModified": 5,
                "name": "aggregate-overflow.png",
                "size": 1,
                "type": "image/png"
            },
            "idempotency_key": "aggregate-overflow",
            "island": {
                "component": "tests.upload-route-component",
                "documentKey": "avatar-document",
                "slot": "avatar-slot"
            },
            "operation": "create",
            "protocol_version": 1
        }),
        None,
    )
    .await;
    assert_eq!(
        status,
        hyper::StatusCode::SERVICE_UNAVAILABLE,
        "{exhausted}"
    );
    assert_eq!(exhausted["error"], "upload_resource_exhausted");
}

#[tokio::test]
#[serial_test::serial]
async fn required_scanner_rejects_when_the_default_scanner_is_unavailable() {
    ensure_crypt();
    let _container = TestContainer::fake();
    let router = semantic_router();
    let middleware = Arc::new(MiddlewareRegistry::new().append(StrictUploadFacts));
    let bytes =
        Bytes::from_static(b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01");
    let checksum = hex::encode(Sha256::digest(&bytes));

    let (status, _, created) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        json!({
            "field": "scanned_avatar",
            "file": {
                "lastModified": 1,
                "name": "scanned-avatar.png",
                "size": bytes.len(),
                "type": "image/png"
            },
            "idempotency_key": "create-scanned-avatar",
            "island": {
                "component": "tests.upload-route-component",
                "documentKey": "avatar-document",
                "slot": "avatar-slot"
            },
            "operation": "create",
            "protocol_version": 1
        }),
        None,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::CREATED, "{created}");
    let handle = created["handle"].as_str().expect("created handle");
    let grant = created["grant"].as_str().expect("created grant");

    let chunk_request = hyper::Request::builder()
        .method(Method::POST)
        .uri(LIVE_UPLOAD_PATH)
        .header("authorization", format!("SuprnovaUpload {grant}"))
        .header("content-type", "application/octet-stream")
        .header("x-suprnova-live", "upload-v1")
        .header("x-suprnova-upload-checksum", &checksum)
        .header("x-suprnova-upload-chunk", "0")
        .header("x-suprnova-upload-offset", "0")
        .header("x-suprnova-upload-handle", handle)
        .header("x-suprnova-upload-idempotency", "put-scanned-avatar-0")
        .header("x-suprnova-upload-operation", "put_chunk")
        .header("x-suprnova-upload-revision", "1")
        .body(Full::new(bytes))
        .expect("build scanned upload chunk request");
    let (status, _, chunk_body) =
        dispatch_shared(Arc::clone(&router), Arc::clone(&middleware), chunk_request).await;
    let chunk: Value = serde_json::from_slice(&chunk_body).expect("chunk response JSON");
    assert_eq!(status, hyper::StatusCode::OK, "{chunk}");
    assert_eq!(chunk["revision"], "4");

    let (status, _, completed) = send_control(
        router,
        middleware,
        json!({
            "expected_revision": "4",
            "handle": handle,
            "idempotency_key": "complete-scanned-avatar",
            "operation": "complete",
            "protocol_version": 1,
            "whole_checksum": checksum
        }),
        Some(grant),
    )
    .await;
    assert_eq!(
        status,
        hyper::StatusCode::UNPROCESSABLE_ENTITY,
        "{completed}"
    );
    assert_eq!(completed["state"], "rejected");
    assert_eq!(completed["revision"], "6");
}

#[tokio::test]
#[serial_test::serial]
async fn authoritative_media_validation_rejects_a_false_png_claim() {
    ensure_crypt();
    let _container = TestContainer::fake();
    let router = semantic_router();
    let middleware = Arc::new(MiddlewareRegistry::new().append(StrictUploadFacts));
    let bytes = Bytes::from_static(b"plain text pretending to be a PNG");
    let checksum = hex::encode(Sha256::digest(&bytes));

    let (status, _, created) = send_control(
        Arc::clone(&router),
        Arc::clone(&middleware),
        json!({
            "field": "avatar",
            "file": {
                "lastModified": 1,
                "name": "false-claim.png",
                "size": bytes.len(),
                "type": "image/png"
            },
            "idempotency_key": "create-false-claim",
            "island": {
                "component": "tests.upload-route-component",
                "documentKey": "avatar-document",
                "slot": "avatar-slot"
            },
            "operation": "create",
            "protocol_version": 1
        }),
        None,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::CREATED, "{created}");
    let handle = created["handle"].as_str().expect("created handle");
    let grant = created["grant"].as_str().expect("created grant");

    let chunk_request = hyper::Request::builder()
        .method(Method::POST)
        .uri(LIVE_UPLOAD_PATH)
        .header("authorization", format!("SuprnovaUpload {grant}"))
        .header("content-type", "application/octet-stream")
        .header("x-suprnova-live", "upload-v1")
        .header("x-suprnova-upload-checksum", &checksum)
        .header("x-suprnova-upload-chunk", "0")
        .header("x-suprnova-upload-offset", "0")
        .header("x-suprnova-upload-handle", handle)
        .header("x-suprnova-upload-idempotency", "put-false-claim-0")
        .header("x-suprnova-upload-operation", "put_chunk")
        .header("x-suprnova-upload-revision", "1")
        .body(Full::new(bytes))
        .expect("build false-claim upload chunk request");
    let (status, _, chunk_body) =
        dispatch_shared(Arc::clone(&router), Arc::clone(&middleware), chunk_request).await;
    let chunk: Value = serde_json::from_slice(&chunk_body).expect("chunk response JSON");
    assert_eq!(status, hyper::StatusCode::OK, "{chunk}");
    assert_eq!(chunk["revision"], "4");

    let (status, _, completed) = send_control(
        router,
        middleware,
        json!({
            "expected_revision": "4",
            "handle": handle,
            "idempotency_key": "complete-false-claim",
            "operation": "complete",
            "protocol_version": 1,
            "whole_checksum": checksum
        }),
        Some(grant),
    )
    .await;
    assert_eq!(
        status,
        hyper::StatusCode::UNPROCESSABLE_ENTITY,
        "{completed}"
    );
    assert_eq!(completed["state"], "rejected");
    assert_eq!(completed["revision"], "6");
}

#[tokio::test]
#[serial_test::serial]
async fn rejected_uploads_leave_no_ledger_provider_or_metadata_residue() {
    ensure_crypt();
    let _container = TestContainer::fake();
    let (router, runtime) = semantic_router_and_runtime();
    let authority = inspect_upload_mount_authority_for_test(
        &runtime,
        "tests.upload-route-component",
        "avatar-slot",
        "avatar-document",
        Some(b"upload-session"),
        Some(b"upload-principal"),
        Some(b"upload-tenant"),
    )
    .expect("derive test upload authority");
    let strict = Arc::new(MiddlewareRegistry::new().append(StrictUploadFacts));
    let no_auth = Arc::new(MiddlewareRegistry::new().append(UploadFactsWithoutCurrentAuth));

    let create = |field: &str, size: u64, idempotency_key: &str| {
        json!({
            "field": field,
            "file": {
                "lastModified": 1,
                "name": "avatar.png",
                "size": size,
                "type": "image/png"
            },
            "idempotency_key": idempotency_key,
            "island": {
                "component": "tests.upload-route-component",
                "documentKey": "avatar-document",
                "slot": "avatar-slot"
            },
            "operation": "create",
            "protocol_version": 1
        })
    };

    let wrong_media_key = "reject-media";
    let wrong_media = hyper::Request::builder()
        .method(Method::POST)
        .uri(LIVE_UPLOAD_PATH)
        .header("content-type", "text/plain")
        .header("x-suprnova-live", "upload-v1")
        .body(Full::new(Bytes::from(
            serde_json::to_vec(&create("avatar", 24, wrong_media_key))
                .expect("encode wrong-media create"),
        )))
        .expect("build wrong-media create");
    let (status, _, _) =
        dispatch_shared(Arc::clone(&router), Arc::clone(&strict), wrong_media).await;
    assert_eq!(status, hyper::StatusCode::UNSUPPORTED_MEDIA_TYPE);

    let denied_key = "reject-auth";
    let (status, _, _) = send_control(
        Arc::clone(&router),
        no_auth,
        create("avatar", 24, denied_key),
        None,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::FORBIDDEN);

    let oversized_key = "reject-size";
    let (status, _, _) = send_control(
        Arc::clone(&router),
        Arc::clone(&strict),
        create("avatar", 1025, oversized_key),
        None,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::PAYLOAD_TOO_LARGE);

    let missing_policy_key = "reject-policy";
    let (status, _, _) = send_control(
        router,
        strict,
        create("undeclared", 24, missing_policy_key),
        None,
    )
    .await;
    assert_eq!(status, hyper::StatusCode::BAD_REQUEST);

    for (field, key) in [
        ("avatar", wrong_media_key),
        ("avatar", denied_key),
        ("avatar", oversized_key),
        ("undeclared", missing_policy_key),
    ] {
        let residue = inspect_configured_upload_residue_for_test(&runtime, &authority, field, key)
            .await
            .expect("inspect rejected upload residue");
        assert!(
            residue.is_empty(),
            "pre-semantic rejection must not touch ledger, provider, or metadata memo"
        );
    }
}
