//! Thin Rust reference host for Task 9 real-browser async conformance.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use suprnova_live::async_updates::{SseEncoder, SseResponseContract};
use suprnova_live::identity::UnixMillis;
use suprnova_live_test_support::{
    ASYNC_REFERENCE_ORIGIN, AsyncReferenceAuthority, AsyncReferenceAuthorizationRequest,
    AsyncReferenceMembershipRequest, AsyncReferenceScenario,
};

const ADDRESS: &str = "127.0.0.1:4174";
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_ARCHIVED_STREAMS: usize = 8;
const INITIAL_SNAPSHOT: &str = "eyJib2R5Ijp7ImJ1aWxkX2lkIjoiYnVpbGQtMjAyNi0wOC0yMSIsImNvbXBvbmVudCI6eyJjb250cmFjdF9kaWdlc3QiOiJJQ0VpSXlRbEppY29LU29yTEMwdUx6QXhNak0wTlRZM09EazZPenc5UGo4IiwibWVtb19zY2hlbWFfdmVyc2lvbiI6MSwibW91bnRfc2NoZW1hX3ZlcnNpb24iOjEsIm5hbWUiOiJjYXRhbG9nLnNlYXJjaCIsInN0YXRlX3NjaGVtYV92ZXJzaW9uIjoxfSwiZXhwaXJlc19hdCI6IjIwMDAiLCJleHRlbnNpb25zIjp7fSwiZm9ybSI6Imluc3RhbmNlIiwiaW5zdGFuY2VfaWQiOiJzTEd5czdTMXRyZTR1YnE3dkwyLXZ3IiwiaXNzdWVkX2F0IjoiMTAwMCIsImtleV9pZCI6InNuYXBzaG90LXYxIiwibWVtbyI6eyJwYWdlIjoxfSwicmV2aXNpb24iOiI3Iiwicm91dGUiOiJBUUlEQkFVR0J3Z0pDZ3NNRFE0UEVCRVNFeFFWRmhjWUdSb2JIQjBlSHlBIiwic2NoZW1hX3ZlcnNpb24iOjEsInNjb3BlIjoia0pHU2s1U1ZscGVZbVpxYm5KMmVuNkNob3FPa3BhYW5xS21xcTZ5dHJxOCIsInNsb3QiOiJzZWFyY2gtcmVzdWx0cyIsInN0YXRlIjp7InF1ZXJ5IjoicnVzdCIsInNlbGVjdGVkIjoiMSJ9fSwic2lnbmF0dXJlIjoicEE4OG1aMEhkNGpiOWpUcXZyTmZyd3BNRDRwa0lCNzRYZkhpT09oQ3B6RSJ9";

enum StreamCommand {
    Close,
    Envelope { encoded: String, sequence: u64 },
}

struct StreamRecord {
    id: u64,
    sender: mpsc::Sender<StreamCommand>,
}

struct HostState {
    authority: AsyncReferenceAuthority,
    authorization_calls: u64,
    effect_count: u64,
    late_attempts: u64,
    live_actions: u64,
    membership_controls: u64,
    recovery_allowed: bool,
    streams: Vec<StreamRecord>,
    archived: Vec<mpsc::Sender<StreamCommand>>,
}

impl HostState {
    fn new() -> Self {
        Self {
            authority: AsyncReferenceAuthority::new(now()),
            authorization_calls: 0,
            effect_count: 0,
            late_attempts: 0,
            live_actions: 0,
            membership_controls: 0,
            recovery_allowed: true,
            streams: Vec::new(),
            archived: Vec::new(),
        }
    }

    fn reset(&mut self) {
        for record in self.streams.drain(..) {
            let _ = record.sender.send(StreamCommand::Close);
        }
        *self = Self::new();
    }

    fn archive(&mut self, sender: mpsc::Sender<StreamCommand>) {
        if self.archived.len() == MAX_ARCHIVED_STREAMS {
            self.archived.remove(0);
        }
        self.archived.push(sender);
    }
}

struct Request {
    body: Vec<u8>,
    headers: BTreeMap<String, String>,
    method: String,
    target: String,
}

fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind(ADDRESS)?;
    let state = Arc::new(Mutex::new(HostState::new()));
    for accepted in listener.incoming() {
        let Ok(stream) = accepted else { continue };
        let shared = Arc::clone(&state);
        thread::spawn(move || {
            if let Err(error) = serve(stream, shared)
                && !matches!(
                    error.kind(),
                    std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::UnexpectedEof
                )
            {
                eprintln!("async reference request failed: {error}");
            }
        });
    }
    Ok(())
}

fn serve(mut stream: TcpStream, state: Arc<Mutex<HostState>>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let request = read_request(&mut stream)?;
    let path = request.target.split('?').next().unwrap_or(&request.target);
    if request.method == "OPTIONS" {
        return respond(&mut stream, 204, "text/plain", b"");
    }
    match (request.method.as_str(), path) {
        ("GET", "/health") => respond(&mut stream, 200, "text/plain", b"ok"),
        ("GET", "/scenario/asyncLifecycle") => scenario_document(&mut stream),
        ("GET", "/scenario/lifecycleDestination") => respond(
            &mut stream,
            200,
            "text/html; charset=utf-8",
            b"<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Lifecycle destination</title></head><body><main><h1>Lifecycle destination</h1></main></body></html>",
        ),
        ("POST", "/navigation/post") => navigation_post(&mut stream, &request),
        ("POST", "/authorize") => authorize(&mut stream, &request, &state),
        ("POST", "/membership") => membership(&mut stream, &request, &state),
        ("GET", "/__live/async/events") => event_stream(stream, request, state),
        ("GET", "/__live/async/websocket") => respond(
            &mut stream,
            426,
            "text/plain",
            b"websocket upgrade required",
        ),
        ("POST", "/__live/async/poll") => poll(&mut stream, &request, &state),
        ("POST", "/live") => live(&mut stream, &request, &state),
        ("POST", "/control/degrade") => {
            broadcast_gap(&state);
            respond(&mut stream, 204, "text/plain", b"")
        }
        ("POST", "/control/reconnect") => {
            state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .recovery_allowed = true;
            close_streams(&state);
            respond(&mut stream, 204, "text/plain", b"")
        }
        ("POST", "/control/current") => {
            broadcast_next_heartbeat(&state);
            respond(&mut stream, 204, "text/plain", b"")
        }
        ("POST", "/control/loss") => {
            close_streams(&state);
            respond(&mut stream, 204, "text/plain", b"")
        }
        ("POST", "/control/late") => late_work(&mut stream, &state),
        ("POST", "/control/reset") => {
            state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .reset();
            respond(&mut stream, 204, "text/plain", b"")
        }
        ("POST", "/control/close") => {
            broadcast_completion(&state);
            respond(&mut stream, 204, "text/plain", b"")
        }
        ("GET", "/diagnostics") => diagnostics(&mut stream, &state),
        _ => respond(&mut stream, 404, "text/plain", b"not found"),
    }
}

fn scenario_document(stream: &mut TcpStream) -> std::io::Result<()> {
    let body = async_island_body(0, false, false);
    let document = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Suprnova Live conformance</title></head><body>\
         <script nonce=\"suprnova-async-test\" id=\"suprnova-live-config\" type=\"application/json\">{{\"asset_identity\":\"suprnova-live-test-v1\",\"credentials\":\"same-origin\",\"endpoint\":\"/live\",\"max_parallel_per_island\":1,\"max_queued_per_island\":8,\"max_response_bytes\":1048576,\"protocol\":{{\"maximum\":2,\"minimum\":1}},\"request_timeout_ms\":5000,\"runtime_contract_version\":1}}</script>\
         <main><section data-suprnova-live-root=\"search-results\" data-suprnova-live-island data-suprnova-live-component=\"catalog.search\" data-suprnova-live-slot=\"search-results\" data-suprnova-live-document-key=\"primary\" data-suprnova-live-protocol-min=\"2\" data-suprnova-live-contract=\"1\" data-suprnova-live-snapshot-kind=\"instance\" data-suprnova-live-snapshot=\"{INITIAL_SNAPSHOT}\" data-suprnova-live-revision=\"7\" data-suprnova-live-lazy-complete=\"false\" data-suprnova-live-instance=\"sLGys7S1tre4ubq7vL2-vw\" live:stream=\"orders\" live:signal=\"open:false\" aria-busy=\"false\" data-live-stream-state=\"disconnected\" data-live-stream-motion=\"allowed\">{body}</section>\
         <button id=\"remove-island\" type=\"button\">Remove island</button>\
         <form action=\"/navigation/post\" method=\"post\"><label>Native value <input name=\"value\"></label><button type=\"submit\">Submit normally</button></form>\
         <a href=\"/scenario/lifecycleDestination\">Native destination</a></main>\
         <script type=\"module\" crossorigin=\"anonymous\" nonce=\"suprnova-async-test\" src=\"http://127.0.0.1:4173/test-async/lifecycle.js\"></script></body></html>"
    );
    let mut headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: private, max-age=0, must-revalidate\r\nContent-Security-Policy: default-src 'none'; script-src 'nonce-suprnova-async-test' http://127.0.0.1:4173; connect-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'\r\nConnection: close\r\n\r\n",
        document.len()
    )
    .into_bytes();
    headers.extend_from_slice(document.as_bytes());
    stream.write_all(&headers)
}

