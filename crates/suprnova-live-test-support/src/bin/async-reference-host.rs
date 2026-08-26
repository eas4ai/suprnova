//! Thin Rust reference host for Task 9 real-browser async conformance.

#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use suprnova_live::async_updates::{SseEncoder, SseResponseContract};
use suprnova_live_test_support::AsyncReferenceScenario;

const ADDRESS: &str = "127.0.0.1:4174";
const STATIC_ORIGIN: &str = "http://127.0.0.1:4173";
const MAX_REQUEST_BYTES: usize = 64 * 1024;

enum StreamCommand {
    Close,
    Completion {
        delivered: mpsc::Sender<()>,
        encoded: String,
        sequence: u64,
    },
    Envelope {
        encoded: String,
        sequence: u64,
    },
}

#[derive(Default)]
struct HostState {
    authorization_count: u64,
    sequence: u64,
    streams: Vec<mpsc::Sender<StreamCommand>>,
}

struct Request {
    method: String,
    target: String,
}

fn main() -> std::io::Result<()> {
    let listener = TcpListener::bind(ADDRESS)?;
    let state = Arc::new(Mutex::new(HostState::default()));
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
        ("GET", "/authorize") => authorize(&mut stream, &request.target, &state),
        ("POST", "/membership") => membership(&mut stream, &request.target),
        ("GET", "/__live/async/events") => event_stream(stream, state),
        ("POST", "/control/degrade") => {
            broadcast_gap(&state);
            respond(&mut stream, 204, "text/plain", b"")
        }
        ("POST", "/control/reconnect") => respond(&mut stream, 204, "text/plain", b""),
        ("POST", "/control/current") => {
            broadcast_next_heartbeat(&state);
            respond(&mut stream, 204, "text/plain", b"")
        }
        ("POST", "/control/reset") => {
            reset(&state);
            respond(&mut stream, 204, "text/plain", b"")
        }
        ("POST", "/control/close") => {
            broadcast_completion(&state);
            respond(&mut stream, 204, "text/plain", b"")
        }
        _ => respond(&mut stream, 404, "text/plain", b"not found"),
    }
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
    let headers = std::str::from_utf8(&bytes[..header_end]).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "request headers were not UTF-8",
        )
    })?;
    let mut lines = headers.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split_whitespace();
    let method = request_line.next().unwrap_or_default().to_owned();
    let target = request_line.next().unwrap_or_default().to_owned();
    let content_length = lines
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
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
    Ok(Request { method, target })
}

fn authorize(
    stream: &mut TcpStream,
    target: &str,
    state: &Mutex<HostState>,
) -> std::io::Result<()> {
    let position = query_value(target, "sequence")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let has_prior = query_value(target, "prior").is_some_and(|value| !value.is_empty());
    let (authorization, current) = {
        let mut locked = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locked.authorization_count = locked.authorization_count.saturating_add(1);
        (locked.authorization_count, locked.sequence)
    };
    let scenario = AsyncReferenceScenario::lifecycle();
    let replay = if has_prior {
        ((position + 1)..=current)
            .map(|sequence| scenario.heartbeat(sequence))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let baseline = if has_prior { position } else { current };
    let body = json!({
        "authorization": authorization,
        "baseline": { "epoch": "1", "sequence": baseline.to_string() },
        "descriptor_binding": format!("binding-{authorization}"),
        "replay": replay,
        "stream": scenario.stream,
        "subscription_id": scenario.subscription_id,
    });
    respond_json(stream, &body)
}

fn membership(stream: &mut TcpStream, target: &str) -> std::io::Result<()> {
    let operation = query_value(target, "operation").unwrap_or_default();
    let acknowledgment = json!({
        "controlNonce": query_value(target, "control_nonce").unwrap_or_default(),
        "descriptorBinding": query_value(target, "binding").unwrap_or_default(),
        "kind": "authenticated",
        "operation": operation,
        "stream": query_value(target, "stream").unwrap_or_default(),
        "subscriptionId": query_value(target, "subscription").unwrap_or_default(),
        "transportGeneration": query_value(target, "generation")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0),
    });
    respond_json(stream, &acknowledgment)
}

