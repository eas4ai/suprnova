//! The deadline of every call to Stripe.
//!
//! The async client of `async-stripe` sets no request deadline (only its
//! blocking client does), so a Stripe that takes the connection and never
//! answers would hold the caller for as long as the connection stays open.
//! Every call of the SDK goes through one of the two functions here.

use std::time::Duration;

use suprnova::payments::{PaymentError, PaymentResult};

/// How long a call to Stripe may take, from the request to the whole
/// answer, retries of the SDK included.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Wait at most [`REQUEST_TIMEOUT`] for a call that only reads from Stripe.
pub(crate) async fn read<F: Future>(operation: &str, call: F) -> PaymentResult<F::Output> {
    tokio::time::timeout(REQUEST_TIMEOUT, call)
        .await
        .map_err(|_| PaymentError::Provider(format!("stripe {operation} timed out")))
}

/// Wait at most [`REQUEST_TIMEOUT`] for a call that changes something at
/// Stripe.
///
/// A request that ran out of time may still have reached Stripe and taken
/// effect, so the error says that the outcome is unknown, not that the call
/// failed: the caller has to look at Stripe before it tries again, or
/// retry under the same idempotency key.
pub(crate) async fn change<F: Future>(operation: &str, call: F) -> PaymentResult<F::Output> {
    tokio::time::timeout(REQUEST_TIMEOUT, call)
        .await
        .map_err(|_| unknown_outcome(operation))
}

/// The error of a change that ran out of time.
fn unknown_outcome(operation: &str) -> PaymentError {
    PaymentError::Provider(format!(
        "stripe {operation} timed out; outcome is unknown, reconcile before retrying"
    ))
}