fn navigation_post(stream: &mut TcpStream, request: &Request) -> std::io::Result<()> {
    let body = String::from_utf8_lossy(&request.body)
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");
    respond(
        stream,
        200,
        "text/html; charset=utf-8",
        format!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>POST destination</title></head><body><main><h1>POST destination</h1><p id=\"post-body\">{body}</p></main></body></html>"
        )
        .as_bytes(),
    )
}

fn now() -> UnixMillis {
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis();
    UnixMillis::new(u64::try_from(milliseconds).unwrap_or(u64::MAX))
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Request> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end;
    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "request ended before headers",
            ));
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "request exceeded bound",
            ));
        }
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = index + 4;
            break;
        }
    }
    let header_text = std::str::from_utf8(&bytes[..header_end]).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "request headers were not UTF-8",
        )
    })?;
    let mut lines = header_text.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split_whitespace();
    let method = request_line.next().unwrap_or_default().to_owned();
    let target = request_line.next().unwrap_or_default().to_owned();
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.to_ascii_lowercase(), value.trim().to_owned()))
        })
        .collect::<BTreeMap<_, _>>();
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if header_end + content_length > MAX_REQUEST_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "request body exceeded bound",
        ));
    }
    while bytes.len() < header_end + content_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "request body truncated",
            ));
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(Request {
        body: bytes[header_end..header_end + content_length].to_vec(),
        headers,
        method,
        target,
    })
}

fn request_origin(request: &Request) -> &str {
    if let Some(origin) = request.headers.get("origin") {
        return origin;
    }
    if request
        .headers
        .get("host")
        .is_some_and(|host| host == ADDRESS)
        && request
            .headers
            .get("sec-fetch-site")
            .is_some_and(|site| site == "same-origin")
    {
        return ASYNC_REFERENCE_ORIGIN;
    }
    ""
}

fn bearer(request: &Request) -> &str {
    request
        .headers
        .get("authorization")
        .and_then(|value| value.strip_prefix("SuprnovaAsync "))
        .unwrap_or_default()
}

fn authorize(
    stream: &mut TcpStream,
    request: &Request,
    state: &Mutex<HostState>,
) -> std::io::Result<()> {
    let facts = match serde_json::from_slice::<AsyncReferenceAuthorizationRequest>(&request.body) {
        Ok(facts) => facts,
        Err(_) => return respond_error(stream, 400, "authorization_request_invalid"),
    };
    let mut locked = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locked.authorization_calls = locked.authorization_calls.saturating_add(1);
    if facts.prior_subscription_id.is_some() && !locked.recovery_allowed {
        return respond_error(stream, 409, "recovery_pending");
    }
    match locked
        .authority
        .authorize(request_origin(request), &facts, now())
    {
        Ok(value) => respond_json(stream, 200, &value),
        Err(reason) => respond_error(stream, 403, reason),
    }
}

fn membership(
    stream: &mut TcpStream,
    request: &Request,
    state: &Mutex<HostState>,
) -> std::io::Result<()> {
    let facts = match serde_json::from_slice::<AsyncReferenceMembershipRequest>(&request.body) {
        Ok(facts) => facts,
        Err(_) => return respond_error(stream, 400, "membership_request_invalid"),
    };
    let mut locked = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locked.membership_controls = locked.membership_controls.saturating_add(1);
    match locked
        .authority
        .membership(request_origin(request), bearer(request), &facts, now())
    {
        Ok(value) => respond_json(stream, 200, &value),
        Err(reason) => respond_error(stream, 403, reason),
    }
}

