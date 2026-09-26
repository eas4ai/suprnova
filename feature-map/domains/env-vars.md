# Feature map: `manual/env-vars.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 156 checked.

## Configuration

### APP_

- [ ] env `APP_BASE_PATH` · framework/src/app/paths.rs:37 (suprnova)
- [ ] env `APP_BUILD_ID` · framework/src/render_cache/config.rs:634 (suprnova)
- [ ] env `APP_DEBUG` · framework/src/config/env.rs:394 (suprnova)
  - also read at framework/src/config/providers/app.rs:51, framework/src/config/providers/app.rs:83
- [ ] env `APP_ENV` · framework/src/config/env.rs:72 (suprnova, suprnova-cli)
  - also read at suprnova-cli/src/commands/migrate_fresh.rs:124
- [ ] env `APP_FALLBACK_LOCALE` · framework/src/localization/config.rs:54 (suprnova)
- [ ] env `APP_KEY` · framework/src/crypto/key.rs:27 (suprnova)
  - also read at framework/src/crypto/mod.rs:793
- [ ] env `APP_KEY_PREVIOUS` · framework/src/crypto/mod.rs:797 (suprnova)
- [ ] env `APP_LOCALE` · framework/src/localization/config.rs:52 (suprnova)
- [ ] env `APP_LOCALE_PARENTS` · framework/src/localization/config.rs:55 (suprnova)
- [ ] env `APP_NAME` · framework/src/auth/middleware.rs:321 (suprnova)
  - also read at framework/src/auth_flows/mod.rs:97, framework/src/auth_flows/two_factor/mod.rs:47, framework/src/config/providers/app.rs:68, framework/src/config/providers/app.rs:85
- [ ] env `APP_PREVIOUS_KEYS` · framework/src/crypto/mod.rs:798 (suprnova)
- [ ] env `APP_SECRET` · framework/src/config/env.rs:330 (suprnova)
  - also read at framework/src/config/env.rs:364
- [ ] env `APP_TRUSTED_PROXIES` · framework/src/config/providers/app.rs:140 (suprnova)
  - also read at framework/src/config/providers/app.rs:159
- [ ] env `APP_URL` · framework/src/config/providers/app.rs:71 (suprnova)
  - also read at framework/src/config/providers/app.rs:87, framework/src/routing/url.rs:304

### AUTH_

- [ ] env `AUTH_GUARD` · framework/src/auth/config.rs:123 (suprnova)

### CACHE_

- [ ] env `CACHE_DEFAULT_TTL` · framework/src/cache/config.rs:98 (suprnova)
- [ ] env `CACHE_DRIVER` · framework/src/cache/config.rs:90 (suprnova)
  - also read at framework/src/schedule/mod.rs:244

### DATABASE_

- [ ] env `DATABASE_URL` · framework/src/app/mod.rs:1047 (suprnova, suprnova-cli)
  - also read at framework/src/database/config.rs:139, suprnova-cli/src/commands/db_sync.rs:87

### DB_

- [ ] env `DB_ACQUIRE_TIMEOUT` · framework/src/database/config.rs:151 (suprnova)
- [ ] env `DB_CONNECT_TIMEOUT` · framework/src/database/config.rs:147 (suprnova)
- [ ] env `DB_IDLE_TIMEOUT` · framework/src/database/config.rs:149 (suprnova)
- [ ] env `DB_LOGGING` · framework/src/database/config.rs:148 (suprnova)
- [ ] env `DB_MAX_CONNECTIONS` · framework/src/database/config.rs:145 (suprnova)
- [ ] env `DB_MAX_LIFETIME` · framework/src/database/config.rs:150 (suprnova)
- [ ] env `DB_MIN_CONNECTIONS` · framework/src/database/config.rs:146 (suprnova)
- [ ] env `DB_PING_AFTER_IDLE` · framework/src/database/config.rs:153 (suprnova)
- [ ] env `DB_TEST_BEFORE_ACQUIRE` · framework/src/database/config.rs:152 (suprnova)

### EVENT_

- [ ] env `EVENT_MAX_CONCURRENCY` · framework/src/events/dispatcher.rs:112 (suprnova)

### HASH_

