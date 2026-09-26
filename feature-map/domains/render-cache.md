# Feature map: `manual/render-cache.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 336 checked.

## Rust API: suprnova

### `suprnova::render_cache::config`

- [ ] struct `suprnova::render_cache::L0Limits` · framework/src/render_cache/config.rs:37 (also `suprnova::render_cache::config::L0Limits`)
  - Public fields: `max_entries`, `max_bytes`
- [ ] struct `suprnova::render_cache::RenderCacheConfig` · framework/src/render_cache/config.rs:264 (also `suprnova::render_cache::config::RenderCacheConfig`)
  - Public fields: `enabled`, `profile`, `l0`, `l1`, `coordinator`, `failure`, `hints`, `build_id`
  - [ ] fn `suprnova::render_cache::RenderCacheConfig::with_build_id` · framework/src/render_cache/config.rs:398
  - [ ] fn `suprnova::render_cache::RenderCacheConfig::from_env` · framework/src/render_cache/config.rs:458
  - [ ] fn `suprnova::render_cache::RenderCacheConfig::enabled_from_env` · framework/src/render_cache/config.rs:471
- [ ] enum `suprnova::render_cache::CoordinatorConfig` · framework/src/render_cache/config.rs:149 (also `suprnova::render_cache::config::CoordinatorConfig`)
  - Variants: `Local`, `Database`, `Redis`
- [ ] enum `suprnova::render_cache::HintsConfig` · framework/src/render_cache/config.rs:233 (also `suprnova::render_cache::config::HintsConfig`)
  - Variants: `Disabled`, `Redis`
- [ ] enum `suprnova::render_cache::L1Config` · framework/src/render_cache/config.rs:70 (also `suprnova::render_cache::config::L1Config`)
  - Variants: `Disabled`, `File`, `Database`, `Redis`
- [ ] enum `suprnova::render_cache::Profile` · framework/src/render_cache/config.rs:53 (also `suprnova::render_cache::config::Profile`)
  - Variants: `Embedded`, `Database`, `Redis`

### `suprnova::render_cache::hints`

- [ ] const `suprnova::render_cache::hints::MAX_HINT_DIGESTS` · framework/src/render_cache/hints.rs:83
- [ ] const `suprnova::render_cache::hints::MAX_INBOUND_HINTS` · framework/src/render_cache/hints.rs:127

### `suprnova::render_cache::live`

- [ ] fn `suprnova::render_cache::live::document_declines` · framework/src/render_cache/live.rs:211
- [ ] fn `suprnova::render_cache::live::record_bootstrap_nonce` · framework/src/render_cache/live.rs:135
- [ ] fn `suprnova::render_cache::live::record_document_digest` · framework/src/render_cache/live.rs:143
- [ ] fn `suprnova::render_cache::live::record_document_intent` · framework/src/render_cache/live.rs:163
- [ ] fn `suprnova::render_cache::live::record_mount` · framework/src/render_cache/live.rs:107
- [ ] fn `suprnova::render_cache::live::record_shell_island` · framework/src/render_cache/live.rs:123
- [ ] fn `suprnova::render_cache::live::record_stitch_capture_invalid` · framework/src/render_cache/live.rs:151
- [ ] fn `suprnova::render_cache::live::record_stitch_slot` · framework/src/render_cache/live.rs:116
- [ ] fn `suprnova::render_cache::live::seed_remaining_ms` · framework/src/render_cache/live.rs:234
- [ ] struct `suprnova::render_cache::live::CapturedSlot` · framework/src/render_cache/live.rs:47
  - Public fields: `descriptor`, `html`
- [ ] struct `suprnova::render_cache::live::LiveDocumentFacts` · framework/src/render_cache/live.rs:22
  - Public fields: `public_seed_islands`, `identity_bound_islands`, `seed_deadline_ms`, `no_store`, `stitch`
- [ ] struct `suprnova::render_cache::live::StitchCapture` · framework/src/render_cache/live.rs:84
  - Public fields: `slots`, `shell_islands`, `nonce`, `document_digest`, `invalid`
- [ ] enum `suprnova::render_cache::live::LiveDocumentDecline` · framework/src/render_cache/live.rs:174
  - Variants: `IdentityBoundWithoutStitching`, `InvalidStitchCapture`, `NoStoreIntent`, `UnresolvableSeedDeadline`

### `suprnova::render_cache::middleware`

- [ ] struct `suprnova::render_cache::RenderCacheMiddleware` · framework/src/render_cache/middleware.rs:285 (also `suprnova::render_cache::middleware::RenderCacheMiddleware`)
  - Implements: `suprnova::Middleware`
- [ ] struct `suprnova::render_cache::RenderCacheRuntime` · framework/src/render_cache/middleware.rs:369 (also `suprnova::render_cache::middleware::RenderCacheRuntime`)

### `suprnova::render_cache::registry`

- [ ] struct `suprnova::render_cache::registry::RenderCachePolicyTable` · framework/src/render_cache/registry.rs:33
  - [ ] fn `suprnova::render_cache::registry::RenderCachePolicyTable::register_group` · framework/src/render_cache/registry.rs:40
  - [ ] fn `suprnova::render_cache::registry::RenderCachePolicyTable::register_route` · framework/src/render_cache/registry.rs:64
  - [ ] fn `suprnova::render_cache::registry::RenderCachePolicyTable::effective_policy` · framework/src/render_cache/registry.rs:110
- [ ] enum `suprnova::render_cache::registry::GroupPolicy` · framework/src/render_cache/registry.rs:12
  - Variants: `Policy`, `Patch`

### `suprnova::render_cache`

- [ ] struct `suprnova::RenderCache` · framework/src/render_cache/mod.rs:331 (also `suprnova::render_cache::RenderCache`)
  - [ ] fn `suprnova::RenderCache::install` · framework/src/render_cache/mod.rs:490
  - [ ] fn `suprnova::RenderCache::bump_permission_version` · framework/src/render_cache/mod.rs:670
  - [ ] fn `suprnova::RenderCache::advance_epoch` · framework/src/render_cache/mod.rs:720
  - [ ] fn `suprnova::RenderCache::inspect` · framework/src/render_cache/mod.rs:738
  - [ ] fn `suprnova::RenderCache::store_inspection` · framework/src/render_cache/mod.rs:760
  - [ ] fn `suprnova::RenderCache::sweep` · framework/src/render_cache/mod.rs:793
- [ ] struct `suprnova::render_cache::StoreInspection` · framework/src/render_cache/mod.rs:174
  - Public fields: `entries`, `bytes`, `epoch`
- [ ] enum `suprnova::render_cache::L1Provider` · framework/src/render_cache/mod.rs:103
  - Variants: `File`, `Database`, `Redis`
  - Implements: `suprnova_live::render_cache::RenderStore`

## Rust API: suprnova-live

### `suprnova_live::render_cache::coherence`

- [ ] fn `suprnova_live::render_cache::age_seconds` · crates/suprnova-live/src/render_cache/coherence.rs:62 (also `suprnova_live::render_cache::coherence::age_seconds`)
- [ ] fn `suprnova_live::render_cache::evaluate_freshness` · crates/suprnova-live/src/render_cache/coherence.rs:26 (also `suprnova_live::render_cache::coherence::evaluate_freshness`)
- [ ] fn `suprnova_live::render_cache::warning_header` · crates/suprnova-live/src/render_cache/coherence.rs:68 (also `suprnova_live::render_cache::coherence::warning_header`)
- [ ] struct `suprnova_live::render_cache::ValidationLease` · crates/suprnova-live/src/render_cache/coherence.rs:79 (also `suprnova_live::render_cache::coherence::ValidationLease`)
  - [ ] fn `suprnova_live::render_cache::ValidationLease::grant` · crates/suprnova-live/src/render_cache/coherence.rs:87
  - [ ] fn `suprnova_live::render_cache::ValidationLease::valid_at` · crates/suprnova-live/src/render_cache/coherence.rs:97
  - [ ] fn `suprnova_live::render_cache::ValidationLease::hint_invalidate` · crates/suprnova-live/src/render_cache/coherence.rs:102
