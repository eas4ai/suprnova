# The quarantine store the Live gate runs completes a write only when the bytes have reached the file

Level: Consequential
Decided by: developer
Promotes: the-upload-file-provider-s-retry-tests-fail-about-one-run-in-fifteen-which-turns-the-live-gate-red-at-random
Rests on: LIVE-010
Would be wrong if: the flake survives the flush, which would mean the provider, not the store, reorders its operations

## Decision

Chosen 2026-09-17 by the developer, in chat at 21:46 and 21:47 EDT: "fix the fucking problem" and "no I am not deferring a goddamn fucking bug", answering escalation live-010 with instead. The test-support quarantine store writes through a tokio file and never flushes it, so a write the provider has awaited can still be in the file's buffer, waiting on a blocking task that dropping the file neither awaits nor reports. A later read of the same object then sees an earlier attempt's bytes or too few bytes, which the provider reports as a checksum mismatch or an incomplete transfer. Measured on an idle machine: six failures in sixty whole-file runs without the flush, none in a hundred and twenty with it. The store now flushes before an operation completes. Rejected: serializing the tests, which would hide the store's contract instead of fixing it, and retrying the gate, which the developer refused.

## Realized by

- 24e0a408  cairn: escalate live-010, whether to promote the upload provider's flaky retry tests
- 12a7df56  live: the quarantine store the gate runs completes a write only once the bytes have reached the file (LIVE-038)
- 5dd1441c  cairn: live-010 answered instead; promote the store's write completion into upload-store-write-completion (LIVE-038), roadmap item 15 current, mechanism declared
