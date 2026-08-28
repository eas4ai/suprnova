//! Browser-facing production-artifact host for Iteration 004 conformance.

mod artifacts;
mod async_updates;
mod engine_async;
mod faults;
mod uploads;

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Request, State};
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, ORIGIN};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt as _;
use serde_json::{Value, json};
use suprnova_live::limits::InputLimits;
use suprnova_live::protocol::{
    OperationV2, ProtocolLimitConfig, ProtocolLimits, VersionedUpdateRequest,
    parse_versioned_update_request,
};
use suprnova_live::upload::{UploadError, UploadErrorKind};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Duration;

use artifacts::ValidatedArtifacts;
use async_updates::{AsyncRuntime, MembershipRequest, PollRequest, TransportCreateRequest};
pub use faults::ReferenceFaultSchedule;
use uploads::{CompleteUploadRequest, CreateUploadRequest, UploadRuntime};

/// Upload-creation route.
pub const CREATE_UPLOAD: &str = "/__live/uploads";
/// Bounded reverse-proxy upload-chunk route.
pub const UPLOAD_CHUNK: &str = "/__live/uploads/:handle/chunks/:part";
/// Current upload-status route.
pub const UPLOAD_STATUS: &str = "/__live/uploads/:handle";
/// Upload-completion route.
pub const UPLOAD_COMPLETE: &str = "/__live/uploads/:handle/complete";
/// Upload-cancellation route.
pub const UPLOAD_CANCEL: &str = "/__live/uploads/:handle/cancel";
/// Authenticated application-owned upload-grant reacquisition example.
pub const EXAMPLE_REACQUIRE: &str = "/example/uploads/:handle/reacquire";
/// Ordinary fresh-render polling route.
pub const POLL: &str = "/__live/async/poll";
/// Physical async-transport creation route.
pub const TRANSPORT_CREATE: &str = "/__live/async/transports";
/// Exact logical-membership route.
pub const TRANSPORT_MEMBERSHIP: &str =
    "/__live/async/transports/:transport/subscriptions/:subscription";
/// One physical document SSE route.
pub const SSE: &str = "/__live/async/sse/:transport";
/// One physical document WebSocket route.
pub const WEBSOCKET: &str = "/__live/async/ws";
/// Physical-browser fresh-render conformance scenario.
pub const FRESH_RENDER_SCENARIO: &str = "/scenario/referenceFreshRender";

/// Static bearer used only by the closed reference host.
pub const REFERENCE_AUTHORIZATION: &str = "Bearer task1-reference-session";

/// Complete deterministic reference-host startup configuration.
#[derive(Clone, Debug)]
pub struct ReferenceHostConfig {
    address: SocketAddr,
    artifact_root: PathBuf,
    quarantine_root: PathBuf,
    fault_schedule: ReferenceFaultSchedule,
}

impl ReferenceHostConfig {
    /// Creates a configuration for one exact port and two explicit roots.
    #[must_use]
    pub const fn new(
        address: SocketAddr,
        artifact_root: PathBuf,
        quarantine_root: PathBuf,
    ) -> Self {
        Self {
            address,
            artifact_root,
            quarantine_root,
            fault_schedule: ReferenceFaultSchedule::None,
        }
    }

    /// Selects one compiled server-owned fault schedule.
    #[must_use]
    pub const fn with_fault_schedule(mut self, fault_schedule: ReferenceFaultSchedule) -> Self {
        self.fault_schedule = fault_schedule;
        self
    }
}

/// Safe identifier-free host counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReferenceHostInspection {
    /// Calls admitted to the Rust upload runtime and providers.
    pub upload_service_calls: usize,
    /// Total physical SSE connections opened.
    pub physical_sse_connections: usize,
    /// Total physical WebSocket connections opened.
    pub physical_websocket_connections: usize,
    /// Largest bounded logical-membership count observed.
    pub maximum_logical_memberships: usize,
    /// Compiled fault steps applied.
    pub compiled_faults_applied: usize,
    /// Requests rejected for attempting arbitrary fault selection.
    pub rejected_arbitrary_fault_selectors: usize,
    /// Currently owned sockets.
    pub open_sockets: usize,
    /// Currently owned files.
    pub open_files: usize,
    /// Currently owned timers.
    pub open_timers: usize,
    /// Currently pending uploads.
    pub active_uploads: usize,
    /// Currently authenticated logical memberships.
    pub logical_memberships: usize,
}

