# Feature map: `manual/render-cache-deployment.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 38 checked.

## Endpoints and tables

### framework migration `render_cache::migration::Migration` / `TierMigration`

- [ ] table `suprnova_render_generations (framework)` · framework/src/render_cache/migration.rs:168
- [ ] table `suprnova_render_generation_log (framework)` · framework/src/render_cache/migration.rs:178
- [ ] table `suprnova_render_epochs (framework)` · framework/src/render_cache/migration.rs:189
- [ ] table `suprnova_render_entries (framework)` · framework/src/render_cache/migration.rs:485
- [ ] table `suprnova_render_leases (framework)` · framework/src/render_cache/migration.rs:498
- [ ] table `suprnova_live_instances (framework)` · framework/src/render_cache/migration.rs:509
- [ ] table `suprnova_live_promotions (framework)` · framework/src/render_cache/migration.rs:520

## Rust API: suprnova

### `suprnova::render_cache::file_store`

- [ ] struct `suprnova::render_cache::file_store::FileRenderStore` · framework/src/render_cache/file_store.rs:117
  - Implements: `suprnova_live::render_cache::RenderStore`
  - [ ] fn `suprnova::render_cache::file_store::FileRenderStore::open` · framework/src/render_cache/file_store.rs:145
  - [ ] fn `suprnova::render_cache::file_store::FileRenderStore::sweep` · framework/src/render_cache/file_store.rs:577
- [ ] struct `suprnova::render_cache::SweepOutcome` · framework/src/render_cache/file_store.rs:630 (also `suprnova::render_cache::file_store::SweepOutcome`)
  - Public fields: `removed`, `more_remain`

### `suprnova::render_cache::ledger`

- [ ] fn `suprnova::render_cache::ledger::advance_in_current_transaction` · framework/src/render_cache/ledger.rs:372
- [ ] fn `suprnova::render_cache::ledger::advance_via_handle` · framework/src/render_cache/ledger.rs:599
- [ ] fn `suprnova::render_cache::ledger::advance_via_tx` · framework/src/render_cache/ledger.rs:574
- [ ] fn `suprnova::render_cache::ledger::tier_migration_present` · framework/src/render_cache/ledger.rs:207
- [ ] struct `suprnova::render_cache::ledger::SqlGenerationLedger` · framework/src/render_cache/ledger.rs:615
  - Implements: `suprnova_live::render_cache::GenerationLedger`
  - [ ] fn `suprnova::render_cache::ledger::SqlGenerationLedger::new` · framework/src/render_cache/ledger.rs:620
  - [ ] fn `suprnova::render_cache::ledger::SqlGenerationLedger::advance_epoch` · framework/src/render_cache/ledger.rs:636

### `suprnova::render_cache::migration`

- [ ] struct `suprnova::render_cache::migration::Migration` · framework/src/render_cache/migration.rs:23
- [ ] struct `suprnova::render_cache::migration::TierMigration` · framework/src/render_cache/migration.rs:244

### `suprnova::render_cache::providers::redis_instances`

- [ ] struct `suprnova::render_cache::providers::RedisInstanceRecordStore` · framework/src/render_cache/providers/redis_instances.rs:245 (also `suprnova::render_cache::providers::redis_instances::RedisInstanceRecordStore`)
  - Implements: `suprnova_live::ledger::InstanceRecordStore`
  - [ ] fn `suprnova::render_cache::providers::RedisInstanceRecordStore::connect` · framework/src/render_cache/providers/redis_instances.rs:261
  - [ ] fn `suprnova::render_cache::providers::RedisInstanceRecordStore::open` · framework/src/render_cache/providers/redis_instances.rs:280

### `suprnova::render_cache::providers::redis_lease`

- [ ] struct `suprnova::render_cache::providers::RedisLeaseStore` · framework/src/render_cache/providers/redis_lease.rs:152 (also `suprnova::render_cache::providers::redis_lease::RedisLeaseStore`)
  - Implements: `suprnova_live::render_cache::LeaseStore`
  - [ ] fn `suprnova::render_cache::providers::RedisLeaseStore::connect` · framework/src/render_cache/providers/redis_lease.rs:168

### `suprnova::render_cache::providers::redis_store`

- [ ] struct `suprnova::render_cache::RedisRenderStore` · framework/src/render_cache/providers/redis_store.rs:169 (also `suprnova::render_cache::providers::RedisRenderStore`, `suprnova::render_cache::providers::redis_store::RedisRenderStore`)
  - Implements: `suprnova_live::render_cache::RenderStore`
  - [ ] fn `suprnova::render_cache::RedisRenderStore::connect` · framework/src/render_cache/providers/redis_store.rs:186

### `suprnova::render_cache::providers::redis`

- [ ] fn `suprnova::render_cache::providers::redis::ping` · framework/src/render_cache/providers/redis.rs:295
- [ ] struct `suprnova::render_cache::providers::RedisProviderConfig` · framework/src/render_cache/providers/redis.rs:115 (also `suprnova::render_cache::providers::redis::RedisProviderConfig`)
  - Public fields: `url`, `prefix`

### `suprnova::render_cache::providers::sql_instances`

- [ ] struct `suprnova::render_cache::providers::SqlInstanceRecordStore` · framework/src/render_cache/providers/sql_instances.rs:93 (also `suprnova::render_cache::providers::sql_instances::SqlInstanceRecordStore`)
  - Implements: `suprnova_live::ledger::InstanceRecordStore`
  - [ ] fn `suprnova::render_cache::providers::SqlInstanceRecordStore::new` · framework/src/render_cache/providers/sql_instances.rs:104

### `suprnova::render_cache::providers::sql_lease`

- [ ] struct `suprnova::render_cache::providers::SqlLeaseStore` · framework/src/render_cache/providers/sql_lease.rs:73 (also `suprnova::render_cache::providers::sql_lease::SqlLeaseStore`)
  - Implements: `suprnova_live::render_cache::LeaseStore`
  - [ ] fn `suprnova::render_cache::providers::SqlLeaseStore::new` · framework/src/render_cache/providers/sql_lease.rs:83

### `suprnova::render_cache::providers::sql_store`

- [ ] struct `suprnova::render_cache::SqlRenderStore` · framework/src/render_cache/providers/sql_store.rs:82 (also `suprnova::render_cache::providers::SqlRenderStore`, `suprnova::render_cache::providers::sql_store::SqlRenderStore`)
  - Implements: `suprnova_live::render_cache::RenderStore`
  - [ ] fn `suprnova::render_cache::SqlRenderStore::new` · framework/src/render_cache/providers/sql_store.rs:94
  - [ ] fn `suprnova::render_cache::SqlRenderStore::sweep` · framework/src/render_cache/providers/sql_store.rs:136

### `suprnova::render_cache::providers`

- [ ] fn `suprnova::render_cache::providers::sql_now_ms` · framework/src/render_cache/providers/mod.rs:66
- [ ] fn `suprnova::render_cache::providers::store_now_ms` · framework/src/render_cache/providers/mod.rs:96
