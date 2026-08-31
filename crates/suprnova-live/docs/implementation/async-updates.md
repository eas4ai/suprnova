# Asynchronous updates

Iteration 004 implements typed server events, authorized logical
subscriptions, polling, multiplexed push transports, continuity recovery, and
the optional `async@1` browser artifact. This host-neutral engine and
conformance machinery now lives in the integrated internal crate. It does not
claim that Iteration 005's production broadcaster, Suprnova route adapter, or a
document-wide scheduler shared with uploads and Live actions is complete.

Suprnova can serve SSR pages, links, and forms without JavaScript, and a Live
island's initial HTML is server rendered. `live:poll`, `live:stream`, registered
browser events, fresh renders, and presentation signals require the Live
runtime. Live does not synthesize alternate no-JavaScript execution for those
directives.

An Askama-compatible external template declares freshness rather than a client
callback:

```html
<section
  data-suprnova-live-island
  live:stream.hybrid="orders"
  live:poll.visible.30s=""
  aria-busy="false"
  data-live-stream-state="disconnected"
>
  <h2>Orders</h2>
  <p data-live-stream-status role="status" aria-live="polite">
    Updates disconnected
  </p>
  <div data-suprnova-live-key="order-results">
    {# Server-rendered order rows belong here. #}
  </div>
</section>
```

`live:poll` has an empty value; its generated modifiers select immediate,
visibility, and one of the 5/15/30/60-second template intervals. The signed
subscription descriptor supplies the exact authoritative fallback interval and
jitter used for hybrid mode. A template cannot name an unregistered stream or
event.

## Event schemas

Events are Rust types with a stable registered browser operation name and a
closed payload contract. `EventMetadata` fixes source, bounded targets,
per-source sequence ordering, cycle policy, fanout, and contract version. A
stream may emit only event names and payload schemas present in both the
component metadata and its signed subscription descriptor.

The current internal metadata form is shown below. Iteration 005 owns any
`suprnova::live` macro-generated declaration; until that integration is proved,
this lower-level form is conformance machinery rather than application API.

```rust
let event = EventMetadata::from_payload_with_contract::<OrdersUpdated>(
    EventSource::Stream,
    BoundedTargets::new(vec![EventTarget::SelfIsland])?,
    EventOrder::PerSourceSequence,
    EventCyclePolicy::ForbidRepeatedIsland,
    1,
)?;

let subscription = SubscriptionMetadata::new(
    StreamName::parse("orders")?,
    BoundedTopics::new(vec![TopicName::parse("tenant/orders")?])?,
    BoundedEventNames::new(vec![BrowserOperationName::parse(
        OrdersUpdated::NAME,
    )?])?,
    SubscriptionModes::new(vec![
        SubscriptionMode::ServerSentEvents,
        SubscriptionMode::WebSocket,
    ])?,
    ReconnectPolicy::ResumeOrRefresh {
        maximum_attempts: NonZeroU8::new(3).expect("three is nonzero"),
    },
);
```

Targets are closed to the source island, direct parent, owned child, exact
registered island slot, validated current document, or exact registered browser
listener. `dispatchRegisteredEvent` validates the registered name, payload
schema, source, target scope, sequence, and cycle policy before core dispatch.
There is no server-supplied JavaScript, arbitrary DOM event name, executable
expression, or dynamic listener lookup. Refresh invalidations and bounded
presentation-signal writes are separate typed payload kinds and enter the
ordinary island scheduler.

Async envelope protocol v1 recognizes `refresh`, `browser_event`,
`presentation_signal`, `heartbeat`, `complete`, and `error`. Canonical envelopes
are limited to 64 KiB, nested depth 8, 1,024 object entries, 4,096-byte strings,
and a 32 KiB payload. Invalid or oversized input closes or degrades the affected
delivery scope with a safe typed code; payload contents do not enter logs.

## Subscription authorization

`SubscriptionMetadata` is registration, not current authority. The server
derives a canonical descriptor for the mounted component and signs it under the
independently versioned async-subscription v1 purpose. Claims bind capability,
stream, topics, registered event contracts, replay baseline, reconnect policy,
hybrid fallback, bounded authorization memo, protocol, and exclusive expiry.
Descriptors live for at most five minutes and are reverified against current
component registration and trusted request context.

Every logical membership is reauthorized against the current principal,
session, tenant, component, island/document scope, topics, and event set. A
separate short-lived transport credential is secret; it is not embedded in the
descriptor, DOM, URL, diagnostics, or telemetry. Credential rotation and
membership establishment repeat authority checks around asynchronous source
work so a stale authorization decision cannot be committed after an await.

One physical transport may multiplex many logical memberships only when their
document, origin, credential, and transport compatibility keys match. Adding or
removing a membership is descriptor-, binding-, stream-, and generation-bound.
Authorization loss removes that membership only; it does not tear down healthy
siblings unless the physical transport itself is no longer valid.

Server-sent events are same-origin. The built-in `BrowserWebSocketAdapter`
accepts only `session_cookie` authorization. Browser WebSocket cannot attach an
arbitrary `Authorization` header, and the built-in adapter rejects every other
authorization kind. Fetch-based SSE and polling may use bearer authorization
when the host policy permits it.

A custom bearer-authorized or cross-origin WebSocket transport requires an
explicit non-wildcard Origin allowlist and separate non-cookie credentials. It
must still satisfy the signed subscription, exact Origin, and authentication
contracts. Every WebSocket handshake requires an exact configured or allowlisted
origin match plus its accepted authentication policy. Missing, malformed,
unlisted, or opaque origins fail closed. Unapproved cross-site origins and every
attempt to use cookie authority cross-site fail closed before upgrade.
Browser-chosen topics, arbitrary origin reflection, query-string credentials,
and wildcard production origins are not accepted.

