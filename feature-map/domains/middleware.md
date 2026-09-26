# Feature map: `manual/middleware.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 72 checked.

## Rust API: suprnova

### `suprnova::middleware::aliases` (private module; items are public through re-exports)

- [ ] fn `suprnova::append_middleware_priority` · framework/src/middleware/aliases.rs:387 (also `suprnova::middleware::append_middleware_priority`)
- [ ] fn `suprnova::clear_middleware_alias` · framework/src/middleware/aliases.rs:152 (also `suprnova::middleware::clear_middleware_alias`)
- [ ] fn `suprnova::clear_middleware_group` · framework/src/middleware/aliases.rs:348 (also `suprnova::middleware::clear_middleware_group`)
- [ ] fn `suprnova::has_middleware_alias` · framework/src/middleware/aliases.rs:129 (also `suprnova::middleware::has_middleware_alias`)
- [ ] fn `suprnova::has_middleware_group` · framework/src/middleware/aliases.rs:331 (also `suprnova::middleware::has_middleware_group`)
- [ ] fn `suprnova::middleware_priority` · framework/src/middleware/aliases.rs:400 (also `suprnova::middleware::middleware_priority`)
- [ ] fn `suprnova::prepend_middleware_priority` · framework/src/middleware/aliases.rs:373 (also `suprnova::middleware::prepend_middleware_priority`)
- [ ] fn `suprnova::register_middleware_alias` · framework/src/middleware/aliases.rs:92 (also `suprnova::middleware::register_middleware_alias`)
- [ ] fn `suprnova::register_middleware_group` · framework/src/middleware/aliases.rs:186 (also `suprnova::middleware::register_middleware_group`)
- [ ] fn `suprnova::registered_middleware_aliases` · framework/src/middleware/aliases.rs:140 (also `suprnova::middleware::registered_middleware_aliases`)
- [ ] fn `suprnova::registered_middleware_groups` · framework/src/middleware/aliases.rs:337 (also `suprnova::middleware::registered_middleware_groups`)
- [ ] fn `suprnova::resolve_middleware_alias` · framework/src/middleware/aliases.rs:116 (also `suprnova::middleware::resolve_middleware_alias`)
- [ ] fn `suprnova::resolve_middleware_group` · framework/src/middleware/aliases.rs:259 (also `suprnova::middleware::resolve_middleware_group`)
- [ ] enum `suprnova::MiddlewareResolveError` · framework/src/middleware/aliases.rs:203 (also `suprnova::middleware::MiddlewareResolveError`)
  - Variants: `UnknownGroup`, `UnknownAlias`, `UnknownNestedGroup`, `CycleDetected`
- [ ] type `suprnova::MiddlewareFactory` · framework/src/middleware/aliases.rs:31 (also `suprnova::middleware::MiddlewareFactory`)

### `suprnova::middleware::chain` (private module; items are public through re-exports)

- [ ] struct `suprnova::middleware::MiddlewareChain` · framework/src/middleware/chain.rs:21
  - [ ] fn `suprnova::middleware::MiddlewareChain::new` · framework/src/middleware/chain.rs:27
  - [ ] fn `suprnova::middleware::MiddlewareChain::with_capacity` · framework/src/middleware/chain.rs:42
  - [ ] fn `suprnova::middleware::MiddlewareChain::push` · framework/src/middleware/chain.rs:51
  - [ ] fn `suprnova::middleware::MiddlewareChain::extend` · framework/src/middleware/chain.rs:56
  - [ ] fn `suprnova::middleware::MiddlewareChain::execute` · framework/src/middleware/chain.rs:81

### `suprnova::middleware::pipeline` (private module; items are public through re-exports)

- [ ] struct `suprnova::Pipeline` · framework/src/middleware/pipeline.rs:64 (also `suprnova::middleware::Pipeline`)
  - [ ] fn `suprnova::Pipeline::new` · framework/src/middleware/pipeline.rs:82
  - [ ] fn `suprnova::Pipeline::send` · framework/src/middleware/pipeline.rs:91
  - [ ] fn `suprnova::Pipeline::with_request` · framework/src/middleware/pipeline.rs:97
  - [ ] fn `suprnova::Pipeline::through` · framework/src/middleware/pipeline.rs:103
  - [ ] fn `suprnova::Pipeline::through_boxed` · framework/src/middleware/pipeline.rs:115
  - [ ] fn `suprnova::Pipeline::with_middleware` · framework/src/middleware/pipeline.rs:124
  - [ ] fn `suprnova::Pipeline::pipe` · framework/src/middleware/pipeline.rs:134
  - [ ] fn `suprnova::Pipeline::pipe_boxed` · framework/src/middleware/pipeline.rs:140
  - [ ] fn `suprnova::Pipeline::push` · framework/src/middleware/pipeline.rs:146
  - [ ] fn `suprnova::Pipeline::finally_with` · framework/src/middleware/pipeline.rs:155
  - [ ] fn `suprnova::Pipeline::on_finally` · framework/src/middleware/pipeline.rs:164
  - [ ] fn `suprnova::Pipeline::then` · framework/src/middleware/pipeline.rs:182
  - [ ] fn `suprnova::Pipeline::try_then` · framework/src/middleware/pipeline.rs:201
  - [ ] fn `suprnova::Pipeline::then_with` · framework/src/middleware/pipeline.rs:221
  - [ ] fn `suprnova::Pipeline::then_return` · framework/src/middleware/pipeline.rs:245
  - [ ] fn `suprnova::Pipeline::execute` · framework/src/middleware/pipeline.rs:252
  - [ ] fn `suprnova::Pipeline::len` · framework/src/middleware/pipeline.rs:262
  - [ ] fn `suprnova::Pipeline::is_empty` · framework/src/middleware/pipeline.rs:267
  - [ ] fn `suprnova::Pipeline::pipes` · framework/src/middleware/pipeline.rs:272