const RESOURCE_RETIRED: usize = 1 << (usize::BITS - 1);

#[derive(Default)]
pub(super) struct ResourceCounter {
    state: AtomicUsize,
    drained: Notify,
}

impl ResourceCounter {
    fn acquire(self: &Arc<Self>) -> Option<ResourceLease> {
        self.state
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                if state & RESOURCE_RETIRED != 0
                    || state & !RESOURCE_RETIRED == RESOURCE_RETIRED - 1
                {
                    None
                } else {
                    Some(state + 1)
                }
            })
            .ok()
            .map(|_| ResourceLease {
                counter: Arc::clone(self),
            })
    }

    fn current(&self) -> usize {
        self.state.load(Ordering::Acquire) & !RESOURCE_RETIRED
    }

    fn retire(&self) {
        self.state.fetch_or(RESOURCE_RETIRED, Ordering::AcqRel);
    }

    async fn wait_until_drained(&self) {
        loop {
            let drained = self.drained.notified();
            if self.current() == 0 {
                return;
            }
            drained.await;
        }
    }
}

pub(super) struct ResourceLease {
    counter: Arc<ResourceCounter>,
}

impl Drop for ResourceLease {
    fn drop(&mut self) {
        let previous = self.counter.state.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous & !RESOURCE_RETIRED > 0);
        if previous & !RESOURCE_RETIRED == 1 {
            self.counter.drained.notify_one();
        }
    }
}

#[derive(Clone, Default)]
struct ResourceCounters {
    open_sockets: Arc<ResourceCounter>,
    open_files: Arc<ResourceCounter>,
    open_timers: Arc<ResourceCounter>,
    active_uploads: Arc<ResourceCounter>,
    logical_memberships: Arc<ResourceCounter>,
}

impl ResourceCounters {
    fn retire(&self) {
        self.open_sockets.retire();
        self.open_files.retire();
        self.open_timers.retire();
        self.active_uploads.retire();
        self.logical_memberships.retire();
    }

    async fn wait_until_drained(&self) -> Result<(), String> {
        tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(
                self.open_sockets.wait_until_drained(),
                self.open_files.wait_until_drained(),
                self.open_timers.wait_until_drained(),
                self.active_uploads.wait_until_drained(),
                self.logical_memberships.wait_until_drained(),
            );
        })
        .await
        .map_err(|_| {
            format!(
                "reference host resource drain timed out: sockets={}, files={}, timers={}, uploads={}, memberships={}",
                self.open_sockets.current(),
                self.open_files.current(),
                self.open_timers.current(),
                self.active_uploads.current(),
                self.logical_memberships.current(),
            )
        })
    }
}

#[derive(Default)]
struct InspectionState {
    physical_sse_connections: AtomicUsize,
    physical_websocket_connections: AtomicUsize,
    rejected_arbitrary_fault_selectors: AtomicUsize,
    resources: ResourceCounters,
}

/// Cloneable observer retained across reference-host shutdown.
#[derive(Clone)]
pub struct ReferenceHostInspectionHandle {
    state: Arc<HostState>,
}

impl ReferenceHostInspectionHandle {
    /// Takes one secret-free point-in-time snapshot.
    #[must_use]
    pub fn snapshot(&self) -> ReferenceHostInspection {
        self.state.snapshot()
    }
}

struct HostState {
    origin: String,
    artifacts: ValidatedArtifacts,
    uploads: UploadRuntime,
    async_runtime: AsyncRuntime,
    request_shutdown: watch::Receiver<bool>,
    inspection: InspectionState,
}

