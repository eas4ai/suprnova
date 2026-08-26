//! Deterministic async browser-conformance scenarios for the thin Rust host.

use serde_json::{Value, json};

/// A deterministic fault that the browser suite may ask the Rust host to schedule.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsyncReferenceFault {
    /// Emit a sequence gap so the production continuity machine degrades.
    SequenceGap,
    /// Emit a registered terminal completion.
    ServerShutdown,
}

/// Static facts for the Task 9 browser-conformance scenario.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AsyncReferenceScenario {
    /// Human-readable scenario identity.
    pub name: &'static str,
    /// Registered stream selected by the checked directive.
    pub stream: &'static str,
    /// Canonical subscription identity used by the generated codec fixture.
    pub subscription_id: &'static str,
}

impl AsyncReferenceScenario {
    /// Returns the deterministic Task 9 lifecycle scenario.
    #[must_use]
    pub const fn lifecycle() -> Self {
        Self {
            name: "async-lifecycle-accessibility",
            stream: "orders",
            subscription_id: "c3Vic2NyaXB0aW9uLTAwMQ",
        }
    }

    /// Returns the bounded, deterministic fault schedule exercised by the browser suite.
    #[must_use]
    pub const fn faults(self) -> [AsyncReferenceFault; 2] {
        [
            AsyncReferenceFault::SequenceGap,
            AsyncReferenceFault::ServerShutdown,
        ]
    }

    /// Produces one canonical Task 3 heartbeat envelope for the actual browser decoder.
    #[must_use]
    pub fn heartbeat(self, sequence: u64) -> String {
        self.envelope(sequence, json!({ "kind": "heartbeat" }))
    }

    /// Produces one canonical Task 3 completion envelope for the actual browser decoder.
    #[must_use]
    pub fn completion(self, sequence: u64) -> String {
        self.envelope(
            sequence,
            json!({ "kind": "complete", "reason": "server_shutdown" }),
        )
    }

    /// Wraps a canonical envelope in the exact bounded SSE wire record shape.
    #[must_use]
    pub fn sse_record(self, sequence: u64, encoded: &str) -> Vec<u8> {
        format!(
            "id:{}/{}/{}\nevent:suprnova-live-async\ndata:{}\n\n",
            self.subscription_id, 1, sequence, encoded
        )
        .into_bytes()
    }

    fn envelope(self, sequence: u64, payload: Value) -> String {
        serde_json::to_string(&json!({
            "payload": payload,
            "position": { "epoch": "1", "sequence": sequence.to_string() },
            "protocol_version": 1,
            "stream": self.stream,
            "subscription": self.subscription_id,
        }))
        .expect("static async reference envelope serializes")
    }
}

#[cfg(test)]
mod tests {
    use super::AsyncReferenceScenario;

    #[test]
    fn scenario_emits_canonical_bounded_sse_records() {
        let scenario = AsyncReferenceScenario::lifecycle();
        let envelope = scenario.heartbeat(1);
        let record = scenario.sse_record(1, &envelope);
        assert!(record.starts_with(b"id:c3Vic2NyaXB0aW9uLTAwMQ/1/1\n"));
        assert!(record.ends_with(b"\n\n"));
        assert!(record.len() < 65_536);
    }
}
