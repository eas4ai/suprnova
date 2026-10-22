//! The text of an SDK error, safe to put in a `PaymentError`.
//!
//! The SDK renders a transport error with the URL of the request, which
//! holds the customer or subscription id, and an API error with Paddle's
//! `detail`, which names the entity it could not find. An error text ends up
//! in logs and in responses, so no id and no URL may reach it. Every call of
//! the adapter that maps an SDK error goes through [`provider_error`].

use paddle_rust_sdk::Error;
use paddle_rust_sdk::error::ErrorType;
use suprnova::payments::PaymentError;

/// A `PaymentError::Provider` for `error`, naming the `operation` that
/// failed: `paddle customer_get: request error: error sending request`.
pub(crate) fn provider_error(operation: &str, error: Error) -> PaymentError {
    PaymentError::Provider(format!("paddle {operation}: {}", describe(error)))
}

/// What the error says without an id or a URL: a transport error without its
/// URL, an API error as its type and code, and any other variant as a fixed
/// text that names it.
fn describe(error: Error) -> String {
    match error {
        Error::Request(source) => format!("request error: {}", source.without_url()),
        Error::PaddleApi(response) => {
            let kind = match response.error.error_type {
                ErrorType::RequestError => "request_error",
                ErrorType::ApiError => "api_error",
            };
            format!("{kind} {}", response.error.code)
        }
        Error::Url(_) => "URL error".into(),
        Error::QueryString(_) => "query string error".into(),
        Error::PaddleSignature(_) => "signature error".into(),
        Error::ParseIntError(_) => "integer parsing error".into(),
        Error::MacError(_) => "HMAC error".into(),
        Error::JsonError(_) => "response body is not the expected JSON".into(),
    }
}