- [ ] enum `suprnova_live::render_cache::FreshnessState` · crates/suprnova-live/src/render_cache/coherence.rs:8 (also `suprnova_live::render_cache::coherence::FreshnessState`)
  - Variants: `Fresh`, `StaleServable`, `StaleOnError`, `Dead`

### `suprnova_live::render_cache::composite`

- [ ] fn `suprnova_live::render_cache::assemble` · crates/suprnova-live/src/render_cache/composite.rs:868 (also `suprnova_live::render_cache::composite::assemble`)
- [ ] fn `suprnova_live::render_cache::composite::assemble_nested` · crates/suprnova-live/src/render_cache/composite.rs:896
- [ ] fn `suprnova_live::render_cache::composite::descend_nested` · crates/suprnova-live/src/render_cache/composite.rs:733
- [ ] fn `suprnova_live::render_cache::fresh_nonce` · crates/suprnova-live/src/render_cache/composite.rs:623 (also `suprnova_live::render_cache::composite::fresh_nonce`)
- [ ] fn `suprnova_live::render_cache::surrounding_digest` · crates/suprnova-live/src/render_cache/composite.rs:429 (also `suprnova_live::render_cache::composite::surrounding_digest`)
- [ ] fn `suprnova_live::render_cache::valid_nonce` · crates/suprnova-live/src/render_cache/composite.rs:614 (also `suprnova_live::render_cache::composite::valid_nonce`)
- [ ] fn `suprnova_live::render_cache::composite::verify_nested` · crates/suprnova-live/src/render_cache/composite.rs:747
- [ ] struct `suprnova_live::render_cache::AssembledDocument` · crates/suprnova-live/src/render_cache/composite.rs:672 (also `suprnova_live::render_cache::composite::AssembledDocument`)
  - [ ] fn `suprnova_live::render_cache::AssembledDocument::body` · crates/suprnova-live/src/render_cache/composite.rs:681
  - [ ] fn `suprnova_live::render_cache::AssembledDocument::validator` · crates/suprnova-live/src/render_cache/composite.rs:687
  - [ ] fn `suprnova_live::render_cache::AssembledDocument::headers` · crates/suprnova-live/src/render_cache/composite.rs:693
- [ ] struct `suprnova_live::render_cache::AssemblyInput` · crates/suprnova-live/src/render_cache/composite.rs:663 (also `suprnova_live::render_cache::composite::AssemblyInput`)
  - Public fields: `outcomes`, `nonce`
- [ ] struct `suprnova_live::render_cache::CheckedIsland` · crates/suprnova-live/src/render_cache/composite.rs:632 (also `suprnova_live::render_cache::composite::CheckedIsland`)
  - [ ] fn `suprnova_live::render_cache::CheckedIsland::new` · crates/suprnova-live/src/render_cache/composite.rs:641
- [ ] struct `suprnova_live::render_cache::CompositeEntry` · crates/suprnova-live/src/render_cache/composite.rs:513 (also `suprnova_live::render_cache::composite::CompositeEntry`)
  - [ ] fn `suprnova_live::render_cache::CompositeEntry::new` · crates/suprnova-live/src/render_cache/composite.rs:540
  - [ ] fn `suprnova_live::render_cache::CompositeEntry::header` · crates/suprnova-live/src/render_cache/composite.rs:571
  - [ ] fn `suprnova_live::render_cache::CompositeEntry::graph` · crates/suprnova-live/src/render_cache/composite.rs:577
  - [ ] fn `suprnova_live::render_cache::CompositeEntry::shell` · crates/suprnova-live/src/render_cache/composite.rs:583
  - [ ] fn `suprnova_live::render_cache::CompositeEntry::structural_digest` · crates/suprnova-live/src/render_cache/composite.rs:589
  - [ ] fn `suprnova_live::render_cache::CompositeEntry::needs_nonce` · crates/suprnova-live/src/render_cache/composite.rs:595
  - [ ] fn `suprnova_live::render_cache::CompositeEntry::canonical_header_bytes` · crates/suprnova-live/src/render_cache/composite.rs:600
- [ ] struct `suprnova_live::render_cache::CompositeHeader` · crates/suprnova-live/src/render_cache/composite.rs:492 (also `suprnova_live::render_cache::composite::CompositeHeader`)
  - Public fields: `entry`, `graph`
  - [ ] fn `suprnova_live::render_cache::CompositeHeader::canonical_bytes` · crates/suprnova-live/src/render_cache/composite.rs:502
- [ ] struct `suprnova_live::render_cache::HeaderTemplate` · crates/suprnova-live/src/render_cache/composite.rs:290 (also `suprnova_live::render_cache::composite::HeaderTemplate`)
  - Public fields: `name`, `pieces`
- [ ] struct `suprnova_live::render_cache::ParsedSlot` · crates/suprnova-live/src/render_cache/composite.rs:197 (also `suprnova_live::render_cache::composite::ParsedSlot`)
  - Public fields: `route`, `slot`, `document_key`, `component`, `contract_digest`, `protocol`, `build`, `parameters`, `flags`, `on_failure`
- [ ] struct `suprnova_live::render_cache::SegmentGraph` · crates/suprnova-live/src/render_cache/composite.rs:299 (also `suprnova_live::render_cache::composite::SegmentGraph`)
  - Public fields: `segments`, `slots`, `shell_islands`, `nonce_headers`
  - [ ] fn `suprnova_live::render_cache::SegmentGraph::needs_nonce` · crates/suprnova-live/src/render_cache/composite.rs:313
  - [ ] fn `suprnova_live::render_cache::SegmentGraph::validate` · crates/suprnova-live/src/render_cache/composite.rs:321
- [ ] struct `suprnova_live::render_cache::ShellIsland` · crates/suprnova-live/src/render_cache/composite.rs:268 (also `suprnova_live::render_cache::composite::ShellIsland`)
  - Public fields: `slot`, `document_key`
- [ ] struct `suprnova_live::render_cache::StitchSlot` · crates/suprnova-live/src/render_cache/composite.rs:170 (also `suprnova_live::render_cache::composite::StitchSlot`)
  - Public fields: `route`, `slot`, `document_key`, `component`, `contract_digest`, `protocol`, `build`, `parameters`, `flags`, `on_failure`, `surrounding`
  - [ ] fn `suprnova_live::render_cache::StitchSlot::parse` · crates/suprnova-live/src/render_cache/composite.rs:222
- [ ] enum `suprnova_live::render_cache::HeaderPiece` · crates/suprnova-live/src/render_cache/composite.rs:278 (also `suprnova_live::render_cache::composite::HeaderPiece`)
  - Variants: `Text`, `Nonce`
- [ ] enum `suprnova_live::render_cache::composite::NestedFailureCause` · crates/suprnova-live/src/render_cache/composite.rs:709
  - Variants: `Cycle`, `DepthExceeded`, `VersionMismatch`, `LengthMismatch`
