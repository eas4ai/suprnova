//! MEM-003 on the context, queue and SQS paths.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use suprnova::queue::Envelope;
use suprnova::queue::worker::{register_job, run_through_middleware};
use suprnova::{
    Context, ContextStore, FrameworkError, Job, QueueDriver, SqsConfig, SqsCredentials,
    SqsQueueDriver,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::support::{Heap, exclusive};

/// MEM-003: pushing onto an array context value moves the value in.
#[tokio::test]
async fn mem_audit_pushing_onto_an_array_does_not_copy_the_value() {
    let _lock = exclusive().await;
    Context::scope(ContextStore::default(), async {
        Context::push("k", "seed");
        let big = "x".repeat(1 << 20);
        let heap = Heap::start();
        let before = heap.bytes();
        Context::push("k", &big);
        let grew = heap.bytes() - before;
        assert!(
            grew < (1 << 20) * 3 / 2,
            "pushing a 1 MiB value allocated {grew} bytes"
        );
    })
    .await;
}

static SEEN: AtomicUsize = AtomicUsize::new(0);

#[derive(Serialize, Deserialize)]
struct AddressJob {
    blob: String,
}

#[async_trait::async_trait]
impl Job for AddressJob {
    fn job_name() -> &'static str {
        "mem-audit-address"
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        SEEN.store(self.blob.as_ptr() as usize, Ordering::SeqCst);
        Ok(())
    }
}

fn envelope(job_name: &str, payload: serde_json::Value) -> Envelope {
    let now = suprnova::clock::now();
    Envelope {
        schema_version: suprnova::queue::CURRENT_SCHEMA_VERSION,
        id: uuid::Uuid::new_v4(),
        job_name: job_name.into(),
        queue: None,
        payload,
        dispatched_at: now,
        available_at: now,
        attempts: 0,
        max_tries: 5,
        backoff: suprnova::queue::BackoffSchedule::default(),
        timeout_secs: None,
        fail_on_timeout: false,
        idempotency_key: None,
        unique_lock_owner: None,
        debounce_id: None,
        debounce_owner: None,
        batch_id: None,
        chain_remaining: Vec::new(),
        context: None,
    }
}

/// MEM-003: the worker hands the job the envelope's own payload.
#[tokio::test]
async fn mem_audit_the_job_receives_the_payload_itself() {
    let _lock = exclusive().await;
    register_job::<AddressJob>();
    let blob = "x".repeat(64 * 1024);
    let original = blob.as_ptr() as usize;
    let mut map = serde_json::Map::new();
    map.insert("blob".into(), serde_json::Value::String(blob));
    run_through_middleware(envelope(
        "mem-audit-address",
        serde_json::Value::Object(map),
    ))
    .await
    .expect("the job ran");
    assert_eq!(SEEN.load(Ordering::SeqCst), original, "the job got a copy");
}

/// Replies built once, so answering a request allocates nothing.
static THROTTLED: std::sync::LazyLock<Vec<u8>> = std::sync::LazyLock::new(|| {
    reply(
        400,
        r#"{"__type":"com.amazonaws.sqs#RequestThrottled","message":"slow down"}"#,
    )
});
static SENT: std::sync::LazyLock<Vec<u8>> =
    std::sync::LazyLock::new(|| reply(200, r#"{"MessageId":"m-1","MD5OfMessageBody":"00"}"#));

/// An SQS JSON reply with `status` and `body`.
fn reply(status: u16, body: &str) -> Vec<u8> {
    let reason = if status == 200 { "OK" } else { "Bad Request" };
    format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/x-amz-json-1.0\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// A loopback SQS that discards every request into a fixed buffer and
/// answers `ReceiveMessage` with `receive`; after the first send, which
/// warms the connection, it refuses the next `throttles` as throttled.
async fn sqs(receive: Arc<Vec<u8>>, throttles: usize) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port");
    let addr = listener.local_addr().expect("its address");
    let refused = Arc::new(AtomicUsize::new(0));
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let receive = receive.clone();
            let refused = refused.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 64 * 1024];
                loop {
                    let mut head = [0u8; 4096];
                    let mut filled = 0;
                    let end = loop {
                        let n = match stream.read(&mut head[filled..]).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => n,
                        };
                        filled += n;
                        if let Some(at) = head[..filled].windows(4).position(|w| w == b"\r\n\r\n") {
                            break at + 4;
                        }
                    };
                    let text = String::from_utf8_lossy(&head[..end]).to_ascii_lowercase();
                    let length: usize = text
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    let mut remaining = length.saturating_sub(filled - end);
                    while remaining > 0 {
                        let take = remaining.min(buf.len());
                        let n = match stream.read(&mut buf[..take]).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => n,
                        };
                        remaining -= n;
                    }
                    let reply: &[u8] = if text.contains("amazonsqs.receivemessage") {
                        &receive
                    } else if (1..=throttles).contains(&refused.fetch_add(1, Ordering::SeqCst)) {
                        &THROTTLED
                    } else {
                        &SENT
                    };
                    if stream.write_all(reply).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    addr
}

