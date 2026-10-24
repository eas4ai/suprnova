//! Web push notification channel.
//!
//! Delivers notifications via the vendored
//! [`crate::web_push::WebPushClient`]. The route returned by
//! `Notifiable::route_for("webpush")` MUST be a JSON-encoded
//! [`SubscriptionInfo`] (`{"endpoint": "...", "keys": {"p256dh": "...",
//! "auth": "..."}}`) - this matches the shape browsers hand back from
//! `PushSubscription.toJSON()`, so callers can store the subscription
//! verbatim and return it untouched.
//!
//! A subscription that nothing can be sent to is logged at WARN and
//! skipped, and the dispatch succeeds: the push service answered that it
//! is gone (HTTP 404/410), the stored route is no subscription, its
//! endpoint or its keys are refused, its host name resolves to an
//! address the strict endpoint policy refuses, or its origin is not in the
//! endpoint allowlist. There is nobody to retry against, and a failed
//! dispatch would make a queue send the job again, with every channel that
//! delivered before this one. The framework does not remove the stored
//! subscription. The warning is what an operator acts on.
//!
//! ## What a warning says of the endpoint
//!
//! The path of an endpoint is the token that lets a sender reach the
//! browser, so no log line has the endpoint. A line has the host, and
//! `endpoint_sha256`: the first 16 hexadecimal digits of the SHA-256 of
//! the endpoint as it is stored. Compare it with the same digest of the
//! stored endpoints to find the row.
//!
//! ## What an error says of the push service
//!
//! A push service that rejects the message fails the delivery with a
//! `FrameworkError` that carries the status, never the body the service
//! sent: the body is remote data, and the error reaches the dispatcher's
//! log and every `NotificationFailed` listener. The client writes the
//! body's first 200 bytes to its debug log.
//!
//! ## Which push services it sends to
//!
//! `WEB_PUSH_ENDPOINT_ORIGINS` names the origins of the push services the
//! channel may send to, separated by commas, such as
//! `https://fcm.googleapis.com,https://*.notify.windows.com`. When it names
//! one or more, every send goes under [`EndpointPolicy::Allowlist`] with
//! those origins, in place of the policy the client was built with: the
//! strict checks run, and an endpoint at any other origin is refused before
//! its host name is looked up. The refusal is logged as a subscription that
//! cannot be used, with the origin as the reason, and the dispatch
//! succeeds. Unset, or blank, the client's own policy applies.
//! [`WebPushChannel::new`] reads the variable and fails when an entry is
//! not an origin, so a typo fails where the application builds the
//! channel, instead of leaving it open to every host.
//!
//! ## Why `Arc<WebPushClient>`
//!
//! `WebPushClient` wraps a `VapidSigner` which wraps a private
//! `ES256KeyPair`. None of those are `Clone` (private keys shouldn't be
//! casually duplicated), and constructing a fresh signer for every channel
//! registration would mean N independent VAPID identities for the same
//! application. Wrapping in `Arc` lets a single signed identity back every
//! registration and every concurrent delivery.

use crate::error::FrameworkError;
use crate::notifications::{Channel, DynNotification};
use crate::web_push::{
    ContentEncoding, EndpointAllowlist, EndpointPolicy, SubscriptionInfo, WebPushClient,
    WebPushError,
};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Notification channel that POSTs an encrypted payload to a stored
/// browser push subscription endpoint.
///
/// Construct with an `Arc<WebPushClient>` so a single VAPID-signing
/// client can be shared across channel registrations and concurrent
/// fan-out without re-constructing the signer (which is not `Clone`).
/// `ttl_secs` is forwarded as the `TTL` header - the push service caps
/// this and discards undelivered messages after that window.
pub struct WebPushChannel {
    client: Arc<WebPushClient>,
    ttl_secs: u32,
    /// The allowlist `WEB_PUSH_ENDPOINT_ORIGINS` names, which every send
    /// goes under in place of the client's own policy. `None` when the
    /// variable names no origin.
    endpoint_policy: Option<EndpointPolicy>,
}

/// The variable that names the push-service origins the channel sends to.
const ENDPOINT_ORIGINS_ENV: &str = "WEB_PUSH_ENDPOINT_ORIGINS";

impl WebPushChannel {
    /// Build a `WebPushChannel` sending via `client` with the given push
    /// `ttl_secs` (seconds the push service will retry delivery).
    ///
    /// Reads `WEB_PUSH_ENDPOINT_ORIGINS`. When it names one or more
    /// origins, every send goes under [`EndpointPolicy::Allowlist`] with
    /// them, whatever policy `client` was built with; see the module
    /// documentation.
    ///
    /// # Errors
    ///
    /// Returns an error that names `WEB_PUSH_ENDPOINT_ORIGINS` when the
    /// variable is not UTF-8, or when one of its entries is not an origin
    /// of the form `https://host` or `https://host:port`. A value the
    /// channel cannot read fails here, where the application builds the
    /// channel at boot, rather than leaving the channel without the list.
    pub fn new(client: Arc<WebPushClient>, ttl_secs: u32) -> Result<Self, FrameworkError> {
        let raw = match std::env::var(ENDPOINT_ORIGINS_ENV) {
            Ok(raw) => Some(raw),
            Err(std::env::VarError::NotPresent) => None,
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err(FrameworkError::internal(format!(
                    "{ENDPOINT_ORIGINS_ENV} is not valid UTF-8; set it to a comma-separated \
                     list of push service origins, such as https://fcm.googleapis.com"
                )));
            }
        };
        Ok(Self {
            client,
            ttl_secs,
            endpoint_policy: endpoint_policy_from(raw.as_deref())?,
        })
    }
}

