# Session revocation reaches memberships in process, with a store re-check for other nodes

Level: Consequential
Decided by: Shawn
Rests on: LIVE-019, LIVE-020
Would be wrong if: a membership on the node that destroyed its session receives an event after the destruction, or a deployment observes one store read per membership per delivered event

## Decision

Ruled 2026-09-13 23:22, accepting the recommendation: session invalidation, session id regeneration, and destroy_for_user revoke this node's memberships directly, keyed by the session fingerprint the framework already computes and by the user id; other nodes learn of a destroyed session through a bounded re-check of the membership's session against the shared store before delivery. The alternative, asking the store on every delivery for every membership, was refused for its cost of one store round trip per membership per event.

## Realized by

(none yet: recorded, not built)
- cf7332d1  fix(live): end a membership with the session that opened it (LIVE-019, LIVE-020)
