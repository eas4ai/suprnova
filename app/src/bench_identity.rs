//! Which process am I - the one thing a benchmark replica must know.
//!
//! Phase 1.2 and 1.5 both ask a question of the form "did N processes each
//! decide this work was theirs?". Answering it needs every replica to write
//! a distinct identity next to what it did; without one, a duplicate row is
//! indistinguishable from one process writing twice.
//!
//! Compose scales a service by cloning it, environment block included, so
//! there is no per-replica variable to set. The container hostname is the
//! identity Docker already assigns, and it is stable for the life of the
//! container - which is exactly the lifetime being measured.

/// A stable, per-process identity for benchmark rows.
///
/// `BENCH_INSTANCE_ID` wins when set, so a run outside Docker can still
/// label its processes. Otherwise the container hostname, which Docker sets
/// to the container id.
pub fn process_id() -> String {
    if let Ok(explicit) = std::env::var("BENCH_INSTANCE_ID")
        && !explicit.trim().is_empty()
    {
        return explicit;
    }
    if let Ok(host) = std::env::var("HOSTNAME")
        && !host.trim().is_empty()
    {
        return host;
    }
    // Falling back to the pid keeps two processes on one host distinct.
    // "unknown" would make a duplicate look like a single process, which
    // is the one reading the experiment must never be handed.
    format!("pid-{}", std::process::id())
}
