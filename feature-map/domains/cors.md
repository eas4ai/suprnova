# Feature map: `manual/cors.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 23 checked.

## Rust API: suprnova

### `suprnova::cors`

- [ ] struct `suprnova::CorsConfig` · framework/src/cors/mod.rs:146 (also `suprnova::cors::CorsConfig`)
  - [ ] fn `suprnova::CorsConfig::allow_origins` · framework/src/cors/mod.rs:244
  - [ ] fn `suprnova::CorsConfig::any_origin` · framework/src/cors/mod.rs:266
  - [ ] fn `suprnova::CorsConfig::paths` · framework/src/cors/mod.rs:303
  - [ ] fn `suprnova::CorsConfig::allow_origin_patterns` · framework/src/cors/mod.rs:335
  - [ ] fn `suprnova::CorsConfig::skip_when` · framework/src/cors/mod.rs:368
  - [ ] fn `suprnova::CorsConfig::methods` · framework/src/cors/mod.rs:377
  - [ ] fn `suprnova::CorsConfig::allow_headers` · framework/src/cors/mod.rs:388
  - [ ] fn `suprnova::CorsConfig::allow_any_headers` · framework/src/cors/mod.rs:398
  - [ ] fn `suprnova::CorsConfig::expose_headers` · framework/src/cors/mod.rs:405
  - [ ] fn `suprnova::CorsConfig::allow_credentials` · framework/src/cors/mod.rs:427
  - [ ] fn `suprnova::CorsConfig::supports_credentials` · framework/src/cors/mod.rs:447
  - [ ] fn `suprnova::CorsConfig::allowed_methods` · framework/src/cors/mod.rs:453
  - [ ] fn `suprnova::CorsConfig::allowed_headers` · framework/src/cors/mod.rs:463
  - [ ] fn `suprnova::CorsConfig::exposed_headers` · framework/src/cors/mod.rs:473
  - [ ] fn `suprnova::CorsConfig::allowed_origins_patterns` · framework/src/cors/mod.rs:483
  - [ ] fn `suprnova::CorsConfig::max_age` · framework/src/cors/mod.rs:493
  - [ ] fn `suprnova::CorsConfig::max_age_secs` · framework/src/cors/mod.rs:501
- [ ] struct `suprnova::CorsMiddleware` · framework/src/cors/mod.rs:578 (also `suprnova::cors::CorsMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::CorsMiddleware::new` · framework/src/cors/mod.rs:584
- [ ] enum `suprnova::AllowedHeaders` · framework/src/cors/mod.rs:131 (also `suprnova::cors::AllowedHeaders`)
  - Variants: `Any`, `List`
- [ ] enum `suprnova::AllowedOrigins` · framework/src/cors/mod.rs:119 (also `suprnova::cors::AllowedOrigins`)
  - Variants: `Any`, `List`
- [ ] type `suprnova::cors::SkipPredicate` · framework/src/cors/mod.rs:115