- [ ] enum `suprnova_live::render_cache::composite::NestedOutcome` · crates/suprnova-live/src/render_cache/composite.rs:767
  - Variants: `Resolved`, `Fallback`, `Omitted`
- [ ] enum `suprnova_live::render_cache::Segment` · crates/suprnova-live/src/render_cache/composite.rs:82 (also `suprnova_live::render_cache::composite::Segment`)
  - Variants: `Literal`, `Slot`, `Nonce`, `Nested`
  - [ ] fn `suprnova_live::render_cache::Segment::literal_len` · crates/suprnova-live/src/render_cache/composite.rs:121
- [ ] enum `suprnova_live::render_cache::SlotFailurePolicy` · crates/suprnova-live/src/render_cache/composite.rs:136 (also `suprnova_live::render_cache::composite::SlotFailurePolicy`)
  - Variants: `FailDocument`, `Omit`, `Fallback`
- [ ] enum `suprnova_live::render_cache::SlotOutcome` · crates/suprnova-live/src/render_cache/composite.rs:652 (also `suprnova_live::render_cache::composite::SlotOutcome`)
  - Variants: `Rendered`, `Fallback`, `Omitted`
- [ ] const `suprnova_live::render_cache::composite::MAX_FALLBACK_BYTES` · crates/suprnova-live/src/render_cache/composite.rs:61
- [ ] const `suprnova_live::render_cache::composite::MAX_NESTED_SEGMENTS` · crates/suprnova-live/src/render_cache/composite.rs:57
- [ ] const `suprnova_live::render_cache::composite::MAX_NESTING_DEPTH` · crates/suprnova-live/src/render_cache/composite.rs:52
- [ ] const `suprnova_live::render_cache::composite::MAX_NONCE_BYTES` · crates/suprnova-live/src/render_cache/composite.rs:69
- [ ] const `suprnova_live::render_cache::composite::MAX_NONCE_HEADERS` · crates/suprnova-live/src/render_cache/composite.rs:65
- [ ] const `suprnova_live::render_cache::composite::MAX_NONCE_HOLES` · crates/suprnova-live/src/render_cache/composite.rs:36
- [ ] const `suprnova_live::render_cache::composite::MAX_SEGMENTS` · crates/suprnova-live/src/render_cache/composite.rs:43
- [ ] const `suprnova_live::render_cache::composite::MAX_SHELL_ISLANDS` · crates/suprnova-live/src/render_cache/composite.rs:63
- [ ] const `suprnova_live::render_cache::composite::MAX_SLOT_PARAMETER_BYTES` · crates/suprnova-live/src/render_cache/composite.rs:59
- [ ] const `suprnova_live::render_cache::MAX_STITCH_SLOTS` · crates/suprnova-live/src/render_cache/composite.rs:34 (also `suprnova_live::render_cache::composite::MAX_STITCH_SLOTS`)
- [ ] const `suprnova_live::render_cache::composite::SURROUNDING_WINDOW_BYTES` · crates/suprnova-live/src/render_cache/composite.rs:67

### `suprnova_live::render_cache::entry`

- [ ] fn `suprnova_live::render_cache::decode` · crates/suprnova-live/src/render_cache/entry.rs:479 (also `suprnova_live::render_cache::entry::decode`)
- [ ] fn `suprnova_live::render_cache::encode` · crates/suprnova-live/src/render_cache/entry.rs:375 (also `suprnova_live::render_cache::entry::encode`)
- [ ] fn `suprnova_live::render_cache::encode_composite` · crates/suprnova-live/src/render_cache/entry.rs:388 (also `suprnova_live::render_cache::entry::encode_composite`)
- [ ] fn `suprnova_live::render_cache::inspect` · crates/suprnova-live/src/render_cache/entry.rs:579 (also `suprnova_live::render_cache::entry::inspect`)
- [ ] struct `suprnova_live::render_cache::CompleteEntry` · crates/suprnova-live/src/render_cache/entry.rs:207 (also `suprnova_live::render_cache::entry::CompleteEntry`)
  - [ ] fn `suprnova_live::render_cache::CompleteEntry::new` · crates/suprnova-live/src/render_cache/entry.rs:216
  - [ ] fn `suprnova_live::render_cache::CompleteEntry::header` · crates/suprnova-live/src/render_cache/entry.rs:227
  - [ ] fn `suprnova_live::render_cache::CompleteEntry::body` · crates/suprnova-live/src/render_cache/entry.rs:233
  - [ ] fn `suprnova_live::render_cache::CompleteEntry::validator` · crates/suprnova-live/src/render_cache/entry.rs:239
- [ ] struct `suprnova_live::render_cache::EntryHeader` · crates/suprnova-live/src/render_cache/entry.rs:175 (also `suprnova_live::render_cache::entry::EntryHeader`)
  - Public fields: `key`, `class`, `variance`, `published_at_ms`, `fresh_ms`, `stale_servable_ms`, `stale_on_error_ms`, `observed`, `epoch`, `seed_deadline_ms`, `status`, `headers`, `content_encoding`
- [ ] struct `suprnova_live::render_cache::EntryInspection` · crates/suprnova-live/src/render_cache/entry.rs:284 (also `suprnova_live::render_cache::entry::EntryInspection`) (re-exported as `suprnova::render_cache::EntryInspection`)
  - Public fields: `kind`, `class`, `body_bytes`, `status`, `published_at_ms`, `epoch`, `observations`, `slots`
- [ ] struct `suprnova_live::render_cache::EntryLimits` · crates/suprnova-live/src/render_cache/entry.rs:31 (also `suprnova_live::render_cache::entry::EntryLimits`)
  - Public fields: `max_body_bytes`, `max_header_bytes`, `max_headers`, `max_observations`
- [ ] struct `suprnova_live::render_cache::SafeHeaders` · crates/suprnova-live/src/render_cache/entry.rs:65 (also `suprnova_live::render_cache::entry::SafeHeaders`)
  - [ ] fn `suprnova_live::render_cache::SafeHeaders::from_pairs` · crates/suprnova-live/src/render_cache/entry.rs:115
  - [ ] fn `suprnova_live::render_cache::SafeHeaders::iter` · crates/suprnova-live/src/render_cache/entry.rs:139
- [ ] enum `suprnova_live::render_cache::DecodedEntry` · crates/suprnova-live/src/render_cache/entry.rs:246 (also `suprnova_live::render_cache::entry::DecodedEntry`)
  - Variants: `Complete`, `Composite`
  - [ ] fn `suprnova_live::render_cache::DecodedEntry::header` · crates/suprnova-live/src/render_cache/entry.rs:256
  - [ ] fn `suprnova_live::render_cache::DecodedEntry::kind` · crates/suprnova-live/src/render_cache/entry.rs:265
  - [ ] fn `suprnova_live::render_cache::DecodedEntry::into_complete` · crates/suprnova-live/src/render_cache/entry.rs:274
- [ ] enum `suprnova_live::render_cache::EntryKind` · crates/suprnova-live/src/render_cache/entry.rs:55 (also `suprnova_live::render_cache::entry::EntryKind`) (re-exported as `suprnova::render_cache::EntryKind`)
  - Variants: `Complete`, `Composite`
- [ ] enum `suprnova_live::render_cache::Validator` · crates/suprnova-live/src/render_cache/entry.rs:146 (also `suprnova_live::render_cache::entry::Validator`)
  - Variants: `Strong`
  - [ ] fn `suprnova_live::render_cache::Validator::strong_for` · crates/suprnova-live/src/render_cache/entry.rs:154
  - [ ] fn `suprnova_live::render_cache::Validator::digest_base64url` · crates/suprnova-live/src/render_cache/entry.rs:160
  - [ ] fn `suprnova_live::render_cache::Validator::etag` · crates/suprnova-live/src/render_cache/entry.rs:168
