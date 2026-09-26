# Feature map: `manual/requests.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 135 checked.

## Rust API: suprnova

### Re-exported from other crates

- [ ] struct `suprnova::HeaderMap` re-exports `http::header::map::HeaderMap`
- [ ] struct `suprnova::Method` re-exports `http::method::Method`
- [ ] struct `suprnova::StatusCode` re-exports `http::status::StatusCode`
- [ ] struct `suprnova::Uri` re-exports `http::uri::Uri`
- [ ] struct `suprnova::view::HeaderName` re-exports `http::header::name::HeaderName`
- [ ] struct `suprnova::view::HeaderValue` re-exports `http::header::value::HeaderValue`
- [ ] struct `suprnova::view::StatusCode` re-exports `http::status::StatusCode`

### `suprnova::http::extract` (private module; items are public through re-exports)

- [ ] trait `suprnova::FromParam` · framework/src/http/extract.rs:73 (also `suprnova::http::FromParam`)
  - Implemented here by: `String`
  - [ ] fn `suprnova::FromParam::from_param` · framework/src/http/extract.rs:78 (required)
- [ ] trait `suprnova::FromRequest` · framework/src/http/extract.rs:40 (also `suprnova::http::FromRequest`)
  - Implemented here by: `Request`
  - [ ] fn `suprnova::FromRequest::from_request` · framework/src/http/extract.rs:45 (required)

### `suprnova::http::request` (private module; items are public through re-exports)