- [ ] env `HASH_DRIVER` · framework/src/hashing/config.rs:99 (suprnova)
- [ ] env `HASH_MEMORY` · framework/src/hashing/config.rs:117 (suprnova)
- [ ] env `HASH_ROUNDS` · framework/src/hashing/config.rs:107 (suprnova)
- [ ] env `HASH_THREADS` · framework/src/hashing/config.rs:135 (suprnova)
- [ ] env `HASH_TIME` · framework/src/hashing/config.rs:127 (suprnova)
- [ ] env `HASH_VERIFY` · framework/src/hashing/config.rs:145 (suprnova)

### HIBP_

- [ ] env `HIBP_TIMEOUT_SECS` · framework/src/validation/rule.rs:1125 (suprnova)

### IMAGE_

- [ ] env `IMAGE_DRIVER` · framework/src/media/mod.rs:115 (suprnova)
- [ ] env `IMAGE_MAGICK_BINARY` · framework/src/media/magick.rs:95 (suprnova)
- [ ] env `IMAGE_MAGICK_TIMEOUT_SECS` · framework/src/media/mod.rs:216 (suprnova)
- [ ] env `IMAGE_MAX_ALLOC_BYTES` · framework/src/media/mod.rs:212 (suprnova)
- [ ] env `IMAGE_MAX_DIMENSION` · framework/src/media/mod.rs:208 (suprnova)

### INERTIA_

- [ ] env `INERTIA_VITE_DEV_SERVER` · framework/src/inertia/config.rs:504 (suprnova)

### LIVE_

- [ ] env `LIVE_LEDGER_DRIVER` · framework/src/live/config.rs:92 (suprnova)
- [ ] env `LIVE_REDIS_PREFIX` · framework/src/live/config.rs:99 (suprnova)
- [ ] env `LIVE_REDIS_URL` · framework/src/live/config.rs:96 (suprnova)
  - also read at framework/src/render_cache/providers/redis.rs:108

### LOG_

- [ ] env `LOG_FORMAT` · framework/src/logging/config.rs:38 (suprnova)
- [ ] env `LOG_LEVEL` · framework/src/logging/config.rs:37 (suprnova)

### MAIL_

- [ ] env `MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION` · framework/src/mail/boot.rs:66 (suprnova)
- [ ] env `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION` · framework/src/mail/boot.rs:56 (suprnova)
- [ ] env `MAIL_DRIVER` · framework/src/mail/boot.rs:365 (suprnova)
- [ ] env `MAIL_FILE_PATH` · framework/src/mail/boot.rs:406 (suprnova)
- [ ] env `MAIL_FROM` · framework/src/auth_flows/mod.rs:82 (suprnova)
  - also read at framework/src/auth_flows/mod.rs:84
- [ ] env `MAIL_FROM_NAME` · framework/src/auth_flows/mail.rs:39 (suprnova)
- [ ] env `MAIL_MAILGUN_API_KEY` · framework/src/mail/boot.rs:529 (suprnova)
- [ ] env `MAIL_MAILGUN_DOMAIN` · framework/src/mail/boot.rs:532 (suprnova)
- [ ] env `MAIL_MAILGUN_ENDPOINT` · framework/src/mail/boot.rs:535 (suprnova)
- [ ] env `MAIL_POSTMARK_ENDPOINT` · framework/src/mail/boot.rs:496 (suprnova)
- [ ] env `MAIL_POSTMARK_TOKEN` · framework/src/mail/boot.rs:493 (suprnova)
- [ ] env `MAIL_RESEND_API_KEY` · framework/src/mail/boot.rs:542 (suprnova)
- [ ] env `MAIL_RESEND_ENDPOINT` · framework/src/mail/boot.rs:545 (suprnova)
- [ ] env `MAIL_SENDGRID_API_KEY` · framework/src/mail/boot.rs:517 (suprnova)
- [ ] env `MAIL_SENDGRID_ENDPOINT` · framework/src/mail/boot.rs:522 (suprnova)
- [ ] env `MAIL_SES_ACCESS_KEY` · framework/src/mail/boot.rs:503 (suprnova)
- [ ] env `MAIL_SES_ENDPOINT` · framework/src/mail/boot.rs:510 (suprnova)
- [ ] env `MAIL_SES_REGION` · framework/src/mail/boot.rs:509 (suprnova)
- [ ] env `MAIL_SES_SECRET_KEY` · framework/src/mail/boot.rs:506 (suprnova)
- [ ] env `MAIL_SMTP_ENCRYPTION` · framework/src/mail/boot.rs:445 (suprnova)
- [ ] env `MAIL_SMTP_HOST` · framework/src/mail/boot.rs:413 (suprnova)
- [ ] env `MAIL_SMTP_PASS` · framework/src/mail/boot.rs:419 (suprnova)
- [ ] env `MAIL_SMTP_PORT` · framework/src/mail/boot.rs:414 (suprnova)
- [ ] env `MAIL_SMTP_USER` · framework/src/mail/boot.rs:418 (suprnova)