impl HostState {
    fn snapshot(&self) -> ReferenceHostInspection {
        ReferenceHostInspection {
            upload_service_calls: self.uploads.service_calls(),
            physical_sse_connections: self
                .inspection
                .physical_sse_connections
                .load(Ordering::SeqCst),
            physical_websocket_connections: self
                .inspection
                .physical_websocket_connections
                .load(Ordering::SeqCst),
            maximum_logical_memberships: self.async_runtime.maximum_memberships(),
            compiled_faults_applied: self
                .async_runtime
                .fault_count()
                .saturating_add(self.uploads.fault_count()),
            rejected_arbitrary_fault_selectors: self
                .inspection
                .rejected_arbitrary_fault_selectors
                .load(Ordering::SeqCst),
            open_sockets: self.inspection.resources.open_sockets.current(),
            open_files: self.inspection.resources.open_files.current(),
            open_timers: self.inspection.resources.open_timers.current(),
            active_uploads: self.inspection.resources.active_uploads.current(),
            logical_memberships: self.inspection.resources.logical_memberships.current(),
        }
    }
}

/// Running deterministic reference host.
pub struct ReferenceHost {
    address: SocketAddr,
    origin: String,
    state: Arc<HostState>,
    shutdown: Option<oneshot::Sender<()>>,
    request_shutdown: watch::Sender<bool>,
    server: Mutex<Option<JoinHandle<Result<(), std::io::Error>>>>,
}

impl ReferenceHost {
    /// Validates production artifacts, binds the exact address, and starts serving.
    pub async fn start(config: ReferenceHostConfig) -> Result<Self, String> {
        let artifacts = ValidatedArtifacts::load(&config.artifact_root).await?;
        tokio::fs::create_dir_all(&config.quarantine_root)
            .await
            .map_err(|error| format!("quarantine root: {error}"))?;
        let inspection = InspectionState::default();
        let (request_shutdown, request_shutdown_receiver) = watch::channel(false);
        let uploads = UploadRuntime::open(
            &config.quarantine_root,
            config.fault_schedule,
            request_shutdown_receiver.clone(),
            Arc::clone(&inspection.resources.active_uploads),
        )
        .await?;
        let listener = TcpListener::bind(config.address)
            .await
            .map_err(|error| format!("reference host bind: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("reference host address: {error}"))?;
        if address != config.address {
            return Err("reference host did not bind the configured deterministic port".to_owned());
        }
        let origin = format!("http://{address}");
        let async_runtime = AsyncRuntime::new(
            config.fault_schedule,
            Arc::clone(&inspection.resources.logical_memberships),
        )
        .await?;
        let state = Arc::new(HostState {
            origin: origin.clone(),
            artifacts,
            uploads,
            async_runtime,
            request_shutdown: request_shutdown_receiver,
            inspection,
        });
        let router = router(state.clone());
        let (shutdown, receiver) = oneshot::channel();
        let listener_lease = state
            .inspection
            .resources
            .open_sockets
            .acquire()
            .ok_or_else(|| "reference host resources retired before serve".to_owned())?;
        let server = tokio::spawn(async move {
            let _listener_lease = listener_lease;
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = receiver.await;
                })
                .await
        });
        Ok(Self {
            address,
            origin,
            state,
            shutdown: Some(shutdown),
            request_shutdown,
            server: Mutex::new(Some(server)),
        })
    }

