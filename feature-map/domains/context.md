# Feature map: `manual/context.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 19 checked.

## Rust API: suprnova

### `suprnova::context`

- [ ] struct `suprnova::Context` · framework/src/context/mod.rs:108 (also `suprnova::context::Context`)
  - [ ] fn `suprnova::Context::current` · framework/src/context/mod.rs:119
  - [ ] fn `suprnova::Context::scope` · framework/src/context/mod.rs:137
  - [ ] fn `suprnova::Context::add` · framework/src/context/mod.rs:148
  - [ ] fn `suprnova::Context::get` · framework/src/context/mod.rs:189
  - [ ] fn `suprnova::Context::push` · framework/src/context/mod.rs:215
  - [ ] fn `suprnova::Context::has` · framework/src/context/mod.rs:259
  - [ ] fn `suprnova::Context::forget` · framework/src/context/mod.rs:266
  - [ ] fn `suprnova::Context::all` · framework/src/context/mod.rs:283
  - [ ] fn `suprnova::Context::hidden_add` · framework/src/context/mod.rs:297
  - [ ] fn `suprnova::Context::hidden_get` · framework/src/context/mod.rs:335
  - [ ] fn `suprnova::Context::query_param` · framework/src/context/mod.rs:370
  - [ ] fn `suprnova::Context::test_set_query` · framework/src/context/mod.rs:402
  - [ ] fn `suprnova::Context::test_clear_query` · framework/src/context/mod.rs:418
  - [ ] fn `suprnova::Context::test_query_guard` · framework/src/context/mod.rs:453
- [ ] struct `suprnova::ContextStore` · framework/src/context/mod.rs:57 (also `suprnova::context::ContextStore`)
  - [ ] fn `suprnova::ContextStore::with_query` · framework/src/context/mod.rs:67
- [ ] struct `suprnova::context::TestQueryGuard` · framework/src/context/mod.rs:469 (feature: `testing`)
- [ ] static `suprnova::context::CONTEXT` · framework/src/context/mod.rs:80
