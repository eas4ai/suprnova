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
//! is gone (HTTP 404/410), the stored route is no subscription, or its
//! endpoint or its keys are refused. There is nobody to retry against, and
//! a failed dispatch would make a queue send the job again, with every
//! channel that delivered before this one. The framework does not remove
//! the stored subscription. The warning is what an operator acts on.
//!
//! ## What a warning says of the endpoint
//!
//! The path of an endpoint is the token that lets a sender reach the
//! browser, so no log line has the endpoint. A line has the host, and
//! `endpoint_sha256`: the first 16 hexadecimal digits of the SHA-256 of
//! the endpoint as it is stored. Compare it with the same digest of the
//! stored endpoints to find the row.
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
use crate::web_push::{ContentEncoding, SubscriptionInfo, WebPushClient, WebPushError};
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
}

impl WebPushChannel {
    /// Build a `WebPushChannel` sending via `client` with the given push
    /// `ttl_secs` (seconds the push service will retry delivery).
    pub fn new(client: Arc<WebPushClient>, ttl_secs: u32) -> Self {
        Self { client, ttl_secs }
    }
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

        match self
            .client
            .send(
                &subscription,
                &payload,
                ContentEncoding::Aes128Gcm,
                self.ttl_secs,
            )
            .await
        {
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
                // The stored subscription cannot be sent to, and nothing
                // was sent. It is the same end as a subscription that is
                // gone: there is nobody to retry against, so the dispatch
                // does not fail and a queue does not send the job again.
                let (host, endpoint_sha256) = endpoint_for_log(&subscription.endpoint);
                tracing::warn!(
                    channel = "webpush",
                    host = %host,
                    endpoint_sha256 = %endpoint_sha256,
                    notification = %notification.name(),
                    reason = %reason,
                    "webpush subscription cannot be used; caller should remove"
                );
                Ok(())
            }
            Err(WebPushError::PushServiceRejected {
                retry_after: Some(retry_after),
                status,
                ..
            }) => Err(FrameworkError::rate_limited(
                Some(retry_after),
                format!("WebPushChannel: push service rejected (status {status})"),
            )),
            Err(e) => Err(FrameworkError::internal(format!("WebPushChannel: {e}"))),
        }
    }
}
