# Feature map: `manual/frontend-inertia-responses.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 203 checked.

## Command line

### `suprnova` developer CLI (suprnova-cli)

- [ ] command `suprnova ssr:start` · suprnova-cli/src/main.rs:326
  - Launch the Inertia SSR worker in the foreground
- [ ] command `suprnova ssr:check` · suprnova-cli/src/main.rs:338
  - Verify the Inertia SSR worker is reachable

## Rust API: suprnova

### `suprnova::inertia::config` (private module; items are public through re-exports)

- [ ] struct `suprnova::InertiaConfig` · framework/src/inertia/config.rs:212 (also `suprnova::inertia::InertiaConfig`)
  - Public fields: `vite_dev_server`, `entry_point`, `version`, `development`, `frontend`, `default_title`, `encrypt_history_default`, `ssr`, `manifest_path`, `assets_base_url`, `with_all_errors`, `max_concurrent_resolvers`, `error_page`
  - [ ] fn `suprnova::InertiaConfig::new` · framework/src/inertia/config.rs:555
  - [ ] fn `suprnova::InertiaConfig::vite_dev_server` · framework/src/inertia/config.rs:560
  - [ ] fn `suprnova::InertiaConfig::entry_point` · framework/src/inertia/config.rs:567
  - [ ] fn `suprnova::InertiaConfig::version` · framework/src/inertia/config.rs:574
  - [ ] fn `suprnova::InertiaConfig::version_with` · framework/src/inertia/config.rs:612
  - [ ] fn `suprnova::InertiaConfig::production` · framework/src/inertia/config.rs:621
  - [ ] fn `suprnova::InertiaConfig::development` · framework/src/inertia/config.rs:631
  - [ ] fn `suprnova::InertiaConfig::frontend` · framework/src/inertia/config.rs:638
  - [ ] fn `suprnova::InertiaConfig::default_title` · framework/src/inertia/config.rs:647
  - [ ] fn `suprnova::InertiaConfig::encrypt_history` · framework/src/inertia/config.rs:654
  - [ ] fn `suprnova::InertiaConfig::ssr` · framework/src/inertia/config.rs:660
  - [ ] fn `suprnova::InertiaConfig::ssr_disabled` · framework/src/inertia/config.rs:667
  - [ ] fn `suprnova::InertiaConfig::ssr_timeout` · framework/src/inertia/config.rs:673
  - [ ] fn `suprnova::InertiaConfig::ssr_throw_on_error` · framework/src/inertia/config.rs:679
  - [ ] fn `suprnova::InertiaConfig::ssr_exclude` · framework/src/inertia/config.rs:685
  - [ ] fn `suprnova::InertiaConfig::ssr_max_response_bytes` · framework/src/inertia/config.rs:697
  - [ ] fn `suprnova::InertiaConfig::ssr_bundle_path` · framework/src/inertia/config.rs:708
  - [ ] fn `suprnova::InertiaConfig::ssr_ensure_bundle_exists` · framework/src/inertia/config.rs:718
  - [ ] fn `suprnova::InertiaConfig::on_ssr_error` · framework/src/inertia/config.rs:725
  - [ ] fn `suprnova::InertiaConfig::manifest_path` · framework/src/inertia/config.rs:739
  - [ ] fn `suprnova::InertiaConfig::assets_base_url` · framework/src/inertia/config.rs:758
  - [ ] fn `suprnova::InertiaConfig::max_concurrent_resolvers` · framework/src/inertia/config.rs:766
  - [ ] fn `suprnova::InertiaConfig::with_all_errors` · framework/src/inertia/config.rs:780
  - [ ] fn `suprnova::InertiaConfig::error_page` · framework/src/inertia/config.rs:817
  - [ ] fn `suprnova::InertiaConfig::url_resolver` · framework/src/inertia/config.rs:841
  - [ ] fn `suprnova::InertiaConfig::vite_manifest` · framework/src/inertia/config.rs:858
- [ ] struct `suprnova::SsrConfig` · framework/src/inertia/config.rs:342 (also `suprnova::inertia::SsrConfig`)
  - Public fields: `enabled`, `url`, `timeout`, `throw_on_error`, `excluded_paths`, `on_error`, `max_response_bytes`, `bundle_path`, `ensure_bundle_exists`
  - [ ] fn `suprnova::SsrConfig::is_path_excluded` · framework/src/inertia/config.rs:428
