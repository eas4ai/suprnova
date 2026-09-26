# Feature map: `manual/errors.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 52 checked.

## Rust API: suprnova

### `suprnova::error`

- [ ] fn `suprnova::render_error_chain` · framework/src/error.rs:1419 (also `suprnova::error::render_error_chain`)
- [ ] struct `suprnova::AppError` · framework/src/error.rs:67 (also `suprnova::error::AppError`, `suprnova::prelude::AppError`)
  - Implements: `suprnova::HttpError`
  - [ ] fn `suprnova::AppError::new` · framework/src/error.rs:74
  - [ ] fn `suprnova::AppError::status` · framework/src/error.rs:82
  - [ ] fn `suprnova::AppError::not_found` · framework/src/error.rs:88
  - [ ] fn `suprnova::AppError::bad_request` · framework/src/error.rs:93
  - [ ] fn `suprnova::AppError::unauthorized` · framework/src/error.rs:98
  - [ ] fn `suprnova::AppError::forbidden` · framework/src/error.rs:103
  - [ ] fn `suprnova::AppError::unprocessable` · framework/src/error.rs:108
  - [ ] fn `suprnova::AppError::conflict` · framework/src/error.rs:113
- [ ] struct `suprnova::ValidationErrors` · framework/src/error.rs:172 (also `suprnova::error::ValidationErrors`)
  - Public fields: `errors`
  - [ ] fn `suprnova::ValidationErrors::new` · framework/src/error.rs:179
  - [ ] fn `suprnova::ValidationErrors::add` · framework/src/error.rs:191
  - [ ] fn `suprnova::ValidationErrors::add_to_bag` · framework/src/error.rs:205
  - [ ] fn `suprnova::ValidationErrors::is_empty` · framework/src/error.rs:216
  - [ ] fn `suprnova::ValidationErrors::into_result` · framework/src/error.rs:237
  - [ ] fn `suprnova::ValidationErrors::from_validator` · framework/src/error.rs:294
  - [ ] fn `suprnova::ValidationErrors::messages_for` · framework/src/error.rs:395
  - [ ] fn `suprnova::ValidationErrors::to_json` · framework/src/error.rs:407
  - [ ] fn `suprnova::ValidationErrors::retain_fields` · framework/src/error.rs:432
- [ ] enum `suprnova::FrameworkError` · framework/src/error.rs:790 (also `suprnova::error::FrameworkError`, `suprnova::prelude::FrameworkError`)
  - Variants: `ServiceNotFound`, `ParamError`, `ValidationError`, `Database`, `Internal`, `Domain`, `Validation`, `Unauthorized`, `ModelNotFound`, `ParamParse`, `UnsupportedMediaType`, `PrecognitionSuccess`, `PrecognitionFailure`, `AlreadyReported`, `RateLimited`, `External`
  - [ ] fn `suprnova::FrameworkError::service_not_found` · framework/src/error.rs:955
  - [ ] fn `suprnova::FrameworkError::param` · framework/src/error.rs:962
  - [ ] fn `suprnova::FrameworkError::validation` · framework/src/error.rs:969
  - [ ] fn `suprnova::FrameworkError::database` · framework/src/error.rs:977
  - [ ] fn `suprnova::FrameworkError::internal` · framework/src/error.rs:982
  - [ ] fn `suprnova::FrameworkError::silent` · framework/src/error.rs:998
  - [ ] fn `suprnova::FrameworkError::is_silent` · framework/src/error.rs:1007
  - [ ] fn `suprnova::FrameworkError::domain` · framework/src/error.rs:1012
  - [ ] fn `suprnova::FrameworkError::from_http_error` · framework/src/error.rs:1055
  - [ ] fn `suprnova::FrameworkError::from_external` · framework/src/error.rs:1070
  - [ ] fn `suprnova::FrameworkError::from_external_with` · framework/src/error.rs:1088
  - [ ] fn `suprnova::FrameworkError::external_source` · framework/src/error.rs:1105
  - [ ] fn `suprnova::FrameworkError::bad_request` · framework/src/error.rs:1113
  - [ ] fn `suprnova::FrameworkError::status_code` · framework/src/error.rs:1121
  - [ ] fn `suprnova::FrameworkError::rate_limited` · framework/src/error.rs:1147
  - [ ] fn `suprnova::FrameworkError::retry_after` · framework/src/error.rs:1160
  - [ ] fn `suprnova::FrameworkError::validation_errors` · framework/src/error.rs:1168
  - [ ] fn `suprnova::FrameworkError::from_unique_violation` · framework/src/error.rs:1216
  - [ ] fn `suprnova::FrameworkError::model_not_found` · framework/src/error.rs:1235
  - [ ] fn `suprnova::FrameworkError::param_parse` · framework/src/error.rs:1242
  - [ ] fn `suprnova::FrameworkError::not_found` · framework/src/error.rs:1250
  - [ ] fn `suprnova::FrameworkError::message` · framework/src/error.rs:1263
  - [ ] fn `suprnova::FrameworkError::field` · framework/src/error.rs:1287
  - [ ] fn `suprnova::FrameworkError::context` · framework/src/error.rs:1326
  - [ ] fn `suprnova::FrameworkError::into_json_api_response` · framework/src/resources/errors.rs:30
- [ ] trait `suprnova::HttpError` · framework/src/error.rs:36 (also `suprnova::error::HttpError`, `suprnova::prelude::HttpError`)
  - Implemented here by: `AppError`
  - [ ] fn `suprnova::HttpError::status_code` · framework/src/error.rs:38 (provided)
  - [ ] fn `suprnova::HttpError::error_message` · framework/src/error.rs:43 (provided)

### `suprnova::http::abort`

- [ ] fn `suprnova::abort_if` · framework/src/http/abort.rs:49 (also `suprnova::http::abort::abort_if`, `suprnova::http::abort_if`)
- [ ] fn `suprnova::abort_unless` · framework/src/http/abort.rs:63 (also `suprnova::http::abort::abort_unless`, `suprnova::http::abort_unless`)
- [ ] fn `suprnova::abort_with` · framework/src/http/abort.rs:37 (also `suprnova::http::abort::abort`, `suprnova::http::abort_with`)
