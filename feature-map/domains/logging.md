# Feature map: `manual/logging.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 14 checked.

## Rust API: suprnova

### `suprnova::logging::config`

- [ ] struct `suprnova::LogConfig` · framework/src/logging/config.rs:17 (also `suprnova::logging::LogConfig`, `suprnova::logging::config::LogConfig`)
  - Public fields: `level`, `format`
  - [ ] fn `suprnova::LogConfig::from_env` · framework/src/logging/config.rs:36
- [ ] enum `suprnova::LogFormat` · framework/src/logging/config.rs:8 (also `suprnova::logging::LogFormat`, `suprnova::logging::config::LogFormat`)
  - Variants: `Pretty`, `Json`

### `suprnova::logging::init`

- [ ] fn `suprnova::init_subscriber` · framework/src/logging/init.rs:57 (also `suprnova::logging::init::init_subscriber`, `suprnova::logging::init_subscriber`)

### `suprnova::logging::request_id`

- [ ] fn `suprnova::current_request_id` · framework/src/logging/request_id.rs:58 (also `suprnova::logging::current_request_id`, `suprnova::logging::request_id::current_request_id`)
- [ ] fn `suprnova::spawn_with_request_id` · framework/src/logging/request_id.rs:80 (also `suprnova::logging::request_id::spawn_with_request_id`, `suprnova::logging::spawn_with_request_id`)
- [ ] struct `suprnova::RequestId` · framework/src/logging/request_id.rs:16 (also `suprnova::logging::RequestId`, `suprnova::logging::request_id::RequestId`)
  - [ ] fn `suprnova::RequestId::new` · framework/src/logging/request_id.rs:20
  - [ ] fn `suprnova::RequestId::from_string` · framework/src/logging/request_id.rs:26
  - [ ] fn `suprnova::RequestId::as_str` · framework/src/logging/request_id.rs:31
- [ ] struct `suprnova::RequestIdMiddleware` · framework/src/logging/request_id.rs:104 (also `suprnova::logging::RequestIdMiddleware`, `suprnova::logging::request_id::RequestIdMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::RequestIdMiddleware::new` · framework/src/logging/request_id.rs:116
  - [ ] fn `suprnova::RequestIdMiddleware::with_id` · framework/src/logging/request_id.rs:127
- [ ] static `suprnova::logging::REQUEST_ID` · framework/src/logging/request_id.rs:48 (also `suprnova::logging::request_id::REQUEST_ID`)