- [ ] enum `suprnova::Frontend` · framework/src/inertia/config.rs:150 (also `suprnova::inertia::Frontend`)
  - Variants: `Svelte`, `React`, `Vue`
  - [ ] fn `suprnova::Frontend::detect_from_env` · framework/src/inertia/config.rs:164
  - [ ] fn `suprnova::Frontend::default_entry_point` · framework/src/inertia/config.rs:174
  - [ ] fn `suprnova::Frontend::page_extensions` · framework/src/inertia/config.rs:186
  - [ ] fn `suprnova::Frontend::as_str` · framework/src/inertia/config.rs:195
- [ ] enum `suprnova::VersionResolver` · framework/src/inertia/config.rs:28 (also `suprnova::inertia::VersionResolver`)
  - Variants: `Static`, `Dynamic`, `Manifest`
  - [ ] fn `suprnova::VersionResolver::new` · framework/src/inertia/config.rs:43
  - [ ] fn `suprnova::VersionResolver::with` · framework/src/inertia/config.rs:49
  - [ ] fn `suprnova::VersionResolver::from_manifest` · framework/src/inertia/config.rs:75
  - [ ] fn `suprnova::VersionResolver::resolve` · framework/src/inertia/config.rs:80
- [ ] const `suprnova::MANIFEST_VERSION_FALLBACK` · framework/src/inertia/config.rs:115 (also `suprnova::inertia::MANIFEST_VERSION_FALLBACK`)

### `suprnova::inertia::conversion_middleware` (private module; items are public through re-exports)