    /// Returns the exact bound socket address.
    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        self.address
    }

    /// Returns the browser-facing same origin without a trailing slash.
    #[must_use]
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Takes one secret-free point-in-time inspection snapshot.
    #[must_use]
    pub fn inspection(&self) -> ReferenceHostInspection {
        self.state.snapshot()
    }

    /// Returns a cloneable observer that remains valid after shutdown.
    #[must_use]
    pub fn inspection_handle(&self) -> ReferenceHostInspectionHandle {
        ReferenceHostInspectionHandle {
            state: self.state.clone(),
        }
    }

    /// Builds one exact request for the reference host's current engine-owned island.
    pub async fn fresh_render_request(
        &self,
        correlation: &str,
        seed: u8,
    ) -> Result<String, String> {
        self.state
            .async_runtime
            .fresh_render_request(correlation, seed)
            .await
            .map_err(str::to_owned)
    }

    /// Pauses the production component renderer for cancellation conformance.
    pub fn pause_fresh_render(&self) {
        self.state.async_runtime.pause_fresh_render();
    }

    /// Waits until a paused production component render has started.
    pub async fn wait_until_fresh_render_paused(&self) {
        self.state
            .async_runtime
            .wait_until_fresh_render_paused()
            .await;
    }

    /// Releases a renderer paused for cancellation conformance.
    pub fn resume_fresh_render(&self) {
        self.state.async_runtime.resume_fresh_render();
    }

    /// Executes one exact fresh-render body without involving socket cancellation semantics.
    pub async fn execute_fresh_render_direct(
        &self,
        body: axum::body::Bytes,
    ) -> Result<suprnova_live::endpoint::LiveEndpointResponse, String> {
        self.state
            .async_runtime
            .execute_fresh_render_direct(body)
            .await
            .map_err(str::to_owned)
    }

    /// Retires owned services and waits for the listening socket to close.
    pub async fn shutdown(mut self) -> Result<(), String> {
        self.state.inspection.resources.retire();
        let _ = self.request_shutdown.send(true);
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let upload_retirement = self.state.uploads.retire().await;
        let async_retirement = self.state.async_runtime.retire().await;
        let mut server_retirement = Ok(());
        if let Some(mut server) = self.server.lock().await.take() {
            match tokio::time::timeout(Duration::from_secs(1), &mut server).await {
                Ok(result) => {
                    server_retirement = result
                        .map_err(|error| format!("reference host task: {error}"))
                        .and_then(|result| {
                            result.map_err(|error| format!("reference host server: {error}"))
                        });
                }
                Err(_) => {
                    server.abort();
                    match server.await {
                        Err(error) if error.is_cancelled() => {}
                        Err(error) => {
                            server_retirement = Err(format!("reference host task: {error}"));
                        }
                        Ok(Err(error)) => {
                            server_retirement = Err(format!("reference host server: {error}"));
                        }
                        Ok(Ok(())) => {}
                    }
                }
            }
        }
        let resource_retirement = self.state.inspection.resources.wait_until_drained().await;
        upload_retirement?;
        async_retirement?;
        server_retirement?;
        resource_retirement
    }
}

fn router(state: Arc<HostState>) -> Router {
    let query_guard_state = state.clone();
    Router::new()
        .route(CREATE_UPLOAD, post(create_upload))
        .route(UPLOAD_CHUNK, post(upload_chunk))
        .route(UPLOAD_STATUS, get(upload_status))
        .route(UPLOAD_COMPLETE, post(upload_complete))
        .route(UPLOAD_CANCEL, post(upload_cancel))
        .route(EXAMPLE_REACQUIRE, post(upload_reacquire))
        .route(POLL, post(poll))
        .route(TRANSPORT_CREATE, post(transport_create))
        .route(TRANSPORT_MEMBERSHIP, post(transport_membership))
        .route(SSE, get(sse))
        .route(WEBSOCKET, get(websocket))
        .route(FRESH_RENDER_SCENARIO, get(fresh_render_scenario))
        .fallback(get(static_asset).head(static_asset))
        .with_state(state)
        .layer(middleware::from_fn_with_state(
            query_guard_state,
            reject_dynamic_query_selectors,
        ))
}

async fn reject_dynamic_query_selectors(
    State(state): State<Arc<HostState>>,
    request: Request,
    next: Next,
) -> Response {
    if (request.uri().path().starts_with("/__live/")
        || request.uri().path().starts_with("/example/uploads/"))
        && request.uri().query().is_some()
    {
        state
            .inspection
            .rejected_arbitrary_fault_selectors
            .fetch_add(1, Ordering::SeqCst);
        error(StatusCode::BAD_REQUEST, "query_selector_rejected")
    } else {
        next.run(request).await
    }
}

async fn create_upload(
    State(state): State<Arc<HostState>>,
    headers: HeaderMap,
    Json(request): Json<CreateUploadRequest>,
) -> Response {
    if let Some(response) = session_error(&headers) {
        return response;
    }
    match state.uploads.create(request).await {
        Ok(value) => (StatusCode::CREATED, Json(value)).into_response(),
        Err(error) => upload_error(error),
    }
}

