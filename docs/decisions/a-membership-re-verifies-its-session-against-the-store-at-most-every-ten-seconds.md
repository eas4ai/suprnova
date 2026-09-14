# A membership re-verifies its session against the store at most every ten seconds

Level: Consequential
Decided by: Shawn
Rests on: LIVE-020
Would be wrong if: a session destroyed on another node keeps receiving events for longer than ten seconds, or the re-check runs more often than once per membership per ten seconds

## Decision

Ruled 2026-09-13 23:22, accepting the recommendation: the re-check interval is ten seconds per membership, so a logged-out user receives events on another node for at most that long. The alternative, re-checking only when a subscription renews, adds no new constant but stretches that window to the subscription lifetime of two minutes.

## Realized by

(none yet: recorded, not built)
- cf7332d1  fix(live): end a membership with the session that opened it (LIVE-019, LIVE-020)
