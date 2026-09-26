# Feature map: `manual/localization.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 62 checked.

## Endpoints and tables

### HTTP endpoints the framework owns

- [ ] endpoint `/_suprnova/lang/{locale}.ftl` · framework/src/server.rs:2007
  - any; feature `localization`; serves the locale's Fluent catalog

## Rust API: suprnova

### `suprnova::localization::config` (private module; items are public through re-exports)

- [ ] struct `suprnova::LocalizationConfig` · framework/src/localization/config.rs:21 (feature: `localization`; also `suprnova::localization::LocalizationConfig`)
  - Public fields: `default_locale`, `fallback_locale`, `use_isolating`, `detection`, `session_key`, `cookie_name`, `parents`
  - [ ] fn `suprnova::LocalizationConfig::from_env` · framework/src/localization/config.rs:50
  - [ ] fn `suprnova::LocalizationConfig::default_locale` · framework/src/localization/config.rs:68
  - [ ] fn `suprnova::LocalizationConfig::fallback_locale` · framework/src/localization/config.rs:74
  - [ ] fn `suprnova::LocalizationConfig::use_isolating` · framework/src/localization/config.rs:80
  - [ ] fn `suprnova::LocalizationConfig::detection` · framework/src/localization/config.rs:86
  - [ ] fn `suprnova::LocalizationConfig::session_key` · framework/src/localization/config.rs:92
  - [ ] fn `suprnova::LocalizationConfig::cookie_name` · framework/src/localization/config.rs:98
  - [ ] fn `suprnova::LocalizationConfig::parent` · framework/src/localization/config.rs:107
- [ ] enum `suprnova::Detect` · framework/src/localization/config.rs:10 (feature: `localization`; also `suprnova::localization::Detect`)
  - Variants: `Session`, `Cookie`, `Header`

### `suprnova::localization::fluent` (private module; items are public through re-exports)

- [ ] struct `suprnova::FluentTranslator` · framework/src/localization/fluent.rs:68 (feature: `localization`; also `suprnova::localization::FluentTranslator`)
  - Implements: `suprnova::Translator`
  - [ ] fn `suprnova::FluentTranslator::from_dir` · framework/src/localization/fluent.rs:106
  - [ ] fn `suprnova::FluentTranslator::reload_if_stale` · framework/src/localization/fluent.rs:129

### `suprnova::localization::format` (private module; items are public through re-exports)

- [ ] enum `suprnova::DateStyle` · framework/src/localization/format.rs:45 (feature: `localization`; also `suprnova::localization::DateStyle`)
  - Variants: `Full`, `Long`, `Medium`, `Short`
- [ ] enum `suprnova::ListStyle` · framework/src/localization/format.rs:71 (feature: `localization`; also `suprnova::localization::ListStyle`)
  - Variants: `And`, `Or`, `Unit`
- [ ] enum `suprnova::RelativeUnit` · framework/src/localization/format.rs:82 (feature: `localization`; also `suprnova::localization::RelativeUnit`)
  - Variants: `Second`, `Minute`, `Hour`, `Day`, `Week`, `Month`, `Year`
- [ ] enum `suprnova::TimeStyle` · framework/src/localization/format.rs:62 (feature: `localization`; also `suprnova::localization::TimeStyle`)
  - Variants: `Medium`, `Short`

### `suprnova::localization::locale` (private module; items are public through re-exports)

- [ ] fn `suprnova::localization::negotiate` · framework/src/localization/locale.rs:70 (feature: `localization`)
- [ ] struct `suprnova::Locale` · framework/src/localization/locale.rs:14 (feature: `localization`; also `suprnova::localization::Locale`)
  - [ ] fn `suprnova::Locale::parse` · framework/src/localization/locale.rs:18
  - [ ] fn `suprnova::Locale::as_str` · framework/src/localization/locale.rs:27
  - [ ] fn `suprnova::Locale::language` · framework/src/localization/locale.rs:32

### `suprnova::localization::middleware` (private module; items are public through re-exports)

