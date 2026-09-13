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

### CACHE-002, `cache-vary`, 2026-09-13 14:45

Violating example: the test `vary_must_match_declared_dimensions` on the
tree at `1d1605d8`, with only the test and its `/vary-undeclared` support
route added. Result:

    FAIL hardening::vary_must_match_declared_dimensions
    assertion `left == right` failed: one variant's body was served to
    another
      left: "vanilla"
     right: "chocolate"

That is ASTRA-09 as the audit reproduced it. After the fix (the lead
render parses the response's `Vary` after eligibility and declines
publication on `*` or any field outside the declared variance's header
set, decline reason `vary_undeclared`,
`framework/src/render_cache/middleware.rs`) the same test passes with
each variant rendered and carrying its own `Vary`, and the full
`render_cache` binary (369 tests) passes.

### CACHE-009, `cache-write-atomicity`, 2026-09-13 15:40

Violating example: the test `write_and_generation_commit_together` on the
tree at `24feffc4`, with only the test and its `/write-atomicity/{id}`
support route added. Result:

    FAIL hardening::write_and_generation_commit_together
    assertion `left == right` failed: the row write rolled back with its
    failed advancement
      left: "after"
     right: "before"

That is ASTRA-10 as the audit reproduced it: the `UPDATE` reported an
error and the row was durable anyway. After the fix (every write terminal
runs under `render_cache::orm::atomic`, which opens one transaction the
row write and the advancement share; the ledger's missing-table swallow
is gone; a failed dedicated advancement suspends serving) the same test
passes with the row rolled back and the cache and database agreeing. The
full `render_cache` binary (370 tests), the `eloquent` binary (601), and
the `database` binary (121) pass. Two fixtures in `operations.rs` that
dropped a ledger table through the `DB` facade now drop it through the
raw connection, because the facade's own write hook would roll the drop
back: the behavior the requirement asks for, and the same reason the
hardening test drops its table that way.

The fallback sentence of CACHE-009 (serving stops while an advancement
that could not share a transaction is unconfirmed) is implemented
(`write_side::suspend_serving`) and tested under CACHE-008's
named-connection fixture; see the next entry.

### CACHE-008, `cache-named-connection`, 2026-09-13 16:30

Violating example: the test `named_connection_is_preserved` on the tree at
`89d94ea8`, with only the test, the `hardening_aux` connection fixture, and
the `/named-connection` support route added. Result:

    FAIL hardening::named_connection_is_preserved
    assertion `left == right` failed: the render read the row from the
    connection the query named
      left: "primary"
     right: "auxiliary"

That is ASTRA-06 as the audit reproduced it. After the fix (both read
resolvers in `framework/src/database/transaction.rs` route a read bound
for another connection to that connection even inside an ambient
transaction and tell the collector; the report's gate bucket folds the
flag into the content bucket; `lead_render` declines publication under
`foreign_connection_read`) the same test passes with the auxiliary row
served twice and rendered twice. The full `render_cache` binary (372
tests) passes. One diagnosis on the way: a plain handler's reads land in
the collector's gate bucket, because only the stitched path marks the
handler begun, and the fold into the content bucket copies each flag by
name; the new flag had to join that fold.

The same fixture carries CACHE-009's fallback test,
`serving_stops_while_a_named_connection_advance_is_unconfirmed`: a write
on the named connection whose dedicated advancement fails (the log table
removed through the raw connection) makes the next lookup miss, and a
later successful advancement (a write to a table the watched entry does
not depend on) makes stored entries servable again. It passes; the
successful shared-transaction path now confirms serving too, which the
first draft of `atomic` had left to the dedicated path alone.

One existing test pinned the old rule: `multiconnection::
transaction_ignores_on_name_routing` asserted that a read naming another
connection inside `DB::transaction` was rerouted to the primary. That is
the defect as the audit described it, so the test now asserts the agreed
behavior under the name `transaction_routes_named_reads_to_their_connection`,
and the manual's routing precedence list (Eloquent chapter, mirrored in
six locales) states the read exception. Writes keep routing through the
transaction. The `database` binary (121 tests), `eloquent` (601), and the
dogfood app (117) pass.

### LIVE-016, `live-async-revocation`, 2026-09-13 17:05

Violating example: the test `revoked_gate_ends_delivery` on the tree at
`905c0ce4`, with only the test added (Astra's probe with its last
assertion inverted). Result:

    FAIL hardening::revoked_gate_ends_delivery
    an event published after the Gate denied reached the old stream

