# Feature map: `manual/http-tests.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 36 checked.

## Rust API: suprnova

### `suprnova::testing::inertia` (private module; items are public through re-exports)

- [ ] struct `suprnova::testing::AssertableInertia` · framework/src/testing/inertia.rs:71
  - [ ] fn `suprnova::testing::AssertableInertia::from_response` · framework/src/testing/inertia.rs:100
  - [ ] fn `suprnova::testing::AssertableInertia::with_reload` · framework/src/testing/inertia.rs:176
  - [ ] fn `suprnova::testing::AssertableInertia::component` · framework/src/testing/inertia.rs:186
  - [ ] fn `suprnova::testing::AssertableInertia::url` · framework/src/testing/inertia.rs:198
  - [ ] fn `suprnova::testing::AssertableInertia::version` · framework/src/testing/inertia.rs:214
  - [ ] fn `suprnova::testing::AssertableInertia::prop` · framework/src/testing/inertia.rs:229
  - [ ] fn `suprnova::testing::AssertableInertia::has` · framework/src/testing/inertia.rs:234
  - [ ] fn `suprnova::testing::AssertableInertia::missing` · framework/src/testing/inertia.rs:245
  - [ ] fn `suprnova::testing::AssertableInertia::where_` · framework/src/testing/inertia.rs:256
  - [ ] fn `suprnova::testing::AssertableInertia::count` · framework/src/testing/inertia.rs:272
  - [ ] fn `suprnova::testing::AssertableInertia::has_flash` · framework/src/testing/inertia.rs:293
  - [ ] fn `suprnova::testing::AssertableInertia::reload_only` · framework/src/testing/inertia.rs:322
  - [ ] fn `suprnova::testing::AssertableInertia::reload_except` · framework/src/testing/inertia.rs:344
  - [ ] fn `suprnova::testing::AssertableInertia::load_deferred_props` · framework/src/testing/inertia.rs:368
- [ ] struct `suprnova::testing::ReloadRequest` · framework/src/testing/inertia.rs:448
  - Public fields: `url`, `component`, `version`, `only`, `except`
  - [ ] fn `suprnova::testing::ReloadRequest::headers` · framework/src/testing/inertia.rs:472

### `suprnova::testing::response` (private module; items are public through re-exports)

- [ ] struct `suprnova::testing::TestResponse` · framework/src/testing/response.rs:19
  - [ ] fn `suprnova::testing::TestResponse::new` · framework/src/testing/response.rs:36
  - [ ] fn `suprnova::testing::TestResponse::with_session_store` · framework/src/testing/response.rs:59
  - [ ] fn `suprnova::testing::TestResponse::status` · framework/src/testing/response.rs:69
  - [ ] fn `suprnova::testing::TestResponse::header` · framework/src/testing/response.rs:74
  - [ ] fn `suprnova::testing::TestResponse::cookie` · framework/src/testing/response.rs:94
  - [ ] fn `suprnova::testing::TestResponse::body_text` · framework/src/testing/response.rs:102
  - [ ] fn `suprnova::testing::TestResponse::json` · framework/src/testing/response.rs:112
  - [ ] fn `suprnova::testing::TestResponse::assert_status` · framework/src/testing/response.rs:122
  - [ ] fn `suprnova::testing::TestResponse::assert_ok` · framework/src/testing/response.rs:134
  - [ ] fn `suprnova::testing::TestResponse::assert_redirect` · framework/src/testing/response.rs:148
  - [ ] fn `suprnova::testing::TestResponse::assert_json` · framework/src/testing/response.rs:174
  - [ ] fn `suprnova::testing::TestResponse::assert_json_path` · framework/src/testing/response.rs:188
  - [ ] fn `suprnova::testing::TestResponse::assert_json_count` · framework/src/testing/response.rs:206
  - [ ] fn `suprnova::testing::TestResponse::assert_see` · framework/src/testing/response.rs:229
  - [ ] fn `suprnova::testing::TestResponse::assert_header` · framework/src/testing/response.rs:238
  - [ ] fn `suprnova::testing::TestResponse::assert_cookie` · framework/src/testing/response.rs:250
  - [ ] fn `suprnova::testing::TestResponse::assert_session_has` · framework/src/testing/response.rs:278
  - [ ] fn `suprnova::testing::TestResponse::assert_inertia` · framework/src/testing/response.rs:342