- [ ] struct `suprnova::LocaleMiddleware` · framework/src/localization/middleware.rs:36 (feature: `localization`; also `suprnova::localization::LocaleMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::LocaleMiddleware::new` · framework/src/localization/middleware.rs:44
  - [ ] fn `suprnova::LocaleMiddleware::from_env` · framework/src/localization/middleware.rs:54

### `suprnova::localization::translator` (private module; items are public through re-exports)

- [ ] struct `suprnova::CatalogSource` · framework/src/localization/translator.rs:11 (feature: `localization`; also `suprnova::localization::CatalogSource`)
  - Public fields: `text`, `hash`
- [ ] trait `suprnova::Translator` · framework/src/localization/translator.rs:23 (feature: `localization`; also `suprnova::localization::Translator`)
  - Implemented here by: `FluentTranslator`
  - [ ] fn `suprnova::Translator::translate` · framework/src/localization/translator.rs:28 (required)
  - [ ] fn `suprnova::Translator::has` · framework/src/localization/translator.rs:36 (required)
  - [ ] fn `suprnova::Translator::available_locales` · framework/src/localization/translator.rs:39 (required)
  - [ ] fn `suprnova::Translator::catalog` · framework/src/localization/translator.rs:47 (required)
  - [ ] fn `suprnova::Translator::reload` · framework/src/localization/translator.rs:50 (required)
  - [ ] fn `suprnova::Translator::reload_if_stale` · framework/src/localization/translator.rs:59 (provided)

### `suprnova::localization` (feature: `localization`)

- [ ] fn `suprnova::scope_locale` · framework/src/localization/mod.rs:160 (also `suprnova::localization::scope_locale`)
- [ ] struct `suprnova::Lang` · framework/src/localization/mod.rs:204 (also `suprnova::localization::Lang`)
  - [ ] fn `suprnova::Lang::locale` · framework/src/localization/mod.rs:212
  - [ ] fn `suprnova::Lang::set_locale` · framework/src/localization/mod.rs:240
  - [ ] fn `suprnova::Lang::get` · framework/src/localization/mod.rs:256
  - [ ] fn `suprnova::Lang::get_with` · framework/src/localization/mod.rs:262
  - [ ] fn `suprnova::Lang::try_get` · framework/src/localization/mod.rs:284
  - [ ] fn `suprnova::Lang::try_get_with` · framework/src/localization/mod.rs:312
  - [ ] fn `suprnova::Lang::has` · framework/src/localization/mod.rs:337
  - [ ] fn `suprnova::Lang::available_locales` · framework/src/localization/mod.rs:352
  - [ ] fn `suprnova::Lang::number` · framework/src/localization/mod.rs:362
  - [ ] fn `suprnova::Lang::try_number` · framework/src/localization/mod.rs:371
  - [ ] fn `suprnova::Lang::currency` · framework/src/localization/mod.rs:379
  - [ ] fn `suprnova::Lang::try_currency` · framework/src/localization/mod.rs:388
  - [ ] fn `suprnova::Lang::date` · framework/src/localization/mod.rs:395
  - [ ] fn `suprnova::Lang::try_date` · framework/src/localization/mod.rs:404
  - [ ] fn `suprnova::Lang::time` · framework/src/localization/mod.rs:411
  - [ ] fn `suprnova::Lang::try_time` · framework/src/localization/mod.rs:420
  - [ ] fn `suprnova::Lang::datetime` · framework/src/localization/mod.rs:427
  - [ ] fn `suprnova::Lang::try_datetime` · framework/src/localization/mod.rs:436
  - [ ] fn `suprnova::Lang::list` · framework/src/localization/mod.rs:447
  - [ ] fn `suprnova::Lang::try_list` · framework/src/localization/mod.rs:456
  - [ ] fn `suprnova::Lang::relative` · framework/src/localization/mod.rs:464
  - [ ] fn `suprnova::Lang::try_relative` · framework/src/localization/mod.rs:473
- [ ] struct `suprnova::LocaleShare` · framework/src/localization/mod.rs:548 (also `suprnova::localization::LocaleShare`)
  - Implements: `suprnova::InertiaSharedData`
- [ ] struct `suprnova::Localization` · framework/src/localization/mod.rs:168 (also `suprnova::localization::Localization`)
- [ ] static `suprnova::localization::CURRENT_LOCALE` · framework/src/localization/mod.rs:36

### `suprnova`

- [ ] macro `suprnova::__` · framework/src/localization/mod.rs:500
