# Iteration 006 implementation ledger

This ledger records implementation checkpoints for iteration 006, the sweep
of everything iteration 005 staged rather than built. It is evidence about
the current implementation state, not a replacement for the normative
Iteration 006 contract at
`docs/specs/suprnova-live/iterations/006.md`.

## 2026-09-08 -- RenderCache correctness and coherence

Plan E, the first of the sweep's five plans, closed definition-of-done items
1 to 6 and the parts of 15 those six touch. Six observations that were
either conservative to the point of uselessness or missing became exact.

### The engine surface

Four additions, and nothing else. `AuthorizationConsult` (`None`,
`TenantOnly`, `Principal`, with a `Principal`-absorbing `join`) replaced the
`authorization_read` boolean on `ObservedContext`, and
`ClassificationReason::AuthorizationTenantRead` is what a tenant-only
consult narrows through. `DependencyIdentity::UnkeyedWrite` took digest tag
10. `CoherenceCheck::Rewound { stamped, authority }` is reported before any
dependency comparison. `GenerationLedger::lift_epoch_above` is a required
method with no default body, because a ledger that cannot lift cannot claim
rewind safety.

### What the framework does with them

An authorization decision is bracketed by a consult window and judged by
what it recorded: principal material, or nothing resolvable, requires
`Principal`; tenant material alone requires `Tenant`. The framework's RBAC
statements were named and given their table lists, and three crate-private
observing helpers on `DB` run them without marking the render unobservable,
so an RBAC-gated route caches and a permission grant rebuilds it.

A `GlobalScope` declares `ScopeDependency::Constant` or the conservative
default `PerRequest`, and the registry brackets each evaluation with
`collector::resolvable_reads`: a per-request scope that read nothing the
collector can name is recorded as an undeclared read under
`global_scope:<type>`, which narrows the render to `Uncacheable`.
`suprnova::live::current_tenant()` is the instrumented accessor a gate body
or a scope reaches for, scoped by `LiveTenantMiddleware` around the rest of
the chain.

A flag read observes a `Feature` generation whenever the snapshot holds the
flag at any scope key; `set_flag` advances it after the snapshot swap, and
`reload` advances it for every flag its diff found changed and forwards
those names to caches alone through `on_snapshot_reloaded`, which
`DatabaseEvaluator` deliberately does not override.

The write side is a process-wide tri-state, probed at most once and never
inside a caller's transaction, so a queue worker, a scheduled task, or a
console command advances the same generations the server does while an
application with RenderCache disabled still issues no RenderCache SQL at
all.

A primary-key point read that returns a row observes that record and the
table's unkeyed-write identity; one that returns nothing observes the table.
Bulk, table-builder, and named-table raw writes advance both `Table` and
`UnkeyedWrite`.

An entry or a lease stamped above the authority is refused at any age, and
the detecting node lifts the ledger epoch past the stamp, drops its lease,
clears its L0, and increments `suprnova.render_cache.epoch_rewinds`.

### Evidence

Every rule above is proven by a named test; the iteration 006 checkpoint
list names them. The workloads bench gained
`point_read_invalidation_ratio` and now measures
`every_write_invalidates_every_key` as `false`; the checked-in result was
regenerated under its existing `exploratory` label, which is a result-shape
change and not a qualification refresh. No benchmark budget, wire format,
storage codec, or tier semantic changed.
