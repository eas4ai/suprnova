//! Framework-owned HTTP and WebSocket adaptation for authorized asynchronous updates.
//!
//! Three reserved control and stream routes plus one WebSocket route carry
//! the browser runtime's asynchronous-update protocol. Every request is
//! admitted only after the ordinary Live security facts were recorded; the
//! routes then delegate all authority decisions to [`AsyncState`].

use std::convert::Infallible;
use std::sync::Arc;

use hyper::Method;
use serde::Deserialize;
use suprnova_live::async_updates::{
    AsyncTransportErrorKind, SseResponseContract, StreamEpoch, StreamName, StreamPosition,
    StreamSequence, VerifiedOrigin, WebSocketAuthentication, WebSocketCodec,
    WebSocketControlRecord, WebSocketFrame, WebSocketOriginPolicy,
};
use suprnova_live::host::HostScopeFacts;
use suprnova_live::identity::UnixMillis;
use tokio_tungstenite::tungstenite::Message;

use crate::ws::{WebSocketHandler, WsSocket};
use crate::{FrameworkError, HttpResponse, Request, Response, async_trait};

use super::async_updates::{
    AsyncErrorKind, AsyncState, MAX_CONTROL_NONCE_BYTES, MAX_DOCUMENT_INSTANCE_BYTES,
    MIN_DOCUMENT_INSTANCE_BYTES, TransportKey, TransportKind, browser_safe_generation,
};
use super::runtime::LiveRuntime;

