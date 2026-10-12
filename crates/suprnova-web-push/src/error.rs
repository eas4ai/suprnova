//! Errors for the web-push crate.

use std::fmt;
use std::time::Duration;
use thiserror::Error;

/// Why the `Strict` endpoint policy refused the address a subscription's
/// host name resolves to.
///
/// The class, never the address, is what a refusal reports: the class is
/// what an operator acts on, and the address of an internal service is
/// itself worth keeping out of logs. It displays as the phrase the
/// [`WebPushError::RefusedDestination`] message uses, such as
/// `a loopback address`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AddressClass {
    /// `127.0.0.0/8` or `::1`.
    Loopback,
    /// RFC 1918 (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`), the RFC
    /// 6598 shared address space (`100.64.0.0/10`), IPv6 unique local
    /// (`fc00::/7`), the deprecated site-local range (`fec0::/10`) and the
    /// local-use NAT64 prefix (`64:ff9b:1::/48`).
    Private,
    /// `169.254.0.0/16` or `fe80::/10`.
    LinkLocal,
    /// `224.0.0.0/4` or `ff00::/8`.
    Multicast,
    /// `0.0.0.0` or `::`, which a connect on Linux reaches as the local host.
    Unspecified,
    /// The ranges set aside for examples: `192.0.2.0/24`, `198.51.100.0/24`,
    /// `203.0.113.0/24`, `2001:db8::/32` and `3fff::/20`.
    Documentation,
    /// Any other range IANA keeps off the public internet: `0.0.0.0/8`,
    /// `192.0.0.0/24`, `192.88.99.0/24`, `198.18.0.0/15`, `240.0.0.0/4` and
    /// the broadcast address, and in IPv6 everything outside `2000::/3`,
    /// `2001::/23` and the 6to4 prefix `2002::/16`.
    Reserved,
    /// A cloud instance metadata service: `169.254.169.254`,
    /// `fd00:ec2::254` or `100.100.100.200`.
    Metadata,
}

impl fmt::Display for AddressClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Loopback => "a loopback address",
            Self::Private => "a private address",
            Self::LinkLocal => "a link-local address",
            Self::Multicast => "a multicast address",
            Self::Unspecified => "the unspecified address",
            Self::Documentation => "a documentation address",
            Self::Reserved => "a reserved address",
            Self::Metadata => "a cloud metadata address",
        })
    }
}

#[derive(Debug, Error)]
pub enum WebPushError {
    #[error("VAPID key error: {0}")]
    Vapid(String),
    #[error("payload encryption failed: {0}")]
    Encryption(String),
    #[error("HTTP transport error: {0}")]
    Http(#[from] reqwest::Error),
    /// The push service responded with a non-2xx, non-404/410 status.
    ///
    /// `retry_after` is populated when the push service supplied an
    /// RFC 7231 `Retry-After` header in delta-seconds form. The HTTP-date
    /// form is recognised but not parsed - callers that need date-form
    /// support should re-fetch the header themselves; the overwhelming
    /// majority of push services (FCM, Mozilla AutoPush, APNs HTTP/2 web
    /// push) emit delta-seconds.
    ///
    /// The error carries no part of the response body. The body is remote
    /// data, and an error is what a caller logs, stores and shows; its
    /// first 200 bytes go to the debug log instead.
    /// See [`WebPushError::is_retryable`] for whether a caller should
    /// retry vs. drop.
    #[error(
        "push service rejected: status {status}{retry_hint}",
        retry_hint = retry_after
            .map(|d| format!(", retry-after {}s", d.as_secs()))
            .unwrap_or_default()
    )]
    PushServiceRejected {
        status: u16,
        retry_after: Option<Duration>,
    },
    #[error("base64 decode: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("JSON encode/decode: {0}")]
    Json(#[from] serde_json::Error),
    #[error("subscription expired or invalid (HTTP 404/410)")]
    SubscriptionGone,
    /// The stored subscription cannot be sent to, and nothing was sent.
    /// Its endpoint is no URL, or the `Strict` endpoint policy refuses it
    /// because it is not `https`, names an address in place of a host, or
    /// names a host that is no push service. Or one of its keys is not
    /// what RFC 8291 asks for: `p256dh` is no base64url, is not 65 bytes,
    /// is not in uncompressed form or is no point of the P-256 curve,
    /// `auth` is no base64url or is not 16 bytes.
    ///
    /// This is a fault of the stored data and not of this crate. No retry
    /// changes it, so the caller removes the subscription, as it does for
    /// [`Self::SubscriptionGone`]. The text says which rule refused it.
    #[error("subscription cannot be used: {0}")]
    InvalidSubscription(String),
    /// Refused before any I/O: a caller-supplied HTTP client with an
    /// unknown redirect policy must not send under the `Strict` endpoint
    /// policy, which validates only the initial URL. Build the transport
    /// with `WebPushClient::with_client_builder` (redirects forcibly
    /// disabled) or explicitly accept the risk with
    /// `WebPushClient::allow_unconfined_redirects`.
    #[error(
        "refusing Strict send over a redirect-uncharacterized HTTP client; \
         use WebPushClient::with_client_builder or allow_unconfined_redirects"
    )]
    UnconfinedRedirects,
    /// The `Strict` endpoint policy refused the subscription because its
    /// host name resolves to an address of this class, and nothing was
    /// sent. A refusal names the class, not the address.
    ///
    /// Like [`Self::InvalidSubscription`], this is a fault of the stored
    /// data: no retry changes it, so the caller removes the subscription.
    #[error("subscription cannot be used: its host name resolves to {0}")]
    RefusedDestination(AddressClass),
    /// The `Strict` endpoint policy could not look the subscription's host
    /// name up, or the name resolved to no address, and nothing was sent.
    /// A lookup can fail for a moment, so a retry may succeed.
    #[error("the host name of the subscription endpoint could not be resolved: {0}")]
    Lookup(std::io::Error),
    #[error("internal: {0}")]
    Internal(String),
}

impl WebPushError {
    /// Whether a caller should retry the send after a transient failure.
    ///
    /// Returns `true` for HTTP transport errors (no response received -
    /// network/timeout/DNS), for a failed host name lookup (`Lookup`), and
    /// for `PushServiceRejected` with a 408
    /// (request timeout), 429 (too many requests), or any 5xx status.
    /// Returns `false` for terminal outcomes: `SubscriptionGone` (404/410),
    /// other 4xx (authn/authz/protocol errors), and for the local errors
    /// that fired before any HTTP I/O (`InvalidSubscription`,
    /// `RefusedDestination`, `Vapid`, `Encryption`, `Base64`, `Json`,
    /// `Internal`, `UnconfinedRedirects`).
    ///
    /// When `Some`, [`Self::retry_after`] gives the push-service-suggested
    /// minimum delay.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Http(_) | Self::Lookup(_) => true,
            Self::PushServiceRejected { status, .. } => {
                matches!(*status, 408 | 429) || (500..=599).contains(status)
            }
            Self::SubscriptionGone
            | Self::InvalidSubscription(_)
            | Self::RefusedDestination(_)
            | Self::Vapid(_)
            | Self::Encryption(_)
            | Self::Base64(_)
            | Self::Json(_)
            | Self::Internal(_)
            | Self::UnconfinedRedirects => false,
        }
    }

    /// The push service's suggested retry delay, if it sent one and the
    /// header was in delta-seconds form. Only meaningful when
    /// [`Self::is_retryable`] is `true`; absent retry hints return `None`.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::PushServiceRejected { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}
