//! The per-caller limit on tool calls (AGENT-002).
//!
//! Every tool call counts against its caller: a user, an agent account, or,
//! for a guest, the client address. Calls through the browser and through
//! the MCP endpoint count in one bucket per caller, so a route without a
//! throttle of its own is still limited.

use super::AgentsConfig;
use super::caller::{CallerKind, ToolCaller};
use crate::cache::ConditionalIncrement;
use crate::error::FrameworkError;
use crate::rate_limit::RateLimiter;

/// Why a call was not admitted.
pub(crate) enum Refusal {
    /// The caller used its calls for the window; it may call again after
    /// this many seconds.
    Limited(u64),
    /// The limiter's store failed. The call is refused rather than run
    /// unlimited.
    Unavailable(FrameworkError),
}

/// Count one call for `caller`, or refuse it when the caller's window is
/// full. A refused call is not counted.
pub(crate) async fn admit(config: &AgentsConfig, caller: &ToolCaller) -> Result<(), Refusal> {
    let limit = match caller.kind() {
        CallerKind::Public => config.address_limit,
        CallerKind::User | CallerKind::Agent => config.caller_limit,
    };
    let key = caller.limit_key();
    let window = limit.window.as_secs().max(1);
    match RateLimiter::increment_if_below(&key, window, 1, i64::from(limit.max_calls)).await {
        Ok(ConditionalIncrement::Incremented(_)) => Ok(()),
        Ok(ConditionalIncrement::Unchanged(_)) => {
            let retry_after = RateLimiter::available_in(&key)
                .await
                .unwrap_or(window)
                .max(1);
            Err(Refusal::Limited(retry_after))
        }
        Err(error) => {
            tracing::error!(
                error = %crate::error::render_error_chain(&error),
                "the tool call limiter's store failed; the call is refused"
            );
            Err(Refusal::Unavailable(error))
        }
    }
}