- [ ] struct `suprnova::Request` · framework/src/http/request.rs:35 (also `suprnova::http::Request`, `suprnova::prelude::Request`)
  - Implements: `suprnova::FromRequest`, `suprnova::InertiaRequestExt`
  - [ ] fn `suprnova::Request::new` · framework/src/http/request.rs:95
  - [ ] fn `suprnova::Request::with_params` · framework/src/http/request.rs:115
  - [ ] fn `suprnova::Request::with_route_pattern` · framework/src/http/request.rs:125
  - [ ] fn `suprnova::Request::with_peer_addr` · framework/src/http/request.rs:134
  - [ ] fn `suprnova::Request::with_trusted_proxies` · framework/src/http/request.rs:148
  - [ ] fn `suprnova::Request::with_auth_user_id` · framework/src/http/request.rs:159
  - [ ] fn `suprnova::Request::for_test` · framework/src/http/request.rs:175
  - [ ] fn `suprnova::Request::for_test_with_headers` · framework/src/http/request.rs:205
  - [ ] fn `suprnova::Request::live_tenant` · framework/src/http/request.rs:373
  - [ ] fn `suprnova::Request::auth_user_id` · framework/src/http/request.rs:391
  - [ ] fn `suprnova::Request::trusted_proxies` · framework/src/http/request.rs:399
  - [ ] fn `suprnova::Request::peer_is_trusted_proxy` · framework/src/http/request.rs:407
  - [ ] fn `suprnova::Request::method` · framework/src/http/request.rs:412
  - [ ] fn `suprnova::Request::path` · framework/src/http/request.rs:417
  - [ ] fn `suprnova::Request::query` · framework/src/http/request.rs:423
  - [ ] fn `suprnova::Request::uri` · framework/src/http/request.rs:428
  - [ ] fn `suprnova::Request::headers` · framework/src/http/request.rs:433
  - [ ] fn `suprnova::Request::param` · framework/src/http/request.rs:439
  - [ ] fn `suprnova::Request::params` · framework/src/http/request.rs:449
  - [ ] fn `suprnova::Request::all_route_params` · framework/src/http/request.rs:457
  - [ ] fn `suprnova::Request::header` · framework/src/http/request.rs:462
  - [ ] fn `suprnova::Request::content_type` · framework/src/http/request.rs:467
  - [ ] fn `suprnova::Request::is_inertia` · framework/src/http/request.rs:472
  - [ ] fn `suprnova::Request::cookies` · framework/src/http/request.rs:493
  - [ ] fn `suprnova::Request::cookie` · framework/src/http/request.rs:516
  - [ ] fn `suprnova::Request::has_header` · framework/src/http/request.rs:523
  - [ ] fn `suprnova::Request::bearer_token` · framework/src/http/request.rs:532
  - [ ] fn `suprnova::Request::is_method` · framework/src/http/request.rs:552
  - [ ] fn `suprnova::Request::ajax` · framework/src/http/request.rs:560
  - [ ] fn `suprnova::Request::pjax` · framework/src/http/request.rs:568
  - [ ] fn `suprnova::Request::prefetch` · framework/src/http/request.rs:582
  - [ ] fn `suprnova::Request::secure` · framework/src/http/request.rs:616
  - [ ] fn `suprnova::Request::scheme` · framework/src/http/request.rs:641
  - [ ] fn `suprnova::Request::ip` · framework/src/http/request.rs:670
  - [ ] fn `suprnova::Request::ips` · framework/src/http/request.rs:703
  - [ ] fn `suprnova::Request::user_agent` · framework/src/http/request.rs:737
  - [ ] fn `suprnova::Request::host` · framework/src/http/request.rs:746
  - [ ] fn `suprnova::Request::http_host` · framework/src/http/request.rs:763
  - [ ] fn `suprnova::Request::scheme_and_http_host` · framework/src/http/request.rs:782
  - [ ] fn `suprnova::Request::port` · framework/src/http/request.rs:796
  - [ ] fn `suprnova::Request::decoded_path` · framework/src/http/request.rs:823
  - [ ] fn `suprnova::Request::segments` · framework/src/http/request.rs:832
  - [ ] fn `suprnova::Request::segment` · framework/src/http/request.rs:842
  - [ ] fn `suprnova::Request::url` · framework/src/http/request.rs:855
  - [ ] fn `suprnova::Request::full_url` · framework/src/http/request.rs:868
  - [ ] fn `suprnova::Request::full_url_with_query` · framework/src/http/request.rs:877
  - [ ] fn `suprnova::Request::full_url_without_query` · framework/src/http/request.rs:887
  - [ ] fn `suprnova::Request::query_params` · framework/src/http/request.rs:912
  - [ ] fn `suprnova::Request::query_param` · framework/src/http/request.rs:937
  - [ ] fn `suprnova::Request::has_query` · framework/src/http/request.rs:949
  - [ ] fn `suprnova::Request::query_into` · framework/src/http/request.rs:957
  - [ ] fn `suprnova::Request::route_pattern` · framework/src/http/request.rs:966
  - [ ] fn `suprnova::Request::route_name` · framework/src/http/request.rs:974
  - [ ] fn `suprnova::Request::route_is` · framework/src/http/request.rs:982
  - [ ] fn `suprnova::Request::is` · framework/src/http/request.rs:993
  - [ ] fn `suprnova::Request::full_url_is` · framework/src/http/request.rs:1004
  - [ ] fn `suprnova::Request::is_json` · framework/src/http/request.rs:1012
  - [ ] fn `suprnova::Request::expects_json` · framework/src/http/request.rs:1023
  - [ ] fn `suprnova::Request::wants_json` · framework/src/http/request.rs:1029
  - [ ] fn `suprnova::Request::acceptable_content_types` · framework/src/http/request.rs:1043
  - [ ] fn `suprnova::Request::accepts` · framework/src/http/request.rs:1053
  - [ ] fn `suprnova::Request::prefers` · framework/src/http/request.rs:1079
  - [ ] fn `suprnova::Request::accepts_any_content_type` · framework/src/http/request.rs:1102
  - [ ] fn `suprnova::Request::accepts_json` · framework/src/http/request.rs:1113
  - [ ] fn `suprnova::Request::accepts_html` · framework/src/http/request.rs:1119
  - [ ] fn `suprnova::Request::inertia_version` · framework/src/http/request.rs:1149
  - [ ] fn `suprnova::Request::inertia_partial_component` · framework/src/http/request.rs:1154
  - [ ] fn `suprnova::Request::inertia_partial_data` · framework/src/http/request.rs:1159
  - [ ] fn `suprnova::Request::body_bytes` · framework/src/http/request.rs:1175
  - [ ] fn `suprnova::Request::body_bytes_with_cap` · framework/src/http/request.rs:1190
  - [ ] fn `suprnova::Request::buffer_body` · framework/src/http/request.rs:1258
  - [ ] fn `suprnova::Request::cached_body` · framework/src/http/request.rs:1282
  - [ ] fn `suprnova::Request::cached_form_field` · framework/src/http/request.rs:1305
  - [ ] fn `suprnova::Request::json` · framework/src/http/request.rs:1338
  - [ ] fn `suprnova::Request::form` · framework/src/http/request.rs:1361
  - [ ] fn `suprnova::Request::input` · framework/src/http/request.rs:1373
  - [ ] fn `suprnova::Request::into_parts` · framework/src/http/request.rs:1388
