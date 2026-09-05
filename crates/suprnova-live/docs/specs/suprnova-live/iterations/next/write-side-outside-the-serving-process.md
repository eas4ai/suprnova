# The write side should run in every process that writes -- staged for next iteration

Status: Staged (not in current contract)
Captured: 2026-09-05
Target domain: `16-cache-variance-privacy-and-stitching.md`

## What it is

RenderCache's write side, the code that advances dependency generations when
a model is saved or deleted and that `bump_permission_version` advances a
persisted generation through, is opened by `RenderCache::install`. Only the
serving process calls `install`, because `install` also registers the
middleware and needs the router. A queue worker, a scheduled task, or a
console command that writes through the same ORM therefore advances no
generation at all, and a `bump_permission_version` call from such a process
returns without effect. A cached page that depends on a table those
processes write stays current only within its freshness window, and the
manual's remedy is an epoch advance after the job. This is correct in the
fail-closed sense (nothing stale is presented as fresh beyond `fresh_ms`) but
it makes the invalidation story depend on where a write happened rather than
on what it wrote.

A future revision may gate the write side on configuration rather than on
`install`: a process with RenderCache enabled opens the write-side
instrumentation at boot whether or not it serves HTTP, so that the
generation advances travel with the write regardless of the process that
made it. The middleware and the router stay in the serving process; only the
instrumentation and the ledger writes become process-neutral.

## Acceptance criteria

- A write made by a queue worker, a scheduled task, or a console command
  advances the same generations the same write would advance in the serving
  process, proven by a test that performs the write outside the served
  application and observes the next lookup rebuild.
- `bump_permission_version` from a non-serving process advances the persisted
  permission generation, proven the same way.
- A process with RenderCache disabled by configuration opens nothing and
  writes nothing to the ledger.
- The honest-boundary bullet and the manual bullet that describe today's
  limit are removed when this ships, not left as stale warnings.
