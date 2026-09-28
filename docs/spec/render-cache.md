# RenderCache

Status: Agreed 2026-09-13
Prefix: CACHE

Requirements drawn from the adversarial audit of 2026-09-13 (Astra,
report outside the repository; findings ASTRA-02, 03, 04, 06, 08, 09, 10,
11, 12, 13, each reproduced over loopback HTTP against `0c386d60`). Each
requirement refines a sentence already agreed in Live specs 15-18 and
names the falsifier the audit demonstrated; the mechanism for each is
that probe ported into `framework/tests/render_cache/hardening.rs` and
run by name. The developer agreed the ten requirements and their
falsifiers as one set on 2026-09-13, with LIVE-016 to LIVE-018.
The framework is the actor throughout: `framework/src/render_cache/`
over the engine in `crates/suprnova-live/src/render_cache/`.

## Response safety contract

[CACHE-001] The framework MUST NOT store a response whose `Cache-Control`
carries `no-store`. The framework MUST NOT replace a handler's `no-store`
with the route policy's directives.
Falsifier: a public cached route answering `Cache-Control: no-store`
renders once and the second request replays it with
`public, max-age=60, s-maxage=60` (ASTRA-02).
Mechanism: `.cairn/mechanisms/cache-no-store`.
Refines: Live spec 15, "Cache-Control, Vary, surrogate directives, age,
and private/public markers agree with variance and coherence policy".
Status: Agreed 2026-09-13

[CACHE-002] The framework MUST parse the final response's `Vary` header
before publication. The framework MUST decline storage when `Vary` names
a field that is not a declared, normalized key dimension, or when it is
`*`.
Falsifier: a handler answering the request's `X-Flavor` with
`Vary: X-Flavor` serves the `vanilla` body to a `chocolate` request from
storage, with no `Vary` header (ASTRA-09).
Mechanism: `.cairn/mechanisms/cache-vary`.
Refines: Live spec 16, "Vary headers and server-side key dimensions
remain consistent" and "the key, the Vary header, and the stored
representation SHALL agree".
Status: Agreed 2026-09-13

[CACHE-003] The framework MUST decline storage of a response that carries
`Content-Disposition`, `Cross-Origin-Opener-Policy`,
`Cross-Origin-Embedder-Policy`, `Cross-Origin-Resource-Policy`,
`Permissions-Policy`, or `X-Frame-Options` unless it replays that header
byte for byte.
Falsifier: an HTML response with `Content-Disposition: attachment` is
served from storage without it, so the same bytes render inline
(ASTRA-11).
Mechanism: `.cairn/mechanisms/cache-security-headers`.
Refines: Live spec 15, "unsafe per-request headers are never replayed
from storage" and the entry-codec rule at its 2026-09-06 revision.
Status: Agreed 2026-09-13

[CACHE-004] The framework MUST NOT replay a `Content-Security-Policy`
that carries a nonce source from a complete entry. The framework MUST
either decline storage of such a response or issue a fresh nonce in the
header and every matching body occurrence on each hit.
Falsifier: a route minting a nonce per render serves the same
`script-src 'nonce-...'` and body nonce on a cache hit (ASTRA-12).
Mechanism: `.cairn/mechanisms/cache-csp-nonce`.
Refines: Live spec 16, "Per-response CSP nonces, CSRF data, and other
request-specific metadata are generated at assembly time where
required".
Status: Agreed 2026-09-13

[CACHE-005] The framework MUST store a response's content coding with its
body. The framework MUST replay `Content-Encoding` on every hit.
Falsifier: a gzip-encoded response replays its encoded bytes without
`Content-Encoding` (ASTRA-04).
Mechanism: `.cairn/mechanisms/cache-content-encoding`.
Refines: Live spec 16, Media and Encoding negotiation.
Status: Agreed 2026-09-13

[CACHE-006] The framework MUST NOT publish a representation under the GET
key from a HEAD render unless the handler rendered the complete GET body.
Falsifier: a cold HEAD on a route that renders an empty body for HEAD
leaves later GET requests answering zero bytes (ASTRA-03).
Mechanism: `.cairn/mechanisms/cache-head-first`.
Refines: Live spec 15, replayable representation = body bytes and
metadata of the represented variant.
Status: Agreed 2026-09-13

[CACHE-007] The framework MUST honor a request's `Cache-Control: no-cache`
by revalidating or rendering fresh. The framework MUST honor a request's
`Cache-Control: no-store` by bypassing lookup and publication for that
request.
Falsifier: a `no-cache` request is answered from storage with `Age`, and
a cold `no-store` request populates the cache for the next request
(ASTRA-13).
Mechanism: `.cairn/mechanisms/cache-request-directives`.
Refines: Live spec 15, HTTP cache metadata so browsers and proxies "can
participate".
Status: Agreed 2026-09-13

## Database and coherence contract

[CACHE-008] The framework MUST run a cache-miss render against the
connection each query selects. The framework MUST decline publication
when a render's query selects a connection other than the snapshot's.
Falsifier: `DB::table_on("audit_aux", ...)` returns the auxiliary row
uncached and the primary row under a cached route (ASTRA-06).
Mechanism: `.cairn/mechanisms/cache-named-connection`.
Refines: Live spec 18, rebuild data and generations share one consistent
read view.
Status: Agreed 2026-09-13

[CACHE-009] The framework MUST advance dependency generations in the same
database transaction as the data write it observes, for ORM, query
builder, and raw writes alike. When the two cannot share a transaction,
the framework MUST stop serving affected entries until advancement is
confirmed.
Falsifier: after the generation log table is removed, a raw `UPDATE`
commits, the API returns an error, and the next cached GET serves the
pre-write body (ASTRA-10).
Mechanism: `.cairn/mechanisms/cache-write-atomicity`.
Refines: Live spec 17, "generation events are written as part of the
successful data transaction and become observable only after commit".
Status: Agreed 2026-09-13

[CACHE-010] When the framework cannot open the snapshot transaction for a
render, it MUST serve that render uncacheable and release the rebuild
lease.
Falsifier: a render whose `begin` failed is published after its fallback
generation reread agrees (ASTRA-08).
Mechanism: `.cairn/mechanisms/cache-snapshot-failure`.
Refines: Live spec 18, "failed rebuild preserves the prior atomic entry
and releases/times out its singleflight ownership safely".
Status: Agreed 2026-09-13