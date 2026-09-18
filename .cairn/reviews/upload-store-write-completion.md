# Review - upload-store-write-completion

commitment: upload-store-write-completion
commit: 12a7df56
examined:
  - LIVE-038 against the built tree: `write_all_fragmented` in `crates/suprnova-live/crates/suprnova-live-test-support/src/file_quarantine_store.rs`, which now flushes the tokio file before the operation completes, and the store's read and sync paths, which open their own handles and therefore depend on that flush.
  - The provider's side of the contract: `wait_store` in `crates/suprnova-live/src/upload/provider.rs` awaits each operation in full, and each store call is a detached task, so the provider's ordering holds only if completion means the bytes landed.
  - The measurement: sixty whole-file runs of `crates/suprnova-live/tests/upload_file_provider.rs` with the flush removed failed six times, in three different tests, with a checksum mismatch or an incomplete transfer; a hundred and twenty runs with the flush passed.
  - The Live gate, which had been red on four consecutive runs, green on the first run after the fix.
findings:
  - resolved: The failures read as provider defects, a checksum mismatch and an incomplete transfer, but the provider was correct: the store told it a write was done while the bytes were still in a tokio file's buffer, waiting on a blocking task that dropping the file neither awaits nor reports. Two of the three failing tests write the same range twice, which is why an earlier attempt's bytes could reach the file after a later one's.
  - resolved: No shipped code writes through a tokio file this way; a search over `framework/src`, `crates/*/src` and `suprnova-cli/src` finds only reads, so the defect was confined to the store the tests and the reference host run against.

## Build review, 2026-09-17

### Attacked: contradictions

- "The provider reorders its own writes" against `wait_store`: it awaits
  each operation to completion and only cancellation returns early, so a
  reordering would have to come from the store. Removing the flush brings
  the failures back, which settles it.
- "The tests are simply parallel and racy" against the fix: the tests share
  no store and no directory. What they share is the machine, and the
  buffered write's blocking task is what the scheduling delay exposed.
- A flush on every fragment against the store's purpose: the fragment loop
  exists to prove the provider handles short writes, and the flush is
  after the loop, so the observed fragment counts the tests assert are
  unchanged.

### Falsifiers, each demonstrated

- LIVE-038: with the flush removed, six of sixty whole-file runs failed; the
  falsifier names sixty runs, and the mechanism runs exactly that many,
  labeling each.

### Limits recorded

- The three browser cases that were also red this evening (a firefox
  asynchronous-updates case, a chrome-bfcache case, a firefox activity-feed
  case) each passed alone and are not explained by this fix. The Live gate
  has been green once since; if they return, they are their own item.