- [ ] const `suprnova_live::render_cache::entry::ENTRY_FORMAT_VERSION` · crates/suprnova-live/src/render_cache/entry.rs:26
- [ ] const `suprnova_live::render_cache::entry::REPLAYABLE_HEADERS` · crates/suprnova-live/src/render_cache/entry.rs:74

### `suprnova_live::render_cache::generation`

- [ ] struct `suprnova_live::render_cache::GenerationSet` · crates/suprnova-live/src/render_cache/generation.rs:236 (also `suprnova_live::render_cache::generation::GenerationSet`)
  - [ ] fn `suprnova_live::render_cache::GenerationSet::insert_digest` · crates/suprnova-live/src/render_cache/generation.rs:243
  - [ ] fn `suprnova_live::render_cache::GenerationSet::insert` · crates/suprnova-live/src/render_cache/generation.rs:257
  - [ ] fn `suprnova_live::render_cache::GenerationSet::get` · crates/suprnova-live/src/render_cache/generation.rs:267
  - [ ] fn `suprnova_live::render_cache::GenerationSet::get_digest` · crates/suprnova-live/src/render_cache/generation.rs:273
  - [ ] fn `suprnova_live::render_cache::GenerationSet::len` · crates/suprnova-live/src/render_cache/generation.rs:279
  - [ ] fn `suprnova_live::render_cache::GenerationSet::is_empty` · crates/suprnova-live/src/render_cache/generation.rs:285
  - [ ] fn `suprnova_live::render_cache::GenerationSet::digests` · crates/suprnova-live/src/render_cache/generation.rs:291
  - [ ] fn `suprnova_live::render_cache::GenerationSet::digest` · crates/suprnova-live/src/render_cache/generation.rs:298
- [ ] struct `suprnova_live::render_cache::MemoryGenerationLedger` · crates/suprnova-live/src/render_cache/generation.rs:417 (also `suprnova_live::render_cache::generation::MemoryGenerationLedger`)
  - Implements: `suprnova_live::render_cache::GenerationLedger`
  - [ ] fn `suprnova_live::render_cache::MemoryGenerationLedger::new` · crates/suprnova-live/src/render_cache/generation.rs:424
  - [ ] fn `suprnova_live::render_cache::MemoryGenerationLedger::advance_epoch` · crates/suprnova-live/src/render_cache/generation.rs:434
  - [ ] fn `suprnova_live::render_cache::MemoryGenerationLedger::rewind_epoch_for_test` · crates/suprnova-live/src/render_cache/generation.rs:443
- [ ] struct `suprnova_live::render_cache::ObservationWindow` · crates/suprnova-live/src/render_cache/generation.rs:497 (also `suprnova_live::render_cache::generation::ObservationWindow`)
  - [ ] fn `suprnova_live::render_cache::ObservationWindow::open` · crates/suprnova-live/src/render_cache/generation.rs:511
  - [ ] fn `suprnova_live::render_cache::ObservationWindow::observe` · crates/suprnova-live/src/render_cache/generation.rs:521
  - [ ] fn `suprnova_live::render_cache::ObservationWindow::epoch` · crates/suprnova-live/src/render_cache/generation.rs:531
  - [ ] fn `suprnova_live::render_cache::ObservationWindow::close` · crates/suprnova-live/src/render_cache/generation.rs:536
- [ ] enum `suprnova_live::render_cache::CoherenceCheck` · crates/suprnova-live/src/render_cache/generation.rs:551 (also `suprnova_live::render_cache::generation::CoherenceCheck`)
  - Variants: `Coherent`, `Moved`, `Rewound`
  - [ ] fn `suprnova_live::render_cache::CoherenceCheck::compare` · crates/suprnova-live/src/render_cache/generation.rs:574
- [ ] enum `suprnova_live::render_cache::DependencyIdentity` · crates/suprnova-live/src/render_cache/generation.rs:58 (also `suprnova_live::render_cache::generation::DependencyIdentity`) (re-exported as `suprnova::render_cache::DependencyIdentity`)
  - Variants: `Table`, `Record`, `QueryClass`, `Relation`, `Config`, `Feature`, `UnkeyedWrite`, `Locale`, `Route`, `Broad`
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::table` · crates/suprnova-live/src/render_cache/generation.rs:116
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::try_table` · crates/suprnova-live/src/render_cache/generation.rs:121
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::record` · crates/suprnova-live/src/render_cache/generation.rs:129
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::try_record` · crates/suprnova-live/src/render_cache/generation.rs:134
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::query_class` · crates/suprnova-live/src/render_cache/generation.rs:148
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::try_query_class` · crates/suprnova-live/src/render_cache/generation.rs:153
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::config` · crates/suprnova-live/src/render_cache/generation.rs:165
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::try_config` · crates/suprnova-live/src/render_cache/generation.rs:170
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::feature` · crates/suprnova-live/src/render_cache/generation.rs:178
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::try_feature` · crates/suprnova-live/src/render_cache/generation.rs:183
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::unkeyed_write` · crates/suprnova-live/src/render_cache/generation.rs:191
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::try_unkeyed_write` · crates/suprnova-live/src/render_cache/generation.rs:196
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::broad` · crates/suprnova-live/src/render_cache/generation.rs:203
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::digest` · crates/suprnova-live/src/render_cache/generation.rs:209
- [ ] trait `suprnova_live::render_cache::GenerationLedger` · crates/suprnova-live/src/render_cache/generation.rs:374 (also `suprnova_live::render_cache::generation::GenerationLedger`)
  - Implemented here by: `render_cache::MemoryGenerationLedger`
  - [ ] fn `suprnova_live::render_cache::GenerationLedger::current` · crates/suprnova-live/src/render_cache/generation.rs:378 (required)
  - [ ] fn `suprnova_live::render_cache::GenerationLedger::advance` · crates/suprnova-live/src/render_cache/generation.rs:381 (required)
  - [ ] fn `suprnova_live::render_cache::GenerationLedger::epoch` · crates/suprnova-live/src/render_cache/generation.rs:383 (required)
  - [ ] fn `suprnova_live::render_cache::GenerationLedger::lift_epoch_above` · crates/suprnova-live/src/render_cache/generation.rs:395 (required)
  - [ ] fn `suprnova_live::render_cache::GenerationLedger::current_with_epoch` · crates/suprnova-live/src/render_cache/generation.rs:400 (provided)
- [ ] type `suprnova_live::render_cache::Generation` · crates/suprnova-live/src/render_cache/generation.rs:45 (also `suprnova_live::render_cache::generation::Generation`)
- [ ] const `suprnova_live::render_cache::IDENTITY_VERSION` · crates/suprnova-live/src/render_cache/generation.rs:38 (also `suprnova_live::render_cache::generation::IDENTITY_VERSION`)
- [ ] const `suprnova_live::render_cache::MAX_OBSERVATIONS` · crates/suprnova-live/src/render_cache/generation.rs:35 (also `suprnova_live::render_cache::generation::MAX_OBSERVATIONS`)

### `suprnova_live::render_cache::hot`

