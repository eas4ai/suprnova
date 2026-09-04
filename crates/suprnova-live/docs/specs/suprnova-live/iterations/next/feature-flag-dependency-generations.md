# Feature-flag dependency generations -- staged for next iteration

Status: Staged (not in current contract)
Captured: 2026-09-04
Target domain: `17-dependency-tracking-and-generations.md`

## What it is

`DependencyIdentity::Feature` exists in the engine, but nothing produces
one: `DatabaseEvaluator` reads an in-memory snapshot rather than the ORM,
so a flag read through `is_enabled!` records no dependency identity for the
flag it just consulted. A flag flip, or a new `user:`- or `team:`-scoped
rule that makes a previously global flag identity-dependent, therefore does
not invalidate a published RenderCache entry; the entry keeps serving its
old answer until it ages out on its own freshness schedule, which may be
long after the flag actually changed.

A related gap sits one layer up. `DatabaseEvaluator::reload()` refreshes
the in-memory snapshot from an out-of-band SQL change, but does not call
`sync::notify`. A `CachedEvaluator` sitting in front of that evaluator
therefore keeps its own cached, stale identity-scope bits (whether a flag
has a `user:` or `team:` rule at all, which is what
`crate::features::fields::observe_identity` reads to decide what to
record) until the cache's own TTL expires, independent of whatever
RenderCache does or does not invalidate. `set_flag` does call
`sync::notify`; only the out-of-band-SQL-plus-`reload()` path misses it.

A future revision may have the snapshot refresh advance a `Feature`
generation identified by the flag's name, and have `is_enabled!` observe
that generation the same way a table or record read already is observed,
so a flag change invalidates a published entry through the ordinary
coherence path rather than waiting for the entry to age out. The same
revision should also close the `reload()` notification gap, since a
generation-based fix to the first problem does nothing for a
`CachedEvaluator` that is still reasoning from stale identity-scope bits.

## Acceptance criteria

- A flag flip (`set_flag`, or an out-of-band SQL change reflected through
  `DatabaseEvaluator::reload()`) advances a generation an `is_enabled!`
  read of that flag has observed, and a published RenderCache entry that
  depended on the flag's answer is invalidated through the ordinary
  coherence path rather than only aging out.
- `DatabaseEvaluator::reload()` calls `sync::notify` (or an equivalent) so
  a `CachedEvaluator` in front of it does not keep serving stale
  identity-scope bits past an out-of-band change.
- The fix does not record a `Feature` dependency for a flag with no
  scoped rule at all, matching the existing "no ambient cost for a flag
  that does not depend on the reader" property `observe_identity` already
  preserves for the identity axes.