async fn upload_chunk(
    State(state): State<Arc<HostState>>,
    Path((handle, part)): Path<(String, u32)>,
    headers: HeaderMap,
    request: Request,
) -> Response {
    if let Some(response) = session_error(&headers) {
        return response;
    }
    let Some(grant) = header(&headers, "x-live-upload-grant") else {
        return error(StatusCode::UNAUTHORIZED, "upload_grant_missing");
    };
    let Some(checksum) = header(&headers, "x-live-chunk-sha256") else {
        return error(StatusCode::BAD_REQUEST, "chunk_checksum_missing");
    };
    let content_length = headers
        .get("x-live-chunk-bytes")
        .or_else(|| headers.get("content-length"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let Some(_file_lease) = state.inspection.resources.open_files.acquire() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "host_retired");
    };
    match state
        .uploads
        .write_chunk(
            &handle,
            part,
            grant,
            checksum,
            content_length,
            request.into_body(),
        )
        .await
    {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => upload_error(error),
    }
}

async fn upload_status(
    State(state): State<Arc<HostState>>,
    Path(handle): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(response) = session_error(&headers) {
        return response;
    }
    let Some(grant) = header(&headers, "x-live-upload-grant") else {
        return error(StatusCode::UNAUTHORIZED, "upload_grant_missing");
    };
    match state.uploads.status(&handle, grant).await {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => upload_error(error),
    }
}

async fn upload_complete(
    State(state): State<Arc<HostState>>,
    Path(handle): Path<String>,
    headers: HeaderMap,
    Json(request): Json<CompleteUploadRequest>,
) -> Response {
    if let Some(response) = session_error(&headers) {
        return response;
    }
    match state.uploads.complete(&handle, request).await {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => upload_error(error),
    }
}

async fn upload_cancel(
    State(state): State<Arc<HostState>>,
    Path(handle): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(response) = session_error(&headers) {
        return response;
    }
    let Some(grant) = header(&headers, "x-live-upload-grant") else {
        return error(StatusCode::UNAUTHORIZED, "upload_grant_missing");
    };
    match state.uploads.cancel(&handle, grant).await {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => upload_error(error),
    }
}

async fn upload_reacquire(
    State(state): State<Arc<HostState>>,
    Path(handle): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(response) = session_error(&headers) {
        return response;
    }
    let Some(grant) = header(&headers, "x-live-upload-grant") else {
        return error(StatusCode::UNAUTHORIZED, "upload_grant_missing");
    };
    match state.uploads.reacquire(&handle, grant).await {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => upload_error(error),
    }
}

async fn transport_create(
    State(state): State<Arc<HostState>>,
    headers: HeaderMap,
    Json(request): Json<TransportCreateRequest>,
) -> Response {
    if let Some(response) = session_error(&headers) {
        return response;
    }
    async_result(
        state.async_runtime.create(request, &state.origin).await,
        StatusCode::CREATED,
    )
}

async fn transport_membership(
    State(state): State<Arc<HostState>>,
    Path((transport, subscription)): Path<(String, String)>,
    headers: HeaderMap,
    Json(request): Json<MembershipRequest>,
) -> Response {
    if let Some(response) = session_error(&headers) {
        return response;
    }
    match state
        .async_runtime
        .membership(&transport, &subscription, request)
        .await
    {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(code) => error(StatusCode::UNAUTHORIZED, code),
    }
}

async fn poll(
    State(state): State<Arc<HostState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if let Some(response) = session_error(&headers) {
        return response;
    }
    let Some(subscription) = header(&headers, "x-live-subscription") else {
        return error(StatusCode::UNAUTHORIZED, "poll_authority_invalid");
    };
    let Some(authority) = header(&headers, "x-live-subscription-authority") else {
        return error(StatusCode::UNAUTHORIZED, "poll_authority_invalid");
    };
    let parsed = match parse_versioned_update_request(&body, &reference_protocol_limits()) {
        Ok(VersionedUpdateRequest::V2(request))
            if request.operations() == [OperationV2::FreshRender] =>
        {
            request
        }
        Ok(_) | Err(_) => return error(StatusCode::BAD_REQUEST, "poll_facts_invalid"),
    };
    debug_assert!(parsed.operations()[0].is_recovery_without_replay());
    match state
        .async_runtime
        .poll(
            PollRequest {
                subscription: subscription.to_owned(),
                authority: authority.to_owned(),
            },
            body,
        )
        .await
    {
        Ok(live) => {
            let mut response = Response::new(Body::from(live.body));
            *response.status_mut() = live.status;
            *response.headers_mut() = live.headers;
            if response.status() == StatusCode::OK {
                response
                    .headers_mut()
                    .insert("x-live-operation", HeaderValue::from_static("fresh-render"));
                response
                    .headers_mut()
                    .insert("x-live-action-executed", HeaderValue::from_static("false"));
            }
            response
        }
        Err(code) => error(StatusCode::UNAUTHORIZED, code),
    }
}