### `suprnova::middleware::registry` (private module; items are public through re-exports)

- [ ] fn `suprnova::get_global_middleware` · framework/src/middleware/registry.rs:103 (also `suprnova::middleware::get_global_middleware`)
- [ ] fn `suprnova::global_middleware_count` · framework/src/middleware/registry.rs:156 (also `suprnova::middleware::global_middleware_count`)
- [ ] fn `suprnova::has_global_middleware` · framework/src/middleware/registry.rs:141 (also `suprnova::middleware::has_global_middleware`)
- [ ] fn `suprnova::prepend_global_middleware` · framework/src/middleware/registry.rs:122 (also `suprnova::middleware::prepend_global_middleware`)
- [ ] fn `suprnova::register_global_middleware` · framework/src/middleware/registry.rs:61 (also `suprnova::middleware::register_global_middleware`)
- [ ] struct `suprnova::MiddlewareRegistry` · framework/src/middleware/registry.rs:212 (also `suprnova::middleware::MiddlewareRegistry`)
  - [ ] fn `suprnova::MiddlewareRegistry::new` · framework/src/middleware/registry.rs:219
  - [ ] fn `suprnova::MiddlewareRegistry::from_global` · framework/src/middleware/registry.rs:234
  - [ ] fn `suprnova::MiddlewareRegistry::append` · framework/src/middleware/registry.rs:266
  - [ ] fn `suprnova::MiddlewareRegistry::prepend` · framework/src/middleware/registry.rs:276
  - [ ] fn `suprnova::MiddlewareRegistry::append_boxed` · framework/src/middleware/registry.rs:284
  - [ ] fn `suprnova::MiddlewareRegistry::prepend_boxed` · framework/src/middleware/registry.rs:290
  - [ ] fn `suprnova::MiddlewareRegistry::global_middleware` · framework/src/middleware/registry.rs:305
  - [ ] fn `suprnova::MiddlewareRegistry::len` · framework/src/middleware/registry.rs:311
  - [ ] fn `suprnova::MiddlewareRegistry::is_empty` · framework/src/middleware/registry.rs:316

### `suprnova::middleware::terminable` (private module; items are public through re-exports)

- [ ] fn `suprnova::dispatch_termination` · framework/src/middleware/terminable.rs:151 (also `suprnova::middleware::dispatch_termination`)
- [ ] fn `suprnova::has_terminable` · framework/src/middleware/terminable.rs:137 (also `suprnova::middleware::has_terminable`)
- [ ] fn `suprnova::register_terminable` · framework/src/middleware/terminable.rs:97 (also `suprnova::middleware::register_terminable`)
- [ ] fn `suprnova::registered_terminables` · framework/src/middleware/terminable.rs:117 (also `suprnova::middleware::registered_terminables`)
- [ ] fn `suprnova::terminable_count` · framework/src/middleware/terminable.rs:127 (also `suprnova::middleware::terminable_count`)
- [ ] struct `suprnova::TerminationSnapshot` · framework/src/middleware/terminable.rs:58 (also `suprnova::middleware::TerminationSnapshot`)
  - Public fields: `method`, `path`, `status`
  - [ ] fn `suprnova::TerminationSnapshot::from_response` · framework/src/middleware/terminable.rs:70
- [ ] trait `suprnova::Terminable` · framework/src/middleware/terminable.rs:42 (also `suprnova::middleware::Terminable`)
  - [ ] fn `suprnova::Terminable::terminate` · framework/src/middleware/terminable.rs:45 (required)

### `suprnova::middleware`

- [ ] fn `suprnova::middleware::into_boxed` · framework/src/middleware/mod.rs:110
- [ ] trait `suprnova::Middleware` · framework/src/middleware/mod.rs:100 (also `suprnova::middleware::Middleware`)
  - Implemented here by: `AuthMiddleware`, `BasicAuthMiddleware`, `BearerTokenMiddleware`, `CorsMiddleware`, `CsrfMiddleware`, `EncryptHistoryMiddleware`, `EnsureEmailVerifiedMiddleware`, `GuestMiddleware`, `IncludeMiddleware`, `Inertia303Middleware`, `InertiaErrorPageMiddleware`, `InertiaHeadersMiddleware`, `InertiaValidationRedirectMiddleware`, `InertiaVersionMiddleware`, `LocaleMiddleware`, `LoginThrottleMiddleware`, `MaintenanceMiddleware`, `PermissionMiddleware`, `RateLimitMiddleware`, `RequestIdMiddleware`, `RoleMiddleware`, `SessionMiddleware`, `ThrottleRequestsMiddleware`, `TimeoutMiddleware`, `TwoFactorChallengeMiddleware`, `features::FeatureMiddleware`, `live::LiveTenantMiddleware`, `render_cache::RenderCacheMiddleware`
  - [ ] fn `suprnova::Middleware::handle` · framework/src/middleware/mod.rs:106 (required)
- [ ] type `suprnova::middleware::BoxedMiddleware` · framework/src/middleware/mod.rs:75
- [ ] type `suprnova::MiddlewareFuture` · framework/src/middleware/mod.rs:67 (also `suprnova::middleware::MiddlewareFuture`)
- [ ] type `suprnova::Next` · framework/src/middleware/mod.rs:72 (also `suprnova::middleware::Next`)

### `suprnova`

- [ ] macro `suprnova::global_middleware` · framework/src/lib.rs:718
