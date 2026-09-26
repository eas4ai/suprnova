# Feature map: `manual/installation.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 34 checked.

## Configuration

### `suprnova-magnetar`

- [ ] feature `suprnova-magnetar/password` · crates/suprnova-magnetar/Cargo.toml:18 (default)
- [ ] feature `suprnova-magnetar/email-verification` · crates/suprnova-magnetar/Cargo.toml:19 (off by default)
- [ ] feature `suprnova-magnetar/password-management` · crates/suprnova-magnetar/Cargo.toml:20 (off by default)
- [ ] feature `suprnova-magnetar/session-management` · crates/suprnova-magnetar/Cargo.toml:21 (off by default)
- [ ] feature `suprnova-magnetar/magic-link` · crates/suprnova-magnetar/Cargo.toml:22 (off by default)
- [ ] feature `suprnova-magnetar/passkey` · crates/suprnova-magnetar/Cargo.toml:23 (off by default)
  - enables `dep:webauthn-rs`, `dep:uuid`
- [ ] feature `suprnova-magnetar/two-factor` · crates/suprnova-magnetar/Cargo.toml:24 (off by default)
  - enables `dep:totp-rs`
- [ ] feature `suprnova-magnetar/oauth` · crates/suprnova-magnetar/Cargo.toml:25 (off by default)
  - enables `dep:jsonwebtoken`
- [ ] feature `suprnova-magnetar/oauth-apple` · crates/suprnova-magnetar/Cargo.toml:26 (off by default)
  - enables `oauth`, `dep:apple`, `dep:jsonwebtoken`, `dep:p256`
- [ ] feature `suprnova-magnetar/oauth-google` · crates/suprnova-magnetar/Cargo.toml:27 (off by default)
  - enables `oauth`
- [ ] feature `suprnova-magnetar/oauth-facebook` · crates/suprnova-magnetar/Cargo.toml:28 (off by default)
  - enables `oauth`
- [ ] feature `suprnova-magnetar/oauth-x` · crates/suprnova-magnetar/Cargo.toml:29 (off by default)
  - enables `oauth`
- [ ] feature `suprnova-magnetar/oauth-tiktok` · crates/suprnova-magnetar/Cargo.toml:30 (off by default)
  - enables `oauth`
- [ ] feature `suprnova-magnetar/device-authorization` · crates/suprnova-magnetar/Cargo.toml:31 (off by default)
  - enables `oauth`
- [ ] feature `suprnova-magnetar/seaorm-sqlite` · crates/suprnova-magnetar/Cargo.toml:32 (off by default)
  - enables `sea-orm/sqlx-sqlite`, `sea-orm/runtime-tokio-rustls`
- [ ] feature `suprnova-magnetar/seaorm-postgres` · crates/suprnova-magnetar/Cargo.toml:33 (off by default)
  - enables `sea-orm/sqlx-postgres`, `sea-orm/runtime-tokio-rustls`
- [ ] feature `suprnova-magnetar/seaorm-mysql` · crates/suprnova-magnetar/Cargo.toml:34 (off by default)
  - enables `sea-orm/sqlx-mysql`, `sea-orm/runtime-tokio-rustls`
- [ ] feature `suprnova-magnetar/migration` · crates/suprnova-magnetar/Cargo.toml:35 (off by default)
- [ ] feature `suprnova-magnetar/redis` · crates/suprnova-magnetar/Cargo.toml:36 (off by default)
  - enables `dep:redis`

### `suprnova`

- [ ] feature `suprnova/testing` · framework/Cargo.toml:230 (default)
- [ ] feature `suprnova/magnetar-oauth` · framework/Cargo.toml:231 (default)
  - enables `suprnova-magnetar/oauth`, `suprnova-magnetar/oauth-apple`, `suprnova-magnetar/oauth-facebook`, `suprnova-magnetar/oauth-google`, `suprnova-magnetar/oauth-tiktok`, `suprnova-magnetar/oauth-x`
- [ ] feature `suprnova/filesystem` · framework/Cargo.toml:241 (default)
  - enables `dep:opendal`
- [ ] feature `suprnova/filesystem-azure` · framework/Cargo.toml:269 (off by default)
  - enables `filesystem`, `opendal/services-azblob`
- [ ] feature `suprnova/filesystem-gcs` · framework/Cargo.toml:270 (off by default)
  - enables `filesystem`, `opendal/services-gcs`
- [ ] feature `suprnova/media` · framework/Cargo.toml:281 (default)
  - enables `dep:oxideav-core`, `dep:oxideav-image-filter`, `dep:oxideav-pixfmt`, `dep:oxideav-mjpeg`, `dep:oxideav-png`, `dep:oxideav-webp`, `dep:oxideav-gif`, `dep:oxideav-bmp`
- [ ] feature `suprnova/database-sqlite` · framework/Cargo.toml:292 (default)
  - enables `sea-orm/sqlx-sqlite`, `sea-orm-migration/sqlx-sqlite`, `suprnova-magnetar/seaorm-sqlite`
- [ ] feature `suprnova/database-postgres` · framework/Cargo.toml:297 (default)
  - enables `sea-orm/sqlx-postgres`, `sea-orm-migration/sqlx-postgres`, `suprnova-magnetar/seaorm-postgres`
- [ ] feature `suprnova/database-mysql` · framework/Cargo.toml:302 (default)
  - enables `sea-orm/sqlx-mysql`, `sea-orm-migration/sqlx-mysql`, `suprnova-magnetar/seaorm-mysql`
- [ ] feature `suprnova/vector-mariadb` · framework/Cargo.toml:308 (default)
  - enables `dep:sqlx`, `sqlx/mysql`, `database-mysql`
- [ ] feature `suprnova/web-push` · framework/Cargo.toml:311 (default)
  - enables `dep:suprnova-web-push`
- [ ] feature `suprnova/localization` · framework/Cargo.toml:317 (default)
  - enables `dep:fluent-bundle`, `dep:fluent-syntax`, `dep:fluent-langneg`, `dep:unic-langid`, `dep:intl-memoizer`, `dep:icu_decimal`, `dep:icu_datetime`, `dep:icu_list`, `dep:icu_calendar`, `dep:icu_locale_core`, `dep:icu_experimental`, `dep:fixed_decimal`, `dep:tinystr`, `dep:writeable`
- [ ] feature `suprnova/otel` · framework/Cargo.toml:333 (off by default)
  - enables `dep:opentelemetry`, `dep:opentelemetry_sdk`, `dep:opentelemetry-otlp`, `dep:opentelemetry-appender-tracing`, `dep:opentelemetry-http`, `dep:opentelemetry-semantic-conventions`, `dep:tracing-opentelemetry`
- [ ] feature `suprnova/broadcasting-fanout` · framework/Cargo.toml:346 (off by default)
- [ ] feature `suprnova/vector-pinecone` · framework/Cargo.toml:359 (off by default)
