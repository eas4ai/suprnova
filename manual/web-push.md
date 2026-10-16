# Web Push

Web Push delivers a short message to a browser even when your site is
closed - the Service Worker wakes up, decrypts the payload, and shows
an OS-level notification. Suprnova ships the protocol end-to-end:
VAPID key generation, AES128GCM payload encryption, the HTTP transport,
and a `WebPushChannel` that plugs into the notifications subsystem so
the same `Notification` you send to mail or database also lands as a
push.

Reach for this when you want to alert users in real time without an
open WebSocket - order shipped, friend request, mention, balance
posted. If the user is on a desktop browser with the site closed, web
push is the only mechanism that reaches them; if they're on the site,
[Broadcasting](broadcasting.md) is usually a better fit.

The API is behind the `web-push` Cargo feature, which is enabled by default.
Applications using `default-features = false` must enable `web-push`
explicitly.

## The four pieces

Web Push has more moving parts than mail or database, because the
spec ([RFC 8030](https://datatracker.ietf.org/doc/html/rfc8030) +
[RFC 8291](https://datatracker.ietf.org/doc/html/rfc8291) +
[RFC 8292](https://datatracker.ietf.org/doc/html/rfc8292)) splits
identity, encryption, and transport across three contracts:

| Piece | What it is |
|---|---|
| `VapidKey` / `VapidSigner` | A P-256 ECDSA keypair used to sign JWTs that prove your server is who it claims to be |
| `WebPushClient` | The HTTP client that encrypts a payload, signs a VAPID JWT, and POSTs to the subscription's endpoint |
| `WebPushChannel` | The notifications-subsystem adapter that turns a `Notification` into a `WebPushClient::send` call |
| `SubscriptionInfo` | The opaque (`endpoint`, `p256dh`, `auth`) triple the browser hands you when a user subscribes - you store it; you don't generate it |

The bottom three layers - `VapidKey`, `WebPushClient`, the encrypted
POST - are re-exported from `suprnova::web_push` so applications never
need to depend on the underlying `suprnova-web-push` crate directly.

## Generate a VAPID keypair

Web Push uses VAPID (Voluntary Application Server Identification) to
let push services rate-limit and contact misbehaving senders. You need
one P-256 keypair per application; the public key goes into your
frontend so the browser can pin subscriptions to your server, and the
private key stays on the server signing JWTs.

Generate one once, persist it, and reuse it forever:

```rust
use suprnova::VapidKey;

let key = VapidKey::generate();

// Save the PEM somewhere durable - a secrets manager, a file the deploy
// pipeline mounts, an env-vars-as-files volume. You CANNOT regenerate
// this without invalidating every existing subscription.
let pem = key.to_pem()?;
std::fs::write("vapid_private.pem", &pem)?;

// The frontend needs the base64url-no-padding uncompressed public key.
// Hand this to your JS so `pushManager.subscribe()` can use it as
// `applicationServerKey`.
println!("PUBLIC_VAPID_KEY={}", key.public_key_uncompressed_b64url());
```

At boot, load the saved PEM:

```rust
use suprnova::{VapidKey, VapidSigner};

let pem = std::fs::read_to_string("vapid_private.pem")?;
let key = VapidKey::from_pem(&pem)?;
let signer = VapidSigner::new(key);
```

A `VapidSigner` produces JWTs but does not send anything - it's
purely a signing primitive. The next layer wraps it.

## Build a WebPushClient

`WebPushClient` is the HTTP-side primitive: feed it a signer and a
contact URI ("how the push service can reach you if you misbehave"),
get back an object whose `send` method encrypts a payload, signs a
JWT, and POSTs to the subscription endpoint.

```rust
use std::sync::Arc;
use suprnova::{VapidKey, VapidSigner, WebPushClient};

let signer = VapidSigner::new(VapidKey::from_pem(&pem)?);

// The subject MUST be a mailto: URI or an https: URL per RFC 8292 §2.1.
// Anything else is rejected at construction so a misconfigured deploy
// fails fast at boot - not silently after the first failed dispatch.
let client = WebPushClient::new(signer, "mailto:ops@example.org")?;

let client = Arc::new(client);
```

Why `Arc<WebPushClient>`? `WebPushClient` wraps a `VapidSigner` which
wraps a private `ES256KeyPair`. None of those are `Clone` - private
keys shouldn't be casually duplicated - and constructing a fresh
signer for every channel registration would mean N independent VAPID
identities for the same application. Wrapping in `Arc` lets a single
signed identity back every registration and every concurrent delivery.

### Endpoint policy

Subscription endpoints are user-derived data: the browser receives the
URL from a remote push service when a user subscribes, and your server
stores whatever the browser handed back. A maliciously stored
subscription can point the HTTP POST anywhere reachable, turning the
push sender into an SSRF gadget.

`WebPushClient` defaults to `EndpointPolicy::Strict`, which checks the
endpoint before anything is sent:

- The scheme must be `https`.
- The host must be a name, not an IP address.
- The name must not be `localhost`, a cloud metadata name such as
  `metadata.google.internal`, or a name under an RFC 2606 reserved
  top-level domain (`.localhost`, `.local`, `.internal`, `.test`,
  `.example`, `.invalid`).
- Every address the name resolves to must be on the public internet. The
  client looks the name up and refuses the send when any answer is a
  loopback, private, link-local, multicast, unspecified, documentation
  or reserved address, or a cloud metadata address (`169.254.169.254`,
  `fd00:ec2::254`, `100.100.100.200`). Private covers RFC 1918, the RFC
  6598 shared address space and IPv6 unique local addresses.

The last check matters because the name in a stored endpoint is not
yours: a name someone else registered can resolve to `127.0.0.1` or to
`10.0.0.5`. After the check, the client connects to one of the addresses
it checked. TLS and the `Host` header still use the name, and the name
is not looked up a second time, so a DNS answer that changes between the
check and the connection (DNS rebinding) cannot move the request.

Real push services (FCM, Mozilla Autopush, Apple's `web.push.apple.com`)
pass every check.

A refused address returns `WebPushError::RefusedDestination`. It names
the class of the address, such as `AddressClass::Private`, and never the
address itself. A lookup that fails, or a name that resolves to no
address, returns `WebPushError::Lookup`, which `is_retryable()` reports as
retryable.

The connection goes to a checked address when the client builds its own
transport, with `WebPushClient::new` or
`WebPushClient::with_client_builder`. That transport sends through no
proxy. A proxy set on the builder is dropped, and `HTTPS_PROXY` and the
other proxy variables are not read: a proxy receives the name and
resolves it again, where the check cannot follow.

A client you pass to `WebPushClient::with_client` resolves the name
itself when it connects, so the check cannot hold the connection to the
addresses it checked. Under `Strict`, a send over such a client returns
`WebPushError::UnpinnedTransport` before anything is sent, until you
call `.allow_unpinned_destinations()` on the `WebPushClient`. After that
call, the endpoint and every address the name resolves to where your
application runs are still checked, and the connection goes wherever
your client sends it. See [Custom transport](#custom-transport).

For local integration tests against a `wiremock` mock server you have
to opt out:

```rust
use suprnova::{EndpointPolicy, WebPushClient};

let client = WebPushClient::new(signer, "mailto:test@example.org")?
    .with_endpoint_policy(EndpointPolicy::AllowAny);
```

Do not use `AllowAny` in production. The strict checks exist to keep
a tampered subscriptions table from being weaponised.

If your push service runs on a private network, `Strict` refuses it,
because its name resolves to a private address. The only policy that
reaches it is `AllowAny`, which turns off every check, so use it only when
your application, not a browser, decides which endpoints are stored.

### Endpoint allowlist

`Strict` refuses private and local destinations, but any public host
passes it, so a tampered subscription can still make your server POST to a
host of the attacker's choice. A browser subscribes only through its
vendor's push service, so when your subscriptions come from browsers you
can name every host they reach. `EndpointPolicy::Allowlist` refuses every
other host:

```rust
use suprnova::{EndpointAllowlist, EndpointPolicy, WebPushClient};

let allowlist = EndpointAllowlist::new([
    "https://updates.push.services.mozilla.com",
    "https://fcm.googleapis.com",
    "https://web.push.apple.com",
    "https://*.notify.windows.com",
])?;
let client = WebPushClient::new(signer, "mailto:ops@example.org")?
    .with_endpoint_policy(EndpointPolicy::Allowlist(allowlist));
```

These are the origins of the known browser push services:

| Browser | Push service origin |
|---|---|
| Firefox | `https://updates.push.services.mozilla.com` |
| Chrome and most other Chromium browsers | `https://fcm.googleapis.com` |
| Safari | `https://web.push.apple.com` |
| Microsoft Edge | `https://*.notify.windows.com` |

The allowlist runs every check of `Strict`, with the same errors. It checks
the endpoint's origin after the checks of the URL's shape and before the
host name is looked up, so an endpoint at a host outside the list causes
no DNS query and no connection. An endpoint the list admits then goes
through the address check and connects to a checked address, as under
`Strict`.

An origin is `https://host` or `https://host:port`, with no path, no
trailing slash, no query and no user name. The host is a name, not an IP
address. An endpoint matches an origin when its scheme is `https`, its host
is the same name in any letter case, and its port is the same; an endpoint
without a port uses 443. `EndpointAllowlist::new` returns
`InvalidAllowlist`, which quotes the entry, for any other form, and for an
empty list.

Microsoft's push service spreads its subscriptions over many hosts, such
as `wns2-par02p.notify.windows.com`, so its origin needs a wildcard. A host
can start with the label `*.`, which matches exactly one label:
`https://*.notify.windows.com` admits `wns2-par02p.notify.windows.com` but
not `a.b.notify.windows.com` or `notify.windows.com`. The wildcard is the
whole first label and appears once, and the name after it has two labels
or more, so `https://*.com` is refused.

An endpoint at another origin returns `WebPushError::OriginNotAllowed`
before anything is sent. It carries the origin, such as
`https://push.example.net`, and never the path of the endpoint, which is
the token that reaches the browser. `is_retryable()` returns `false`.

To send one message under a policy other than the client's own, such as
from a client that sits behind an `Arc` shared with other senders, call
`WebPushClient::send_with_policy`.

#### Set the allowlist for the notifications channel

`WebPushChannel` reads the allowlist from `WEB_PUSH_ENDPOINT_ORIGINS`, a
comma-separated list of origins in the form above:

```env
WEB_PUSH_ENDPOINT_ORIGINS=https://updates.push.services.mozilla.com,https://fcm.googleapis.com,https://web.push.apple.com,https://*.notify.windows.com
```

When the variable names one or more origins, every send of the channel
goes under `EndpointPolicy::Allowlist` with them, whatever policy you built
the client with, `AllowAny` included. Unset, or blank, the client's own
policy applies. Spaces around an entry and empty entries are ignored.
`WebPushChannel::new` reads the variable and returns an error that names it
when an entry is not an origin, so a typo fails at boot, where you build
the channel, instead of leaving the channel open to every host.

Set the variable when your application stores subscriptions from browsers.
Leave it unset when your push service runs on a private network, because
the allowlist keeps the strict address check, which refuses it.

### Custom transport

`WebPushClient::new` applies a 30-second per-request timeout. If you
need a different transport policy, such as pinned TLS or a shorter
timeout, pass a `reqwest::ClientBuilder` to
`WebPushClient::with_client_builder`. Your builder options are honoured,
but three settings are replaced after yours, because each would let a
send reach an address the strict policy did not check:

- Redirects are disabled. A validated endpoint that answers 3xx must not
  bounce the POST to an unvalidated URL.
- The DNS resolver is replaced by the one that connects a checked name
  to its checked addresses, and looks any other name up as usual.
- No proxy is used, whether you set one on the builder or the
  environment names one.

A name you map with `ClientBuilder::resolve` or
`ClientBuilder::resolve_to_addrs` still connects where you mapped it,
not to the checked address. reqwest applies a mapping only to a host
name equal to the name you mapped, compared without case, so a stored
endpoint reaches your address only at a name you chose, and cannot add
a mapping of its own. Map only a name whose address you want every
endpoint at that name to reach. `ClientBuilder::unix_socket` and
`ClientBuilder::connector_layer` also decide where a connection goes,
and the library cannot see them, so use them only to reach an address
you chose.

```rust
use reqwest::Client;
use std::time::Duration;
use suprnova::WebPushClient;

let client = WebPushClient::with_client_builder(
    Client::builder().timeout(Duration::from_secs(10)),
    signer,
    "mailto:ops@example.org",
)?;
```

`WebPushClient::with_client` takes an already-built client whose
redirect policy and name resolution the library cannot inspect. Under
the default `Strict` policy, a send over such a client is refused before
any I/O until you accept both by name:

- `.allow_unconfined_redirects()`, when the client does not follow
  redirects. Without it, the send returns
  `WebPushError::UnconfinedRedirects`.
- `.allow_unpinned_destinations()`, when you accept that the client
  connects wherever it resolves the name. Without it, the send returns
  `WebPushError::UnpinnedTransport`.

A push service you can reach only through a proxy needs such a client:

```rust
use std::time::Duration;
use suprnova::WebPushClient;

let http = reqwest::Client::builder()
    .proxy(reqwest::Proxy::https("http://proxy.internal:3128")?)
    .redirect(reqwest::redirect::Policy::none())
    .timeout(Duration::from_secs(30))
    .build()?;
let client = WebPushClient::with_client(http, signer, "mailto:ops@example.org")?
    .allow_unconfined_redirects()
    .allow_unpinned_destinations();
```

The strict checks still run on the answers your application gets for the
name. When your application cannot resolve the name at all, every send
returns `WebPushError::Lookup`, and the only policy that sends is
`AllowAny`, which turns off every check.

## Wire WebPushChannel into notifications

The raw `WebPushClient::send` works - but the way you actually send
push notifications in Suprnova is through the
[Notifications](notifications.md) subsystem. A `Notification` declares
`vec!["webpush"]` in its `channels()`, a `Notifiable` recipient
returns a JSON-encoded `SubscriptionInfo` from `route_for("webpush")`,
and the bound `NotificationDispatcher` does the fan-out.

```rust
use std::sync::Arc;
use suprnova::{
    NotificationDispatcher, WebPushChannel, WebPushClient,
    notifications::set_dispatcher,
};

let client: Arc<WebPushClient> = Arc::new(
    WebPushClient::new(signer, "mailto:ops@example.org")?
);

// ttl_secs: how long the push service holds an undelivered message.
// 86_400 (24h) is a reasonable default for non-urgent notifications;
// drop to 60 for "act right now" alerts where a stale message is
// worse than no message.
let webpush = Arc::new(WebPushChannel::new(client, 86_400)?);

let dispatcher = NotificationDispatcher::new()
    .register_channel(webpush);

set_dispatcher(Arc::new(dispatcher))?;
```

`WebPushChannel::new` returns an error when `WEB_PUSH_ENDPOINT_ORIGINS`
holds an entry that is not an origin. See
[Set the allowlist for the notifications channel](#set-the-allowlist-for-the-notifications-channel).

`register_channel` is last-write-wins on the channel's `name()`, so
tests can swap in a stub without affecting the production binding.

## Define a notification

A push-bound notification is the same shape as any other Suprnova
notification - declare `"webpush"` in `channels()` and put whatever
JSON you want delivered into `data()`:

```rust
use serde::{Deserialize, Serialize};
use suprnova::Notification;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OrderShipped {
    pub order_id: i64,
    pub tracking_url: String,
}

impl Notification for OrderShipped {
    fn notification_name() -> &'static str {
        "OrderShipped"
    }

    fn channels(&self) -> Vec<&'static str> {
        vec!["webpush"]
    }

    fn data(&self) -> serde_json::Value {
        serde_json::json!({
            "title":   "Your order has shipped",
            "body":    format!("Track order #{}", self.order_id),
            "url":     self.tracking_url,
        })
    }
}
```

The `data()` JSON is what your Service Worker receives. Pick a stable
shape and document it for the frontend - Suprnova doesn't impose one,
because notification UI is a frontend concern.

## Route the recipient

A `Notifiable` returns the route for each channel it supports.
For Web Push, that route is the JSON-encoded `SubscriptionInfo` -
exactly what the browser produced via `PushSubscription.toJSON()`,
stored verbatim:

```rust
use suprnova::Notifiable;

pub struct User {
    pub id: i64,
    pub push_subscription_json: Option<String>,
}

impl Notifiable for User {
    fn route_for(&self, channel: &str) -> Option<String> {
        match channel {
            "webpush" => self.push_subscription_json.clone(),
            _ => None,
        }
    }
}
```

Returning `None` causes the dispatcher to skip the channel silently -
useful for users who haven't subscribed to push but still get email.

## Send it

Synchronous:

```rust
use suprnova::Notify;

let user = User::find(42).await?.unwrap();
Notify::send(&user, &OrderShipped {
    order_id: 1234,
    tracking_url: "https://ship.example.org/o/1234".into(),
}).await?;
```

Queued - pre-resolves the subscription route at queue time so the
worker doesn't need to re-load the user:

```rust
Notify::queue(&user, OrderShipped {
    order_id: 1234,
    tracking_url: "https://ship.example.org/o/1234".into(),
}).await?;
```

For `Notify::queue` to work, register the notification's factory at
boot so the worker can rebuild the JSON payload into the typed
notification:

```rust
suprnova::notifications::register_notification_factory::<OrderShipped>()?;
suprnova::queue::worker::register_job::<suprnova::SendNotificationJob>();
```

Behind the scenes, queued dispatch builds a `SendNotificationJob`
carrying `(notification_name, payload, per_channel_routes, channels)`.
The worker re-hydrates the notification, looks up `WebPushChannel` by
name on the bound dispatcher, and calls `deliver(route, &notification)` -
the same code path as the synchronous `Notify::send`.

## The browser side

Suprnova does not ship a JavaScript SDK - the browser side is plain
Web Push API. The flow your frontend needs to implement:

1. Register a Service Worker.
2. Ask the user for permission.
3. Subscribe via `pushManager.subscribe({ userVisibleOnly: true,
   applicationServerKey: <your VAPID public key> })`.
4. POST `subscription.toJSON()` to a Suprnova endpoint that stores
   it on the user row.

```js
// Service Worker registration (somewhere in your app entrypoint)
const registration = await navigator.serviceWorker.register('/sw.js');

if (Notification.permission === 'default') {
    await Notification.requestPermission();
}

if (Notification.permission === 'granted') {
    const subscription = await registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: window.PUBLIC_VAPID_KEY,
    });

    await fetch('/api/push/subscribe', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(subscription.toJSON()),
    });
}
```

Your Suprnova endpoint receives the JSON, validates the shape, and
stores it on the user - the string is opaque to your server, but it
must be the exact JSON the browser produced (the
`SubscriptionInfo` type uses `Deserialize` to parse it later):

```rust
use suprnova::{Auth, Request, Response, SubscriptionInfo, attrs, json_response};

pub async fn subscribe(req: Request) -> Response {
    let user_id = Auth::id().expect("auth middleware");

    let (_parts, bytes) = match req.body_bytes().await {
        Ok(b) => b,
        Err(e) => return json_response!({ "error": e.to_string() }).map(|r| r.status(400)),
    };
    let raw = match std::str::from_utf8(&bytes) {
        Ok(s) => s.to_string(),
        Err(_) => return json_response!({ "error": "body not utf-8" }).map(|r| r.status(400)),
    };

    // Parse to validate the shape - endpoint, keys.p256dh, keys.auth.
    // If parsing fails, the browser handed us something malformed.
    let sub: SubscriptionInfo = match serde_json::from_str(&raw) {
        Ok(s) => s,
        Err(e) => return json_response!({ "error": e.to_string() }).map(|r| r.status(400)),
    };

    // Persist `raw` verbatim - that's the exact string WebPushChannel
    // will hand to serde_json::from_str on dispatch.
    User::query()
        .db_where_op("id", "=", user_id)
        .update_all(attrs! { push_subscription_json: raw })
        .await
        .unwrap();

    json_response!({ "ok": true, "endpoint": sub.endpoint })
}
```

The Service Worker decrypts the push payload and renders the
notification:

```js
// /sw.js
self.addEventListener('push', (event) => {
    const data = event.data.json();
    event.waitUntil(
        self.registration.showNotification(data.title, {
            body: data.body,
            data: { url: data.url },
        }),
    );
});

self.addEventListener('notificationclick', (event) => {
    event.notification.close();
    event.waitUntil(clients.openWindow(event.notification.data.url));
});
```

## Payload limits

The Web Push spec caps each encrypted payload at 4096 bytes total.
Suprnova rejects plaintexts larger than 3992 bytes (the cap minus the
~85-byte AES128GCM encryption overhead) at encrypt time so the
failure surfaces in your code, not in a 413 from the push service.
A `Notification` whose serialized `data()` exceeds that limit
fails in the channel's `deliver`: the client returns
`WebPushError::Encryption`, and the channel reports it as an internal
error.

For anything larger - a long message body, a thumbnail - send a short
notification carrying a URL the Service Worker fetches on click. That's
both faster (no encryption on a multi-KB payload) and more flexible
(the fetch can return whatever shape you want).

## Dead subscriptions

When the push service returns 404 or 410, the subscription is dead -
the user uninstalled the browser, revoked the permission, or cleared
storage. `WebPushChannel` treats this as a non-fatal warn:

```text
WARN webpush subscription gone (404/410); caller should remove
     channel=webpush host=fcm.googleapis.com endpoint_sha256=4e9bdab8bbe7189c
```

Dispatch returns `Ok(())` because the notification reached a terminal
state - there's no recipient to retry against. Your application is
expected to act on the warn and remove the subscription row. Suprnova
ships the warn; it does not auto-prune the subscriptions table for you.

The warning has no `endpoint` field. The path of an endpoint is the token
that reaches the browser, so the log keeps it out. `host` is the host of
the endpoint, and `endpoint_sha256` is the first 16 hexadecimal digits of
the SHA-256 of the endpoint as you stored it. To find the row, compute
the same digest of each stored endpoint and compare:

```rust
use sha2::{Digest, Sha256};

fn endpoint_digest(endpoint: &str) -> String {
    hex::encode(&Sha256::digest(endpoint.as_bytes())[..8])
}
```

### Subscriptions that cannot be used

A stored subscription can be unusable before any request leaves your
server. The client then returns `WebPushError::InvalidSubscription`, and
nothing is sent. The text of the error names the rule that refused the
subscription. Two groups of checks raise it:

- The endpoint. It is no URL, or the `Strict` endpoint policy refuses it
  because it is not `https`, has no host, names an IP address in place of a
  host, or names a host that is no push service (`localhost`, a cloud
  metadata host, or a reserved name such as `.local` or `.internal`).
  When its host name resolves to an address the policy refuses, the error
  is `WebPushError::RefusedDestination`, which carries the class of the
  address in place of a text. When the allowlist does not name its origin,
  the error is `WebPushError::OriginNotAllowed`, which carries the origin.
- The keys. `p256dh` is no base64url, does not decode to 65 bytes, is not
  in uncompressed form or is no point of the P-256 curve. `auth` is no
  base64url or does not decode to 16 bytes.

The stored data is wrong, so a retry cannot help. `is_retryable()` returns
`false`. `WebPushChannel` handles these errors as it handles a gone
subscription: it logs a warning and dispatch returns `Ok(())`, so a queue
does not send the job again. For a refused address, the reason is the
class, such as `its host name resolves to a private address`. For an origin
outside the allowlist, it is the origin, such as
`its origin https://push.example.net is not in the endpoint allowlist`.
When that origin belongs to a push service your users subscribe through,
add it to `WEB_PUSH_ENDPOINT_ORIGINS` instead of removing the
subscription.

```text
WARN webpush subscription cannot be used; caller should remove
     channel=webpush host=10.0.0.7 endpoint_sha256=533bb6dc756981d0 reason=subscription endpoint host '10.0.0.7' is an IP literal; real push services use named hosts
```

The warning carries the reason, the host and the digest of the endpoint,
as the warning for a gone subscription does, and no `endpoint` field.

A stored route that is no subscription at all gets the same warning:
text that is no JSON, or JSON without an `endpoint` or without `keys`. The
reason is the kind of the mistake and its position, such as
`the value is not JSON (line 1, column 1)`. It never quotes the route,
which holds the secret of the subscription. This warning has no host and
no digest, because the route has no endpoint to take them from.

Your application removes the subscription row, as it does for a gone
subscription. Dispatch succeeds, so no `NotificationFailed` event fires;
you learn of it from the log. When you call `WebPushClient::send`
yourself, match on `WebPushError::InvalidSubscription`:

```rust
match client.send(&sub, payload, ContentEncoding::Aes128Gcm, 60).await {
    Ok(_) => (),
    Err(
        WebPushError::SubscriptionGone
        | WebPushError::InvalidSubscription(_)
        | WebPushError::RefusedDestination(_)
        | WebPushError::OriginNotAllowed(_),
    ) => {
        // remove the subscription
    }
    Err(e) => return Err(e.into()),
}
```

## Retries and Retry-After

When the push service returns a transient 5xx, 408, or 429, the
underlying `WebPushError::PushServiceRejected` carries the parsed
`Retry-After` hint (delta-seconds form only - HTTP-date form returns
`None`):

```rust
use suprnova::WebPushError;

match client.send(&sub, payload, ContentEncoding::Aes128Gcm, 60).await {
    Ok(_) => (),
    Err(e) if e.is_retryable() => {
        let wait = e.retry_after().unwrap_or(Duration::from_secs(30));
        tokio::time::sleep(wait).await;
        // ...try again, or push back onto the queue with a delay
    }
    Err(WebPushError::SubscriptionGone) => {
        // remove the subscription
    }
    Err(e) => return Err(e.into()),
}
```

The `Retry-After` hint is capped at 24 hours so a hostile server
can't park a worker on a multi-year sleep.

`PushServiceRejected` carries the status and the hint, and no part of the
body the push service sent. The body is remote data, and an error is what
your code logs, stores and shows. The client reads the first 200 bytes of
the body and writes them to its `debug` log, with control characters
escaped, and `WebPushChannel` fails the delivery with an error that names
the status alone, such as
`WebPushChannel: push service rejected (status 500)`.

When you use `Notify::queue`, the queue retries the job for you. A
`PushServiceRejected` that carries a `Retry-After` hint leaves
`WebPushChannel::deliver` as a `FrameworkError::RateLimited` with that
hint. The worker then retries the job after the hinted time instead of the
delay from the job's backoff schedule. A job that the push service refused
with `429` goes back when the service said it would take it, not earlier and
not later. The worker caps the wait at 24 hours
(`queue::retry::RETRY_HINT_CEILING`).

The attempt still counts against `Notification::max_tries`, and the job
dead-letters when the attempts run out. Every other failure, including a
rejection that carries no `Retry-After` hint, keeps the delay from
`Notification::backoff`.

## Telemetry

The notifications dispatcher wraps the fan-out in a
`notification.dispatch` info span tagged with the notification name
and channel count. Each successful delivery emits a
`NotificationSent` event; failures emit `NotificationFailed` carrying
the channel name, route, and error string. Wire any of those into
your metrics/log pipeline the same way you wire other framework
events - see [Events](events.md).

A dead subscription emits a structured WARN with `channel="webpush"`,
the host and the digest of the endpoint, and the notification name. A
subscription that cannot be used emits a WARN with the same fields and
the reason. Neither has the endpoint. That's the signal to scrape for an
automated subscription cleanup job.

### Why Suprnova diverges

Laravel's `WebPush` driver is a community package
(`laravel-notification-channels/webpush`) - not in core, separately
versioned, opinionated about ORM. Suprnova bakes Web Push into the
framework because the protocol is well-defined and the encrypted
HTTP POST is too small a contract to wrap in a third-party
abstraction. The notifications subsystem keeps the surface uniform:
the same `Notification` you send to mail or database also lands as a
push, no driver matrix, no separate config tree.

We also surface the strict-endpoint policy by default. The Laravel
community package leaves SSRF protection to the application; we
take the position that "the endpoint came from user data" is the
shape of every Web Push subscription, and the safe default belongs
in the framework, not in your code.

The retry classification (`is_retryable`, `retry_after`) is exposed
as typed methods on `WebPushError` rather than as a magic constant
table in the queue layer. The queue still owns retry policy - the
error tells you whether a retry could succeed and how long to wait;
the queue decides whether and when to dequeue again. Separating the
two means your custom retry strategies (exponential backoff,
jittered, capped) don't have to special-case Web Push.

## Testing

Stand up a `wiremock` server, point a `WebPushClient` at it with
`EndpointPolicy::AllowAny`, and assert on the requests it receives:

```rust
use std::sync::Arc;
use suprnova::{
    EndpointPolicy, NotificationDispatcher, Notify, VapidKey, VapidSigner,
    WebPushChannel, WebPushClient,
    notifications::set_dispatcher,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn order_shipped_pushes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/push"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;

    let signer = VapidSigner::new(VapidKey::generate());
    let client = Arc::new(
        WebPushClient::new(signer, "mailto:test@example.org")
            .unwrap()
            .with_endpoint_policy(EndpointPolicy::AllowAny),
    );
    let channel = Arc::new(WebPushChannel::new(client, 60).unwrap());

    let dispatcher = NotificationDispatcher::new().register_channel(channel);
    set_dispatcher(Arc::new(dispatcher)).unwrap();

    let user = test_user_with_subscription(&server.uri()).await;
    Notify::send(&user, &OrderShipped {
        order_id: 1,
        tracking_url: "https://ship.example.org/o/1".into(),
    }).await.unwrap();
    // server.received_requests() now contains the encrypted POST.
}
```

Leave `WEB_PUSH_ENDPOINT_ORIGINS` unset in a test that uses `AllowAny`.
When it names origins, the channel sends under the allowlist, which
refuses the mock server's `http` address.

For end-to-end tests that don't care about the encrypted bytes,
`Notify::fake()` (covered in [Notifications](notifications.md))
captures the dispatch without running the channel - faster, no
mock server, no encryption round-trip.

## Reference

- Primitives: `suprnova::VapidKey`, `suprnova::VapidSigner`,
  `suprnova::VapidClaims`
- Client: `suprnova::WebPushClient`, `suprnova::EndpointPolicy`,
  `suprnova::EndpointAllowlist`, `suprnova::PushResponse`,
  `suprnova::SubscriptionInfo`
- Error: `suprnova::WebPushError` - `.is_retryable()`, `.retry_after()`,
  `WebPushError::SubscriptionGone`, `WebPushError::InvalidSubscription`,
  `WebPushError::RefusedDestination`, `WebPushError::OriginNotAllowed`,
  `WebPushError::Lookup`, `WebPushError::PushServiceRejected`;
  `suprnova::InvalidAllowlist` for an origin list that does not parse
- Environment: `WEB_PUSH_ENDPOINT_ORIGINS` (see
  [Environment Variables](env-vars.md#web-push))
- Refused address classes: `suprnova::AddressClass`
- Encoding: `suprnova::ContentEncoding` (Aes128Gcm; 3992-byte plaintext cap)
- Channel: `suprnova::WebPushChannel`
- Facade: `suprnova::Notify`
- Queue job: `suprnova::SendNotificationJob`
- Factory registration:
  `suprnova::notifications::register_notification_factory`

## Next

- [Notifications](notifications.md) - the multi-channel dispatcher that
  `WebPushChannel` plugs into
- [Mail](mail.md) - the email-channel counterpart for users without push
- [Broadcasting](broadcasting.md) - real-time delivery for users who are
  on the site
- [Queues](queues.md) - how `Notify::queue` backs `SendNotificationJob`
- [Events](events.md) - listening for `NotificationSent` /
  `NotificationFailed` to watch deliveries
