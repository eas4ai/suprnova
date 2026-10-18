//! The limits on agent traffic (AGENT-002, AGENT-003).
//!
//! Every tool call counts against its caller: a user, an agent account, or,
//! for a guest, the client address. MCP calls and browser tool calls count
//! in one bucket per caller, so a route without a throttle of its own is
//! still limited. Every request to the MCP endpoint also counts against its
//! client address, which bounds what the tool budget does not see: lists,
//! handshakes and requests refused for a bad token.

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
    count(&caller.limit_key(), limit).await
}

/// Count one request to the MCP endpoint from `address`, whatever it asks
/// and however it ends, or refuse it when the address's window is full.
pub(crate) async fn admit_request(
    config: &AgentsConfig,
    address: Option<std::net::IpAddr>,
) -> Result<(), Refusal> {
    let key = format!(
        "agents:requests:{}",
        address
            .map(|address| address.to_canonical().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    );
    count(&key, config.request_limit).await
}

/// Count one step against `limit` under `key`.
async fn count(key: &str, limit: super::CallLimit) -> Result<(), Refusal> {
    let window = limit.window.as_secs().max(1);
    match RateLimiter::increment_if_below(key, window, 1, i64::from(limit.max_calls)).await {
        Ok(ConditionalIncrement::Incremented(_)) => Ok(()),
        Ok(ConditionalIncrement::Unchanged(_)) => {
            let retry_after = RateLimiter::available_in(key)
                .await
                .unwrap_or(window)
                .max(1);
            Err(Refusal::Limited(retry_after))
        }
        Err(error) => {
            tracing::error!(
                error = %crate::error::render_error_chain(&error),
                "the agents limiter's store failed; the request is refused"
            );
            Err(Refusal::Unavailable(error))
        }
    }
}
