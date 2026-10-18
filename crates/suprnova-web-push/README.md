# suprnova-web-push

Web Push (RFC 8030 + RFC 8291 + RFC 8292) for the
[Suprnova](https://github.com/eas4ai/suprnova) framework.

Ported from [`web-push 0.11.0`](https://crates.io/crates/web-push) with the
upstream `isahc`/`hyper 0.14` HTTP layer replaced by Suprnova's pinned
`reqwest 0.13`. The crypto (VAPID signing + ECE payload encryption) is
identical to upstream - only the transport changed.

## What you get

- `WebPushClient` - send encrypted notifications to FCM / Mozilla / Apple endpoints
- `VapidSigner` + `VapidKey` + `VapidClaims` - RFC 8292 application server signing
- `Payload` + `ContentEncoding` (aes128gcm) - RFC 8291 message encryption
- `SubscriptionInfo` - the browser-side subscription envelope
- `WebPushError` - typed error surface; `AddressClass` names the class of a
  refused address, never the address itself
- `EndpointPolicy` - three policies. `Strict`, the default, refuses a
  subscription endpoint that is not `https`, that names an IP address or a
  local or cloud metadata name, or whose name resolves to a loopback,
  private, link-local, multicast, reserved or metadata address, and the
  transport the client builds connects only to an address it checked.
  `Allowlist` runs every check of `Strict` and also refuses an endpoint
  whose origin is not in an `EndpointAllowlist`, such as
  `https://fcm.googleapis.com` or `https://*.notify.windows.com`, before
  its host name is looked up. `AllowAny` sends to any endpoint, for tests
  against a local mock server

## Install

The framework re-exports this crate as `suprnova::web_push`, and the
common pieces (`WebPushClient`, `SubscriptionInfo`, `VapidKey`, ...) at its
crate root, so application code rarely depends on this crate directly. If
you need the lower-level API:

```toml
[dependencies]
suprnova-web-push = { git = "https://github.com/eas4ai/suprnova.git", tag = "v4.3.0" }
```

## Usage

See `manual/web-push.md` in the Suprnova repo for the full guide, and the
framework's `WebPushChannel`, which adapts a `WebPushClient` to the
notifications `Channel` trait so a `Notification` that names `webpush` in
its channels is delivered without calling the client directly.

```rust,no_run
use suprnova_web_push::{ContentEncoding, SubscriptionInfo, VapidKey, VapidSigner, WebPushClient};

# async fn send() -> Result<(), Box<dyn std::error::Error>> {
let key = VapidKey::from_pem(&std::fs::read_to_string("vapid_private.pem")?)?;
let client = WebPushClient::new(VapidSigner::new(key), "mailto:admin@example.com")?;

let subscription: SubscriptionInfo = serde_json::from_str(/* from the browser */ "{}")?;
let response = client
    .send(&subscription, b"hello", ContentEncoding::Aes128Gcm, 3600)
    .await?;
println!("the push service answered {}", response.status);
# Ok(())
# }
```

`WebPushClient::new` builds a transport that follows no redirect and uses
no proxy. To change timeouts or TLS, pass a `reqwest::ClientBuilder` to
`WebPushClient::with_client_builder`; a `reqwest::Client` you built
yourself goes to `WebPushClient::with_client`, which under `Strict` or
`Allowlist` sends only after `allow_unconfined_redirects` and
`allow_unpinned_destinations` accept, by name, what the library cannot see
in a finished client.

## License

MIT
