# Feature map: `manual/idempotency.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 8 checked.

## Rust API: suprnova

### `suprnova::idempotency`

- [ ] struct `suprnova::Idempotency` · framework/src/idempotency/mod.rs:111 (also `suprnova::idempotency::Idempotency`)
  - [ ] fn `suprnova::Idempotency::once` · framework/src/idempotency/mod.rs:136
  - [ ] fn `suprnova::Idempotency::commit_on_success` · framework/src/idempotency/mod.rs:177
  - [ ] fn `suprnova::Idempotency::commit_on_success_owned` · framework/src/idempotency/mod.rs:213
  - [ ] fn `suprnova::Idempotency::release_owned` · framework/src/idempotency/mod.rs:259
  - [ ] fn `suprnova::Idempotency::remember` · framework/src/idempotency/mod.rs:326
- [ ] enum `suprnova::Idempotent` · framework/src/idempotency/mod.rs:66 (also `suprnova::idempotency::Idempotent`)
  - Variants: `Fresh`, `FreshUnfenced`, `Duplicate`
- [ ] enum `suprnova::Replay` · framework/src/idempotency/mod.rs:91 (also `suprnova::idempotency::Replay`)
  - Variants: `Fresh`, `FreshUnfenced`, `Replayed`, `InProgress`
