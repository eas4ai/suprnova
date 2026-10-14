# Primary-key point reads observe the record, not the table -- promoted to iteration 006

Status: Promoted (iteration 006, 2026-09-08)
Captured: 2026-09-08
Promoted: 2026-09-08
Target domain: `17-dependency-tracking-and-generations.md`

## What it is

A primary-key point read that returns a row SHALL observe that record's
generation rather than the whole table's generation. A point read that returns
no row, and every read whose row set is not fixed by a primary key, SHALL keep
the table observation, because only a table authority survives a row that does
not exist yet. A write to another row of the same table SHALL therefore leave a
point-read entry current instead of invalidating it.

`Model::find` calls `observe_table_read` unconditionally
(`framework/src/eloquent/model.rs`, around line 220), so today any write to a
table invalidates every published entry that read a row from that table, however
narrow the read was. The invalidation-storm workload records the consequence
directly as `every_write_invalidates_every_key: true`. Ruling R21 of the
RenderCache qualification plan recorded the unconditional table observation as
designed conservative behavior and raised the narrowing as a capture candidate
rather than a defect to fix inside iteration 005.

## Acceptance criteria

- A primary-key point read that returns a row SHALL observe that record's
  generation rather than the table's.
- A point read that returns no row, and every read whose row set is not fixed
  by a primary key, SHALL keep the table observation.
- A write to another row of the same table SHALL leave a point-read entry
  current; a write to the observed row, or its deletion, SHALL invalidate it.
- The invalidation-storm workload SHALL record the point-read ratio, and the
  generations chapter of the manual SHALL state the rule.
