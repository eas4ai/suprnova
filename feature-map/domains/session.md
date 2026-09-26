# Feature map: `manual/session.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 114 checked.

## Rust API: suprnova

### `suprnova::session::blocking`

- [ ] struct `suprnova::SessionBlock` · framework/src/session/blocking.rs:33 (also `suprnova::session::SessionBlock`, `suprnova::session::blocking::SessionBlock`)
  - [ ] fn `suprnova::SessionBlock::new` · framework/src/session/blocking.rs:51
  - [ ] fn `suprnova::SessionBlock::lock_for` · framework/src/session/blocking.rs:59
  - [ ] fn `suprnova::SessionBlock::wait_for` · framework/src/session/blocking.rs:64

### `suprnova::session::config`

- [ ] struct `suprnova::SessionConfig` · framework/src/session/config.rs:32 (also `suprnova::session::SessionConfig`, `suprnova::session::config::SessionConfig`)
  - Public fields: `lifetime`, `touch_interval`, `gc_interval`, `cookie_name`, `cookie_path`, `cookie_domain`, `cookie_secure`, `cookie_http_only`, `cookie_same_site`, `cookie_partitioned`, `cookie_prefix`, `expire_on_close`, `table_name`, `connection`, `remember_lifetime`, `block`
  - [ ] fn `suprnova::SessionConfig::new` · framework/src/session/config.rs:121
  - [ ] fn `suprnova::SessionConfig::from_env` · framework/src/session/config.rs:155
  - [ ] fn `suprnova::SessionConfig::lifetime` · framework/src/session/config.rs:240
  - [ ] fn `suprnova::SessionConfig::touch_interval` · framework/src/session/config.rs:246
  - [ ] fn `suprnova::SessionConfig::gc_interval` · framework/src/session/config.rs:252
  - [ ] fn `suprnova::SessionConfig::cookie_name` · framework/src/session/config.rs:258
  - [ ] fn `suprnova::SessionConfig::secure` · framework/src/session/config.rs:264
  - [ ] fn `suprnova::SessionConfig::remember_lifetime` · framework/src/session/config.rs:270
  - [ ] fn `suprnova::SessionConfig::domain` · framework/src/session/config.rs:276
  - [ ] fn `suprnova::SessionConfig::partitioned` · framework/src/session/config.rs:282
  - [ ] fn `suprnova::SessionConfig::expire_on_close` · framework/src/session/config.rs:289
  - [ ] fn `suprnova::SessionConfig::connection` · framework/src/session/config.rs:295
  - [ ] fn `suprnova::SessionConfig::block` · framework/src/session/config.rs:302
- [ ] const `suprnova::session::MAX_SESSION_LIFETIME_MINUTES` · framework/src/session/config.rs:23 (also `suprnova::session::config::MAX_SESSION_LIFETIME_MINUTES`)
- [ ] const `suprnova::session::MAX_SESSION_LIFETIME_SECS` · framework/src/session/config.rs:18 (also `suprnova::session::config::MAX_SESSION_LIFETIME_SECS`)

### `suprnova::session::driver::database::sessions`

- [ ] struct `suprnova::session::driver::database::sessions::ActiveModel` · framework/src/session/driver/database.rs:336
  - Public fields: `id`, `user_id`, `payload`, `csrf_token`, `last_activity`
- [ ] struct `suprnova::session::driver::database::sessions::ColumnIter` · framework/src/session/driver/database.rs:336
- [ ] struct `suprnova::session::driver::database::sessions::Entity` · framework/src/session/driver/database.rs:336
- [ ] struct `suprnova::session::driver::database::sessions::Model` · framework/src/session/driver/database.rs:338
  - Public fields: `id`, `user_id`, `payload`, `csrf_token`, `last_activity`
  - [ ] fn `suprnova::session::driver::database::sessions::Model::into_ex` · framework/src/session/driver/database.rs:336