That is ASTRA-01 as the audit reproduced it. After the fix (the issued
record carries the principal captured at issuance; `publish` is async,
asks the Gate again per membership outside the tables lock through the
same ability strings issuance uses, and retires a denied membership
through the unsubscribe path) the same test passes: the new subscription
is refused and the old stream never carries the marker within its
window. The full `live` binary passes (107 of 108; the one failure is
LIVE-018's test, not yet fixed).

Recorded limit, reported to the owner in the session summary: LIVE-016
also names the session and revocation state. The host carries a session
fingerprint at issuance and no per-membership revocation version, so a
membership outlives a destroyed session until its lifetime ends. The
Live spec 14 revision says the same; the requirement's session clause is
not narrowed here, it is left visibly open for the owner's ruling.

### LIVE-017, `live-action-transaction`, 2026-09-13 17:05

Under the recorded decision the mechanism's test is
`required_transaction_is_refused_until_real` (the mechanism's filter was
renamed from the draft's `required_transaction_rolls_back`, which named
the rejected option). Violating example on the tree at `905c0ce4`, with
only the test and the new error variant declared:

    FAIL hardening::required_transaction_is_refused_until_real
    a Required-transaction action is refused at registration:
    <LiveRegistryBuilder:redacted>

That is ASTRA-05 as the audit described it: a component declaring the
policy registered and served. After the fix (`LiveRegistryBuilder`
refuses any component whose action declares `transaction = "required"`
with `RegistryErrorKind::RequiredTransactionUnsupported`) the same test
passes, and a component without the policy still registers.

### LIVE-018, `live-async-issuance-cap`, 2026-09-13 17:40

Taken out of the audit's order because its unfixed test, added with
LIVE-016, made the workspace tests and so the gate on `cfea9215` red.
Violating example: `issuance_cap_holds_under_concurrency` on the tree at
`905c0ce4` (Astra's probe with its assertion inverted; the probe itself
never linked in the audit, so this is the first run of that schedule).
Its status histogram on the unfixed tree:

    {201: 462, 403 async_authority_invalid: 51}

That is ASTRA-07 as the audit reasoned it: none of 513 concurrent
issuances was refused by the per-scope limit. After the fix (the slot is
reserved under the same lock as the count, held by a guard released on
every error path and consumed under the insert lock) the histogram reads
`{201: 473, 403: 39, 409 async_subscription_limit: 1}`: exactly the
overflow refused, at most 512 admitted to authorization. The test asserts
that contract (admitted at most the limit, successes at most the limit,
exactly the overflow limited) and the full `live` binary (108 tests)
passes.

The 403 population is a separate observation, not this requirement's:
under that burst, some of the admitted requests on one SSE document
instance answer `async_authority_invalid` from the subscription
service's connect step. Captured in `.cairn/backlog/` from LIVE-018 for
the owner; not fixed here.

### CACHE-006, CACHE-005, CACHE-010, CACHE-007, 2026-09-13 18:10

Landed together in two stages so each baseline is honest: stage A added
the four tests, their support routes, the two new decline labels, the
content-coding plumbing with the field still unfilled, and the
snapshot-begin test seam with the fallback unchanged; stage B added only
the four behavior changes. Violating examples on the tree at `8f4c0638`
plus stage A:

    FAIL hardening::head_first_does_not_publish_get
      left: [] right: "GET body"            (ASTRA-03: the GET served the
                                              HEAD's empty body)
    FAIL hardening::content_encoding_replays
      left: None right: Some("gzip")         (ASTRA-04)
    FAIL hardening::snapshot_failure_is_uncacheable
      left: 1 right: 2                       (ASTRA-08: the fallback render
                                              was published)
    FAIL hardening::request_directives_are_honored
      left: 1 right: 2                       (ASTRA-13: no-cache served from
                                              storage)

After stage B (`cache-head-first`: a HEAD miss declines publication under
`head_render`; `cache-content-encoding`: the entry header carries the
handler's `Content-Encoding` and the hit builder emits it;
`cache-snapshot-failure`: the begin-failure fallback reports
`SnapshotUnavailable`, which declines under `snapshot_unavailable` and
releases the lease; `cache-request-directives`: `no-store` bypasses
lookup and publication, `no-cache` skips the lookup) all four pass and
the full `render_cache` binary (376 tests) passes. One test adjustment
on the way: a fresh render legitimately answers `Age: 0`, so the
directive test accepts an absent or zero `Age` and rejects any older
value.
