//! Framework-neutral abuse-limiting contracts.
//!
//! Production implementations must use a shared backend. In-process
//! implementations belong in tests and are intentionally not provided here.

use std::time::Duration;

use crate::Result;

/// A request budget over a fixed time window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AbusePolicy {
    /// Maximum number of requests admitted during the window.
    pub max_requests: u32,
    /// Length of the request window.
    pub window: Duration,
}

impl AbusePolicy {
    /// Validate a policy before sending it to a backend.
    pub fn validate(self) -> Result<()> {
        if self.max_requests == 0 {
            return Err(crate::Error::InvalidInput {
                field: "max_requests".to_owned(),
                message: "must be greater than zero".to_owned(),
            });
        }
        if self.window < Duration::from_millis(1) {
            return Err(crate::Error::InvalidInput {
                field: "window".to_owned(),
                message: "must be at least one millisecond".to_owned(),
            });
        }
        Ok(())
    }
}

/// The outcome of attempting to consume one request from an abuse budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Permit {
    /// The request is admitted. A retry duration is supplied when useful to a
    /// caller that wants to communicate the current window boundary.
    Allowed {
        /// Optional duration after which another request should be attempted.
        retry_after: Option<Duration>,
    },
    /// The request is denied until the current window expires.
    Rejected {
        /// Duration until another request may be attempted.
        retry_after: Duration,
    },
}

/// A backend-backed boundary for limiting abusive request patterns.
#[async_trait::async_trait]
pub trait AbuseLimiter: Send + Sync {
    /// Attempt to consume one request for a route-scoped identity key.
    ///
    /// Implementations must fail closed: a backend failure is returned as an
    /// error and must never be converted into [`Permit::Allowed`].
    async fn acquire(&self, key: &str, policy: AbusePolicy) -> Result<Permit>;
}