- [ ] struct `suprnova::session::driver::database::sessions::PrimaryKeyIter` · framework/src/session/driver/database.rs:336
- [ ] struct `suprnova::session::driver::database::sessions::RelationIter` · framework/src/session/driver/database.rs:355
- [ ] enum `suprnova::session::driver::database::sessions::Column` · framework/src/session/driver/database.rs:336
  - Variants: `Id`, `UserId`, `Payload`, `CsrfToken`, `LastActivity`
- [ ] enum `suprnova::session::driver::database::sessions::PrimaryKey` · framework/src/session/driver/database.rs:336
  - Variants: `Id`
- [ ] enum `suprnova::session::driver::database::sessions::Relation` · framework/src/session/driver/database.rs:356

### `suprnova::session::driver::database`

- [ ] struct `suprnova::DatabaseSessionDriver` · framework/src/session/driver/database.rs:25 (also `suprnova::session::DatabaseSessionDriver`, `suprnova::session::driver::DatabaseSessionDriver`, `suprnova::session::driver::database::DatabaseSessionDriver`)
  - Implements: `suprnova::SessionStore`
  - [ ] fn `suprnova::DatabaseSessionDriver::new` · framework/src/session/driver/database.rs:31

### `suprnova::session::middleware`

- [ ] fn `suprnova::auth_user_id` · framework/src/session/middleware.rs:2202 (also `suprnova::session::auth_user_id`, `suprnova::session::middleware::auth_user_id`)
- [ ] fn `suprnova::clear_auth_user` · framework/src/session/middleware.rs:2256 (also `suprnova::session::clear_auth_user`, `suprnova::session::middleware::clear_auth_user`)
- [ ] fn `suprnova::session::clear_two_factor_pending` · framework/src/session/middleware.rs:2300 (also `suprnova::session::middleware::clear_two_factor_pending`)
- [ ] fn `suprnova::session::clear_two_factor_pending_remember` · framework/src/session/middleware.rs:2361 (also `suprnova::session::middleware::clear_two_factor_pending_remember`)
- [ ] fn `suprnova::generate_csrf_token` · framework/src/session/middleware.rs:465 (also `suprnova::session::generate_csrf_token`, `suprnova::session::middleware::generate_csrf_token`)
- [ ] fn `suprnova::generate_session_id` · framework/src/session/middleware.rs:450 (also `suprnova::session::generate_session_id`, `suprnova::session::middleware::generate_session_id`)
- [ ] fn `suprnova::get_csrf_token` · framework/src/session/middleware.rs:2079 (also `suprnova::session::get_csrf_token`, `suprnova::session::middleware::get_csrf_token`)
- [ ] fn `suprnova::invalidate_session` · framework/src/session/middleware.rs:2070 (also `suprnova::session::invalidate_session`, `suprnova::session::middleware::invalidate_session`)
- [ ] fn `suprnova::is_authenticated` · framework/src/session/middleware.rs:2097 (also `suprnova::session::is_authenticated`, `suprnova::session::middleware::is_authenticated`)
- [ ] fn `suprnova::regenerate_csrf_token` · framework/src/session/middleware.rs:2087 (also `suprnova::session::middleware::regenerate_csrf_token`, `suprnova::session::regenerate_csrf_token`)
- [ ] fn `suprnova::regenerate_session_id` · framework/src/session/middleware.rs:2056 (also `suprnova::session::middleware::regenerate_session_id`, `suprnova::session::regenerate_session_id`)
- [ ] fn `suprnova::session` · framework/src/session/middleware.rs:410 (also `suprnova::session::middleware::session`, `suprnova::session::session`)
- [ ] fn `suprnova::session::session_gc_metrics` · framework/src/session/middleware.rs:543 (also `suprnova::session::middleware::session_gc_metrics`)
- [ ] fn `suprnova::session_mut` · framework/src/session/middleware.rs:429 (also `suprnova::session::middleware::session_mut`, `suprnova::session::session_mut`)
- [ ] fn `suprnova::set_auth_user` · framework/src/session/middleware.rs:2237 (also `suprnova::session::middleware::set_auth_user`, `suprnova::session::set_auth_user`)
- [ ] fn `suprnova::session::set_two_factor_pending` · framework/src/session/middleware.rs:2286 (also `suprnova::session::middleware::set_two_factor_pending`)
- [ ] fn `suprnova::session::set_two_factor_pending_remember` · framework/src/session/middleware.rs:2343 (also `suprnova::session::middleware::set_two_factor_pending_remember`)
- [ ] fn `suprnova::session::two_factor_pending_remember` · framework/src/session/middleware.rs:2326 (also `suprnova::session::middleware::two_factor_pending_remember`)
- [ ] fn `suprnova::session::two_factor_pending_user_id` · framework/src/session/middleware.rs:2274 (also `suprnova::session::middleware::two_factor_pending_user_id`)
- [ ] struct `suprnova::session::SessionGcMetrics` · framework/src/session/middleware.rs:527 (also `suprnova::session::middleware::SessionGcMetrics`)
  - Public fields: `runs`, `successes`, `failures`, `removed_rows`, `last_success_unix_seconds`, `last_failure_unix_seconds`
