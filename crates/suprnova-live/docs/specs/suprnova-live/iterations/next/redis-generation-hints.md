# Credible generation hints over Redis pub/sub -- promoted to iteration 006

Status: Promoted (iteration 006, 2026-09-08)
Captured: 2026-09-07
Promoted: 2026-09-08
Target domain: `18-cache-coherence-and-rebuilding.md`, with
`15-render-representations-and-storage.md` as the affected storage domain

## What it is

A validation lease is the interval during which a node serves a stored entry
without re-reading generation truth. The Tier 1 and Tier 2 providers this
iteration shipped do not change it: the lease is set by the route's policy,
it runs to its own end, and a write committed on another node becomes visible
to this one only when the lease expires and the coherence check runs again.
On the Redis tier the same is true even though every node is already
connected to one instance, because Redis carries bytes and instance records
and never generation truth.

A **credible generation hint** is a small message published on Redis pub/sub
when a dependency generation advances, naming the dependency digests that
moved and nothing else. A node that hears one for a digest it observed
shortens the validation lease it is already holding, so the next request for
that entry re-validates against the database ledger earlier than the policy
alone would have made it.

The whole value of the idea depends on one asymmetry. A hint SHALL only ever
shorten a lease, never extend one, never create one, and never stand in for
the database ledger as generation authority. A hint that is lost, duplicated,
delayed, reordered, or forged changes nothing about correctness: a node that
hears nothing behaves exactly as this build behaves today, and a node that
hears a hint pays one extra ledger read. That is why it is a hint and not a
protocol: nothing is ever served because a hint said so, and nothing is ever
held back because a hint did not arrive.

The transport is Redis pub/sub because a Tier 2 deployment already has the
connection and the key namespace. A deployment on the database tier, or one
that turns hints off, keeps today's behaviour unchanged, so this is an
optional accelerator on an accelerator and never a new required daemon.

Bounds a design has to settle before this is built: how many digests one hint
message carries and what happens to a larger advance; how a subscriber that
falls behind is dropped rather than allowed to grow a queue; how a shortened
lease interacts with an entry currently being rebuilt under a fenced lease;
and whether a hint's own authenticity is checked at all, given that believing
a forged hint costs a ledger read and nothing more.

## Acceptance criteria

- A hint can only shorten a validation lease a node already holds. A test
  proves that a hint naming a digest a node observed makes the next lookup
  re-validate earlier, and that no hint, however constructed, extends a
  lease, creates one, or causes an entry to be served that the coherence
  check would have refused.
- Hints are never authority. A test proves that a hit still reads the
  database generation ledger, and that a deployment receiving a stream of
  forged, duplicated, reordered, or stale hints serves exactly what the same
  deployment serves with hints disabled.
- Losing the hint channel entirely, or running with hints turned off, leaves
  behaviour identical to this build: the same entries are served, the same
  rebuilds are admitted, and the only difference is how soon a lease is
  re-validated.
- Publication and subscription are bounded. One hint message carries a
  bounded number of digests, a subscriber that falls behind is dropped rather
  than queued without limit, and neither the publisher nor the subscriber
  blocks a request that is not waiting on it.
- The database tier and the embedded tier are unaffected, and no new external
  daemon becomes required at any tier.
- Telemetry, if any is added, keeps the existing closed low-cardinality label
  rule and never names a route, a key, or a dependency identity.
