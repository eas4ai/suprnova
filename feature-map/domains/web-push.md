# Feature map: `manual/web-push.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 36 checked.

## Rust API: suprnova

### `suprnova::notifications::channels::webpush` (feature: `web-push`)

- [ ] struct `suprnova::WebPushChannel` · framework/src/notifications/channels/webpush.rs:37 (also `suprnova::notifications::channels::webpush::WebPushChannel`)
  - Implements: `suprnova::Channel`
  - [ ] fn `suprnova::WebPushChannel::new` · framework/src/notifications/channels/webpush.rs:45

## Rust API: suprnova-web-push

### `suprnova_web_push::client`

- [ ] struct `suprnova_web_push::PushResponse` · crates/suprnova-web-push/src/client.rs:44 (also `suprnova_web_push::client::PushResponse`) (re-exported as `suprnova::PushResponse`, `suprnova::web_push::PushResponse`)
  - Public fields: `status`
- [ ] struct `suprnova_web_push::SubscriptionInfo` · crates/suprnova-web-push/src/client.rs:32 (also `suprnova_web_push::client::SubscriptionInfo`) (re-exported as `suprnova::SubscriptionInfo`, `suprnova::web_push::SubscriptionInfo`)
  - Public fields: `endpoint`, `keys`
- [ ] struct `suprnova_web_push::client::SubscriptionKeys` · crates/suprnova-web-push/src/client.rs:38
  - Public fields: `p256dh`, `auth`
- [ ] struct `suprnova_web_push::WebPushClient` · crates/suprnova-web-push/src/client.rs:96 (also `suprnova_web_push::client::WebPushClient`) (re-exported as `suprnova::WebPushClient`, `suprnova::web_push::WebPushClient`)
  - [ ] fn `suprnova_web_push::WebPushClient::new` · crates/suprnova-web-push/src/client.rs:116
  - [ ] fn `suprnova_web_push::WebPushClient::with_client_builder` · crates/suprnova-web-push/src/client.rs:140
  - [ ] fn `suprnova_web_push::WebPushClient::with_client` · crates/suprnova-web-push/src/client.rs:170
  - [ ] fn `suprnova_web_push::WebPushClient::allow_unconfined_redirects` · crates/suprnova-web-push/src/client.rs:185
  - [ ] fn `suprnova_web_push::WebPushClient::with_endpoint_policy` · crates/suprnova-web-push/src/client.rs:215
  - [ ] fn `suprnova_web_push::WebPushClient::send` · crates/suprnova-web-push/src/client.rs:220
- [ ] enum `suprnova_web_push::EndpointPolicy` · crates/suprnova-web-push/src/client.rs:62 (also `suprnova_web_push::client::EndpointPolicy`) (re-exported as `suprnova::EndpointPolicy`, `suprnova::web_push::EndpointPolicy`)
  - Variants: `Strict`, `AllowAny`

### `suprnova_web_push::error`

- [ ] enum `suprnova_web_push::WebPushError` · crates/suprnova-web-push/src/error.rs:7 (also `suprnova_web_push::error::WebPushError`) (re-exported as `suprnova::WebPushError`, `suprnova::web_push::WebPushError`)
  - Variants: `Vapid`, `Encryption`, `Http`, `PushServiceRejected`, `Base64`, `Json`, `SubscriptionGone`, `UnconfinedRedirects`, `Internal`
  - [ ] fn `suprnova_web_push::WebPushError::is_retryable` · crates/suprnova-web-push/src/error.rs:72
  - [ ] fn `suprnova_web_push::WebPushError::retry_after` · crates/suprnova-web-push/src/error.rs:91

### `suprnova_web_push::payload`

- [ ] struct `suprnova_web_push::Payload` · crates/suprnova-web-push/src/payload.rs:39 (also `suprnova_web_push::ece::Payload`, `suprnova_web_push::payload::Payload`)
  - [ ] fn `suprnova_web_push::Payload::body` · crates/suprnova-web-push/src/payload.rs:45
  - [ ] fn `suprnova_web_push::Payload::content_encoding` · crates/suprnova-web-push/src/payload.rs:48
  - [ ] fn `suprnova_web_push::Payload::encrypt` · crates/suprnova-web-push/src/payload.rs:55
- [ ] enum `suprnova_web_push::ContentEncoding` · crates/suprnova-web-push/src/payload.rs:9 (also `suprnova_web_push::ece::ContentEncoding`, `suprnova_web_push::payload::ContentEncoding`) (re-exported as `suprnova::ContentEncoding`, `suprnova::web_push::ContentEncoding`)
  - Variants: `Aes128Gcm`
  - [ ] fn `suprnova_web_push::ContentEncoding::header_value` · crates/suprnova-web-push/src/payload.rs:15
- [ ] const `suprnova_web_push::payload::AUTH_SECRET_LEN` · crates/suprnova-web-push/src/payload.rs:35
- [ ] const `suprnova_web_push::ece::MAX_PLAINTEXT_BYTES` · crates/suprnova-web-push/src/payload.rs:24 (also `suprnova_web_push::payload::MAX_PLAINTEXT_BYTES`)
- [ ] const `suprnova_web_push::payload::P256DH_KEY_LEN` · crates/suprnova-web-push/src/payload.rs:30

### `suprnova_web_push::vapid`

- [ ] struct `suprnova_web_push::VapidClaims` · crates/suprnova-web-push/src/vapid.rs:72 (also `suprnova_web_push::vapid::VapidClaims`) (re-exported as `suprnova::VapidClaims`, `suprnova::web_push::VapidClaims`)
  - Public fields: `aud`, `exp`, `sub`
- [ ] struct `suprnova_web_push::VapidKey` · crates/suprnova-web-push/src/vapid.rs:14 (also `suprnova_web_push::vapid::VapidKey`) (re-exported as `suprnova::VapidKey`, `suprnova::web_push::VapidKey`)
  - [ ] fn `suprnova_web_push::VapidKey::generate` · crates/suprnova-web-push/src/vapid.rs:25
  - [ ] fn `suprnova_web_push::VapidKey::from_bytes` · crates/suprnova-web-push/src/vapid.rs:32
  - [ ] fn `suprnova_web_push::VapidKey::from_pem` · crates/suprnova-web-push/src/vapid.rs:45
  - [ ] fn `suprnova_web_push::VapidKey::to_pem` · crates/suprnova-web-push/src/vapid.rs:51
  - [ ] fn `suprnova_web_push::VapidKey::public_key_uncompressed_b64url` · crates/suprnova-web-push/src/vapid.rs:62
- [ ] struct `suprnova_web_push::VapidSigner` · crates/suprnova-web-push/src/vapid.rs:93 (also `suprnova_web_push::vapid::VapidSigner`) (re-exported as `suprnova::VapidSigner`, `suprnova::web_push::VapidSigner`)
  - [ ] fn `suprnova_web_push::VapidSigner::new` · crates/suprnova-web-push/src/vapid.rs:98
  - [ ] fn `suprnova_web_push::VapidSigner::sign` · crates/suprnova-web-push/src/vapid.rs:109
  - [ ] fn `suprnova_web_push::VapidSigner::public_key_b64url` · crates/suprnova-web-push/src/vapid.rs:158
