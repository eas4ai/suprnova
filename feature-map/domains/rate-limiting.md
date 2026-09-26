# Feature map: `manual/rate-limiting.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 81 checked.

## Rust API: suprnova

### `suprnova::rate_limit::algorithm`

- [ ] struct `suprnova::rate_limit::algorithm::Bucket` · framework/src/rate_limit/algorithm.rs:9
  - [ ] fn `suprnova::rate_limit::algorithm::Bucket::new` · framework/src/rate_limit/algorithm.rs:16
  - [ ] fn `suprnova::rate_limit::algorithm::Bucket::evict_old` · framework/src/rate_limit/algorithm.rs:24
  - [ ] fn `suprnova::rate_limit::algorithm::Bucket::try_record` · framework/src/rate_limit/algorithm.rs:37
  - [ ] fn `suprnova::rate_limit::algorithm::Bucket::retry_after` · framework/src/rate_limit/algorithm.rs:53
  - [ ] fn `suprnova::rate_limit::algorithm::Bucket::is_inactive` · framework/src/rate_limit/algorithm.rs:75

### `suprnova::rate_limit::laravel`

- [ ] fn `suprnova::rate_limit::laravel::registry` · framework/src/rate_limit/laravel.rs:133
- [ ] struct `suprnova::rate_limit::NamedLimiterRegistry` · framework/src/rate_limit/laravel.rs:78 (also `suprnova::rate_limit::laravel::NamedLimiterRegistry`)
  - [ ] fn `suprnova::rate_limit::NamedLimiterRegistry::new` · framework/src/rate_limit/laravel.rs:92
  - [ ] fn `suprnova::rate_limit::NamedLimiterRegistry::insert` · framework/src/rate_limit/laravel.rs:100
  - [ ] fn `suprnova::rate_limit::NamedLimiterRegistry::get` · framework/src/rate_limit/laravel.rs:116
  - [ ] fn `suprnova::rate_limit::NamedLimiterRegistry::has` · framework/src/rate_limit/laravel.rs:122
- [ ] struct `suprnova::RateLimiter` · framework/src/rate_limit/laravel.rs:176 (also `suprnova::rate_limit::RateLimiter`, `suprnova::rate_limit::laravel::RateLimiter`)
  - [ ] fn `suprnova::RateLimiter::define` · framework/src/rate_limit/laravel.rs:206
  - [ ] fn `suprnova::RateLimiter::for` · framework/src/rate_limit/laravel.rs:216
  - [ ] fn `suprnova::RateLimiter::limiter` · framework/src/rate_limit/laravel.rs:226
  - [ ] fn `suprnova::RateLimiter::has_limiter` · framework/src/rate_limit/laravel.rs:231
  - [ ] fn `suprnova::RateLimiter::available_in` · framework/src/rate_limit/laravel.rs:242
  - [ ] fn `suprnova::RateLimiter::attempts` · framework/src/rate_limit/laravel.rs:256
  - [ ] fn `suprnova::RateLimiter::reset_attempts` · framework/src/rate_limit/laravel.rs:265
  - [ ] fn `suprnova::RateLimiter::too_many_attempts` · framework/src/rate_limit/laravel.rs:280
  - [ ] fn `suprnova::RateLimiter::hit` · framework/src/rate_limit/laravel.rs:296
  - [ ] fn `suprnova::RateLimiter::hit_and_check` · framework/src/rate_limit/laravel.rs:322
  - [ ] fn `suprnova::RateLimiter::increment` · framework/src/rate_limit/laravel.rs:341
  - [ ] fn `suprnova::RateLimiter::decrement` · framework/src/rate_limit/laravel.rs:364
  - [ ] fn `suprnova::RateLimiter::remaining` · framework/src/rate_limit/laravel.rs:380
  - [ ] fn `suprnova::RateLimiter::retries_left` · framework/src/rate_limit/laravel.rs:388
  - [ ] fn `suprnova::RateLimiter::clear` · framework/src/rate_limit/laravel.rs:395
  - [ ] fn `suprnova::RateLimiter::clean_rate_limiter_key` · framework/src/rate_limit/laravel.rs:413
  - [ ] fn `suprnova::RateLimiter::attempt` · framework/src/rate_limit/laravel.rs:456
  - [ ] fn `suprnova::RateLimiter::is_cache_initialized` · framework/src/rate_limit/laravel.rs:478