- [ ] fn `suprnova_live::render_cache::respond` · crates/suprnova-live/src/render_cache/hot.rs:512 (also `suprnova_live::render_cache::hot::respond`)
- [ ] fn `suprnova_live::render_cache::serve_hot` · crates/suprnova-live/src/render_cache/hot.rs:448 (also `suprnova_live::render_cache::hot::serve_hot`)
- [ ] struct `suprnova_live::render_cache::HotEntry` · crates/suprnova-live/src/render_cache/hot.rs:324 (also `suprnova_live::render_cache::hot::HotEntry`)
  - [ ] fn `suprnova_live::render_cache::HotEntry::prepare` · crates/suprnova-live/src/render_cache/hot.rs:355
  - [ ] fn `suprnova_live::render_cache::HotEntry::entry` · crates/suprnova-live/src/render_cache/hot.rs:395
  - [ ] fn `suprnova_live::render_cache::HotEntry::fence` · crates/suprnova-live/src/render_cache/hot.rs:401
  - [ ] fn `suprnova_live::render_cache::HotEntry::published_at_ms` · crates/suprnova-live/src/render_cache/hot.rs:407
- [ ] struct `suprnova_live::render_cache::HotRequest` · crates/suprnova-live/src/render_cache/hot.rs:428 (also `suprnova_live::render_cache::hot::HotRequest`)
  - Public fields: `method`, `if_none_match`, `now_ms`
- [ ] struct `suprnova_live::render_cache::ResponseParts` · crates/suprnova-live/src/render_cache/hot.rs:457 (also `suprnova_live::render_cache::hot::ResponseParts`)
  - Public fields: `status`, `class`, `shared`, `freshness`, `headers`, `variance`, `validator`, `body`, `published_at_ms`, `seed_deadline_ms`, `cache_control_override`, `content_encoding`

### `suprnova_live::render_cache::http`

- [ ] fn `suprnova_live::render_cache::cache_control_value` · crates/suprnova-live/src/render_cache/http.rs:90 (also `suprnova_live::render_cache::http::cache_control_value`)
- [ ] fn `suprnova_live::render_cache::conditional_matches` · crates/suprnova-live/src/render_cache/http.rs:20 (also `suprnova_live::render_cache::http::conditional_matches`)
- [ ] fn `suprnova_live::render_cache::evaluate_conditional` · crates/suprnova-live/src/render_cache/http.rs:41 (also `suprnova_live::render_cache::http::evaluate_conditional`)
- [ ] fn `suprnova_live::render_cache::vary_value` · crates/suprnova-live/src/render_cache/http.rs:104 (also `suprnova_live::render_cache::http::vary_value`)
- [ ] enum `suprnova_live::render_cache::ConditionalOutcome` · crates/suprnova-live/src/render_cache/http.rs:9 (also `suprnova_live::render_cache::http::ConditionalOutcome`)
  - Variants: `NotModified`, `Full`

### `suprnova_live::render_cache::key`

- [ ] struct `suprnova_live::render_cache::RenderKey` · crates/suprnova-live/src/render_cache/key.rs:65 (also `suprnova_live::render_cache::key::RenderKey`)
  - [ ] fn `suprnova_live::render_cache::RenderKey::derive` · crates/suprnova-live/src/render_cache/key.rs:71
  - [ ] fn `suprnova_live::render_cache::RenderKey::to_base64url` · crates/suprnova-live/src/render_cache/key.rs:129
  - [ ] fn `suprnova_live::render_cache::RenderKey::from_base64url` · crates/suprnova-live/src/render_cache/key.rs:143
  - [ ] fn `suprnova_live::render_cache::RenderKey::digest` · crates/suprnova-live/src/render_cache/key.rs:155
- [ ] struct `suprnova_live::render_cache::RenderKeyDimensions` · crates/suprnova-live/src/render_cache/key.rs:202 (also `suprnova_live::render_cache::key::RenderKeyDimensions`)
  - [ ] fn `suprnova_live::render_cache::RenderKeyDimensions::describe` · crates/suprnova-live/src/render_cache/key.rs:219
  - [ ] fn `suprnova_live::render_cache::RenderKeyDimensions::opaque` · crates/suprnova-live/src/render_cache/key.rs:250
  - [ ] fn `suprnova_live::render_cache::RenderKeyDimensions::route` · crates/suprnova-live/src/render_cache/key.rs:266
  - [ ] fn `suprnova_live::render_cache::RenderKeyDimensions::params` · crates/suprnova-live/src/render_cache/key.rs:271
  - [ ] fn `suprnova_live::render_cache::RenderKeyDimensions::query` · crates/suprnova-live/src/render_cache/key.rs:276
  - [ ] fn `suprnova_live::render_cache::RenderKeyDimensions::variance` · crates/suprnova-live/src/render_cache/key.rs:281
- [ ] struct `suprnova_live::render_cache::RenderKeyInput` · crates/suprnova-live/src/render_cache/key.rs:27 (also `suprnova_live::render_cache::key::RenderKeyInput`)
  - Public fields: `route`, `route_pattern`, `params`, `query`, `host`, `media`, `encoding`, `build`, `epoch`, `variance`
- [ ] const `suprnova_live::render_cache::key::KEY_FORMAT_VERSION` · crates/suprnova-live/src/render_cache/key.rs:15
- [ ] const `suprnova_live::render_cache::key::MAX_PARAM_BYTES` · crates/suprnova-live/src/render_cache/key.rs:19
- [ ] const `suprnova_live::render_cache::key::MAX_PARAMS` · crates/suprnova-live/src/render_cache/key.rs:17

### `suprnova_live::render_cache::lease`

- [ ] struct `suprnova_live::render_cache::FencedLeaseCoordinator` · crates/suprnova-live/src/render_cache/lease.rs:218 (also `suprnova_live::render_cache::lease::FencedLeaseCoordinator`)
  - Implements: `suprnova_live::render_cache::RebuildCoordinator`
  - [ ] fn `suprnova_live::render_cache::FencedLeaseCoordinator::new` · crates/suprnova-live/src/render_cache/lease.rs:227
- [ ] struct `suprnova_live::render_cache::MemoryLeaseStore` · crates/suprnova-live/src/render_cache/lease.rs:108 (also `suprnova_live::render_cache::lease::MemoryLeaseStore`)
  - Implements: `suprnova_live::render_cache::LeaseStore`
  - [ ] fn `suprnova_live::render_cache::MemoryLeaseStore::new` · crates/suprnova-live/src/render_cache/lease.rs:117
  - [ ] fn `suprnova_live::render_cache::MemoryLeaseStore::epoch` · crates/suprnova-live/src/render_cache/lease.rs:143
- [ ] enum `suprnova_live::render_cache::LeaseAttempt` · crates/suprnova-live/src/render_cache/lease.rs:42 (also `suprnova_live::render_cache::lease::LeaseAttempt`)
  - Variants: `Acquired`, `Held`
- [ ] trait `suprnova_live::render_cache::LeaseStore` · crates/suprnova-live/src/render_cache/lease.rs:60 (also `suprnova_live::render_cache::lease::LeaseStore`)
  - Implemented here by: `render_cache::MemoryLeaseStore`
  - [ ] fn `suprnova_live::render_cache::LeaseStore::try_acquire` · crates/suprnova-live/src/render_cache/lease.rs:65 (required)
  - [ ] fn `suprnova_live::render_cache::LeaseStore::mint_token` · crates/suprnova-live/src/render_cache/lease.rs:75 (required)
  - [ ] fn `suprnova_live::render_cache::LeaseStore::release` · crates/suprnova-live/src/render_cache/lease.rs:82 (required)

### `suprnova_live::render_cache::policy`