- [ ] struct `suprnova::http::RequestParts` · framework/src/http/request.rs:1412
  - Public fields: `params`, `content_type`
- [ ] enum `suprnova::http::BodyState` · framework/src/http/request.rs:21
  - Variants: `Streaming`, `Buffered`, `Consumed`

### `suprnova::http::trusted_proxies` (private module; items are public through re-exports)

- [ ] struct `suprnova::http::TrustedProxiesConfig` · framework/src/http/trusted_proxies.rs:69
  - [ ] fn `suprnova::http::TrustedProxiesConfig::empty` · framework/src/http/trusted_proxies.rs:76
  - [ ] fn `suprnova::http::TrustedProxiesConfig::with_ips` · framework/src/http/trusted_proxies.rs:84
  - [ ] fn `suprnova::http::TrustedProxiesConfig::is_empty` · framework/src/http/trusted_proxies.rs:98
  - [ ] fn `suprnova::http::TrustedProxiesConfig::trusts` · framework/src/http/trusted_proxies.rs:108
  - [ ] fn `suprnova::http::TrustedProxiesConfig::proxies` · framework/src/http/trusted_proxies.rs:115

### `suprnova::http::upload::validators`

- [ ] struct `suprnova::ImageFile` · framework/src/http/upload/validators.rs:115 (also `suprnova::http::upload::validators::ImageFile`)
  - Implements: `suprnova::http::upload::validators::UploadValidator`
- [ ] struct `suprnova::MaxSize` · framework/src/http/upload/validators.rs:83 (also `suprnova::http::upload::validators::MaxSize`)
  - Implements: `suprnova::http::upload::validators::UploadValidator`
- [ ] struct `suprnova::MimeType` · framework/src/http/upload/validators.rs:228 (also `suprnova::http::upload::validators::MimeType`)
  - Implements: `suprnova::http::upload::validators::UploadValidator`
- [ ] trait `suprnova::MimeAllowlist` · framework/src/http/upload/validators.rs:141 (also `suprnova::http::upload::validators::MimeAllowlist`)
  - [ ] fn `suprnova::MimeAllowlist::allowed` · framework/src/http/upload/validators.rs:143 (required)
- [ ] trait `suprnova::http::upload::validators::UploadValidator` · framework/src/http/upload/validators.rs:42
  - Implemented here by: `ImageFile`, `MaxSize`, `MimeType`
  - [ ] fn `suprnova::http::upload::validators::UploadValidator::validate_chunk` · framework/src/http/upload/validators.rs:55 (provided)
  - [ ] fn `suprnova::http::upload::validators::UploadValidator::validate_final` · framework/src/http/upload/validators.rs:67 (provided)

### `suprnova::http::upload`

- [ ] fn `suprnova::global_max_multipart_body_bytes` · framework/src/http/upload/mod.rs:89 (also `suprnova::http::upload::global_max_multipart_body_bytes`)
- [ ] fn `suprnova::global_max_multipart_parts` · framework/src/http/upload/mod.rs:159 (also `suprnova::http::upload::global_max_multipart_parts`)
- [ ] fn `suprnova::global_upload_spill_threshold` · framework/src/http/upload/mod.rs:116 (also `suprnova::http::upload::global_upload_spill_threshold`)
- [ ] fn `suprnova::parse_multipart_streaming` · framework/src/http/upload/mod.rs:930 (also `suprnova::http::upload::parse_multipart_streaming`)
- [ ] fn `suprnova::parse_multipart_streaming_with_cap` · framework/src/http/upload/mod.rs:900 (also `suprnova::http::upload::parse_multipart_streaming_with_cap`)
- [ ] fn `suprnova::parse_multipart_streaming_with_limits` · framework/src/http/upload/mod.rs:666 (also `suprnova::http::upload::parse_multipart_streaming_with_limits`)
- [ ] fn `suprnova::set_global_max_multipart_body_bytes` · framework/src/http/upload/mod.rs:82 (also `suprnova::http::upload::set_global_max_multipart_body_bytes`)
- [ ] fn `suprnova::set_global_max_multipart_parts` · framework/src/http/upload/mod.rs:152 (also `suprnova::http::upload::set_global_max_multipart_parts`)
- [ ] fn `suprnova::set_global_upload_spill_threshold` · framework/src/http/upload/mod.rs:109 (also `suprnova::http::upload::set_global_upload_spill_threshold`)
- [ ] fn `suprnova::upload_tempfiles_spilled_total` · framework/src/http/upload/mod.rs:175 (also `suprnova::http::upload::upload_tempfiles_spilled_total`)
- [ ] struct `suprnova::MultipartLimits` · framework/src/http/upload/mod.rs:187 (also `suprnova::http::upload::MultipartLimits`)
  - Public fields: `max_body_bytes`, `max_parts`, `spill_threshold`, `per_field_max_counts`