### `suprnova::rate_limit::limit`

- [ ] struct `suprnova::GlobalLimit` · framework/src/rate_limit/limit.rs:231 (also `suprnova::rate_limit::GlobalLimit`, `suprnova::rate_limit::limit::GlobalLimit`)
  - Public tuple fields: 1
  - [ ] fn `suprnova::GlobalLimit::new` · framework/src/rate_limit/limit.rs:235
  - [ ] fn `suprnova::GlobalLimit::per_minute` · framework/src/rate_limit/limit.rs:240
  - [ ] fn `suprnova::GlobalLimit::per_hour` · framework/src/rate_limit/limit.rs:245
- [ ] struct `suprnova::Limit` · framework/src/rate_limit/limit.rs:84 (also `suprnova::rate_limit::Limit`, `suprnova::rate_limit::limit::Limit`)
  - Public fields: `key`, `max_attempts`, `decay`, `after_callback`, `response_callback`
  - [ ] fn `suprnova::Limit::new` · framework/src/rate_limit/limit.rs:108
  - [ ] fn `suprnova::Limit::per_second` · framework/src/rate_limit/limit.rs:120
  - [ ] fn `suprnova::Limit::per_minute` · framework/src/rate_limit/limit.rs:125
  - [ ] fn `suprnova::Limit::per_minutes` · framework/src/rate_limit/limit.rs:132
  - [ ] fn `suprnova::Limit::per_hour` · framework/src/rate_limit/limit.rs:137
  - [ ] fn `suprnova::Limit::per_hours` · framework/src/rate_limit/limit.rs:142
  - [ ] fn `suprnova::Limit::per_day` · framework/src/rate_limit/limit.rs:147
  - [ ] fn `suprnova::Limit::per_days` · framework/src/rate_limit/limit.rs:152
  - [ ] fn `suprnova::Limit::none` · framework/src/rate_limit/limit.rs:159
  - [ ] fn `suprnova::Limit::by` · framework/src/rate_limit/limit.rs:172
  - [ ] fn `suprnova::Limit::after` · framework/src/rate_limit/limit.rs:180
  - [ ] fn `suprnova::Limit::response` · framework/src/rate_limit/limit.rs:190
  - [ ] fn `suprnova::Limit::decay_seconds` · framework/src/rate_limit/limit.rs:201
  - [ ] fn `suprnova::Limit::fallback_key` · framework/src/rate_limit/limit.rs:209
- [ ] struct `suprnova::Unlimited` · framework/src/rate_limit/limit.rs:271 (also `suprnova::rate_limit::Unlimited`, `suprnova::rate_limit::limit::Unlimited`)
  - Public tuple fields: 1
  - [ ] fn `suprnova::Unlimited::new` · framework/src/rate_limit/limit.rs:276
- [ ] enum `suprnova::LimitResult` · framework/src/rate_limit/limit.rs:47 (also `suprnova::rate_limit::LimitResult`, `suprnova::rate_limit::limit::LimitResult`)
  - Variants: `Single`, `Many`, `Response`
- [ ] type `suprnova::rate_limit::limit::AfterCallback` · framework/src/rate_limit/limit.rs:33
- [ ] type `suprnova::rate_limit::limit::ResponseCallback` · framework/src/rate_limit/limit.rs:37

### `suprnova::rate_limit::memory`