fn driver(addr: SocketAddr) -> SqsQueueDriver {
    let mut config = SqsConfig::new("us-east-1", "memory");
    config.prefix = Some(format!("http://{addr}/123456789012"));
    config.endpoint = Some(format!("http://{addr}"));
    config.credentials = Some(SqsCredentials {
        access_key_id: "AKIDSUPRNOVATEST".into(),
        secret_access_key: "secret".into(),
        session_token: None,
    });
    SqsQueueDriver::new(config).expect("a driver")
}

/// A `ReceiveMessage` reply carrying `body`.
fn received(body: &str) -> Vec<u8> {
    let json = serde_json::json!({ "Messages": [{
        "MessageId": "m-1",
        "ReceiptHandle": "r-1",
        "Body": body,
        "Attributes": { "ApproximateReceiveCount": "1" }
    }] })
    .to_string();
    reply(200, &json)
}

/// MEM-003: an inline SQS message is decoded into one envelope, not
/// parsed twice.
#[tokio::test]
async fn mem_audit_an_sqs_message_is_parsed_once() {
    let _lock = exclusive().await;
    const SIZE: usize = 512 * 1024;
    let full = serde_json::to_string(&envelope(
        "sqs-probe",
        serde_json::json!({ "blob": "a".repeat(SIZE) }),
    ))
    .expect("an envelope");
    let empty = serde_json::to_string(&envelope("sqs-probe", serde_json::json!({ "blob": "" })))
        .expect("an envelope");
    let padded = format!("{{{}{}", " ".repeat(full.len() - empty.len()), &empty[1..]);
    assert_eq!(padded.len(), full.len());

    let mut used = Vec::new();
    for body in [&full, &padded] {
        let addr = sqs(Arc::new(received(body)), 0).await;
        let driver = driver(addr);
        driver.pop(Duration::from_secs(30)).await.expect("a pop");
        let heap = Heap::start();
        let before = heap.bytes();
        let reservation = driver.pop(Duration::from_secs(30)).await.expect("a pop");
        used.push(heap.bytes() - before);
        drop(reservation);
        drop(heap);
    }
    assert!(
        used[0] < used[1] + (SIZE as u64) * 5 / 2,
        "decoding a {SIZE}-byte payload allocated {} bytes more than whitespace of that size",
        used[0] - used[1]
    );
}

/// MEM-003: a retried SQS request sends the bytes it serialized once.
#[tokio::test]
async fn mem_audit_a_retried_sqs_request_does_not_copy_the_body() {
    let _lock = exclusive().await;
    const SIZE: usize = 512 * 1024;
    let payload = serde_json::json!({ "blob": "a".repeat(SIZE) });
    let mut used = Vec::new();
    for throttles in [2, 0] {
        let addr = sqs(Arc::new(received("{}")), throttles).await;
        let driver = driver(addr);
        driver
            .push(envelope("sqs-probe", serde_json::json!({ "blob": "warm" })))
            .await
            .ok();
        let job = envelope("sqs-probe", payload.clone());
        let heap = Heap::start();
        let before = heap.bytes();
        driver.push(job).await.expect("a push");
        used.push(heap.bytes() - before);
        drop(heap);
    }
    assert!(
        used[0] < used[1] + (SIZE as u64) / 2,
        "two retries allocated {} bytes more than one attempt",
        used[0] - used[1]
    );
}
