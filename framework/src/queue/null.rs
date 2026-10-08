//! Null queue driver - discards every push, returns nothing.
//!
//! Mirrors Laravel's `NullQueue`. Useful for code paths that want to keep
//! the `Queue::push` call site without firing the side effect (e.g.
//! `QUEUE_DRIVER=null` in CI when the work being queued is what's under
//! test, not the queueing itself).

use crate::error::FrameworkError;
use crate::queue::driver::{QueueDriver, Reservation, ReservationToken};
use crate::queue::envelope::Envelope;
use crate::queue::inspect::InspectedJob;
use async_trait::async_trait;
use std::time::Duration;

/// [`QueueDriver`] that drops every push and never returns a
/// reservation. Mirrors Laravel's `NullQueue` - useful for CI runs
/// where the queueing side-effect is not under test.
#[derive(Default)]
pub struct NullQueueDriver;

impl NullQueueDriver {
    /// Construct a fresh null driver.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl QueueDriver for NullQueueDriver {
    async fn push(&self, _env: Envelope) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn pop(&self, _vt: Duration) -> Result<Option<Reservation>, FrameworkError> {
        Ok(None)
    }

    async fn ack(&self, _t: &ReservationToken) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn nack(&self, _t: &ReservationToken, _delay: Duration) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn size(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }

    async fn clear(&self) -> Result<u64, FrameworkError> {
        Ok(0)
    }

    /// Always empty: every push is discarded on arrival, so there is never
    /// anything to list. `Ok(vec![])` is the honest answer here - not a lie
    /// of omission the way Laravel's Beanstalkd/SQS stubs are - because for
    /// this driver "nothing to list" is the literal truth, not an
    /// unimplemented method. See the trait default's doc comment on
    /// [`QueueDriver::pending_jobs`].
    async fn pending_jobs(
        &self,
        _queue: Option<&str>,
    ) -> Result<Vec<InspectedJob>, FrameworkError> {
        Ok(Vec::new())
    }

    /// Always empty. See [`pending_jobs`](Self::pending_jobs).
    async fn delayed_jobs(
        &self,
        _queue: Option<&str>,
    ) -> Result<Vec<InspectedJob>, FrameworkError> {
        Ok(Vec::new())
    }

    /// Always empty. See [`pending_jobs`](Self::pending_jobs).
    async fn reserved_jobs(
        &self,
        _queue: Option<&str>,
    ) -> Result<Vec<InspectedJob>, FrameworkError> {
        Ok(Vec::new())
    }

    fn name(&self) -> &'static str {
        "null"
    }
}
