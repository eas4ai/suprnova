//! The text of an SDK error, safe to put in a `PaymentError`.
//!
//! The SDK renders an API error as a debug dump of Stripe's error object,
//! whose message, parameter and log URL name the entity and the request,
//! and a transport error as free text from the HTTP client. An error text
//! ends up in logs and in responses, so no id and no URL may reach it.
//! Every call of the adapter that maps an SDK error goes through
//! [`provider_error`].

use stripe::StripeError;
use suprnova::payments::PaymentError;

/// A `PaymentError::Provider` for `error`, naming the `operation` that
/// failed: `stripe customers.retrieve: api error invalid_request_error
/// resource_missing, status 404`.
pub(crate) fn provider_error(operation: &str, error: StripeError) -> PaymentError {
    PaymentError::Provider(format!("stripe {operation}: {}", describe(&error)))
}

/// What the error says without an id or a URL: an API error as its kind,
/// its code and the HTTP status, and any other variant as a fixed text that
/// names it.
pub(crate) fn describe(error: &StripeError) -> String {
    match error {
        StripeError::Stripe(errors, status) => {
            let code = errors.code.as_ref().map_or("no code", |code| code.as_str());
            format!(
                "api error {} {code}, status {status}",
                errors.type_.as_str()
            )
        }
        StripeError::JSONDeserialize(_) => "the response could not be deserialized".into(),
        StripeError::ClientError(_) => "the request could not be completed".into(),
        StripeError::ConfigError(_) => "the client configuration is invalid".into(),
        StripeError::Timeout => "the request timed out".into(),
    }
}