## Polling and push modes

Polling is an ordinary fresh-render request, never a timer-driven action. A
poll's empty directive value prevents template authors from naming a mutating
method. The browser applies deterministic jitter to bounded 1-300-second policy
intervals, allows at most one queued and one in-flight fresh render per island,
and skips visibility-bound work while hidden. `immediate`, `visible`, and
`always` adjust the generated eligibility contract; duplicate or conflicting
modifiers are rejected.

`live:stream="orders"` requests push with hybrid fallback. Explicit
`live:stream.hybrid="orders"` has the same freshness class and may pair with an
empty `live:poll` directive to override presentation of the fallback interval.
`live:stream.push-only="orders"` disables polling and conflicts with
`live:poll`. If the optional async artifact is absent or incompatible, ordinary
core Live remains available and unsupported async directives stay inert with a
bounded diagnostic.

The browser owns at most one physical SSE or WebSocket connection for each
compatible document transport key and multiplexes logical memberships over it.
SSE and WebSocket are transport choices for the same envelope and continuity
contract, not separate application protocols. The scheduler limits concurrent
authorization and same-origin connection attempts; the per-origin WebSocket
handshake scheduler never exceeds eight.

Lifecycle is centralized inside the async feature: document freeze, pagehide,
bfcache entry, island retirement, and feature disposal suspend or retire owned
polls, authorizations, memberships, sockets, timers, and callbacks. Resume,
pageshow, online, and visibility restoration re-check the current DOM and
authority before work restarts. Offline state pauses new polling and reconnect
attempts; reconnect delay is bounded and jittered rather than synchronized into
a storm.

## Continuity

Every authorized stream position is `(epoch, sequence)`. Delivery is ordered
per source. Exact duplicates are ignored, a successor advances the current
position, and stale epochs, sequence gaps, or unproved replay coverage cannot be
applied as if current. Replay prevalidation and admission are atomic for the
complete transcript. Invalid scope, epoch, sequence, coverage, authority, or
resource bounds therefore enter no replay dispatch. Dispatch then commits each
successful event in order; if a later dispatch fails, recovery reports and
preserves the truthful committed prefix, current position, degraded state, and
retained high-water mark. It resumes only the undispatched suffix after new
authority proves recovery.

On reconnect the browser supplies its last committed position. The server may
resume with a complete bounded replay, announce that the subscriber is already
current, or require an authoritative fresh render. `R100` proves that 100
logical memberships on one document recover through exactly one physical
reconnect handshake; the separate 16-document workload checks the eight
concurrent-handshakes-per-origin ceiling.

Fresh render is the recovery authority. Refresh payloads enqueue the normal
revision-bearing island request and use the existing response ordering and
commit-after-morph rules. At most one refresh is in flight and one is queued per
island. If a response, replay, or membership result arrives after retirement,
generation change, morph replacement, or a newer committed result, the browser
drops it.

## Degraded freshness

`current` means the membership has proved continuity through its known
high-water mark. `degraded` means freshness is no longer provable due to a gap,
queue pressure, replay failure, authorization uncertainty, transport failure,
or unavailable push. Degraded is a first-class freshness state, not an excuse
to apply best-effort messages out of order.

Hybrid mode falls back to bounded jittered polling while push is degraded, then
stops redundant fallback work only after continuity or a fresh render proves
the membership current. Push-only mode reports degraded freshness and retries
according to its bounded reconnect policy; it does not silently turn into
polling. A membership whose authorization is denied closes locally while other
memberships continue.

Status text is exposed through the server-rendered status element and
`data-live-stream-state`; assistive technology receives meaningful connected,
degraded, reconnecting, or closed status without event-by-event noise.
`aria-busy` describes an authoritative refresh, not the mere existence of an
open socket. Reduced-motion preference suppresses decorative stream
presentation only and never changes freshness semantics.

## Backpressure

The server queue is bounded to 64 unapplied envelopes and 256 KiB of canonical
envelope bytes per document delivery owner. A payload is at most 32 KiB and an
event declares at most 16 target scopes. Although the internal Rust metadata
type can represent fanout up to 1,024, browser authorization and registered-event
admission reject a contract above 256. The effective end-to-end event fanout
ceiling is 256 and signed registration or deployment policy may lower it. The
independent replay transcript limit is 1,024 envelopes; it is not a fanout
allowance. A component declares at most 32 subscriptions, and one server
document transport owns at most 128 logical memberships. Browser-side logical
membership and diagnostic/resource collections have their own closed bounds.

Replaceable refresh and presentation-signal work may coalesce only under the
same fully authorized key. Registered browser events are not silently dropped
or reordered to make room. Full admission, payload/fanout violations, detached
delivery, or an unprovable sequence transition records bounded pressure and
moves the affected membership to degraded or a typed closed state. Recovery
must cover the recorded high-water position before the pressure obligation is
cleared.

The feature exposes count-only resource and pressure observations for tests and
operations: memberships, physical transports, queued events/bytes, retained
payload ownership, timers, and reconnect activity. It does not expose topic
names, descriptor claims, credentials, event payloads, island HTML, or raw
authorization memos.

Iteration 004 does not implement the proposed document-wide cross-feature
arbiter that would prioritize actions, fresh renders, background refreshes, and
upload chunks together. Upload and async features each enforce their shipped
bounded ownership and lifecycle contracts; describing inter-feature priority,
fairness, or coalescing as present would be inaccurate.
