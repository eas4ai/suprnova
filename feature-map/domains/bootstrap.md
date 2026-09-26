# Feature map: `manual/bootstrap.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 23 checked.

## Rust API: suprnova

### `suprnova::app`

- [ ] struct `suprnova::Application` · framework/src/app/mod.rs:240 (also `suprnova::app::Application`)
  - [ ] fn `suprnova::Application::new` · framework/src/app/mod.rs:264
  - [ ] fn `suprnova::Application::framework_version` · framework/src/app/mod.rs:278
  - [ ] fn `suprnova::Application::config` · framework/src/app/mod.rs:601
  - [ ] fn `suprnova::Application::bootstrap` · framework/src/app/mod.rs:628
  - [ ] fn `suprnova::Application::http_bootstrap` · framework/src/app/mod.rs:668
  - [ ] fn `suprnova::Application::routes` · framework/src/app/mod.rs:693
  - [ ] fn `suprnova::Application::try_routes` · framework/src/app/mod.rs:710
  - [ ] fn `suprnova::Application::try_routes_async` · framework/src/app/mod.rs:756
  - [ ] fn `suprnova::Application::booted` · framework/src/app/mod.rs:786
  - [ ] fn `suprnova::Application::schedule` · framework/src/app/mod.rs:811
  - [ ] fn `suprnova::Application::migrations` · framework/src/app/mod.rs:837
  - [ ] fn `suprnova::Application::run` · framework/src/app/mod.rs:863
- [ ] struct `suprnova::app::NoMigrator` · framework/src/app/mod.rs:254

### `suprnova::boot`

- [ ] fn `suprnova::boot::default_build_id` · framework/src/boot.rs:50
- [ ] fn `suprnova::boot::env_loaded_pre_runtime` · framework/src/boot.rs:118
- [ ] fn `suprnova::boot::initialize_crypt_or_exit` · framework/src/boot.rs:107
- [ ] fn `suprnova::boot::load_env` · framework/src/boot.rs:67
- [ ] fn `suprnova::boot::load_env_or_exit` · framework/src/boot.rs:88
- [ ] fn `suprnova::boot::set_default_build_id` · framework/src/boot.rs:40

## Rust API: suprnova-macros

### `suprnova_macros`

- [ ] proc macro `suprnova_macros::main` · suprnova-macros/src/lib.rs:284 (re-exported as `suprnova::main`)
  - Form: attribute `#[main]`
  - [ ] argument `flavor = "multi_thread" | "current_thread"` · suprnova-macros/src/main_macro.rs:39
  - [ ] argument `worker_threads = N` · suprnova-macros/src/main_macro.rs:56
