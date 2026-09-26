# Suprnova feature map: `suprnova-web-push` Rust API

Source: `crates/suprnova-web-push/` at d03b4f1, rustdoc JSON (all features), cross-checked against a default-features build.

A checked box means the documentation for that item has been remediated
against the source. Items are listed under their shortest public path;
`also` names the other paths the same item is reachable by.

## Counts

- Top-level items (including re-exports): 14
- Members (methods, associated consts and types): 20
- By kind: constant 3, enum 3, struct 8

## client

### `suprnova_web_push::client`

- [ ] struct `suprnova_web_push::PushResponse` · crates/suprnova-web-push/src/client.rs:44 (also `suprnova_web_push::client::PushResponse`)
  - Public fields: `status`
- [ ] struct `suprnova_web_push::SubscriptionInfo` · crates/suprnova-web-push/src/client.rs:32 (also `suprnova_web_push::client::SubscriptionInfo`)
  - Public fields: `endpoint`, `keys`
- [ ] struct `suprnova_web_push::client::SubscriptionKeys` · crates/suprnova-web-push/src/client.rs:38
  - Public fields: `p256dh`, `auth`
- [ ] struct `suprnova_web_push::WebPushClient` · crates/suprnova-web-push/src/client.rs:96 (also `suprnova_web_push::client::WebPushClient`)
  - [ ] fn `suprnova_web_push::WebPushClient::new` · crates/suprnova-web-push/src/client.rs:116
  - [ ] fn `suprnova_web_push::WebPushClient::with_client_builder` · crates/suprnova-web-push/src/client.rs:140
  - [ ] fn `suprnova_web_push::WebPushClient::with_client` · crates/suprnova-web-push/src/client.rs:170
  - [ ] fn `suprnova_web_push::WebPushClient::allow_unconfined_redirects` · crates/suprnova-web-push/src/client.rs:185
  - [ ] fn `suprnova_web_push::WebPushClient::with_endpoint_policy` · crates/suprnova-web-push/src/client.rs:215
  - [ ] fn `suprnova_web_push::WebPushClient::send` · crates/suprnova-web-push/src/client.rs:220
- [ ] enum `suprnova_web_push::EndpointPolicy` · crates/suprnova-web-push/src/client.rs:62 (also `suprnova_web_push::client::EndpointPolicy`)
  - Variants: `Strict`, `AllowAny`

## error

### `suprnova_web_push::error`

- [ ] enum `suprnova_web_push::WebPushError` · crates/suprnova-web-push/src/error.rs:7 (also `suprnova_web_push::error::WebPushError`)
  - Variants: `Vapid`, `Encryption`, `Http`, `PushServiceRejected`, `Base64`, `Json`, `SubscriptionGone`, `UnconfinedRedirects`, `Internal`
  - [ ] fn `suprnova_web_push::WebPushError::is_retryable` · crates/suprnova-web-push/src/error.rs:72
  - [ ] fn `suprnova_web_push::WebPushError::retry_after` · crates/suprnova-web-push/src/error.rs:91

## payload

### `suprnova_web_push::payload`

- [ ] struct `suprnova_web_push::Payload` · crates/suprnova-web-push/src/payload.rs:39 (also `suprnova_web_push::ece::Payload`, `suprnova_web_push::payload::Payload`)
  - [ ] fn `suprnova_web_push::Payload::body` · crates/suprnova-web-push/src/payload.rs:45
  - [ ] fn `suprnova_web_push::Payload::content_encoding` · crates/suprnova-web-push/src/payload.rs:48
  - [ ] fn `suprnova_web_push::Payload::encrypt` · crates/suprnova-web-push/src/payload.rs:55
- [ ] enum `suprnova_web_push::ContentEncoding` · crates/suprnova-web-push/src/payload.rs:9 (also `suprnova_web_push::ece::ContentEncoding`, `suprnova_web_push::payload::ContentEncoding`)
  - Variants: `Aes128Gcm`
  - [ ] fn `suprnova_web_push::ContentEncoding::header_value` · crates/suprnova-web-push/src/payload.rs:15
- [ ] const `suprnova_web_push::payload::AUTH_SECRET_LEN` · crates/suprnova-web-push/src/payload.rs:35
- [ ] const `suprnova_web_push::ece::MAX_PLAINTEXT_BYTES` · crates/suprnova-web-push/src/payload.rs:24 (also `suprnova_web_push::payload::MAX_PLAINTEXT_BYTES`)
- [ ] const `suprnova_web_push::payload::P256DH_KEY_LEN` · crates/suprnova-web-push/src/payload.rs:30

## vapid

### `suprnova_web_push::vapid`

- [ ] struct `suprnova_web_push::VapidClaims` · crates/suprnova-web-push/src/vapid.rs:72 (also `suprnova_web_push::vapid::VapidClaims`)
  - Public fields: `aud`, `exp`, `sub`
- [ ] struct `suprnova_web_push::VapidKey` · crates/suprnova-web-push/src/vapid.rs:14 (also `suprnova_web_push::vapid::VapidKey`)
  - [ ] fn `suprnova_web_push::VapidKey::generate` · crates/suprnova-web-push/src/vapid.rs:25
  - [ ] fn `suprnova_web_push::VapidKey::from_bytes` · crates/suprnova-web-push/src/vapid.rs:32
  - [ ] fn `suprnova_web_push::VapidKey::from_pem` · crates/suprnova-web-push/src/vapid.rs:45
  - [ ] fn `suprnova_web_push::VapidKey::to_pem` · crates/suprnova-web-push/src/vapid.rs:51
  - [ ] fn `suprnova_web_push::VapidKey::public_key_uncompressed_b64url` · crates/suprnova-web-push/src/vapid.rs:62
- [ ] struct `suprnova_web_push::VapidSigner` · crates/suprnova-web-push/src/vapid.rs:93 (also `suprnova_web_push::vapid::VapidSigner`)
  - [ ] fn `suprnova_web_push::VapidSigner::new` · crates/suprnova-web-push/src/vapid.rs:98
  - [ ] fn `suprnova_web_push::VapidSigner::sign` · crates/suprnova-web-push/src/vapid.rs:109
  - [ ] fn `suprnova_web_push::VapidSigner::public_key_b64url` · crates/suprnova-web-push/src/vapid.rs:158