- [ ] struct `suprnova_live::render_cache::FreshnessPolicy` · crates/suprnova-live/src/render_cache/policy.rs:44 (also `suprnova_live::render_cache::policy::FreshnessPolicy`) (re-exported as `suprnova::render_cache::FreshnessPolicy`)
  - [ ] fn `suprnova_live::render_cache::FreshnessPolicy::new` · crates/suprnova-live/src/render_cache/policy.rs:52
  - [ ] fn `suprnova_live::render_cache::FreshnessPolicy::fresh_ms` · crates/suprnova-live/src/render_cache/policy.rs:72
  - [ ] fn `suprnova_live::render_cache::FreshnessPolicy::stale_servable_ms` · crates/suprnova-live/src/render_cache/policy.rs:78
  - [ ] fn `suprnova_live::render_cache::FreshnessPolicy::stale_on_error_ms` · crates/suprnova-live/src/render_cache/policy.rs:84
  - [ ] fn `suprnova_live::render_cache::FreshnessPolicy::dead_after_ms` · crates/suprnova-live/src/render_cache/policy.rs:109
- [ ] struct `suprnova_live::render_cache::NegotiatedPolicy` · crates/suprnova-live/src/render_cache/policy.rs:267 (also `suprnova_live::render_cache::policy::NegotiatedPolicy`) (re-exported as `suprnova::render_cache::NegotiatedPolicy`)
  - [ ] fn `suprnova_live::render_cache::NegotiatedPolicy::declared` · crates/suprnova-live/src/render_cache/policy.rs:278
  - [ ] fn `suprnova_live::render_cache::NegotiatedPolicy::accepted` · crates/suprnova-live/src/render_cache/policy.rs:306
  - [ ] fn `suprnova_live::render_cache::NegotiatedPolicy::default_value` · crates/suprnova-live/src/render_cache/policy.rs:313
  - [ ] fn `suprnova_live::render_cache::NegotiatedPolicy::negotiate` · crates/suprnova-live/src/render_cache/policy.rs:322
- [ ] struct `suprnova_live::render_cache::PolicyPatch` · crates/suprnova-live/src/render_cache/policy.rs:745 (also `suprnova_live::render_cache::policy::PolicyPatch`) (re-exported as `suprnova::render_cache::PolicyPatch`)
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::class` · crates/suprnova-live/src/render_cache/policy.rs:761
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::freshness` · crates/suprnova-live/src/render_cache/policy.rs:768
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::layers` · crates/suprnova-live/src/render_cache/policy.rs:775
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::coherence` · crates/suprnova-live/src/render_cache/policy.rs:782
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::shared` · crates/suprnova-live/src/render_cache/policy.rs:789
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::failure` · crates/suprnova-live/src/render_cache/policy.rs:796
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::query` · crates/suprnova-live/src/render_cache/policy.rs:803
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::vary` · crates/suprnova-live/src/render_cache/policy.rs:814
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::media` · crates/suprnova-live/src/render_cache/policy.rs:823
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::encoding` · crates/suprnova-live/src/render_cache/policy.rs:832
- [ ] struct `suprnova_live::render_cache::QueryPolicy` · crates/suprnova-live/src/render_cache/policy.rs:201 (also `suprnova_live::render_cache::policy::QueryPolicy`) (re-exported as `suprnova::render_cache::QueryPolicy`)
  - [ ] fn `suprnova_live::render_cache::QueryPolicy::declared` · crates/suprnova-live/src/render_cache/policy.rs:208
  - [ ] fn `suprnova_live::render_cache::QueryPolicy::none` · crates/suprnova-live/src/render_cache/policy.rs:221
  - [ ] fn `suprnova_live::render_cache::QueryPolicy::declared_names` · crates/suprnova-live/src/render_cache/policy.rs:227
  - [ ] fn `suprnova_live::render_cache::QueryPolicy::unknown` · crates/suprnova-live/src/render_cache/policy.rs:233