### MAINTENANCE_

- [ ] env `MAINTENANCE_DRIVER` · framework/src/app/maintenance.rs:313 (suprnova)
  - also read at framework/src/app/mod.rs:1743

### NOWPAYMENTS_

- [ ] env `NOWPAYMENTS_API_KEY` · crates/suprnova-payments-nowpayments/src/lib.rs:140 (suprnova-payments-nowpayments)
- [ ] env `NOWPAYMENTS_ENVIRONMENT` · crates/suprnova-payments-nowpayments/src/lib.rs:129 (suprnova-payments-nowpayments)
- [ ] env `NOWPAYMENTS_IPN_CALLBACK_URL` · crates/suprnova-payments-nowpayments/src/lib.rs:142 (suprnova-payments-nowpayments)
- [ ] env `NOWPAYMENTS_IPN_SECRET` · crates/suprnova-payments-nowpayments/src/lib.rs:141 (suprnova-payments-nowpayments)

### OTEL_

- [ ] env `OTEL_EXPORTER_OTLP_ENDPOINT` · framework/src/telemetry/init.rs:54 (suprnova)
- [ ] env `OTEL_SDK_DISABLED` · framework/src/telemetry/init.rs:65 (suprnova)
- [ ] env `OTEL_SERVICE_NAME` · framework/src/telemetry/init.rs:62 (suprnova)
- [ ] env `OTEL_SERVICE_VERSION` · framework/src/telemetry/init.rs:63 (suprnova)

### PADDLE_

- [ ] env `PADDLE_API_KEY` · crates/suprnova-payments-paddle/src/lib.rs:117 (suprnova-payments-paddle)
  - also read at crates/suprnova-payments-paddle/src/lib.rs:118
- [ ] env `PADDLE_CLIENT_TOKEN` · crates/suprnova-payments-paddle/src/lib.rs:126 (suprnova-payments-paddle)
  - also read at crates/suprnova-payments-paddle/src/lib.rs:127
- [ ] env `PADDLE_ENVIRONMENT` · crates/suprnova-payments-paddle/src/lib.rs:130 (suprnova-payments-paddle)
- [ ] env `PADDLE_WEBHOOK_KEY` · crates/suprnova-payments-paddle/src/lib.rs:121 (suprnova-payments-paddle)
  - also read at crates/suprnova-payments-paddle/src/lib.rs:122

### PINECONE_

- [ ] env `PINECONE_API_KEY` · framework/src/vector/pinecone.rs:230 (suprnova)
- [ ] env `PINECONE_API_VERSION` · framework/src/vector/pinecone.rs:239 (suprnova)
- [ ] env `PINECONE_CONTROLLER_HOST` · framework/src/vector/pinecone.rs:234 (suprnova)

### PORTLESS_

- [ ] env `PORTLESS_STATE_DIR` · suprnova-cli/src/commands/dev_tls.rs:173 (suprnova-cli)

### QUEUE_

- [ ] env `QUEUE_DB_TABLE` · framework/src/queue/mod.rs:1355 (suprnova)
- [ ] env `QUEUE_DRIVER` · framework/src/queue/mod.rs:1293 (suprnova)
- [ ] env `QUEUE_FAILED_DB_TABLE` · framework/src/queue/mod.rs:1382 (suprnova)
- [ ] env `QUEUE_FAILOVER_CONNECTIONS` · framework/src/queue/mod.rs:1413 (suprnova)
- [ ] env `QUEUE_PAUSABLE` · framework/src/queue/mod.rs:1263 (suprnova)
- [ ] env `QUEUE_REDIS_CONSUMER` · framework/src/queue/mod.rs:1343 (suprnova)
- [ ] env `QUEUE_REDIS_GROUP` · framework/src/queue/mod.rs:1342 (suprnova)
- [ ] env `QUEUE_REDIS_STREAM` · framework/src/queue/mod.rs:1341 (suprnova)
- [ ] env `QUEUE_REDIS_URL` · framework/src/queue/mod.rs:1338 (suprnova)
- [ ] env `QUEUE_VISIBILITY_TIMEOUT_SECS` · framework/src/queue/mod.rs:1345 (suprnova)

