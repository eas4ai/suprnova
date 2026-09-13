# Review - live-and-render-cache-hardening

## Specification review, 2026-09-13 (before agreement)

Reviewed `docs/spec/render-cache.md` (CACHE-001 to CACHE-010) and the
audit section of `docs/spec/live.md` (LIVE-016 to LIVE-018) against the
audit report and against the source the report cites. Every finding held
up where it was checked: `response_signals` carries header names only
(`framework/src/render_cache/middleware.rs`, near line 3393), the
replayable header set in `crates/suprnova-live/src/render_cache/entry.rs`
lines 68-77 omits `Content-Disposition` and admits CSP verbatim, the hot
path in `crates/suprnova-live/src/render_cache/hot.rs` lines 164-187
regenerates `Vary` from policy, and
`framework/src/live/ports/transaction.rs` documents its own no-op. The
spec lint ran clean after four sentences were split (LIVE-017, LIVE-018,
CACHE-005, CACHE-007 each carried two obligations).

### Attacked: contradictions

- CACHE-004 against Live spec 16's stitched-shell nonce mechanism: no
  contradiction; the stitched path already re-nonces, CACHE-004 governs
  the complete-entry path the audit exercised.
- CACHE-009 against the pool-deadlock history recorded in
  `framework/src/live/ports/transaction.rs`: that history concerns the
  Live action port, not the ORM write path; the autocommit path opens
  its own transaction today and can widen it.
- LIVE-017 against the macro, which accepts the Required policy: refusing
  at registration keeps the macro surface and moves the failure to boot,
  where a typed `RegistryError` already exists (LIVE-006).

### Attacked: falsifiers that would not catch a violation

Each falsifier is the audit's reproduced assertion, so each is known to
fail on the current tree. The weak one is LIVE-018: a 513-way race is
timing dependent. The mechanism will use a barrier-controlled authorizer
so every issuance waits at the same point, which the audit itself
recommends.

### Attacked: requirements no mechanism can check

CACHE-010 needs a `begin` failure injected deterministically; the test
support under `framework/tests/support/` has no such seam yet, so the
mechanism's test will add one. CACHE-009 needs the generation-log insert
to fail after the row write; the audit did this by dropping the log
table through the write-side test seam, which the test will repeat.

## Agreement record

Presented to the developer at 11:35 on 2026-09-13 as one set of thirteen
requirements with falsifiers and three decisions, each with a
recommended option. The developer's words at 11:40: "I will accept your
recommendations", then "confirmed". Recorded as `Status: Agreed
2026-09-13` on every block, the three decisions under `docs/decisions/`,
and the commitment's decisions section.

## Mechanism demonstrations

Recorded as each hardening test lands: the safe violating example for
each mechanism is the audit's probe as written, which passes against the
tree the audit examined, and the mechanism is that probe with its
assertion inverted to the agreed behavior. Both directions are recorded
here per requirement, with the commit that flipped them.

### CACHE-003, `cache-security-headers`, 2026-09-13 12:10

Violating example: the test `security_headers_replay_or_decline` on the
tree at `cdf31ac6`, with only the test and its `/security-headers`
support route added. Result:

    FAIL hardening::security_headers_replay_or_decline
    assertion `left == right` failed: Content-Disposition survived the
    second request byte for byte (served from storage)
      left: None
     right: Some("attachment; filename=report.html")

That is ASTRA-11 as the audit reproduced it: the second request was a
hit and carried no disposition. After the fix (the six headers join
`REPLAYABLE_HEADERS`, `crates/suprnova-live/src/render_cache/entry.rs`)
the same test passes, and the full `render_cache` binary (366 tests) and
the engine's render_cache tests (57) pass with it.

### CACHE-004, `cache-csp-nonce`, 2026-09-13 13:35

Violating example: the test `csp_nonce_is_never_replayed` on the tree at
`50f37943`, with only the test and its `/csp-nonce` support route added.
Result:

    FAIL hardening::csp_nonce_is_never_replayed
    assertion `left != right` failed: the CSP nonce was reused across
    requests (second served from storage)
      left: "script-src 'nonce-hardening-nonce-1'"
     right: "script-src 'nonce-hardening-nonce-1'"

That is ASTRA-12 as the audit reproduced it. After the fix (the lead
render declines Complete publication when the stored CSP names a nonce
source, decline reason `nonce_source_policy`,
`framework/src/render_cache/middleware.rs`) the same test passes with the
second request served by a fresh render, and the full `render_cache`
binary (367 tests, including the one that pins every decline label to the
operations chapter) passes with it.

### CACHE-001, `cache-no-store`, 2026-09-13 14:10

Violating example: the test `no_store_is_a_storage_veto` on the tree at
`a5a035ae`, with only the test and its `/no-store` support route added.
Result:

    FAIL hardening::no_store_is_a_storage_veto
    assertion `left == right` failed: the render keeps the handler's own
    directive
      left: Some("private, max-age=60")
     right: Some("no-store")

That is ASTRA-02 as the audit reproduced it, and one step worse than the
report described: the very first response already carried the policy's
directive in place of the handler's. After the fix (the engine's
eligibility check reads the response's `Cache-Control` and declines on
the `no-store` token, decline reason `no_store_directive`,
`crates/suprnova-live/src/render_cache/policy.rs`) the same test passes
with both requests rendered and both carrying `no-store`; the engine's
policy tests gain a unit test for the token rule, and the full
`render_cache` binary (368 tests) passes.