/// The policy a value of `WEB_PUSH_ENDPOINT_ORIGINS` selects: the allowlist
/// of the origins it names, or `None` when it is unset or names none.
///
/// Entries are separated by commas, and the spaces around an entry and an
/// empty entry, as a trailing comma leaves, are ignored. Every other entry
/// must be an origin, so one typo refuses the whole value.
fn endpoint_policy_from(raw: Option<&str>) -> Result<Option<EndpointPolicy>, FrameworkError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let origins: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .collect();
    if origins.is_empty() {
        return Ok(None);
    }
    let allowlist = EndpointAllowlist::new(&origins).map_err(|error| {
        FrameworkError::internal(format!(
            "{ENDPOINT_ORIGINS_ENV} is not a comma-separated list of push service \
             origins: {error}. An origin is https://host or https://host:port, such as \
             https://fcm.googleapis.com, or a wildcard origin such as \
             https://*.notify.windows.com"
        ))
    })?;
    Ok(Some(EndpointPolicy::Allowlist(allowlist)))
}

/// The host of a stored endpoint and the digest that finds its row. See
/// the module documentation.
fn endpoint_for_log(endpoint: &str) -> (String, String) {
    let host = url::Url::parse(endpoint)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_default();
    let digest = hex::encode(&Sha256::digest(endpoint.as_bytes())[..8]);
    (host, digest)
}

/// Log a stored subscription that cannot be sent to, and that nothing was
/// sent to. It is the same end as a subscription that is gone: there is
/// nobody to retry against, so the dispatch does not fail and a queue does
/// not send the job again. `reason` names the rule that refused it.
fn warn_unusable(endpoint: &str, notification: &dyn DynNotification, reason: &str) {
    let (host, endpoint_sha256) = endpoint_for_log(endpoint);
    tracing::warn!(
        channel = "webpush",
        host = %host,
        endpoint_sha256 = %endpoint_sha256,
        notification = %notification.name(),
        reason = %reason,
        "webpush subscription cannot be used; caller should remove"
    );
}

#[async_trait]
impl Channel for WebPushChannel {
    fn name(&self) -> &'static str {
        "webpush"
    }

    async fn deliver(
        &self,
        route: &str,
        notification: &dyn DynNotification,
    ) -> Result<(), FrameworkError> {
        let subscription: SubscriptionInfo = match serde_json::from_str(route) {
            Ok(subscription) => subscription,
            Err(error) => {
                // The stored route is no subscription. That is stored data
                // that cannot be used, like an endpoint that is refused.
                // The reason is the kind of the mistake and its position:
                // the text of the decoder can quote the route, which has
                // the endpoint and the secret of the subscription.
                tracing::warn!(
                    channel = "webpush",
                    notification = %notification.name(),
                    reason = %crate::crypto::json_decode_reason(&error),
                    "webpush subscription cannot be used; caller should remove"
                );
                return Ok(());
            }
        };
        let payload = serde_json::to_vec(&notification.data()).map_err(|e| {
            FrameworkError::internal(format!("WebPushChannel: payload encode: {e}"))
        })?;

        let sent = match &self.endpoint_policy {
            Some(policy) => {
                self.client
                    .send_with_policy(
                        policy,
                        &subscription,
                        &payload,
                        ContentEncoding::Aes128Gcm,
                        self.ttl_secs,
                    )
                    .await
            }
            None => {
                self.client
                    .send(
                        &subscription,
                        &payload,
                        ContentEncoding::Aes128Gcm,
                        self.ttl_secs,
                    )
                    .await
            }
        };
        match sent {
            Ok(_) => Ok(()),
            Err(WebPushError::SubscriptionGone) => {
                // The push service told us this subscription is dead (404/410).
                // Surface a structured warn - callers should remove the stored
                // subscription, but we don't fail dispatch over it because the
                // notification "succeeded" in the only sense available: it
                // reached a terminal state with no recipient to retry against.
                let (host, endpoint_sha256) = endpoint_for_log(&subscription.endpoint);
                tracing::warn!(
                    channel = "webpush",
                    host = %host,
                    endpoint_sha256 = %endpoint_sha256,
                    notification = %notification.name(),
                    "webpush subscription gone (404/410); caller should remove"
                );
                Ok(())
            }
            Err(WebPushError::InvalidSubscription(reason)) => {
                warn_unusable(&subscription.endpoint, notification, &reason);
                Ok(())
            }
            Err(WebPushError::RefusedDestination(class)) => {
                // The class of the address, never the address itself.
                warn_unusable(
                    &subscription.endpoint,
                    notification,
                    &format!("its host name resolves to {class}"),
                );
                Ok(())
            }
            Err(WebPushError::OriginNotAllowed(origin)) => {
                // The origin, never the path, which is the token that
                // reaches the browser.
                warn_unusable(
                    &subscription.endpoint,
                    notification,
                    &format!("its origin {origin} is not in the endpoint allowlist"),
                );
                Ok(())
            }
            Err(WebPushError::PushServiceRejected {
                retry_after: Some(retry_after),
                status,
            }) => Err(FrameworkError::rate_limited(
                Some(retry_after),
                format!("WebPushChannel: push service rejected (status {status})"),
            )),
            // The status alone: the body the push service sent is remote
            // data and stays out of an error a listener receives.
            Err(WebPushError::PushServiceRejected {
                retry_after: None,
                status,
            }) => Err(FrameworkError::internal(format!(
                "WebPushChannel: push service rejected (status {status})"
            ))),
            Err(e) => Err(FrameworkError::internal(format!("WebPushChannel: {e}"))),
        }
    }
}
