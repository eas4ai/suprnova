# Live browser runtime

Iteration 003 supplies the browser half of Live as standalone development
machinery. The final application facade remains `suprnova::live`; this host and
the private `@suprnova/live` package prove the runtime contract but do not claim
registered Suprnova router or asset-pipeline integration.

Initial island content is ordinary server-rendered HTML. JavaScript is required
only when a developer opts into Live directives, model synchronization, local
signals, effects, morphing, or Live actions. The runtime does not invent a
parallel no-JavaScript action protocol.

## Boot and configuration

The module entry exports `boot()` and the classic entry exposes the same frozen
API. Boot is idempotent per `Window` through `Symbol.for("suprnova.live.runtime.v1")`;
a conflicting value fails closed. A single inert element with id
`suprnova-live-config` and type `application/json` contains the exact closed
configuration object:

```html
<script id="suprnova-live-config" type="application/json">
{"asset_identity":"live-0.1.0","credentials":"same-origin","endpoint":"/_suprnova/live","max_parallel_per_island":2,"max_queued_per_island":16,"max_response_bytes":1048576,"protocol":{"maximum":2,"minimum":1},"request_timeout_ms":10000,"runtime_contract_version":1}
</script>
<script type="module" src="/assets/application.js"></script>
```

The external application module imports and boots the runtime without requiring
inline script:

```js
import { boot } from "/assets/suprnova-live.esm.js";

boot();
```

The classic artifact installs `window.SuprnovaLive`; an external classic
bootstrap calls `window.SuprnovaLive.boot()`.

Configuration parsing is bounded, duplicate-aware, and exact-keyed. Endpoints
default to the document origin; a cross-origin endpoint must be an explicit
HTTP(S) origin in `allowedEndpointOrigins`. Credentials, timeouts, response
bytes, queue depth, parallelism, protocol window, and asset identity are all
validated before listeners or observers are installed. Invalid configuration
therefore leaves the initial SSR content visible and inert.

Production ports use browser clocks, cryptographic randomness, `fetch`, native
navigation, observers, and schedulers. Tests may inject those ports through
`BootstrapOptions`; application markup cannot select them.

## Island lifecycle

An island root is identified by an empty `data-suprnova-live-island` marker and
closed, bounded `data-suprnova-live-*` metadata. Component, slot, document key,
runtime contract, protocol minimum, revision, snapshot kind, snapshot bytes,
and lazy-completion state must agree. Public seeds have revision zero and no
instance identifier. The first server intent obtains a cryptographic browser
nonce; the server remains the authority that promotes the seed and issues an
instance identifier.

One document runtime owns delegated listeners and observers. It discovers
initial and inserted islands, respects nested-island ownership, lazily connects
eligible roots, retires removed roots exactly once, and never sends a request
merely because a seed appeared. Each island owns its scheduler, model state,
signal graph, feedback targets, morph state, and resource ledger.

`pagehide`, `pageshow`, freeze, resume, DOM removal, and explicit `stop()` flow
through the same idempotent lifecycle. Suspension detaches active work without
forgetting restorable state; final disposal releases observers, listeners,
timers, transports, transitions, extensions, controllers, and island records.

## Extensions and diagnostics

Effects and public calls are registered at boot under closed names, versions,
schemas, phases, result limits, and deadlines. Markup or server responses can
select only registered entries. The runtime has no evaluator, dynamic import
from markup, raw-script effect, or browser-side snapshot signer.

Diagnostics are `off`, `errors`, or `verbose`. They accept only the closed
diagnostic ledger and retain neither arbitrary exceptions nor attacker-provided
payload text. Failure details use bounded codes so snapshots, model values,
URLs, credentials, and response bodies are not reflected into logs. The public
handle exposes only status, stop, registered-effect execution, and registered
calls.

See [Live directives](live-directives.md), [local reactivity](local-reactivity.md),
and [browser testing](browser-testing.md) for the authoring and evidence layers.