- [ ] struct `suprnova_live::render_cache::RenderCachePolicy` · crates/suprnova-live/src/render_cache/policy.rs:369 (also `suprnova_live::render_cache::policy::RenderCachePolicy`) (re-exported as `suprnova::render_cache::RenderCachePolicy`)
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::builder` · crates/suprnova-live/src/render_cache/policy.rs:387
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::class` · crates/suprnova-live/src/render_cache/policy.rs:410
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::freshness` · crates/suprnova-live/src/render_cache/policy.rs:416
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::layers` · crates/suprnova-live/src/render_cache/policy.rs:422
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::coherence` · crates/suprnova-live/src/render_cache/policy.rs:428
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::shared` · crates/suprnova-live/src/render_cache/policy.rs:434
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::failure` · crates/suprnova-live/src/render_cache/policy.rs:440
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::query` · crates/suprnova-live/src/render_cache/policy.rs:446
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::vary` · crates/suprnova-live/src/render_cache/policy.rs:452
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::media` · crates/suprnova-live/src/render_cache/policy.rs:459
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::encoding` · crates/suprnova-live/src/render_cache/policy.rs:467
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::apply` · crates/suprnova-live/src/render_cache/policy.rs:473
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::eligibility` · crates/suprnova-live/src/render_cache/policy.rs:594
- [ ] struct `suprnova_live::render_cache::RenderCachePolicyBuilder` · crates/suprnova-live/src/render_cache/policy.rs:653 (also `suprnova_live::render_cache::policy::RenderCachePolicyBuilder`) (re-exported as `suprnova::render_cache::RenderCachePolicyBuilder`)
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::freshness` · crates/suprnova-live/src/render_cache/policy.rs:660
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::layers` · crates/suprnova-live/src/render_cache/policy.rs:667
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::coherence` · crates/suprnova-live/src/render_cache/policy.rs:674
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::shared` · crates/suprnova-live/src/render_cache/policy.rs:681
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::failure` · crates/suprnova-live/src/render_cache/policy.rs:688
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::query` · crates/suprnova-live/src/render_cache/policy.rs:695
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::vary` · crates/suprnova-live/src/render_cache/policy.rs:708
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::vary_media` · crates/suprnova-live/src/render_cache/policy.rs:718
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::vary_encoding` · crates/suprnova-live/src/render_cache/policy.rs:730
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::build` · crates/suprnova-live/src/render_cache/policy.rs:737
- [ ] struct `suprnova_live::render_cache::ResponseSignals` · crates/suprnova-live/src/render_cache/policy.rs:840 (also `suprnova_live::render_cache::policy::ResponseSignals`)
  - Public fields: `method`, `status`, `streaming`, `sets_cookie`, `content_type`, `cache_control`, `header_names`, `private_observed`
- [ ] struct `suprnova_live::render_cache::StorageLayers` · crates/suprnova-live/src/render_cache/policy.rs:124 (also `suprnova_live::render_cache::policy::StorageLayers`) (re-exported as `suprnova::render_cache::StorageLayers`)
  - [ ] fn `suprnova_live::render_cache::StorageLayers::l0_only` · crates/suprnova-live/src/render_cache/policy.rs:132
  - [ ] fn `suprnova_live::render_cache::StorageLayers::l0_and_l1` · crates/suprnova-live/src/render_cache/policy.rs:141
  - [ ] fn `suprnova_live::render_cache::StorageLayers::l0` · crates/suprnova-live/src/render_cache/policy.rs:147
  - [ ] fn `suprnova_live::render_cache::StorageLayers::l1` · crates/suprnova-live/src/render_cache/policy.rs:153
- [ ] enum `suprnova_live::render_cache::CoherenceMode` · crates/suprnova-live/src/render_cache/policy.rs:160 (also `suprnova_live::render_cache::policy::CoherenceMode`) (re-exported as `suprnova::render_cache::CoherenceMode`)
  - Variants: `Authority`, `Lease`
- [ ] enum `suprnova_live::render_cache::DeclineReason` · crates/suprnova-live/src/render_cache/policy.rs:873 (also `suprnova_live::render_cache::policy::DeclineReason`) (re-exported as `suprnova::render_cache::DeclineReason`)
  - Variants: `PolicyUncacheable`, `Method`, `Status`, `Streaming`, `SetsCookie`, `UnsafeHeader`, `NoStore`
- [ ] enum `suprnova_live::render_cache::Eligibility` · crates/suprnova-live/src/render_cache/policy.rs:864 (also `suprnova_live::render_cache::policy::Eligibility`) (re-exported as `suprnova::render_cache::Eligibility`)
  - Variants: `Store`, `Decline`
- [ ] enum `suprnova_live::render_cache::FailurePolicy` · crates/suprnova-live/src/render_cache/policy.rs:185 (also `suprnova_live::render_cache::policy::FailurePolicy`) (re-exported as `suprnova::render_cache::FailurePolicy`, `suprnova::render_cache::config::FailurePolicy`)
  - Variants: `Open`, `Closed`
- [ ] enum `suprnova_live::render_cache::QueryUnknown` · crates/suprnova-live/src/render_cache/policy.rs:194 (also `suprnova_live::render_cache::policy::QueryUnknown`)
  - Variants: `Bypass`
- [ ] enum `suprnova_live::render_cache::RepresentationClass` · crates/suprnova-live/src/render_cache/policy.rs:23 (also `suprnova_live::render_cache::policy::RepresentationClass`) (re-exported as `suprnova::render_cache::RepresentationClass`)
  - Variants: `PublicShared`, `PublicShellStitched`, `PrivateCached`, `Uncacheable`
  - [ ] fn `suprnova_live::render_cache::RepresentationClass::narrowest` · crates/suprnova-live/src/render_cache/policy.rs:37
- [ ] enum `suprnova_live::render_cache::SharedCachePolicy` · crates/suprnova-live/src/render_cache/policy.rs:173 (also `suprnova_live::render_cache::policy::SharedCachePolicy`) (re-exported as `suprnova::render_cache::SharedCachePolicy`)
  - Variants: `Private`, `SMaxAge`
- [ ] const `suprnova_live::render_cache::policy::MAX_DECLARED_QUERY` · crates/suprnova-live/src/render_cache/policy.rs:12
- [ ] const `suprnova_live::render_cache::policy::MAX_INTERVAL_MS` · crates/suprnova-live/src/render_cache/policy.rs:10
- [ ] const `suprnova_live::render_cache::policy::MAX_NEGOTIATED_VALUE_BYTES` · crates/suprnova-live/src/render_cache/policy.rs:241
- [ ] const `suprnova_live::render_cache::policy::MAX_NEGOTIATED_VALUES` · crates/suprnova-live/src/render_cache/policy.rs:239
- [ ] const `suprnova_live::render_cache::policy::MAX_NEGOTIATION_ENTRIES` · crates/suprnova-live/src/render_cache/policy.rs:245
- [ ] const `suprnova_live::render_cache::policy::UNSAFE_RESPONSE_HEADERS` · crates/suprnova-live/src/render_cache/policy.rs:640

### `suprnova_live::render_cache::singleflight`

- [ ] struct `suprnova_live::render_cache::LocalCoordinatorLimits` · crates/suprnova-live/src/render_cache/singleflight.rs:192 (also `suprnova_live::render_cache::singleflight::LocalCoordinatorLimits`)
  - Public fields: `lease_ms`, `max_waiters`
- [ ] struct `suprnova_live::render_cache::LocalRebuildCoordinator` · crates/suprnova-live/src/render_cache/singleflight.rs:220 (also `suprnova_live::render_cache::singleflight::LocalRebuildCoordinator`)
  - Implements: `suprnova_live::render_cache::RebuildCoordinator`
  - [ ] fn `suprnova_live::render_cache::LocalRebuildCoordinator::new` · crates/suprnova-live/src/render_cache/singleflight.rs:228
- [ ] struct `suprnova_live::render_cache::RebuildLease` · crates/suprnova-live/src/render_cache/singleflight.rs:19 (also `suprnova_live::render_cache::singleflight::RebuildLease`)
  - [ ] fn `suprnova_live::render_cache::RebuildLease::key` · crates/suprnova-live/src/render_cache/singleflight.rs:57
  - [ ] fn `suprnova_live::render_cache::RebuildLease::distributed_lease_id` · crates/suprnova-live/src/render_cache/singleflight.rs:64
- [ ] struct `suprnova_live::render_cache::RebuildWait` · crates/suprnova-live/src/render_cache/singleflight.rs:115 (also `suprnova_live::render_cache::singleflight::RebuildWait`)
  - [ ] fn `suprnova_live::render_cache::RebuildWait::wait` · crates/suprnova-live/src/render_cache/singleflight.rs:121
- [ ] enum `suprnova_live::render_cache::RebuildAdmission` · crates/suprnova-live/src/render_cache/singleflight.rs:160 (also `suprnova_live::render_cache::singleflight::RebuildAdmission`)
  - Variants: `Lead`, `Wait`, `Bypass`
- [ ] trait `suprnova_live::render_cache::RebuildCoordinator` · crates/suprnova-live/src/render_cache/singleflight.rs:172 (also `suprnova_live::render_cache::singleflight::RebuildCoordinator`)
  - Implemented here by: `render_cache::FencedLeaseCoordinator`, `render_cache::LocalRebuildCoordinator`
  - [ ] fn `suprnova_live::render_cache::RebuildCoordinator::admit` · crates/suprnova-live/src/render_cache/singleflight.rs:174 (required)
  - [ ] fn `suprnova_live::render_cache::RebuildCoordinator::publish_token` · crates/suprnova-live/src/render_cache/singleflight.rs:181 (required)
  - [ ] fn `suprnova_live::render_cache::RebuildCoordinator::release` · crates/suprnova-live/src/render_cache/singleflight.rs:187 (required)

### `suprnova_live::render_cache::store`

- [ ] struct `suprnova_live::render_cache::MemoryRenderStore` · crates/suprnova-live/src/render_cache/store.rs:119 (also `suprnova_live::render_cache::store::MemoryRenderStore`)
  - Implements: `suprnova_live::render_cache::RenderStore`
  - [ ] fn `suprnova_live::render_cache::MemoryRenderStore::new` · crates/suprnova-live/src/render_cache/store.rs:127
  - [ ] fn `suprnova_live::render_cache::MemoryRenderStore::publish_hot` · crates/suprnova-live/src/render_cache/store.rs:154
  - [ ] fn `suprnova_live::render_cache::MemoryRenderStore::hot_get` · crates/suprnova-live/src/render_cache/store.rs:177
  - [ ] fn `suprnova_live::render_cache::MemoryRenderStore::clear` · crates/suprnova-live/src/render_cache/store.rs:273
- [ ] struct `suprnova_live::render_cache::MemoryStoreLimits` · crates/suprnova-live/src/render_cache/store.rs:100 (also `suprnova_live::render_cache::store::MemoryStoreLimits`)
  - Public fields: `max_entries`, `max_bytes`
- [ ] struct `suprnova_live::render_cache::PublicationFence` · crates/suprnova-live/src/render_cache/store.rs:16 (also `suprnova_live::render_cache::store::PublicationFence`)
  - Public fields: `epoch`, `generation_digest`, `token`
  - [ ] fn `suprnova_live::render_cache::PublicationFence::supersedes` · crates/suprnova-live/src/render_cache/store.rs:30
- [ ] struct `suprnova_live::render_cache::StoredEntry` · crates/suprnova-live/src/render_cache/store.rs:37 (also `suprnova_live::render_cache::store::StoredEntry`)
  - Public fields: `bytes`, `published_at_ms`, `fence`
- [ ] struct `suprnova_live::render_cache::StoreInspection` · crates/suprnova-live/src/render_cache/store.rs:59 (also `suprnova_live::render_cache::store::StoreInspection`)
  - Public fields: `entries`, `bytes`
- [ ] enum `suprnova_live::render_cache::PublishOutcome` · crates/suprnova-live/src/render_cache/store.rs:48 (also `suprnova_live::render_cache::store::PublishOutcome`)
  - Variants: `Published`, `Fenced`, `Rejected`
- [ ] trait `suprnova_live::render_cache::RenderStore` · crates/suprnova-live/src/render_cache/store.rs:69 (also `suprnova_live::render_cache::store::RenderStore`)
  - Implemented here by: `render_cache::MemoryRenderStore`
  - [ ] fn `suprnova_live::render_cache::RenderStore::get` · crates/suprnova-live/src/render_cache/store.rs:71 (required)
  - [ ] fn `suprnova_live::render_cache::RenderStore::publish` · crates/suprnova-live/src/render_cache/store.rs:84 (required)
  - [ ] fn `suprnova_live::render_cache::RenderStore::evict` · crates/suprnova-live/src/render_cache/store.rs:93 (required)
  - [ ] fn `suprnova_live::render_cache::RenderStore::inspect` · crates/suprnova-live/src/render_cache/store.rs:95 (required)

### `suprnova_live::render_cache::variance`

- [ ] fn `suprnova_live::render_cache::classify` · crates/suprnova-live/src/render_cache/variance.rs:413 (also `suprnova_live::render_cache::variance::classify`)
- [ ] struct `suprnova_live::render_cache::ClassificationOutcome` · crates/suprnova-live/src/render_cache/variance.rs:404 (also `suprnova_live::render_cache::variance::ClassificationOutcome`)
  - Public fields: `class`, `reasons`
- [ ] struct `suprnova_live::render_cache::ObservedContext` · crates/suprnova-live/src/render_cache/variance.rs:368 (also `suprnova_live::render_cache::variance::ObservedContext`)
  - Public fields: `principal`, `tenant`, `session_read`, `authorization`, `secret_context_read`, `undeclared_reads`
- [ ] struct `suprnova_live::render_cache::PrivateMaterial` · crates/suprnova-live/src/render_cache/variance.rs:131 (also `suprnova_live::render_cache::variance::PrivateMaterial`)
  - [ ] fn `suprnova_live::render_cache::PrivateMaterial::principal` · crates/suprnova-live/src/render_cache/variance.rs:146
  - [ ] fn `suprnova_live::render_cache::PrivateMaterial::tenant` · crates/suprnova-live/src/render_cache/variance.rs:156
  - [ ] fn `suprnova_live::render_cache::PrivateMaterial::as_bytes` · crates/suprnova-live/src/render_cache/variance.rs:166
- [ ] struct `suprnova_live::render_cache::VarianceDescriptor` · crates/suprnova-live/src/render_cache/variance.rs:221 (also `suprnova_live::render_cache::variance::VarianceDescriptor`)
  - [ ] fn `suprnova_live::render_cache::VarianceDescriptor::new` · crates/suprnova-live/src/render_cache/variance.rs:228
  - [ ] fn `suprnova_live::render_cache::VarianceDescriptor::declare` · crates/suprnova-live/src/render_cache/variance.rs:233
  - [ ] fn `suprnova_live::render_cache::VarianceDescriptor::dimensions` · crates/suprnova-live/src/render_cache/variance.rs:265
  - [ ] fn `suprnova_live::render_cache::VarianceDescriptor::vary_headers` · crates/suprnova-live/src/render_cache/variance.rs:271
  - [ ] fn `suprnova_live::render_cache::VarianceDescriptor::canonical_len` · crates/suprnova-live/src/render_cache/variance.rs:284
  - [ ] fn `suprnova_live::render_cache::VarianceDescriptor::write_canonical` · crates/suprnova-live/src/render_cache/variance.rs:302
  - [ ] fn `suprnova_live::render_cache::VarianceDescriptor::canonical_bytes` · crates/suprnova-live/src/render_cache/variance.rs:327
- [ ] enum `suprnova_live::render_cache::AuthorizationConsult` · crates/suprnova-live/src/render_cache/variance.rs:340 (also `suprnova_live::render_cache::variance::AuthorizationConsult`)
  - Variants: `None`, `TenantOnly`, `Principal`
  - [ ] fn `suprnova_live::render_cache::AuthorizationConsult::join` · crates/suprnova-live/src/render_cache/variance.rs:357
- [ ] enum `suprnova_live::render_cache::ClassificationReason` · crates/suprnova-live/src/render_cache/variance.rs:385 (also `suprnova_live::render_cache::variance::ClassificationReason`)
  - Variants: `PrincipalObserved`, `TenantObserved`, `SessionValueRead`, `AuthorizationRead`, `AuthorizationTenantRead`, `SecretContextRead`, `UndeclaredContext`
- [ ] enum `suprnova_live::render_cache::DimensionValue` · crates/suprnova-live/src/render_cache/variance.rs:209 (also `suprnova_live::render_cache::variance::DimensionValue`)
  - Variants: `Public`, `Private`, `Anonymous`
- [ ] enum `suprnova_live::render_cache::VarianceDimension` · crates/suprnova-live/src/render_cache/variance.rs:20 (also `suprnova_live::render_cache::variance::VarianceDimension`) (re-exported as `suprnova::render_cache::VarianceDimension`)
  - Variants: `Host`, `Locale`, `Media`, `Encoding`, `Tenant`, `Principal`, `FeatureVersion`, `ConfigVersion`, `Application`
  - [ ] fn `suprnova_live::render_cache::VarianceDimension::vary_header` · crates/suprnova-live/src/render_cache/variance.rs:44
- [ ] const `suprnova_live::render_cache::variance::MAX_DIMENSION_VALUE_BYTES` · crates/suprnova-live/src/render_cache/variance.rs:14
- [ ] const `suprnova_live::render_cache::variance::MAX_DIMENSIONS` · crates/suprnova-live/src/render_cache/variance.rs:16

### `suprnova_live::render_cache`

- [ ] struct `suprnova_live::render_cache::RenderCacheError` · crates/suprnova-live/src/render_cache/mod.rs:120
  - [ ] fn `suprnova_live::render_cache::RenderCacheError::new` · crates/suprnova-live/src/render_cache/mod.rs:127
  - [ ] fn `suprnova_live::render_cache::RenderCacheError::kind` · crates/suprnova-live/src/render_cache/mod.rs:133
- [ ] enum `suprnova_live::render_cache::RenderCacheErrorKind` · crates/suprnova-live/src/render_cache/mod.rs:96
  - Variants: `PolicyInvalid`, `VarianceInvalid`, `KeyInvalid`, `EntryInvalid`, `EntryUnsupported`, `ProviderUnavailable`, `PublicationFenced`, `LeaseFenced`, `AssemblyFailed`
