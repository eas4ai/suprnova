# Feature map: `manual/render-cache-operations.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 21 checked.

## Command line

### Console binary (framework-registered commands)

- [ ] command `console render-cache:epoch-advance` · framework/src/render_cache/console.rs:98 (hidden from `help`; dispatchable)
  - Advances the RenderCache authority epoch, making every stored entry unreachable at its next freshness check (emergency invalidation)
- [ ] command `console render-cache:inspect` · framework/src/render_cache/console.rs:168 (hidden from `help`; dispatchable)
  - Body-free inspection of one RenderCache entry by its rk1. key
  - argument `<key>`: The entry's rk1. lookup key, as printed by application logging

## Rust API: suprnova

### `suprnova::render_cache::telemetry`

- [ ] fn `suprnova::render_cache::telemetry::await_hint_for_test` · framework/src/render_cache/telemetry.rs:328 (feature: `testing`)
- [ ] fn `suprnova::render_cache::telemetry::decline_reason_labels_for_test` · framework/src/render_cache/telemetry.rs:212 (feature: `testing`)
- [ ] fn `suprnova::render_cache::telemetry::recorded_hints_for_test` · framework/src/render_cache/telemetry.rs:309 (feature: `testing`)
- [ ] fn `suprnova::render_cache::telemetry::recorded_lookups_for_test` · framework/src/render_cache/telemetry.rs:195 (feature: `testing`)
- [ ] fn `suprnova::render_cache::telemetry::reset_recorded_hints_for_test` · framework/src/render_cache/telemetry.rs:316 (feature: `testing`)
- [ ] fn `suprnova::render_cache::telemetry::reset_recorded_lookups_for_test` · framework/src/render_cache/telemetry.rs:202 (feature: `testing`)
- [ ] struct `suprnova::render_cache::telemetry::RecordedLookup` · framework/src/render_cache/telemetry.rs:137 (feature: `testing`)
  - Public fields: `outcome`, `reason`
- [ ] const `suprnova::render_cache::telemetry::CAUSE` · framework/src/render_cache/telemetry.rs:48
- [ ] const `suprnova::render_cache::telemetry::EPOCH_REWINDS` · framework/src/render_cache/telemetry.rs:88
- [ ] const `suprnova::render_cache::telemetry::HINTS` · framework/src/render_cache/telemetry.rs:73
- [ ] const `suprnova::render_cache::telemetry::HITS` · framework/src/render_cache/telemetry.rs:18
- [ ] const `suprnova::render_cache::telemetry::LOOKUPS` · framework/src/render_cache/telemetry.rs:4
- [ ] const `suprnova::render_cache::telemetry::OUTCOME` · framework/src/render_cache/telemetry.rs:95
- [ ] const `suprnova::render_cache::telemetry::PUBLICATIONS` · framework/src/render_cache/telemetry.rs:20
- [ ] const `suprnova::render_cache::telemetry::REASON` · framework/src/render_cache/telemetry.rs:126
- [ ] const `suprnova::render_cache::telemetry::REBUILDS` · framework/src/render_cache/telemetry.rs:22
- [ ] const `suprnova::render_cache::telemetry::STITCH_ASSEMBLIES` · framework/src/render_cache/telemetry.rs:28
- [ ] const `suprnova::render_cache::telemetry::STITCH_NESTED` · framework/src/render_cache/telemetry.rs:45
- [ ] const `suprnova::render_cache::telemetry::STITCH_SLOTS` · framework/src/render_cache/telemetry.rs:36
