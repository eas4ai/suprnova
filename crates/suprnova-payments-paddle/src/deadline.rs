//! The deadline of every call to Paddle.
//!
//! The SDK builds its reqwest client without a request deadline, so a
//! Paddle that takes the connection and never answers would hold the caller
//! for as long as the connection stays open. Every call of the SDK goes
//! through one of the two functions here.

use std::time::Duration;

use suprnova::payments::{PaymentError, PaymentResult};

/// How long a call to Paddle may take, from the request to the whole answer.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Wait at most [`REQUEST_TIMEOUT`] for a call that only reads from Paddle.
pub(crate) async fn read<F: Future>(operation: &str, call: F) -> PaymentResult<F::Output> {
    tokio::time::timeout(REQUEST_TIMEOUT, call)
        .await
        .map_err(|_| PaymentError::Provider(format!("paddle {operation} timed out")))
}

/// Wait at most [`REQUEST_TIMEOUT`] for a call that changes something at
/// Paddle.
///
/// A request that ran out of time may still have reached Paddle and taken
/// effect, so the error says that the outcome is unknown, not that the call
/// failed: the caller has to look at Paddle before it tries again.
pub(crate) async fn change<F: Future>(operation: &str, call: F) -> PaymentResult<F::Output> {
    tokio::time::timeout(REQUEST_TIMEOUT, call)
        .await
        .map_err(|_| unknown_outcome(operation))
}

/// The error of a change that ran out of time.
fn unknown_outcome(operation: &str) -> PaymentError {
    PaymentError::Provider(format!(
        "paddle {operation} timed out; outcome is unknown, reconcile before retrying"
    ))
}
