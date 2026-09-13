# Decline storage of nonce-bearing CSP responses; hash-based CSP still caches

Level: Consequential
Decided by: Shawn
Rests on: CACHE-004
Would be wrong if: a public page that mints a per-response nonce is replayed from a complete entry with the same nonce in its header or body, or a hash-based CSP response is declined storage

## Decision

Ruled 2026-09-13 11:40, accepting the recommendation: when a response's
`Content-Security-Policy` carries a nonce source, the framework declines
complete-entry storage and serves the render uncacheable. Hash-based
policies carry no per-response secret and replay as they are. Rejected:
generalizing the stitched-shell re-nonce mechanism to every complete
entry (keeps nonce pages cacheable, but widens the body-templating
surface the audit found only in `framework/src/render_cache/stitch.rs`
and costs far more than the fail-closed veto). The developer's words: "I
will accept your recommendations".

## Realized by

- 3995878c  docs(cairn): agree the hardening requirements as one set and record the review
- 15ba2924  fix(render-cache): never replay a CSP nonce from a complete entry (CACHE-004)