fn event_stream(mut stream: TcpStream, state: Arc<Mutex<HostState>>) -> std::io::Result<()> {
    // Register the physical stream before exposing response bytes. Otherwise a
    // fast client can complete membership and ask for its first envelope while
    // the host is still between the header write and ledger registration.
    let (sender, receiver) = mpsc::channel();
    {
        let mut locked = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locked.streams.push(sender);
    }
    let response_headers = SseResponseContract::headers();
    let content_type = response_headers
        .get("content-type")
        .ok_or_else(|| std::io::Error::other("SSE content type missing"))?
        .to_str()
        .map_err(std::io::Error::other)?;
    let cache_control = response_headers
        .get("cache-control")
        .ok_or_else(|| std::io::Error::other("SSE cache control missing"))?
        .to_str()
        .map_err(std::io::Error::other)?;
    let content_options = response_headers
        .get("x-content-type-options")
        .ok_or_else(|| std::io::Error::other("SSE content options missing"))?
        .to_str()
        .map_err(std::io::Error::other)?;
    let buffering = response_headers
        .get("x-accel-buffering")
        .ok_or_else(|| std::io::Error::other("SSE buffering policy missing"))?
        .to_str()
        .map_err(std::io::Error::other)?;
    stream.write_all(
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nCache-Control: {cache_control}\r\nX-Content-Type-Options: {content_options}\r\nAccess-Control-Allow-Origin: {STATIC_ORIGIN}\r\nVary: Origin\r\nX-Accel-Buffering: {buffering}\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n"
        )
        .as_bytes(),
    )?;
    stream.flush()?;
    write_chunk(&mut stream, SseEncoder::heartbeat_comment())?;
    let scenario = AsyncReferenceScenario::lifecycle();
    loop {
        match receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(StreamCommand::Close) => {
                stream.write_all(b"0\r\n\r\n")?;
                return Ok(());
            }
            Ok(StreamCommand::Envelope { encoded, sequence }) => {
                write_envelope(&mut stream, scenario, sequence, encoded)?;
            }
            Ok(StreamCommand::Completion {
                delivered,
                encoded,
                sequence,
            }) => {
                write_envelope(&mut stream, scenario, sequence, encoded)?;
                stream.write_all(b"0\r\n\r\n")?;
                stream.flush()?;
                let _ = delivered.send(());
                return Ok(());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                write_chunk(&mut stream, SseEncoder::heartbeat_comment())?;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

fn broadcast_next_heartbeat(state: &Mutex<HostState>) {
    let scenario = AsyncReferenceScenario::lifecycle();
    let mut locked = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locked.sequence = locked.sequence.saturating_add(1);
    let sequence = locked.sequence;
    let encoded = scenario.heartbeat(sequence);
    locked.streams.retain(|sender| {
        sender
            .send(StreamCommand::Envelope {
                encoded: encoded.clone(),
                sequence,
            })
            .is_ok()
    });
}

fn write_envelope(
    stream: &mut TcpStream,
    scenario: AsyncReferenceScenario,
    sequence: u64,
    encoded: String,
) -> std::io::Result<()> {
    write_chunk(stream, &scenario.sse_record(sequence, &encoded))
}

fn write_chunk(stream: &mut TcpStream, body: &[u8]) -> std::io::Result<()> {
    stream.write_all(format!("{:x}\r\n", body.len()).as_bytes())?;
    stream.write_all(body)?;
    stream.write_all(b"\r\n")?;
    stream.flush()
}

fn broadcast_gap(state: &Mutex<HostState>) {
    let mut locked = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for sender in locked.streams.drain(..) {
        let _ = sender.send(StreamCommand::Close);
    }
}

fn broadcast_completion(state: &Mutex<HostState>) {
    let scenario = AsyncReferenceScenario::lifecycle();
    let mut locked = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locked.sequence = locked.sequence.saturating_add(1);
    let sequence = locked.sequence;
    let encoded = scenario.completion(sequence);
    let mut deliveries = Vec::with_capacity(locked.streams.len());
    locked.streams.retain(|sender| {
        let (delivered, receiver) = mpsc::channel();
        deliveries.push(receiver);
        sender
            .send(StreamCommand::Completion {
                delivered,
                encoded: encoded.clone(),
                sequence,
            })
            .is_ok()
    });
    drop(locked);
    for delivered in deliveries {
        let _ = delivered.recv_timeout(Duration::from_secs(2));
    }
}

fn reset(state: &Mutex<HostState>) {
    let mut locked = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for sender in locked.streams.drain(..) {
        let _ = sender.send(StreamCommand::Close);
    }
    locked.authorization_count = 0;
    locked.sequence = 0;
}

fn query_value<'a>(target: &'a str, name: &str) -> Option<&'a str> {
    target.split_once('?')?.1.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then_some(value)
    })
}

fn respond_json(stream: &mut TcpStream, body: &Value) -> std::io::Result<()> {
    let encoded = serde_json::to_vec(body).map_err(std::io::Error::other)?;
    respond(stream, 200, "application/json", &encoded)
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
        404 => "Not Found",
        _ => "Error",
    };
    stream.write_all(
        format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: {STATIC_ORIGIN}\r\nAccess-Control-Allow-Headers: authorization,content-type\r\nAccess-Control-Allow-Methods: GET,POST,OPTIONS\r\nVary: Origin\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .as_bytes(),
    )?;
    stream.write_all(body)
}