### RATE_

- [ ] env `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION` · framework/src/rate_limit/mod.rs:128 (suprnova)
- [ ] env `RATE_LIMIT_DRIVER` · framework/src/rate_limit/mod.rs:251 (suprnova)
- [ ] env `RATE_LIMIT_PREFIX` · framework/src/rate_limit/mod.rs:279 (suprnova)
- [ ] env `RATE_LIMIT_REDIS_URL` · framework/src/rate_limit/mod.rs:277 (suprnova)

### REDIS_

- [ ] env `REDIS_COMMAND_RETRIES` · framework/src/redis_retry.rs:39 (suprnova)
- [ ] env `REDIS_PREFIX` · framework/src/cache/config.rs:97 (suprnova)
- [ ] env `REDIS_URL` · framework/src/cache/config.rs:96 (suprnova)
  - also read at framework/src/live/config.rs:97, framework/src/render_cache/config.rs:514

### REMEMBER_

- [ ] env `REMEMBER_LIFETIME` · framework/src/session/config.rs:186 (suprnova)

### RENDER_

- [ ] env `RENDER_CACHE_COORDINATOR` · framework/src/render_cache/config.rs:580 (suprnova)
- [ ] env `RENDER_CACHE_ENABLED` · framework/src/render_cache/config.rs:478 (suprnova)
  - also read at framework/src/render_cache/config.rs:620
- [ ] env `RENDER_CACHE_FAILURE` · framework/src/render_cache/config.rs:629 (suprnova)
- [ ] env `RENDER_CACHE_HINTS` · framework/src/render_cache/config.rs:607 (suprnova)
- [ ] env `RENDER_CACHE_L0_BYTES` · framework/src/render_cache/config.rs:624 (suprnova)
- [ ] env `RENDER_CACHE_L0_ENTRIES` · framework/src/render_cache/config.rs:623 (suprnova)
- [ ] env `RENDER_CACHE_L1` · framework/src/render_cache/config.rs:536 (suprnova)
- [ ] env `RENDER_CACHE_L1_BYTES` · framework/src/render_cache/config.rs:506 (suprnova)
- [ ] env `RENDER_CACHE_L1_DIR` · framework/src/render_cache/config.rs:523 (suprnova)
- [ ] env `RENDER_CACHE_LEASE_MS` · framework/src/render_cache/config.rs:507 (suprnova)
- [ ] env `RENDER_CACHE_MAX_WAITERS` · framework/src/render_cache/config.rs:509 (suprnova)
- [ ] env `RENDER_CACHE_PROFILE` · framework/src/render_cache/config.rs:494 (suprnova)
- [ ] env `RENDER_CACHE_REDIS_PREFIX` · framework/src/render_cache/config.rs:516 (suprnova)
- [ ] env `RENDER_CACHE_REDIS_URL` · framework/src/render_cache/config.rs:513 (suprnova)
  - also read at framework/src/render_cache/providers/redis.rs:104

### SCHEDULE_

- [ ] env `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION` · framework/src/schedule/mod.rs:145 (suprnova)

### SERVER_

- [ ] env `SERVER_HEADER_READ_TIMEOUT` · framework/src/config/providers/server.rs:138 (suprnova)
  - also read at framework/src/config/providers/server.rs:173
- [ ] env `SERVER_HEALTH_READINESS_TOKEN` · framework/src/config/providers/server.rs:141 (suprnova)
  - also read at framework/src/config/providers/server.rs:179
- [ ] env `SERVER_HOST` · framework/src/config/env.rs:295 (suprnova)
  - also read at framework/src/config/providers/server.rs:131, framework/src/config/providers/server.rs:153
- [ ] env `SERVER_MAX_BODY_SIZE` · framework/src/config/providers/server.rs:133 (suprnova)
  - also read at framework/src/config/providers/server.rs:161
- [ ] env `SERVER_MAX_CONNECTIONS` · framework/src/config/providers/server.rs:135 (suprnova)
  - also read at framework/src/config/providers/server.rs:166
- [ ] env `SERVER_PORT` · framework/src/config/env.rs:294 (suprnova, suprnova-cli)
  - also read at framework/src/config/providers/server.rs:156, framework/src/config/providers/server.rs:212, suprnova-cli/src/commands/dev_tls.rs:668, suprnova-cli/src/commands/serve.rs:988