- [ ] struct `suprnova::rate_limit::memory::InMemoryRateLimiter` · framework/src/rate_limit/memory.rs:41
  - Implements: `suprnova::RateLimiterDriver`
  - [ ] fn `suprnova::rate_limit::memory::InMemoryRateLimiter::new` · framework/src/rate_limit/memory.rs:51
  - [ ] fn `suprnova::rate_limit::memory::InMemoryRateLimiter::with_periodic_sweep` · framework/src/rate_limit/memory.rs:74
  - [ ] fn `suprnova::rate_limit::memory::InMemoryRateLimiter::purge_inactive` · framework/src/rate_limit/memory.rs:115
  - [ ] fn `suprnova::rate_limit::memory::InMemoryRateLimiter::bucket_count` · framework/src/rate_limit/memory.rs:127

### `suprnova::rate_limit::redis`

- [ ] struct `suprnova::rate_limit::redis::RedisRateLimiter` · framework/src/rate_limit/redis.rs:14
  - Implements: `suprnova::RateLimiterDriver`
  - [ ] fn `suprnova::rate_limit::redis::RedisRateLimiter::connect` · framework/src/rate_limit/redis.rs:22

### `suprnova::rate_limit::throttle`

- [ ] struct `suprnova::ThrottleRequestsMiddleware` · framework/src/rate_limit/throttle.rs:48 (also `suprnova::rate_limit::ThrottleRequestsMiddleware`, `suprnova::rate_limit::throttle::ThrottleRequestsMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::ThrottleRequestsMiddleware::by_name` · framework/src/rate_limit/throttle.rs:73
  - [ ] fn `suprnova::ThrottleRequestsMiddleware::with` · framework/src/rate_limit/throttle.rs:83
  - [ ] fn `suprnova::ThrottleRequestsMiddleware::with_limits` · framework/src/rate_limit/throttle.rs:97
  - [ ] fn `suprnova::ThrottleRequestsMiddleware::prefix` · framework/src/rate_limit/throttle.rs:107

### `suprnova::rate_limit`

- [ ] fn `suprnova::rate_limit::bootstrap_default` · framework/src/rate_limit/mod.rs:110
- [ ] fn `suprnova::rate_limit::bootstrap_from_env` · framework/src/rate_limit/mod.rs:250
- [ ] fn `suprnova::identity_key` · framework/src/rate_limit/mod.rs:357 (also `suprnova::rate_limit::identity_key`)
- [ ] fn `suprnova::names_identity` · framework/src/rate_limit/mod.rs:397 (also `suprnova::rate_limit::names_identity`)
- [ ] struct `suprnova::RateLimitMiddleware` · framework/src/rate_limit/mod.rs:480 (also `suprnova::rate_limit::RateLimitMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::RateLimitMiddleware::new` · framework/src/rate_limit/mod.rs:507
  - [ ] fn `suprnova::RateLimitMiddleware::only_when` · framework/src/rate_limit/mod.rs:539
  - [ ] fn `suprnova::RateLimitMiddleware::key_reads_body` · framework/src/rate_limit/mod.rs:569
  - [ ] fn `suprnova::RateLimitMiddleware::on_backend_error` · framework/src/rate_limit/mod.rs:580
- [ ] struct `suprnova::SlidingWindowConfig` · framework/src/rate_limit/mod.rs:51 (also `suprnova::rate_limit::SlidingWindowConfig`)
  - Public fields: `max_requests`, `window`
- [ ] enum `suprnova::BackendErrorPolicy` · framework/src/rate_limit/mod.rs:433 (also `suprnova::rate_limit::BackendErrorPolicy`)
  - Variants: `FailOpen`, `FailClosed`
- [ ] trait `suprnova::RateLimiterDriver` · framework/src/rate_limit/mod.rs:66 (also `suprnova::rate_limit::RateLimiterDriver`)
  - Implemented here by: `rate_limit::memory::InMemoryRateLimiter`, `rate_limit::redis::RedisRateLimiter`
  - [ ] fn `suprnova::RateLimiterDriver::try_acquire` · framework/src/rate_limit/mod.rs:69 (required)
  - [ ] fn `suprnova::RateLimiterDriver::retry_after` · framework/src/rate_limit/mod.rs:77 (required)