fn reference_protocol_limits() -> ProtocolLimits {
    ProtocolLimits::new(ProtocolLimitConfig {
        input: InputLimits::new(64 * 1024, 12, 512, 40 * 1024)
            .expect("reference protocol input limits"),
        max_snapshot_bytes: 32 * 1024,
        max_html_bytes: 32 * 1024,
        max_model_proposals: 8,
        max_operations: 8,
        max_arguments: 16,
        max_validation_entries: 16,
        max_events: 8,
        max_effects: 8,
        max_extensions: 8,
    })
    .expect("reference protocol limits")
}

async fn sse(
    State(state): State<Arc<HostState>>,
    Path(transport): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Some(response) = session_error(&headers) {
        return response;
    }
    let reader = match state.async_runtime.acquire_reader(
        &transport,
        suprnova_live::async_updates::DocumentTransportKind::ServerSentEvents,
    ) {
        Ok(reader) => reader,
        Err(code) => return error(StatusCode::CONFLICT, code),
    };
    match state.async_runtime.sse_batch(&transport).await {
        Ok(first_event) => {
            let Some(socket_lease) = state.inspection.resources.open_sockets.acquire() else {
                return error(StatusCode::SERVICE_UNAVAILABLE, "host_retired");
            };
            let Some(timer_lease) = state.inspection.resources.open_timers.acquire() else {
                return error(StatusCode::SERVICE_UNAVAILABLE, "host_retired");
            };
            state
                .inspection
                .physical_sse_connections
                .fetch_add(1, Ordering::SeqCst);
            let stream_state = state.clone();
            let stream_transport = transport.clone();
            let stream = futures_util::stream::unfold(
                (Some(first_event), socket_lease, timer_lease, reader),
                move |(first, socket_lease, timer_lease, reader)| {
                    let state = stream_state.clone();
                    let transport = stream_transport.clone();
                    async move {
                        let event = match first {
                            Some(event) => event,
                            None => {
                                tokio::time::sleep(Duration::from_millis(25)).await;
                                match state.async_runtime.sse_batch(&transport).await {
                                    Ok(event) => event,
                                    Err(_) => return None,
                                }
                            }
                        };
                        Some((
                            Ok::<_, Infallible>(axum::body::Bytes::from(event)),
                            (None, socket_lease, timer_lease, reader),
                        ))
                    }
                },
            );
            let mut response = Response::new(Body::from_stream(stream));
            *response.status_mut() = StatusCode::OK;
            response.headers_mut().insert(
                CONTENT_TYPE,
                HeaderValue::from_static("text/event-stream; charset=utf-8"),
            );
            response
                .headers_mut()
                .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(code) => error(StatusCode::UNAUTHORIZED, code),
    }
}

async fn websocket(
    State(state): State<Arc<HostState>>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    if let Some(response) = session_error(&headers) {
        return response;
    }
    if header(&headers, ORIGIN.as_str()) != Some(state.origin.as_str()) {
        return error(StatusCode::FORBIDDEN, "websocket_origin_rejected");
    }
    let Some(transport) = header(&headers, "x-live-transport").map(ToOwned::to_owned) else {
        return error(StatusCode::UNAUTHORIZED, "transport_authority_missing");
    };
    let reader = match state.async_runtime.acquire_reader(
        &transport,
        suprnova_live::async_updates::DocumentTransportKind::WebSocket,
    ) {
        Ok(reader) => reader,
        Err(code) => return error(StatusCode::CONFLICT, code),
    };
    let Some(timer_lease) = state.inspection.resources.open_timers.acquire() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "host_retired");
    };
    state
        .inspection
        .physical_websocket_connections
        .fetch_add(1, Ordering::SeqCst);
    let Some(socket_lease) = state.inspection.resources.open_sockets.acquire() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "host_retired");
    };
    upgrade
        .on_upgrade(move |socket| {
            websocket_session(state, transport, socket, socket_lease, timer_lease, reader)
        })
        .into_response()
}

