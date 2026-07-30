//! Shared helpers for HTTP-based mail providers (Postmark, SES, SendGrid, …).
//!
//! The timeout envelope and the capped error-body reader live in
//! [`crate::http_client::vendor`], because the Pinecone vector driver needs
//! exactly the same safety properties against exactly the same threat — an
//! operator-overridable endpoint that may be slow, hostile, or wrong. This
//! module keeps the mail-specific parts: the pooled client tagged with the
//! mail user agent, and the provider error formatter.

use crate::error::FrameworkError;
use crate::http_client::vendor::build_client;
use reqwest::Client;
use std::sync::OnceLock;

pub(crate) use crate::http_client::vendor::read_error_body;

/// One shared `reqwest::Client` across all HTTP-mail transports.
/// Connection-pooled, rustls, no PII headers. Carries an explicit
/// request + connect timeout so a slow or unresponsive provider cannot
/// hold a `MailTransport::send` await indefinitely.
pub(crate) fn shared_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| build_client(concat!("suprnova-mail/", env!("CARGO_PKG_VERSION"))))
}

pub(crate) fn err(provider: &'static str, status: u16, body: String) -> FrameworkError {
    FrameworkError::internal(format!("{provider} HTTP {status}: {body}"))
}
