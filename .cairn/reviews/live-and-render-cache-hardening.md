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

To be recorded as each hardening test lands: the safe violating example
for each mechanism is the audit's probe as written, which passes against
the current tree, and the mechanism is that probe with its assertion
inverted to the agreed behavior. Both directions are recorded here per
requirement, with the commit that flipped them.