fn event_stream(
    mut stream: TcpStream,
    request: Request,
    state: Arc<Mutex<HostState>>,
) -> std::io::Result<()> {
    let (sender, receiver) = mpsc::channel();
    let transport_generation = request
        .headers
        .get("suprnova-transport-generation")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let transport = {
        let mut locked = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match locked.authority.open_transport(
            request_origin(&request),
            bearer(&request),
            transport_generation,
            now(),
        ) {
            Ok(transport) => {
                locked.streams.push(StreamRecord {
                    id: transport,
                    sender,
                });
                transport
            }
            Err(reason) => return respond_error(&mut stream, 403, reason),
        }
    };
    let result = write_event_stream(&mut stream, receiver, &state);
    let mut locked = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(index) = locked
        .streams
        .iter()
        .position(|record| record.id == transport)
    {
        let record = locked.streams.remove(index);
        locked.archive(record.sender);
    }
    locked.authority.close_transport(transport);
    result
}

fn write_event_stream(
    stream: &mut TcpStream,
    receiver: mpsc::Receiver<StreamCommand>,
    state: &Mutex<HostState>,
) -> std::io::Result<()> {
    let response_headers = SseResponseContract::headers();
    let header = |name: &str| -> std::io::Result<&str> {
        response_headers
            .get(name)
            .ok_or_else(|| std::io::Error::other("SSE response header missing"))?
            .to_str()
            .map_err(std::io::Error::other)
    };
    stream.write_all(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nCache-Control: {}\r\nX-Content-Type-Options: {}\r\nAccess-Control-Allow-Origin: {ASYNC_REFERENCE_ORIGIN}\r\nVary: Origin\r\nX-Accel-Buffering: {}\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n",
            header("content-type")?,
            header("cache-control")?,
            header("x-content-type-options")?,
            header("x-accel-buffering")?,
        )
        .as_bytes(),
    )?;
    stream.flush()?;
    write_chunk(stream, SseEncoder::heartbeat_comment())?;
    loop {
        match receiver.recv_timeout(Duration::from_secs(2)) {
            Ok(StreamCommand::Close) => {
                stream.write_all(b"0\r\n\r\n")?;
                return Ok(());
            }
            Ok(StreamCommand::Envelope { encoded, sequence }) => {
                write_chunk(
                    stream,
                    &AsyncReferenceScenario::lifecycle().sse_record(sequence, &encoded),
                )?;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                write_chunk(stream, SseEncoder::heartbeat_comment())?;
                broadcast_next_heartbeat(state);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

fn poll(
    stream: &mut TcpStream,
    request: &Request,
    state: &Mutex<HostState>,
) -> std::io::Result<()> {
    if request_origin(request) != ASYNC_REFERENCE_ORIGIN || bearer(request).is_empty() {
        return respond_error(stream, 403, "poll_authority_invalid");
    }
    let locked = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    respond_json(
        stream,
        200,
        &json!({
            "current_position": { "epoch": "1", "sequence": locked.authority.current_sequence().to_string() },
            "envelopes": [],
            "fallback": { "interval_ms": 30_000, "visibility": "visible" }
        }),
    )
}

fn live(
    stream: &mut TcpStream,
    request: &Request,
    state: &Mutex<HostState>,
) -> std::io::Result<()> {
    if request_origin(request) != ASYNC_REFERENCE_ORIGIN {
        return respond_error(stream, 403, "live_origin_invalid");
    }
    let parsed: Value = match serde_json::from_slice(&request.body) {
        Ok(value) => value,
        Err(_) => return respond_error(stream, 400, "live_request_invalid"),
    };
    if parsed.get("protocol_version") != Some(&json!(2)) {
        return respond_error(stream, 400, "live_protocol_invalid");
    }
    let correlation = parsed
        .get("correlation_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let base_revision = parsed
        .get("base_revision")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let revision = base_revision.saturating_add(1);
    let operations = parsed.get("operations").and_then(Value::as_array);
    let action = operations.and_then(|operations| {
        operations.iter().find_map(|operation| {
            (operation.get("kind")?.as_str()? == "invoke_action")
                .then(|| operation.get("name")?.as_str())
                .flatten()
        })
    });
    let fresh_render = operations.is_some_and(|operations| {
        operations
            .iter()
            .any(|operation| operation.get("kind") == Some(&Value::String("fresh_render".into())))
    });
    let mut snapshot = parsed
        .pointer("/snapshot/envelope")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if let Some(body) = snapshot.get_mut("body").and_then(Value::as_object_mut) {
        body.insert("form".to_owned(), Value::String("instance".to_owned()));
        body.insert("revision".to_owned(), Value::String(revision.to_string()));
    }
    let encoded_snapshot =
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&snapshot).map_err(std::io::Error::other)?);
    let instance = snapshot
        .pointer("/body/instance_id")
        .and_then(Value::as_str)
        .unwrap_or("sLGys7S1tre4ubq7vL2-vw");
    let document_key = parsed
        .pointer("/extensions/x_suprnova_live_document_key_v1")
        .and_then(Value::as_str)
        .unwrap_or("primary");
    let mut locked = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let replacement = action == Some("replace_stream");
    if action == Some("save") {
        locked.live_actions = locked.live_actions.saturating_add(1);
    }
    if fresh_render {
        locked.effect_count = locked.effect_count.saturating_add(1);
    }
    let body = async_island_body(
        if fresh_render { locked.effect_count } else { 0 },
        action == Some("save"),
        replacement,
    );
    let async_attributes = if replacement {
        " live:poll=\"\" aria-busy=\"false\""
    } else {
        " live:stream=\"orders\" live:signal=\"open:false\" aria-busy=\"false\" data-live-stream-state=\"current\" data-live-stream-motion=\"allowed\""
    };
    let html = format!(
        "<section data-suprnova-live-root=\"search-results\" data-suprnova-live-island data-suprnova-live-component=\"catalog.search\" data-suprnova-live-slot=\"search-results\" data-suprnova-live-document-key=\"{document_key}\" data-suprnova-live-protocol-min=\"2\" data-suprnova-live-contract=\"1\" data-suprnova-live-snapshot-kind=\"instance\" data-suprnova-live-snapshot=\"{encoded_snapshot}\" data-suprnova-live-revision=\"{revision}\" data-suprnova-live-lazy-complete=\"false\" data-suprnova-live-instance=\"{instance}\"{async_attributes}>{body}</section>"
    );
    let response = json!({
        "accepted_revision": revision.to_string(),
        "child_deliveries": [],
        "correlation_id": correlation,
        "effects": [],
        "events": [],
        "extensions": {},
        "outcome": "accepted",
        "protocol_version": 2,
        "render": { "html": html, "kind": "html" },
        "snapshot": snapshot,
        "url_intent": null,
        "validation": {}
    });
    let media_type = request.headers.get("content-type").map_or(
        "application/vnd.suprnova.live+json; charset=utf-8; version=2",
        String::as_str,
    );
    respond_json_with_type(stream, 200, media_type, &response)
}

fn async_island_body(effect_count: u64, action_committed: bool, replacement: bool) -> String {
    if replacement {
        return "<h1>Async order updates</h1><p data-live-stream-status aria-label=\"Order updates\">Server rendered status baseline</p><p id=\"async-content\">Morphed async content</p>"
            .to_owned();
    }
    format!(
        "<h1>Async order updates</h1>\
         <p data-live-stream-status aria-label=\"Order updates\">Updates current</p>\
         <p id=\"async-content\">Server-rendered async content</p>\
         <output id=\"async-effect-count\" aria-label=\"Applied async effects\">{effect_count}</output>\
         <button id=\"keep-focus\" type=\"button\">Keep focus</button>\
         <button id=\"degrade-stream\" type=\"button\">Degrade stream</button>\
         <button id=\"reconnect-stream\" type=\"button\">Reconnect stream</button>\
         <button id=\"close-stream\" type=\"button\">Close stream</button>\
         <button id=\"replace-island\" type=\"button\" live:click.prevent=\"replace_stream\">Replace island contents</button>\
         <button id=\"run-live-action\" type=\"button\" live:click.prevent=\"save\">Run Live action</button>\
         <output id=\"async-action-result\">{}</output>\
         <button id=\"local-toggle\" type=\"button\" live:toggle=\"open\">Local details</button>\
         <p id=\"local-panel\" hidden aria-hidden=\"true\" inert live:show=\"open\">Local signal remains available</p>",
        if action_committed {
            "Live action committed"
        } else {
            ""
        }
    )
}

fn broadcast_gap(state: &Mutex<HostState>) {
    let mut locked = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (sequence, encoded) = locked.authority.sequence_gap();
    locked.recovery_allowed = true;
    broadcast(&mut locked.streams, sequence, &encoded);
}

fn broadcast_next_heartbeat(state: &Mutex<HostState>) {
    let mut locked = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (sequence, encoded) = locked.authority.next_heartbeat();
    broadcast(&mut locked.streams, sequence, &encoded);
}

fn broadcast_completion(state: &Mutex<HostState>) {
    let mut locked = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (sequence, encoded) = locked.authority.completion();
    broadcast(&mut locked.streams, sequence, &encoded);
}

fn broadcast(streams: &mut Vec<StreamRecord>, sequence: u64, encoded: &str) {
    streams.retain(|record| {
        record
            .sender
            .send(StreamCommand::Envelope {
                encoded: encoded.to_owned(),
                sequence,
            })
            .is_ok()
    });
}

fn close_streams(state: &Mutex<HostState>) {
    let mut locked = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let drained = locked.streams.drain(..).collect::<Vec<_>>();
    for record in drained {
        locked.authority.close_transport(record.id);
        let _ = record.sender.send(StreamCommand::Close);
        locked.archive(record.sender);
    }
    locked.authority.retire_membership();
}

fn late_work(stream: &mut TcpStream, state: &Mutex<HostState>) -> std::io::Result<()> {
    let mut locked = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let sequence = locked.authority.current_sequence().saturating_add(1);
    let encoded = AsyncReferenceScenario::lifecycle().heartbeat(sequence);
    let old_transport_attempted = locked.archived.iter().any(|sender| {
        let _ = sender.send(StreamCommand::Envelope {
            encoded: encoded.clone(),
            sequence,
        });
        true
    });
    locked.late_attempts = locked.late_attempts.saturating_add(3);
    respond_json(
        stream,
        200,
        &json!({
            "attempts": 3,
            "kinds": ["old_transport_envelope", "old_membership_ack", "late_authorization_completion"],
            "old_transport_attempted": old_transport_attempted
        }),
    )
}

fn diagnostics(stream: &mut TcpStream, state: &Mutex<HostState>) -> std::io::Result<()> {
    let locked = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    respond_json(
        stream,
        200,
        &json!({
            "authorization_calls": locked.authorization_calls,
            "current_sequence": locked.authority.current_sequence(),
            "effect_count": locked.effect_count,
            "late_attempts": locked.late_attempts,
            "live_actions": locked.live_actions,
            "membership_controls": locked.membership_controls,
            "open_transports": locked.streams.len(),
            "recovery_allowed": locked.recovery_allowed
        }),
    )
}

fn write_chunk(stream: &mut TcpStream, body: &[u8]) -> std::io::Result<()> {
    stream.write_all(format!("{:x}\r\n", body.len()).as_bytes())?;
    stream.write_all(body)?;
    stream.write_all(b"\r\n")?;
    stream.flush()
}

fn respond_error(stream: &mut TcpStream, status: u16, reason: &str) -> std::io::Result<()> {
    respond_json(stream, status, &json!({ "error": reason }))
}

fn respond_json(stream: &mut TcpStream, status: u16, body: &Value) -> std::io::Result<()> {
    respond_json_with_type(stream, status, "application/json", body)
}

fn respond_json_with_type(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &Value,
) -> std::io::Result<()> {
    let encoded = serde_json::to_vec(body).map_err(std::io::Error::other)?;
    respond(stream, status, content_type, &encoded)
}

fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        426 => "Upgrade Required",
        _ => "Error",
    };
    stream.write_all(
        format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: private, max-age=0, must-revalidate\r\nAccess-Control-Allow-Origin: {ASYNC_REFERENCE_ORIGIN}\r\nAccess-Control-Allow-Headers: authorization,content-type,suprnova-transport-generation\r\nAccess-Control-Allow-Methods: GET,POST,OPTIONS\r\nVary: Origin\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .as_bytes(),
    )?;
    stream.write_all(body)
}