- [ ] struct `suprnova::MultipartPayload` · framework/src/http/upload/mod.rs:419 (also `suprnova::http::upload::MultipartPayload`)
  - Public fields: `fields`
- [ ] struct `suprnova::UploadedFile` · framework/src/http/upload/mod.rs:234 (also `suprnova::http::upload::UploadedFile`)
  - Public fields: `size`, `file_name`, `content_type`
  - [ ] fn `suprnova::UploadedFile::bytes` · framework/src/http/upload/mod.rs:311
  - [ ] fn `suprnova::UploadedFile::store_as` · framework/src/http/upload/mod.rs:342
  - [ ] fn `suprnova::UploadedFile::extension_from_magic` · framework/src/http/upload/mod.rs:411
- [ ] enum `suprnova::MultipartValue` · framework/src/http/upload/mod.rs:426 (also `suprnova::http::upload::MultipartValue`)
  - Variants: `File`, `Text`
- [ ] trait `suprnova::MultipartRequestHooks` · framework/src/http/upload/mod.rs:957 (also `suprnova::http::upload::MultipartRequestHooks`)
  - [ ] fn `suprnova::MultipartRequestHooks::authorize` · framework/src/http/upload/mod.rs:960 (provided)
  - [ ] fn `suprnova::MultipartRequestHooks::after_validation` · framework/src/http/upload/mod.rs:967 (provided)
- [ ] const `suprnova::DEFAULT_MAX_MULTIPART_BODY_BYTES` · framework/src/http/upload/mod.rs:46 (also `suprnova::http::upload::DEFAULT_MAX_MULTIPART_BODY_BYTES`)
- [ ] const `suprnova::DEFAULT_MAX_MULTIPART_PARTS` · framework/src/http/upload/mod.rs:134 (also `suprnova::http::upload::DEFAULT_MAX_MULTIPART_PARTS`)
- [ ] const `suprnova::DEFAULT_UPLOAD_SPILL_THRESHOLD` · framework/src/http/upload/mod.rs:52 (also `suprnova::http::upload::DEFAULT_UPLOAD_SPILL_THRESHOLD`)

### `suprnova::http`

- [ ] fn `suprnova::json` · framework/src/http/mod.rs:68 (also `suprnova::http::json`)
- [ ] fn `suprnova::text` · framework/src/http/mod.rs:63 (also `suprnova::http::text`)
- [ ] struct `suprnova::http::ParamError` · framework/src/http/mod.rs:33
  - Public fields: `param_name`

## Rust API: suprnova-macros

### `suprnova_macros`

- [ ] proc macro `suprnova_macros::MultipartRequest` · suprnova-macros/src/lib.rs:722 (re-exported as `suprnova::MultipartRequest`)
  - Form: derive `#[derive(MultipartRequest)]`
  - Helper attributes: `#[field]`, `#[multipart]`
  - [ ] argument `#[multipart(max_body_bytes)]` · suprnova-macros/src/multipart.rs:50
  - [ ] argument `#[multipart(custom_hooks)]` · suprnova-macros/src/multipart.rs:46
  - [ ] argument field `#[field(max_count = N)]` · suprnova-macros/src/multipart.rs:122
