//! Single-shot OAuth grant executors
//! (`docs/specs/suprnova-magnetar/09-oauth-engine.md`'s "Refresh and
//! revocation" and "Client credentials and JWT bearer" sections).
//!
//! Each submodule renders one grant's wire parameters from Task 1's
//! declarative [`super::request_shape`] types plus
//! [`super::provider::OAuthProvider::client_id`]/
//! [`super::provider::OAuthProvider::client_authentication`], executes the
//! token-endpoint POST through the host-supplied
//! [`crate::plugin::HttpTransport`] (spec 09's "reqwest-compatible trait"
//! transport seam), and parses the result through Task 1's
//! [`super::protocol`] types -- including providers that deliver an error
//! body under an HTTP 200 status
//! ([`super::request_shape::TokenRequestShape::accept_http_success_error_body`]).
//!
//! These executors are single-shot: one token request in, one
//! [`super::protocol::TokenSuccessResponse`] or
//! [`super::errors::OAuthProtocolError`] out. Persisting, rotating, or
//! single-flighting the resulting tokens is the token broker's job (a later
//! iteration-003 task), not this module's.

pub mod authorization_code;
pub mod client_credentials;
pub mod jwt_bearer;
pub mod refresh;
pub mod revocation;

use crate::oauth::errors::{OAuthProtocolError, OAuthResult};
use crate::oauth::protocol::{TokenResponseBody, TokenSuccessResponse, parse_token_response_body};
use crate::plugin::{HttpRequest, HttpTransport};

/// Percent-encode one `application/x-www-form-urlencoded` component per
/// the WHATWG URL standard's `application/x-www-form-urlencoded` serializer
/// (space -> `+`, everything outside `[A-Za-z0-9*\-._]` percent-encoded).
/// This crate carries no HTTP/URL crate dependency
/// (`docs/specs/suprnova-magnetar/09-oauth-engine.md`'s provenance note on
/// `io-oauth`'s mandatory `io-http` dependency), so grant executors render
/// their own bodies.
fn form_urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Encode `pairs` as one `application/x-www-form-urlencoded` request body.
fn encode_form_body(pairs: &[(String, String)]) -> Vec<u8> {
    pairs
        .iter()
        .map(|(key, value)| format!("{}={}", form_urlencode(key), form_urlencode(value)))
        .collect::<Vec<_>>()
        .join("&")
        .into_bytes()
}

/// Execute one token-endpoint POST and classify the result per 09's error
/// contract: network/5xx failures map to
/// [`OAuthProtocolError::UpstreamUnavailable`], a malformed body to
/// [`OAuthProtocolError::MalformedTokenResponse`], and a provider-reported
/// OAuth error (whether carried by a 4xx status or, when `shape` allows, an
/// HTTP-200 body) to [`OAuthProtocolError::ProviderReportedError`].
async fn execute_token_request(
    transport: &dyn HttpTransport,
    provider: &'static str,
    endpoint: &str,
    wire: Vec<(String, String)>,
    extra_headers: Vec<(String, String)>,
    accept_http_success_error_body: bool,
) -> OAuthResult<TokenSuccessResponse> {
    execute_token_request_raw(
        transport,
        provider,
        endpoint,
        wire,
        extra_headers,
        accept_http_success_error_body,
    )
    .await
    .map(|(success, _raw_body)| success)
}

/// [`execute_token_request`], additionally returning the exact response
/// body text alongside the parsed value -- the token broker
/// (`docs/specs/suprnova-magnetar/11-token-broker.md`) needs the raw body
/// to store a byte-faithful copy of provider-specific fields that
/// [`TokenSuccessResponse`]'s fixed shape does not carry.
pub(crate) async fn execute_token_request_raw(
    transport: &dyn HttpTransport,
    provider: &'static str,
    endpoint: &str,
    wire: Vec<(String, String)>,
    extra_headers: Vec<(String, String)>,
    accept_http_success_error_body: bool,
) -> OAuthResult<(TokenSuccessResponse, String)> {
    let mut headers = vec![(
        "Content-Type".to_owned(),
        "application/x-www-form-urlencoded".to_owned(),
    )];
    headers.extend(extra_headers);
    let request = HttpRequest {
        method: "POST".to_owned(),
        url: endpoint.to_owned(),
        headers,
        body: encode_form_body(&wire),
    };
    let response =
        transport
            .send(request)
            .await
            .map_err(|error| OAuthProtocolError::UpstreamUnavailable {
                provider,
                message: error.to_string(),
                retry_after_seconds: None,
            })?;
    let body_text = String::from_utf8_lossy(&response.body).into_owned();

    if !(200..300).contains(&response.status) {
        if response.status >= 500 {
            return Err(OAuthProtocolError::UpstreamUnavailable {
                provider,
                message: format!("token endpoint returned http {}", response.status),
                retry_after_seconds: retry_after_seconds(&response.headers),
            });
        }
        return Err(error_from_body(provider, &body_text));
    }

    match parse_token_response_body(&body_text)? {
        TokenResponseBody::Success(success) => Ok((success, body_text)),
        TokenResponseBody::Error(error) => {
            if accept_http_success_error_body {
                Err(OAuthProtocolError::ProviderReportedError {
                    provider,
                    code: error.error.wire_str().to_owned(),
                    message: error.error_description,
                })
            } else {
                Err(OAuthProtocolError::MalformedTokenResponse {
                    message: format!(
                        "provider '{provider}' returned an oauth error body ('{}') under an \
                         HTTP success status, and this provider's token shape does not accept \
                         HTTP-200 error bodies",
                        error.error.wire_str()
                    ),
                })
            }
        }
    }
}

/// Parse a `Retry-After` response header as a `delay-seconds` value (RFC
/// 9110 §10.2.3). The `HTTP-date` form is not parsed here (this crate
/// carries no date-parsing dependency and every fixture/provider observed
/// so far sends `delay-seconds`); an unparseable or absent header yields
/// `None`, never an error.
fn retry_after_seconds(headers: &[(String, String)]) -> Option<u64> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("retry-after"))
        .and_then(|(_, value)| value.trim().parse::<u64>().ok())
}

/// Classify a non-2xx response body as a provider-reported OAuth error, or
/// as malformed when the body does not itself carry an RFC 6749 §5.2 error
/// shape.
fn error_from_body(provider: &'static str, body: &str) -> OAuthProtocolError {
    match parse_token_response_body(body) {
        Ok(TokenResponseBody::Error(error)) => OAuthProtocolError::ProviderReportedError {
            provider,
            code: error.error.wire_str().to_owned(),
            message: error.error_description,
        },
        Ok(TokenResponseBody::Success(_)) => OAuthProtocolError::MalformedTokenResponse {
            message: format!(
                "provider '{provider}' returned a token success body under a non-2xx HTTP status"
            ),
        },
        Err(parse_error) => parse_error,
    }
}