### SESSION_

- [ ] env `SESSION_BLOCK` · framework/src/session/config.rs:190 (suprnova)
- [ ] env `SESSION_BLOCK_LOCK_SECONDS` · framework/src/session/config.rs:191 (suprnova)
- [ ] env `SESSION_BLOCK_WAIT_SECONDS` · framework/src/session/config.rs:194 (suprnova)
- [ ] env `SESSION_CONNECTION` · framework/src/session/config.rs:227 (suprnova)
- [ ] env `SESSION_COOKIE` · framework/src/session/config.rs:215 (suprnova)
- [ ] env `SESSION_COOKIE_PREFIX` · framework/src/config/mod.rs:86 (suprnova)
  - also read at framework/src/session/config.rs:181
- [ ] env `SESSION_DOMAIN` · framework/src/config/mod.rs:93 (suprnova)
  - also read at framework/src/session/config.rs:218
- [ ] env `SESSION_EXPIRE_ON_CLOSE` · framework/src/session/config.rs:184 (suprnova)
- [ ] env `SESSION_GC_INTERVAL` · framework/src/session/config.rs:171 (suprnova)
- [ ] env `SESSION_LIFETIME` · framework/src/session/config.rs:165 (suprnova)
- [ ] env `SESSION_PARTITIONED` · framework/src/session/config.rs:176 (suprnova)
- [ ] env `SESSION_PATH` · framework/src/config/mod.rs:95 (suprnova)
  - also read at framework/src/session/config.rs:217
- [ ] env `SESSION_SAME_SITE` · framework/src/session/config.rs:221 (suprnova)
- [ ] env `SESSION_SECURE` · framework/src/session/config.rs:175 (suprnova)
- [ ] env `SESSION_TOUCH_INTERVAL` · framework/src/session/config.rs:168 (suprnova)

### STRIPE_

- [ ] env `STRIPE_MANAGED_PAYMENTS` · crates/suprnova-payments-stripe/src/lib.rs:181 (suprnova-payments-stripe)
- [ ] env `STRIPE_PUBLISHABLE_KEY` · crates/suprnova-payments-stripe/src/lib.rs:171 (suprnova-payments-stripe)
  - also read at crates/suprnova-payments-stripe/src/lib.rs:172
- [ ] env `STRIPE_SECRET_KEY` · crates/suprnova-payments-stripe/src/lib.rs:166 (suprnova-payments-stripe)
  - also read at crates/suprnova-payments-stripe/src/lib.rs:167
- [ ] env `STRIPE_WEBHOOK_SIGNING_SECRET` · crates/suprnova-payments-stripe/src/lib.rs:176 (suprnova-payments-stripe)
  - also read at crates/suprnova-payments-stripe/src/lib.rs:177

### SUPRNOVA_

- [ ] env `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT` · framework/src/app/mod.rs:467 (suprnova)
- [ ] env `SUPRNOVA_FRONTEND` · framework/src/inertia/config.rs:165 (suprnova, suprnova-cli)
  - also read at suprnova-cli/src/templates/mod.rs:477
- [ ] env `SUPRNOVA_SSR_BUNDLE` · suprnova-cli/src/commands/ssr_start.rs:40 (suprnova-cli)
- [ ] env `SUPRNOVA_SSR_RUNTIME` · suprnova-cli/src/commands/ssr_start.rs:20 (suprnova-cli)
- [ ] env `SUPRNOVA_SSR_URL` · suprnova-cli/src/commands/ssr_check.rs:27 (suprnova-cli)

### VITE_

- [ ] env `VITE_PORT` · framework/src/inertia/config.rs:510 (suprnova, suprnova-cli)
  - also read at suprnova-cli/src/commands/serve.rs:989

### WORKFLOW_

- [ ] env `WORKFLOW_CONCURRENCY` · framework/src/workflow/config.rs:62 (suprnova)
- [ ] env `WORKFLOW_LOCK_TIMEOUT_SECS` · framework/src/workflow/config.rs:101 (suprnova)
- [ ] env `WORKFLOW_MAX_ATTEMPTS` · framework/src/workflow/config.rs:75 (suprnova)
- [ ] env `WORKFLOW_POLL_INTERVAL_MS` · framework/src/workflow/config.rs:115 (suprnova)
- [ ] env `WORKFLOW_RETRY_BACKOFF_SECS` · framework/src/workflow/config.rs:88 (suprnova)