const PROTOCOL_HEADER_VALUE: &str = "async-v1";
const MAX_CONTROL_BODY_BYTES: usize = 16 * 1024;
const MAX_SOCKET_CONTROL_TEXT_BYTES: usize = 512;
const BEARER_SCHEME: &str = "SuprnovaAsync ";
const POLICY_VIOLATION: u16 = 1008;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubscriptionControl {
    protocol_version: u16,
    operation: String,
    transport: String,
    stream: String,
    island: IslandReference,
    document_instance: String,
    #[serde(default)]
    prior: Option<PriorSubscription>,
    #[serde(default)]
    position: Option<WirePosition>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IslandReference {
    component: String,
    slot: String,
    document_key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PriorSubscription {
    subscription_id: String,
    descriptor_binding: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePosition {
    epoch: String,
    sequence: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MembershipControl {
    protocol_version: u16,
    operation: String,
    subscription_id: String,
    descriptor_binding: String,
    stream: String,
    control_nonce: String,
    transport_generation: u64,
}

/// Issues or renews one logical subscription for a validated island mount.
pub(crate) async fn subscriptions(request: Request) -> Response {
    Ok(match control(request, ControlRoute::Subscriptions).await {
        Ok(response) => response,
        Err(kind) => error_response(kind),
    })
}

/// Adds or removes one authenticated membership on an open SSE transport.
pub(crate) async fn memberships(request: Request) -> Response {
    Ok(match control(request, ControlRoute::Memberships).await {
        Ok(response) => response,
        Err(kind) => error_response(kind),
    })
}

/// Opens the single reader of one SSE document transport.
pub(crate) async fn events(request: Request) -> Response {
    Ok(match events_inner(request).await {
        Ok(response) => response,
        Err(kind) => error_response(kind),
    })
}

#[derive(Clone, Copy)]
enum ControlRoute {
    Subscriptions,
    Memberships,
}

async fn control(request: Request, route: ControlRoute) -> Result<HttpResponse, AsyncErrorKind> {
    if request.method() != Method::POST {
        return Ok(closed_response(405).header("Allow", "POST"));
    }
    if request.header("x-suprnova-live") != Some(PROTOCOL_HEADER_VALUE) {
        return Err(AsyncErrorKind::ProtocolInvalid);
    }
    if request.header("content-type") != Some("application/json") {
        return Ok(closed_response(415));
    }
    let (runtime, state) = bind_state()?;
    let request = match request.buffer_body(MAX_CONTROL_BODY_BYTES).await {
        Ok(request) => request,
        Err(error) if error.status_code() == 413 => return Ok(closed_response(413)),
        Err(_) => return Err(AsyncErrorKind::Unavailable),
    };
    let body = request.cached_body().ok_or(AsyncErrorKind::Unavailable)?;
    if !matches!(
        serde_json::from_slice::<serde_json::Value>(body),
        Ok(serde_json::Value::Object(_))
    ) {
        return Err(AsyncErrorKind::ProtocolInvalid);
    }
    if crate::auth::guard::Auth::id().is_none() {
        return Err(AsyncErrorKind::AuthorizationDenied);
    }
    match route {
        ControlRoute::Subscriptions => subscription_control(&runtime, &state, &request, body).await,
        ControlRoute::Memberships => membership_control(&state, &request, body).await,
    }
}

async fn subscription_control(
    runtime: &LiveRuntime,
    state: &Arc<AsyncState>,
    request: &Request,
    body: &[u8],
) -> Result<HttpResponse, AsyncErrorKind> {
    let control: SubscriptionControl =
        serde_json::from_slice(body).map_err(|_| AsyncErrorKind::ProtocolInvalid)?;
    if control.protocol_version != 1 {
        return Err(AsyncErrorKind::ProtocolInvalid);
    }
    let kind = TransportKind::parse(&control.transport).ok_or(AsyncErrorKind::TransportInvalid)?;
    if !valid_document_instance(&control.document_instance) {
        return Err(AsyncErrorKind::ProtocolInvalid);
    }
    let stream = StreamName::parse(&control.stream).map_err(|_| AsyncErrorKind::StreamUnknown)?;
    let origin = request_origin(request)?;
    let now = state.now()?;
    match control.operation.as_str() {
        "issue" => {
            if control.prior.is_some() || control.position.is_some() {
                return Err(AsyncErrorKind::ProtocolInvalid);
            }
            let baseline = StreamPosition::new(StreamEpoch::new(now.get()), StreamSequence::new(0));
            let (context, parameters) = runtime.validate_async_request_context(
                request,
                &control.island.component,
                &control.island.slot,
                &control.island.document_key,
                baseline,
            )?;
            let view = state
                .issue(
                    &context,
                    parameters,
                    stream,
                    kind,
                    &control.document_instance,
                    origin,
                    baseline,
                )
                .await?;
            Ok(json_response(201, view.value))
        }
        "renew" => {
            let (Some(prior), Some(position)) = (control.prior, control.position) else {
                return Err(AsyncErrorKind::ProtocolInvalid);
            };
            let epoch = parse_position_part(&position.epoch)?;
            let sequence = parse_position_part(&position.sequence)?;
            let baseline =
                StreamPosition::new(StreamEpoch::new(epoch), StreamSequence::new(sequence));
            let (context, _) = runtime.validate_async_request_context(
                request,
                &control.island.component,
                &control.island.slot,
                &control.island.document_key,
                baseline,
            )?;
            let view = state
                .renew(
                    &context,
                    &stream,
                    kind,
                    &control.document_instance,
                    &prior.subscription_id,
                    &prior.descriptor_binding,
                    (epoch, sequence),
                )
                .await?;
            Ok(json_response(200, view.value))
        }
        _ => Err(AsyncErrorKind::ProtocolInvalid),
    }
}

async fn membership_control(
    state: &Arc<AsyncState>,
    request: &Request,
    body: &[u8],
) -> Result<HttpResponse, AsyncErrorKind> {
    let control: MembershipControl =
        serde_json::from_slice(body).map_err(|_| AsyncErrorKind::ProtocolInvalid)?;
    if control.protocol_version != 1 {
        return Err(AsyncErrorKind::ProtocolInvalid);
    }
    if control.control_nonce.is_empty() || control.control_nonce.len() > MAX_CONTROL_NONCE_BYTES {
        return Err(AsyncErrorKind::ProtocolInvalid);
    }
    if !browser_safe_generation(control.transport_generation) {
        return Err(AsyncErrorKind::GenerationInvalid);
    }
    let credential = bearer_credential(request).ok_or(AsyncErrorKind::AuthorityMissing)?;
    let facts = request_scope_facts(request, state.now()?)?;
    let key = state.transport_for_credential(&credential, &facts)?;
    state.admit_sse_control(&key, control.transport_generation, &control.control_nonce)?;
    let expected = Some((control.descriptor_binding.as_str(), control.stream.as_str()));
    let kind = match control.operation.as_str() {
        "subscribe" => {
            state
                .add_membership(&key, &control.subscription_id, expected, None)
                .await?;
            "authenticated"
        }
        "unsubscribe" => {
            state
                .remove_membership(&key, &control.subscription_id, expected, false)
                .await?;
            "released"
        }
        _ => return Err(AsyncErrorKind::ProtocolInvalid),
    };
    Ok(json_response(
        200,
        serde_json::json!({
            "kind": kind,
            "operation": control.operation,
            "subscription_id": control.subscription_id,
            "descriptor_binding": control.descriptor_binding,
            "stream": control.stream,
            "control_nonce": control.control_nonce,
            "transport_generation": control.transport_generation,
        }),
    ))
}

async fn events_inner(request: Request) -> Result<HttpResponse, AsyncErrorKind> {
    if request.method() != Method::GET {
        return Ok(closed_response(405).header("Allow", "GET"));
    }
    if !accepts_event_stream(&request) {
        return Ok(closed_response(406));
    }
    let credential = bearer_credential(&request).ok_or(AsyncErrorKind::AuthorityMissing)?;
    let generation = request
        .header("suprnova-transport-generation")
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|generation| browser_safe_generation(*generation))
        .ok_or(AsyncErrorKind::GenerationInvalid)?;
    let (_, state) = bind_state()?;
    if crate::auth::guard::Auth::id().is_none() {
        return Err(AsyncErrorKind::AuthorityInvalid);
    }
    let facts = request_scope_facts(&request, state.now()?)?;
    let key = state.transport_for_credential(&credential, &facts)?;
    let mut receiver = state.open_sse_reader(&key, generation)?;
    let stream = futures::stream::poll_fn(move |context| {
        receiver
            .poll_recv(context)
            .map(|frame| frame.map(Ok::<_, Infallible>))
    });
    let mut response = HttpResponse::stream_bytes(stream);
    for (name, value) in &SseResponseContract::headers() {
        if let Ok(value) = value.to_str() {
            response = response.header(name.as_str(), value);
        }
    }
    Ok(response.header("Referrer-Policy", "no-referrer"))
}

/// Same-origin WebSocket document transport for authenticated memberships.
pub(crate) struct AsyncSocketHandler;

#[async_trait]
impl WebSocketHandler for AsyncSocketHandler {
    async fn handle(&self, mut socket: WsSocket, request: Request) -> Result<(), FrameworkError> {
        let Ok((_, state)) = bind_state() else {
            return close(&mut socket, "unavailable").await;
        };
        let Ok(origin) = socket_origin(&request) else {
            return close(&mut socket, "origin_invalid").await;
        };
        let Ok(now) = state.now() else {
            return close(&mut socket, "unavailable").await;
        };
        let Ok(facts) = request_scope_facts(&request, now) else {
            return close(&mut socket, "membership_authority_invalid").await;
        };
        let authenticated = request.auth_user_id().is_some();
        let (key, mut outbound) = match state.open_socket(&facts, origin) {
            Ok(opened) => opened,
            Err(kind) => return close(&mut socket, kind.socket_reason()).await,
        };
        let codec = WebSocketCodec::v1();
        let outcome = socket_session(
            &state,
            &key,
            &codec,
            authenticated,
            &mut socket,
            &mut outbound,
        )
        .await;
        state.retire_transport(&key, 0).await;
        match outcome {
            Ok(()) => Ok(()),
            Err(reason) => close(&mut socket, reason).await,
        }
    }
}

async fn socket_session(
    state: &Arc<AsyncState>,
    key: &TransportKey,
    codec: &WebSocketCodec,
    authenticated: bool,
    socket: &mut WsSocket,
    outbound: &mut tokio::sync::mpsc::Receiver<bytes::Bytes>,
) -> Result<(), &'static str> {
    loop {
        tokio::select! {
            frame = outbound.recv() => {
                let Some(frame) = frame else {
                    return Ok(());
                };
                let text = String::from_utf8(frame.to_vec()).map_err(|_| "unavailable")?;
                if socket.send_text(text).await.is_err() {
                    return Ok(());
                }
            }
            message = socket.recv() => {
                match message {
                    Ok(Some(Message::Text(text))) => {
                        socket_text(state, key, codec, authenticated, socket, &text).await?;
                    }
                    Ok(Some(Message::Binary(_))) => return Err("unsupported_frame"),
                    Ok(Some(Message::Close(_)) | None) | Err(_) => return Ok(()),
                    Ok(Some(_)) => {}
                }
            }
        }
    }
}

async fn socket_text(
    state: &Arc<AsyncState>,
    key: &TransportKey,
    codec: &WebSocketCodec,
    authenticated: bool,
    socket: &mut WsSocket,
    text: &str,
) -> Result<(), &'static str> {
    if text.len() > MAX_SOCKET_CONTROL_TEXT_BYTES {
        return Err("frame_too_large");
    }
    let frame = || WebSocketFrame::Text {
        payload: text.as_bytes(),
        final_fragment: true,
    };
    match codec.decode_membership_request(frame()) {
        Ok(membership) => {
            if !authenticated {
                return Err("membership_authority_invalid");
            }
            state
                .admit_socket_control(key, membership.transport_generation())
                .map_err(AsyncErrorKind::socket_reason)?;
            let subscription = membership.subscription().to_base64url();
            let acknowledgment = state
                .add_membership(key, &subscription, None, Some(membership))
                .await
                .map_err(AsyncErrorKind::socket_reason)?
                .ok_or("unavailable")?;
            let encoded = codec
                .encode_membership_acknowledgment(&acknowledgment)
                .map_err(|_| "unavailable")?;
            let text = String::from_utf8(encoded).map_err(|_| "unavailable")?;
            let _ = socket.send_text(text).await;
            Ok(())
        }
        Err(error) if error.kind() == AsyncTransportErrorKind::FrameTooLarge => {
            Err("frame_too_large")
        }
        Err(_) => match codec.decode_control(frame()) {
            Ok(WebSocketControlRecord::Unsubscribe(subscription)) => {
                if !authenticated {
                    return Err("membership_authority_invalid");
                }
                state
                    .count_socket_control(key)
                    .map_err(AsyncErrorKind::socket_reason)?;
                state
                    .remove_membership(key, &subscription.to_base64url(), None, true)
                    .await
                    .map_err(AsyncErrorKind::socket_reason)
            }
            Ok(WebSocketControlRecord::Subscribe(_)) | Err(_) => Err("invalid_envelope"),
        },
    }
}

async fn close(socket: &mut WsSocket, reason: &'static str) -> Result<(), FrameworkError> {
    let _ = socket.close(POLICY_VIOLATION, reason).await;
    Ok(())
}

fn bind_state() -> Result<(LiveRuntime, Arc<AsyncState>), AsyncErrorKind> {
    let runtime = LiveRuntime::bind().map_err(|_| AsyncErrorKind::Unavailable)?;
    let state = Arc::clone(runtime.async_state());
    Ok((runtime, state))
}

fn request_scope_facts(
    request: &Request,
    now: UnixMillis,
) -> Result<HostScopeFacts, AsyncErrorKind> {
    super::context::request_host_scope_facts(request, now)
        .map_err(|_| AsyncErrorKind::ContextRejected)
}

/// Derives the exact application origin the browser document was served from.
fn request_origin(request: &Request) -> Result<VerifiedOrigin, AsyncErrorKind> {
    let host = request.http_host().ok_or(AsyncErrorKind::ProtocolInvalid)?;
    VerifiedOrigin::parse(&format!("{}://{host}", request.scheme()))
        .map_err(|_| AsyncErrorKind::ProtocolInvalid)
}

/// Verifies the upgrade `Origin` against the application origin with the engine policy.
fn socket_origin(request: &Request) -> Result<VerifiedOrigin, AsyncErrorKind> {
    let application = request_origin(request)?;
    let policy = WebSocketOriginPolicy::new(application, Vec::new())
        .map_err(|_| AsyncErrorKind::Unavailable)?;
    let origin_header = request
        .header("origin")
        .ok_or(AsyncErrorKind::ContextRejected)?;
    policy
        .authorize_upgrade(&[origin_header], || Ok(WebSocketAuthentication::Cookie(())))
        .map(|upgrade| upgrade.origin().clone())
        .map_err(|_| AsyncErrorKind::ContextRejected)
}

fn bearer_credential(request: &Request) -> Option<String> {
    let value = request.header("authorization")?;
    let credential = value.strip_prefix(BEARER_SCHEME)?.trim();
    (!credential.is_empty() && credential.len() <= 1_024 && credential.is_ascii())
        .then(|| credential.to_owned())
}

fn accepts_event_stream(request: &Request) -> bool {
    request.header("accept").is_some_and(|accept| {
        accept
            .split(',')
            .map(|part| part.split(';').next().unwrap_or("").trim())
            .any(|media| media.eq_ignore_ascii_case("text/event-stream"))
    })
}

fn valid_document_instance(value: &str) -> bool {
    (MIN_DOCUMENT_INSTANCE_BYTES..=MAX_DOCUMENT_INSTANCE_BYTES).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn parse_position_part(value: &str) -> Result<u64, AsyncErrorKind> {
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(AsyncErrorKind::ProtocolInvalid);
    }
    if value.len() > 1 && value.starts_with('0') {
        return Err(AsyncErrorKind::ProtocolInvalid);
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|parsed| *parsed == 0 || browser_safe_generation(*parsed))
        .ok_or(AsyncErrorKind::ProtocolInvalid)
}

fn error_response(kind: AsyncErrorKind) -> HttpResponse {
    json_response(kind.status(), serde_json::json!({ "error": kind.code() }))
}

fn json_response(status: u16, body: serde_json::Value) -> HttpResponse {
    HttpResponse::json(body)
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("Referrer-Policy", "no-referrer")
        .status(status)
}

fn closed_response(status: u16) -> HttpResponse {
    HttpResponse::new()
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("Referrer-Policy", "no-referrer")
        .header(
            "Content-Security-Policy",
            "default-src 'none'; frame-ancestors 'none'",
        )
        .header("Content-Length", "0")
        .status(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_instances_are_bounded_opaque_segments() {
        assert!(valid_document_instance("doc-instance-0001"));
        assert!(!valid_document_instance("short"));
        assert!(!valid_document_instance(&"x".repeat(65)));
        assert!(!valid_document_instance("doc instance 0001"));
    }

    #[test]
    fn position_parts_are_canonical_browser_safe_decimals() {
        assert_eq!(parse_position_part("0"), Ok(0));
        assert_eq!(parse_position_part("42"), Ok(42));
        assert!(parse_position_part("").is_err());
        assert!(parse_position_part("007").is_err());
        assert!(parse_position_part("9007199254740992").is_err());
        assert!(parse_position_part("-1").is_err());
    }

    #[test]
    fn stream_acceptance_ignores_parameters_and_case() {
        let request = Request::for_test_with_headers(
            "GET",
            "/__live/v1/async/events",
            [("accept", "text/html, TEXT/EVENT-STREAM;q=0.9")],
        );
        assert!(accepts_event_stream(&request));
        let request = Request::for_test_with_headers(
            "GET",
            "/__live/v1/async/events",
            [("accept", "application/json")],
        );
        assert!(!accepts_event_stream(&request));
    }

    #[test]
    fn bearer_credentials_require_the_async_scheme() {
        let request = Request::for_test_with_headers(
            "POST",
            "/__live/v1/async/memberships",
            [("authorization", "SuprnovaAsync abc123")],
        );
        assert_eq!(bearer_credential(&request).as_deref(), Some("abc123"));
        let request = Request::for_test_with_headers(
            "POST",
            "/__live/v1/async/memberships",
            [("authorization", "Bearer abc123")],
        );
        assert!(bearer_credential(&request).is_none());
    }
}
