# Feature map: `manual/controllers.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 25 checked.

## Rust API: suprnova

### `suprnova::routing::resource` (private module; items are public through re-exports)

- [ ] struct `suprnova::ResourceRoutes` · framework/src/routing/resource.rs:290 (also `suprnova::routing::ResourceRoutes`)
  - [ ] fn `suprnova::ResourceRoutes::only` · framework/src/routing/resource.rs:327
  - [ ] fn `suprnova::ResourceRoutes::keep` · framework/src/routing/resource.rs:335
  - [ ] fn `suprnova::ResourceRoutes::except` · framework/src/routing/resource.rs:341
  - [ ] fn `suprnova::ResourceRoutes::drop` · framework/src/routing/resource.rs:348
  - [ ] fn `suprnova::ResourceRoutes::names` · framework/src/routing/resource.rs:359
  - [ ] fn `suprnova::ResourceRoutes::rename` · framework/src/routing/resource.rs:373
  - [ ] fn `suprnova::ResourceRoutes::parameter` · framework/src/routing/resource.rs:384
  - [ ] fn `suprnova::ResourceRoutes::unnamed` · framework/src/routing/resource.rs:394
  - [ ] fn `suprnova::ResourceRoutes::authorize_resource` · framework/src/routing/resource.rs:444
  - [ ] fn `suprnova::ResourceRoutes::register` · framework/src/routing/resource.rs:467
  - [ ] fn `suprnova::ResourceRoutes::try_register` · framework/src/routing/resource.rs:474
- [ ] enum `suprnova::ResourceAction` · framework/src/routing/resource.rs:70 (also `suprnova::routing::ResourceAction`)
  - Variants: `Index`, `Create`, `Store`, `Show`, `Edit`, `Update`, `Destroy`
  - [ ] fn `suprnova::ResourceAction::key` · framework/src/routing/resource.rs:90
  - [ ] fn `suprnova::ResourceAction::web_defaults` · framework/src/routing/resource.rs:104
  - [ ] fn `suprnova::ResourceAction::api_defaults` · framework/src/routing/resource.rs:118
- [ ] trait `suprnova::ResourceController` · framework/src/routing/resource.rs:229 (also `suprnova::routing::ResourceController`)
  - [ ] fn `suprnova::ResourceController::index` · framework/src/routing/resource.rs:231 (provided)
  - [ ] fn `suprnova::ResourceController::create` · framework/src/routing/resource.rs:237 (provided)
  - [ ] fn `suprnova::ResourceController::store` · framework/src/routing/resource.rs:243 (provided)
  - [ ] fn `suprnova::ResourceController::show` · framework/src/routing/resource.rs:249 (provided)
  - [ ] fn `suprnova::ResourceController::edit` · framework/src/routing/resource.rs:255 (provided)
  - [ ] fn `suprnova::ResourceController::update` · framework/src/routing/resource.rs:261 (provided)
  - [ ] fn `suprnova::ResourceController::destroy` · framework/src/routing/resource.rs:267 (provided)

## Rust API: suprnova-macros

### `suprnova_macros`

- [ ] proc macro `suprnova_macros::handler` · suprnova-macros/src/lib.rs:387 (re-exported as `suprnova::handler`)
  - Form: attribute `#[handler]`