async fn websocket_session(
    state: Arc<HostState>,
    transport: String,
    mut socket: WebSocket,
    _socket_lease: ResourceLease,
    _timer_lease: ResourceLease,
    _reader: async_updates::TransportReaderLease,
) {
    const MAX_CONTROLS_PER_CONNECTION: usize = 64;
    let mut accepted = 0_usize;
    let mut shutdown = state.request_shutdown.clone();
    let mut events = tokio::time::interval(Duration::from_millis(25));
    events.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    events.tick().await;
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    let _ = socket.close().await;
                    break;
                }
            }
            _ = events.tick() => {
                let Ok(messages) = state.async_runtime.websocket_batch(&transport) else {
                    break;
                };
                for response in messages {
                    let Ok(response) = String::from_utf8(response) else { return; };
                    if socket.send(Message::Text(response)).await.is_err() { return; }
                }
            }
            message = socket.next() => {
                let Some(Ok(message)) = message else { break; };
                let Message::Text(text) = message else {
                    if matches!(message, Message::Close(_)) { break; }
                    continue;
                };
                if text.len() > 512 || accepted >= MAX_CONTROLS_PER_CONNECTION {
                    let _ = socket.close().await;
                    break;
                }
                accepted += 1;
                match state.async_runtime.websocket_control(&transport, text.as_bytes()).await {
                    Ok(outcome) => {
                        for response in outcome.messages {
                            let Ok(response) = String::from_utf8(response) else { return; };
                            if socket.send(Message::Text(response)).await.is_err() { return; }
                        }
                    }
                    Err(_) => {
                        let _ = socket.close().await;
                        break;
                    }
                }
            }
        }
    }
}

