# Generation-ledger epoch rewind detection -- promoted to iteration 006

Status: Promoted (iteration 006, 2026-09-08)
Captured: 2026-09-08
Promoted: 2026-09-08
Target domain: `18-cache-coherence-and-rebuilding.md`

## What it is

A restore that rewinds the generation ledger's epoch below the value live nodes
or L1 entries already carry SHALL be detected. No entry stamped above the
ledger's current authority SHALL be served as current; such an entry SHALL be
treated as unproven and rebuilt. The restore procedure in the operations
chapter SHALL state the automatic behavior and SHALL keep manual steps only
where they are still needed.

After a database restore the ledger's epoch can sit below the value live nodes
and L1 entries were stamped with, because those nodes and entries recorded an
epoch the restored database no longer knows about. `RenderCache::advance_epoch`
is a plain increment, so it does not by itself lift the epoch back above that
high-water mark. The operations chapter today tells the operator to advance the
epoch and empty L1 by hand, which is safe when it is followed, and it asserts
nothing at all about the rewind itself; ruling R29 recorded that honest
boundary and raised the automatic detection as a capture candidate.

## Acceptance criteria

- A restore that rewinds the generation ledger's epoch below the value live
  nodes or L1 entries carry SHALL be detected.
- No entry stamped above the ledger's current authority SHALL be served as
  current, proven by a test that performs a rewind and observes the refusal and
  the rebuild.
- The operations chapter's restore procedure SHALL state the automatic
  behavior and SHALL keep the manual steps only where they are still needed.
