//! MEM-003 on the context and queue paths. The SQS paths are in `sqs`.

use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};
use suprnova::queue::Envelope;
use suprnova::queue::worker::{register_job, run_through_middleware};
use suprnova::{Context, ContextStore, FrameworkError, Job};

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

pub(crate) fn envelope(job_name: &str, payload: serde_json::Value) -> Envelope {
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
