# Data write and generation advancement share one transaction on the autocommit path

Level: Consequential
Decided by: Shawn
Rests on: CACHE-009
Would be wrong if: a row write commits while its generation advancement rolls back and a cached entry keeps passing coherence, or the write path deadlocks the pool because the widened transaction holds a connection across the ORM executor

## Decision

Ruled 2026-09-13 11:40, accepting the recommendation: for ORM, query
builder, and raw writes with no ambient transaction, the framework opens
one transaction, performs the data write inside it, advances the
dependency generations inside it, and commits once, so the two cannot
diverge. This is the sentence Live spec 17 already carries: generation
events are written as part of the successful data transaction. Rejected:
a transactional outbox with fail-closed serving while invalidation is
uncertain (correct, but a second durable protocol where one transaction
suffices on the path the audit reproduced). The developer's words: "I
will accept your recommendations".

## Realized by

- 3995878c  docs(cairn): agree the hardening requirements as one set and record the review
- 56f524cd  fix(render-cache): commit a data write and its generation advancement together (CACHE-009)