- [ ] struct `suprnova::SessionGcSupervisor` · framework/src/session/middleware.rs:975 (also `suprnova::session::SessionGcSupervisor`, `suprnova::session::middleware::SessionGcSupervisor`)
  - Public fields: `store`, `interval`
  - Implements: `suprnova::Supervisor`
- [ ] struct `suprnova::SessionMiddleware` · framework/src/session/middleware.rs:562 (also `suprnova::session::SessionMiddleware`, `suprnova::session::middleware::SessionMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::SessionMiddleware::new` · framework/src/session/middleware.rs:607
  - [ ] fn `suprnova::SessionMiddleware::with_store` · framework/src/session/middleware.rs:614
  - [ ] fn `suprnova::SessionMiddleware::install_with_gc` · framework/src/session/middleware.rs:636
  - [ ] fn `suprnova::SessionMiddleware::install` · framework/src/session/middleware.rs:649
  - [ ] fn `suprnova::SessionMiddleware::store` · framework/src/session/middleware.rs:656

### `suprnova::session::store`

- [ ] fn `suprnova::is_valid_session_id` · framework/src/session/store.rs:744 (also `suprnova::session::is_valid_session_id`, `suprnova::session::store::is_valid_session_id`)
- [ ] struct `suprnova::SessionData` · framework/src/session/store.rs:24 (also `suprnova::session::SessionData`, `suprnova::session::store::SessionData`)
  - Public fields: `id`, `data`, `user_id`, `csrf_token`, `dirty`, `loaded_from_store`
  - [ ] fn `suprnova::SessionData::new` · framework/src/session/store.rs:57
  - [ ] fn `suprnova::SessionData::rotate_id` · framework/src/session/store.rs:82
  - [ ] fn `suprnova::SessionData::get` · framework/src/session/store.rs:225
  - [ ] fn `suprnova::SessionData::put` · framework/src/session/store.rs:241
  - [ ] fn `suprnova::SessionData::forget` · framework/src/session/store.rs:251
  - [ ] fn `suprnova::SessionData::has` · framework/src/session/store.rs:263
  - [ ] fn `suprnova::SessionData::flash` · framework/src/session/store.rs:276
  - [ ] fn `suprnova::SessionData::get_flash` · framework/src/session/store.rs:281
  - [ ] fn `suprnova::SessionData::age_flash_data` · framework/src/session/store.rs:291
  - [ ] fn `suprnova::SessionData::flush` · framework/src/session/store.rs:325
  - [ ] fn `suprnova::SessionData::set_magnetar_web_binding` · framework/src/session/store.rs:332
  - [ ] fn `suprnova::SessionData::magnetar_web_binding` · framework/src/session/store.rs:337
  - [ ] fn `suprnova::SessionData::clear_magnetar_web_binding` · framework/src/session/store.rs:352
  - [ ] fn `suprnova::SessionData::is_dirty` · framework/src/session/store.rs:357
  - [ ] fn `suprnova::SessionData::mark_clean` · framework/src/session/store.rs:362
  - [ ] fn `suprnova::SessionData::pull` · framework/src/session/store.rs:375
  - [ ] fn `suprnova::SessionData::push` · framework/src/session/store.rs:387
  - [ ] fn `suprnova::SessionData::increment` · framework/src/session/store.rs:403
  - [ ] fn `suprnova::SessionData::decrement` · framework/src/session/store.rs:412
  - [ ] fn `suprnova::SessionData::remember` · framework/src/session/store.rs:419
  - [ ] fn `suprnova::SessionData::has_any` · framework/src/session/store.rs:434
  - [ ] fn `suprnova::SessionData::has_all` · framework/src/session/store.rs:440
  - [ ] fn `suprnova::SessionData::missing` · framework/src/session/store.rs:446
  - [ ] fn `suprnova::SessionData::all` · framework/src/session/store.rs:454
  - [ ] fn `suprnova::SessionData::only` · framework/src/session/store.rs:460
  - [ ] fn `suprnova::SessionData::except` · framework/src/session/store.rs:472
  - [ ] fn `suprnova::SessionData::replace` · framework/src/session/store.rs:489
  - [ ] fn `suprnova::SessionData::put_many` · framework/src/session/store.rs:503
  - [ ] fn `suprnova::SessionData::forget_many` · framework/src/session/store.rs:514
  - [ ] fn `suprnova::SessionData::now` · framework/src/session/store.rs:525
  - [ ] fn `suprnova::SessionData::reflash` · framework/src/session/store.rs:534
  - [ ] fn `suprnova::SessionData::keep` · framework/src/session/store.rs:555
  - [ ] fn `suprnova::SessionData::flash_input` · framework/src/session/store.rs:574
  - [ ] fn `suprnova::SessionData::old_input` · framework/src/session/store.rs:582
  - [ ] fn `suprnova::SessionData::get_old_input` · framework/src/session/store.rs:599
  - [ ] fn `suprnova::SessionData::has_old_input` · framework/src/session/store.rs:608
  - [ ] fn `suprnova::SessionData::pull_errors_flash` · framework/src/session/store.rs:631
  - [ ] fn `suprnova::SessionData::previous_url` · framework/src/session/store.rs:675
  - [ ] fn `suprnova::SessionData::set_previous_url` · framework/src/session/store.rs:682
  - [ ] fn `suprnova::SessionData::previous_route` · framework/src/session/store.rs:688
  - [ ] fn `suprnova::SessionData::set_previous_route` · framework/src/session/store.rs:694
  - [ ] fn `suprnova::SessionData::has_previous_uri` · framework/src/session/store.rs:701
  - [ ] fn `suprnova::SessionData::password_confirmed` · framework/src/session/store.rs:710
  - [ ] fn `suprnova::SessionData::password_confirmed_at` · framework/src/session/store.rs:717
- [ ] enum `suprnova::SessionMigrationError` · framework/src/session/store.rs:756 (also `suprnova::session::SessionMigrationError`, `suprnova::session::store::SessionMigrationError`)
  - Variants: `RolledBack`, `OutcomeUnknown`
- [ ] trait `suprnova::SessionStore` · framework/src/session/store.rs:782 (also `suprnova::session::SessionStore`, `suprnova::session::store::SessionStore`)
  - Implemented here by: `DatabaseSessionDriver`
  - [ ] fn `suprnova::SessionStore::read` · framework/src/session/store.rs:786 (required)
  - [ ] fn `suprnova::SessionStore::write` · framework/src/session/store.rs:791 (required)
  - [ ] fn `suprnova::SessionStore::migrate_two_factor_session` · framework/src/session/store.rs:809 (provided)
  - [ ] fn `suprnova::SessionStore::destroy` · framework/src/session/store.rs:820 (required)
  - [ ] fn `suprnova::SessionStore::destroy_for_user` · framework/src/session/store.rs:828 (required)
  - [ ] fn `suprnova::SessionStore::gc` · framework/src/session/store.rs:833 (required)

### `suprnova::session`

- [ ] fn `suprnova::destroy_all_for_user` · framework/src/session/mod.rs:87 (also `suprnova::session::destroy_all_for_user`)
