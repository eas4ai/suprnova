# Refuse registration of Required-transaction actions until the ambient transaction exists

Level: Consequential
Decided by: Shawn
Rests on: LIVE-017, LIVE-006
Would be wrong if: an action declaring `transaction = "required"` registers and serves while the transaction port is a no-op, or a component with only Optional or None policies is refused

## Decision

Ruled 2026-09-13 11:40, accepting the recommendation of the audit and
the reviewer: the registry fails with a typed `RegistryError` for any
component whose action declares the Required policy, because the port in
`framework/src/live/ports/transaction.rs` opens a transaction only to
roll it back and answers commit and rollback with success. A refused
contract is honest; a successful no-op is not. Rejected: implementing the
ambient transaction inside this commitment (the pool-deadlock history
recorded in that module is why it was left a no-op, and the fix belongs
to a commitment that can carry that risk on its own). The developer's
words: "I will accept your recommendations".

## Realized by

- 3995878c  docs(cairn): agree the hardening requirements as one set and record the review
- cecb87df  fix(live): re-authorize each async delivery and refuse Required-transaction actions (LIVE-016, LIVE-017)
