# Feature map: `manual/container.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 55 checked.

## Rust API: suprnova

### `suprnova::container::provider`

- [ ] fn `suprnova::container::provider::bootstrap` · framework/src/container/provider.rs:190
- [ ] fn `suprnova::container::provider::register_service_bindings` · framework/src/container/provider.rs:106
- [ ] fn `suprnova::container::provider::register_singletons` · framework/src/container/provider.rs:127
- [ ] struct `suprnova::container::provider::ServiceBindingEntry` · framework/src/container/provider.rs:69
  - Public fields: `register`, `name`
- [ ] struct `suprnova::container::provider::SingletonEntry` · framework/src/container/provider.rs:85
  - Public fields: `register`, `name`

### `suprnova::container`

- [ ] struct `suprnova::App` · framework/src/container/mod.rs:386 (also `suprnova::container::App`, `suprnova::prelude::App`)
  - [ ] fn `suprnova::App::init` · framework/src/container/mod.rs:393
  - [ ] fn `suprnova::App::singleton` · framework/src/container/mod.rs:416
  - [ ] fn `suprnova::App::instance` · framework/src/container/mod.rs:440
  - [ ] fn `suprnova::App::factory` · framework/src/container/mod.rs:456
  - [ ] fn `suprnova::App::bind` · framework/src/container/mod.rs:485
  - [ ] fn `suprnova::App::bind_if_absent` · framework/src/container/mod.rs:501
  - [ ] fn `suprnova::App::singleton_if_absent` · framework/src/container/mod.rs:518
  - [ ] fn `suprnova::App::bind_factory` · framework/src/container/mod.rs:541
  - [ ] fn `suprnova::App::get` · framework/src/container/mod.rs:577
  - [ ] fn `suprnova::App::make` · framework/src/container/mod.rs:638
  - [ ] fn `suprnova::App::resolve` · framework/src/container/mod.rs:687
  - [ ] fn `suprnova::App::resolve_make` · framework/src/container/mod.rs:706
  - [ ] fn `suprnova::App::has` · framework/src/container/mod.rs:715
  - [ ] fn `suprnova::App::has_binding` · framework/src/container/mod.rs:746
  - [ ] fn `suprnova::App::bound` · framework/src/container/mod.rs:789
  - [ ] fn `suprnova::App::bound_binding` · framework/src/container/mod.rs:795
  - [ ] fn `suprnova::App::boot_services` · framework/src/container/mod.rs:810
  - [ ] fn `suprnova::App::inertia_registry` · framework/src/container/mod.rs:823
  - [ ] fn `suprnova::App::inertia_share` · framework/src/container/mod.rs:867
  - [ ] fn `suprnova::App::inertia_share_lazy` · framework/src/container/mod.rs:887
  - [ ] fn `suprnova::App::register_inertia_shared` · framework/src/container/mod.rs:901
  - [ ] fn `suprnova::App::inertia_share_once` · framework/src/container/mod.rs:913
  - [ ] fn `suprnova::App::inertia_shared` · framework/src/container/mod.rs:947
  - [ ] fn `suprnova::App::flush_inertia_shared` · framework/src/container/mod.rs:957
  - [ ] fn `suprnova::App::flash` · framework/src/container/mod.rs:975
  - [ ] fn `suprnova::App::clear_history` · framework/src/container/mod.rs:1033
  - [ ] fn `suprnova::App::disable_ssr_for_request` · framework/src/container/mod.rs:1051
- [ ] struct `suprnova::Container` · framework/src/container/mod.rs:157 (also `suprnova::container::Container`)
  - [ ] fn `suprnova::Container::new` · framework/src/container/mod.rs:167
  - [ ] fn `suprnova::Container::inertia` · framework/src/container/mod.rs:175
  - [ ] fn `suprnova::Container::singleton` · framework/src/container/mod.rs:191
  - [ ] fn `suprnova::Container::factory` · framework/src/container/mod.rs:207
  - [ ] fn `suprnova::Container::bind` · framework/src/container/mod.rs:238
  - [ ] fn `suprnova::Container::bind_if_absent` · framework/src/container/mod.rs:252
  - [ ] fn `suprnova::Container::singleton_if_absent` · framework/src/container/mod.rs:269
  - [ ] fn `suprnova::Container::bind_factory` · framework/src/container/mod.rs:292
  - [ ] fn `suprnova::Container::get` · framework/src/container/mod.rs:322
  - [ ] fn `suprnova::Container::make` · framework/src/container/mod.rs:337
  - [ ] fn `suprnova::Container::has` · framework/src/container/mod.rs:342
  - [ ] fn `suprnova::Container::has_binding` · framework/src/container/mod.rs:347

### `suprnova`

- [ ] macro `suprnova::bind` · framework/src/container/mod.rs:1075
- [ ] macro `suprnova::bind_factory` · framework/src/container/mod.rs:1095
- [ ] macro `suprnova::factory` · framework/src/container/mod.rs:1135
- [ ] macro `suprnova::singleton` · framework/src/container/mod.rs:1117

## Rust API: suprnova-macros

### `suprnova_macros`

- [ ] proc macro `suprnova_macros::injectable` · suprnova-macros/src/lib.rs:309 (re-exported as `suprnova::injectable`)
  - Form: attribute `#[injectable]`
  - [ ] argument field `#[inject]` · suprnova-macros/src/injectable.rs:17
- [ ] proc macro `suprnova_macros::service` · suprnova-macros/src/lib.rs:246 (re-exported as `suprnova::service`)
  - Form: attribute `#[service]`
  - [ ] argument `impl = Type` · suprnova-macros/src/service.rs:47
  - [ ] argument `fake = Type` · suprnova-macros/src/service.rs:48
