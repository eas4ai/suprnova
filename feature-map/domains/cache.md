# Feature map: `manual/cache.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 60 checked.

## Rust API: suprnova

### `suprnova::cache::config`

- [ ] struct `suprnova::CacheConfig` · framework/src/cache/config.rs:69 (also `suprnova::cache::CacheConfig`, `suprnova::cache::config::CacheConfig`)
  - Public fields: `driver`, `url`, `prefix`, `default_ttl`
  - [ ] fn `suprnova::CacheConfig::from_env` · framework/src/cache/config.rs:89
  - [ ] fn `suprnova::CacheConfig::builder` · framework/src/cache/config.rs:103
- [ ] struct `suprnova::cache::CacheConfigBuilder` · framework/src/cache/config.rs:125 (also `suprnova::cache::config::CacheConfigBuilder`)
  - [ ] fn `suprnova::cache::CacheConfigBuilder::driver` · framework/src/cache/config.rs:134
  - [ ] fn `suprnova::cache::CacheConfigBuilder::url` · framework/src/cache/config.rs:140
  - [ ] fn `suprnova::cache::CacheConfigBuilder::prefix` · framework/src/cache/config.rs:146
  - [ ] fn `suprnova::cache::CacheConfigBuilder::default_ttl` · framework/src/cache/config.rs:152
  - [ ] fn `suprnova::cache::CacheConfigBuilder::build` · framework/src/cache/config.rs:160
- [ ] enum `suprnova::cache::CacheDriver` · framework/src/cache/config.rs:16 (also `suprnova::cache::config::CacheDriver`)
  - Variants: `Memory`, `Redis`
  - [ ] fn `suprnova::cache::CacheDriver::parse` · framework/src/cache/config.rs:29

### `suprnova::cache::memory`

- [ ] struct `suprnova::InMemoryCache` · framework/src/cache/memory.rs:103 (also `suprnova::cache::InMemoryCache`, `suprnova::cache::memory::InMemoryCache`)
  - Implements: `suprnova::CacheStore`
  - [ ] fn `suprnova::InMemoryCache::new` · framework/src/cache/memory.rs:119
  - [ ] fn `suprnova::InMemoryCache::with_prefix` · framework/src/cache/memory.rs:129
  - [ ] fn `suprnova::InMemoryCache::with_config` · framework/src/cache/memory.rs:141
  - [ ] fn `suprnova::InMemoryCache::purge_expired` · framework/src/cache/memory.rs:228

### `suprnova::cache::redis`

- [ ] struct `suprnova::RedisCache` · framework/src/cache/redis.rs:118 (also `suprnova::cache::RedisCache`, `suprnova::cache::redis::RedisCache`)
  - Implements: `suprnova::CacheStore`
  - [ ] fn `suprnova::RedisCache::connect` · framework/src/cache/redis.rs:126

### `suprnova::cache::store`

- [ ] trait `suprnova::CacheStore` · framework/src/cache/store.rs:24 (also `suprnova::cache::CacheStore`, `suprnova::cache::store::CacheStore`)
  - Implemented here by: `InMemoryCache`, `RedisCache`
  - [ ] fn `suprnova::CacheStore::get_raw` · framework/src/cache/store.rs:26 (required)
  - [ ] fn `suprnova::CacheStore::put_raw` · framework/src/cache/store.rs:32 (required)
  - [ ] fn `suprnova::CacheStore::add_raw` · framework/src/cache/store.rs:51 (provided)
  - [ ] fn `suprnova::CacheStore::default_ttl` · framework/src/cache/store.rs:68 (provided)
  - [ ] fn `suprnova::CacheStore::has` · framework/src/cache/store.rs:73 (required)
  - [ ] fn `suprnova::CacheStore::forget` · framework/src/cache/store.rs:76 (required)
  - [ ] fn `suprnova::CacheStore::flush` · framework/src/cache/store.rs:79 (required)
  - [ ] fn `suprnova::CacheStore::increment` · framework/src/cache/store.rs:84 (required)
  - [ ] fn `suprnova::CacheStore::decrement` · framework/src/cache/store.rs:89 (required)
  - [ ] fn `suprnova::CacheStore::tagged_put_raw` · framework/src/cache/store.rs:96 (required)
  - [ ] fn `suprnova::CacheStore::flush_tags` · framework/src/cache/store.rs:118 (required)
  - [ ] fn `suprnova::CacheStore::acquire_lock` · framework/src/cache/store.rs:123 (required)
  - [ ] fn `suprnova::CacheStore::release_lock` · framework/src/cache/store.rs:132 (required)
  - [ ] fn `suprnova::CacheStore::refresh_lock` · framework/src/cache/store.rs:136 (required)
  - [ ] fn `suprnova::CacheStore::touch` · framework/src/cache/store.rs:146 (required)

### `suprnova::cache`

- [ ] struct `suprnova::Cache` · framework/src/cache/mod.rs:85 (also `suprnova::cache::Cache`, `suprnova::prelude::Cache`)
  - [ ] fn `suprnova::Cache::store` · framework/src/cache/mod.rs:130
  - [ ] fn `suprnova::Cache::is_initialized` · framework/src/cache/mod.rs:135
  - [ ] fn `suprnova::Cache::get` · framework/src/cache/mod.rs:155
  - [ ] fn `suprnova::Cache::put` · framework/src/cache/mod.rs:185
  - [ ] fn `suprnova::Cache::forever` · framework/src/cache/mod.rs:212
  - [ ] fn `suprnova::Cache::has` · framework/src/cache/mod.rs:234
  - [ ] fn `suprnova::Cache::missing` · framework/src/cache/mod.rs:253
  - [ ] fn `suprnova::Cache::pull` · framework/src/cache/mod.rs:275
  - [ ] fn `suprnova::Cache::add` · framework/src/cache/mod.rs:314
  - [ ] fn `suprnova::Cache::sear` · framework/src/cache/mod.rs:343
  - [ ] fn `suprnova::Cache::forget` · framework/src/cache/mod.rs:364
  - [ ] fn `suprnova::Cache::flush` · framework/src/cache/mod.rs:379
  - [ ] fn `suprnova::Cache::increment` · framework/src/cache/mod.rs:397
  - [ ] fn `suprnova::Cache::decrement` · framework/src/cache/mod.rs:415
  - [ ] fn `suprnova::Cache::remember` · framework/src/cache/mod.rs:469
  - [ ] fn `suprnova::Cache::remember_forever` · framework/src/cache/mod.rs:498
  - [ ] fn `suprnova::Cache::tags_put` · framework/src/cache/mod.rs:523
  - [ ] fn `suprnova::Cache::flush_tags` · framework/src/cache/mod.rs:545
  - [ ] fn `suprnova::Cache::lock` · framework/src/cache/mod.rs:572
  - [ ] fn `suprnova::Cache::touch` · framework/src/cache/mod.rs:598
- [ ] struct `suprnova::LockGuard` · framework/src/cache/mod.rs:608 (also `suprnova::cache::LockGuard`)
  - [ ] fn `suprnova::LockGuard::token` · framework/src/cache/mod.rs:616
  - [ ] fn `suprnova::LockGuard::owner` · framework/src/cache/mod.rs:623
  - [ ] fn `suprnova::LockGuard::release` · framework/src/cache/mod.rs:629
  - [ ] fn `suprnova::LockGuard::refresh` · framework/src/cache/mod.rs:635