async fn static_asset(State(state): State<Arc<HostState>>, uri: Uri) -> Response {
    if uri.query().is_some() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if uri.path() == "/suprnova-live.assets.json" {
        return exact_bytes(
            state.artifacts.manifest().to_vec(),
            "application/json; charset=utf-8",
            "no-store",
        );
    }
    let Some(asset) = state.artifacts.asset(uri.path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    exact_bytes(
        asset.bytes.clone(),
        &asset.content_type,
        &asset.cache_control,
    )
}

async fn fresh_render_scenario(State(state): State<Arc<HostState>>) -> Response {
    let island = state.async_runtime.fresh_render_document().await;
    let body = format!(
        r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Reference fresh render</title></head>
<body>
<script id="suprnova-live-config" type="application/json">{{"asset_identity":"reference-host","credentials":"same-origin","endpoint":"/__live/async/poll","max_parallel_per_island":1,"max_queued_per_island":8,"max_response_bytes":65536,"protocol":{{"maximum":2,"minimum":2}},"request_timeout_ms":5000,"runtime_contract_version":1}}</script>
{island}
<script type="module">
import {{ configureAsync }} from "/suprnova-live.async.esm.js";
import {{ boot }} from "/suprnova-live.esm.js";
const issued = await fetch("/__live/async/transports", {{
  body: JSON.stringify({{ kind: "sse", subscription: "orders" }}),
  headers: {{ "Authorization": "{REFERENCE_AUTHORIZATION}", "Content-Type": "application/json" }},
  method: "POST"
}}).then((response) => response.json());
const membership = issued.memberships[0];
const evidence = {{ acceptedRevision: null, requests: 0 }};
Object.defineProperty(window, "__suprnovaFreshRender", {{ value: evidence }});
configureAsync({{
  clock: {{ now: () => Date.now() }},
  randomness: {{ number: () => 0.5 }},
  timers: {{
    clearTimeout: (handle) => window.clearTimeout(handle),
    timeout: (callback, milliseconds) => window.setTimeout(callback, milliseconds)
  }}
}});
boot({{
  diagnostics: "verbose",
  transport: {{
    async fetch(input, init) {{
      const headers = new Headers(init?.headers);
      headers.set("Authorization", "{REFERENCE_AUTHORIZATION}");
      headers.set("X-Live-Subscription", membership.subscription);
      headers.set("X-Live-Subscription-Authority", membership.authority);
      evidence.requests += 1;
      const response = await window.fetch(input, {{ ...init, headers }});
      if (response.ok) {{
        const value = await response.clone().json();
        evidence.acceptedRevision = value.accepted_revision;
      }}
      return response;
    }}
  }}
}});
</script>
</body>
</html>"#
    );
    exact_bytes(body.into_bytes(), "text/html; charset=utf-8", "no-store")
}

fn exact_bytes(bytes: Vec<u8>, content_type: &str, cache_control: &str) -> Response {
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_str(content_type).expect("validated content type"),
    );
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_str(cache_control).expect("validated cache policy"),
    );
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn session_error(headers: &HeaderMap) -> Option<Response> {
    if header(headers, AUTHORIZATION.as_str()) == Some(REFERENCE_AUTHORIZATION) {
        None
    } else {
        Some(error(StatusCode::UNAUTHORIZED, "session_authority_invalid"))
    }
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn async_result(result: Result<Value, &'static str>, status: StatusCode) -> Response {
    match result {
        Ok(value) => (status, Json(value)).into_response(),
        Err(code) => error(StatusCode::BAD_REQUEST, code),
    }
}

fn upload_error(upload: UploadError) -> Response {
    let status = match upload.kind() {
        UploadErrorKind::InvalidGrantEncoding
        | UploadErrorKind::InvalidGrant
        | UploadErrorKind::GrantExpired
        | UploadErrorKind::ScopeMismatch
        | UploadErrorKind::RequestAuthorityExpired => StatusCode::UNAUTHORIZED,
        UploadErrorKind::AuthorizationDenied => StatusCode::FORBIDDEN,
        UploadErrorKind::InvalidHandle => StatusCode::NOT_FOUND,
        UploadErrorKind::InputTooLarge | UploadErrorKind::FileCountExceeded => {
            StatusCode::PAYLOAD_TOO_LARGE
        }
        UploadErrorKind::CreationRateExceeded
        | UploadErrorKind::PendingLimitExceeded
        | UploadErrorKind::ResourceExhausted => StatusCode::TOO_MANY_REQUESTS,
        UploadErrorKind::UploadConflict
        | UploadErrorKind::InvalidTransition
        | UploadErrorKind::RevisionExhausted
        | UploadErrorKind::IdempotencyHistoryFull
        | UploadErrorKind::StorageConflict
        | UploadErrorKind::ChecksumMismatch
        | UploadErrorKind::IncompleteTransfer
        | UploadErrorKind::ValidationEvidenceUnavailable
        | UploadErrorKind::UploadExpired
        | UploadErrorKind::ReconciliationRequired => StatusCode::CONFLICT,
        UploadErrorKind::BodyInterrupted | UploadErrorKind::TransferCanceled => {
            StatusCode::REQUEST_TIMEOUT
        }
        UploadErrorKind::UnsupportedProtocol
        | UploadErrorKind::DuplicateField
        | UploadErrorKind::UnsupportedOperation
        | UploadErrorKind::UnknownField
        | UploadErrorKind::MissingField
        | UploadErrorKind::InvalidField
        | UploadErrorKind::MediaHeaderUnproved => StatusCode::BAD_REQUEST,
        UploadErrorKind::AuthorizationUnavailable
        | UploadErrorKind::LedgerUnavailable
        | UploadErrorKind::ServiceRetired
        | UploadErrorKind::RandomUnavailable
        | UploadErrorKind::ProviderUnavailable
        | UploadErrorKind::FinalizationFailed
        | UploadErrorKind::CompensationFailed => StatusCode::SERVICE_UNAVAILABLE,
    };
    error(status, upload.kind().as_str())
}

fn error(status: StatusCode, code: &str) -> Response {
    (status, Json(json!({"error": code}))).into_response()
}
