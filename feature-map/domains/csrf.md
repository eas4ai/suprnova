# Feature map: `manual/csrf.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 19 checked.

## Rust API: suprnova

### `suprnova::csrf::middleware`

- [ ] struct `suprnova::CsrfMiddleware` · framework/src/csrf/middleware.rs:190 (also `suprnova::csrf::CsrfMiddleware`, `suprnova::csrf::middleware::CsrfMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::CsrfMiddleware::new` · framework/src/csrf/middleware.rs:233
  - [ ] fn `suprnova::CsrfMiddleware::except` · framework/src/csrf/middleware.rs:277
  - [ ] fn `suprnova::CsrfMiddleware::except_method` · framework/src/csrf/middleware.rs:304
  - [ ] fn `suprnova::CsrfMiddleware::allow_same_site` · framework/src/csrf/middleware.rs:319
  - [ ] fn `suprnova::CsrfMiddleware::origin_only` · framework/src/csrf/middleware.rs:337
  - [ ] fn `suprnova::CsrfMiddleware::with_origin_policy` · framework/src/csrf/middleware.rs:345
  - [ ] fn `suprnova::CsrfMiddleware::without_xsrf_cookie` · framework/src/csrf/middleware.rs:355
  - [ ] fn `suprnova::CsrfMiddleware::xsrf_cookie_name` · framework/src/csrf/middleware.rs:361
  - [ ] fn `suprnova::CsrfMiddleware::xsrf_cookie_path` · framework/src/csrf/middleware.rs:367
  - [ ] fn `suprnova::CsrfMiddleware::xsrf_cookie_domain` · framework/src/csrf/middleware.rs:373
  - [ ] fn `suprnova::CsrfMiddleware::xsrf_cookie_secure` · framework/src/csrf/middleware.rs:381
  - [ ] fn `suprnova::CsrfMiddleware::xsrf_cookie_same_site` · framework/src/csrf/middleware.rs:387
  - [ ] fn `suprnova::CsrfMiddleware::xsrf_cookie_lifetime` · framework/src/csrf/middleware.rs:395
  - [ ] fn `suprnova::CsrfMiddleware::with_session_config` · framework/src/csrf/middleware.rs:434
- [ ] enum `suprnova::OriginPolicy` · framework/src/csrf/middleware.rs:31 (also `suprnova::csrf::OriginPolicy`, `suprnova::csrf::middleware::OriginPolicy`)
  - Variants: `Disabled`, `SameOriginOnly`, `AllowSameSite`, `OriginOnly`

### `suprnova::csrf`

- [ ] fn `suprnova::csrf_field` · framework/src/csrf/mod.rs:105 (also `suprnova::csrf::csrf_field`)
- [ ] fn `suprnova::csrf_meta_tag` · framework/src/csrf/mod.rs:89 (also `suprnova::csrf::csrf_meta_tag`)
- [ ] fn `suprnova::csrf_token` · framework/src/csrf/mod.rs:75 (also `suprnova::csrf::csrf_token`)