- [ ] struct `suprnova::Inertia303Middleware` · framework/src/inertia/conversion_middleware.rs:22 (also `suprnova::inertia::Inertia303Middleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::Inertia303Middleware::new` · framework/src/inertia/conversion_middleware.rs:26

### `suprnova::inertia::encrypt_middleware` (private module; items are public through re-exports)

- [ ] struct `suprnova::EncryptHistoryMiddleware` · framework/src/inertia/encrypt_middleware.rs:28 (also `suprnova::inertia::EncryptHistoryMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::EncryptHistoryMiddleware::new` · framework/src/inertia/encrypt_middleware.rs:32

### `suprnova::inertia::error_page_middleware` (private module; items are public through re-exports)

- [ ] struct `suprnova::InertiaErrorPageMiddleware` · framework/src/inertia/error_page_middleware.rs:103 (also `suprnova::inertia::InertiaErrorPageMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::InertiaErrorPageMiddleware::new` · framework/src/inertia/error_page_middleware.rs:115

### `suprnova::inertia::facade` (private module; items are public through re-exports)

- [ ] struct `suprnova::Inertia` · framework/src/inertia/facade.rs:16 (also `suprnova::inertia::Inertia`)
  - [ ] fn `suprnova::Inertia::paginate` · framework/src/inertia/facade.rs:30
  - [ ] fn `suprnova::Inertia::data` · framework/src/inertia/facade.rs:47
  - [ ] fn `suprnova::Inertia::try_data` · framework/src/inertia/facade.rs:63
  - [ ] fn `suprnova::Inertia::install` · framework/src/inertia/facade.rs:172

### `suprnova::inertia::headers_middleware` (private module; items are public through re-exports)

- [ ] struct `suprnova::InertiaHeadersMiddleware` · framework/src/inertia/headers_middleware.rs:36 (also `suprnova::inertia::InertiaHeadersMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::InertiaHeadersMiddleware::new` · framework/src/inertia/headers_middleware.rs:40

### `suprnova::inertia::manifest` (private module; items are public through re-exports)

- [ ] struct `suprnova::ManifestEntry` · framework/src/inertia/manifest.rs:48 (also `suprnova::inertia::ManifestEntry`)
  - Public fields: `file`, `css`, `imports`, `is_entry`
- [ ] struct `suprnova::ResolvedAssets` · framework/src/inertia/manifest.rs:76 (also `suprnova::inertia::ResolvedAssets`)
  - Public fields: `js`, `css`, `preload`
- [ ] struct `suprnova::ViteManifest` · framework/src/inertia/manifest.rs:69 (also `suprnova::inertia::ViteManifest`)
  - [ ] fn `suprnova::ViteManifest::load` · framework/src/inertia/manifest.rs:87
  - [ ] fn `suprnova::ViteManifest::resolve_entry` · framework/src/inertia/manifest.rs:116

### `suprnova::inertia::prop` (private module; items are public through re-exports)

- [ ] struct `suprnova::DeferOptions` · framework/src/inertia/prop.rs:115 (also `suprnova::inertia::DeferOptions`)
  - [ ] fn `suprnova::DeferOptions::new` · framework/src/inertia/prop.rs:131
  - [ ] fn `suprnova::DeferOptions::group` · framework/src/inertia/prop.rs:137
  - [ ] fn `suprnova::DeferOptions::rescue` · framework/src/inertia/prop.rs:145
- [ ] struct `suprnova::OnceOptions` · framework/src/inertia/prop.rs:297 (also `suprnova::inertia::OnceOptions`)
  - [ ] fn `suprnova::OnceOptions::new` · framework/src/inertia/prop.rs:305
  - [ ] fn `suprnova::OnceOptions::as_key` · framework/src/inertia/prop.rs:311
  - [ ] fn `suprnova::OnceOptions::until` · framework/src/inertia/prop.rs:319
  - [ ] fn `suprnova::OnceOptions::fresh` · framework/src/inertia/prop.rs:327
- [ ] struct `suprnova::PartialFilter` · framework/src/inertia/prop.rs:1078 (also `suprnova::inertia::PartialFilter`)
  - Public fields: `matched`, `only`, `except`
  - [ ] fn `suprnova::PartialFilter::build` · framework/src/inertia/prop.rs:1092
  - [ ] fn `suprnova::PartialFilter::should_include_eager` · framework/src/inertia/prop.rs:1146
  - [ ] fn `suprnova::PartialFilter::should_include_optional` · framework/src/inertia/prop.rs:1180
  - [ ] fn `suprnova::PartialFilter::should_include` · framework/src/inertia/prop.rs:1213
  - [ ] fn `suprnova::PartialFilter::narrow` · framework/src/inertia/prop.rs:1265
- [ ] struct `suprnova::Prop` · framework/src/inertia/prop.rs:422 (also `suprnova::inertia::Prop`)
  - [ ] fn `suprnova::Prop::eager` · framework/src/inertia/prop.rs:570
  - [ ] fn `suprnova::Prop::absent` · framework/src/inertia/prop.rs:580
  - [ ] fn `suprnova::Prop::from_resolver` · framework/src/inertia/prop.rs:590
  - [ ] fn `suprnova::Prop::lazy` · framework/src/inertia/prop.rs:598
  - [ ] fn `suprnova::Prop::always` · framework/src/inertia/prop.rs:616
  - [ ] fn `suprnova::Prop::optional` · framework/src/inertia/prop.rs:626
  - [ ] fn `suprnova::Prop::defer` · framework/src/inertia/prop.rs:637
  - [ ] fn `suprnova::Prop::group` · framework/src/inertia/prop.rs:649
  - [ ] fn `suprnova::Prop::rescue` · framework/src/inertia/prop.rs:661
  - [ ] fn `suprnova::Prop::merge` · framework/src/inertia/prop.rs:670
  - [ ] fn `suprnova::Prop::prepend` · framework/src/inertia/prop.rs:677
  - [ ] fn `suprnova::Prop::deep_merge` · framework/src/inertia/prop.rs:684
  - [ ] fn `suprnova::Prop::merge_with_path` · framework/src/inertia/prop.rs:722
  - [ ] fn `suprnova::Prop::match_on` · framework/src/inertia/prop.rs:738
  - [ ] fn `suprnova::Prop::merge_strategy` · framework/src/inertia/prop.rs:746
  - [ ] fn `suprnova::Prop::once` · framework/src/inertia/prop.rs:764
  - [ ] fn `suprnova::Prop::as_key` · framework/src/inertia/prop.rs:775
  - [ ] fn `suprnova::Prop::until` · framework/src/inertia/prop.rs:786
  - [ ] fn `suprnova::Prop::fresh` · framework/src/inertia/prop.rs:795
  - [ ] fn `suprnova::Prop::scroll` · framework/src/inertia/prop.rs:818
  - [ ] fn `suprnova::Prop::scroll_wrap` · framework/src/inertia/prop.rs:842
  - [ ] fn `suprnova::Prop::visibility` · framework/src/inertia/prop.rs:850
  - [ ] fn `suprnova::Prop::is_always` · framework/src/inertia/prop.rs:855
  - [ ] fn `suprnova::Prop::is_optional` · framework/src/inertia/prop.rs:860
  - [ ] fn `suprnova::Prop::is_defer` · framework/src/inertia/prop.rs:866
  - [ ] fn `suprnova::Prop::is_absent` · framework/src/inertia/prop.rs:871
  - [ ] fn `suprnova::Prop::is_lazy` · framework/src/inertia/prop.rs:880
  - [ ] fn `suprnova::Prop::has_resolver` · framework/src/inertia/prop.rs:890
  - [ ] fn `suprnova::Prop::as_value` · framework/src/inertia/prop.rs:895
  - [ ] fn `suprnova::Prop::defer_group` · framework/src/inertia/prop.rs:903
  - [ ] fn `suprnova::Prop::rescues` · framework/src/inertia/prop.rs:909
  - [ ] fn `suprnova::Prop::merge_mode` · framework/src/inertia/prop.rs:914
  - [ ] fn `suprnova::Prop::match_on_fields` · framework/src/inertia/prop.rs:919
  - [ ] fn `suprnova::Prop::merge_paths` · framework/src/inertia/prop.rs:924
  - [ ] fn `suprnova::Prop::is_once` · framework/src/inertia/prop.rs:929
  - [ ] fn `suprnova::Prop::once_cache_key` · framework/src/inertia/prop.rs:935
  - [ ] fn `suprnova::Prop::once_expires_at` · framework/src/inertia/prop.rs:942
  - [ ] fn `suprnova::Prop::is_fresh` · framework/src/inertia/prop.rs:947
  - [ ] fn `suprnova::Prop::scroll_metadata` · framework/src/inertia/prop.rs:952
  - [ ] fn `suprnova::Prop::scroll_wrap_key` · framework/src/inertia/prop.rs:958
  - [ ] fn `suprnova::Prop::resolve` · framework/src/inertia/prop.rs:977
  - [ ] fn `suprnova::Prop::resolve_with_owner` · framework/src/inertia/prop.rs:1008
- [ ] struct `suprnova::ScrollMetadata` · framework/src/inertia/prop.rs:195 (also `suprnova::inertia::ScrollMetadata`)
  - Public fields: `page_name`, `previous_page`, `next_page`, `current_page`
  - [ ] fn `suprnova::ScrollMetadata::new` · framework/src/inertia/prop.rs:212
  - [ ] fn `suprnova::ScrollMetadata::current` · framework/src/inertia/prop.rs:222
  - [ ] fn `suprnova::ScrollMetadata::previous` · framework/src/inertia/prop.rs:228
  - [ ] fn `suprnova::ScrollMetadata::next` · framework/src/inertia/prop.rs:234
- [ ] enum `suprnova::MergeMode` · framework/src/inertia/prop.rs:358 (also `suprnova::inertia::MergeMode`)
  - Variants: `Append`, `Prepend`, `Deep`
- [ ] enum `suprnova::MergeStrategy` · framework/src/inertia/prop.rs:167 (also `suprnova::inertia::MergeStrategy`)
  - Variants: `Append`, `Prepend`, `Deep`
- [ ] enum `suprnova::Visibility` · framework/src/inertia/prop.rs:378 (also `suprnova::inertia::Visibility`)
  - Variants: `Standard`, `Always`, `Optional`, `Deferred`
- [ ] trait `suprnova::InertiaRequestExt` · framework/src/inertia/prop.rs:15 (also `suprnova::inertia::InertiaRequestExt`)
  - Implemented here by: `Request`
  - [ ] fn `suprnova::InertiaRequestExt::path` · framework/src/inertia/prop.rs:17 (required)
  - [ ] fn `suprnova::InertiaRequestExt::path_and_query` · framework/src/inertia/prop.rs:33 (provided)
  - [ ] fn `suprnova::InertiaRequestExt::header` · framework/src/inertia/prop.rs:37 (required)
  - [ ] fn `suprnova::InertiaRequestExt::is_inertia` · framework/src/inertia/prop.rs:39 (provided)
  - [ ] fn `suprnova::InertiaRequestExt::is_prefetch` · framework/src/inertia/prop.rs:49 (provided)
- [ ] trait `suprnova::MatchOnFields` · framework/src/inertia/prop.rs:515 (also `suprnova::inertia::MatchOnFields`)
  - Implemented here by: `String`, `Vec`
  - [ ] fn `suprnova::MatchOnFields::into_match_on_fields` · framework/src/inertia/prop.rs:517 (required)
- [ ] trait `suprnova::ProvidesScrollMetadata` · framework/src/inertia/prop.rs:267 (also `suprnova::inertia::ProvidesScrollMetadata`)
  - Implemented here by: `CursorPaginator`, `LengthAwarePaginator`, `Paginator`
  - [ ] fn `suprnova::ProvidesScrollMetadata::page_name` · framework/src/inertia/prop.rs:270 (required)
  - [ ] fn `suprnova::ProvidesScrollMetadata::previous_page` · framework/src/inertia/prop.rs:274 (required)
  - [ ] fn `suprnova::ProvidesScrollMetadata::next_page` · framework/src/inertia/prop.rs:278 (required)
  - [ ] fn `suprnova::ProvidesScrollMetadata::current_page` · framework/src/inertia/prop.rs:281 (required)
  - [ ] fn `suprnova::ProvidesScrollMetadata::scroll_metadata` · framework/src/inertia/prop.rs:284 (provided)
- [ ] type `suprnova::PropFuture` · framework/src/inertia/prop.rs:104 (also `suprnova::inertia::PropFuture`)
- [ ] type `suprnova::PropResolver` · framework/src/inertia/prop.rs:110 (also `suprnova::inertia::PropResolver`)

### `suprnova::inertia::response` (private module; items are public through re-exports)

- [ ] struct `suprnova::InertiaResponse` · framework/src/inertia/response.rs:103 (also `suprnova::inertia::InertiaResponse`)
  - [ ] fn `suprnova::InertiaResponse::new` · framework/src/inertia/response.rs:274
  - [ ] fn `suprnova::InertiaResponse::with_config` · framework/src/inertia/response.rs:307
  - [ ] fn `suprnova::InertiaResponse::title` · framework/src/inertia/response.rs:326
  - [ ] fn `suprnova::InertiaResponse::with` · framework/src/inertia/response.rs:335
  - [ ] fn `suprnova::InertiaResponse::always` · framework/src/inertia/response.rs:344
  - [ ] fn `suprnova::InertiaResponse::always_with` · framework/src/inertia/response.rs:358
  - [ ] fn `suprnova::InertiaResponse::lazy` · framework/src/inertia/response.rs:384
  - [ ] fn `suprnova::InertiaResponse::prop_lazy_with_owner` · framework/src/inertia/response.rs:414
  - [ ] fn `suprnova::InertiaResponse::prop` · framework/src/inertia/response.rs:448
  - [ ] fn `suprnova::InertiaResponse::from_data_props` · framework/src/inertia/response.rs:460
  - [ ] fn `suprnova::InertiaResponse::optional` · framework/src/inertia/response.rs:481
  - [ ] fn `suprnova::InertiaResponse::defer` · framework/src/inertia/response.rs:498
  - [ ] fn `suprnova::InertiaResponse::defer_with` · framework/src/inertia/response.rs:511
  - [ ] fn `suprnova::InertiaResponse::merge` · framework/src/inertia/response.rs:535
  - [ ] fn `suprnova::InertiaResponse::merge_prepend` · framework/src/inertia/response.rs:541
  - [ ] fn `suprnova::InertiaResponse::deep_merge` · framework/src/inertia/response.rs:547
  - [ ] fn `suprnova::InertiaResponse::merge_with` · framework/src/inertia/response.rs:553
  - [ ] fn `suprnova::InertiaResponse::merge_lazy` · framework/src/inertia/response.rs:577
  - [ ] fn `suprnova::InertiaResponse::merge_lazy_with` · framework/src/inertia/response.rs:589
  - [ ] fn `suprnova::InertiaResponse::once` · framework/src/inertia/response.rs:612
  - [ ] fn `suprnova::InertiaResponse::once_with` · framework/src/inertia/response.rs:625
  - [ ] fn `suprnova::InertiaResponse::scroll` · framework/src/inertia/response.rs:678
  - [ ] fn `suprnova::InertiaResponse::scroll_with` · framework/src/inertia/response.rs:691
  - [ ] fn `suprnova::InertiaResponse::scroll_wrapped` · framework/src/inertia/response.rs:716
  - [ ] fn `suprnova::InertiaResponse::scroll_with_wrapped` · framework/src/inertia/response.rs:728
  - [ ] fn `suprnova::InertiaResponse::paginate` · framework/src/inertia/response.rs:770
  - [ ] fn `suprnova::InertiaResponse::flash` · framework/src/inertia/response.rs:785
  - [ ] fn `suprnova::InertiaResponse::try_with` · framework/src/inertia/response.rs:803
  - [ ] fn `suprnova::InertiaResponse::try_always` · framework/src/inertia/response.rs:815
  - [ ] fn `suprnova::InertiaResponse::try_merge_with` · framework/src/inertia/response.rs:830
  - [ ] fn `suprnova::InertiaResponse::try_scroll` · framework/src/inertia/response.rs:845
  - [ ] fn `suprnova::InertiaResponse::try_scroll_wrapped` · framework/src/inertia/response.rs:860
  - [ ] fn `suprnova::InertiaResponse::try_flash` · framework/src/inertia/response.rs:873
  - [ ] fn `suprnova::InertiaResponse::encrypt_history` · framework/src/inertia/response.rs:888
  - [ ] fn `suprnova::InertiaResponse::clear_history` · framework/src/inertia/response.rs:904
  - [ ] fn `suprnova::InertiaResponse::preserve_fragment` · framework/src/inertia/response.rs:919
  - [ ] fn `suprnova::InertiaResponse::location` · framework/src/inertia/response.rs:943
  - [ ] fn `suprnova::InertiaResponse::location_for` · framework/src/inertia/response.rs:961
  - [ ] fn `suprnova::InertiaResponse::redirect` · framework/src/inertia/response.rs:985
  - [ ] fn `suprnova::InertiaResponse::resolve` · framework/src/inertia/response.rs:1013
  - [ ] fn `suprnova::InertiaResponse::version_conflict` · framework/src/inertia/response.rs:1297
- [ ] enum `suprnova::PropEntry` · framework/src/inertia/response.rs:37 (also `suprnova::inertia::PropEntry`)
  - Variants: `Eager`, `LazyOwned`, `DeferredOwned`, `ClosureOwned`
- [ ] trait `suprnova::IntoInertiaData` · framework/src/inertia/response.rs:73 (also `suprnova::inertia::IntoInertiaData`)
  - [ ] fn `suprnova::IntoInertiaData::__into_inertia_props` · framework/src/inertia/response.rs:76 (required)
  - [ ] fn `suprnova::IntoInertiaData::__try_into_inertia_props` · framework/src/inertia/response.rs:87 (provided)

### `suprnova::inertia::shared` (private module; items are public through re-exports)

- [ ] struct `suprnova::InertiaRegistry` · framework/src/inertia/shared.rs:84 (also `suprnova::inertia::InertiaRegistry`)
  - [ ] fn `suprnova::InertiaRegistry::new` · framework/src/inertia/shared.rs:102
  - [ ] fn `suprnova::InertiaRegistry::share_value` · framework/src/inertia/shared.rs:128
  - [ ] fn `suprnova::InertiaRegistry::share_lazy` · framework/src/inertia/shared.rs:136
  - [ ] fn `suprnova::InertiaRegistry::share_once` · framework/src/inertia/shared.rs:150
  - [ ] fn `suprnova::InertiaRegistry::register_trait` · framework/src/inertia/shared.rs:193
- [ ] trait `suprnova::InertiaSharedData` · framework/src/inertia/shared.rs:53 (also `suprnova::inertia::InertiaSharedData`)
  - Implemented here by: `LocaleShare`
  - [ ] fn `suprnova::InertiaSharedData::share` · framework/src/inertia/shared.rs:64 (required)

### `suprnova::inertia::ssr` (private module; items are public through re-exports)

- [ ] struct `suprnova::SsrResponse` · framework/src/inertia/ssr.rs:27 (also `suprnova::inertia::SsrResponse`)
  - Public fields: `head`, `body`

### `suprnova::inertia::validation_redirect_middleware` (private module; items are public through re-exports)

- [ ] struct `suprnova::InertiaValidationRedirectMiddleware` · framework/src/inertia/validation_redirect_middleware.rs:24 (also `suprnova::inertia::InertiaValidationRedirectMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::InertiaValidationRedirectMiddleware::new` · framework/src/inertia/validation_redirect_middleware.rs:29

### `suprnova::inertia::version_middleware` (private module; items are public through re-exports)

- [ ] struct `suprnova::InertiaVersionMiddleware` · framework/src/inertia/version_middleware.rs:54 (also `suprnova::inertia::InertiaVersionMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::InertiaVersionMiddleware::new` · framework/src/inertia/version_middleware.rs:61
  - [ ] fn `suprnova::InertiaVersionMiddleware::with_resolver` · framework/src/inertia/version_middleware.rs:70

## Rust API: suprnova-macros

### `suprnova_macros`

- [ ] proc macro `suprnova_macros::inertia_response` · suprnova-macros/src/lib.rs:171 (re-exported as `suprnova::inertia_response`)
  - Form: function-like `inertia_response!(...)`
