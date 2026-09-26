# Suprnova feature map: `suprnova-live` Rust API

Source: `crates/suprnova-live/` at d03b4f1, rustdoc JSON (all features), cross-checked against a default-features build.

A checked box means the documentation for that item has been remediated
against the source. Items are listed under their shortest public path;
`also` names the other paths the same item is reachable by.

## Counts

- Top-level items (including re-exports): 965
- Members (methods, associated consts and types): 1893
- By kind: constant 94, enum 212, function 67, struct 530, trait 50, type_alias 10, unnameable 2

## (crate root)

### `suprnova_live`

- [ ] const `suprnova_live::ENGINE_VERSION` · crates/suprnova-live/src/lib.rs:69
- [ ] const `suprnova_live::SUPPORTED_PROTOCOL_VERSIONS` · crates/suprnova-live/src/lib.rs:75
- [ ] const `suprnova_live::SUPPORTED_SNAPSHOT_VERSIONS` · crates/suprnova-live/src/lib.rs:72

## action

### `suprnova_live::action::arguments` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::action::ActionArgumentField` · crates/suprnova-live/src/action/arguments.rs:19
  - [ ] fn `suprnova_live::action::ActionArgumentField::new` · crates/suprnova-live/src/action/arguments.rs:27
  - [ ] fn `suprnova_live::action::ActionArgumentField::name` · crates/suprnova-live/src/action/arguments.rs:40
  - [ ] fn `suprnova_live::action::ActionArgumentField::codec` · crates/suprnova-live/src/action/arguments.rs:46
  - [ ] fn `suprnova_live::action::ActionArgumentField::required` · crates/suprnova-live/src/action/arguments.rs:52
- [ ] struct `suprnova_live::action::ActionArgumentSchema` · crates/suprnova-live/src/action/arguments.rs:59
  - [ ] fn `suprnova_live::action::ActionArgumentSchema::new` · crates/suprnova-live/src/action/arguments.rs:65
  - [ ] fn `suprnova_live::action::ActionArgumentSchema::empty` · crates/suprnova-live/src/action/arguments.rs:80
  - [ ] fn `suprnova_live::action::ActionArgumentSchema::fields` · crates/suprnova-live/src/action/arguments.rs:88
- [ ] struct `suprnova_live::action::PreparedActionArguments` · crates/suprnova-live/src/action/arguments.rs:117
  - [ ] fn `suprnova_live::action::PreparedActionArguments::decode` · crates/suprnova-live/src/action/arguments.rs:168
  - [ ] fn `suprnova_live::action::PreparedActionArguments::canonical` · crates/suprnova-live/src/action/arguments.rs:190
- [ ] struct `suprnova_live::action::RawActionArguments` · crates/suprnova-live/src/action/arguments.rs:98
  - [ ] fn `suprnova_live::action::RawActionArguments::new` · crates/suprnova-live/src/action/arguments.rs:105
  - [ ] fn `suprnova_live::action::RawActionArguments::empty` · crates/suprnova-live/src/action/arguments.rs:111

### `suprnova_live::action::dispatch` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::action::ActionAuthorizationRequest` · crates/suprnova-live/src/action/dispatch.rs:52
  - [ ] fn `suprnova_live::action::ActionAuthorizationRequest::component` · crates/suprnova-live/src/action/dispatch.rs:64
  - [ ] fn `suprnova_live::action::ActionAuthorizationRequest::action` · crates/suprnova-live/src/action/dispatch.rs:70
- [ ] struct `suprnova_live::action::ActionEntry` · crates/suprnova-live/src/action/dispatch.rs:162
  - [ ] fn `suprnova_live::action::ActionEntry::new` · crates/suprnova-live/src/action/dispatch.rs:170
  - [ ] fn `suprnova_live::action::ActionEntry::metadata` · crates/suprnova-live/src/action/dispatch.rs:179
- [ ] struct `suprnova_live::action::ActionError` · crates/suprnova-live/src/action/dispatch.rs:407
  - [ ] fn `suprnova_live::action::ActionError::dispatcher_contract` · crates/suprnova-live/src/action/dispatch.rs:422
  - [ ] fn `suprnova_live::action::ActionError::kind` · crates/suprnova-live/src/action/dispatch.rs:428
- [ ] struct `suprnova_live::action::ActionTable` · crates/suprnova-live/src/action/dispatch.rs:195
  - [ ] fn `suprnova_live::action::ActionTable::new` · crates/suprnova-live/src/action/dispatch.rs:201
  - [ ] fn `suprnova_live::action::ActionTable::len` · crates/suprnova-live/src/action/dispatch.rs:216
  - [ ] fn `suprnova_live::action::ActionTable::is_empty` · crates/suprnova-live/src/action/dispatch.rs:222
  - [ ] fn `suprnova_live::action::ActionTable::prepare` · crates/suprnova-live/src/action/dispatch.rs:236
  - [ ] fn `suprnova_live::action::ActionTable::metadata` · crates/suprnova-live/src/action/dispatch.rs:250
  - [ ] fn `suprnova_live::action::ActionTable::authorize` · crates/suprnova-live/src/action/dispatch.rs:258
  - [ ] fn `suprnova_live::action::ActionTable::dispatch_prepared` · crates/suprnova-live/src/action/dispatch.rs:288
  - [ ] fn `suprnova_live::action::ActionTable::invoke` · crates/suprnova-live/src/action/dispatch.rs:306
- [ ] struct `suprnova_live::action::AuthorizedAction` · crates/suprnova-live/src/action/dispatch.rs:96
  - [ ] fn `suprnova_live::action::AuthorizedAction::component` · crates/suprnova-live/src/action/dispatch.rs:113
  - [ ] fn `suprnova_live::action::AuthorizedAction::action` · crates/suprnova-live/src/action/dispatch.rs:119
  - [ ] fn `suprnova_live::action::AuthorizedAction::is_current` · crates/suprnova-live/src/action/dispatch.rs:125
- [ ] enum `suprnova_live::action::ActionErrorKind` · crates/suprnova-live/src/action/dispatch.rs:384
  - Variants: `UnknownAction`, `DuplicateAction`, `InvalidArguments`, `AuthorizationUnavailable`, `AuthorizationDenied`, `DispatcherContract`, `ComponentFailure`, `Panicked`, `InvalidOutcome`
- [ ] enum `suprnova_live::action::AuthorizationDecision` · crates/suprnova-live/src/action/dispatch.rs:43
  - Variants: `Allow`, `Deny`
- [ ] enum `suprnova_live::action::AuthorizationRequirement` · crates/suprnova-live/src/action/dispatch.rs:25
  - Variants: `Public`, `Current`
- [ ] enum `suprnova_live::action::TransactionPolicy` · crates/suprnova-live/src/action/dispatch.rs:34
  - Variants: `None`, `Required`
- [ ] trait `suprnova_live::action::ActionAuthorizationPort` · crates/suprnova-live/src/action/dispatch.rs:86
  - [ ] fn `suprnova_live::action::ActionAuthorizationPort::authorize` · crates/suprnova-live/src/action/dispatch.rs:88 (required)
- [ ] trait `suprnova_live::action::ActionTarget` · crates/suprnova-live/src/action/dispatch.rs:137
  - [ ] fn `suprnova_live::action::ActionTarget::as_any_mut` · crates/suprnova-live/src/action/dispatch.rs:139 (required)
- [ ] trait `suprnova_live::action::IntoActionResult` · crates/suprnova-live/src/action/dispatch.rs:352
  - Implemented here by: `Result`, `action::ActionOutcome`, `action::ActionResult`
  - [ ] fn `suprnova_live::action::IntoActionResult::into_action_result` · crates/suprnova-live/src/action/dispatch.rs:354 (required)
- [ ] type `suprnova_live::action::ActionDispatchFn` · crates/suprnova-live/src/action/dispatch.rs:154
- [ ] type `suprnova_live::action::ActionFuture` · crates/suprnova-live/src/action/dispatch.rs:21

### `suprnova_live::action::emission` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::action::RegisteredEmission` · crates/suprnova-live/src/action/emission.rs:33
  - [ ] fn `suprnova_live::action::RegisteredEmission::event` · crates/suprnova-live/src/action/emission.rs:43
  - [ ] fn `suprnova_live::action::RegisteredEmission::effect` · crates/suprnova-live/src/action/emission.rs:65
  - [ ] fn `suprnova_live::action::RegisteredEmission::kind` · crates/suprnova-live/src/action/emission.rs:117
  - [ ] fn `suprnova_live::action::RegisteredEmission::name` · crates/suprnova-live/src/action/emission.rs:123
  - [ ] fn `suprnova_live::action::RegisteredEmission::version` · crates/suprnova-live/src/action/emission.rs:129
  - [ ] fn `suprnova_live::action::RegisteredEmission::payload` · crates/suprnova-live/src/action/emission.rs:135
- [ ] enum `suprnova_live::action::EmissionKind` · crates/suprnova-live/src/action/emission.rs:24
  - Variants: `Event`, `Effect`
- [ ] trait `suprnova_live::action::LiveEffectPayload` · crates/suprnova-live/src/action/emission.rs:20
- [ ] trait `suprnova_live::action::LiveEventPayload` · crates/suprnova-live/src/action/emission.rs:17

### `suprnova_live::action::outcome` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::action::ActionResult` · crates/suprnova-live/src/action/outcome.rs:222
  - Implements: `suprnova_live::action::IntoActionResult`
  - [ ] fn `suprnova_live::action::ActionResult::new` · crates/suprnova-live/src/action/outcome.rs:229
  - [ ] fn `suprnova_live::action::ActionResult::render` · crates/suprnova-live/src/action/outcome.rs:250
  - [ ] fn `suprnova_live::action::ActionResult::no_render` · crates/suprnova-live/src/action/outcome.rs:256
  - [ ] fn `suprnova_live::action::ActionResult::from_outcome` · crates/suprnova-live/src/action/outcome.rs:262
  - [ ] fn `suprnova_live::action::ActionResult::outcome` · crates/suprnova-live/src/action/outcome.rs:271
  - [ ] fn `suprnova_live::action::ActionResult::metadata` · crates/suprnova-live/src/action/outcome.rs:277
- [ ] struct `suprnova_live::action::FlashIntent` · crates/suprnova-live/src/action/outcome.rs:116
  - [ ] fn `suprnova_live::action::FlashIntent::new` · crates/suprnova-live/src/action/outcome.rs:123
  - [ ] fn `suprnova_live::action::FlashIntent::key` · crates/suprnova-live/src/action/outcome.rs:135
  - [ ] fn `suprnova_live::action::FlashIntent::value` · crates/suprnova-live/src/action/outcome.rs:141
- [ ] struct `suprnova_live::action::OutcomeError` · crates/suprnova-live/src/action/outcome.rs:299
  - [ ] fn `suprnova_live::action::OutcomeError::kind` · crates/suprnova-live/src/action/outcome.rs:310
- [ ] struct `suprnova_live::action::OutcomeMetadata` · crates/suprnova-live/src/action/outcome.rs:157
  - [ ] fn `suprnova_live::action::OutcomeMetadata::new` · crates/suprnova-live/src/action/outcome.rs:166
  - [ ] fn `suprnova_live::action::OutcomeMetadata::flash` · crates/suprnova-live/src/action/outcome.rs:197
  - [ ] fn `suprnova_live::action::OutcomeMetadata::events` · crates/suprnova-live/src/action/outcome.rs:203
  - [ ] fn `suprnova_live::action::OutcomeMetadata::effects` · crates/suprnova-live/src/action/outcome.rs:209
  - [ ] fn `suprnova_live::action::OutcomeMetadata::url` · crates/suprnova-live/src/action/outcome.rs:215
- [ ] struct `suprnova_live::action::RouteIntent` · crates/suprnova-live/src/action/outcome.rs:42
  - [ ] fn `suprnova_live::action::RouteIntent::new` · crates/suprnova-live/src/action/outcome.rs:49
  - [ ] fn `suprnova_live::action::RouteIntent::route` · crates/suprnova-live/src/action/outcome.rs:64
  - [ ] fn `suprnova_live::action::RouteIntent::parameters` · crates/suprnova-live/src/action/outcome.rs:70
- [ ] struct `suprnova_live::action::UrlIntent` · crates/suprnova-live/src/action/outcome.rs:83
  - [ ] fn `suprnova_live::action::UrlIntent::replace_same_route` · crates/suprnova-live/src/action/outcome.rs:89
  - [ ] fn `suprnova_live::action::UrlIntent::query` · crates/suprnova-live/src/action/outcome.rs:103
- [ ] enum `suprnova_live::action::ActionOutcome` · crates/suprnova-live/src/action/outcome.rs:17
  - Variants: `Render`, `NoRender`, `Redirect`
  - Implements: `suprnova_live::action::IntoActionResult`
  - [ ] fn `suprnova_live::action::ActionOutcome::requires_render` · crates/suprnova-live/src/action/outcome.rs:29
  - [ ] fn `suprnova_live::action::ActionOutcome::redirects` · crates/suprnova-live/src/action/outcome.rs:35
- [ ] enum `suprnova_live::action::OutcomeErrorKind` · crates/suprnova-live/src/action/outcome.rs:284
  - Variants: `InvalidPayload`, `UnregisteredEmission`, `TooManyItems`, `InvalidEmissionChannel`, `IncompatibleOutcome`

## artifacts

### `suprnova_live::artifacts`

- [ ] fn `suprnova_live::artifacts::runtime_artifacts` · crates/suprnova-live/src/artifacts.rs:653
- [ ] struct `suprnova_live::artifacts::ArtifactError` · crates/suprnova-live/src/artifacts.rs:307
  - [ ] fn `suprnova_live::artifacts::ArtifactError::kind` · crates/suprnova-live/src/artifacts.rs:318
- [ ] struct `suprnova_live::artifacts::RuntimeArtifact` · crates/suprnova-live/src/artifacts.rs:338
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifact::role` · crates/suprnova-live/src/artifacts.rs:348
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifact::file` · crates/suprnova-live/src/artifacts.rs:354
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifact::bytes` · crates/suprnova-live/src/artifacts.rs:360
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifact::sha256_hex` · crates/suprnova-live/src/artifacts.rs:366
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifact::sri` · crates/suprnova-live/src/artifacts.rs:372
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifact::content_type` · crates/suprnova-live/src/artifacts.rs:378
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifact::cache_control` · crates/suprnova-live/src/artifacts.rs:384
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifact::capability` · crates/suprnova-live/src/artifacts.rs:390
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifact::capability_version` · crates/suprnova-live/src/artifacts.rs:396
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifact::compatible_core` · crates/suprnova-live/src/artifacts.rs:402
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifact::script_kind` · crates/suprnova-live/src/artifacts.rs:408
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifact::preload_relation` · crates/suprnova-live/src/artifacts.rs:414
- [ ] struct `suprnova_live::artifacts::RuntimeArtifactManifest` · crates/suprnova-live/src/artifacts.rs:421
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifactManifest::validate` · crates/suprnova-live/src/artifacts.rs:483
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifactManifest::manifest_bytes` · crates/suprnova-live/src/artifacts.rs:575
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifactManifest::schema_version` · crates/suprnova-live/src/artifacts.rs:581
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifactManifest::engine_version` · crates/suprnova-live/src/artifacts.rs:587
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifactManifest::runtime_contract_version` · crates/suprnova-live/src/artifacts.rs:593
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifactManifest::protocol_versions` · crates/suprnova-live/src/artifacts.rs:599
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifactManifest::snapshot_versions` · crates/suprnova-live/src/artifacts.rs:605
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifactManifest::built_at` · crates/suprnova-live/src/artifacts.rs:611
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifactManifest::artifacts` · crates/suprnova-live/src/artifacts.rs:617
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifactManifest::artifact` · crates/suprnova-live/src/artifacts.rs:626
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifactManifest::artifact_by_file` · crates/suprnova-live/src/artifacts.rs:632
  - [ ] fn `suprnova_live::artifacts::RuntimeArtifactManifest::asset_identity` · crates/suprnova-live/src/artifacts.rs:644
- [ ] enum `suprnova_live::artifacts::ArtifactErrorKind` · crates/suprnova-live/src/artifacts.rs:294
  - Variants: `ManifestInvalid`, `RoleMissing`, `MetadataMismatch`, `IntegrityMismatch`
- [ ] enum `suprnova_live::artifacts::ArtifactRole` · crates/suprnova-live/src/artifacts.rs:94
  - Variants: `CoreEsm`, `CoreClassic`, `StimulusEsm`, `StimulusClassic`, `UploadsEsm`, `UploadsClassic`, `AsyncEsm`, `AsyncClassic`, `UiStyles`
  - [ ] const `suprnova_live::artifacts::ArtifactRole::ALL` · crates/suprnova-live/src/artifacts.rs:118
  - [ ] fn `suprnova_live::artifacts::ArtifactRole::as_str` · crates/suprnova-live/src/artifacts.rs:132
  - [ ] fn `suprnova_live::artifacts::ArtifactRole::parse` · crates/suprnova-live/src/artifacts.rs:148
  - [ ] fn `suprnova_live::artifacts::ArtifactRole::file` · crates/suprnova-live/src/artifacts.rs:154
  - [ ] fn `suprnova_live::artifacts::ArtifactRole::capability` · crates/suprnova-live/src/artifacts.rs:170
  - [ ] fn `suprnova_live::artifacts::ArtifactRole::script_kind` · crates/suprnova-live/src/artifacts.rs:182
  - [ ] fn `suprnova_live::artifacts::ArtifactRole::preload_relation` · crates/suprnova-live/src/artifacts.rs:197
  - [ ] fn `suprnova_live::artifacts::ArtifactRole::is_core` · crates/suprnova-live/src/artifacts.rs:206
  - [ ] fn `suprnova_live::artifacts::ArtifactRole::is_stylesheet` · crates/suprnova-live/src/artifacts.rs:212
  - [ ] fn `suprnova_live::artifacts::ArtifactRole::content_type` · crates/suprnova-live/src/artifacts.rs:218
- [ ] enum `suprnova_live::artifacts::PreloadRelation` · crates/suprnova-live/src/artifacts.rs:274
  - Variants: `ModulePreload`, `Preload`
  - [ ] fn `suprnova_live::artifacts::PreloadRelation::as_str` · crates/suprnova-live/src/artifacts.rs:284
- [ ] enum `suprnova_live::artifacts::ScriptKind` · crates/suprnova-live/src/artifacts.rs:251
  - Variants: `Module`, `Classic`, `Stylesheet`
  - [ ] fn `suprnova_live::artifacts::ScriptKind::as_str` · crates/suprnova-live/src/artifacts.rs:263
- [ ] const `suprnova_live::artifacts::ARTIFACT_CACHE_CONTROL` · crates/suprnova-live/src/artifacts.rs:40
- [ ] const `suprnova_live::artifacts::ARTIFACT_CONTENT_TYPE` · crates/suprnova-live/src/artifacts.rs:34
- [ ] const `suprnova_live::artifacts::BROWSER_RUNTIME_VERSION` · crates/suprnova-live/src/artifacts.rs:22
- [ ] const `suprnova_live::artifacts::MANIFEST_FILE` · crates/suprnova-live/src/artifacts.rs:43
- [ ] const `suprnova_live::artifacts::MANIFEST_SCHEMA_VERSION` · crates/suprnova-live/src/artifacts.rs:28
- [ ] const `suprnova_live::artifacts::REPRODUCIBLE_BUILD_TIMESTAMP` · crates/suprnova-live/src/artifacts.rs:31
- [ ] const `suprnova_live::artifacts::RUNTIME_CONTRACT_VERSION` · crates/suprnova-live/src/artifacts.rs:25
- [ ] const `suprnova_live::artifacts::STYLESHEET_CONTENT_TYPE` · crates/suprnova-live/src/artifacts.rs:37

## async_updates

### `suprnova_live::async_updates::authorization` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::async_updates::AuthoritativeStreamPosition` · crates/suprnova-live/src/async_updates/authorization.rs:434
  - [ ] fn `suprnova_live::async_updates::AuthoritativeStreamPosition::from_host_continuity` · crates/suprnova-live/src/async_updates/authorization.rs:439
- [ ] struct `suprnova_live::async_updates::AuthorizedSubscription` · crates/suprnova-live/src/async_updates/authorization.rs:777
  - [ ] fn `suprnova_live::async_updates::AuthorizedSubscription::verified` · crates/suprnova-live/src/async_updates/authorization.rs:786
  - [ ] fn `suprnova_live::async_updates::AuthorizedSubscription::binding` · crates/suprnova-live/src/async_updates/authorization.rs:795
  - [ ] fn `suprnova_live::async_updates::AuthorizedSubscription::renewal_credential` · crates/suprnova-live/src/async_updates/authorization.rs:801
- [ ] struct `suprnova_live::async_updates::CurrentSubscriptionRegistration` · crates/suprnova-live/src/async_updates/authorization.rs:277
  - [ ] fn `suprnova_live::async_updates::CurrentSubscriptionRegistration::from_registered` · crates/suprnova-live/src/async_updates/authorization.rs:289
  - [ ] fn `suprnova_live::async_updates::CurrentSubscriptionRegistration::component` · crates/suprnova-live/src/async_updates/authorization.rs:322
  - [ ] fn `suprnova_live::async_updates::CurrentSubscriptionRegistration::component_contract` · crates/suprnova-live/src/async_updates/authorization.rs:328
  - [ ] fn `suprnova_live::async_updates::CurrentSubscriptionRegistration::stream` · crates/suprnova-live/src/async_updates/authorization.rs:334
  - [ ] fn `suprnova_live::async_updates::CurrentSubscriptionRegistration::topics` · crates/suprnova-live/src/async_updates/authorization.rs:340
  - [ ] fn `suprnova_live::async_updates::CurrentSubscriptionRegistration::events` · crates/suprnova-live/src/async_updates/authorization.rs:346
  - [ ] fn `suprnova_live::async_updates::CurrentSubscriptionRegistration::reconnect` · crates/suprnova-live/src/async_updates/authorization.rs:352
  - [ ] fn `suprnova_live::async_updates::CurrentSubscriptionRegistration::canonical_claim_budget_bytes` · crates/suprnova-live/src/async_updates/authorization.rs:358
- [ ] struct `suprnova_live::async_updates::IssuedSubscription` · crates/suprnova-live/src/async_updates/authorization.rs:744
  - [ ] fn `suprnova_live::async_updates::IssuedSubscription::descriptor` · crates/suprnova-live/src/async_updates/authorization.rs:753
  - [ ] fn `suprnova_live::async_updates::IssuedSubscription::transport_credential` · crates/suprnova-live/src/async_updates/authorization.rs:759
  - [ ] fn `suprnova_live::async_updates::IssuedSubscription::expires_at` · crates/suprnova-live/src/async_updates/authorization.rs:765
- [ ] struct `suprnova_live::async_updates::SubscriptionAuthorizationRequest` · crates/suprnova-live/src/async_updates/authorization.rs:153
  - [ ] fn `suprnova_live::async_updates::SubscriptionAuthorizationRequest::operation` · crates/suprnova-live/src/async_updates/authorization.rs:181
  - [ ] fn `suprnova_live::async_updates::SubscriptionAuthorizationRequest::component` · crates/suprnova-live/src/async_updates/authorization.rs:187
  - [ ] fn `suprnova_live::async_updates::SubscriptionAuthorizationRequest::host_scope` · crates/suprnova-live/src/async_updates/authorization.rs:193
  - [ ] fn `suprnova_live::async_updates::SubscriptionAuthorizationRequest::stream` · crates/suprnova-live/src/async_updates/authorization.rs:199
  - [ ] fn `suprnova_live::async_updates::SubscriptionAuthorizationRequest::topics` · crates/suprnova-live/src/async_updates/authorization.rs:205
  - [ ] fn `suprnova_live::async_updates::SubscriptionAuthorizationRequest::binding` · crates/suprnova-live/src/async_updates/authorization.rs:211
- [ ] struct `suprnova_live::async_updates::SubscriptionBaselineRequest` · crates/suprnova-live/src/async_updates/authorization.rs:456
  - [ ] fn `suprnova_live::async_updates::SubscriptionBaselineRequest::component` · crates/suprnova-live/src/async_updates/authorization.rs:482
  - [ ] fn `suprnova_live::async_updates::SubscriptionBaselineRequest::component_contract` · crates/suprnova-live/src/async_updates/authorization.rs:488
  - [ ] fn `suprnova_live::async_updates::SubscriptionBaselineRequest::host_scope` · crates/suprnova-live/src/async_updates/authorization.rs:494
  - [ ] fn `suprnova_live::async_updates::SubscriptionBaselineRequest::stream` · crates/suprnova-live/src/async_updates/authorization.rs:500
  - [ ] fn `suprnova_live::async_updates::SubscriptionBaselineRequest::topics` · crates/suprnova-live/src/async_updates/authorization.rs:506
  - [ ] fn `suprnova_live::async_updates::SubscriptionBaselineRequest::events` · crates/suprnova-live/src/async_updates/authorization.rs:512
- [ ] struct `suprnova_live::async_updates::SubscriptionBinding` · crates/suprnova-live/src/async_updates/authorization.rs:55
  - [ ] fn `suprnova_live::async_updates::SubscriptionBinding::parse` · crates/suprnova-live/src/async_updates/authorization.rs:66
  - [ ] fn `suprnova_live::async_updates::SubscriptionBinding::to_base64url` · crates/suprnova-live/src/async_updates/authorization.rs:74
- [ ] struct `suprnova_live::async_updates::SubscriptionCredentialRequest` · crates/suprnova-live/src/async_updates/authorization.rs:534
  - [ ] fn `suprnova_live::async_updates::SubscriptionCredentialRequest::operation` · crates/suprnova-live/src/async_updates/authorization.rs:581
  - [ ] fn `suprnova_live::async_updates::SubscriptionCredentialRequest::binding` · crates/suprnova-live/src/async_updates/authorization.rs:587
  - [ ] fn `suprnova_live::async_updates::SubscriptionCredentialRequest::scope` · crates/suprnova-live/src/async_updates/authorization.rs:593
  - [ ] fn `suprnova_live::async_updates::SubscriptionCredentialRequest::expires_at` · crates/suprnova-live/src/async_updates/authorization.rs:599
  - [ ] fn `suprnova_live::async_updates::SubscriptionCredentialRequest::now` · crates/suprnova-live/src/async_updates/authorization.rs:605
  - [ ] fn `suprnova_live::async_updates::SubscriptionCredentialRequest::presented` · crates/suprnova-live/src/async_updates/authorization.rs:611
- [ ] struct `suprnova_live::async_updates::SubscriptionCredentialRotationRequest` · crates/suprnova-live/src/async_updates/authorization.rs:632
  - [ ] fn `suprnova_live::async_updates::SubscriptionCredentialRotationRequest::predecessor` · crates/suprnova-live/src/async_updates/authorization.rs:650
  - [ ] fn `suprnova_live::async_updates::SubscriptionCredentialRotationRequest::successor` · crates/suprnova-live/src/async_updates/authorization.rs:656
- [ ] struct `suprnova_live::async_updates::SubscriptionCredentialScope` · crates/suprnova-live/src/async_updates/authorization.rs:87
  - [ ] fn `suprnova_live::async_updates::SubscriptionCredentialScope::component` · crates/suprnova-live/src/async_updates/authorization.rs:110
  - [ ] fn `suprnova_live::async_updates::SubscriptionCredentialScope::component_contract` · crates/suprnova-live/src/async_updates/authorization.rs:116
  - [ ] fn `suprnova_live::async_updates::SubscriptionCredentialScope::authorization_memo` · crates/suprnova-live/src/async_updates/authorization.rs:122
  - [ ] fn `suprnova_live::async_updates::SubscriptionCredentialScope::stream` · crates/suprnova-live/src/async_updates/authorization.rs:128
  - [ ] fn `suprnova_live::async_updates::SubscriptionCredentialScope::topics` · crates/suprnova-live/src/async_updates/authorization.rs:134
  - [ ] fn `suprnova_live::async_updates::SubscriptionCredentialScope::events` · crates/suprnova-live/src/async_updates/authorization.rs:140
- [ ] struct `suprnova_live::async_updates::SubscriptionIssueRequest` · crates/suprnova-live/src/async_updates/authorization.rs:718
  - [ ] fn `suprnova_live::async_updates::SubscriptionIssueRequest::new` · crates/suprnova-live/src/async_updates/authorization.rs:728
- [ ] struct `suprnova_live::async_updates::SubscriptionRegistryRequest` · crates/suprnova-live/src/async_updates/authorization.rs:365
  - [ ] fn `suprnova_live::async_updates::SubscriptionRegistryRequest::operation` · crates/suprnova-live/src/async_updates/authorization.rs:388
  - [ ] fn `suprnova_live::async_updates::SubscriptionRegistryRequest::component` · crates/suprnova-live/src/async_updates/authorization.rs:394
  - [ ] fn `suprnova_live::async_updates::SubscriptionRegistryRequest::component_contract` · crates/suprnova-live/src/async_updates/authorization.rs:400
  - [ ] fn `suprnova_live::async_updates::SubscriptionRegistryRequest::stream` · crates/suprnova-live/src/async_updates/authorization.rs:406
- [ ] struct `suprnova_live::async_updates::SubscriptionService` · crates/suprnova-live/src/async_updates/authorization.rs:813
  - [ ] fn `suprnova_live::async_updates::SubscriptionService::new` · crates/suprnova-live/src/async_updates/authorization.rs:820
  - [ ] fn `suprnova_live::async_updates::SubscriptionService::issue` · crates/suprnova-live/src/async_updates/authorization.rs:827
  - [ ] fn `suprnova_live::async_updates::SubscriptionService::connect` · crates/suprnova-live/src/async_updates/authorization.rs:888
  - [ ] fn `suprnova_live::async_updates::SubscriptionService::renew` · crates/suprnova-live/src/async_updates/authorization.rs:948
- [ ] struct `suprnova_live::async_updates::TrustedMountParameters` · crates/suprnova-live/src/async_updates/authorization.rs:240
  - [ ] fn `suprnova_live::async_updates::TrustedMountParameters::new` · crates/suprnova-live/src/async_updates/authorization.rs:244
- [ ] enum `suprnova_live::async_updates::SubscriptionAuthorizationDecision` · crates/suprnova-live/src/async_updates/authorization.rs:46
  - Variants: `Allow`, `Deny`
- [ ] enum `suprnova_live::async_updates::SubscriptionAuthorizationOperation` · crates/suprnova-live/src/async_updates/authorization.rs:35
  - Variants: `Issue`, `Connect`, `Renew`
- [ ] enum `suprnova_live::async_updates::SubscriptionCredentialRotationOutcome` · crates/suprnova-live/src/async_updates/authorization.rs:668
  - Variants: `Rotated`, `Reject`, `Failed`, `Uncertain`
- [ ] trait `suprnova_live::async_updates::SubscriptionAuthorizationPort` · crates/suprnova-live/src/async_updates/authorization.rs:230
  - [ ] fn `suprnova_live::async_updates::SubscriptionAuthorizationPort::authorize` · crates/suprnova-live/src/async_updates/authorization.rs:232 (required)
- [ ] trait `suprnova_live::async_updates::SubscriptionContinuityPort` · crates/suprnova-live/src/async_updates/authorization.rs:524
  - [ ] fn `suprnova_live::async_updates::SubscriptionContinuityPort::authoritative_baseline` · crates/suprnova-live/src/async_updates/authorization.rs:526 (required)
- [ ] trait `suprnova_live::async_updates::SubscriptionCredentialPort` · crates/suprnova-live/src/async_updates/authorization.rs:691
  - [ ] fn `suprnova_live::async_updates::SubscriptionCredentialPort::issue` · crates/suprnova-live/src/async_updates/authorization.rs:696 (required)
  - [ ] fn `suprnova_live::async_updates::SubscriptionCredentialPort::consume_and_rotate` · crates/suprnova-live/src/async_updates/authorization.rs:710 (required)
- [ ] trait `suprnova_live::async_updates::SubscriptionRegistryPort` · crates/suprnova-live/src/async_updates/authorization.rs:424
  - [ ] fn `suprnova_live::async_updates::SubscriptionRegistryPort::resolve` · crates/suprnova-live/src/async_updates/authorization.rs:426 (required)
- [ ] type `suprnova_live::async_updates::SubscriptionFuture` · crates/suprnova-live/src/async_updates/authorization.rs:31

### `suprnova_live::async_updates::backpressure` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::async_updates::AsyncBackpressureError` · crates/suprnova-live/src/async_updates/backpressure.rs:457
  - [ ] fn `suprnova_live::async_updates::AsyncBackpressureError::close_code` · crates/suprnova-live/src/async_updates/backpressure.rs:464
- [ ] struct `suprnova_live::async_updates::AsyncPolicy` · crates/suprnova-live/src/async_updates/backpressure.rs:87
  - Public fields: `max_payload_bytes`, `max_replay_events`, `max_fanout`
- [ ] enum `suprnova_live::async_updates::AsyncCloseCode` · crates/suprnova-live/src/async_updates/backpressure.rs:39
  - Variants: `InvalidPolicy`, `InvalidEnvelope`, `PayloadTooLarge`, `FanoutExceeded`, `Retired`
  - [ ] fn `suprnova_live::async_updates::AsyncCloseCode::as_str` · crates/suprnova-live/src/async_updates/backpressure.rs:55
- [ ] enum `suprnova_live::async_updates::BufferDisposition` · crates/suprnova-live/src/async_updates/backpressure.rs:68
  - Variants: `Queued`, `Coalesced`, `Degraded`, `Closed`
- [ ] const `suprnova_live::async_updates::MAX_ASYNC_BUFFER_BYTES` · crates/suprnova-live/src/async_updates/backpressure.rs:33
- [ ] const `suprnova_live::async_updates::MAX_ASYNC_BUFFER_EVENTS` · crates/suprnova-live/src/async_updates/backpressure.rs:31
- [ ] const `suprnova_live::async_updates::MAX_ASYNC_PAYLOAD_BYTES` · crates/suprnova-live/src/async_updates/backpressure.rs:35

### `suprnova_live::async_updates::envelope` (private module; items are public through re-exports)

- [ ] fn `suprnova_live::async_updates::decode_async_envelope` · crates/suprnova-live/src/async_updates/envelope.rs:1406
- [ ] fn `suprnova_live::async_updates::encode_async_envelope` · crates/suprnova-live/src/async_updates/envelope.rs:1668
- [ ] struct `suprnova_live::async_updates::AsyncCodecLimits` · crates/suprnova-live/src/async_updates/envelope.rs:135
  - [ ] fn `suprnova_live::async_updates::AsyncCodecLimits::new` · crates/suprnova-live/src/async_updates/envelope.rs:142
  - [ ] fn `suprnova_live::async_updates::AsyncCodecLimits::v1` · crates/suprnova-live/src/async_updates/envelope.rs:161
  - [ ] fn `suprnova_live::async_updates::AsyncCodecLimits::hostile_test` · crates/suprnova-live/src/async_updates/envelope.rs:170
- [ ] struct `suprnova_live::async_updates::AsyncEnvelope` · crates/suprnova-live/src/async_updates/envelope.rs:1346
  - [ ] fn `suprnova_live::async_updates::AsyncEnvelope::new` · crates/suprnova-live/src/async_updates/envelope.rs:1356
  - [ ] fn `suprnova_live::async_updates::AsyncEnvelope::protocol_version` · crates/suprnova-live/src/async_updates/envelope.rs:1376
  - [ ] fn `suprnova_live::async_updates::AsyncEnvelope::subscription` · crates/suprnova-live/src/async_updates/envelope.rs:1382
  - [ ] fn `suprnova_live::async_updates::AsyncEnvelope::stream` · crates/suprnova-live/src/async_updates/envelope.rs:1388
  - [ ] fn `suprnova_live::async_updates::AsyncEnvelope::position` · crates/suprnova-live/src/async_updates/envelope.rs:1394
  - [ ] fn `suprnova_live::async_updates::AsyncEnvelope::payload` · crates/suprnova-live/src/async_updates/envelope.rs:1400
- [ ] struct `suprnova_live::async_updates::AsyncEnvelopeContext` · crates/suprnova-live/src/async_updates/envelope.rs:742
  - [ ] fn `suprnova_live::async_updates::AsyncEnvelopeContext::from_authorized` · crates/suprnova-live/src/async_updates/envelope.rs:753
  - [ ] fn `suprnova_live::async_updates::AsyncEnvelopeContext::subscription` · crates/suprnova-live/src/async_updates/envelope.rs:772
  - [ ] fn `suprnova_live::async_updates::AsyncEnvelopeContext::stream` · crates/suprnova-live/src/async_updates/envelope.rs:778
  - [ ] fn `suprnova_live::async_updates::AsyncEnvelopeContext::authoritative_baseline` · crates/suprnova-live/src/async_updates/envelope.rs:784
- [ ] struct `suprnova_live::async_updates::AsyncEnvelopeError` · crates/suprnova-live/src/async_updates/envelope.rs:103
  - [ ] fn `suprnova_live::async_updates::AsyncEnvelopeError::kind` · crates/suprnova-live/src/async_updates/envelope.rs:114
- [ ] struct `suprnova_live::async_updates::AsyncMembershipRequest` · crates/suprnova-live/src/async_updates/envelope.rs:347
  - [ ] fn `suprnova_live::async_updates::AsyncMembershipRequest::verified` · crates/suprnova-live/src/async_updates/envelope.rs:406
  - [ ] fn `suprnova_live::async_updates::AsyncMembershipRequest::subscription` · crates/suprnova-live/src/async_updates/envelope.rs:412
  - [ ] fn `suprnova_live::async_updates::AsyncMembershipRequest::envelope` · crates/suprnova-live/src/async_updates/envelope.rs:418
  - [ ] fn `suprnova_live::async_updates::AsyncMembershipRequest::binding` · crates/suprnova-live/src/async_updates/envelope.rs:424
  - [ ] fn `suprnova_live::async_updates::AsyncMembershipRequest::document_scope` · crates/suprnova-live/src/async_updates/envelope.rs:430
- [ ] struct `suprnova_live::async_updates::AsyncMembershipValidation` · crates/suprnova-live/src/async_updates/envelope.rs:519
  - [ ] fn `suprnova_live::async_updates::AsyncMembershipValidation::accept_current` · crates/suprnova-live/src/async_updates/envelope.rs:532
  - [ ] fn `suprnova_live::async_updates::AsyncMembershipValidation::accept_delivery_current` · crates/suprnova-live/src/async_updates/envelope.rs:566
  - [ ] fn `suprnova_live::async_updates::AsyncMembershipValidation::accept_scope_current` · crates/suprnova-live/src/async_updates/envelope.rs:608
- [ ] struct `suprnova_live::async_updates::AsyncReplayMembershipRequest` · crates/suprnova-live/src/async_updates/envelope.rs:357
  - [ ] fn `suprnova_live::async_updates::AsyncReplayMembershipRequest::verified` · crates/suprnova-live/src/async_updates/envelope.rs:368
  - [ ] fn `suprnova_live::async_updates::AsyncReplayMembershipRequest::subscription` · crates/suprnova-live/src/async_updates/envelope.rs:374
  - [ ] fn `suprnova_live::async_updates::AsyncReplayMembershipRequest::envelopes` · crates/suprnova-live/src/async_updates/envelope.rs:380
  - [ ] fn `suprnova_live::async_updates::AsyncReplayMembershipRequest::binding` · crates/suprnova-live/src/async_updates/envelope.rs:386
  - [ ] fn `suprnova_live::async_updates::AsyncReplayMembershipRequest::document_scope` · crates/suprnova-live/src/async_updates/envelope.rs:392
- [ ] struct `suprnova_live::async_updates::AsyncReplayMembershipValidation` · crates/suprnova-live/src/async_updates/envelope.rs:676
  - [ ] fn `suprnova_live::async_updates::AsyncReplayMembershipValidation::accept_current` · crates/suprnova-live/src/async_updates/envelope.rs:686
- [ ] struct `suprnova_live::async_updates::BoundedPresentationSignalContracts` · crates/suprnova-live/src/async_updates/envelope.rs:302
  - [ ] fn `suprnova_live::async_updates::BoundedPresentationSignalContracts::new` · crates/suprnova-live/src/async_updates/envelope.rs:306
- [ ] struct `suprnova_live::async_updates::Heartbeat` · crates/suprnova-live/src/async_updates/envelope.rs:1301
- [ ] struct `suprnova_live::async_updates::PresentationSignalContract` · crates/suprnova-live/src/async_updates/envelope.rs:260
  - [ ] fn `suprnova_live::async_updates::PresentationSignalContract::new` · crates/suprnova-live/src/async_updates/envelope.rs:269
  - [ ] fn `suprnova_live::async_updates::PresentationSignalContract::scope` · crates/suprnova-live/src/async_updates/envelope.rs:283
  - [ ] fn `suprnova_live::async_updates::PresentationSignalContract::name` · crates/suprnova-live/src/async_updates/envelope.rs:289
  - [ ] fn `suprnova_live::async_updates::PresentationSignalContract::schema` · crates/suprnova-live/src/async_updates/envelope.rs:295
- [ ] struct `suprnova_live::async_updates::RegisteredBrowserEvent` · crates/suprnova-live/src/async_updates/envelope.rs:1159
  - [ ] fn `suprnova_live::async_updates::RegisteredBrowserEvent::new` · crates/suprnova-live/src/async_updates/envelope.rs:1181
  - [ ] fn `suprnova_live::async_updates::RegisteredBrowserEvent::name` · crates/suprnova-live/src/async_updates/envelope.rs:1204
  - [ ] fn `suprnova_live::async_updates::RegisteredBrowserEvent::schema_version` · crates/suprnova-live/src/async_updates/envelope.rs:1210
  - [ ] fn `suprnova_live::async_updates::RegisteredBrowserEvent::target` · crates/suprnova-live/src/async_updates/envelope.rs:1216
  - [ ] fn `suprnova_live::async_updates::RegisteredBrowserEvent::payload` · crates/suprnova-live/src/async_updates/envelope.rs:1222
  - [ ] fn `suprnova_live::async_updates::RegisteredBrowserEvent::maximum_fanout` · crates/suprnova-live/src/async_updates/envelope.rs:1228
- [ ] struct `suprnova_live::async_updates::RegisteredPresentationSignal` · crates/suprnova-live/src/async_updates/envelope.rs:1235
  - [ ] fn `suprnova_live::async_updates::RegisteredPresentationSignal::new` · crates/suprnova-live/src/async_updates/envelope.rs:1255
  - [ ] fn `suprnova_live::async_updates::RegisteredPresentationSignal::name` · crates/suprnova-live/src/async_updates/envelope.rs:1276
  - [ ] fn `suprnova_live::async_updates::RegisteredPresentationSignal::scope` · crates/suprnova-live/src/async_updates/envelope.rs:1282
  - [ ] fn `suprnova_live::async_updates::RegisteredPresentationSignal::value` · crates/suprnova-live/src/async_updates/envelope.rs:1288
  - [ ] fn `suprnova_live::async_updates::RegisteredPresentationSignal::schema` · crates/suprnova-live/src/async_updates/envelope.rs:1294
- [ ] struct `suprnova_live::async_updates::RegisteredRefresh` · crates/suprnova-live/src/async_updates/envelope.rs:1155
- [ ] struct `suprnova_live::async_updates::ResolvedEventFanout` · crates/suprnova-live/src/async_updates/envelope.rs:468
  - [ ] fn `suprnova_live::async_updates::ResolvedEventFanout::from_host` · crates/suprnova-live/src/async_updates/envelope.rs:476
  - [ ] fn `suprnova_live::async_updates::ResolvedEventFanout::recipients` · crates/suprnova-live/src/async_updates/envelope.rs:485
- [ ] struct `suprnova_live::async_updates::SubscriptionId` · crates/suprnova-live/src/async_updates/envelope.rs:188
  - [ ] const `suprnova_live::async_updates::SubscriptionId::MIN_ENCODED_LEN` · crates/suprnova-live/src/async_updates/envelope.rs:192
  - [ ] const `suprnova_live::async_updates::SubscriptionId::MAX_ENCODED_LEN` · crates/suprnova-live/src/async_updates/envelope.rs:195
  - [ ] fn `suprnova_live::async_updates::SubscriptionId::from_bytes` · crates/suprnova-live/src/async_updates/envelope.rs:198
  - [ ] fn `suprnova_live::async_updates::SubscriptionId::parse` · crates/suprnova-live/src/async_updates/envelope.rs:208
  - [ ] fn `suprnova_live::async_updates::SubscriptionId::to_base64url` · crates/suprnova-live/src/async_updates/envelope.rs:232
- [ ] enum `suprnova_live::async_updates::AsyncEnvelopeErrorKind` · crates/suprnova-live/src/async_updates/envelope.rs:44
  - Variants: `TooLarge`, `TooDeep`, `TooManyEntries`, `StringTooLong`, `DuplicateField`, `NonCanonical`, `InvalidEnvelope`, `UnsupportedProtocol`, `SubscriptionMismatch`, `StreamMismatch`, `PayloadTooLarge`, `UnsupportedPayload`, `InvalidPayload`, `UnregisteredPayload`, `MembershipExpired`
  - [ ] fn `suprnova_live::async_updates::AsyncEnvelopeErrorKind::as_str` · crates/suprnova-live/src/async_updates/envelope.rs:80
- [ ] enum `suprnova_live::async_updates::AsyncPayload` · crates/suprnova-live/src/async_updates/envelope.rs:1329
  - Variants: `Refresh`, `BrowserEvent`, `PresentationSignal`, `Heartbeat`, `Complete`, `Error`
- [ ] enum `suprnova_live::async_updates::CompletionReason` · crates/suprnova-live/src/async_updates/envelope.rs:1305
  - Variants: `ServerShutdown`, `SubscriptionRetired`, `StreamCompleted`
- [ ] enum `suprnova_live::async_updates::PresentationSignalSchema` · crates/suprnova-live/src/async_updates/envelope.rs:245
  - Variants: `Null`, `Boolean`, `I64`, `U64`, `String`
- [ ] enum `suprnova_live::async_updates::StreamErrorCode` · crates/suprnova-live/src/async_updates/envelope.rs:1316
  - Variants: `AuthorizationLost`, `ReplayUnavailable`, `Backpressure`, `StreamUnavailable`
- [ ] trait `suprnova_live::async_updates::AsyncMembershipRegistryPort` · crates/suprnova-live/src/async_updates/envelope.rs:445
  - [ ] fn `suprnova_live::async_updates::AsyncMembershipRegistryPort::validate_current` · crates/suprnova-live/src/async_updates/envelope.rs:447 (required)
  - [ ] fn `suprnova_live::async_updates::AsyncMembershipRegistryPort::validate_replay_current` · crates/suprnova-live/src/async_updates/envelope.rs:457 (provided)
- [ ] const `suprnova_live::async_updates::MAX_ASYNC_ENVELOPE_ENTRIES` · crates/suprnova-live/src/async_updates/envelope.rs:32
- [ ] const `suprnova_live::async_updates::SUPPORTED_ASYNC_PROTOCOL_VERSIONS` · crates/suprnova-live/src/async_updates/envelope.rs:26

### `suprnova_live::async_updates::metadata` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::async_updates::BoundedEventNames` · crates/suprnova-live/src/async_updates/metadata.rs:204
  - [ ] fn `suprnova_live::async_updates::BoundedEventNames::new` · crates/suprnova-live/src/async_updates/metadata.rs:208
  - [ ] fn `suprnova_live::async_updates::BoundedEventNames::as_slice` · crates/suprnova-live/src/async_updates/metadata.rs:230
- [ ] struct `suprnova_live::async_updates::BoundedTargets` · crates/suprnova-live/src/async_updates/metadata.rs:75
  - [ ] fn `suprnova_live::async_updates::BoundedTargets::new` · crates/suprnova-live/src/async_updates/metadata.rs:79
  - [ ] fn `suprnova_live::async_updates::BoundedTargets::as_slice` · crates/suprnova-live/src/async_updates/metadata.rs:95
- [ ] struct `suprnova_live::async_updates::BoundedTopics` · crates/suprnova-live/src/async_updates/metadata.rs:171
  - [ ] fn `suprnova_live::async_updates::BoundedTopics::new` · crates/suprnova-live/src/async_updates/metadata.rs:175
  - [ ] fn `suprnova_live::async_updates::BoundedTopics::as_slice` · crates/suprnova-live/src/async_updates/metadata.rs:197
- [ ] struct `suprnova_live::async_updates::StreamName` · crates/suprnova-live/src/async_updates/metadata.rs:118
  - [ ] fn `suprnova_live::async_updates::StreamName::parse` · crates/suprnova-live/src/async_updates/metadata.rs:122
  - [ ] fn `suprnova_live::async_updates::StreamName::as_str` · crates/suprnova-live/src/async_updates/metadata.rs:128
- [ ] struct `suprnova_live::async_updates::SubscriptionMetadata` · crates/suprnova-live/src/async_updates/metadata.rs:291
  - [ ] fn `suprnova_live::async_updates::SubscriptionMetadata::new` · crates/suprnova-live/src/async_updates/metadata.rs:302
  - [ ] fn `suprnova_live::async_updates::SubscriptionMetadata::stream` · crates/suprnova-live/src/async_updates/metadata.rs:320
  - [ ] fn `suprnova_live::async_updates::SubscriptionMetadata::topics` · crates/suprnova-live/src/async_updates/metadata.rs:326
  - [ ] fn `suprnova_live::async_updates::SubscriptionMetadata::events` · crates/suprnova-live/src/async_updates/metadata.rs:332
  - [ ] fn `suprnova_live::async_updates::SubscriptionMetadata::modes` · crates/suprnova-live/src/async_updates/metadata.rs:338
  - [ ] fn `suprnova_live::async_updates::SubscriptionMetadata::reconnect` · crates/suprnova-live/src/async_updates/metadata.rs:344
- [ ] struct `suprnova_live::async_updates::SubscriptionModes` · crates/suprnova-live/src/async_updates/metadata.rs:246
  - [ ] fn `suprnova_live::async_updates::SubscriptionModes::new` · crates/suprnova-live/src/async_updates/metadata.rs:250
  - [ ] fn `suprnova_live::async_updates::SubscriptionModes::as_slice` · crates/suprnova-live/src/async_updates/metadata.rs:272
- [ ] struct `suprnova_live::async_updates::TopicName` · crates/suprnova-live/src/async_updates/metadata.rs:141
  - [ ] fn `suprnova_live::async_updates::TopicName::parse` · crates/suprnova-live/src/async_updates/metadata.rs:145
  - [ ] fn `suprnova_live::async_updates::TopicName::as_str` · crates/suprnova-live/src/async_updates/metadata.rs:158
- [ ] enum `suprnova_live::async_updates::BrowserPayloadSchema` · crates/suprnova-live/src/async_updates/metadata.rs:27
  - Variants: `Json`, `Null`, `Boolean`, `I64`, `U64`, `F64`, `String`
- [ ] enum `suprnova_live::async_updates::EventCyclePolicy` · crates/suprnova-live/src/async_updates/metadata.rs:109
  - Variants: `ForbidRepeatedIsland`, `MaximumHops`
- [ ] enum `suprnova_live::async_updates::EventOrder` · crates/suprnova-live/src/async_updates/metadata.rs:102
  - Variants: `PerSourceSequence`
- [ ] enum `suprnova_live::async_updates::EventSource` · crates/suprnova-live/src/async_updates/metadata.rs:46
  - Variants: `Component`, `Stream`
- [ ] enum `suprnova_live::async_updates::EventTarget` · crates/suprnova-live/src/async_updates/metadata.rs:58
  - Variants: `SelfIsland`, `Parent`, `Child`, `NamedIsland`, `Document`, `Browser`
- [ ] enum `suprnova_live::async_updates::ReconnectPolicy` · crates/suprnova-live/src/async_updates/metadata.rs:279
  - Variants: `RefreshOnReconnect`, `ResumeOrRefresh`
- [ ] enum `suprnova_live::async_updates::SubscriptionMode` · crates/suprnova-live/src/async_updates/metadata.rs:237
  - Variants: `ServerSentEvents`, `WebSocket`
- [ ] const `suprnova_live::async_updates::MAX_EVENT_FANOUT` · crates/suprnova-live/src/async_updates/metadata.rs:12
- [ ] const `suprnova_live::async_updates::MAX_EVENT_TARGETS` · crates/suprnova-live/src/async_updates/metadata.rs:10
- [ ] const `suprnova_live::async_updates::MAX_SUBSCRIPTION_EVENTS` · crates/suprnova-live/src/async_updates/metadata.rs:18
- [ ] const `suprnova_live::async_updates::MAX_SUBSCRIPTION_MODES` · crates/suprnova-live/src/async_updates/metadata.rs:20
- [ ] const `suprnova_live::async_updates::MAX_SUBSCRIPTION_TOPICS` · crates/suprnova-live/src/async_updates/metadata.rs:16
- [ ] const `suprnova_live::async_updates::MAX_SUBSCRIPTIONS` · crates/suprnova-live/src/async_updates/metadata.rs:14

### `suprnova_live::async_updates::sequence` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::async_updates::AsyncContinuityRequest` · crates/suprnova-live/src/async_updates/sequence.rs:259
  - [ ] fn `suprnova_live::async_updates::AsyncContinuityRequest::subscription` · crates/suprnova-live/src/async_updates/sequence.rs:269
  - [ ] fn `suprnova_live::async_updates::AsyncContinuityRequest::stream` · crates/suprnova-live/src/async_updates/sequence.rs:275
  - [ ] fn `suprnova_live::async_updates::AsyncContinuityRequest::current` · crates/suprnova-live/src/async_updates/sequence.rs:281
  - [ ] fn `suprnova_live::async_updates::AsyncContinuityRequest::high_water` · crates/suprnova-live/src/async_updates/sequence.rs:287
- [ ] struct `suprnova_live::async_updates::AsyncDispatchError` · crates/suprnova-live/src/async_updates/sequence.rs:115
  - [ ] fn `suprnova_live::async_updates::AsyncDispatchError::rejected` · crates/suprnova-live/src/async_updates/sequence.rs:122
  - [ ] fn `suprnova_live::async_updates::AsyncDispatchError::failed` · crates/suprnova-live/src/async_updates/sequence.rs:130
  - [ ] fn `suprnova_live::async_updates::AsyncDispatchError::kind` · crates/suprnova-live/src/async_updates/sequence.rs:138
- [ ] struct `suprnova_live::async_updates::ReplayDispatchError` · crates/suprnova-live/src/async_updates/sequence.rs:787
  - [ ] fn `suprnova_live::async_updates::ReplayDispatchError::kind` · crates/suprnova-live/src/async_updates/sequence.rs:798
  - [ ] fn `suprnova_live::async_updates::ReplayDispatchError::applied` · crates/suprnova-live/src/async_updates/sequence.rs:804
  - [ ] fn `suprnova_live::async_updates::ReplayDispatchError::current` · crates/suprnova-live/src/async_updates/sequence.rs:810
  - [ ] fn `suprnova_live::async_updates::ReplayDispatchError::state` · crates/suprnova-live/src/async_updates/sequence.rs:816
  - [ ] fn `suprnova_live::async_updates::ReplayDispatchError::high_water` · crates/suprnova-live/src/async_updates/sequence.rs:822
- [ ] struct `suprnova_live::async_updates::ReplayDispatchOutcome` · crates/suprnova-live/src/async_updates/sequence.rs:759
  - [ ] fn `suprnova_live::async_updates::ReplayDispatchOutcome::applied` · crates/suprnova-live/src/async_updates/sequence.rs:768
  - [ ] fn `suprnova_live::async_updates::ReplayDispatchOutcome::current` · crates/suprnova-live/src/async_updates/sequence.rs:774
  - [ ] fn `suprnova_live::async_updates::ReplayDispatchOutcome::state` · crates/suprnova-live/src/async_updates/sequence.rs:780
- [ ] struct `suprnova_live::async_updates::ResolvedAsyncDelivery` · crates/suprnova-live/src/async_updates/sequence.rs:161
  - [ ] fn `suprnova_live::async_updates::ResolvedAsyncDelivery::envelope` · crates/suprnova-live/src/async_updates/sequence.rs:182
  - [ ] fn `suprnova_live::async_updates::ResolvedAsyncDelivery::resolved_recipients` · crates/suprnova-live/src/async_updates/sequence.rs:188
  - [ ] fn `suprnova_live::async_updates::ResolvedAsyncDelivery::resolved_target_scope` · crates/suprnova-live/src/async_updates/sequence.rs:194
  - [ ] fn `suprnova_live::async_updates::ResolvedAsyncDelivery::deployment_fanout_limit` · crates/suprnova-live/src/async_updates/sequence.rs:200
- [ ] struct `suprnova_live::async_updates::SequenceError` · crates/suprnova-live/src/async_updates/sequence.rs:227
  - [ ] fn `suprnova_live::async_updates::SequenceError::kind` · crates/suprnova-live/src/async_updates/sequence.rs:238
- [ ] enum `suprnova_live::async_updates::AsyncDispatchErrorKind` · crates/suprnova-live/src/async_updates/sequence.rs:106
  - Variants: `Rejected`, `Failed`
- [ ] enum `suprnova_live::async_updates::BaselineDisposition` · crates/suprnova-live/src/async_updates/sequence.rs:53
  - Variants: `Adopted`, `AlreadyCurrent`
- [ ] enum `suprnova_live::async_updates::SequenceDegradation` · crates/suprnova-live/src/async_updates/sequence.rs:27
  - Variants: `Gap`, `EpochChanged`
- [ ] enum `suprnova_live::async_updates::SequenceDisposition` · crates/suprnova-live/src/async_updates/sequence.rs:36
  - Variants: `Apply`, `IgnoreDuplicate`, `IgnoreStaleEpoch`, `ScopeMismatch`, `Degraded`, `AwaitingRecovery`
- [ ] enum `suprnova_live::async_updates::SequenceErrorKind` · crates/suprnova-live/src/async_updates/sequence.rs:62
  - Variants: `InvalidReplayTranscript`, `ScopeMismatch`, `MembershipExpired`, `DeliveryRetired`, `AuthorizationLost`, `BaselineRegression`, `AuthoritativeBaselineInsufficient`, `AuthoritativeRefreshUnavailable`, `DispatchRejected`, `DispatchFailed`
  - [ ] fn `suprnova_live::async_updates::SequenceErrorKind::as_str` · crates/suprnova-live/src/async_updates/sequence.rs:88
- [ ] enum `suprnova_live::async_updates::SequenceState` · crates/suprnova-live/src/async_updates/sequence.rs:18
  - Variants: `Current`, `Degraded`
- [ ] trait `suprnova_live::async_updates::AsyncContinuityAuthorityPort` · crates/suprnova-live/src/async_updates/sequence.rs:308
  - [ ] fn `suprnova_live::async_updates::AsyncContinuityAuthorityPort::authoritative_refresh` · crates/suprnova-live/src/async_updates/sequence.rs:310 (required)
- [ ] trait `suprnova_live::async_updates::AsyncEnvelopeDispatchPort` · crates/suprnova-live/src/async_updates/sequence.rs:220
  - [ ] fn `suprnova_live::async_updates::AsyncEnvelopeDispatchPort::dispatch` · crates/suprnova-live/src/async_updates/sequence.rs:222 (required)
- [ ] const `suprnova_live::async_updates::MAX_REPLAY_TRANSCRIPT_ENVELOPES` · crates/suprnova-live/src/async_updates/sequence.rs:14

### `suprnova_live::async_updates::sse` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::async_updates::SseEncoder` · crates/suprnova-live/src/async_updates/sse.rs:65
  - [ ] fn `suprnova_live::async_updates::SseEncoder::encode_envelope` · crates/suprnova-live/src/async_updates/sse.rs:69
  - [ ] fn `suprnova_live::async_updates::SseEncoder::heartbeat_comment` · crates/suprnova-live/src/async_updates/sse.rs:97
- [ ] struct `suprnova_live::async_updates::SseEvent` · crates/suprnova-live/src/async_updates/sse.rs:17
  - [ ] fn `suprnova_live::async_updates::SseEvent::id` · crates/suprnova-live/src/async_updates/sse.rs:30
  - [ ] fn `suprnova_live::async_updates::SseEvent::event` · crates/suprnova-live/src/async_updates/sse.rs:36
  - [ ] fn `suprnova_live::async_updates::SseEvent::data` · crates/suprnova-live/src/async_updates/sse.rs:42
  - [ ] fn `suprnova_live::async_updates::SseEvent::as_bytes` · crates/suprnova-live/src/async_updates/sse.rs:48
- [ ] struct `suprnova_live::async_updates::SseMembershipControl` · crates/suprnova-live/src/async_updates/sse.rs:128
  - [ ] fn `suprnova_live::async_updates::SseMembershipControl::prepare_subscribe` · crates/suprnova-live/src/async_updates/sse.rs:132
  - [ ] fn `suprnova_live::async_updates::SseMembershipControl::prepare_unsubscribe` · crates/suprnova-live/src/async_updates/sse.rs:143
- [ ] struct `suprnova_live::async_updates::SseResponseContract` · crates/suprnova-live/src/async_updates/sse.rs:103
  - [ ] fn `suprnova_live::async_updates::SseResponseContract::headers` · crates/suprnova-live/src/async_updates/sse.rs:108

### `suprnova_live::async_updates::subscription` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::async_updates::AuthorizationMemo` · crates/suprnova-live/src/async_updates/subscription.rs:251
  - [ ] fn `suprnova_live::async_updates::AuthorizationMemo::parse` · crates/suprnova-live/src/async_updates/subscription.rs:255
  - [ ] fn `suprnova_live::async_updates::AuthorizationMemo::as_str` · crates/suprnova-live/src/async_updates/subscription.rs:269
- [ ] struct `suprnova_live::async_updates::BoundedEventContracts` · crates/suprnova-live/src/async_updates/subscription.rs:431
  - [ ] fn `suprnova_live::async_updates::BoundedEventContracts::new` · crates/suprnova-live/src/async_updates/subscription.rs:435
  - [ ] fn `suprnova_live::async_updates::BoundedEventContracts::as_slice` · crates/suprnova-live/src/async_updates/subscription.rs:455
- [ ] struct `suprnova_live::async_updates::CapabilityVersion` · crates/suprnova-live/src/async_updates/subscription.rs:166
  - [ ] fn `suprnova_live::async_updates::CapabilityVersion::new` · crates/suprnova-live/src/async_updates/subscription.rs:170
  - [ ] fn `suprnova_live::async_updates::CapabilityVersion::get` · crates/suprnova-live/src/async_updates/subscription.rs:181
- [ ] struct `suprnova_live::async_updates::PollFallbackPolicy` · crates/suprnova-live/src/async_updates/subscription.rs:300
  - [ ] fn `suprnova_live::async_updates::PollFallbackPolicy::new` · crates/suprnova-live/src/async_updates/subscription.rs:462
  - [ ] fn `suprnova_live::async_updates::PollFallbackPolicy::interval_ms` · crates/suprnova-live/src/async_updates/subscription.rs:485
  - [ ] fn `suprnova_live::async_updates::PollFallbackPolicy::jitter_basis_points` · crates/suprnova-live/src/async_updates/subscription.rs:491
  - [ ] fn `suprnova_live::async_updates::PollFallbackPolicy::initial` · crates/suprnova-live/src/async_updates/subscription.rs:497
  - [ ] fn `suprnova_live::async_updates::PollFallbackPolicy::visibility` · crates/suprnova-live/src/async_updates/subscription.rs:503
- [ ] struct `suprnova_live::async_updates::StreamEpoch` · crates/suprnova-live/src/async_updates/subscription.rs:188
  - [ ] fn `suprnova_live::async_updates::StreamEpoch::new` · crates/suprnova-live/src/async_updates/subscription.rs:193
  - [ ] fn `suprnova_live::async_updates::StreamEpoch::get` · crates/suprnova-live/src/async_updates/subscription.rs:199
- [ ] struct `suprnova_live::async_updates::StreamPosition` · crates/suprnova-live/src/async_updates/subscription.rs:224
  - [ ] fn `suprnova_live::async_updates::StreamPosition::new` · crates/suprnova-live/src/async_updates/subscription.rs:232
  - [ ] fn `suprnova_live::async_updates::StreamPosition::epoch` · crates/suprnova-live/src/async_updates/subscription.rs:238
  - [ ] fn `suprnova_live::async_updates::StreamPosition::sequence` · crates/suprnova-live/src/async_updates/subscription.rs:244
- [ ] struct `suprnova_live::async_updates::StreamSequence` · crates/suprnova-live/src/async_updates/subscription.rs:206
  - [ ] fn `suprnova_live::async_updates::StreamSequence::new` · crates/suprnova-live/src/async_updates/subscription.rs:211
  - [ ] fn `suprnova_live::async_updates::StreamSequence::get` · crates/suprnova-live/src/async_updates/subscription.rs:217
- [ ] struct `suprnova_live::async_updates::SubscriptionClaims` · crates/suprnova-live/src/async_updates/subscription.rs:510
  - [ ] fn `suprnova_live::async_updates::SubscriptionClaims::new` · crates/suprnova-live/src/async_updates/subscription.rs:529
  - [ ] fn `suprnova_live::async_updates::SubscriptionClaims::stream` · crates/suprnova-live/src/async_updates/subscription.rs:558
  - [ ] fn `suprnova_live::async_updates::SubscriptionClaims::protocol` · crates/suprnova-live/src/async_updates/subscription.rs:564
  - [ ] fn `suprnova_live::async_updates::SubscriptionClaims::capability` · crates/suprnova-live/src/async_updates/subscription.rs:570
  - [ ] fn `suprnova_live::async_updates::SubscriptionClaims::topics` · crates/suprnova-live/src/async_updates/subscription.rs:576
  - [ ] fn `suprnova_live::async_updates::SubscriptionClaims::events` · crates/suprnova-live/src/async_updates/subscription.rs:582
  - [ ] fn `suprnova_live::async_updates::SubscriptionClaims::authorization_memo` · crates/suprnova-live/src/async_updates/subscription.rs:588
  - [ ] fn `suprnova_live::async_updates::SubscriptionClaims::baseline` · crates/suprnova-live/src/async_updates/subscription.rs:594
  - [ ] fn `suprnova_live::async_updates::SubscriptionClaims::expires_at` · crates/suprnova-live/src/async_updates/subscription.rs:600
  - [ ] fn `suprnova_live::async_updates::SubscriptionClaims::reconnect` · crates/suprnova-live/src/async_updates/subscription.rs:606
  - [ ] fn `suprnova_live::async_updates::SubscriptionClaims::fallback_poll` · crates/suprnova-live/src/async_updates/subscription.rs:612
- [ ] struct `suprnova_live::async_updates::SubscriptionDescriptor` · crates/suprnova-live/src/async_updates/subscription.rs:619
  - [ ] fn `suprnova_live::async_updates::SubscriptionDescriptor::parse` · crates/suprnova-live/src/async_updates/subscription.rs:623
  - [ ] fn `suprnova_live::async_updates::SubscriptionDescriptor::as_str` · crates/suprnova-live/src/async_updates/subscription.rs:630
- [ ] struct `suprnova_live::async_updates::SubscriptionDescriptorCodec` · crates/suprnova-live/src/async_updates/subscription.rs:722
  - [ ] fn `suprnova_live::async_updates::SubscriptionDescriptorCodec::new` · crates/suprnova-live/src/async_updates/subscription.rs:729
  - [ ] fn `suprnova_live::async_updates::SubscriptionDescriptorCodec::sign` · crates/suprnova-live/src/async_updates/subscription.rs:734
  - [ ] fn `suprnova_live::async_updates::SubscriptionDescriptorCodec::verify` · crates/suprnova-live/src/async_updates/subscription.rs:755
- [ ] struct `suprnova_live::async_updates::SubscriptionError` · crates/suprnova-live/src/async_updates/subscription.rs:132
  - [ ] fn `suprnova_live::async_updates::SubscriptionError::new` · crates/suprnova-live/src/async_updates/subscription.rs:139
  - [ ] fn `suprnova_live::async_updates::SubscriptionError::kind` · crates/suprnova-live/src/async_updates/subscription.rs:145
- [ ] struct `suprnova_live::async_updates::SubscriptionEventContract` · crates/suprnova-live/src/async_updates/subscription.rs:309
  - [ ] fn `suprnova_live::async_updates::SubscriptionEventContract::from_registered` · crates/suprnova-live/src/async_updates/subscription.rs:323
  - [ ] fn `suprnova_live::async_updates::SubscriptionEventContract::name` · crates/suprnova-live/src/async_updates/subscription.rs:376
  - [ ] fn `suprnova_live::async_updates::SubscriptionEventContract::version` · crates/suprnova-live/src/async_updates/subscription.rs:382
  - [ ] fn `suprnova_live::async_updates::SubscriptionEventContract::payload_contract` · crates/suprnova-live/src/async_updates/subscription.rs:388
  - [ ] fn `suprnova_live::async_updates::SubscriptionEventContract::schema` · crates/suprnova-live/src/async_updates/subscription.rs:394
  - [ ] fn `suprnova_live::async_updates::SubscriptionEventContract::source` · crates/suprnova-live/src/async_updates/subscription.rs:400
  - [ ] fn `suprnova_live::async_updates::SubscriptionEventContract::targets` · crates/suprnova-live/src/async_updates/subscription.rs:406
  - [ ] fn `suprnova_live::async_updates::SubscriptionEventContract::order` · crates/suprnova-live/src/async_updates/subscription.rs:412
  - [ ] fn `suprnova_live::async_updates::SubscriptionEventContract::cycle` · crates/suprnova-live/src/async_updates/subscription.rs:418
  - [ ] fn `suprnova_live::async_updates::SubscriptionEventContract::maximum_fanout` · crates/suprnova-live/src/async_updates/subscription.rs:424
- [ ] struct `suprnova_live::async_updates::TransportCredential` · crates/suprnova-live/src/async_updates/subscription.rs:647
  - [ ] fn `suprnova_live::async_updates::TransportCredential::from_host_authority_bearer` · crates/suprnova-live/src/async_updates/subscription.rs:651
  - [ ] fn `suprnova_live::async_updates::TransportCredential::from_zeroizing_host_authority_bearer` · crates/suprnova-live/src/async_updates/subscription.rs:656
  - [ ] fn `suprnova_live::async_updates::TransportCredential::expose_authorization_bearer` · crates/suprnova-live/src/async_updates/subscription.rs:674
- [ ] struct `suprnova_live::async_updates::VerifiedSubscriptionDescriptor` · crates/suprnova-live/src/async_updates/subscription.rs:687
  - [ ] fn `suprnova_live::async_updates::VerifiedSubscriptionDescriptor::claims` · crates/suprnova-live/src/async_updates/subscription.rs:698
  - [ ] fn `suprnova_live::async_updates::VerifiedSubscriptionDescriptor::baseline` · crates/suprnova-live/src/async_updates/subscription.rs:704
  - [ ] fn `suprnova_live::async_updates::VerifiedSubscriptionDescriptor::expires_at` · crates/suprnova-live/src/async_updates/subscription.rs:710
- [ ] enum `suprnova_live::async_updates::PollInitialBehavior` · crates/suprnova-live/src/async_updates/subscription.rs:282
  - Variants: `Immediate`, `AfterInterval`
- [ ] enum `suprnova_live::async_updates::PollVisibilityPolicy` · crates/suprnova-live/src/async_updates/subscription.rs:291
  - Variants: `PauseWhenHidden`, `ContinueWhenHidden`
- [ ] enum `suprnova_live::async_updates::SubscriptionErrorKind` · crates/suprnova-live/src/async_updates/subscription.rs:70
  - Variants: `InvalidDescriptor`, `DescriptorExpired`, `UnsupportedProtocol`, `InvalidCapability`, `InvalidAuthorizationMemo`, `InvalidPollFallback`, `InvalidReconnectPolicy`, `DescriptorBudgetExceeded`, `ScopeMismatch`, `AuthorizationUnavailable`, `AuthorizationDenied`, `CredentialUnavailable`, `CredentialRotationUncertain`, `InvalidCredential`, `ContextExpired`, `UnregisteredSubscription`
  - [ ] fn `suprnova_live::async_updates::SubscriptionErrorKind::as_str` · crates/suprnova-live/src/async_updates/subscription.rs:108
- [ ] const `suprnova_live::async_updates::ASYNC_SUBSCRIPTION_PROTOCOL_V1` · crates/suprnova-live/src/async_updates/subscription.rs:24
- [ ] const `suprnova_live::async_updates::MAX_CANONICAL_SUBSCRIPTION_CLAIMS_BYTES` · crates/suprnova-live/src/async_updates/subscription.rs:50
- [ ] const `suprnova_live::async_updates::MAX_POLL_INTERVAL_MS` · crates/suprnova-live/src/async_updates/subscription.rs:28
- [ ] const `suprnova_live::async_updates::MAX_POLL_JITTER_BASIS_POINTS` · crates/suprnova-live/src/async_updates/subscription.rs:30
- [ ] const `suprnova_live::async_updates::MAX_RECONNECT_ATTEMPTS` · crates/suprnova-live/src/async_updates/subscription.rs:32
- [ ] const `suprnova_live::async_updates::MAX_SUBSCRIPTION_DESCRIPTOR_BYTES` · crates/suprnova-live/src/async_updates/subscription.rs:42
- [ ] const `suprnova_live::async_updates::MAX_SUBSCRIPTION_LIFETIME_MS` · crates/suprnova-live/src/async_updates/subscription.rs:34
- [ ] const `suprnova_live::async_updates::MIN_POLL_INTERVAL_MS` · crates/suprnova-live/src/async_updates/subscription.rs:26

### `suprnova_live::async_updates::telemetry` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::async_updates::AsyncTelemetrySnapshot` · crates/suprnova-live/src/async_updates/telemetry.rs:70
  - [ ] fn `suprnova_live::async_updates::AsyncTelemetrySnapshot::count` · crates/suprnova-live/src/async_updates/telemetry.rs:77
- [ ] enum `suprnova_live::async_updates::AsyncTelemetryCounter` · crates/suprnova-live/src/async_updates/telemetry.rs:10
  - Variants: `Queued`, `Coalesced`, `Degraded`, `Closed`, `Rejected`, `Cleanup`
  - [ ] const `suprnova_live::async_updates::AsyncTelemetryCounter::ALL` · crates/suprnova-live/src/async_updates/telemetry.rs:27
  - [ ] fn `suprnova_live::async_updates::AsyncTelemetryCounter::as_str` · crates/suprnova-live/src/async_updates/telemetry.rs:38

### `suprnova_live::async_updates::transport` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::async_updates::AsyncDeliveryError` · crates/suprnova-live/src/async_updates/transport.rs:208
  - [ ] fn `suprnova_live::async_updates::AsyncDeliveryError::kind` · crates/suprnova-live/src/async_updates/transport.rs:234
  - [ ] fn `suprnova_live::async_updates::AsyncDeliveryError::replay_error` · crates/suprnova-live/src/async_updates/transport.rs:240
- [ ] struct `suprnova_live::async_updates::AsyncTransportAuthorityRequest` · crates/suprnova-live/src/async_updates/transport.rs:453
  - [ ] fn `suprnova_live::async_updates::AsyncTransportAuthorityRequest::operation` · crates/suprnova-live/src/async_updates/transport.rs:467
  - [ ] fn `suprnova_live::async_updates::AsyncTransportAuthorityRequest::descriptor` · crates/suprnova-live/src/async_updates/transport.rs:473
  - [ ] fn `suprnova_live::async_updates::AsyncTransportAuthorityRequest::binding` · crates/suprnova-live/src/async_updates/transport.rs:479
  - [ ] fn `suprnova_live::async_updates::AsyncTransportAuthorityRequest::subscription` · crates/suprnova-live/src/async_updates/transport.rs:485
  - [ ] fn `suprnova_live::async_updates::AsyncTransportAuthorityRequest::document_scope` · crates/suprnova-live/src/async_updates/transport.rs:491
  - [ ] fn `suprnova_live::async_updates::AsyncTransportAuthorityRequest::document_origin` · crates/suprnova-live/src/async_updates/transport.rs:497
  - [ ] fn `suprnova_live::async_updates::AsyncTransportAuthorityRequest::document_kind` · crates/suprnova-live/src/async_updates/transport.rs:503
  - [ ] fn `suprnova_live::async_updates::AsyncTransportAuthorityRequest::document_handle` · crates/suprnova-live/src/async_updates/transport.rs:509
- [ ] struct `suprnova_live::async_updates::AsyncTransportAuthorityValidation` · crates/suprnova-live/src/async_updates/transport.rs:537
  - [ ] fn `suprnova_live::async_updates::AsyncTransportAuthorityValidation::accept_current` · crates/suprnova-live/src/async_updates/transport.rs:550
- [ ] struct `suprnova_live::async_updates::AsyncTransportError` · crates/suprnova-live/src/async_updates/transport.rs:169
  - [ ] fn `suprnova_live::async_updates::AsyncTransportError::new` · crates/suprnova-live/src/async_updates/transport.rs:176
  - [ ] fn `suprnova_live::async_updates::AsyncTransportError::kind` · crates/suprnova-live/src/async_updates/transport.rs:182
- [ ] struct `suprnova_live::async_updates::AuthorizedTransportAdd` · crates/suprnova-live/src/async_updates/transport.rs:958
- [ ] struct `suprnova_live::async_updates::AuthorizedTransportSubscription` · crates/suprnova-live/src/async_updates/transport.rs:686
  - [ ] fn `suprnova_live::async_updates::AuthorizedTransportSubscription::new` · crates/suprnova-live/src/async_updates/transport.rs:707
  - [ ] fn `suprnova_live::async_updates::AuthorizedTransportSubscription::subscription` · crates/suprnova-live/src/async_updates/transport.rs:737
  - [ ] fn `suprnova_live::async_updates::AuthorizedTransportSubscription::baseline` · crates/suprnova-live/src/async_updates/transport.rs:743
  - [ ] fn `suprnova_live::async_updates::AuthorizedTransportSubscription::context` · crates/suprnova-live/src/async_updates/transport.rs:749
  - [ ] fn `suprnova_live::async_updates::AuthorizedTransportSubscription::origin` · crates/suprnova-live/src/async_updates/transport.rs:755
  - [ ] fn `suprnova_live::async_updates::AuthorizedTransportSubscription::binding` · crates/suprnova-live/src/async_updates/transport.rs:761
  - [ ] fn `suprnova_live::async_updates::AuthorizedTransportSubscription::document_scope` · crates/suprnova-live/src/async_updates/transport.rs:767
- [ ] struct `suprnova_live::async_updates::BoundedDocumentTransportSession` · crates/suprnova-live/src/async_updates/transport.rs:1880
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::new` · crates/suprnova-live/src/async_updates/transport.rs:1910
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::transport` · crates/suprnova-live/src/async_updates/transport.rs:1935
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::pump_next` · crates/suprnova-live/src/async_updates/transport.rs:1940
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::admit_replay` · crates/suprnova-live/src/async_updates/transport.rs:2086
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::dispatch_next` · crates/suprnova-live/src/async_updates/transport.rs:2195
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::retained_events` · crates/suprnova-live/src/async_updates/transport.rs:2356
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::retained_bytes` · crates/suprnova-live/src/async_updates/transport.rs:2362
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::active_permits` · crates/suprnova-live/src/async_updates/transport.rs:2368
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::is_degraded` · crates/suprnova-live/src/async_updates/transport.rs:2374
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::unresolved_pressure_cause_count` · crates/suprnova-live/src/async_updates/transport.rs:2384
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::telemetry_snapshot` · crates/suprnova-live/src/async_updates/transport.rs:2390
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::sequence_position` · crates/suprnova-live/src/async_updates/transport.rs:2396
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::sequence_state` · crates/suprnova-live/src/async_updates/transport.rs:2408
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::recover_from_authoritative_refresh` · crates/suprnova-live/src/async_updates/transport.rs:2419
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::delivery_lane_count` · crates/suprnova-live/src/async_updates/transport.rs:2480
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::terminal_drain_count` · crates/suprnova-live/src/async_updates/transport.rs:2486
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::prepare_add` · crates/suprnova-live/src/async_updates/transport.rs:2491
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::prepare_establish` · crates/suprnova-live/src/async_updates/transport.rs:2499
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::commit_add` · crates/suprnova-live/src/async_updates/transport.rs:2507
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::prepare_remove` · crates/suprnova-live/src/async_updates/transport.rs:2527
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::commit_remove` · crates/suprnova-live/src/async_updates/transport.rs:2535
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::close` · crates/suprnova-live/src/async_updates/transport.rs:2551
  - [ ] fn `suprnova_live::async_updates::BoundedDocumentTransportSession::retire_delivery` · crates/suprnova-live/src/async_updates/transport.rs:2559
- [ ] struct `suprnova_live::async_updates::DocumentAuthorizationScope` · crates/suprnova-live/src/async_updates/transport.rs:44
  - [ ] fn `suprnova_live::async_updates::DocumentAuthorizationScope::derive` · crates/suprnova-live/src/async_updates/transport.rs:48
  - [ ] fn `suprnova_live::async_updates::DocumentAuthorizationScope::to_base64url` · crates/suprnova-live/src/async_updates/transport.rs:76
- [ ] struct `suprnova_live::async_updates::DocumentTransportHandle` · crates/suprnova-live/src/async_updates/transport.rs:369
  - [ ] fn `suprnova_live::async_updates::DocumentTransportHandle::from_bytes` · crates/suprnova-live/src/async_updates/transport.rs:373
  - [ ] fn `suprnova_live::async_updates::DocumentTransportHandle::parse` · crates/suprnova-live/src/async_updates/transport.rs:383
  - [ ] fn `suprnova_live::async_updates::DocumentTransportHandle::to_base64url` · crates/suprnova-live/src/async_updates/transport.rs:407
- [ ] struct `suprnova_live::async_updates::DocumentTransportLimits` · crates/suprnova-live/src/async_updates/transport.rs:420
  - [ ] fn `suprnova_live::async_updates::DocumentTransportLimits::new` · crates/suprnova-live/src/async_updates/transport.rs:616
  - [ ] fn `suprnova_live::async_updates::DocumentTransportLimits::max_memberships` · crates/suprnova-live/src/async_updates/transport.rs:627
- [ ] struct `suprnova_live::async_updates::DocumentTransportSession` · crates/suprnova-live/src/async_updates/transport.rs:1104
  - [ ] fn `suprnova_live::async_updates::DocumentTransportSession::new` · crates/suprnova-live/src/async_updates/transport.rs:1127
  - [ ] fn `suprnova_live::async_updates::DocumentTransportSession::origin` · crates/suprnova-live/src/async_updates/transport.rs:1157
  - [ ] fn `suprnova_live::async_updates::DocumentTransportSession::kind` · crates/suprnova-live/src/async_updates/transport.rs:1163
  - [ ] fn `suprnova_live::async_updates::DocumentTransportSession::handle` · crates/suprnova-live/src/async_updates/transport.rs:1169
  - [ ] fn `suprnova_live::async_updates::DocumentTransportSession::membership_count` · crates/suprnova-live/src/async_updates/transport.rs:1175
  - [ ] fn `suprnova_live::async_updates::DocumentTransportSession::retiring_count` · crates/suprnova-live/src/async_updates/transport.rs:1181
  - [ ] fn `suprnova_live::async_updates::DocumentTransportSession::last_cleanup_error` · crates/suprnova-live/src/async_updates/transport.rs:1187
  - [ ] fn `suprnova_live::async_updates::DocumentTransportSession::contains_membership` · crates/suprnova-live/src/async_updates/transport.rs:1193
  - [ ] fn `suprnova_live::async_updates::DocumentTransportSession::prepare_add` · crates/suprnova-live/src/async_updates/transport.rs:1351
  - [ ] fn `suprnova_live::async_updates::DocumentTransportSession::prepare_establish` · crates/suprnova-live/src/async_updates/transport.rs:1365
  - [ ] fn `suprnova_live::async_updates::DocumentTransportSession::commit_add` · crates/suprnova-live/src/async_updates/transport.rs:1386
  - [ ] fn `suprnova_live::async_updates::DocumentTransportSession::prepare_remove` · crates/suprnova-live/src/async_updates/transport.rs:1411
  - [ ] fn `suprnova_live::async_updates::DocumentTransportSession::commit_remove` · crates/suprnova-live/src/async_updates/transport.rs:1428
  - [ ] fn `suprnova_live::async_updates::DocumentTransportSession::close` · crates/suprnova-live/src/async_updates/transport.rs:1494
- [ ] struct `suprnova_live::async_updates::EstablishingTransportAdd` · crates/suprnova-live/src/async_updates/transport.rs:971
  - [ ] fn `suprnova_live::async_updates::EstablishingTransportAdd::establish` · crates/suprnova-live/src/async_updates/transport.rs:979
- [ ] struct `suprnova_live::async_updates::PendingTransportAdd` · crates/suprnova-live/src/async_updates/transport.rs:931
  - [ ] fn `suprnova_live::async_updates::PendingTransportAdd::authorize` · crates/suprnova-live/src/async_updates/transport.rs:939
- [ ] struct `suprnova_live::async_updates::PendingTransportRemove` · crates/suprnova-live/src/async_updates/transport.rs:1031
  - [ ] fn `suprnova_live::async_updates::PendingTransportRemove::authorize` · crates/suprnova-live/src/async_updates/transport.rs:1039
- [ ] struct `suprnova_live::async_updates::ReadyTransportAdd` · crates/suprnova-live/src/async_updates/transport.rs:1017
- [ ] struct `suprnova_live::async_updates::ReadyTransportRemove` · crates/suprnova-live/src/async_updates/transport.rs:1061
- [ ] struct `suprnova_live::async_updates::VerifiedOrigin` · crates/suprnova-live/src/async_updates/transport.rs:276
  - [ ] fn `suprnova_live::async_updates::VerifiedOrigin::parse` · crates/suprnova-live/src/async_updates/transport.rs:284
  - [ ] fn `suprnova_live::async_updates::VerifiedOrigin::scheme` · crates/suprnova-live/src/async_updates/transport.rs:329
  - [ ] fn `suprnova_live::async_updates::VerifiedOrigin::host` · crates/suprnova-live/src/async_updates/transport.rs:335
  - [ ] fn `suprnova_live::async_updates::VerifiedOrigin::port` · crates/suprnova-live/src/async_updates/transport.rs:341
- [ ] enum `suprnova_live::async_updates::AsyncDeliveryDisposition` · crates/suprnova-live/src/async_updates/transport.rs:267
  - Variants: `Sequence`, `Replay`
- [ ] enum `suprnova_live::async_updates::AsyncDeliveryErrorKind` · crates/suprnova-live/src/async_updates/transport.rs:197
  - Variants: `Retired`, `AuthorizationLost`, `Sequence`
- [ ] enum `suprnova_live::async_updates::AsyncTransportErrorKind` · crates/suprnova-live/src/async_updates/transport.rs:104
  - Variants: `InvalidOrigin`, `OriginMismatch`, `TransportMismatch`, `AuthorizationScopeMismatch`, `AuthorizationLost`, `DuplicateMembership`, `UnknownMembership`, `DescriptorMismatch`, `StaleControl`, `MembershipLimit`, `BaselineMismatch`, `RoutingMismatch`, `InvalidEnvelope`, `FrameTooLarge`, `UnsupportedFrame`, `SourceFailed`, `Closed`
  - [ ] fn `suprnova_live::async_updates::AsyncTransportErrorKind::as_str` · crates/suprnova-live/src/async_updates/transport.rs:144
- [ ] enum `suprnova_live::async_updates::CloseDisposition` · crates/suprnova-live/src/async_updates/transport.rs:92
  - Variants: `Closed`, `AlreadyClosed`
- [ ] enum `suprnova_live::async_updates::DocumentTransportKind` · crates/suprnova-live/src/async_updates/transport.rs:426
  - Variants: `ServerSentEvents`, `WebSocket`
- [ ] enum `suprnova_live::async_updates::TransportMembershipOperation` · crates/suprnova-live/src/async_updates/transport.rs:444
  - Variants: `Subscribe`, `Unsubscribe`
- [ ] trait `suprnova_live::async_updates::AsyncEventSession` · crates/suprnova-live/src/async_updates/transport.rs:840
  - [ ] fn `suprnova_live::async_updates::AsyncEventSession::baseline` · crates/suprnova-live/src/async_updates/transport.rs:842 (required)
  - [ ] fn `suprnova_live::async_updates::AsyncEventSession::poll_next` · crates/suprnova-live/src/async_updates/transport.rs:845 (required)
  - [ ] fn `suprnova_live::async_updates::AsyncEventSession::poll_close` · crates/suprnova-live/src/async_updates/transport.rs:855 (required)
- [ ] trait `suprnova_live::async_updates::AsyncEventSource` · crates/suprnova-live/src/async_updates/transport.rs:823
  - [ ] fn `suprnova_live::async_updates::AsyncEventSource::subscribe` · crates/suprnova-live/src/async_updates/transport.rs:828 (required)
- [ ] trait `suprnova_live::async_updates::AsyncTransportAuthorityPort` · crates/suprnova-live/src/async_updates/transport.rs:602
  - [ ] fn `suprnova_live::async_updates::AsyncTransportAuthorityPort::now` · crates/suprnova-live/src/async_updates/transport.rs:604 (required)
  - [ ] fn `suprnova_live::async_updates::AsyncTransportAuthorityPort::validate_current` · crates/suprnova-live/src/async_updates/transport.rs:607 (required)
- [ ] type `suprnova_live::async_updates::AsyncTransportFuture` · crates/suprnova-live/src/async_updates/transport.rs:88
- [ ] const `suprnova_live::async_updates::MAX_DOCUMENT_TRANSPORT_MEMBERSHIPS` · crates/suprnova-live/src/async_updates/transport.rs:36

### `suprnova_live::async_updates::websocket` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::async_updates::AuthorizedWebSocketMembershipAdd` · crates/suprnova-live/src/async_updates/websocket.rs:162
  - [ ] fn `suprnova_live::async_updates::AuthorizedWebSocketMembershipAdd::prepare_establish` · crates/suprnova-live/src/async_updates/websocket.rs:169
- [ ] struct `suprnova_live::async_updates::AuthorizedWebSocketUpgrade` · crates/suprnova-live/src/async_updates/websocket.rs:390
  - [ ] fn `suprnova_live::async_updates::AuthorizedWebSocketUpgrade::origin` · crates/suprnova-live/src/async_updates/websocket.rs:399
  - [ ] fn `suprnova_live::async_updates::AuthorizedWebSocketUpgrade::is_cross_origin` · crates/suprnova-live/src/async_updates/websocket.rs:405
  - [ ] fn `suprnova_live::async_updates::AuthorizedWebSocketUpgrade::into_authority` · crates/suprnova-live/src/async_updates/websocket.rs:411
- [ ] struct `suprnova_live::async_updates::EstablishingWebSocketMembershipAdd` · crates/suprnova-live/src/async_updates/websocket.rs:189
  - [ ] fn `suprnova_live::async_updates::EstablishingWebSocketMembershipAdd::establish` · crates/suprnova-live/src/async_updates/websocket.rs:196
- [ ] struct `suprnova_live::async_updates::PendingWebSocketMembershipAdd` · crates/suprnova-live/src/async_updates/websocket.rs:138
  - [ ] fn `suprnova_live::async_updates::PendingWebSocketMembershipAdd::authorize` · crates/suprnova-live/src/async_updates/websocket.rs:145
- [ ] struct `suprnova_live::async_updates::ReadyWebSocketMembershipAdd` · crates/suprnova-live/src/async_updates/websocket.rs:216
- [ ] struct `suprnova_live::async_updates::WebSocketCodec` · crates/suprnova-live/src/async_updates/websocket.rs:499
  - [ ] fn `suprnova_live::async_updates::WebSocketCodec::v1` · crates/suprnova-live/src/async_updates/websocket.rs:506
  - [ ] fn `suprnova_live::async_updates::WebSocketCodec::encode_envelope` · crates/suprnova-live/src/async_updates/websocket.rs:513
  - [ ] fn `suprnova_live::async_updates::WebSocketCodec::decode_envelope` · crates/suprnova-live/src/async_updates/websocket.rs:522
  - [ ] fn `suprnova_live::async_updates::WebSocketCodec::encode_control` · crates/suprnova-live/src/async_updates/websocket.rs:533
  - [ ] fn `suprnova_live::async_updates::WebSocketCodec::decode_control` · crates/suprnova-live/src/async_updates/websocket.rs:547
  - [ ] fn `suprnova_live::async_updates::WebSocketCodec::decode_membership_request` · crates/suprnova-live/src/async_updates/websocket.rs:564
  - [ ] fn `suprnova_live::async_updates::WebSocketCodec::encode_membership_acknowledgment` · crates/suprnova-live/src/async_updates/websocket.rs:582
- [ ] struct `suprnova_live::async_updates::WebSocketMembershipAcknowledgment` · crates/suprnova-live/src/async_updates/websocket.rs:115
- [ ] struct `suprnova_live::async_updates::WebSocketMembershipCommitReceipt` · crates/suprnova-live/src/async_updates/websocket.rs:248
- [ ] struct `suprnova_live::async_updates::WebSocketMembershipControl` · crates/suprnova-live/src/async_updates/websocket.rs:259
  - [ ] fn `suprnova_live::async_updates::WebSocketMembershipControl::prepare_authenticated_subscribe` · crates/suprnova-live/src/async_updates/websocket.rs:263
  - [ ] fn `suprnova_live::async_updates::WebSocketMembershipControl::commit_authenticated_subscribe` · crates/suprnova-live/src/async_updates/websocket.rs:285
  - [ ] fn `suprnova_live::async_updates::WebSocketMembershipControl::commit_authenticated_bounded_subscribe` · crates/suprnova-live/src/async_updates/websocket.rs:297
  - [ ] fn `suprnova_live::async_updates::WebSocketMembershipControl::acknowledge_committed` · crates/suprnova-live/src/async_updates/websocket.rs:310
  - [ ] fn `suprnova_live::async_updates::WebSocketMembershipControl::prepare_subscribe` · crates/suprnova-live/src/async_updates/websocket.rs:324
  - [ ] fn `suprnova_live::async_updates::WebSocketMembershipControl::prepare_unsubscribe` · crates/suprnova-live/src/async_updates/websocket.rs:343
- [ ] struct `suprnova_live::async_updates::WebSocketMembershipRequest` · crates/suprnova-live/src/async_updates/websocket.rs:72
  - [ ] fn `suprnova_live::async_updates::WebSocketMembershipRequest::control_nonce` · crates/suprnova-live/src/async_updates/websocket.rs:83
  - [ ] fn `suprnova_live::async_updates::WebSocketMembershipRequest::subscription` · crates/suprnova-live/src/async_updates/websocket.rs:89
  - [ ] fn `suprnova_live::async_updates::WebSocketMembershipRequest::transport_generation` · crates/suprnova-live/src/async_updates/websocket.rs:95
- [ ] struct `suprnova_live::async_updates::WebSocketOriginPolicy` · crates/suprnova-live/src/async_updates/websocket.rs:428
  - [ ] fn `suprnova_live::async_updates::WebSocketOriginPolicy::new` · crates/suprnova-live/src/async_updates/websocket.rs:435
  - [ ] fn `suprnova_live::async_updates::WebSocketOriginPolicy::authorize_upgrade` · crates/suprnova-live/src/async_updates/websocket.rs:463
- [ ] enum `suprnova_live::async_updates::WebSocketAuthentication` · crates/suprnova-live/src/async_updates/websocket.rs:382
  - Variants: `Cookie`, `SeparateCredential`
- [ ] enum `suprnova_live::async_updates::WebSocketControlRecord` · crates/suprnova-live/src/async_updates/websocket.rs:63
  - Variants: `Subscribe`, `Unsubscribe`
  - [ ] fn `suprnova_live::async_updates::WebSocketControlRecord::subscription` · crates/suprnova-live/src/async_updates/websocket.rs:374
- [ ] enum `suprnova_live::async_updates::WebSocketFrame` · crates/suprnova-live/src/async_updates/websocket.rs:24
  - Variants: `Text`, `Binary`, `Continuation`

## canonical

### `suprnova_live::canonical::parser` (private module; items are public through re-exports)

- [ ] fn `suprnova_live::canonical::parse_canonical_value` · crates/suprnova-live/src/canonical/parser.rs:192

### `suprnova_live::canonical::serializer` (private module; items are public through re-exports)

- [ ] fn `suprnova_live::canonical::to_canonical_bytes` · crates/suprnova-live/src/canonical/serializer.rs:89

### `suprnova_live::canonical::value` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::canonical::CanonicalError` · crates/suprnova-live/src/canonical/value.rs:56
  - [ ] fn `suprnova_live::canonical::CanonicalError::kind` · crates/suprnova-live/src/canonical/value.rs:67
- [ ] struct `suprnova_live::canonical::CanonicalNumber` · crates/suprnova-live/src/canonical/value.rs:82
  - [ ] fn `suprnova_live::canonical::CanonicalNumber::new` · crates/suprnova-live/src/canonical/value.rs:86
  - [ ] fn `suprnova_live::canonical::CanonicalNumber::get` · crates/suprnova-live/src/canonical/value.rs:109
- [ ] enum `suprnova_live::canonical::CanonicalErrorKind` · crates/suprnova-live/src/canonical/value.rs:15
  - Variants: `TooLarge`, `TooDeep`, `TooManyEntries`, `StringTooLong`, `DuplicateKey`, `InvalidUtf8`, `InvalidNumber`, `InvalidJson`, `SerializationFailed`
  - [ ] fn `suprnova_live::canonical::CanonicalErrorKind::as_str` · crates/suprnova-live/src/canonical/value.rs:39
- [ ] enum `suprnova_live::canonical::CanonicalValue` · crates/suprnova-live/src/canonical/value.rs:125
  - Variants: `Null`, `Bool`, `Number`, `String`, `Array`, `Object`
  - [ ] fn `suprnova_live::canonical::CanonicalValue::number` · crates/suprnova-live/src/canonical/value.rs:142
- [ ] const `suprnova_live::canonical::MAX_SAFE_INTEGER` · crates/suprnova-live/src/canonical/value.rs:11

## checker

### `suprnova_live::checker`

- [ ] const `suprnova_live::checker::DIRECTIVE_GRAMMAR_VERSION` · crates/suprnova-live/src/checker/mod.rs:22
- [ ] const `suprnova_live::checker::VIEW_CHECKER_VERSION` · crates/suprnova-live/src/checker/mod.rs:25

### `suprnova_live::checker::diagnostic` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::checker::CheckReport` · crates/suprnova-live/src/checker/diagnostic.rs:216
  - [ ] fn `suprnova_live::checker::CheckReport::is_proved` · crates/suprnova-live/src/checker/diagnostic.rs:227
  - [ ] fn `suprnova_live::checker::CheckReport::diagnostics` · crates/suprnova-live/src/checker/diagnostic.rs:233
- [ ] struct `suprnova_live::checker::TemplateDiagnostic` · crates/suprnova-live/src/checker/diagnostic.rs:132
  - [ ] fn `suprnova_live::checker::TemplateDiagnostic::code` · crates/suprnova-live/src/checker/diagnostic.rs:162
  - [ ] fn `suprnova_live::checker::TemplateDiagnostic::severity` · crates/suprnova-live/src/checker/diagnostic.rs:168
  - [ ] fn `suprnova_live::checker::TemplateDiagnostic::path` · crates/suprnova-live/src/checker/diagnostic.rs:174
  - [ ] fn `suprnova_live::checker::TemplateDiagnostic::line` · crates/suprnova-live/src/checker/diagnostic.rs:180
  - [ ] fn `suprnova_live::checker::TemplateDiagnostic::column` · crates/suprnova-live/src/checker/diagnostic.rs:186
  - [ ] fn `suprnova_live::checker::TemplateDiagnostic::component` · crates/suprnova-live/src/checker/diagnostic.rs:192
- [ ] enum `suprnova_live::checker::DiagnosticCode` · crates/suprnova-live/src/checker/diagnostic.rs:10
  - Variants: `MissingView`, `MissingTemplate`, `AskamaSyntax`, `HtmlSyntax`, `BranchStackMismatch`, `DynamicStructureUnproved`, `RawSafe`, `UnknownDirective`, `ForbiddenLifecycle`, `UnknownAction`, `UnknownModel`, `ForbiddenModel`, `InvalidModifier`, `OwnershipViolation`, `UnknownComponent`, `InvalidKey`, `DuplicateKey`, `InvalidUrlBinding`, `UnknownEvent`, `UnknownEffect`, `AccessibilityViolation`, `SourceLimit`, `NodeLimit`, `IncludeDepthLimit`, `BranchLimit`, `HtmlTokenLimit`, `AttributeLimit`, `StackDepthLimit`, `DiagnosticLimit`, `SubmitProposalLimit`, `InvalidElementId`, `DuplicateElementId`
  - [ ] fn `suprnova_live::checker::DiagnosticCode::as_str` · crates/suprnova-live/src/checker/diagnostic.rs:83
- [ ] enum `suprnova_live::checker::DiagnosticSeverity` · crates/suprnova-live/src/checker/diagnostic.rs:123
  - Variants: `Error`, `Unproved`

### `suprnova_live::checker::generated_directive_contract` (private module; items are public through re-exports)

- [ ] fn `suprnova_live::checker::directive_contract` · crates/suprnova-live/src/checker/generated_directive_contract.rs:185
- [ ] fn `suprnova_live::checker::is_reserved_directive` · crates/suprnova-live/src/checker/generated_directive_contract.rs:192
- [ ] struct `suprnova_live::checker::DirectiveContract` · crates/suprnova-live/src/checker/generated_directive_contract.rs:92
  - Public fields: `name`, `owner`, `value`, `modifiers`, `modifier_conflicts`, `roles`, `conflicts`, `phase`, `fallback`, `capability`
- [ ] enum `suprnova_live::checker::DirectiveFallback` · crates/suprnova-live/src/checker/generated_directive_contract.rs:85
  - Variants: `Inert`, `Native`, `RetainDom`
- [ ] enum `suprnova_live::checker::DirectiveOwner` · crates/suprnova-live/src/checker/generated_directive_contract.rs:11
  - Variants: `Island`, `KeyedScope`, `Element`
- [ ] enum `suprnova_live::checker::DirectivePhase` · crates/suprnova-live/src/checker/generated_directive_contract.rs:76
  - Variants: `Local`, `Schedule`, `Feedback`, `Morph`, `Navigation`
- [ ] enum `suprnova_live::checker::DirectiveValue` · crates/suprnova-live/src/checker/generated_directive_contract.rs:18
  - Variants: `Empty`, `Identifier`, `Literal`, `Field`, `Action`, `Target`, `Mapping`
- [ ] const `suprnova_live::checker::DIRECTIVE_ARGUMENT_FORMS` · crates/suprnova-live/src/checker/generated_directive_contract.rs:180
- [ ] const `suprnova_live::checker::DIRECTIVE_CONTRACTS` · crates/suprnova-live/src/checker/generated_directive_contract.rs:125
- [ ] const `suprnova_live::checker::DIRECTIVE_FALLBACKS` · crates/suprnova-live/src/checker/generated_directive_contract.rs:182
- [ ] const `suprnova_live::checker::DIRECTIVE_FIXTURE_MANIFEST_SHA256` · crates/suprnova-live/src/checker/generated_directive_contract.rs:7
- [ ] const `suprnova_live::checker::DIRECTIVE_LITERAL_KINDS` · crates/suprnova-live/src/checker/generated_directive_contract.rs:178
- [ ] const `suprnova_live::checker::DIRECTIVE_TARGET_KINDS` · crates/suprnova-live/src/checker/generated_directive_contract.rs:176
- [ ] const `suprnova_live::checker::RESERVED_DIRECTIVES` · crates/suprnova-live/src/checker/generated_directive_contract.rs:174

### `suprnova_live::checker::limits` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::checker::CheckerConfigError` · crates/suprnova-live/src/checker/limits.rs:17
- [ ] struct `suprnova_live::checker::CheckerLimits` · crates/suprnova-live/src/checker/limits.rs:29
  - [ ] fn `suprnova_live::checker::CheckerLimits::new` · crates/suprnova-live/src/checker/limits.rs:46

### `suprnova_live::checker::template` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::checker::TemplateCatalog` · crates/suprnova-live/src/checker/template.rs:30
  - [ ] fn `suprnova_live::checker::TemplateCatalog::new` · crates/suprnova-live/src/checker/template.rs:36
- [ ] struct `suprnova_live::checker::TemplateCatalogError` · crates/suprnova-live/src/checker/template.rs:18
- [ ] struct `suprnova_live::checker::TemplateChecker` · crates/suprnova-live/src/checker/template.rs:68
  - [ ] fn `suprnova_live::checker::TemplateChecker::new` · crates/suprnova-live/src/checker/template.rs:77
  - [ ] fn `suprnova_live::checker::TemplateChecker::check_component` · crates/suprnova-live/src/checker/template.rs:91

## child

### `suprnova_live::child::codec` (private module; items are public through re-exports)

- [ ] fn `suprnova_live::child::verify_child_parameters` · crates/suprnova-live/src/child/codec.rs:177
- [ ] fn `suprnova_live::child::verify_child_parameters_v2` · crates/suprnova-live/src/child/codec.rs:195

### `suprnova_live::child::eligibility` (private module; items are public through re-exports)

- [ ] fn `suprnova_live::child::authorize_child_parameters_v2` · crates/suprnova-live/src/child/eligibility.rs:121
- [ ] struct `suprnova_live::child::ChildParameterEligibilityError` · crates/suprnova-live/src/child/eligibility.rs:42
  - [ ] fn `suprnova_live::child::ChildParameterEligibilityError::kind` · crates/suprnova-live/src/child/eligibility.rs:61
- [ ] struct `suprnova_live::child::EligibleChildParametersV2` · crates/suprnova-live/src/child/eligibility.rs:86
  - [ ] fn `suprnova_live::child::EligibleChildParametersV2::parameters` · crates/suprnova-live/src/child/eligibility.rs:97
  - [ ] fn `suprnova_live::child::EligibleChildParametersV2::child_instance` · crates/suprnova-live/src/child/eligibility.rs:103
  - [ ] fn `suprnova_live::child::EligibleChildParametersV2::parent_revision` · crates/suprnova-live/src/child/eligibility.rs:109
- [ ] enum `suprnova_live::child::ChildParameterEligibilityErrorKind` · crates/suprnova-live/src/child/eligibility.rs:14
  - Variants: `BindingMismatch`, `CompositionLineageMismatch`, `ParentAuthorityMissing`, `ParentRevisionMismatch`, `ProviderUnavailable`
  - [ ] fn `suprnova_live::child::ChildParameterEligibilityErrorKind::as_str` · crates/suprnova-live/src/child/eligibility.rs:30

### `suprnova_live::child::schema` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::child::ChildParameterError` · crates/suprnova-live/src/child/schema.rs:96
  - [ ] fn `suprnova_live::child::ChildParameterError::kind` · crates/suprnova-live/src/child/schema.rs:107
- [ ] struct `suprnova_live::child::ChildParameterLimits` · crates/suprnova-live/src/child/schema.rs:128
  - [ ] fn `suprnova_live::child::ChildParameterLimits::new` · crates/suprnova-live/src/child/schema.rs:136
  - [ ] fn `suprnova_live::child::ChildParameterLimits::input` · crates/suprnova-live/src/child/schema.rs:158
- [ ] struct `suprnova_live::child::ChildParametersV1` · crates/suprnova-live/src/child/schema.rs:173
  - [ ] fn `suprnova_live::child::ChildParametersV1::parent_revision` · crates/suprnova-live/src/child/schema.rs:193
  - [ ] fn `suprnova_live::child::ChildParametersV1::child_key` · crates/suprnova-live/src/child/schema.rs:199
  - [ ] fn `suprnova_live::child::ChildParametersV1::parameters` · crates/suprnova-live/src/child/schema.rs:205
- [ ] struct `suprnova_live::child::ChildParametersV2` · crates/suprnova-live/src/child/schema.rs:225
  - [ ] fn `suprnova_live::child::ChildParametersV2::parent_revision` · crates/suprnova-live/src/child/schema.rs:246
  - [ ] fn `suprnova_live::child::ChildParametersV2::child_key` · crates/suprnova-live/src/child/schema.rs:252
  - [ ] fn `suprnova_live::child::ChildParametersV2::child_instance` · crates/suprnova-live/src/child/schema.rs:258
  - [ ] fn `suprnova_live::child::ChildParametersV2::parameters` · crates/suprnova-live/src/child/schema.rs:264
- [ ] struct `suprnova_live::child::ExpectedChildParametersV1` · crates/suprnova-live/src/child/schema.rs:399
  - [ ] fn `suprnova_live::child::ExpectedChildParametersV1::new` · crates/suprnova-live/src/child/schema.rs:412
  - [ ] fn `suprnova_live::child::ExpectedChildParametersV1::after_applied_parent_revision` · crates/suprnova-live/src/child/schema.rs:433
- [ ] struct `suprnova_live::child::ExpectedChildParametersV2` · crates/suprnova-live/src/child/schema.rs:441
  - [ ] fn `suprnova_live::child::ExpectedChildParametersV2::new` · crates/suprnova-live/src/child/schema.rs:455
  - [ ] fn `suprnova_live::child::ExpectedChildParametersV2::after_applied_parent_revision` · crates/suprnova-live/src/child/schema.rs:478
- [ ] struct `suprnova_live::child::PreparedChildParametersV1` · crates/suprnova-live/src/child/schema.rs:285
  - [ ] fn `suprnova_live::child::PreparedChildParametersV1::publish` · crates/suprnova-live/src/child/codec.rs:67
  - [ ] fn `suprnova_live::child::PreparedChildParametersV1::new` · crates/suprnova-live/src/child/schema.rs:295
- [ ] struct `suprnova_live::child::PreparedChildParametersV2` · crates/suprnova-live/src/child/schema.rs:341
  - [ ] fn `suprnova_live::child::PreparedChildParametersV2::publish` · crates/suprnova-live/src/child/codec.rs:106
  - [ ] fn `suprnova_live::child::PreparedChildParametersV2::new` · crates/suprnova-live/src/child/schema.rs:351
- [ ] enum `suprnova_live::child::ChildParameterErrorKind` · crates/suprnova-live/src/child/schema.rs:25
  - Variants: `InvalidConfiguration`, `InputTooLarge`, `InputTooDeep`, `TooManyEntries`, `DuplicateField`, `InvalidEnvelope`, `WrongForm`, `UnsupportedSchema`, `SignatureInvalid`, `SigningKeyMismatch`, `BindingMismatch`, `ParentRevisionMismatch`, `ParentNotAccepted`, `ParameterSchemaMismatch`, `ParameterValueMismatch`, `InvalidParameters`, `IssuedInFuture`, `Expired`, `ValidityTooLong`
  - [ ] fn `suprnova_live::child::ChildParameterErrorKind::as_str` · crates/suprnova-live/src/child/schema.rs:69
- [ ] const `suprnova_live::child::CHILD_PARAMETERS_SCHEMA_V1` · crates/suprnova-live/src/child/schema.rs:18
- [ ] const `suprnova_live::child::CHILD_PARAMETERS_SCHEMA_V2` · crates/suprnova-live/src/child/schema.rs:21

### `suprnova_live::child::verified` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::child::AcceptedParentRevision` · crates/suprnova-live/src/child/verified.rs:13
  - [ ] fn `suprnova_live::child::AcceptedParentRevision::from_accepted_outcome` · crates/suprnova-live/src/child/verified.rs:22
- [ ] struct `suprnova_live::child::VerifiedChildParametersV1` · crates/suprnova-live/src/child/verified.rs:39
  - [ ] fn `suprnova_live::child::VerifiedChildParametersV1::parameters` · crates/suprnova-live/src/child/verified.rs:51
  - [ ] fn `suprnova_live::child::VerifiedChildParametersV1::parent_revision` · crates/suprnova-live/src/child/verified.rs:57
  - [ ] fn `suprnova_live::child::VerifiedChildParametersV1::child_key` · crates/suprnova-live/src/child/verified.rs:63
- [ ] struct `suprnova_live::child::VerifiedChildParametersV2` · crates/suprnova-live/src/child/verified.rs:81
  - [ ] fn `suprnova_live::child::VerifiedChildParametersV2::parameters` · crates/suprnova-live/src/child/verified.rs:93
  - [ ] fn `suprnova_live::child::VerifiedChildParametersV2::parent_revision` · crates/suprnova-live/src/child/verified.rs:99
  - [ ] fn `suprnova_live::child::VerifiedChildParametersV2::child_key` · crates/suprnova-live/src/child/verified.rs:105
  - [ ] fn `suprnova_live::child::VerifiedChildParametersV2::child_instance` · crates/suprnova-live/src/child/verified.rs:111

## clock

### `suprnova_live::clock`

- [ ] struct `suprnova_live::clock::ClockError` · crates/suprnova-live/src/clock.rs:31
  - [ ] fn `suprnova_live::clock::ClockError::timestamp_overflow` · crates/suprnova-live/src/clock.rs:42
  - [ ] fn `suprnova_live::clock::ClockError::kind` · crates/suprnova-live/src/clock.rs:48
- [ ] struct `suprnova_live::clock::SystemClock` · crates/suprnova-live/src/clock.rs:75
  - Implements: `suprnova_live::clock::Clock`
- [ ] enum `suprnova_live::clock::ClockErrorKind` · crates/suprnova-live/src/clock.rs:11
  - Variants: `BeforeUnixEpoch`, `TimestampOverflow`
  - [ ] fn `suprnova_live::clock::ClockErrorKind::as_str` · crates/suprnova-live/src/clock.rs:21
- [ ] trait `suprnova_live::clock::Clock` · crates/suprnova-live/src/clock.rs:68
  - Implemented here by: `clock::SystemClock`
  - [ ] fn `suprnova_live::clock::Clock::now` · crates/suprnova-live/src/clock.rs:70 (required)

## component

### `suprnova_live::component::composition`

- [ ] struct `suprnova_live::component::composition::ChildDeclaration` · crates/suprnova-live/src/component/composition.rs:240
  - [ ] fn `suprnova_live::component::composition::ChildDeclaration::new` · crates/suprnova-live/src/component/composition.rs:249
- [ ] struct `suprnova_live::component::composition::ChildFailureRecovery` · crates/suprnova-live/src/component/composition.rs:617
  - [ ] fn `suprnova_live::component::composition::ChildFailureRecovery::for_child` · crates/suprnova-live/src/component/composition.rs:624
  - [ ] fn `suprnova_live::component::composition::ChildFailureRecovery::child` · crates/suprnova-live/src/component/composition.rs:630
  - [ ] fn `suprnova_live::component::composition::ChildFailureRecovery::rolls_back_parent` · crates/suprnova-live/src/component/composition.rs:636
- [ ] struct `suprnova_live::component::composition::ChildHandle` · crates/suprnova-live/src/component/composition.rs:319
  - [ ] fn `suprnova_live::component::composition::ChildHandle::key` · crates/suprnova-live/src/component/composition.rs:334
  - [ ] fn `suprnova_live::component::composition::ChildHandle::component` · crates/suprnova-live/src/component/composition.rs:340
  - [ ] fn `suprnova_live::component::composition::ChildHandle::instance_id` · crates/suprnova-live/src/component/composition.rs:346
  - [ ] fn `suprnova_live::component::composition::ChildHandle::component_contract` · crates/suprnova-live/src/component/composition.rs:352
  - [ ] fn `suprnova_live::component::composition::ChildHandle::parameter_schema_version` · crates/suprnova-live/src/component/composition.rs:358
- [ ] struct `suprnova_live::component::composition::ChildKey` · crates/suprnova-live/src/component/composition.rs:30
  - [ ] fn `suprnova_live::component::composition::ChildKey::parse` · crates/suprnova-live/src/component/composition.rs:34
  - [ ] fn `suprnova_live::component::composition::ChildKey::as_str` · crates/suprnova-live/src/component/composition.rs:48
- [ ] struct `suprnova_live::component::composition::ChildParameterField` · crates/suprnova-live/src/component/composition.rs:55
  - [ ] fn `suprnova_live::component::composition::ChildParameterField::new` · crates/suprnova-live/src/component/composition.rs:64
- [ ] struct `suprnova_live::component::composition::ChildParameterSchema` · crates/suprnova-live/src/component/composition.rs:113
  - [ ] fn `suprnova_live::component::composition::ChildParameterSchema::new` · crates/suprnova-live/src/component/composition.rs:121
  - [ ] fn `suprnova_live::component::composition::ChildParameterSchema::empty` · crates/suprnova-live/src/component/composition.rs:142
  - [ ] fn `suprnova_live::component::composition::ChildParameterSchema::version` · crates/suprnova-live/src/component/composition.rs:148
  - [ ] fn `suprnova_live::component::composition::ChildParameterSchema::digest` · crates/suprnova-live/src/component/composition.rs:154
  - [ ] fn `suprnova_live::component::composition::ChildParameterSchema::len` · crates/suprnova-live/src/component/composition.rs:160
  - [ ] fn `suprnova_live::component::composition::ChildParameterSchema::is_empty` · crates/suprnova-live/src/component/composition.rs:166
- [ ] struct `suprnova_live::component::composition::CompositionAncestry` · crates/suprnova-live/src/component/composition.rs:430
  - [ ] fn `suprnova_live::component::composition::CompositionAncestry::root` · crates/suprnova-live/src/component/composition.rs:437
  - [ ] fn `suprnova_live::component::composition::CompositionAncestry::enter` · crates/suprnova-live/src/component/composition.rs:444
- [ ] struct `suprnova_live::component::composition::CompositionError` · crates/suprnova-live/src/component/composition.rs:670
  - [ ] fn `suprnova_live::component::composition::CompositionError::kind` · crates/suprnova-live/src/component/composition.rs:681
- [ ] struct `suprnova_live::component::composition::CompositionLimits` · crates/suprnova-live/src/component/composition.rs:466
  - [ ] fn `suprnova_live::component::composition::CompositionLimits::new` · crates/suprnova-live/src/component/composition.rs:476
- [ ] struct `suprnova_live::component::composition::CompositionPlanner` · crates/suprnova-live/src/component/composition.rs:508
  - [ ] fn `suprnova_live::component::composition::CompositionPlanner::new` · crates/suprnova-live/src/component/composition.rs:515
  - [ ] fn `suprnova_live::component::composition::CompositionPlanner::reconcile` · crates/suprnova-live/src/component/composition.rs:520
- [ ] struct `suprnova_live::component::composition::ParameterSchemaDigest` · crates/suprnova-live/src/component/composition.rs:89
  - [ ] fn `suprnova_live::component::composition::ParameterSchemaDigest::as_bytes` · crates/suprnova-live/src/component/composition.rs:94
- [ ] struct `suprnova_live::component::composition::ParameterValueDigest` · crates/suprnova-live/src/component/composition.rs:101
  - [ ] fn `suprnova_live::component::composition::ParameterValueDigest::as_bytes` · crates/suprnova-live/src/component/composition.rs:106
- [ ] struct `suprnova_live::component::composition::PendingChildParameters` · crates/suprnova-live/src/component/composition.rs:365
  - [ ] fn `suprnova_live::component::composition::PendingChildParameters::child` · crates/suprnova-live/src/component/composition.rs:376
  - [ ] fn `suprnova_live::component::composition::PendingChildParameters::parameters` · crates/suprnova-live/src/component/composition.rs:382
  - [ ] fn `suprnova_live::component::composition::PendingChildParameters::parameter_schema` · crates/suprnova-live/src/component/composition.rs:388
  - [ ] fn `suprnova_live::component::composition::PendingChildParameters::parameter_schema_version` · crates/suprnova-live/src/component/composition.rs:394
  - [ ] fn `suprnova_live::component::composition::PendingChildParameters::parameter_value` · crates/suprnova-live/src/component/composition.rs:400
- [ ] struct `suprnova_live::component::composition::PreparedChild` · crates/suprnova-live/src/component/composition.rs:271
  - [ ] fn `suprnova_live::component::composition::PreparedChild::into_handle` · crates/suprnova-live/src/component/composition.rs:286
  - [ ] fn `suprnova_live::component::composition::PreparedChild::parameters` · crates/suprnova-live/src/component/composition.rs:302
- [ ] enum `suprnova_live::component::composition::ChildState` · crates/suprnova-live/src/component/composition.rs:417
  - Variants: `Unchanged`, `PendingParams`, `Remount`, `Removed`
- [ ] enum `suprnova_live::component::composition::CompositionErrorKind` · crates/suprnova-live/src/component/composition.rs:643
  - Variants: `InvalidKey`, `InvalidSchema`, `InvalidLimits`, `DuplicateKey`, `UnknownComponent`, `InvalidParameters`, `ParametersTooLarge`, `TooManyChildren`, `TooManyPending`, `DepthExceeded`, `CircularComposition`

### `suprnova_live::component::executor` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::component::ActionExecutionError` · crates/suprnova-live/src/component/executor.rs:174
  - [ ] fn `suprnova_live::component::ActionExecutionError::kind` · crates/suprnova-live/src/component/executor.rs:219
  - [ ] fn `suprnova_live::component::ActionExecutionError::teardown_failed` · crates/suprnova-live/src/component/executor.rs:225
- [ ] struct `suprnova_live::component::ActionExecutionOutput` · crates/suprnova-live/src/component/executor.rs:42
  - [ ] fn `suprnova_live::component::ActionExecutionOutput::result` · crates/suprnova-live/src/component/executor.rs:88
  - [ ] fn `suprnova_live::component::ActionExecutionOutput::render` · crates/suprnova-live/src/component/executor.rs:94
  - [ ] fn `suprnova_live::component::ActionExecutionOutput::state` · crates/suprnova-live/src/component/executor.rs:100
  - [ ] fn `suprnova_live::component::ActionExecutionOutput::memo` · crates/suprnova-live/src/component/executor.rs:106
  - [ ] fn `suprnova_live::component::ActionExecutionOutput::validation` · crates/suprnova-live/src/component/executor.rs:112
  - [ ] fn `suprnova_live::component::ActionExecutionOutput::action_executed` · crates/suprnova-live/src/component/executor.rs:118
  - [ ] fn `suprnova_live::component::ActionExecutionOutput::transaction` · crates/suprnova-live/src/component/executor.rs:124
  - [ ] fn `suprnova_live::component::ActionExecutionOutput::take_transaction` · crates/suprnova-live/src/component/executor.rs:130
- [ ] struct `suprnova_live::component::ComponentExecutor` · crates/suprnova-live/src/component/executor.rs:281
  - [ ] fn `suprnova_live::component::ComponentExecutor::new` · crates/suprnova-live/src/component/executor.rs:298
  - [ ] fn `suprnova_live::component::ComponentExecutor::initial_mount` · crates/suprnova-live/src/component/executor.rs:303
  - [ ] fn `suprnova_live::component::ComponentExecutor::initial_public_mount` · crates/suprnova-live/src/component/executor.rs:324
  - [ ] fn `suprnova_live::component::ComponentExecutor::reconstruct` · crates/suprnova-live/src/component/executor.rs:346
  - [ ] fn `suprnova_live::component::ComponentExecutor::synchronize` · crates/suprnova-live/src/component/executor.rs:374
  - [ ] fn `suprnova_live::component::ComponentExecutor::action` · crates/suprnova-live/src/component/executor.rs:408
  - [ ] fn `suprnova_live::component::ComponentExecutor::coordinated_action` · crates/suprnova-live/src/component/executor.rs:436
  - [ ] fn `suprnova_live::component::ComponentExecutor::params_changed` · crates/suprnova-live/src/component/executor.rs:533
  - [ ] fn `suprnova_live::component::ComponentExecutor::params_changed_v2` · crates/suprnova-live/src/component/executor.rs:567
  - [ ] fn `suprnova_live::component::ComponentExecutor::lazy_complete` · crates/suprnova-live/src/component/executor.rs:601
- [ ] struct `suprnova_live::component::LifecycleOutput` · crates/suprnova-live/src/component/executor.rs:34
  - [ ] fn `suprnova_live::component::LifecycleOutput::render` · crates/suprnova-live/src/component/executor.rs:252
  - [ ] fn `suprnova_live::component::LifecycleOutput::state` · crates/suprnova-live/src/component/executor.rs:258
  - [ ] fn `suprnova_live::component::LifecycleOutput::memo` · crates/suprnova-live/src/component/executor.rs:264
- [ ] enum `suprnova_live::component::ActionExecutionErrorKind` · crates/suprnova-live/src/component/executor.rs:161
  - Variants: `Action`, `Validation`, `Host`, `Lifecycle`

### `suprnova_live::component::instance` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::component::ComponentError` · crates/suprnova-live/src/component/instance.rs:34
  - [ ] fn `suprnova_live::component::ComponentError::application_failure` · crates/suprnova-live/src/component/instance.rs:41
  - [ ] fn `suprnova_live::component::ComponentError::contract_failure` · crates/suprnova-live/src/component/instance.rs:49
  - [ ] fn `suprnova_live::component::ComponentError::kind` · crates/suprnova-live/src/component/instance.rs:57
- [ ] struct `suprnova_live::component::ComponentHooks` · crates/suprnova-live/src/component/instance.rs:393
  - [ ] fn `suprnova_live::component::ComponentHooks::new` · crates/suprnova-live/src/component/instance.rs:400
- [ ] struct `suprnova_live::component::HydrationContext` · crates/suprnova-live/src/component/instance.rs:204
  - [ ] fn `suprnova_live::component::HydrationContext::new` · crates/suprnova-live/src/component/instance.rs:213
  - [ ] fn `suprnova_live::component::HydrationContext::with_memo` · crates/suprnova-live/src/component/instance.rs:223
  - [ ] fn `suprnova_live::component::HydrationContext::render` · crates/suprnova-live/src/component/instance.rs:230
  - [ ] fn `suprnova_live::component::HydrationContext::state` · crates/suprnova-live/src/component/instance.rs:236
  - [ ] fn `suprnova_live::component::HydrationContext::memo` · crates/suprnova-live/src/component/instance.rs:242
- [ ] struct `suprnova_live::component::MountContext` · crates/suprnova-live/src/component/instance.rs:171
  - [ ] fn `suprnova_live::component::MountContext::new` · crates/suprnova-live/src/component/instance.rs:179
  - [ ] fn `suprnova_live::component::MountContext::render` · crates/suprnova-live/src/component/instance.rs:185
  - [ ] fn `suprnova_live::component::MountContext::parameters` · crates/suprnova-live/src/component/instance.rs:191
- [ ] struct `suprnova_live::component::RenderContext` · crates/suprnova-live/src/component/instance.rs:81
  - [ ] fn `suprnova_live::component::RenderContext::new` · crates/suprnova-live/src/component/instance.rs:92
  - [ ] fn `suprnova_live::component::RenderContext::for_public_seed` · crates/suprnova-live/src/component/instance.rs:110
  - [ ] fn `suprnova_live::component::RenderContext::with_browser_context` · crates/suprnova-live/src/component/instance.rs:126
  - [ ] fn `suprnova_live::component::RenderContext::request` · crates/suprnova-live/src/component/instance.rs:133
  - [ ] fn `suprnova_live::component::RenderContext::browser` · crates/suprnova-live/src/component/instance.rs:139
  - [ ] fn `suprnova_live::component::RenderContext::instance_id` · crates/suprnova-live/src/component/instance.rs:146
  - [ ] fn `suprnova_live::component::RenderContext::revision` · crates/suprnova-live/src/component/instance.rs:152
  - [ ] fn `suprnova_live::component::RenderContext::expires_at` · crates/suprnova-live/src/component/instance.rs:158
- [ ] enum `suprnova_live::component::ComponentErrorKind` · crates/suprnova-live/src/component/instance.rs:25
  - Variants: `ApplicationFailure`, `ContractFailure`
- [ ] trait `suprnova_live::component::ComponentFactory` · crates/suprnova-live/src/component/instance.rs:373
  - [ ] fn `suprnova_live::component::ComponentFactory::mount` · crates/suprnova-live/src/component/instance.rs:379 (required)
  - [ ] fn `suprnova_live::component::ComponentFactory::hydrate` · crates/suprnova-live/src/component/instance.rs:385 (required)
- [ ] trait `suprnova_live::component::ComponentInstance` · crates/suprnova-live/src/component/instance.rs:254
  - [ ] fn `suprnova_live::component::ComponentInstance::metadata` · crates/suprnova-live/src/component/instance.rs:256 (required)
  - [ ] fn `suprnova_live::component::ComponentInstance::action_target` · crates/suprnova-live/src/component/instance.rs:259 (required)
  - [ ] fn `suprnova_live::component::ComponentInstance::hydrated` · crates/suprnova-live/src/component/instance.rs:262 (provided)
  - [ ] fn `suprnova_live::component::ComponentInstance::bind_models` · crates/suprnova-live/src/component/instance.rs:272 (provided)
  - [ ] fn `suprnova_live::component::ComponentInstance::params_changed` · crates/suprnova-live/src/component/instance.rs:284 (provided)
  - [ ] fn `suprnova_live::component::ComponentInstance::params_changed_v2` · crates/suprnova-live/src/component/instance.rs:293 (provided)
  - [ ] fn `suprnova_live::component::ComponentInstance::lazy_complete` · crates/suprnova-live/src/component/instance.rs:302 (provided)
  - [ ] fn `suprnova_live::component::ComponentInstance::before_action` · crates/suprnova-live/src/component/instance.rs:310 (provided)
  - [ ] fn `suprnova_live::component::ComponentInstance::after_action` · crates/suprnova-live/src/component/instance.rs:319 (provided)
  - [ ] fn `suprnova_live::component::ComponentInstance::rendering` · crates/suprnova-live/src/component/instance.rs:329 (provided)
  - [ ] fn `suprnova_live::component::ComponentInstance::render` · crates/suprnova-live/src/component/instance.rs:337 (required)
  - [ ] fn `suprnova_live::component::ComponentInstance::rendered` · crates/suprnova-live/src/component/instance.rs:343 (provided)
  - [ ] fn `suprnova_live::component::ComponentInstance::dehydrating` · crates/suprnova-live/src/component/instance.rs:351 (provided)
  - [ ] fn `suprnova_live::component::ComponentInstance::dehydrate` · crates/suprnova-live/src/component/instance.rs:359 (required)
  - [ ] fn `suprnova_live::component::ComponentInstance::dehydrate_memo` · crates/suprnova-live/src/component/instance.rs:362 (provided)
  - [ ] fn `suprnova_live::component::ComponentInstance::teardown` · crates/suprnova-live/src/component/instance.rs:367 (provided)
- [ ] type `suprnova_live::component::LiveFuture` · crates/suprnova-live/src/component/instance.rs:21

### `suprnova_live::component::lazy`

- [ ] struct `suprnova_live::component::lazy::LazyCompletionRequest` · crates/suprnova-live/src/component/lazy.rs:85
  - [ ] fn `suprnova_live::component::lazy::LazyCompletionRequest::presentation` · crates/suprnova-live/src/component/lazy.rs:93
  - [ ] fn `suprnova_live::component::lazy::LazyCompletionRequest::operation` · crates/suprnova-live/src/component/lazy.rs:99
  - [ ] fn `suprnova_live::component::lazy::LazyCompletionRequest::initial_state` · crates/suprnova-live/src/component/lazy.rs:105
  - [ ] fn `suprnova_live::component::lazy::LazyCompletionRequest::loading_state` · crates/suprnova-live/src/component/lazy.rs:111
- [ ] struct `suprnova_live::component::lazy::LazyError` · crates/suprnova-live/src/component/lazy.rs:202
- [ ] struct `suprnova_live::component::lazy::LazyMount` · crates/suprnova-live/src/component/lazy.rs:166
  - [ ] fn `suprnova_live::component::lazy::LazyMount::new` · crates/suprnova-live/src/component/lazy.rs:174
  - [ ] fn `suprnova_live::component::lazy::LazyMount::schedule` · crates/suprnova-live/src/component/lazy.rs:183
- [ ] struct `suprnova_live::component::lazy::LazyPresentation` · crates/suprnova-live/src/component/lazy.rs:30
  - [ ] fn `suprnova_live::component::lazy::LazyPresentation::new` · crates/suprnova-live/src/component/lazy.rs:34
  - [ ] fn `suprnova_live::component::lazy::LazyPresentation::text` · crates/suprnova-live/src/component/lazy.rs:46
- [ ] enum `suprnova_live::component::lazy::LazyCompletion` · crates/suprnova-live/src/component/lazy.rs:128
  - Variants: `Eager`, `Deferred`
- [ ] enum `suprnova_live::component::lazy::LazyExecutionMode` · crates/suprnova-live/src/component/lazy.rs:19
  - Variants: `Browser`, `TestEager`, `NonBrowserEager`
- [ ] enum `suprnova_live::component::lazy::LazyOperation` · crates/suprnova-live/src/component/lazy.rs:68
  - Variants: `LazyComplete`
  - [ ] fn `suprnova_live::component::lazy::LazyOperation::as_str` · crates/suprnova-live/src/component/lazy.rs:76
- [ ] enum `suprnova_live::component::lazy::LazyPolicy` · crates/suprnova-live/src/component/lazy.rs:10
  - Variants: `Deferred`, `Eager`
- [ ] enum `suprnova_live::component::lazy::LazyPresentationState` · crates/suprnova-live/src/component/lazy.rs:53
  - Variants: `Placeholder`, `Loading`, `Empty`, `Error`, `Success`
- [ ] enum `suprnova_live::component::lazy::LazyServerCompletion` · crates/suprnova-live/src/component/lazy.rs:143
  - Variants: `Render`, `Empty`, `Failed`
  - [ ] fn `suprnova_live::component::lazy::LazyServerCompletion::presentation_state` · crates/suprnova-live/src/component/lazy.rs:155

### `suprnova_live::component::lifecycle` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::component::LifecycleError` · crates/suprnova-live/src/component/lifecycle.rs:52
  - [ ] fn `suprnova_live::component::LifecycleError::kind` · crates/suprnova-live/src/component/lifecycle.rs:74
  - [ ] fn `suprnova_live::component::LifecycleError::phase` · crates/suprnova-live/src/component/lifecycle.rs:80
  - [ ] fn `suprnova_live::component::LifecycleError::teardown_failed` · crates/suprnova-live/src/component/lifecycle.rs:86
- [ ] enum `suprnova_live::component::LifecycleErrorKind` · crates/suprnova-live/src/component/lifecycle.rs:39
  - Variants: `ComponentFailure`, `Panicked`, `ContractMismatch`, `HooksUnavailable`
- [ ] enum `suprnova_live::component::LifecyclePhase` · crates/suprnova-live/src/component/lifecycle.rs:8
  - Variants: `Mount`, `Hydrate`, `Bind`, `ParamsChanged`, `LazyComplete`, `BeforeAction`, `AfterAction`, `Rendering`, `Render`, `Rendered`, `Dehydrating`, `Dehydrate`, `Teardown`

## conformance

### `suprnova_live::conformance`

- [ ] fn `suprnova_live::conformance::expected_fixture_manifest_sha256` · crates/suprnova-live/src/conformance.rs:212
- [ ] fn `suprnova_live::conformance::expected_fixture_manifest_sha256_v2` · crates/suprnova-live/src/conformance.rs:217
- [ ] fn `suprnova_live::conformance::expected_fixture_manifest_sha256_v3` · crates/suprnova-live/src/conformance.rs:222
- [ ] fn `suprnova_live::conformance::expected_fixture_manifest_sha256_v4` · crates/suprnova-live/src/conformance.rs:227
- [ ] fn `suprnova_live::conformance::expected_fixture_manifest_sha256_version` · crates/suprnova-live/src/conformance.rs:232
- [ ] fn `suprnova_live::conformance::fixture_directory` · crates/suprnova-live/src/conformance.rs:159
- [ ] fn `suprnova_live::conformance::fixture_directory_v1` · crates/suprnova-live/src/conformance.rs:135
- [ ] fn `suprnova_live::conformance::fixture_directory_v2` · crates/suprnova-live/src/conformance.rs:141
- [ ] fn `suprnova_live::conformance::fixture_directory_v3` · crates/suprnova-live/src/conformance.rs:147
- [ ] fn `suprnova_live::conformance::fixture_directory_v4` · crates/suprnova-live/src/conformance.rs:153
- [ ] fn `suprnova_live::conformance::fixture_manifest_sha256` · crates/suprnova-live/src/conformance.rs:164
- [ ] fn `suprnova_live::conformance::fixture_manifest_sha256_v2` · crates/suprnova-live/src/conformance.rs:169
- [ ] fn `suprnova_live::conformance::fixture_manifest_sha256_v3` · crates/suprnova-live/src/conformance.rs:174
- [ ] fn `suprnova_live::conformance::fixture_manifest_sha256_v4` · crates/suprnova-live/src/conformance.rs:179
- [ ] fn `suprnova_live::conformance::fixture_manifest_sha256_version` · crates/suprnova-live/src/conformance.rs:184
- [ ] struct `suprnova_live::conformance::ConformanceError` · crates/suprnova-live/src/conformance.rs:107
  - [ ] fn `suprnova_live::conformance::ConformanceError::kind` · crates/suprnova-live/src/conformance.rs:114
- [ ] enum `suprnova_live::conformance::ConformanceErrorKind` · crates/suprnova-live/src/conformance.rs:100
  - Variants: `FixtureUnavailable`
- [ ] enum `suprnova_live::conformance::FixtureVersion` · crates/suprnova-live/src/conformance.rs:55
  - Variants: `V1`, `V2`, `V3`, `V4`
  - [ ] fn `suprnova_live::conformance::FixtureVersion::get` · crates/suprnova-live/src/conformance.rs:69
  - [ ] fn `suprnova_live::conformance::FixtureVersion::files` · crates/suprnova-live/src/conformance.rs:80
- [ ] const `suprnova_live::conformance::FIXTURE_FILES_V1` · crates/suprnova-live/src/conformance.rs:11
- [ ] const `suprnova_live::conformance::FIXTURE_FILES_V2` · crates/suprnova-live/src/conformance.rs:23
- [ ] const `suprnova_live::conformance::FIXTURE_FILES_V3` · crates/suprnova-live/src/conformance.rs:30
- [ ] const `suprnova_live::conformance::FIXTURE_FILES_V4` · crates/suprnova-live/src/conformance.rs:43
- [ ] const `suprnova_live::conformance::FIXTURE_VERSIONS` · crates/suprnova-live/src/conformance.rs:91

## crypto

### `suprnova_live::crypto::key` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::crypto::KeyError` · crates/suprnova-live/src/crypto/key.rs:111
  - [ ] fn `suprnova_live::crypto::KeyError::kind` · crates/suprnova-live/src/crypto/key.rs:122
- [ ] struct `suprnova_live::crypto::KeyRecord` · crates/suprnova-live/src/crypto/key.rs:174
  - [ ] fn `suprnova_live::crypto::KeyRecord::new` · crates/suprnova-live/src/crypto/key.rs:185
  - [ ] fn `suprnova_live::crypto::KeyRecord::key_id` · crates/suprnova-live/src/crypto/key.rs:206
- [ ] struct `suprnova_live::crypto::RootKey` · crates/suprnova-live/src/crypto/key.rs:143
  - [ ] fn `suprnova_live::crypto::RootKey::new` · crates/suprnova-live/src/crypto/key.rs:147
- [ ] enum `suprnova_live::crypto::KeyErrorKind` · crates/suprnova-live/src/crypto/key.rs:67
  - Variants: `WeakRootKey`, `InvalidKeyWindow`, `TooManyKeys`, `DuplicateKeyId`, `UnknownKey`, `KeyNotActive`, `KeyRetired`, `InvalidSignatureEncoding`, `SignatureMismatch`, `DerivationFailure`
  - [ ] fn `suprnova_live::crypto::KeyErrorKind::as_str` · crates/suprnova-live/src/crypto/key.rs:93
- [ ] enum `suprnova_live::crypto::SnapshotPurpose` · crates/suprnova-live/src/crypto/key.rs:28
  - Variants: `SeedV1`, `InstanceV1`, `ChildParametersV1`, `ChildParametersV2`, `UploadGrantV1`, `AsyncSubscriptionV1`, `RenderVarianceV1`, `RenderKeyV1`, `RenderEntryV1`

### `suprnova_live::crypto::key_ring` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::crypto::SnapshotKeyRing` · crates/suprnova-live/src/crypto/key_ring.rs:14
  - [ ] fn `suprnova_live::crypto::SnapshotKeyRing::new` · crates/suprnova-live/src/crypto/key_ring.rs:21
  - [ ] fn `suprnova_live::crypto::SnapshotKeyRing::active_key_id` · crates/suprnova-live/src/crypto/key_ring.rs:47
  - [ ] fn `suprnova_live::crypto::SnapshotKeyRing::sign` · crates/suprnova-live/src/crypto/key_ring.rs:52
  - [ ] fn `suprnova_live::crypto::SnapshotKeyRing::verify` · crates/suprnova-live/src/crypto/key_ring.rs:74

### `suprnova_live::crypto::signature` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::crypto::SignedMac` · crates/suprnova-live/src/crypto/signature.rs:71
  - [ ] fn `suprnova_live::crypto::SignedMac::key_id` · crates/suprnova-live/src/crypto/signature.rs:83
  - [ ] fn `suprnova_live::crypto::SignedMac::signature` · crates/suprnova-live/src/crypto/signature.rs:89
- [ ] struct `suprnova_live::crypto::SnapshotSignature` · crates/suprnova-live/src/crypto/signature.rs:17
  - [ ] fn `suprnova_live::crypto::SnapshotSignature::parse` · crates/suprnova-live/src/crypto/signature.rs:21
  - [ ] fn `suprnova_live::crypto::SnapshotSignature::as_bytes` · crates/suprnova-live/src/crypto/signature.rs:43
  - [ ] fn `suprnova_live::crypto::SnapshotSignature::to_base64url` · crates/suprnova-live/src/crypto/signature.rs:49

## endpoint

### `suprnova_live::endpoint::config` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::endpoint::LiveEndpointConfig` · crates/suprnova-live/src/endpoint/config.rs:12
  - [ ] fn `suprnova_live::endpoint::LiveEndpointConfig::new` · crates/suprnova-live/src/endpoint/config.rs:22
  - [ ] fn `suprnova_live::endpoint::LiveEndpointConfig::max_request_bytes` · crates/suprnova-live/src/endpoint/config.rs:39
  - [ ] fn `suprnova_live::endpoint::LiveEndpointConfig::max_response_bytes` · crates/suprnova-live/src/endpoint/config.rs:45
  - [ ] fn `suprnova_live::endpoint::LiveEndpointConfig::with_max_response_bytes` · crates/suprnova-live/src/endpoint/config.rs:50
  - [ ] fn `suprnova_live::endpoint::LiveEndpointConfig::inspect_mount` · crates/suprnova-live/src/endpoint/config.rs:62

### `suprnova_live::endpoint::error` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::endpoint::EndpointError` · crates/suprnova-live/src/endpoint/error.rs:47
  - [ ] fn `suprnova_live::endpoint::EndpointError::kind` · crates/suprnova-live/src/endpoint/error.rs:58
- [ ] struct `suprnova_live::endpoint::EndpointKernelError` · crates/suprnova-live/src/endpoint/error.rs:97
  - [ ] fn `suprnova_live::endpoint::EndpointKernelError::unavailable` · crates/suprnova-live/src/endpoint/error.rs:104
  - [ ] fn `suprnova_live::endpoint::EndpointKernelError::context_inconsistent` · crates/suprnova-live/src/endpoint/error.rs:112
- [ ] enum `suprnova_live::endpoint::EndpointErrorKind` · crates/suprnova-live/src/endpoint/error.rs:8
  - Variants: `MissingContext`, `CacheAttempt`, `MethodNotAllowed`, `UnsupportedMediaType`, `UnsupportedCharset`, `UnsupportedVersion`, `RequestTooLarge`, `MalformedProtocol`, `ContextExpired`, `ContextInconsistent`, `RegistryMismatch`, `SnapshotRejected`, `InvalidKernelResponse`, `ResponseTooLarge`, `KernelUnavailable`, `ClockUnavailable`, `InvalidConfiguration`

### `suprnova_live::endpoint::request` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::endpoint::LiveEndpointRequest` · crates/suprnova-live/src/endpoint/request.rs:96
  - Public fields: `method`, `content_type`, `body`, `context`
  - [ ] fn `suprnova_live::endpoint::LiveEndpointRequest::try_new` · crates/suprnova-live/src/endpoint/request.rs:112
- [ ] struct `suprnova_live::endpoint::ParsedLiveMediaType` · crates/suprnova-live/src/endpoint/request.rs:26
  - [ ] fn `suprnova_live::endpoint::ParsedLiveMediaType::parse` · crates/suprnova-live/src/endpoint/request.rs:32
  - [ ] fn `suprnova_live::endpoint::ParsedLiveMediaType::protocol_version` · crates/suprnova-live/src/endpoint/request.rs:67
- [ ] enum `suprnova_live::endpoint::RequestCachePolicy` · crates/suprnova-live/src/endpoint/request.rs:88
  - Variants: `Bypass`, `Attempted`
- [ ] const `suprnova_live::endpoint::LIVE_MEDIA_TYPE_V1` · crates/suprnova-live/src/endpoint/request.rs:20
- [ ] const `suprnova_live::endpoint::LIVE_MEDIA_TYPE_V2` · crates/suprnova-live/src/endpoint/request.rs:22

### `suprnova_live::endpoint::response` (private module; items are public through re-exports)

- [ ] fn `suprnova_live::endpoint::dispatch_execution_result` · crates/suprnova-live/src/endpoint/response.rs:446
- [ ] struct `suprnova_live::endpoint::AcceptedResponseRequestBinding` · crates/suprnova-live/src/endpoint/response.rs:114
- [ ] struct `suprnova_live::endpoint::AcceptedResponseSealer` · crates/suprnova-live/src/endpoint/response.rs:232
  - [ ] fn `suprnova_live::endpoint::AcceptedResponseSealer::protocol_version` · crates/suprnova-live/src/endpoint/response.rs:259
- [ ] struct `suprnova_live::endpoint::EndpointDispatch` · crates/suprnova-live/src/endpoint/response.rs:620
  - [ ] fn `suprnova_live::endpoint::EndpointDispatch::new` · crates/suprnova-live/src/endpoint/response.rs:629
- [ ] struct `suprnova_live::endpoint::EndpointNavigationTarget` · crates/suprnova-live/src/endpoint/response.rs:28
  - [ ] fn `suprnova_live::endpoint::EndpointNavigationTarget::parse` · crates/suprnova-live/src/endpoint/response.rs:32
  - [ ] fn `suprnova_live::endpoint::EndpointNavigationTarget::as_str` · crates/suprnova-live/src/endpoint/response.rs:48
- [ ] struct `suprnova_live::endpoint::EndpointNavigationTargetError` · crates/suprnova-live/src/endpoint/response.rs:61
- [ ] struct `suprnova_live::endpoint::EndpointResponseIntents` · crates/suprnova-live/src/endpoint/response.rs:76
  - [ ] fn `suprnova_live::endpoint::EndpointResponseIntents::with_redirect` · crates/suprnova-live/src/endpoint/response.rs:84
  - [ ] fn `suprnova_live::endpoint::EndpointResponseIntents::with_reflected_url` · crates/suprnova-live/src/endpoint/response.rs:91
- [ ] struct `suprnova_live::endpoint::LiveEndpointResponse` · crates/suprnova-live/src/endpoint/response.rs:674
  - Public fields: `status`, `headers`, `body`
  - [ ] fn `suprnova_live::endpoint::LiveEndpointResponse::from_error_kind` · crates/suprnova-live/src/endpoint/response.rs:686
- [ ] enum `suprnova_live::endpoint::EndpointOutcomeKind` · crates/suprnova-live/src/endpoint/response.rs:590
  - Variants: `Accepted`, `Duplicate`, `Rejected`, `Concealed`, `Conflict`, `RefreshRequired`, `Fatal`

### `suprnova_live::endpoint::service` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::endpoint::LiveEndpointService` · crates/suprnova-live/src/endpoint/service.rs:187
  - [ ] fn `suprnova_live::endpoint::LiveEndpointService::new` · crates/suprnova-live/src/endpoint/service.rs:199
  - [ ] fn `suprnova_live::endpoint::LiveEndpointService::handle` · crates/suprnova-live/src/endpoint/service.rs:218
  - [ ] fn `suprnova_live::endpoint::LiveEndpointService::error_response` · crates/suprnova-live/src/endpoint/service.rs:227
- [ ] struct `suprnova_live::endpoint::VerifiedChildAdmissionV2` · crates/suprnova-live/src/endpoint/service.rs:111
  - [ ] fn `suprnova_live::endpoint::VerifiedChildAdmissionV2::parameters` · crates/suprnova-live/src/endpoint/service.rs:119
  - [ ] fn `suprnova_live::endpoint::VerifiedChildAdmissionV2::parent_snapshot` · crates/suprnova-live/src/endpoint/service.rs:125
- [ ] struct `suprnova_live::endpoint::VerifiedEndpointExecutionRequest` · crates/suprnova-live/src/endpoint/service.rs:101
  - [ ] fn `suprnova_live::endpoint::VerifiedEndpointExecutionRequest::descriptor` · crates/suprnova-live/src/endpoint/service.rs:133
  - [ ] fn `suprnova_live::endpoint::VerifiedEndpointExecutionRequest::component` · crates/suprnova-live/src/endpoint/service.rs:139
  - [ ] fn `suprnova_live::endpoint::VerifiedEndpointExecutionRequest::request` · crates/suprnova-live/src/endpoint/service.rs:145
  - [ ] fn `suprnova_live::endpoint::VerifiedEndpointExecutionRequest::snapshot` · crates/suprnova-live/src/endpoint/service.rs:151
  - [ ] fn `suprnova_live::endpoint::VerifiedEndpointExecutionRequest::context` · crates/suprnova-live/src/endpoint/service.rs:157
  - [ ] fn `suprnova_live::endpoint::VerifiedEndpointExecutionRequest::child_admission` · crates/suprnova-live/src/endpoint/service.rs:163
  - [ ] fn `suprnova_live::endpoint::VerifiedEndpointExecutionRequest::response_binding` · crates/suprnova-live/src/endpoint/service.rs:169
- [ ] struct `suprnova_live::endpoint::VerifiedEndpointRequest` · crates/suprnova-live/src/endpoint/service.rs:66
  - [ ] fn `suprnova_live::endpoint::VerifiedEndpointRequest::into_execution_parts` · crates/suprnova-live/src/endpoint/service.rs:78
- [ ] enum `suprnova_live::endpoint::VerifiedEndpointSnapshot` · crates/suprnova-live/src/endpoint/service.rs:49
  - Variants: `Instance`, `Seed`
- [ ] trait `suprnova_live::endpoint::EndpointKernel` · crates/suprnova-live/src/endpoint/service.rs:40
  - [ ] fn `suprnova_live::endpoint::EndpointKernel::dispatch` · crates/suprnova-live/src/endpoint/service.rs:42 (required)
- [ ] type `suprnova_live::endpoint::EndpointFuture` · crates/suprnova-live/src/endpoint/service.rs:36

## error

### `suprnova_live::error`

- [ ] struct `suprnova_live::error::LiveError` · crates/suprnova-live/src/error.rs:165
  - [ ] fn `suprnova_live::error::LiveError::new` · crates/suprnova-live/src/error.rs:175
  - [ ] fn `suprnova_live::error::LiveError::with_source` · crates/suprnova-live/src/error.rs:194
  - [ ] fn `suprnova_live::error::LiveError::category` · crates/suprnova-live/src/error.rs:201
  - [ ] fn `suprnova_live::error::LiveError::recovery` · crates/suprnova-live/src/error.rs:207
  - [ ] fn `suprnova_live::error::LiveError::detail` · crates/suprnova-live/src/error.rs:213
- [ ] enum `suprnova_live::error::ErrorCategory` · crates/suprnova-live/src/error.rs:12
  - Variants: `Protocol`, `Validation`, `Authentication`, `Authorization`, `Csrf`, `Snapshot`, `Revision`, `Render`, `Morph`, `Provider`, `Cache`, `Upload`, `Compatibility`, `SizeLimit`, `RateLimit`, `Security`, `Internal`
  - [ ] fn `suprnova_live::error::ErrorCategory::as_str` · crates/suprnova-live/src/error.rs:52
- [ ] enum `suprnova_live::error::RecoveryInstruction` · crates/suprnova-live/src/error.rs:79
  - Variants: `RetainDom`, `Retry`, `RefreshIsland`, `RemountIsland`, `Navigate`, `Stop`
  - [ ] fn `suprnova_live::error::RecoveryInstruction::as_str` · crates/suprnova-live/src/error.rs:97
- [ ] enum `suprnova_live::error::SafeDiagnosticCode` · crates/suprnova-live/src/error.rs:113
  - Variants: `InputTooLarge`, `InputTooDeep`, `TooManyEntries`, `StringTooLong`, `DuplicateKey`, `InvalidUtf8`, `InvalidNumber`, `InvalidJson`, `SerializationFailed`, `InvalidLimitConfiguration`, `InvalidIdentifier`, `InvalidBase64Identity`, `SignatureInvalid`
  - [ ] fn `suprnova_live::error::SafeDiagnosticCode::as_str` · crates/suprnova-live/src/error.rs:145

## execution

### `suprnova_live::execution::recovery` (private module; items are public through re-exports)

- [ ] enum `suprnova_live::execution::RetryLegality` · crates/suprnova-live/src/execution/recovery.rs:5
  - Variants: `Allowed`, `Prohibited`

### `suprnova_live::execution::service` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::execution::AcceptedExecution` · crates/suprnova-live/src/execution/service.rs:462
  - [ ] fn `suprnova_live::execution::AcceptedExecution::revision` · crates/suprnova-live/src/execution/service.rs:478
  - [ ] fn `suprnova_live::execution::AcceptedExecution::signed_snapshot` · crates/suprnova-live/src/execution/service.rs:484
  - [ ] fn `suprnova_live::execution::AcceptedExecution::render` · crates/suprnova-live/src/execution/service.rs:490
  - [ ] fn `suprnova_live::execution::AcceptedExecution::result` · crates/suprnova-live/src/execution/service.rs:496
  - [ ] fn `suprnova_live::execution::AcceptedExecution::response_intents` · crates/suprnova-live/src/execution/service.rs:502
  - [ ] fn `suprnova_live::execution::AcceptedExecution::validation` · crates/suprnova-live/src/execution/service.rs:508
  - [ ] fn `suprnova_live::execution::AcceptedExecution::action_executed` · crates/suprnova-live/src/execution/service.rs:514
  - [ ] fn `suprnova_live::execution::AcceptedExecution::reporting_failed` · crates/suprnova-live/src/execution/service.rs:520
  - [ ] fn `suprnova_live::execution::AcceptedExecution::child_deliveries` · crates/suprnova-live/src/execution/service.rs:526
- [ ] struct `suprnova_live::execution::AcceptedExecutionReport` · crates/suprnova-live/src/execution/service.rs:565
  - [ ] fn `suprnova_live::execution::AcceptedExecutionReport::revision` · crates/suprnova-live/src/execution/service.rs:573
  - [ ] fn `suprnova_live::execution::AcceptedExecutionReport::outcome` · crates/suprnova-live/src/execution/service.rs:579
- [ ] struct `suprnova_live::execution::ActionExecutionRequest` · crates/suprnova-live/src/execution/service.rs:59
  - [ ] fn `suprnova_live::execution::ActionExecutionRequest::new` · crates/suprnova-live/src/execution/service.rs:81
  - [ ] fn `suprnova_live::execution::ActionExecutionRequest::with_proposals` · crates/suprnova-live/src/execution/service.rs:109
  - [ ] fn `suprnova_live::execution::ActionExecutionRequest::with_response_intent_preparation` · crates/suprnova-live/src/execution/service.rs:116
  - [ ] fn `suprnova_live::execution::ActionExecutionRequest::with_response_sealer` · crates/suprnova-live/src/execution/service.rs:126
- [ ] struct `suprnova_live::execution::ExecutionService` · crates/suprnova-live/src/execution/service.rs:594
  - [ ] fn `suprnova_live::execution::ExecutionService::new` · crates/suprnova-live/src/execution/service.rs:688
  - [ ] fn `suprnova_live::execution::ExecutionService::with_island_stream_directive` · crates/suprnova-live/src/execution/service.rs:711
  - [ ] fn `suprnova_live::execution::ExecutionService::with_reporter` · crates/suprnova-live/src/execution/service.rs:718
  - [ ] fn `suprnova_live::execution::ExecutionService::authorize_child_parameters_v2` · crates/suprnova-live/src/execution/service.rs:724
  - [ ] fn `suprnova_live::execution::ExecutionService::execute_instanced` · crates/suprnova-live/src/execution/service.rs:733
  - [ ] fn `suprnova_live::execution::ExecutionService::execute_fresh_render` · crates/suprnova-live/src/execution/service.rs:808
  - [ ] fn `suprnova_live::execution::ExecutionService::execute_lifecycle` · crates/suprnova-live/src/execution/service.rs:884
  - [ ] fn `suprnova_live::execution::ExecutionService::execute_promoted` · crates/suprnova-live/src/execution/service.rs:981
- [ ] struct `suprnova_live::execution::InstancedActionRequest` · crates/suprnova-live/src/execution/service.rs:189
  - [ ] fn `suprnova_live::execution::InstancedActionRequest::new` · crates/suprnova-live/src/execution/service.rs:299
- [ ] struct `suprnova_live::execution::InstancedFreshRenderRequest` · crates/suprnova-live/src/execution/service.rs:200
  - [ ] fn `suprnova_live::execution::InstancedFreshRenderRequest::new` · crates/suprnova-live/src/execution/service.rs:323
  - [ ] fn `suprnova_live::execution::InstancedFreshRenderRequest::with_response_sealer` · crates/suprnova-live/src/execution/service.rs:347
- [ ] struct `suprnova_live::execution::InstancedLifecycleRequest` · crates/suprnova-live/src/execution/service.rs:226
  - [ ] fn `suprnova_live::execution::InstancedLifecycleRequest::new` · crates/suprnova-live/src/execution/service.rs:365
  - [ ] fn `suprnova_live::execution::InstancedLifecycleRequest::with_response_sealer` · crates/suprnova-live/src/execution/service.rs:391
- [ ] struct `suprnova_live::execution::PromotedActionRequest` · crates/suprnova-live/src/execution/service.rs:240
  - [ ] fn `suprnova_live::execution::PromotedActionRequest::new` · crates/suprnova-live/src/execution/service.rs:252
- [ ] struct `suprnova_live::execution::PromotedRequestIdentity` · crates/suprnova-live/src/execution/service.rs:274
  - [ ] fn `suprnova_live::execution::PromotedRequestIdentity::new` · crates/suprnova-live/src/execution/service.rs:283
- [ ] struct `suprnova_live::execution::RefreshRequiredExecution` · crates/suprnova-live/src/execution/service.rs:424
  - [ ] fn `suprnova_live::execution::RefreshRequiredExecution::reason` · crates/suprnova-live/src/execution/service.rs:433
  - [ ] fn `suprnova_live::execution::RefreshRequiredExecution::retry_legality` · crates/suprnova-live/src/execution/service.rs:439
  - [ ] fn `suprnova_live::execution::RefreshRequiredExecution::accepted_metadata` · crates/suprnova-live/src/execution/service.rs:445
- [ ] struct `suprnova_live::execution::ResponseIntentPreparationRequest` · crates/suprnova-live/src/execution/service.rs:160
  - [ ] fn `suprnova_live::execution::ResponseIntentPreparationRequest::result` · crates/suprnova-live/src/execution/service.rs:168
  - [ ] fn `suprnova_live::execution::ResponseIntentPreparationRequest::authority` · crates/suprnova-live/src/execution/service.rs:174
- [ ] struct `suprnova_live::execution::VerifiedResponseIntentAuthority` · crates/suprnova-live/src/execution/service.rs:139
  - [ ] fn `suprnova_live::execution::VerifiedResponseIntentAuthority::mounted_document_path` · crates/suprnova-live/src/execution/service.rs:147
  - [ ] fn `suprnova_live::execution::VerifiedResponseIntentAuthority::protocol_version` · crates/suprnova-live/src/execution/service.rs:153
- [ ] enum `suprnova_live::execution::ExecutionRefreshReason` · crates/suprnova-live/src/execution/service.rs:404
  - Variants: `Ledger`, `Stale`, `DuplicateResponseUnavailable`, `ExecutionFailed`, `HostCommitFailed`, `LedgerAcceptanceFailed`, `LedgerUnavailable`, `ProtocolUpgradeRequired`
- [ ] enum `suprnova_live::execution::ExecutionResult` · crates/suprnova-live/src/execution/service.rs:549
  - Variants: `Accepted`, `InProgress`, `RefreshRequired`, `IdempotencyConflict`
- [ ] enum `suprnova_live::execution::InstancedLifecycleOperation` · crates/suprnova-live/src/execution/service.rs:214
  - Variants: `SyncModels`, `ParamsChanged`, `ParamsChangedV1Historical`, `LazyComplete`
- [ ] trait `suprnova_live::execution::AcceptedOutcomeReporter` · crates/suprnova-live/src/execution/service.rs:585
  - [ ] fn `suprnova_live::execution::AcceptedOutcomeReporter::report` · crates/suprnova-live/src/execution/service.rs:587 (required)
- [ ] trait `suprnova_live::execution::ResponseIntentPreparationPort` · crates/suprnova-live/src/execution/service.rs:180
  - [ ] fn `suprnova_live::execution::ResponseIntentPreparationPort::prepare` · crates/suprnova-live/src/execution/service.rs:182 (required)

### `suprnova_live::execution::trace` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::execution::NoopExecutionTrace` · crates/suprnova-live/src/execution/trace.rs:56
  - Implements: `suprnova_live::execution::ExecutionTracePort`
- [ ] enum `suprnova_live::execution::ExecutionPhase` · crates/suprnova-live/src/execution/trace.rs:7
  - Variants: `Claim`, `PromotionMount`, `Hydrate`, `Bind`, `Authorize`, `Validate`, `TransactionBegin`, `BeforeAction`, `Action`, `AfterAction`, `Render`, `Dehydrate`, `Sign`, `OutcomeValidation`, `ResponseIntentPreparation`, `ResponseSealing`, `HostCommit`, `LedgerAcceptance`, `Reporting`
- [ ] trait `suprnova_live::execution::ExecutionTracePort` · crates/suprnova-live/src/execution/trace.rs:49
  - Implemented here by: `execution::NoopExecutionTrace`
  - [ ] fn `suprnova_live::execution::ExecutionTracePort::record` · crates/suprnova-live/src/execution/trace.rs:51 (required)

### `suprnova_live::execution::transaction` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::execution::HostError` · crates/suprnova-live/src/execution/transaction.rs:28
  - [ ] fn `suprnova_live::execution::HostError::new` · crates/suprnova-live/src/execution/transaction.rs:35
  - [ ] fn `suprnova_live::execution::HostError::kind` · crates/suprnova-live/src/execution/transaction.rs:41
- [ ] enum `suprnova_live::execution::HostErrorKind` · crates/suprnova-live/src/execution/transaction.rs:13
  - Variants: `Begin`, `Commit`, `Rollback`, `Reporting`, `ResponseIntent`
- [ ] trait `suprnova_live::execution::HostTransaction` · crates/suprnova-live/src/execution/transaction.rs:67
  - [ ] fn `suprnova_live::execution::HostTransaction::commit` · crates/suprnova-live/src/execution/transaction.rs:74 (required)
  - [ ] fn `suprnova_live::execution::HostTransaction::rollback` · crates/suprnova-live/src/execution/transaction.rs:77 (required)
- [ ] trait `suprnova_live::execution::TransactionPort` · crates/suprnova-live/src/execution/transaction.rs:81
  - [ ] fn `suprnova_live::execution::TransactionPort::begin` · crates/suprnova-live/src/execution/transaction.rs:83 (required)

## host

### `suprnova_live::host`

- [ ] struct `suprnova_live::host::HostContextError` · crates/suprnova-live/src/host/mod.rs:101
  - [ ] fn `suprnova_live::host::HostContextError::kind` · crates/suprnova-live/src/host/mod.rs:112
- [ ] enum `suprnova_live::host::HostContextErrorKind` · crates/suprnova-live/src/host/mod.rs:27
  - Variants: `InvalidConfiguration`, `DuplicateCheck`, `MissingCheck`, `InvalidCheckDisposition`, `CheckExpired`, `ContextExpired`, `ContextLifetimeExceeded`, `RouteMismatch`, `SlotMismatch`, `ComponentMismatch`, `ContractMismatch`, `ProtocolMismatch`, `SessionRequirement`, `PrincipalRequirement`, `TenantRequirement`, `ScopeMismatch`, `SessionMismatch`, `PrincipalMismatch`, `TenantMismatch`, `CatalogConflict`
  - [ ] fn `suprnova_live::host::HostContextErrorKind::as_str` · crates/suprnova-live/src/host/mod.rs:73

### `suprnova_live::host::capabilities` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::host::HostCapabilities` · crates/suprnova-live/src/host/capabilities.rs:115
  - [ ] fn `suprnova_live::host::HostCapabilities::bound_to` · crates/suprnova-live/src/host/capabilities.rs:128
  - [ ] fn `suprnova_live::host::HostCapabilities::with_action_authorization` · crates/suprnova-live/src/host/capabilities.rs:142
  - [ ] fn `suprnova_live::host::HostCapabilities::with_upload_authorization` · crates/suprnova-live/src/host/capabilities.rs:152
  - [ ] fn `suprnova_live::host::HostCapabilities::with_subscription_authorization` · crates/suprnova-live/src/host/capabilities.rs:162
  - [ ] fn `suprnova_live::host::HostCapabilities::with_subscription_credentials` · crates/suprnova-live/src/host/capabilities.rs:172
  - [ ] fn `suprnova_live::host::HostCapabilities::with_subscription_registry` · crates/suprnova-live/src/host/capabilities.rs:182
  - [ ] fn `suprnova_live::host::HostCapabilities::with_subscription_continuity` · crates/suprnova-live/src/host/capabilities.rs:192
- [ ] struct `suprnova_live::host::HostScopeFacts` · crates/suprnova-live/src/host/capabilities.rs:54
  - [ ] fn `suprnova_live::host::HostScopeFacts::new` · crates/suprnova-live/src/host/capabilities.rs:64
  - [ ] fn `suprnova_live::host::HostScopeFacts::scope` · crates/suprnova-live/src/host/capabilities.rs:80
  - [ ] fn `suprnova_live::host::HostScopeFacts::session` · crates/suprnova-live/src/host/capabilities.rs:86
  - [ ] fn `suprnova_live::host::HostScopeFacts::principal` · crates/suprnova-live/src/host/capabilities.rs:92
  - [ ] fn `suprnova_live::host::HostScopeFacts::tenant` · crates/suprnova-live/src/host/capabilities.rs:98
- [ ] struct `suprnova_live::host::PrincipalFingerprint` · crates/suprnova-live/src/host/capabilities.rs:39
  - [ ] fn `suprnova_live::host::PrincipalFingerprint::from_bytes` · crates/suprnova-live/src/host/capabilities.rs:39
- [ ] struct `suprnova_live::host::SessionFingerprint` · crates/suprnova-live/src/host/capabilities.rs:43
  - [ ] fn `suprnova_live::host::SessionFingerprint::from_bytes` · crates/suprnova-live/src/host/capabilities.rs:43
- [ ] struct `suprnova_live::host::TenantFingerprint` · crates/suprnova-live/src/host/capabilities.rs:47
  - [ ] fn `suprnova_live::host::TenantFingerprint::from_bytes` · crates/suprnova-live/src/host/capabilities.rs:47

### `suprnova_live::host::catalog` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::host::MountCatalog` · crates/suprnova-live/src/host/catalog.rs:310
- [ ] struct `suprnova_live::host::MountCatalogBuilder` · crates/suprnova-live/src/host/catalog.rs:232
  - [ ] fn `suprnova_live::host::MountCatalogBuilder::new` · crates/suprnova-live/src/host/catalog.rs:240
  - [ ] fn `suprnova_live::host::MountCatalogBuilder::register` · crates/suprnova-live/src/host/catalog.rs:245
  - [ ] fn `suprnova_live::host::MountCatalogBuilder::build` · crates/suprnova-live/src/host/catalog.rs:300
- [ ] struct `suprnova_live::host::MountCatalogEntry` · crates/suprnova-live/src/host/catalog.rs:64
  - [ ] fn `suprnova_live::host::MountCatalogEntry::new` · crates/suprnova-live/src/host/catalog.rs:73
  - [ ] fn `suprnova_live::host::MountCatalogEntry::with_document_key` · crates/suprnova-live/src/host/catalog.rs:83
- [ ] struct `suprnova_live::host::MountScopeRequirements` · crates/suprnova-live/src/host/catalog.rs:29
  - [ ] fn `suprnova_live::host::MountScopeRequirements::new` · crates/suprnova-live/src/host/catalog.rs:38
- [ ] struct `suprnova_live::host::MountSelection` · crates/suprnova-live/src/host/catalog.rs:97
  - [ ] fn `suprnova_live::host::MountSelection::new` · crates/suprnova-live/src/host/catalog.rs:108
  - [ ] fn `suprnova_live::host::MountSelection::route` · crates/suprnova-live/src/host/catalog.rs:126
  - [ ] fn `suprnova_live::host::MountSelection::slot` · crates/suprnova-live/src/host/catalog.rs:132
  - [ ] fn `suprnova_live::host::MountSelection::component` · crates/suprnova-live/src/host/catalog.rs:138
  - [ ] fn `suprnova_live::host::MountSelection::contract_digest` · crates/suprnova-live/src/host/catalog.rs:144
  - [ ] fn `suprnova_live::host::MountSelection::protocol` · crates/suprnova-live/src/host/catalog.rs:150
- [ ] struct `suprnova_live::host::VerifiedMountCatalogMatch` · crates/suprnova-live/src/host/catalog.rs:163
  - [ ] fn `suprnova_live::host::VerifiedMountCatalogMatch::component` · crates/suprnova-live/src/host/catalog.rs:175
  - [ ] fn `suprnova_live::host::VerifiedMountCatalogMatch::route` · crates/suprnova-live/src/host/catalog.rs:181
  - [ ] fn `suprnova_live::host::VerifiedMountCatalogMatch::slot` · crates/suprnova-live/src/host/catalog.rs:187
  - [ ] fn `suprnova_live::host::VerifiedMountCatalogMatch::contract_digest` · crates/suprnova-live/src/host/catalog.rs:193
  - [ ] fn `suprnova_live::host::VerifiedMountCatalogMatch::document_key` · crates/suprnova-live/src/host/catalog.rs:199
  - [ ] fn `suprnova_live::host::VerifiedMountCatalogMatch::minimum_protocol` · crates/suprnova-live/src/host/catalog.rs:205
  - [ ] fn `suprnova_live::host::VerifiedMountCatalogMatch::protocol` · crates/suprnova-live/src/host/catalog.rs:211
- [ ] enum `suprnova_live::host::ScopeRequirement` · crates/suprnova-live/src/host/catalog.rs:18
  - Variants: `Required`, `Optional`, `Absent`

### `suprnova_live::host::checks` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::host::CheckFact` · crates/suprnova-live/src/host/checks.rs:76
  - [ ] fn `suprnova_live::host::CheckFact::new` · crates/suprnova-live/src/host/checks.rs:84
- [ ] struct `suprnova_live::host::HostCheckFacts` · crates/suprnova-live/src/host/checks.rs:94
  - [ ] fn `suprnova_live::host::HostCheckFacts::new` · crates/suprnova-live/src/host/checks.rs:101
  - [ ] fn `suprnova_live::host::HostCheckFacts::record` · crates/suprnova-live/src/host/checks.rs:108
  - [ ] fn `suprnova_live::host::HostCheckFacts::require_complete` · crates/suprnova-live/src/host/checks.rs:122
- [ ] struct `suprnova_live::host::RequiredChecks` · crates/suprnova-live/src/host/checks.rs:129
  - [ ] fn `suprnova_live::host::RequiredChecks::get` · crates/suprnova-live/src/host/checks.rs:161
- [ ] enum `suprnova_live::host::CheckDisposition` · crates/suprnova-live/src/host/checks.rs:67
  - Variants: `Passed`, `NotRequired`
- [ ] enum `suprnova_live::host::CheckKind` · crates/suprnova-live/src/host/checks.rs:11
  - Variants: `Origin`, `Csrf`, `Session`, `Principal`, `Tenant`, `Proxy`, `RateLimit`, `Middleware`
  - [ ] const `suprnova_live::host::CheckKind::ALL` · crates/suprnova-live/src/host/checks.rs:32
- [ ] enum `suprnova_live::host::PolicyReason` · crates/suprnova-live/src/host/checks.rs:46
  - Variants: `TrustedInternalOrigin`, `StatelessCsrfPolicy`, `StatelessRequest`, `AnonymousPrincipal`, `TenantlessRoute`, `DirectPeer`, `UpstreamRateLimited`, `NoAdditionalMiddleware`

### `suprnova_live::host::context` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::host::LiveRequestContextCandidate` · crates/suprnova-live/src/host/context.rs:17
  - [ ] fn `suprnova_live::host::LiveRequestContextCandidate::new` · crates/suprnova-live/src/host/context.rs:30
- [ ] struct `suprnova_live::host::LiveRequestContextValidator` · crates/suprnova-live/src/host/context.rs:59
  - [ ] fn `suprnova_live::host::LiveRequestContextValidator::new` · crates/suprnova-live/src/host/context.rs:65
  - [ ] fn `suprnova_live::host::LiveRequestContextValidator::validate` · crates/suprnova-live/src/host/context.rs:75
- [ ] struct `suprnova_live::host::TrustedLiveRequestContext` · crates/suprnova-live/src/host/context.rs:111
  - [ ] fn `suprnova_live::host::TrustedLiveRequestContext::host_scope_facts` · crates/suprnova-live/src/host/context.rs:122
  - [ ] fn `suprnova_live::host::TrustedLiveRequestContext::scope` · crates/suprnova-live/src/host/context.rs:128
  - [ ] fn `suprnova_live::host::TrustedLiveRequestContext::mount` · crates/suprnova-live/src/host/context.rs:134
  - [ ] fn `suprnova_live::host::TrustedLiveRequestContext::checks` · crates/suprnova-live/src/host/context.rs:140
  - [ ] fn `suprnova_live::host::TrustedLiveRequestContext::capabilities` · crates/suprnova-live/src/host/context.rs:146
  - [ ] fn `suprnova_live::host::TrustedLiveRequestContext::expires_at` · crates/suprnova-live/src/host/context.rs:152
  - [ ] fn `suprnova_live::host::TrustedLiveRequestContext::for_promotion` · crates/suprnova-live/src/host/context.rs:158
  - [ ] fn `suprnova_live::host::TrustedLiveRequestContext::is_current` · crates/suprnova-live/src/host/context.rs:164

## identity

### `suprnova_live::identity`

- [ ] struct `suprnova_live::identity::ActionName` · crates/suprnova-live/src/identity.rs:149
  - [ ] fn `suprnova_live::identity::ActionName::parse` · crates/suprnova-live/src/identity.rs:149
  - [ ] fn `suprnova_live::identity::ActionName::as_str` · crates/suprnova-live/src/identity.rs:149
- [ ] struct `suprnova_live::identity::BrowserNonce` · crates/suprnova-live/src/identity.rs:407
  - [ ] fn `suprnova_live::identity::BrowserNonce::parse` · crates/suprnova-live/src/identity.rs:407
  - [ ] fn `suprnova_live::identity::BrowserNonce::from_bytes` · crates/suprnova-live/src/identity.rs:407
  - [ ] fn `suprnova_live::identity::BrowserNonce::as_bytes` · crates/suprnova-live/src/identity.rs:407
  - [ ] fn `suprnova_live::identity::BrowserNonce::to_base64url` · crates/suprnova-live/src/identity.rs:407
- [ ] struct `suprnova_live::identity::BrowserOperationName` · crates/suprnova-live/src/identity.rs:159
  - [ ] fn `suprnova_live::identity::BrowserOperationName::parse` · crates/suprnova-live/src/identity.rs:159
  - [ ] fn `suprnova_live::identity::BrowserOperationName::as_str` · crates/suprnova-live/src/identity.rs:159
- [ ] struct `suprnova_live::identity::BuildId` · crates/suprnova-live/src/identity.rs:134
  - [ ] fn `suprnova_live::identity::BuildId::parse` · crates/suprnova-live/src/identity.rs:134
  - [ ] fn `suprnova_live::identity::BuildId::as_str` · crates/suprnova-live/src/identity.rs:134
- [ ] struct `suprnova_live::identity::ComponentName` · crates/suprnova-live/src/identity.rs:94
  - [ ] fn `suprnova_live::identity::ComponentName::parse` · crates/suprnova-live/src/identity.rs:94
  - [ ] fn `suprnova_live::identity::ComponentName::as_str` · crates/suprnova-live/src/identity.rs:94
- [ ] struct `suprnova_live::identity::ContentDigest` · crates/suprnova-live/src/identity.rs:443
  - [ ] fn `suprnova_live::identity::ContentDigest::parse` · crates/suprnova-live/src/identity.rs:443
  - [ ] fn `suprnova_live::identity::ContentDigest::from_bytes` · crates/suprnova-live/src/identity.rs:443
  - [ ] fn `suprnova_live::identity::ContentDigest::as_bytes` · crates/suprnova-live/src/identity.rs:443
  - [ ] fn `suprnova_live::identity::ContentDigest::to_base64url` · crates/suprnova-live/src/identity.rs:443
- [ ] struct `suprnova_live::identity::CorrelationId` · crates/suprnova-live/src/identity.rs:419
  - [ ] fn `suprnova_live::identity::CorrelationId::parse` · crates/suprnova-live/src/identity.rs:419
  - [ ] fn `suprnova_live::identity::CorrelationId::from_bytes` · crates/suprnova-live/src/identity.rs:419
  - [ ] fn `suprnova_live::identity::CorrelationId::as_bytes` · crates/suprnova-live/src/identity.rs:419
  - [ ] fn `suprnova_live::identity::CorrelationId::to_base64url` · crates/suprnova-live/src/identity.rs:419
- [ ] struct `suprnova_live::identity::DurationMillis` · crates/suprnova-live/src/identity.rs:521
  - [ ] fn `suprnova_live::identity::DurationMillis::new` · crates/suprnova-live/src/identity.rs:521
  - [ ] fn `suprnova_live::identity::DurationMillis::parse` · crates/suprnova-live/src/identity.rs:521
  - [ ] fn `suprnova_live::identity::DurationMillis::get` · crates/suprnova-live/src/identity.rs:521
- [ ] struct `suprnova_live::identity::Generation` · crates/suprnova-live/src/identity.rs:517
  - [ ] fn `suprnova_live::identity::Generation::new` · crates/suprnova-live/src/identity.rs:517
  - [ ] fn `suprnova_live::identity::Generation::parse` · crates/suprnova-live/src/identity.rs:517
  - [ ] fn `suprnova_live::identity::Generation::get` · crates/suprnova-live/src/identity.rs:517
- [ ] struct `suprnova_live::identity::IdempotencyKey` · crates/suprnova-live/src/identity.rs:425
  - [ ] fn `suprnova_live::identity::IdempotencyKey::parse` · crates/suprnova-live/src/identity.rs:425
  - [ ] fn `suprnova_live::identity::IdempotencyKey::from_bytes` · crates/suprnova-live/src/identity.rs:425
  - [ ] fn `suprnova_live::identity::IdempotencyKey::as_bytes` · crates/suprnova-live/src/identity.rs:425
  - [ ] fn `suprnova_live::identity::IdempotencyKey::to_base64url` · crates/suprnova-live/src/identity.rs:425
- [ ] struct `suprnova_live::identity::IdentityError` · crates/suprnova-live/src/identity.rs:26
  - [ ] fn `suprnova_live::identity::IdentityError::kind` · crates/suprnova-live/src/identity.rs:33
- [ ] struct `suprnova_live::identity::InstanceId` · crates/suprnova-live/src/identity.rs:413
  - [ ] fn `suprnova_live::identity::InstanceId::parse` · crates/suprnova-live/src/identity.rs:413
  - [ ] fn `suprnova_live::identity::InstanceId::from_bytes` · crates/suprnova-live/src/identity.rs:413
  - [ ] fn `suprnova_live::identity::InstanceId::as_bytes` · crates/suprnova-live/src/identity.rs:413
  - [ ] fn `suprnova_live::identity::InstanceId::to_base64url` · crates/suprnova-live/src/identity.rs:413
- [ ] struct `suprnova_live::identity::IslandSlot` · crates/suprnova-live/src/identity.rs:139
  - [ ] fn `suprnova_live::identity::IslandSlot::parse` · crates/suprnova-live/src/identity.rs:139
  - [ ] fn `suprnova_live::identity::IslandSlot::as_str` · crates/suprnova-live/src/identity.rs:139
- [ ] struct `suprnova_live::identity::KeyId` · crates/suprnova-live/src/identity.rs:144
  - [ ] fn `suprnova_live::identity::KeyId::parse` · crates/suprnova-live/src/identity.rs:144
  - [ ] fn `suprnova_live::identity::KeyId::as_str` · crates/suprnova-live/src/identity.rs:144
- [ ] struct `suprnova_live::identity::ModelField` · crates/suprnova-live/src/identity.rs:154
  - [ ] fn `suprnova_live::identity::ModelField::parse` · crates/suprnova-live/src/identity.rs:154
  - [ ] fn `suprnova_live::identity::ModelField::as_str` · crates/suprnova-live/src/identity.rs:154
- [ ] struct `suprnova_live::identity::Revision` · crates/suprnova-live/src/identity.rs:509
  - [ ] fn `suprnova_live::identity::Revision::new` · crates/suprnova-live/src/identity.rs:509
  - [ ] fn `suprnova_live::identity::Revision::parse` · crates/suprnova-live/src/identity.rs:509
  - [ ] fn `suprnova_live::identity::Revision::get` · crates/suprnova-live/src/identity.rs:509
  - [ ] fn `suprnova_live::identity::Revision::checked_next` · crates/suprnova-live/src/identity.rs:528
- [ ] struct `suprnova_live::identity::RouteIdentity` · crates/suprnova-live/src/identity.rs:437
  - [ ] fn `suprnova_live::identity::RouteIdentity::parse` · crates/suprnova-live/src/identity.rs:437
  - [ ] fn `suprnova_live::identity::RouteIdentity::from_bytes` · crates/suprnova-live/src/identity.rs:437
  - [ ] fn `suprnova_live::identity::RouteIdentity::as_bytes` · crates/suprnova-live/src/identity.rs:437
  - [ ] fn `suprnova_live::identity::RouteIdentity::to_base64url` · crates/suprnova-live/src/identity.rs:437
- [ ] struct `suprnova_live::identity::ScopeFingerprint` · crates/suprnova-live/src/identity.rs:431
  - [ ] fn `suprnova_live::identity::ScopeFingerprint::parse` · crates/suprnova-live/src/identity.rs:431
  - [ ] fn `suprnova_live::identity::ScopeFingerprint::from_bytes` · crates/suprnova-live/src/identity.rs:431
  - [ ] fn `suprnova_live::identity::ScopeFingerprint::as_bytes` · crates/suprnova-live/src/identity.rs:431
  - [ ] fn `suprnova_live::identity::ScopeFingerprint::to_base64url` · crates/suprnova-live/src/identity.rs:431
- [ ] struct `suprnova_live::identity::SignalName` · crates/suprnova-live/src/identity.rs:167
  - [ ] fn `suprnova_live::identity::SignalName::parse` · crates/suprnova-live/src/identity.rs:171
  - [ ] fn `suprnova_live::identity::SignalName::as_str` · crates/suprnova-live/src/identity.rs:191
- [ ] struct `suprnova_live::identity::SignalScopeIdentity` · crates/suprnova-live/src/identity.rs:204
  - [ ] fn `suprnova_live::identity::SignalScopeIdentity::parse` · crates/suprnova-live/src/identity.rs:209
  - [ ] fn `suprnova_live::identity::SignalScopeIdentity::as_str` · crates/suprnova-live/src/identity.rs:229
- [ ] struct `suprnova_live::identity::UnixMillis` · crates/suprnova-live/src/identity.rs:513
  - [ ] fn `suprnova_live::identity::UnixMillis::new` · crates/suprnova-live/src/identity.rs:513
  - [ ] fn `suprnova_live::identity::UnixMillis::parse` · crates/suprnova-live/src/identity.rs:513
  - [ ] fn `suprnova_live::identity::UnixMillis::get` · crates/suprnova-live/src/identity.rs:513
- [ ] struct `suprnova_live::identity::ViewName` · crates/suprnova-live/src/identity.rs:102
  - [ ] fn `suprnova_live::identity::ViewName::parse` · crates/suprnova-live/src/identity.rs:106
  - [ ] fn `suprnova_live::identity::ViewName::as_str` · crates/suprnova-live/src/identity.rs:124
- [ ] enum `suprnova_live::identity::IdentityErrorKind` · crates/suprnova-live/src/identity.rs:13
  - Variants: `InvalidSyntax`, `InvalidLength`, `InvalidEncoding`, `InvalidDecimal`

## ledger

### `suprnova_live::ledger::contract` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::ledger::AcceptedOutcome` · crates/suprnova-live/src/ledger/contract.rs:429
  - [ ] fn `suprnova_live::ledger::AcceptedOutcome::new` · crates/suprnova-live/src/ledger/contract.rs:437
  - [ ] fn `suprnova_live::ledger::AcceptedOutcome::kind` · crates/suprnova-live/src/ledger/contract.rs:443
- [ ] struct `suprnova_live::ledger::AcceptedOutcomeMetadata` · crates/suprnova-live/src/ledger/contract.rs:456
  - [ ] fn `suprnova_live::ledger::AcceptedOutcomeMetadata::base_revision` · crates/suprnova-live/src/ledger/contract.rs:469
  - [ ] fn `suprnova_live::ledger::AcceptedOutcomeMetadata::successor_revision` · crates/suprnova-live/src/ledger/contract.rs:475
  - [ ] fn `suprnova_live::ledger::AcceptedOutcomeMetadata::outcome` · crates/suprnova-live/src/ledger/contract.rs:481
- [ ] struct `suprnova_live::ledger::ClaimGrant` · crates/suprnova-live/src/ledger/contract.rs:376
  - [ ] fn `suprnova_live::ledger::ClaimGrant::successor_revision` · crates/suprnova-live/src/ledger/contract.rs:391
  - [ ] fn `suprnova_live::ledger::ClaimGrant::into_token` · crates/suprnova-live/src/ledger/contract.rs:397
- [ ] struct `suprnova_live::ledger::ClaimRequest` · crates/suprnova-live/src/ledger/contract.rs:325
  - [ ] fn `suprnova_live::ledger::ClaimRequest::new` · crates/suprnova-live/src/ledger/contract.rs:336
- [ ] struct `suprnova_live::ledger::ClaimToken` · crates/suprnova-live/src/ledger/contract.rs:362
- [ ] struct `suprnova_live::ledger::InstanceAuthority` · crates/suprnova-live/src/ledger/contract.rs:274
  - [ ] fn `suprnova_live::ledger::InstanceAuthority::instance_id` · crates/suprnova-live/src/ledger/contract.rs:295
  - [ ] fn `suprnova_live::ledger::InstanceAuthority::revision` · crates/suprnova-live/src/ledger/contract.rs:301
  - [ ] fn `suprnova_live::ledger::InstanceAuthority::expires_at` · crates/suprnova-live/src/ledger/contract.rs:307
- [ ] struct `suprnova_live::ledger::LedgerError` · crates/suprnova-live/src/ledger/contract.rs:78
  - [ ] fn `suprnova_live::ledger::LedgerError::new` · crates/suprnova-live/src/ledger/contract.rs:95
  - [ ] fn `suprnova_live::ledger::LedgerError::kind` · crates/suprnova-live/src/ledger/contract.rs:101
- [ ] struct `suprnova_live::ledger::LedgerInspection` · crates/suprnova-live/src/ledger/contract.rs:537
  - [ ] fn `suprnova_live::ledger::LedgerInspection::current_revision` · crates/suprnova-live/src/ledger/contract.rs:546
  - [ ] fn `suprnova_live::ledger::LedgerInspection::accepted_outcome_count` · crates/suprnova-live/src/ledger/contract.rs:552
  - [ ] fn `suprnova_live::ledger::LedgerInspection::phase` · crates/suprnova-live/src/ledger/contract.rs:558
- [ ] struct `suprnova_live::ledger::LedgerLimits` · crates/suprnova-live/src/ledger/contract.rs:122
  - [ ] fn `suprnova_live::ledger::LedgerLimits::new` · crates/suprnova-live/src/ledger/contract.rs:131
- [ ] struct `suprnova_live::ledger::MountInstanceRecord` · crates/suprnova-live/src/ledger/contract.rs:190
  - [ ] fn `suprnova_live::ledger::MountInstanceRecord::new` · crates/suprnova-live/src/ledger/contract.rs:201
  - [ ] fn `suprnova_live::ledger::MountInstanceRecord::scope` · crates/suprnova-live/src/ledger/contract.rs:219
  - [ ] fn `suprnova_live::ledger::MountInstanceRecord::instance_id` · crates/suprnova-live/src/ledger/contract.rs:225
  - [ ] fn `suprnova_live::ledger::MountInstanceRecord::component_contract` · crates/suprnova-live/src/ledger/contract.rs:231
- [ ] struct `suprnova_live::ledger::PromotionRecord` · crates/suprnova-live/src/ledger/contract.rs:175
  - [ ] fn `suprnova_live::ledger::PromotionRecord::new` · crates/suprnova-live/src/ledger/contract.rs:239
  - [ ] fn `suprnova_live::ledger::PromotionRecord::with_instance_id` · crates/suprnova-live/src/ledger/contract.rs:259
  - [ ] fn `suprnova_live::ledger::PromotionRecord::with_request_digest` · crates/suprnova-live/src/ledger/contract.rs:266
- [ ] enum `suprnova_live::ledger::AcceptedOutcomeKind` · crates/suprnova-live/src/ledger/contract.rs:414
  - Variants: `Rendered`, `Validation`, `NoRender`, `Redirect`, `Recovery`
- [ ] enum `suprnova_live::ledger::ClaimOutcome` · crates/suprnova-live/src/ledger/contract.rs:503
  - Variants: `Granted`, `InProgress`, `Accepted`, `Stale`, `IdempotencyConflict`, `RefreshRequired`
- [ ] enum `suprnova_live::ledger::LedgerErrorKind` · crates/suprnova-live/src/ledger/contract.rs:24
  - Variants: `InvalidConfiguration`, `InvalidExpiry`, `InstanceConflict`, `CapacityExceeded`, `ClockUnavailable`, `CounterExhausted`, `ClaimMismatch`, `ClaimExpired`, `InstanceExpired`, `ProviderUnavailable`
  - [ ] fn `suprnova_live::ledger::LedgerErrorKind::as_str` · crates/suprnova-live/src/ledger/contract.rs:60
- [ ] enum `suprnova_live::ledger::LedgerPhase` · crates/suprnova-live/src/ledger/contract.rs:526
  - Variants: `Ready`, `Pending`, `Consumed`
- [ ] enum `suprnova_live::ledger::PromotionOutcome` · crates/suprnova-live/src/ledger/contract.rs:314
  - Variants: `Created`, `Existing`, `IdempotencyConflict`
- [ ] enum `suprnova_live::ledger::RefreshReason` · crates/suprnova-live/src/ledger/contract.rs:488
  - Variants: `Missing`, `InstanceExpired`, `Consumed`, `ClaimExpired`, `RevisionExhausted`
- [ ] trait `suprnova_live::ledger::LiveInstanceLedger` · crates/suprnova-live/src/ledger/contract.rs:569
  - Implemented here by: `ledger::DistributedInstanceLedger`, `ledger::MemoryInstanceLedger`
  - [ ] fn `suprnova_live::ledger::LiveInstanceLedger::mount_instance` · crates/suprnova-live/src/ledger/contract.rs:571 (required)
  - [ ] fn `suprnova_live::ledger::LiveInstanceLedger::promote` · crates/suprnova-live/src/ledger/contract.rs:577 (required)
  - [ ] fn `suprnova_live::ledger::LiveInstanceLedger::claim` · crates/suprnova-live/src/ledger/contract.rs:580 (required)
  - [ ] fn `suprnova_live::ledger::LiveInstanceLedger::current_accepted_revision` · crates/suprnova-live/src/ledger/contract.rs:589 (required)
  - [ ] fn `suprnova_live::ledger::LiveInstanceLedger::commit` · crates/suprnova-live/src/ledger/contract.rs:600 (required)
  - [ ] fn `suprnova_live::ledger::LiveInstanceLedger::abandon` · crates/suprnova-live/src/ledger/contract.rs:604 (required)
  - [ ] fn `suprnova_live::ledger::LiveInstanceLedger::abandon_on_drop` · crates/suprnova-live/src/ledger/contract.rs:612 (required)
  - [ ] fn `suprnova_live::ledger::LiveInstanceLedger::fence_on_drop` · crates/suprnova-live/src/ledger/contract.rs:620 (required)

### `suprnova_live::ledger::distributed` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::ledger::DistributedInstanceLedger` · crates/suprnova-live/src/ledger/distributed.rs:722
  - Implements: `suprnova_live::ledger::LiveInstanceLedger`
  - [ ] fn `suprnova_live::ledger::DistributedInstanceLedger::new` · crates/suprnova-live/src/ledger/distributed.rs:757
  - [ ] fn `suprnova_live::ledger::DistributedInstanceLedger::flush_cleanup` · crates/suprnova-live/src/ledger/distributed.rs:776
  - [ ] fn `suprnova_live::ledger::DistributedInstanceLedger::inspect` · crates/suprnova-live/src/ledger/distributed.rs:782
- [ ] struct `suprnova_live::ledger::InstanceRecordKey` · crates/suprnova-live/src/ledger/distributed.rs:42
  - Public fields: `scope`, `instance_id`
- [ ] struct `suprnova_live::ledger::MemoryRecordStore` · crates/suprnova-live/src/ledger/distributed.rs:373
  - Implements: `suprnova_live::ledger::InstanceRecordStore`
  - [ ] fn `suprnova_live::ledger::MemoryRecordStore::new` · crates/suprnova-live/src/ledger/distributed.rs:381
- [ ] struct `suprnova_live::ledger::PromotionRecordKey` · crates/suprnova-live/src/ledger/distributed.rs:71
  - Public fields: `scope`, `idempotency_key`
- [ ] struct `suprnova_live::ledger::StoredRecord` · crates/suprnova-live/src/ledger/distributed.rs:99
  - Public fields: `bytes`, `version`, `expires_at`
- [ ] enum `suprnova_live::ledger::CasOutcome` · crates/suprnova-live/src/ledger/distributed.rs:138
  - Variants: `Stored`, `Conflict`, `Missing`
- [ ] enum `suprnova_live::ledger::CleanupOp` · crates/suprnova-live/src/ledger/distributed.rs:159
  - Variants: `Abandon`, `Fence`
- [ ] trait `suprnova_live::ledger::InstanceRecordStore` · crates/suprnova-live/src/ledger/distributed.rs:205
  - Implemented here by: `ledger::MemoryRecordStore`
  - [ ] fn `suprnova_live::ledger::InstanceRecordStore::load` · crates/suprnova-live/src/ledger/distributed.rs:208 (required)
  - [ ] fn `suprnova_live::ledger::InstanceRecordStore::insert_if_absent` · crates/suprnova-live/src/ledger/distributed.rs:213 (required)
  - [ ] fn `suprnova_live::ledger::InstanceRecordStore::compare_and_store` · crates/suprnova-live/src/ledger/distributed.rs:222 (required)
  - [ ] fn `suprnova_live::ledger::InstanceRecordStore::remove` · crates/suprnova-live/src/ledger/distributed.rs:232 (required)
  - [ ] fn `suprnova_live::ledger::InstanceRecordStore::load_promotion` · crates/suprnova-live/src/ledger/distributed.rs:236 (required)
  - [ ] fn `suprnova_live::ledger::InstanceRecordStore::insert_promotion_if_absent` · crates/suprnova-live/src/ledger/distributed.rs:243 (required)
  - [ ] fn `suprnova_live::ledger::InstanceRecordStore::count_instances` · crates/suprnova-live/src/ledger/distributed.rs:258 (required)

### `suprnova_live::ledger::memory` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::ledger::MemoryInstanceLedger` · crates/suprnova-live/src/ledger/memory.rs:30
  - Implements: `suprnova_live::ledger::LiveInstanceLedger`
  - [ ] fn `suprnova_live::ledger::MemoryInstanceLedger::new` · crates/suprnova-live/src/ledger/memory.rs:41
  - [ ] fn `suprnova_live::ledger::MemoryInstanceLedger::inspect` · crates/suprnova-live/src/ledger/memory.rs:49

### `suprnova_live::ledger::record` (private module; items are public through re-exports)

- [ ] const `suprnova_live::ledger::MAX_RECORD_BYTES` · crates/suprnova-live/src/ledger/record.rs:50
- [ ] const `suprnova_live::ledger::RECORD_VERSION` · crates/suprnova-live/src/ledger/record.rs:42

## limits

### `suprnova_live::limits`

- [ ] struct `suprnova_live::limits::InputLimits` · crates/suprnova-live/src/limits.rs:17
  - [ ] fn `suprnova_live::limits::InputLimits::new` · crates/suprnova-live/src/limits.rs:26
  - [ ] fn `suprnova_live::limits::InputLimits::upload_protocol_v1` · crates/suprnova-live/src/limits.rs:52
  - [ ] fn `suprnova_live::limits::InputLimits::max_bytes` · crates/suprnova-live/src/limits.rs:63
  - [ ] fn `suprnova_live::limits::InputLimits::max_depth` · crates/suprnova-live/src/limits.rs:69
  - [ ] fn `suprnova_live::limits::InputLimits::max_entries` · crates/suprnova-live/src/limits.rs:75
  - [ ] fn `suprnova_live::limits::InputLimits::max_string_bytes` · crates/suprnova-live/src/limits.rs:81
- [ ] struct `suprnova_live::limits::LimitConfigurationError` · crates/suprnova-live/src/limits.rs:99
- [ ] struct `suprnova_live::limits::UploadLimitConfig` · crates/suprnova-live/src/limits.rs:111
  - Public fields: `max_files_per_field`, `max_pending_per_scope`, `max_file_bytes`, `max_aggregate_bytes`, `max_chunk_bytes`, `max_chunks_per_file`, `max_in_flight_bytes`, `max_concurrent_transfers`, `max_creations_per_window`, `creation_window_ms`, `max_retries`, `max_age_ms`, `max_validation_ms`, `max_scan_ms`, `max_storage_bytes`, `max_cleanup_batch`, `max_idempotency_outcomes`
  - [ ] fn `suprnova_live::limits::UploadLimitConfig::reference` · crates/suprnova-live/src/limits.rs:151
- [ ] struct `suprnova_live::limits::UploadLimits` · crates/suprnova-live/src/limits.rs:182
  - [ ] fn `suprnova_live::limits::UploadLimits::new` · crates/suprnova-live/src/limits.rs:186
  - [ ] fn `suprnova_live::limits::UploadLimits::max_files_per_field` · crates/suprnova-live/src/limits.rs:238
  - [ ] fn `suprnova_live::limits::UploadLimits::max_pending_per_scope` · crates/suprnova-live/src/limits.rs:244
  - [ ] fn `suprnova_live::limits::UploadLimits::max_file_bytes` · crates/suprnova-live/src/limits.rs:250
  - [ ] fn `suprnova_live::limits::UploadLimits::max_aggregate_bytes` · crates/suprnova-live/src/limits.rs:256
  - [ ] fn `suprnova_live::limits::UploadLimits::max_chunk_bytes` · crates/suprnova-live/src/limits.rs:262
  - [ ] fn `suprnova_live::limits::UploadLimits::max_chunks_per_file` · crates/suprnova-live/src/limits.rs:268
  - [ ] fn `suprnova_live::limits::UploadLimits::max_in_flight_bytes` · crates/suprnova-live/src/limits.rs:274
  - [ ] fn `suprnova_live::limits::UploadLimits::max_concurrent_transfers` · crates/suprnova-live/src/limits.rs:280
  - [ ] fn `suprnova_live::limits::UploadLimits::max_creations_per_window` · crates/suprnova-live/src/limits.rs:286
  - [ ] fn `suprnova_live::limits::UploadLimits::creation_window_ms` · crates/suprnova-live/src/limits.rs:292
  - [ ] fn `suprnova_live::limits::UploadLimits::max_retries` · crates/suprnova-live/src/limits.rs:298
  - [ ] fn `suprnova_live::limits::UploadLimits::max_age_ms` · crates/suprnova-live/src/limits.rs:304
  - [ ] fn `suprnova_live::limits::UploadLimits::max_validation_ms` · crates/suprnova-live/src/limits.rs:310
  - [ ] fn `suprnova_live::limits::UploadLimits::max_scan_ms` · crates/suprnova-live/src/limits.rs:316
  - [ ] fn `suprnova_live::limits::UploadLimits::max_storage_bytes` · crates/suprnova-live/src/limits.rs:322
  - [ ] fn `suprnova_live::limits::UploadLimits::max_cleanup_batch` · crates/suprnova-live/src/limits.rs:328
  - [ ] fn `suprnova_live::limits::UploadLimits::max_idempotency_outcomes` · crates/suprnova-live/src/limits.rs:334
- [ ] const `suprnova_live::limits::HARD_MAX_DEPTH` · crates/suprnova-live/src/limits.rs:9
- [ ] const `suprnova_live::limits::HARD_MAX_ENTRIES` · crates/suprnova-live/src/limits.rs:11
- [ ] const `suprnova_live::limits::HARD_MAX_INPUT_BYTES` · crates/suprnova-live/src/limits.rs:7
- [ ] const `suprnova_live::limits::HARD_MAX_STRING_BYTES` · crates/suprnova-live/src/limits.rs:13

## metadata

### `suprnova_live::metadata`

- [ ] struct `suprnova_live::metadata::MetadataError` · crates/suprnova-live/src/metadata/mod.rs:133
  - [ ] fn `suprnova_live::metadata::MetadataError::kind` · crates/suprnova-live/src/metadata/mod.rs:151
- [ ] enum `suprnova_live::metadata::MetadataErrorKind` · crates/suprnova-live/src/metadata/mod.rs:26
  - Variants: `InvalidIdentity`, `InvalidVersion`, `UnsupportedProtocol`, `TooManyFields`, `TooManyActions`, `TooManyEvents`, `TooManyEffects`, `TooManyEventTargets`, `InvalidEventTarget`, `DuplicateEventTarget`, `InvalidEventFanout`, `TooManySubscriptions`, `TooManySubscriptionTopics`, `TooManySubscriptionEvents`, `TooManySubscriptionModes`, `DuplicateField`, `DuplicateAction`, `DuplicateEvent`, `DuplicateEffect`, `DuplicateSubscription`, `DuplicateSubscriptionTopic`, `DuplicateSubscriptionEvent`, `DuplicateSubscriptionMode`, `InvalidSubscriptionMetadata`, `UnknownSubscriptionEvent`, `UnregisteredStreamEvent`, `InvalidBindingMetadata`, `InvalidActionMetadata`, `InvalidUploadMetadata`, `DuplicateUrlQueryKey`, `ContractEncodingFailed`
  - [ ] fn `suprnova_live::metadata::MetadataErrorKind::as_str` · crates/suprnova-live/src/metadata/mod.rs:94

### `suprnova_live::metadata::browser` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::metadata::EffectMetadata` · crates/suprnova-live/src/metadata/browser.rs:200
  - [ ] fn `suprnova_live::metadata::EffectMetadata::from_payload` · crates/suprnova-live/src/metadata/browser.rs:218
  - [ ] fn `suprnova_live::metadata::EffectMetadata::name` · crates/suprnova-live/src/metadata/browser.rs:229
  - [ ] fn `suprnova_live::metadata::EffectMetadata::version` · crates/suprnova-live/src/metadata/browser.rs:235
- [ ] struct `suprnova_live::metadata::EventMetadata` · crates/suprnova-live/src/metadata/browser.rs:60
  - [ ] fn `suprnova_live::metadata::EventMetadata::from_payload` · crates/suprnova-live/src/metadata/browser.rs:91
  - [ ] fn `suprnova_live::metadata::EventMetadata::from_payload_with_contract` · crates/suprnova-live/src/metadata/browser.rs:102
  - [ ] fn `suprnova_live::metadata::EventMetadata::name` · crates/suprnova-live/src/metadata/browser.rs:133
  - [ ] fn `suprnova_live::metadata::EventMetadata::version` · crates/suprnova-live/src/metadata/browser.rs:139
  - [ ] fn `suprnova_live::metadata::EventMetadata::payload_contract` · crates/suprnova-live/src/metadata/browser.rs:145
  - [ ] fn `suprnova_live::metadata::EventMetadata::schema` · crates/suprnova-live/src/metadata/browser.rs:151
  - [ ] fn `suprnova_live::metadata::EventMetadata::source` · crates/suprnova-live/src/metadata/browser.rs:157
  - [ ] fn `suprnova_live::metadata::EventMetadata::targets` · crates/suprnova-live/src/metadata/browser.rs:163
  - [ ] fn `suprnova_live::metadata::EventMetadata::order` · crates/suprnova-live/src/metadata/browser.rs:169
  - [ ] fn `suprnova_live::metadata::EventMetadata::cycle` · crates/suprnova-live/src/metadata/browser.rs:175
  - [ ] fn `suprnova_live::metadata::EventMetadata::maximum_fanout` · crates/suprnova-live/src/metadata/browser.rs:181
- [ ] struct `suprnova_live::metadata::PayloadContractIdentity` · crates/suprnova-live/src/metadata/browser.rs:37
  - [ ] fn `suprnova_live::metadata::PayloadContractIdentity::parse` · crates/suprnova-live/src/metadata/browser.rs:41
  - [ ] fn `suprnova_live::metadata::PayloadContractIdentity::as_str` · crates/suprnova-live/src/metadata/browser.rs:47
- [ ] trait `suprnova_live::metadata::EffectPayloadMetadata` · crates/suprnova-live/src/metadata/browser.rs:28
  - [ ] const `suprnova_live::metadata::EffectPayloadMetadata::NAME` · crates/suprnova-live/src/metadata/browser.rs:30
  - [ ] const `suprnova_live::metadata::EffectPayloadMetadata::VERSION` · crates/suprnova-live/src/metadata/browser.rs:32
- [ ] trait `suprnova_live::metadata::EventPayloadMetadata` · crates/suprnova-live/src/metadata/browser.rs:16
  - [ ] const `suprnova_live::metadata::EventPayloadMetadata::NAME` · crates/suprnova-live/src/metadata/browser.rs:18
  - [ ] const `suprnova_live::metadata::EventPayloadMetadata::VERSION` · crates/suprnova-live/src/metadata/browser.rs:20
  - [ ] const `suprnova_live::metadata::EventPayloadMetadata::SCHEMA` · crates/suprnova-live/src/metadata/browser.rs:22
  - [ ] const `suprnova_live::metadata::EventPayloadMetadata::PAYLOAD_CONTRACT` · crates/suprnova-live/src/metadata/browser.rs:24

### `suprnova_live::metadata::component` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::metadata::ComponentMetadata` · crates/suprnova-live/src/metadata/component.rs:22
  - [ ] fn `suprnova_live::metadata::ComponentMetadata::new` · crates/suprnova-live/src/metadata/component.rs:37
  - [ ] fn `suprnova_live::metadata::ComponentMetadata::new_with_browser_contracts` · crates/suprnova-live/src/metadata/component.rs:61
  - [ ] fn `suprnova_live::metadata::ComponentMetadata::new_with_async_contracts` · crates/suprnova-live/src/metadata/component.rs:89
  - [ ] fn `suprnova_live::metadata::ComponentMetadata::identity` · crates/suprnova-live/src/metadata/component.rs:239
  - [ ] fn `suprnova_live::metadata::ComponentMetadata::view` · crates/suprnova-live/src/metadata/component.rs:245
  - [ ] fn `suprnova_live::metadata::ComponentMetadata::versions` · crates/suprnova-live/src/metadata/component.rs:251
  - [ ] fn `suprnova_live::metadata::ComponentMetadata::fields` · crates/suprnova-live/src/metadata/component.rs:257
  - [ ] fn `suprnova_live::metadata::ComponentMetadata::actions` · crates/suprnova-live/src/metadata/component.rs:263
  - [ ] fn `suprnova_live::metadata::ComponentMetadata::events` · crates/suprnova-live/src/metadata/component.rs:269
  - [ ] fn `suprnova_live::metadata::ComponentMetadata::effects` · crates/suprnova-live/src/metadata/component.rs:275
  - [ ] fn `suprnova_live::metadata::ComponentMetadata::subscriptions` · crates/suprnova-live/src/metadata/component.rs:281
  - [ ] fn `suprnova_live::metadata::ComponentMetadata::refresh_on_promote` · crates/suprnova-live/src/metadata/component.rs:287
  - [ ] fn `suprnova_live::metadata::ComponentMetadata::contract_digest` · crates/suprnova-live/src/metadata/component.rs:293

### `suprnova_live::metadata::field` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::metadata::FieldMetadata` · crates/suprnova-live/src/metadata/field.rs:12
  - [ ] fn `suprnova_live::metadata::FieldMetadata::new` · crates/suprnova-live/src/metadata/field.rs:27
  - [ ] fn `suprnova_live::metadata::FieldMetadata::with_model_binding` · crates/suprnova-live/src/metadata/field.rs:47
  - [ ] fn `suprnova_live::metadata::FieldMetadata::with_session_binding` · crates/suprnova-live/src/metadata/field.rs:71
  - [ ] fn `suprnova_live::metadata::FieldMetadata::with_url_binding` · crates/suprnova-live/src/metadata/field.rs:85
  - [ ] fn `suprnova_live::metadata::FieldMetadata::with_upload_policy` · crates/suprnova-live/src/metadata/field.rs:103
  - [ ] fn `suprnova_live::metadata::FieldMetadata::name` · crates/suprnova-live/src/metadata/field.rs:117
  - [ ] fn `suprnova_live::metadata::FieldMetadata::category` · crates/suprnova-live/src/metadata/field.rs:123
  - [ ] fn `suprnova_live::metadata::FieldMetadata::codec` · crates/suprnova-live/src/metadata/field.rs:129
  - [ ] fn `suprnova_live::metadata::FieldMetadata::required` · crates/suprnova-live/src/metadata/field.rs:135
  - [ ] fn `suprnova_live::metadata::FieldMetadata::model_codec` · crates/suprnova-live/src/metadata/field.rs:141
  - [ ] fn `suprnova_live::metadata::FieldMetadata::session_codec` · crates/suprnova-live/src/metadata/field.rs:147
  - [ ] fn `suprnova_live::metadata::FieldMetadata::binding_timing` · crates/suprnova-live/src/metadata/field.rs:153
  - [ ] fn `suprnova_live::metadata::FieldMetadata::url_binding` · crates/suprnova-live/src/metadata/field.rs:159
  - [ ] fn `suprnova_live::metadata::FieldMetadata::upload_policy` · crates/suprnova-live/src/metadata/field.rs:165

### `suprnova_live::metadata::generated` (private module; items are public through re-exports)

- [ ] trait `suprnova_live::metadata::LiveComponentContract` · crates/suprnova-live/src/metadata/generated.rs:17
  - [ ] fn `suprnova_live::metadata::LiveComponentContract::descriptor` · crates/suprnova-live/src/metadata/generated.rs:19 (required)
  - [ ] fn `suprnova_live::metadata::LiveComponentContract::descriptor_with_hooks` · crates/suprnova-live/src/metadata/generated.rs:22 (required)
  - [ ] fn `suprnova_live::metadata::LiveComponentContract::validation_port` · crates/suprnova-live/src/metadata/generated.rs:25 (provided)
  - [ ] fn `suprnova_live::metadata::LiveComponentContract::origin_crate` · crates/suprnova-live/src/metadata/generated.rs:33 (provided)

### `suprnova_live::metadata::method` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::metadata::ActionMetadata` · crates/suprnova-live/src/metadata/method.rs:11
  - [ ] fn `suprnova_live::metadata::ActionMetadata::new` · crates/suprnova-live/src/metadata/method.rs:22
  - [ ] fn `suprnova_live::metadata::ActionMetadata::new_with_contract` · crates/suprnova-live/src/metadata/method.rs:37
  - [ ] fn `suprnova_live::metadata::ActionMetadata::name` · crates/suprnova-live/src/metadata/method.rs:69
  - [ ] fn `suprnova_live::metadata::ActionMetadata::version` · crates/suprnova-live/src/metadata/method.rs:75
  - [ ] fn `suprnova_live::metadata::ActionMetadata::arguments` · crates/suprnova-live/src/metadata/method.rs:81
  - [ ] fn `suprnova_live::metadata::ActionMetadata::authorization` · crates/suprnova-live/src/metadata/method.rs:87
  - [ ] fn `suprnova_live::metadata::ActionMetadata::validation` · crates/suprnova-live/src/metadata/method.rs:93
  - [ ] fn `suprnova_live::metadata::ActionMetadata::transaction` · crates/suprnova-live/src/metadata/method.rs:99

### `suprnova_live::metadata::version` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::metadata::ContractVersions` · crates/suprnova-live/src/metadata/version.rs:8
  - [ ] fn `suprnova_live::metadata::ContractVersions::new` · crates/suprnova-live/src/metadata/version.rs:18
  - [ ] fn `suprnova_live::metadata::ContractVersions::component` · crates/suprnova-live/src/metadata/version.rs:50
  - [ ] fn `suprnova_live::metadata::ContractVersions::state_schema` · crates/suprnova-live/src/metadata/version.rs:56
  - [ ] fn `suprnova_live::metadata::ContractVersions::action_schema` · crates/suprnova-live/src/metadata/version.rs:62
  - [ ] fn `suprnova_live::metadata::ContractVersions::checker_contract` · crates/suprnova-live/src/metadata/version.rs:68
  - [ ] fn `suprnova_live::metadata::ContractVersions::minimum_protocol` · crates/suprnova-live/src/metadata/version.rs:74

## mount

### `suprnova_live::mount::error` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::mount::MountError` · crates/suprnova-live/src/mount/error.rs:64
  - [ ] fn `suprnova_live::mount::MountError::kind` · crates/suprnova-live/src/mount/error.rs:75
- [ ] enum `suprnova_live::mount::MountErrorKind` · crates/suprnova-live/src/mount/error.rs:8
  - Variants: `InvalidConfiguration`, `ContextRejected`, `ComponentRejected`, `ParametersRejected`, `DuplicateDocumentKey`, `DocumentCapacity`, `MetadataTooLarge`, `RandomUnavailable`, `ClockUnavailable`, `LifecycleRejected`, `SnapshotRejected`, `RenderRejected`, `LedgerRejected`, `IdentityCollision`
  - [ ] fn `suprnova_live::mount::MountErrorKind::as_str` · crates/suprnova-live/src/mount/error.rs:42

### `suprnova_live::mount::output` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::mount::DocumentMountKey` · crates/suprnova-live/src/mount/output.rs:20
  - [ ] fn `suprnova_live::mount::DocumentMountKey::parse` · crates/suprnova-live/src/mount/output.rs:24
  - [ ] fn `suprnova_live::mount::DocumentMountKey::as_str` · crates/suprnova-live/src/mount/output.rs:38
- [ ] struct `suprnova_live::mount::DocumentMountScope` · crates/suprnova-live/src/mount/output.rs:45
  - [ ] fn `suprnova_live::mount::DocumentMountScope::new` · crates/suprnova-live/src/mount/output.rs:53
  - [ ] fn `suprnova_live::mount::DocumentMountScope::with_limit` · crates/suprnova-live/src/mount/output.rs:61
  - [ ] fn `suprnova_live::mount::DocumentMountScope::reserve_existing` · crates/suprnova-live/src/mount/output.rs:84
- [ ] struct `suprnova_live::mount::MountFlags` · crates/suprnova-live/src/mount/output.rs:101
  - [ ] fn `suprnova_live::mount::MountFlags::empty` · crates/suprnova-live/src/mount/output.rs:106
  - [ ] fn `suprnova_live::mount::MountFlags::new` · crates/suprnova-live/src/mount/output.rs:111
  - [ ] fn `suprnova_live::mount::MountFlags::len` · crates/suprnova-live/src/mount/output.rs:141
  - [ ] fn `suprnova_live::mount::MountFlags::is_empty` · crates/suprnova-live/src/mount/output.rs:147
  - [ ] fn `suprnova_live::mount::MountFlags::iter` · crates/suprnova-live/src/mount/output.rs:152
- [ ] struct `suprnova_live::mount::PrivateMountOutput` · crates/suprnova-live/src/mount/output.rs:197
  - [ ] fn `suprnova_live::mount::PrivateMountOutput::body` · crates/suprnova-live/src/mount/output.rs:208
  - [ ] fn `suprnova_live::mount::PrivateMountOutput::metadata` · crates/suprnova-live/src/mount/output.rs:214
  - [ ] fn `suprnova_live::mount::PrivateMountOutput::instance_id` · crates/suprnova-live/src/mount/output.rs:220
  - [ ] fn `suprnova_live::mount::PrivateMountOutput::revision` · crates/suprnova-live/src/mount/output.rs:226
  - [ ] fn `suprnova_live::mount::PrivateMountOutput::expires_at` · crates/suprnova-live/src/mount/output.rs:232
  - [ ] fn `suprnova_live::mount::PrivateMountOutput::into_document_parts` · crates/suprnova-live/src/mount/output.rs:238
- [ ] struct `suprnova_live::mount::PrivateMountRequest` · crates/suprnova-live/src/mount/output.rs:160
  - [ ] fn `suprnova_live::mount::PrivateMountRequest::new` · crates/suprnova-live/src/mount/output.rs:170
  - [ ] fn `suprnova_live::mount::PrivateMountRequest::with_document_path` · crates/suprnova-live/src/mount/output.rs:181

### `suprnova_live::mount::public` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::mount::PublicMountProviders` · crates/suprnova-live/src/mount/public.rs:26
  - [ ] fn `suprnova_live::mount::PublicMountProviders::new` · crates/suprnova-live/src/mount/public.rs:35
- [ ] struct `suprnova_live::mount::PublicSeedMountOutput` · crates/suprnova-live/src/mount/public.rs:87
  - [ ] fn `suprnova_live::mount::PublicSeedMountOutput::body` · crates/suprnova-live/src/mount/public.rs:97
  - [ ] fn `suprnova_live::mount::PublicSeedMountOutput::metadata` · crates/suprnova-live/src/mount/public.rs:103
  - [ ] fn `suprnova_live::mount::PublicSeedMountOutput::revision` · crates/suprnova-live/src/mount/public.rs:109
  - [ ] fn `suprnova_live::mount::PublicSeedMountOutput::expires_at` · crates/suprnova-live/src/mount/public.rs:116
  - [ ] fn `suprnova_live::mount::PublicSeedMountOutput::into_document_parts` · crates/suprnova-live/src/mount/public.rs:122
- [ ] struct `suprnova_live::mount::PublicSeedMountRequest` · crates/suprnova-live/src/mount/public.rs:55
  - [ ] fn `suprnova_live::mount::PublicSeedMountRequest::new` · crates/suprnova-live/src/mount/public.rs:65
- [ ] struct `suprnova_live::mount::PublicSeedMountService` · crates/suprnova-live/src/mount/public.rs:143
  - [ ] fn `suprnova_live::mount::PublicSeedMountService::new` · crates/suprnova-live/src/mount/public.rs:155
  - [ ] fn `suprnova_live::mount::PublicSeedMountService::with_island_stream_directive` · crates/suprnova-live/src/mount/public.rs:180
  - [ ] fn `suprnova_live::mount::PublicSeedMountService::mount` · crates/suprnova-live/src/mount/public.rs:193
  - [ ] fn `suprnova_live::mount::PublicSeedMountService::mount_component` · crates/suprnova-live/src/mount/public.rs:222
  - [ ] fn `suprnova_live::mount::PublicSeedMountService::mount_component_for_document` · crates/suprnova-live/src/mount/public.rs:241

### `suprnova_live::mount::service` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::mount::MountLimits` · crates/suprnova-live/src/mount/service.rs:32
  - [ ] fn `suprnova_live::mount::MountLimits::new` · crates/suprnova-live/src/mount/service.rs:76
- [ ] struct `suprnova_live::mount::MountProviders` · crates/suprnova-live/src/mount/service.rs:40
  - [ ] fn `suprnova_live::mount::MountProviders::new` · crates/suprnova-live/src/mount/service.rs:51
- [ ] struct `suprnova_live::mount::PrivateMountService` · crates/suprnova-live/src/mount/service.rs:101
  - [ ] fn `suprnova_live::mount::PrivateMountService::new` · crates/suprnova-live/src/mount/service.rs:116
  - [ ] fn `suprnova_live::mount::PrivateMountService::with_island_stream_directive` · crates/suprnova-live/src/mount/service.rs:144
  - [ ] fn `suprnova_live::mount::PrivateMountService::mount` · crates/suprnova-live/src/mount/service.rs:150

## promotion

### `suprnova_live::promotion::context` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::promotion::TrustedPromotionContext` · crates/suprnova-live/src/promotion/context.rs:11
  - [ ] fn `suprnova_live::promotion::TrustedPromotionContext::scope` · crates/suprnova-live/src/promotion/context.rs:28

### `suprnova_live::promotion::error` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::promotion::PromotionError` · crates/suprnova-live/src/promotion/error.rs:64
  - [ ] fn `suprnova_live::promotion::PromotionError::kind` · crates/suprnova-live/src/promotion/error.rs:75
- [ ] enum `suprnova_live::promotion::PromotionErrorKind` · crates/suprnova-live/src/promotion/error.rs:8
  - Variants: `InvalidConfiguration`, `InputTooLarge`, `SnapshotRejected`, `ContextRejected`, `NonceConflict`, `InProgress`, `AbandonedRetention`, `RateLimited`, `OutstandingLimit`, `RouteComponentLimit`, `StorageLimit`, `RandomUnavailable`, `LedgerUnavailable`, `ProviderInvariant`
  - [ ] fn `suprnova_live::promotion::PromotionErrorKind::as_str` · crates/suprnova-live/src/promotion/error.rs:42

### `suprnova_live::promotion::policy` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::promotion::PromotionLimitConfig` · crates/suprnova-live/src/promotion/policy.rs:24
  - Public fields: `max_seed_bytes`, `window_ms`, `max_promotions_per_window`, `max_outstanding_per_scope`, `max_outstanding_per_route_component`, `promotion_lease_ms`, `abandoned_retention_ms`, `instance_lifetime_ms`, `max_reservations`, `max_rate_buckets`
- [ ] struct `suprnova_live::promotion::PromotionLimits` · crates/suprnova-live/src/promotion/policy.rs:49
  - [ ] fn `suprnova_live::promotion::PromotionLimits::new` · crates/suprnova-live/src/promotion/policy.rs:53

### `suprnova_live::promotion::service` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::promotion::PromotedInstance` · crates/suprnova-live/src/promotion/service.rs:33
  - [ ] fn `suprnova_live::promotion::PromotedInstance::instance_id` · crates/suprnova-live/src/promotion/service.rs:42
  - [ ] fn `suprnova_live::promotion::PromotedInstance::revision` · crates/suprnova-live/src/promotion/service.rs:48
  - [ ] fn `suprnova_live::promotion::PromotedInstance::expires_at` · crates/suprnova-live/src/promotion/service.rs:54
  - [ ] fn `suprnova_live::promotion::PromotedInstance::refresh_before_action` · crates/suprnova-live/src/promotion/service.rs:60
  - [ ] fn `suprnova_live::promotion::PromotedInstance::advisory_generations` · crates/suprnova-live/src/promotion/service.rs:66
- [ ] struct `suprnova_live::promotion::PromotionService` · crates/suprnova-live/src/promotion/service.rs:86
  - [ ] fn `suprnova_live::promotion::PromotionService::new` · crates/suprnova-live/src/promotion/service.rs:98
  - [ ] fn `suprnova_live::promotion::PromotionService::promote` · crates/suprnova-live/src/promotion/service.rs:128
- [ ] enum `suprnova_live::promotion::RefreshBeforeAction` · crates/suprnova-live/src/promotion/service.rs:25
  - Variants: `Required`, `NotRequired`

## protocol

### `suprnova_live::protocol`

- [ ] fn `suprnova_live::protocol::encode_versioned_update_response` · crates/suprnova-live/src/protocol/mod.rs:79
- [ ] fn `suprnova_live::protocol::parse_versioned_update_request` · crates/suprnova-live/src/protocol/mod.rs:50
- [ ] fn `suprnova_live::protocol::parse_versioned_update_response` · crates/suprnova-live/src/protocol/mod.rs:64
- [ ] enum `suprnova_live::protocol::VersionedUpdateRequest` · crates/suprnova-live/src/protocol/mod.rs:33
  - Variants: `V1`, `V2`
- [ ] enum `suprnova_live::protocol::VersionedUpdateResponse` · crates/suprnova-live/src/protocol/mod.rs:42
  - Variants: `V1`, `V2`

### `suprnova_live::protocol::browser_context` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::protocol::BrowserRenderContext` · crates/suprnova-live/src/protocol/browser_context.rs:15
  - [ ] fn `suprnova_live::protocol::BrowserRenderContext::from_request` · crates/suprnova-live/src/protocol/browser_context.rs:21
  - [ ] fn `suprnova_live::protocol::BrowserRenderContext::checked` · crates/suprnova-live/src/protocol/browser_context.rs:37
  - [ ] fn `suprnova_live::protocol::BrowserRenderContext::document_key` · crates/suprnova-live/src/protocol/browser_context.rs:48
- [ ] const `suprnova_live::protocol::DOCUMENT_KEY_EXTENSION_V1` · crates/suprnova-live/src/protocol/browser_context.rs:11

### `suprnova_live::protocol::compatibility` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::protocol::CompatibilityWindow` · crates/suprnova-live/src/protocol/compatibility.rs:34
  - [ ] fn `suprnova_live::protocol::CompatibilityWindow::v1` · crates/suprnova-live/src/protocol/compatibility.rs:41
  - [ ] fn `suprnova_live::protocol::CompatibilityWindow::v2` · crates/suprnova-live/src/protocol/compatibility.rs:47
  - [ ] fn `suprnova_live::protocol::CompatibilityWindow::evaluate` · crates/suprnova-live/src/protocol/compatibility.rs:53
- [ ] struct `suprnova_live::protocol::VersionSet` · crates/suprnova-live/src/protocol/compatibility.rs:5
  - [ ] fn `suprnova_live::protocol::VersionSet::new` · crates/suprnova-live/src/protocol/compatibility.rs:14
- [ ] enum `suprnova_live::protocol::CompatibilityDecision` · crates/suprnova-live/src/protocol/compatibility.rs:25
  - Variants: `Compatible`, `RefreshDocument`

### `suprnova_live::protocol::error` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::protocol::ProtocolError` · crates/suprnova-live/src/protocol/error.rs:76
  - [ ] fn `suprnova_live::protocol::ProtocolError::kind` · crates/suprnova-live/src/protocol/error.rs:87
- [ ] enum `suprnova_live::protocol::ProtocolErrorKind` · crates/suprnova-live/src/protocol/error.rs:8
  - Variants: `InputTooLarge`, `InputTooDeep`, `TooManyEntries`, `DuplicateField`, `InvalidEnvelope`, `UnsupportedVersion`, `InvalidIdentity`, `InvalidSnapshotForm`, `SnapshotTooLarge`, `TooManyModelProposals`, `TooManyOperations`, `TooManyArguments`, `AmbiguousOperation`, `IncompatibleBatch`, `InvalidExtension`, `OutcomeMismatch`, `ErrorRecoveryMismatch`, `UnsafeRedirect`
  - [ ] fn `suprnova_live::protocol::ProtocolErrorKind::as_str` · crates/suprnova-live/src/protocol/error.rs:50

### `suprnova_live::protocol::idempotency` (private module; items are public through re-exports)

- [ ] fn `suprnova_live::protocol::semantic_idempotency_digest_v1` · crates/suprnova-live/src/protocol/idempotency.rs:157
- [ ] struct `suprnova_live::protocol::SemanticIdempotencyInputV1` · crates/suprnova-live/src/protocol/idempotency.rs:22
  - [ ] fn `suprnova_live::protocol::SemanticIdempotencyInputV1::new` · crates/suprnova-live/src/protocol/idempotency.rs:33

### `suprnova_live::protocol::limits` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::protocol::ProtocolLimitConfig` · crates/suprnova-live/src/protocol/limits.rs:12
  - Public fields: `input`, `max_snapshot_bytes`, `max_html_bytes`, `max_model_proposals`, `max_operations`, `max_arguments`, `max_validation_entries`, `max_events`, `max_effects`, `max_extensions`
- [ ] struct `suprnova_live::protocol::ProtocolLimits` · crates/suprnova-live/src/protocol/limits.rs:37
  - [ ] fn `suprnova_live::protocol::ProtocolLimits::new` · crates/suprnova-live/src/protocol/limits.rs:41
  - [ ] fn `suprnova_live::protocol::ProtocolLimits::with_max_operations` · crates/suprnova-live/src/protocol/limits.rs:69
  - [ ] fn `suprnova_live::protocol::ProtocolLimits::with_max_snapshot_bytes` · crates/suprnova-live/src/protocol/limits.rs:78

### `suprnova_live::protocol::ordering` (private module; items are public through re-exports)

- [ ] fn `suprnova_live::protocol::application_plan` · crates/suprnova-live/src/protocol/ordering.rs:56
- [ ] fn `suprnova_live::protocol::application_plan_v2` · crates/suprnova-live/src/protocol/ordering.rs:102
- [ ] enum `suprnova_live::protocol::ApplicationStep` · crates/suprnova-live/src/protocol/ordering.rs:19
  - Variants: `Navigate`, `PreflightMorph`, `Morph`, `ValidateNoRender`, `CommitSnapshotAndRevision`, `ReconcileModelsAndValidation`, `RestoreFocus`, `QueueChildDeliveries`, `ReflectUrl`, `DispatchEvents`, `RunRegisteredEffects`, `SettleFeedback`, `RetainDom`, `RequestFreshRenderWithoutReplay`, `RequestFreshIsland`, `StopLive`
- [ ] enum `suprnova_live::protocol::MorphDisposition` · crates/suprnova-live/src/protocol/ordering.rs:8
  - Variants: `NotAttempted`, `Succeeded`, `FailedAfterAcceptance`

### `suprnova_live::protocol::request` (private module; items are public through re-exports)

- [ ] fn `suprnova_live::protocol::parse_update_request` · crates/suprnova-live/src/protocol/request.rs:148
- [ ] struct `suprnova_live::protocol::UpdateRequest` · crates/suprnova-live/src/protocol/request.rs:59
  - [ ] fn `suprnova_live::protocol::UpdateRequest::protocol_version` · crates/suprnova-live/src/protocol/request.rs:76
  - [ ] fn `suprnova_live::protocol::UpdateRequest::runtime_contract_version` · crates/suprnova-live/src/protocol/request.rs:82
  - [ ] fn `suprnova_live::protocol::UpdateRequest::snapshot_schema_version` · crates/suprnova-live/src/protocol/request.rs:88
  - [ ] fn `suprnova_live::protocol::UpdateRequest::correlation_id` · crates/suprnova-live/src/protocol/request.rs:94
  - [ ] fn `suprnova_live::protocol::UpdateRequest::idempotency_key` · crates/suprnova-live/src/protocol/request.rs:100
  - [ ] fn `suprnova_live::protocol::UpdateRequest::component` · crates/suprnova-live/src/protocol/request.rs:106
  - [ ] fn `suprnova_live::protocol::UpdateRequest::base_revision` · crates/suprnova-live/src/protocol/request.rs:112
  - [ ] fn `suprnova_live::protocol::UpdateRequest::snapshot` · crates/suprnova-live/src/protocol/request.rs:118
  - [ ] fn `suprnova_live::protocol::UpdateRequest::model_proposals` · crates/suprnova-live/src/protocol/request.rs:124
  - [ ] fn `suprnova_live::protocol::UpdateRequest::operations` · crates/suprnova-live/src/protocol/request.rs:130
  - [ ] fn `suprnova_live::protocol::UpdateRequest::extensions` · crates/suprnova-live/src/protocol/request.rs:136
- [ ] enum `suprnova_live::protocol::Operation` · crates/suprnova-live/src/protocol/request.rs:43
  - Variants: `SyncModel`, `InvokeAction`
- [ ] enum `suprnova_live::protocol::SnapshotInput` · crates/suprnova-live/src/protocol/request.rs:19
  - Variants: `Instance`, `SeedPromotion`

### `suprnova_live::protocol::response` (private module; items are public through re-exports)

- [ ] fn `suprnova_live::protocol::parse_update_response` · crates/suprnova-live/src/protocol/response.rs:304
- [ ] struct `suprnova_live::protocol::Emission` · crates/suprnova-live/src/protocol/response.rs:42
  - [ ] fn `suprnova_live::protocol::Emission::name` · crates/suprnova-live/src/protocol/response.rs:50
  - [ ] fn `suprnova_live::protocol::Emission::payload` · crates/suprnova-live/src/protocol/response.rs:56
- [ ] struct `suprnova_live::protocol::UpdateResponse` · crates/suprnova-live/src/protocol/response.rs:62
  - [ ] fn `suprnova_live::protocol::UpdateResponse::protocol_version` · crates/suprnova-live/src/protocol/response.rs:80
  - [ ] fn `suprnova_live::protocol::UpdateResponse::correlation_id` · crates/suprnova-live/src/protocol/response.rs:86
  - [ ] fn `suprnova_live::protocol::UpdateResponse::outcome` · crates/suprnova-live/src/protocol/response.rs:92
  - [ ] fn `suprnova_live::protocol::UpdateResponse::accepted_revision` · crates/suprnova-live/src/protocol/response.rs:98
  - [ ] fn `suprnova_live::protocol::UpdateResponse::snapshot` · crates/suprnova-live/src/protocol/response.rs:104
  - [ ] fn `suprnova_live::protocol::UpdateResponse::render` · crates/suprnova-live/src/protocol/response.rs:110
  - [ ] fn `suprnova_live::protocol::UpdateResponse::redirect` · crates/suprnova-live/src/protocol/response.rs:116
  - [ ] fn `suprnova_live::protocol::UpdateResponse::error` · crates/suprnova-live/src/protocol/response.rs:122
  - [ ] fn `suprnova_live::protocol::UpdateResponse::validation` · crates/suprnova-live/src/protocol/response.rs:128
  - [ ] fn `suprnova_live::protocol::UpdateResponse::events` · crates/suprnova-live/src/protocol/response.rs:134
  - [ ] fn `suprnova_live::protocol::UpdateResponse::effects` · crates/suprnova-live/src/protocol/response.rs:140
  - [ ] fn `suprnova_live::protocol::UpdateResponse::extensions` · crates/suprnova-live/src/protocol/response.rs:146
- [ ] enum `suprnova_live::protocol::RenderPayload` · crates/suprnova-live/src/protocol/response.rs:33
  - Variants: `Html`, `NoRender`
- [ ] enum `suprnova_live::protocol::ResponseOutcome` · crates/suprnova-live/src/protocol/response.rs:18
  - Variants: `Accepted`, `Duplicate`, `Rejected`, `RefreshRequired`, `Fatal`

### `suprnova_live::protocol::v2` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::protocol::ChildParameterAdmissionCarrier` · crates/suprnova-live/src/protocol/v2.rs:156
  - [ ] fn `suprnova_live::protocol::ChildParameterAdmissionCarrier::envelope` · crates/suprnova-live/src/protocol/v2.rs:165
  - [ ] fn `suprnova_live::protocol::ChildParameterAdmissionCarrier::parent_snapshot` · crates/suprnova-live/src/protocol/v2.rs:171
- [ ] struct `suprnova_live::protocol::ChildParameterDelivery` · crates/suprnova-live/src/protocol/v2.rs:409
  - [ ] fn `suprnova_live::protocol::ChildParameterDelivery::child_instance` · crates/suprnova-live/src/protocol/v2.rs:430
  - [ ] fn `suprnova_live::protocol::ChildParameterDelivery::parameter_hash` · crates/suprnova-live/src/protocol/v2.rs:436
  - [ ] fn `suprnova_live::protocol::ChildParameterDelivery::envelope` · crates/suprnova-live/src/protocol/v2.rs:442
- [ ] struct `suprnova_live::protocol::UpdateRequestV2` · crates/suprnova-live/src/protocol/v2.rs:67
  - [ ] fn `suprnova_live::protocol::UpdateRequestV2::protocol_version` · crates/suprnova-live/src/protocol/v2.rs:84
  - [ ] fn `suprnova_live::protocol::UpdateRequestV2::runtime_contract_version` · crates/suprnova-live/src/protocol/v2.rs:90
  - [ ] fn `suprnova_live::protocol::UpdateRequestV2::snapshot_schema_version` · crates/suprnova-live/src/protocol/v2.rs:96
  - [ ] fn `suprnova_live::protocol::UpdateRequestV2::correlation_id` · crates/suprnova-live/src/protocol/v2.rs:102
  - [ ] fn `suprnova_live::protocol::UpdateRequestV2::idempotency_key` · crates/suprnova-live/src/protocol/v2.rs:108
  - [ ] fn `suprnova_live::protocol::UpdateRequestV2::component` · crates/suprnova-live/src/protocol/v2.rs:114
  - [ ] fn `suprnova_live::protocol::UpdateRequestV2::base_revision` · crates/suprnova-live/src/protocol/v2.rs:120
  - [ ] fn `suprnova_live::protocol::UpdateRequestV2::snapshot` · crates/suprnova-live/src/protocol/v2.rs:126
  - [ ] fn `suprnova_live::protocol::UpdateRequestV2::child_parameters` · crates/suprnova-live/src/protocol/v2.rs:132
  - [ ] fn `suprnova_live::protocol::UpdateRequestV2::model_proposals` · crates/suprnova-live/src/protocol/v2.rs:138
  - [ ] fn `suprnova_live::protocol::UpdateRequestV2::operations` · crates/suprnova-live/src/protocol/v2.rs:144
  - [ ] fn `suprnova_live::protocol::UpdateRequestV2::extensions` · crates/suprnova-live/src/protocol/v2.rs:150
- [ ] struct `suprnova_live::protocol::UpdateResponseV2` · crates/suprnova-live/src/protocol/v2.rs:479
  - [ ] fn `suprnova_live::protocol::UpdateResponseV2::protocol_version` · crates/suprnova-live/src/protocol/v2.rs:498
  - [ ] fn `suprnova_live::protocol::UpdateResponseV2::correlation_id` · crates/suprnova-live/src/protocol/v2.rs:504
  - [ ] fn `suprnova_live::protocol::UpdateResponseV2::outcome` · crates/suprnova-live/src/protocol/v2.rs:510
  - [ ] fn `suprnova_live::protocol::UpdateResponseV2::accepted_revision` · crates/suprnova-live/src/protocol/v2.rs:516
  - [ ] fn `suprnova_live::protocol::UpdateResponseV2::snapshot` · crates/suprnova-live/src/protocol/v2.rs:522
  - [ ] fn `suprnova_live::protocol::UpdateResponseV2::render` · crates/suprnova-live/src/protocol/v2.rs:528
  - [ ] fn `suprnova_live::protocol::UpdateResponseV2::redirect` · crates/suprnova-live/src/protocol/v2.rs:534
  - [ ] fn `suprnova_live::protocol::UpdateResponseV2::validation` · crates/suprnova-live/src/protocol/v2.rs:540
  - [ ] fn `suprnova_live::protocol::UpdateResponseV2::events` · crates/suprnova-live/src/protocol/v2.rs:546
  - [ ] fn `suprnova_live::protocol::UpdateResponseV2::effects` · crates/suprnova-live/src/protocol/v2.rs:552
  - [ ] fn `suprnova_live::protocol::UpdateResponseV2::error` · crates/suprnova-live/src/protocol/v2.rs:558
  - [ ] fn `suprnova_live::protocol::UpdateResponseV2::extensions` · crates/suprnova-live/src/protocol/v2.rs:564
  - [ ] fn `suprnova_live::protocol::UpdateResponseV2::child_deliveries` · crates/suprnova-live/src/protocol/v2.rs:570
  - [ ] fn `suprnova_live::protocol::UpdateResponseV2::url_intent` · crates/suprnova-live/src/protocol/v2.rs:576
- [ ] enum `suprnova_live::protocol::OperationV2` · crates/suprnova-live/src/protocol/v2.rs:30
  - Variants: `SyncModel`, `InvokeAction`, `ParamsChanged`, `LazyComplete`, `FreshRender`
  - [ ] fn `suprnova_live::protocol::OperationV2::is_recovery_without_replay` · crates/suprnova-live/src/protocol/v2.rs:54
- [ ] enum `suprnova_live::protocol::UrlIntent` · crates/suprnova-live/src/protocol/v2.rs:455
  - Variants: `Reflected`, `Navigated`
  - [ ] fn `suprnova_live::protocol::UrlIntent::target` · crates/suprnova-live/src/protocol/v2.rs:471

## random

### `suprnova_live::random`

- [ ] struct `suprnova_live::promotion::RandomError` · crates/suprnova-live/src/random.rs:30 (also `suprnova_live::random::RandomError`)
  - [ ] fn `suprnova_live::promotion::RandomError::generation_failed` · crates/suprnova-live/src/random.rs:41
  - [ ] fn `suprnova_live::promotion::RandomError::kind` · crates/suprnova-live/src/random.rs:47
- [ ] struct `suprnova_live::promotion::SystemInstanceIdGenerator` · crates/suprnova-live/src/random.rs:74 (also `suprnova_live::random::SystemInstanceIdGenerator`)
  - Implements: `suprnova_live::promotion::InstanceIdGenerator`
- [ ] enum `suprnova_live::promotion::RandomErrorKind` · crates/suprnova-live/src/random.rs:10 (also `suprnova_live::random::RandomErrorKind`)
  - Variants: `SourceUnavailable`, `InvalidGeneratedIdentity`
  - [ ] fn `suprnova_live::promotion::RandomErrorKind::as_str` · crates/suprnova-live/src/random.rs:20
- [ ] trait `suprnova_live::promotion::InstanceIdGenerator` · crates/suprnova-live/src/random.rs:67 (also `suprnova_live::random::InstanceIdGenerator`)
  - Implemented here by: `promotion::SystemInstanceIdGenerator`
  - [ ] fn `suprnova_live::promotion::InstanceIdGenerator::generate` · crates/suprnova-live/src/random.rs:69 (required)

## registry

### `suprnova_live::registry`

- [ ] struct `suprnova_live::registry::ComponentRegistry` · crates/suprnova-live/src/registry/mod.rs:17
  - [ ] fn `suprnova_live::registry::ComponentRegistry::names` · crates/suprnova-live/src/registry/mod.rs:27
  - [ ] fn `suprnova_live::registry::ComponentRegistry::resolve` · crates/suprnova-live/src/registry/mod.rs:32
  - [ ] fn `suprnova_live::registry::ComponentRegistry::require_contract` · crates/suprnova-live/src/registry/mod.rs:42
  - [ ] fn `suprnova_live::registry::ComponentRegistry::len` · crates/suprnova-live/src/registry/mod.rs:56
  - [ ] fn `suprnova_live::registry::ComponentRegistry::is_empty` · crates/suprnova-live/src/registry/mod.rs:62

### `suprnova_live::registry::builder` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::registry::ComponentRegistryBuilder` · crates/suprnova-live/src/registry/builder.rs:13
  - [ ] fn `suprnova_live::registry::ComponentRegistryBuilder::new` · crates/suprnova-live/src/registry/builder.rs:21
  - [ ] fn `suprnova_live::registry::ComponentRegistryBuilder::register` · crates/suprnova-live/src/registry/builder.rs:29
  - [ ] fn `suprnova_live::registry::ComponentRegistryBuilder::build` · crates/suprnova-live/src/registry/builder.rs:48

### `suprnova_live::registry::descriptor` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::registry::ComponentDescriptor` · crates/suprnova-live/src/registry/descriptor.rs:15
  - [ ] fn `suprnova_live::registry::ComponentDescriptor::new` · crates/suprnova-live/src/registry/descriptor.rs:27
  - [ ] fn `suprnova_live::registry::ComponentDescriptor::with_hooks` · crates/suprnova-live/src/registry/descriptor.rs:40
  - [ ] fn `suprnova_live::registry::ComponentDescriptor::with_composition` · crates/suprnova-live/src/registry/descriptor.rs:53
  - [ ] fn `suprnova_live::registry::ComponentDescriptor::with_actions` · crates/suprnova-live/src/registry/descriptor.rs:66
  - [ ] fn `suprnova_live::registry::ComponentDescriptor::metadata` · crates/suprnova-live/src/registry/descriptor.rs:76
  - [ ] fn `suprnova_live::registry::ComponentDescriptor::contract_digest` · crates/suprnova-live/src/registry/descriptor.rs:82
  - [ ] fn `suprnova_live::registry::ComponentDescriptor::hooks` · crates/suprnova-live/src/registry/descriptor.rs:88
  - [ ] fn `suprnova_live::registry::ComponentDescriptor::parameter_schema` · crates/suprnova-live/src/registry/descriptor.rs:94
  - [ ] fn `suprnova_live::registry::ComponentDescriptor::actions` · crates/suprnova-live/src/registry/descriptor.rs:100
  - [ ] fn `suprnova_live::registry::ComponentDescriptor::supports_params_changed` · crates/suprnova-live/src/registry/descriptor.rs:106
  - [ ] fn `suprnova_live::registry::ComponentDescriptor::supports_lazy_complete` · crates/suprnova-live/src/registry/descriptor.rs:112
  - [ ] fn `suprnova_live::registry::ComponentDescriptor::snapshot_schemas` · crates/suprnova-live/src/registry/descriptor.rs:117

### `suprnova_live::registry::error` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::registry::RegistryError` · crates/suprnova-live/src/registry/error.rs:37
  - [ ] fn `suprnova_live::registry::RegistryError::kind` · crates/suprnova-live/src/registry/error.rs:48
- [ ] enum `suprnova_live::registry::RegistryErrorKind` · crates/suprnova-live/src/registry/error.rs:8
  - Variants: `DuplicateComponent`, `DuplicateView`, `CapacityExceeded`, `NotRegistered`, `ContractMismatch`
  - [ ] fn `suprnova_live::registry::RegistryErrorKind::as_str` · crates/suprnova-live/src/registry/error.rs:24

## render_cache

### `suprnova_live::render_cache`

- [ ] struct `suprnova_live::render_cache::RenderCacheError` · crates/suprnova-live/src/render_cache/mod.rs:120
  - [ ] fn `suprnova_live::render_cache::RenderCacheError::new` · crates/suprnova-live/src/render_cache/mod.rs:127
  - [ ] fn `suprnova_live::render_cache::RenderCacheError::kind` · crates/suprnova-live/src/render_cache/mod.rs:133
- [ ] enum `suprnova_live::render_cache::RenderCacheErrorKind` · crates/suprnova-live/src/render_cache/mod.rs:96
  - Variants: `PolicyInvalid`, `VarianceInvalid`, `KeyInvalid`, `EntryInvalid`, `EntryUnsupported`, `ProviderUnavailable`, `PublicationFenced`, `LeaseFenced`, `AssemblyFailed`

### `suprnova_live::render_cache::coherence`

- [ ] fn `suprnova_live::render_cache::age_seconds` · crates/suprnova-live/src/render_cache/coherence.rs:62 (also `suprnova_live::render_cache::coherence::age_seconds`)
- [ ] fn `suprnova_live::render_cache::evaluate_freshness` · crates/suprnova-live/src/render_cache/coherence.rs:26 (also `suprnova_live::render_cache::coherence::evaluate_freshness`)
- [ ] fn `suprnova_live::render_cache::warning_header` · crates/suprnova-live/src/render_cache/coherence.rs:68 (also `suprnova_live::render_cache::coherence::warning_header`)
- [ ] struct `suprnova_live::render_cache::ValidationLease` · crates/suprnova-live/src/render_cache/coherence.rs:79 (also `suprnova_live::render_cache::coherence::ValidationLease`)
  - [ ] fn `suprnova_live::render_cache::ValidationLease::grant` · crates/suprnova-live/src/render_cache/coherence.rs:87
  - [ ] fn `suprnova_live::render_cache::ValidationLease::valid_at` · crates/suprnova-live/src/render_cache/coherence.rs:97
  - [ ] fn `suprnova_live::render_cache::ValidationLease::hint_invalidate` · crates/suprnova-live/src/render_cache/coherence.rs:102
- [ ] enum `suprnova_live::render_cache::FreshnessState` · crates/suprnova-live/src/render_cache/coherence.rs:8 (also `suprnova_live::render_cache::coherence::FreshnessState`)
  - Variants: `Fresh`, `StaleServable`, `StaleOnError`, `Dead`

### `suprnova_live::render_cache::composite`

- [ ] fn `suprnova_live::render_cache::assemble` · crates/suprnova-live/src/render_cache/composite.rs:868 (also `suprnova_live::render_cache::composite::assemble`)
- [ ] fn `suprnova_live::render_cache::composite::assemble_nested` · crates/suprnova-live/src/render_cache/composite.rs:896
- [ ] fn `suprnova_live::render_cache::composite::descend_nested` · crates/suprnova-live/src/render_cache/composite.rs:733
- [ ] fn `suprnova_live::render_cache::fresh_nonce` · crates/suprnova-live/src/render_cache/composite.rs:623 (also `suprnova_live::render_cache::composite::fresh_nonce`)
- [ ] fn `suprnova_live::render_cache::surrounding_digest` · crates/suprnova-live/src/render_cache/composite.rs:429 (also `suprnova_live::render_cache::composite::surrounding_digest`)
- [ ] fn `suprnova_live::render_cache::valid_nonce` · crates/suprnova-live/src/render_cache/composite.rs:614 (also `suprnova_live::render_cache::composite::valid_nonce`)
- [ ] fn `suprnova_live::render_cache::composite::verify_nested` · crates/suprnova-live/src/render_cache/composite.rs:747
- [ ] struct `suprnova_live::render_cache::AssembledDocument` · crates/suprnova-live/src/render_cache/composite.rs:672 (also `suprnova_live::render_cache::composite::AssembledDocument`)
  - [ ] fn `suprnova_live::render_cache::AssembledDocument::body` · crates/suprnova-live/src/render_cache/composite.rs:681
  - [ ] fn `suprnova_live::render_cache::AssembledDocument::validator` · crates/suprnova-live/src/render_cache/composite.rs:687
  - [ ] fn `suprnova_live::render_cache::AssembledDocument::headers` · crates/suprnova-live/src/render_cache/composite.rs:693
- [ ] struct `suprnova_live::render_cache::AssemblyInput` · crates/suprnova-live/src/render_cache/composite.rs:663 (also `suprnova_live::render_cache::composite::AssemblyInput`)
  - Public fields: `outcomes`, `nonce`
- [ ] struct `suprnova_live::render_cache::CheckedIsland` · crates/suprnova-live/src/render_cache/composite.rs:632 (also `suprnova_live::render_cache::composite::CheckedIsland`)
  - [ ] fn `suprnova_live::render_cache::CheckedIsland::new` · crates/suprnova-live/src/render_cache/composite.rs:641
- [ ] struct `suprnova_live::render_cache::CompositeEntry` · crates/suprnova-live/src/render_cache/composite.rs:513 (also `suprnova_live::render_cache::composite::CompositeEntry`)
  - [ ] fn `suprnova_live::render_cache::CompositeEntry::new` · crates/suprnova-live/src/render_cache/composite.rs:540
  - [ ] fn `suprnova_live::render_cache::CompositeEntry::header` · crates/suprnova-live/src/render_cache/composite.rs:571
  - [ ] fn `suprnova_live::render_cache::CompositeEntry::graph` · crates/suprnova-live/src/render_cache/composite.rs:577
  - [ ] fn `suprnova_live::render_cache::CompositeEntry::shell` · crates/suprnova-live/src/render_cache/composite.rs:583
  - [ ] fn `suprnova_live::render_cache::CompositeEntry::structural_digest` · crates/suprnova-live/src/render_cache/composite.rs:589
  - [ ] fn `suprnova_live::render_cache::CompositeEntry::needs_nonce` · crates/suprnova-live/src/render_cache/composite.rs:595
  - [ ] fn `suprnova_live::render_cache::CompositeEntry::canonical_header_bytes` · crates/suprnova-live/src/render_cache/composite.rs:600
- [ ] struct `suprnova_live::render_cache::CompositeHeader` · crates/suprnova-live/src/render_cache/composite.rs:492 (also `suprnova_live::render_cache::composite::CompositeHeader`)
  - Public fields: `entry`, `graph`
  - [ ] fn `suprnova_live::render_cache::CompositeHeader::canonical_bytes` · crates/suprnova-live/src/render_cache/composite.rs:502
- [ ] struct `suprnova_live::render_cache::HeaderTemplate` · crates/suprnova-live/src/render_cache/composite.rs:290 (also `suprnova_live::render_cache::composite::HeaderTemplate`)
  - Public fields: `name`, `pieces`
- [ ] struct `suprnova_live::render_cache::ParsedSlot` · crates/suprnova-live/src/render_cache/composite.rs:197 (also `suprnova_live::render_cache::composite::ParsedSlot`)
  - Public fields: `route`, `slot`, `document_key`, `component`, `contract_digest`, `protocol`, `build`, `parameters`, `flags`, `on_failure`
- [ ] struct `suprnova_live::render_cache::SegmentGraph` · crates/suprnova-live/src/render_cache/composite.rs:299 (also `suprnova_live::render_cache::composite::SegmentGraph`)
  - Public fields: `segments`, `slots`, `shell_islands`, `nonce_headers`
  - [ ] fn `suprnova_live::render_cache::SegmentGraph::needs_nonce` · crates/suprnova-live/src/render_cache/composite.rs:313
  - [ ] fn `suprnova_live::render_cache::SegmentGraph::validate` · crates/suprnova-live/src/render_cache/composite.rs:321
- [ ] struct `suprnova_live::render_cache::ShellIsland` · crates/suprnova-live/src/render_cache/composite.rs:268 (also `suprnova_live::render_cache::composite::ShellIsland`)
  - Public fields: `slot`, `document_key`
- [ ] struct `suprnova_live::render_cache::StitchSlot` · crates/suprnova-live/src/render_cache/composite.rs:170 (also `suprnova_live::render_cache::composite::StitchSlot`)
  - Public fields: `route`, `slot`, `document_key`, `component`, `contract_digest`, `protocol`, `build`, `parameters`, `flags`, `on_failure`, `surrounding`
  - [ ] fn `suprnova_live::render_cache::StitchSlot::parse` · crates/suprnova-live/src/render_cache/composite.rs:222
- [ ] enum `suprnova_live::render_cache::HeaderPiece` · crates/suprnova-live/src/render_cache/composite.rs:278 (also `suprnova_live::render_cache::composite::HeaderPiece`)
  - Variants: `Text`, `Nonce`
- [ ] enum `suprnova_live::render_cache::composite::NestedFailureCause` · crates/suprnova-live/src/render_cache/composite.rs:709
  - Variants: `Cycle`, `DepthExceeded`, `VersionMismatch`, `LengthMismatch`
- [ ] enum `suprnova_live::render_cache::composite::NestedOutcome` · crates/suprnova-live/src/render_cache/composite.rs:767
  - Variants: `Resolved`, `Fallback`, `Omitted`
- [ ] enum `suprnova_live::render_cache::Segment` · crates/suprnova-live/src/render_cache/composite.rs:82 (also `suprnova_live::render_cache::composite::Segment`)
  - Variants: `Literal`, `Slot`, `Nonce`, `Nested`
  - [ ] fn `suprnova_live::render_cache::Segment::literal_len` · crates/suprnova-live/src/render_cache/composite.rs:121
- [ ] enum `suprnova_live::render_cache::SlotFailurePolicy` · crates/suprnova-live/src/render_cache/composite.rs:136 (also `suprnova_live::render_cache::composite::SlotFailurePolicy`)
  - Variants: `FailDocument`, `Omit`, `Fallback`
- [ ] enum `suprnova_live::render_cache::SlotOutcome` · crates/suprnova-live/src/render_cache/composite.rs:652 (also `suprnova_live::render_cache::composite::SlotOutcome`)
  - Variants: `Rendered`, `Fallback`, `Omitted`
- [ ] const `suprnova_live::render_cache::composite::MAX_FALLBACK_BYTES` · crates/suprnova-live/src/render_cache/composite.rs:61
- [ ] const `suprnova_live::render_cache::composite::MAX_NESTED_SEGMENTS` · crates/suprnova-live/src/render_cache/composite.rs:57
- [ ] const `suprnova_live::render_cache::composite::MAX_NESTING_DEPTH` · crates/suprnova-live/src/render_cache/composite.rs:52
- [ ] const `suprnova_live::render_cache::composite::MAX_NONCE_BYTES` · crates/suprnova-live/src/render_cache/composite.rs:69
- [ ] const `suprnova_live::render_cache::composite::MAX_NONCE_HEADERS` · crates/suprnova-live/src/render_cache/composite.rs:65
- [ ] const `suprnova_live::render_cache::composite::MAX_NONCE_HOLES` · crates/suprnova-live/src/render_cache/composite.rs:36
- [ ] const `suprnova_live::render_cache::composite::MAX_SEGMENTS` · crates/suprnova-live/src/render_cache/composite.rs:43
- [ ] const `suprnova_live::render_cache::composite::MAX_SHELL_ISLANDS` · crates/suprnova-live/src/render_cache/composite.rs:63
- [ ] const `suprnova_live::render_cache::composite::MAX_SLOT_PARAMETER_BYTES` · crates/suprnova-live/src/render_cache/composite.rs:59
- [ ] const `suprnova_live::render_cache::MAX_STITCH_SLOTS` · crates/suprnova-live/src/render_cache/composite.rs:34 (also `suprnova_live::render_cache::composite::MAX_STITCH_SLOTS`)
- [ ] const `suprnova_live::render_cache::composite::SURROUNDING_WINDOW_BYTES` · crates/suprnova-live/src/render_cache/composite.rs:67

### `suprnova_live::render_cache::entry`

- [ ] fn `suprnova_live::render_cache::decode` · crates/suprnova-live/src/render_cache/entry.rs:479 (also `suprnova_live::render_cache::entry::decode`)
- [ ] fn `suprnova_live::render_cache::encode` · crates/suprnova-live/src/render_cache/entry.rs:375 (also `suprnova_live::render_cache::entry::encode`)
- [ ] fn `suprnova_live::render_cache::encode_composite` · crates/suprnova-live/src/render_cache/entry.rs:388 (also `suprnova_live::render_cache::entry::encode_composite`)
- [ ] fn `suprnova_live::render_cache::inspect` · crates/suprnova-live/src/render_cache/entry.rs:579 (also `suprnova_live::render_cache::entry::inspect`)
- [ ] struct `suprnova_live::render_cache::CompleteEntry` · crates/suprnova-live/src/render_cache/entry.rs:207 (also `suprnova_live::render_cache::entry::CompleteEntry`)
  - [ ] fn `suprnova_live::render_cache::CompleteEntry::new` · crates/suprnova-live/src/render_cache/entry.rs:216
  - [ ] fn `suprnova_live::render_cache::CompleteEntry::header` · crates/suprnova-live/src/render_cache/entry.rs:227
  - [ ] fn `suprnova_live::render_cache::CompleteEntry::body` · crates/suprnova-live/src/render_cache/entry.rs:233
  - [ ] fn `suprnova_live::render_cache::CompleteEntry::validator` · crates/suprnova-live/src/render_cache/entry.rs:239
- [ ] struct `suprnova_live::render_cache::EntryHeader` · crates/suprnova-live/src/render_cache/entry.rs:175 (also `suprnova_live::render_cache::entry::EntryHeader`)
  - Public fields: `key`, `class`, `variance`, `published_at_ms`, `fresh_ms`, `stale_servable_ms`, `stale_on_error_ms`, `observed`, `epoch`, `seed_deadline_ms`, `status`, `headers`, `content_encoding`
- [ ] struct `suprnova_live::render_cache::EntryInspection` · crates/suprnova-live/src/render_cache/entry.rs:284 (also `suprnova_live::render_cache::entry::EntryInspection`)
  - Public fields: `kind`, `class`, `body_bytes`, `status`, `published_at_ms`, `epoch`, `observations`, `slots`
- [ ] struct `suprnova_live::render_cache::EntryLimits` · crates/suprnova-live/src/render_cache/entry.rs:31 (also `suprnova_live::render_cache::entry::EntryLimits`)
  - Public fields: `max_body_bytes`, `max_header_bytes`, `max_headers`, `max_observations`
- [ ] struct `suprnova_live::render_cache::SafeHeaders` · crates/suprnova-live/src/render_cache/entry.rs:65 (also `suprnova_live::render_cache::entry::SafeHeaders`)
  - [ ] fn `suprnova_live::render_cache::SafeHeaders::from_pairs` · crates/suprnova-live/src/render_cache/entry.rs:115
  - [ ] fn `suprnova_live::render_cache::SafeHeaders::iter` · crates/suprnova-live/src/render_cache/entry.rs:139
- [ ] enum `suprnova_live::render_cache::DecodedEntry` · crates/suprnova-live/src/render_cache/entry.rs:246 (also `suprnova_live::render_cache::entry::DecodedEntry`)
  - Variants: `Complete`, `Composite`
  - [ ] fn `suprnova_live::render_cache::DecodedEntry::header` · crates/suprnova-live/src/render_cache/entry.rs:256
  - [ ] fn `suprnova_live::render_cache::DecodedEntry::kind` · crates/suprnova-live/src/render_cache/entry.rs:265
  - [ ] fn `suprnova_live::render_cache::DecodedEntry::into_complete` · crates/suprnova-live/src/render_cache/entry.rs:274
- [ ] enum `suprnova_live::render_cache::EntryKind` · crates/suprnova-live/src/render_cache/entry.rs:55 (also `suprnova_live::render_cache::entry::EntryKind`)
  - Variants: `Complete`, `Composite`
- [ ] enum `suprnova_live::render_cache::Validator` · crates/suprnova-live/src/render_cache/entry.rs:146 (also `suprnova_live::render_cache::entry::Validator`)
  - Variants: `Strong`
  - [ ] fn `suprnova_live::render_cache::Validator::strong_for` · crates/suprnova-live/src/render_cache/entry.rs:154
  - [ ] fn `suprnova_live::render_cache::Validator::digest_base64url` · crates/suprnova-live/src/render_cache/entry.rs:160
  - [ ] fn `suprnova_live::render_cache::Validator::etag` · crates/suprnova-live/src/render_cache/entry.rs:168
- [ ] const `suprnova_live::render_cache::entry::ENTRY_FORMAT_VERSION` · crates/suprnova-live/src/render_cache/entry.rs:26
- [ ] const `suprnova_live::render_cache::entry::REPLAYABLE_HEADERS` · crates/suprnova-live/src/render_cache/entry.rs:74

### `suprnova_live::render_cache::generation`

- [ ] struct `suprnova_live::render_cache::GenerationSet` · crates/suprnova-live/src/render_cache/generation.rs:236 (also `suprnova_live::render_cache::generation::GenerationSet`)
  - [ ] fn `suprnova_live::render_cache::GenerationSet::insert_digest` · crates/suprnova-live/src/render_cache/generation.rs:243
  - [ ] fn `suprnova_live::render_cache::GenerationSet::insert` · crates/suprnova-live/src/render_cache/generation.rs:257
  - [ ] fn `suprnova_live::render_cache::GenerationSet::get` · crates/suprnova-live/src/render_cache/generation.rs:267
  - [ ] fn `suprnova_live::render_cache::GenerationSet::get_digest` · crates/suprnova-live/src/render_cache/generation.rs:273
  - [ ] fn `suprnova_live::render_cache::GenerationSet::len` · crates/suprnova-live/src/render_cache/generation.rs:279
  - [ ] fn `suprnova_live::render_cache::GenerationSet::is_empty` · crates/suprnova-live/src/render_cache/generation.rs:285
  - [ ] fn `suprnova_live::render_cache::GenerationSet::digests` · crates/suprnova-live/src/render_cache/generation.rs:291
  - [ ] fn `suprnova_live::render_cache::GenerationSet::digest` · crates/suprnova-live/src/render_cache/generation.rs:298
- [ ] struct `suprnova_live::render_cache::MemoryGenerationLedger` · crates/suprnova-live/src/render_cache/generation.rs:417 (also `suprnova_live::render_cache::generation::MemoryGenerationLedger`)
  - Implements: `suprnova_live::render_cache::GenerationLedger`
  - [ ] fn `suprnova_live::render_cache::MemoryGenerationLedger::new` · crates/suprnova-live/src/render_cache/generation.rs:424
  - [ ] fn `suprnova_live::render_cache::MemoryGenerationLedger::advance_epoch` · crates/suprnova-live/src/render_cache/generation.rs:434
  - [ ] fn `suprnova_live::render_cache::MemoryGenerationLedger::rewind_epoch_for_test` · crates/suprnova-live/src/render_cache/generation.rs:443
- [ ] struct `suprnova_live::render_cache::ObservationWindow` · crates/suprnova-live/src/render_cache/generation.rs:497 (also `suprnova_live::render_cache::generation::ObservationWindow`)
  - [ ] fn `suprnova_live::render_cache::ObservationWindow::open` · crates/suprnova-live/src/render_cache/generation.rs:511
  - [ ] fn `suprnova_live::render_cache::ObservationWindow::observe` · crates/suprnova-live/src/render_cache/generation.rs:521
  - [ ] fn `suprnova_live::render_cache::ObservationWindow::epoch` · crates/suprnova-live/src/render_cache/generation.rs:531
  - [ ] fn `suprnova_live::render_cache::ObservationWindow::close` · crates/suprnova-live/src/render_cache/generation.rs:536
- [ ] enum `suprnova_live::render_cache::CoherenceCheck` · crates/suprnova-live/src/render_cache/generation.rs:551 (also `suprnova_live::render_cache::generation::CoherenceCheck`)
  - Variants: `Coherent`, `Moved`, `Rewound`
  - [ ] fn `suprnova_live::render_cache::CoherenceCheck::compare` · crates/suprnova-live/src/render_cache/generation.rs:574
- [ ] enum `suprnova_live::render_cache::DependencyIdentity` · crates/suprnova-live/src/render_cache/generation.rs:58 (also `suprnova_live::render_cache::generation::DependencyIdentity`)
  - Variants: `Table`, `Record`, `QueryClass`, `Relation`, `Config`, `Feature`, `UnkeyedWrite`, `Locale`, `Route`, `Broad`
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::table` · crates/suprnova-live/src/render_cache/generation.rs:116
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::try_table` · crates/suprnova-live/src/render_cache/generation.rs:121
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::record` · crates/suprnova-live/src/render_cache/generation.rs:129
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::try_record` · crates/suprnova-live/src/render_cache/generation.rs:134
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::query_class` · crates/suprnova-live/src/render_cache/generation.rs:148
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::try_query_class` · crates/suprnova-live/src/render_cache/generation.rs:153
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::config` · crates/suprnova-live/src/render_cache/generation.rs:165
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::try_config` · crates/suprnova-live/src/render_cache/generation.rs:170
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::feature` · crates/suprnova-live/src/render_cache/generation.rs:178
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::try_feature` · crates/suprnova-live/src/render_cache/generation.rs:183
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::unkeyed_write` · crates/suprnova-live/src/render_cache/generation.rs:191
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::try_unkeyed_write` · crates/suprnova-live/src/render_cache/generation.rs:196
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::broad` · crates/suprnova-live/src/render_cache/generation.rs:203
  - [ ] fn `suprnova_live::render_cache::DependencyIdentity::digest` · crates/suprnova-live/src/render_cache/generation.rs:209
- [ ] trait `suprnova_live::render_cache::GenerationLedger` · crates/suprnova-live/src/render_cache/generation.rs:374 (also `suprnova_live::render_cache::generation::GenerationLedger`)
  - Implemented here by: `render_cache::MemoryGenerationLedger`
  - [ ] fn `suprnova_live::render_cache::GenerationLedger::current` · crates/suprnova-live/src/render_cache/generation.rs:378 (required)
  - [ ] fn `suprnova_live::render_cache::GenerationLedger::advance` · crates/suprnova-live/src/render_cache/generation.rs:381 (required)
  - [ ] fn `suprnova_live::render_cache::GenerationLedger::epoch` · crates/suprnova-live/src/render_cache/generation.rs:383 (required)
  - [ ] fn `suprnova_live::render_cache::GenerationLedger::lift_epoch_above` · crates/suprnova-live/src/render_cache/generation.rs:395 (required)
  - [ ] fn `suprnova_live::render_cache::GenerationLedger::current_with_epoch` · crates/suprnova-live/src/render_cache/generation.rs:400 (provided)
- [ ] type `suprnova_live::render_cache::Generation` · crates/suprnova-live/src/render_cache/generation.rs:45 (also `suprnova_live::render_cache::generation::Generation`)
- [ ] const `suprnova_live::render_cache::IDENTITY_VERSION` · crates/suprnova-live/src/render_cache/generation.rs:38 (also `suprnova_live::render_cache::generation::IDENTITY_VERSION`)
- [ ] const `suprnova_live::render_cache::MAX_OBSERVATIONS` · crates/suprnova-live/src/render_cache/generation.rs:35 (also `suprnova_live::render_cache::generation::MAX_OBSERVATIONS`)

### `suprnova_live::render_cache::hot`

- [ ] fn `suprnova_live::render_cache::respond` · crates/suprnova-live/src/render_cache/hot.rs:512 (also `suprnova_live::render_cache::hot::respond`)
- [ ] fn `suprnova_live::render_cache::serve_hot` · crates/suprnova-live/src/render_cache/hot.rs:448 (also `suprnova_live::render_cache::hot::serve_hot`)
- [ ] struct `suprnova_live::render_cache::HotEntry` · crates/suprnova-live/src/render_cache/hot.rs:324 (also `suprnova_live::render_cache::hot::HotEntry`)
  - [ ] fn `suprnova_live::render_cache::HotEntry::prepare` · crates/suprnova-live/src/render_cache/hot.rs:355
  - [ ] fn `suprnova_live::render_cache::HotEntry::entry` · crates/suprnova-live/src/render_cache/hot.rs:395
  - [ ] fn `suprnova_live::render_cache::HotEntry::fence` · crates/suprnova-live/src/render_cache/hot.rs:401
  - [ ] fn `suprnova_live::render_cache::HotEntry::published_at_ms` · crates/suprnova-live/src/render_cache/hot.rs:407
- [ ] struct `suprnova_live::render_cache::HotRequest` · crates/suprnova-live/src/render_cache/hot.rs:428 (also `suprnova_live::render_cache::hot::HotRequest`)
  - Public fields: `method`, `if_none_match`, `now_ms`
- [ ] struct `suprnova_live::render_cache::ResponseParts` · crates/suprnova-live/src/render_cache/hot.rs:457 (also `suprnova_live::render_cache::hot::ResponseParts`)
  - Public fields: `status`, `class`, `shared`, `freshness`, `headers`, `variance`, `validator`, `body`, `published_at_ms`, `seed_deadline_ms`, `cache_control_override`, `content_encoding`

### `suprnova_live::render_cache::http`

- [ ] fn `suprnova_live::render_cache::cache_control_value` · crates/suprnova-live/src/render_cache/http.rs:90 (also `suprnova_live::render_cache::http::cache_control_value`)
- [ ] fn `suprnova_live::render_cache::conditional_matches` · crates/suprnova-live/src/render_cache/http.rs:20 (also `suprnova_live::render_cache::http::conditional_matches`)
- [ ] fn `suprnova_live::render_cache::evaluate_conditional` · crates/suprnova-live/src/render_cache/http.rs:41 (also `suprnova_live::render_cache::http::evaluate_conditional`)
- [ ] fn `suprnova_live::render_cache::vary_value` · crates/suprnova-live/src/render_cache/http.rs:104 (also `suprnova_live::render_cache::http::vary_value`)
- [ ] enum `suprnova_live::render_cache::ConditionalOutcome` · crates/suprnova-live/src/render_cache/http.rs:9 (also `suprnova_live::render_cache::http::ConditionalOutcome`)
  - Variants: `NotModified`, `Full`

### `suprnova_live::render_cache::key`

- [ ] struct `suprnova_live::render_cache::RenderKey` · crates/suprnova-live/src/render_cache/key.rs:65 (also `suprnova_live::render_cache::key::RenderKey`)
  - [ ] fn `suprnova_live::render_cache::RenderKey::derive` · crates/suprnova-live/src/render_cache/key.rs:71
  - [ ] fn `suprnova_live::render_cache::RenderKey::to_base64url` · crates/suprnova-live/src/render_cache/key.rs:129
  - [ ] fn `suprnova_live::render_cache::RenderKey::from_base64url` · crates/suprnova-live/src/render_cache/key.rs:143
  - [ ] fn `suprnova_live::render_cache::RenderKey::digest` · crates/suprnova-live/src/render_cache/key.rs:155
- [ ] struct `suprnova_live::render_cache::RenderKeyDimensions` · crates/suprnova-live/src/render_cache/key.rs:202 (also `suprnova_live::render_cache::key::RenderKeyDimensions`)
  - [ ] fn `suprnova_live::render_cache::RenderKeyDimensions::describe` · crates/suprnova-live/src/render_cache/key.rs:219
  - [ ] fn `suprnova_live::render_cache::RenderKeyDimensions::opaque` · crates/suprnova-live/src/render_cache/key.rs:250
  - [ ] fn `suprnova_live::render_cache::RenderKeyDimensions::route` · crates/suprnova-live/src/render_cache/key.rs:266
  - [ ] fn `suprnova_live::render_cache::RenderKeyDimensions::params` · crates/suprnova-live/src/render_cache/key.rs:271
  - [ ] fn `suprnova_live::render_cache::RenderKeyDimensions::query` · crates/suprnova-live/src/render_cache/key.rs:276
  - [ ] fn `suprnova_live::render_cache::RenderKeyDimensions::variance` · crates/suprnova-live/src/render_cache/key.rs:281
- [ ] struct `suprnova_live::render_cache::RenderKeyInput` · crates/suprnova-live/src/render_cache/key.rs:27 (also `suprnova_live::render_cache::key::RenderKeyInput`)
  - Public fields: `route`, `route_pattern`, `params`, `query`, `host`, `media`, `encoding`, `build`, `epoch`, `variance`
- [ ] const `suprnova_live::render_cache::key::KEY_FORMAT_VERSION` · crates/suprnova-live/src/render_cache/key.rs:15
- [ ] const `suprnova_live::render_cache::key::MAX_PARAM_BYTES` · crates/suprnova-live/src/render_cache/key.rs:19
- [ ] const `suprnova_live::render_cache::key::MAX_PARAMS` · crates/suprnova-live/src/render_cache/key.rs:17

### `suprnova_live::render_cache::lease`

- [ ] struct `suprnova_live::render_cache::FencedLeaseCoordinator` · crates/suprnova-live/src/render_cache/lease.rs:218 (also `suprnova_live::render_cache::lease::FencedLeaseCoordinator`)
  - Implements: `suprnova_live::render_cache::RebuildCoordinator`
  - [ ] fn `suprnova_live::render_cache::FencedLeaseCoordinator::new` · crates/suprnova-live/src/render_cache/lease.rs:227
- [ ] struct `suprnova_live::render_cache::MemoryLeaseStore` · crates/suprnova-live/src/render_cache/lease.rs:108 (also `suprnova_live::render_cache::lease::MemoryLeaseStore`)
  - Implements: `suprnova_live::render_cache::LeaseStore`
  - [ ] fn `suprnova_live::render_cache::MemoryLeaseStore::new` · crates/suprnova-live/src/render_cache/lease.rs:117
  - [ ] fn `suprnova_live::render_cache::MemoryLeaseStore::epoch` · crates/suprnova-live/src/render_cache/lease.rs:143
- [ ] enum `suprnova_live::render_cache::LeaseAttempt` · crates/suprnova-live/src/render_cache/lease.rs:42 (also `suprnova_live::render_cache::lease::LeaseAttempt`)
  - Variants: `Acquired`, `Held`
- [ ] trait `suprnova_live::render_cache::LeaseStore` · crates/suprnova-live/src/render_cache/lease.rs:60 (also `suprnova_live::render_cache::lease::LeaseStore`)
  - Implemented here by: `render_cache::MemoryLeaseStore`
  - [ ] fn `suprnova_live::render_cache::LeaseStore::try_acquire` · crates/suprnova-live/src/render_cache/lease.rs:65 (required)
  - [ ] fn `suprnova_live::render_cache::LeaseStore::mint_token` · crates/suprnova-live/src/render_cache/lease.rs:75 (required)
  - [ ] fn `suprnova_live::render_cache::LeaseStore::release` · crates/suprnova-live/src/render_cache/lease.rs:82 (required)

### `suprnova_live::render_cache::policy`

- [ ] struct `suprnova_live::render_cache::FreshnessPolicy` · crates/suprnova-live/src/render_cache/policy.rs:44 (also `suprnova_live::render_cache::policy::FreshnessPolicy`)
  - [ ] fn `suprnova_live::render_cache::FreshnessPolicy::new` · crates/suprnova-live/src/render_cache/policy.rs:52
  - [ ] fn `suprnova_live::render_cache::FreshnessPolicy::fresh_ms` · crates/suprnova-live/src/render_cache/policy.rs:72
  - [ ] fn `suprnova_live::render_cache::FreshnessPolicy::stale_servable_ms` · crates/suprnova-live/src/render_cache/policy.rs:78
  - [ ] fn `suprnova_live::render_cache::FreshnessPolicy::stale_on_error_ms` · crates/suprnova-live/src/render_cache/policy.rs:84
  - [ ] fn `suprnova_live::render_cache::FreshnessPolicy::dead_after_ms` · crates/suprnova-live/src/render_cache/policy.rs:109
- [ ] struct `suprnova_live::render_cache::NegotiatedPolicy` · crates/suprnova-live/src/render_cache/policy.rs:267 (also `suprnova_live::render_cache::policy::NegotiatedPolicy`)
  - [ ] fn `suprnova_live::render_cache::NegotiatedPolicy::declared` · crates/suprnova-live/src/render_cache/policy.rs:278
  - [ ] fn `suprnova_live::render_cache::NegotiatedPolicy::accepted` · crates/suprnova-live/src/render_cache/policy.rs:306
  - [ ] fn `suprnova_live::render_cache::NegotiatedPolicy::default_value` · crates/suprnova-live/src/render_cache/policy.rs:313
  - [ ] fn `suprnova_live::render_cache::NegotiatedPolicy::negotiate` · crates/suprnova-live/src/render_cache/policy.rs:322
- [ ] struct `suprnova_live::render_cache::PolicyPatch` · crates/suprnova-live/src/render_cache/policy.rs:745 (also `suprnova_live::render_cache::policy::PolicyPatch`)
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::class` · crates/suprnova-live/src/render_cache/policy.rs:761
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::freshness` · crates/suprnova-live/src/render_cache/policy.rs:768
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::layers` · crates/suprnova-live/src/render_cache/policy.rs:775
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::coherence` · crates/suprnova-live/src/render_cache/policy.rs:782
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::shared` · crates/suprnova-live/src/render_cache/policy.rs:789
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::failure` · crates/suprnova-live/src/render_cache/policy.rs:796
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::query` · crates/suprnova-live/src/render_cache/policy.rs:803
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::vary` · crates/suprnova-live/src/render_cache/policy.rs:814
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::media` · crates/suprnova-live/src/render_cache/policy.rs:823
  - [ ] fn `suprnova_live::render_cache::PolicyPatch::encoding` · crates/suprnova-live/src/render_cache/policy.rs:832
- [ ] struct `suprnova_live::render_cache::QueryPolicy` · crates/suprnova-live/src/render_cache/policy.rs:201 (also `suprnova_live::render_cache::policy::QueryPolicy`)
  - [ ] fn `suprnova_live::render_cache::QueryPolicy::declared` · crates/suprnova-live/src/render_cache/policy.rs:208
  - [ ] fn `suprnova_live::render_cache::QueryPolicy::none` · crates/suprnova-live/src/render_cache/policy.rs:221
  - [ ] fn `suprnova_live::render_cache::QueryPolicy::declared_names` · crates/suprnova-live/src/render_cache/policy.rs:227
  - [ ] fn `suprnova_live::render_cache::QueryPolicy::unknown` · crates/suprnova-live/src/render_cache/policy.rs:233
- [ ] struct `suprnova_live::render_cache::RenderCachePolicy` · crates/suprnova-live/src/render_cache/policy.rs:369 (also `suprnova_live::render_cache::policy::RenderCachePolicy`)
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::builder` · crates/suprnova-live/src/render_cache/policy.rs:387
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::class` · crates/suprnova-live/src/render_cache/policy.rs:410
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::freshness` · crates/suprnova-live/src/render_cache/policy.rs:416
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::layers` · crates/suprnova-live/src/render_cache/policy.rs:422
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::coherence` · crates/suprnova-live/src/render_cache/policy.rs:428
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::shared` · crates/suprnova-live/src/render_cache/policy.rs:434
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::failure` · crates/suprnova-live/src/render_cache/policy.rs:440
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::query` · crates/suprnova-live/src/render_cache/policy.rs:446
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::vary` · crates/suprnova-live/src/render_cache/policy.rs:452
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::media` · crates/suprnova-live/src/render_cache/policy.rs:459
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::encoding` · crates/suprnova-live/src/render_cache/policy.rs:467
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::apply` · crates/suprnova-live/src/render_cache/policy.rs:473
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicy::eligibility` · crates/suprnova-live/src/render_cache/policy.rs:594
- [ ] struct `suprnova_live::render_cache::RenderCachePolicyBuilder` · crates/suprnova-live/src/render_cache/policy.rs:653 (also `suprnova_live::render_cache::policy::RenderCachePolicyBuilder`)
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::freshness` · crates/suprnova-live/src/render_cache/policy.rs:660
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::layers` · crates/suprnova-live/src/render_cache/policy.rs:667
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::coherence` · crates/suprnova-live/src/render_cache/policy.rs:674
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::shared` · crates/suprnova-live/src/render_cache/policy.rs:681
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::failure` · crates/suprnova-live/src/render_cache/policy.rs:688
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::query` · crates/suprnova-live/src/render_cache/policy.rs:695
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::vary` · crates/suprnova-live/src/render_cache/policy.rs:708
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::vary_media` · crates/suprnova-live/src/render_cache/policy.rs:718
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::vary_encoding` · crates/suprnova-live/src/render_cache/policy.rs:730
  - [ ] fn `suprnova_live::render_cache::RenderCachePolicyBuilder::build` · crates/suprnova-live/src/render_cache/policy.rs:737
- [ ] struct `suprnova_live::render_cache::ResponseSignals` · crates/suprnova-live/src/render_cache/policy.rs:840 (also `suprnova_live::render_cache::policy::ResponseSignals`)
  - Public fields: `method`, `status`, `streaming`, `sets_cookie`, `content_type`, `cache_control`, `header_names`, `private_observed`
- [ ] struct `suprnova_live::render_cache::StorageLayers` · crates/suprnova-live/src/render_cache/policy.rs:124 (also `suprnova_live::render_cache::policy::StorageLayers`)
  - [ ] fn `suprnova_live::render_cache::StorageLayers::l0_only` · crates/suprnova-live/src/render_cache/policy.rs:132
  - [ ] fn `suprnova_live::render_cache::StorageLayers::l0_and_l1` · crates/suprnova-live/src/render_cache/policy.rs:141
  - [ ] fn `suprnova_live::render_cache::StorageLayers::l0` · crates/suprnova-live/src/render_cache/policy.rs:147
  - [ ] fn `suprnova_live::render_cache::StorageLayers::l1` · crates/suprnova-live/src/render_cache/policy.rs:153
- [ ] enum `suprnova_live::render_cache::CoherenceMode` · crates/suprnova-live/src/render_cache/policy.rs:160 (also `suprnova_live::render_cache::policy::CoherenceMode`)
  - Variants: `Authority`, `Lease`
- [ ] enum `suprnova_live::render_cache::DeclineReason` · crates/suprnova-live/src/render_cache/policy.rs:873 (also `suprnova_live::render_cache::policy::DeclineReason`)
  - Variants: `PolicyUncacheable`, `Method`, `Status`, `Streaming`, `SetsCookie`, `UnsafeHeader`, `NoStore`
- [ ] enum `suprnova_live::render_cache::Eligibility` · crates/suprnova-live/src/render_cache/policy.rs:864 (also `suprnova_live::render_cache::policy::Eligibility`)
  - Variants: `Store`, `Decline`
- [ ] enum `suprnova_live::render_cache::FailurePolicy` · crates/suprnova-live/src/render_cache/policy.rs:185 (also `suprnova_live::render_cache::policy::FailurePolicy`)
  - Variants: `Open`, `Closed`
- [ ] enum `suprnova_live::render_cache::QueryUnknown` · crates/suprnova-live/src/render_cache/policy.rs:194 (also `suprnova_live::render_cache::policy::QueryUnknown`)
  - Variants: `Bypass`
- [ ] enum `suprnova_live::render_cache::RepresentationClass` · crates/suprnova-live/src/render_cache/policy.rs:23 (also `suprnova_live::render_cache::policy::RepresentationClass`)
  - Variants: `PublicShared`, `PublicShellStitched`, `PrivateCached`, `Uncacheable`
  - [ ] fn `suprnova_live::render_cache::RepresentationClass::narrowest` · crates/suprnova-live/src/render_cache/policy.rs:37
- [ ] enum `suprnova_live::render_cache::SharedCachePolicy` · crates/suprnova-live/src/render_cache/policy.rs:173 (also `suprnova_live::render_cache::policy::SharedCachePolicy`)
  - Variants: `Private`, `SMaxAge`
- [ ] const `suprnova_live::render_cache::policy::MAX_DECLARED_QUERY` · crates/suprnova-live/src/render_cache/policy.rs:12
- [ ] const `suprnova_live::render_cache::policy::MAX_INTERVAL_MS` · crates/suprnova-live/src/render_cache/policy.rs:10
- [ ] const `suprnova_live::render_cache::policy::MAX_NEGOTIATED_VALUE_BYTES` · crates/suprnova-live/src/render_cache/policy.rs:241
- [ ] const `suprnova_live::render_cache::policy::MAX_NEGOTIATED_VALUES` · crates/suprnova-live/src/render_cache/policy.rs:239
- [ ] const `suprnova_live::render_cache::policy::MAX_NEGOTIATION_ENTRIES` · crates/suprnova-live/src/render_cache/policy.rs:245
- [ ] const `suprnova_live::render_cache::policy::UNSAFE_RESPONSE_HEADERS` · crates/suprnova-live/src/render_cache/policy.rs:640

### `suprnova_live::render_cache::singleflight`

- [ ] struct `suprnova_live::render_cache::LocalCoordinatorLimits` · crates/suprnova-live/src/render_cache/singleflight.rs:192 (also `suprnova_live::render_cache::singleflight::LocalCoordinatorLimits`)
  - Public fields: `lease_ms`, `max_waiters`
- [ ] struct `suprnova_live::render_cache::LocalRebuildCoordinator` · crates/suprnova-live/src/render_cache/singleflight.rs:220 (also `suprnova_live::render_cache::singleflight::LocalRebuildCoordinator`)
  - Implements: `suprnova_live::render_cache::RebuildCoordinator`
  - [ ] fn `suprnova_live::render_cache::LocalRebuildCoordinator::new` · crates/suprnova-live/src/render_cache/singleflight.rs:228
- [ ] struct `suprnova_live::render_cache::RebuildLease` · crates/suprnova-live/src/render_cache/singleflight.rs:19 (also `suprnova_live::render_cache::singleflight::RebuildLease`)
  - [ ] fn `suprnova_live::render_cache::RebuildLease::key` · crates/suprnova-live/src/render_cache/singleflight.rs:57
  - [ ] fn `suprnova_live::render_cache::RebuildLease::distributed_lease_id` · crates/suprnova-live/src/render_cache/singleflight.rs:64
- [ ] struct `suprnova_live::render_cache::RebuildWait` · crates/suprnova-live/src/render_cache/singleflight.rs:115 (also `suprnova_live::render_cache::singleflight::RebuildWait`)
  - [ ] fn `suprnova_live::render_cache::RebuildWait::wait` · crates/suprnova-live/src/render_cache/singleflight.rs:121
- [ ] enum `suprnova_live::render_cache::RebuildAdmission` · crates/suprnova-live/src/render_cache/singleflight.rs:160 (also `suprnova_live::render_cache::singleflight::RebuildAdmission`)
  - Variants: `Lead`, `Wait`, `Bypass`
- [ ] trait `suprnova_live::render_cache::RebuildCoordinator` · crates/suprnova-live/src/render_cache/singleflight.rs:172 (also `suprnova_live::render_cache::singleflight::RebuildCoordinator`)
  - Implemented here by: `render_cache::FencedLeaseCoordinator`, `render_cache::LocalRebuildCoordinator`
  - [ ] fn `suprnova_live::render_cache::RebuildCoordinator::admit` · crates/suprnova-live/src/render_cache/singleflight.rs:174 (required)
  - [ ] fn `suprnova_live::render_cache::RebuildCoordinator::publish_token` · crates/suprnova-live/src/render_cache/singleflight.rs:181 (required)
  - [ ] fn `suprnova_live::render_cache::RebuildCoordinator::release` · crates/suprnova-live/src/render_cache/singleflight.rs:187 (required)

### `suprnova_live::render_cache::store`

- [ ] struct `suprnova_live::render_cache::MemoryRenderStore` · crates/suprnova-live/src/render_cache/store.rs:119 (also `suprnova_live::render_cache::store::MemoryRenderStore`)
  - Implements: `suprnova_live::render_cache::RenderStore`
  - [ ] fn `suprnova_live::render_cache::MemoryRenderStore::new` · crates/suprnova-live/src/render_cache/store.rs:127
  - [ ] fn `suprnova_live::render_cache::MemoryRenderStore::publish_hot` · crates/suprnova-live/src/render_cache/store.rs:154
  - [ ] fn `suprnova_live::render_cache::MemoryRenderStore::hot_get` · crates/suprnova-live/src/render_cache/store.rs:177
  - [ ] fn `suprnova_live::render_cache::MemoryRenderStore::clear` · crates/suprnova-live/src/render_cache/store.rs:273
- [ ] struct `suprnova_live::render_cache::MemoryStoreLimits` · crates/suprnova-live/src/render_cache/store.rs:100 (also `suprnova_live::render_cache::store::MemoryStoreLimits`)
  - Public fields: `max_entries`, `max_bytes`
- [ ] struct `suprnova_live::render_cache::PublicationFence` · crates/suprnova-live/src/render_cache/store.rs:16 (also `suprnova_live::render_cache::store::PublicationFence`)
  - Public fields: `epoch`, `generation_digest`, `token`
  - [ ] fn `suprnova_live::render_cache::PublicationFence::supersedes` · crates/suprnova-live/src/render_cache/store.rs:30
- [ ] struct `suprnova_live::render_cache::StoredEntry` · crates/suprnova-live/src/render_cache/store.rs:37 (also `suprnova_live::render_cache::store::StoredEntry`)
  - Public fields: `bytes`, `published_at_ms`, `fence`
- [ ] struct `suprnova_live::render_cache::StoreInspection` · crates/suprnova-live/src/render_cache/store.rs:59 (also `suprnova_live::render_cache::store::StoreInspection`)
  - Public fields: `entries`, `bytes`
- [ ] enum `suprnova_live::render_cache::PublishOutcome` · crates/suprnova-live/src/render_cache/store.rs:48 (also `suprnova_live::render_cache::store::PublishOutcome`)
  - Variants: `Published`, `Fenced`, `Rejected`
- [ ] trait `suprnova_live::render_cache::RenderStore` · crates/suprnova-live/src/render_cache/store.rs:69 (also `suprnova_live::render_cache::store::RenderStore`)
  - Implemented here by: `render_cache::MemoryRenderStore`
  - [ ] fn `suprnova_live::render_cache::RenderStore::get` · crates/suprnova-live/src/render_cache/store.rs:71 (required)
  - [ ] fn `suprnova_live::render_cache::RenderStore::publish` · crates/suprnova-live/src/render_cache/store.rs:84 (required)
  - [ ] fn `suprnova_live::render_cache::RenderStore::evict` · crates/suprnova-live/src/render_cache/store.rs:93 (required)
  - [ ] fn `suprnova_live::render_cache::RenderStore::inspect` · crates/suprnova-live/src/render_cache/store.rs:95 (required)

### `suprnova_live::render_cache::variance`

- [ ] fn `suprnova_live::render_cache::classify` · crates/suprnova-live/src/render_cache/variance.rs:413 (also `suprnova_live::render_cache::variance::classify`)
- [ ] struct `suprnova_live::render_cache::ClassificationOutcome` · crates/suprnova-live/src/render_cache/variance.rs:404 (also `suprnova_live::render_cache::variance::ClassificationOutcome`)
  - Public fields: `class`, `reasons`
- [ ] struct `suprnova_live::render_cache::ObservedContext` · crates/suprnova-live/src/render_cache/variance.rs:368 (also `suprnova_live::render_cache::variance::ObservedContext`)
  - Public fields: `principal`, `tenant`, `session_read`, `authorization`, `secret_context_read`, `undeclared_reads`
- [ ] struct `suprnova_live::render_cache::PrivateMaterial` · crates/suprnova-live/src/render_cache/variance.rs:131 (also `suprnova_live::render_cache::variance::PrivateMaterial`)
  - [ ] fn `suprnova_live::render_cache::PrivateMaterial::principal` · crates/suprnova-live/src/render_cache/variance.rs:146
  - [ ] fn `suprnova_live::render_cache::PrivateMaterial::tenant` · crates/suprnova-live/src/render_cache/variance.rs:156
  - [ ] fn `suprnova_live::render_cache::PrivateMaterial::as_bytes` · crates/suprnova-live/src/render_cache/variance.rs:166
- [ ] struct `suprnova_live::render_cache::VarianceDescriptor` · crates/suprnova-live/src/render_cache/variance.rs:221 (also `suprnova_live::render_cache::variance::VarianceDescriptor`)
  - [ ] fn `suprnova_live::render_cache::VarianceDescriptor::new` · crates/suprnova-live/src/render_cache/variance.rs:228
  - [ ] fn `suprnova_live::render_cache::VarianceDescriptor::declare` · crates/suprnova-live/src/render_cache/variance.rs:233
  - [ ] fn `suprnova_live::render_cache::VarianceDescriptor::dimensions` · crates/suprnova-live/src/render_cache/variance.rs:265
  - [ ] fn `suprnova_live::render_cache::VarianceDescriptor::vary_headers` · crates/suprnova-live/src/render_cache/variance.rs:271
  - [ ] fn `suprnova_live::render_cache::VarianceDescriptor::canonical_len` · crates/suprnova-live/src/render_cache/variance.rs:284
  - [ ] fn `suprnova_live::render_cache::VarianceDescriptor::write_canonical` · crates/suprnova-live/src/render_cache/variance.rs:302
  - [ ] fn `suprnova_live::render_cache::VarianceDescriptor::canonical_bytes` · crates/suprnova-live/src/render_cache/variance.rs:327
- [ ] enum `suprnova_live::render_cache::AuthorizationConsult` · crates/suprnova-live/src/render_cache/variance.rs:340 (also `suprnova_live::render_cache::variance::AuthorizationConsult`)
  - Variants: `None`, `TenantOnly`, `Principal`
  - [ ] fn `suprnova_live::render_cache::AuthorizationConsult::join` · crates/suprnova-live/src/render_cache/variance.rs:357
- [ ] enum `suprnova_live::render_cache::ClassificationReason` · crates/suprnova-live/src/render_cache/variance.rs:385 (also `suprnova_live::render_cache::variance::ClassificationReason`)
  - Variants: `PrincipalObserved`, `TenantObserved`, `SessionValueRead`, `AuthorizationRead`, `AuthorizationTenantRead`, `SecretContextRead`, `UndeclaredContext`
- [ ] enum `suprnova_live::render_cache::DimensionValue` · crates/suprnova-live/src/render_cache/variance.rs:209 (also `suprnova_live::render_cache::variance::DimensionValue`)
  - Variants: `Public`, `Private`, `Anonymous`
- [ ] enum `suprnova_live::render_cache::VarianceDimension` · crates/suprnova-live/src/render_cache/variance.rs:20 (also `suprnova_live::render_cache::variance::VarianceDimension`)
  - Variants: `Host`, `Locale`, `Media`, `Encoding`, `Tenant`, `Principal`, `FeatureVersion`, `ConfigVersion`, `Application`
  - [ ] fn `suprnova_live::render_cache::VarianceDimension::vary_header` · crates/suprnova-live/src/render_cache/variance.rs:44
- [ ] const `suprnova_live::render_cache::variance::MAX_DIMENSION_VALUE_BYTES` · crates/suprnova-live/src/render_cache/variance.rs:14
- [ ] const `suprnova_live::render_cache::variance::MAX_DIMENSIONS` · crates/suprnova-live/src/render_cache/variance.rs:16

## resource

### `suprnova_live::resource::bounds` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::resource::ResourceBounds` · crates/suprnova-live/src/resource/bounds.rs:15
  - [ ] fn `suprnova_live::resource::ResourceBounds::new` · crates/suprnova-live/src/resource/bounds.rs:22
  - [ ] fn `suprnova_live::resource::ResourceBounds::max_items` · crates/suprnova-live/src/resource/bounds.rs:39
  - [ ] fn `suprnova_live::resource::ResourceBounds::max_bytes` · crates/suprnova-live/src/resource/bounds.rs:45
- [ ] struct `suprnova_live::resource::ResourceBoundsError` · crates/suprnova-live/src/resource/bounds.rs:52
- [ ] const `suprnova_live::resource::HARD_MAX_ACTIVE_PERMITS` · crates/suprnova-live/src/resource/bounds.rs:11
- [ ] const `suprnova_live::resource::HARD_MAX_RESOURCE_BYTES` · crates/suprnova-live/src/resource/bounds.rs:9
- [ ] const `suprnova_live::resource::HARD_MAX_RESOURCE_ITEMS` · crates/suprnova-live/src/resource/bounds.rs:7

### `suprnova_live::resource::cancel` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::resource::CancellationFlag` · crates/suprnova-live/src/resource/cancel.rs:12
  - [ ] fn `suprnova_live::resource::CancellationFlag::new` · crates/suprnova-live/src/resource/cancel.rs:19
  - [ ] fn `suprnova_live::resource::CancellationFlag::cancel` · crates/suprnova-live/src/resource/cancel.rs:24
  - [ ] fn `suprnova_live::resource::CancellationFlag::is_canceled` · crates/suprnova-live/src/resource/cancel.rs:30

### `suprnova_live::resource::owner` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::resource::Permit` · crates/suprnova-live/src/resource/owner.rs:82
  - [ ] fn `suprnova_live::resource::Permit::release` · crates/suprnova-live/src/resource/owner.rs:89
- [ ] struct `suprnova_live::resource::PermitPool` · crates/suprnova-live/src/resource/owner.rs:22
  - [ ] fn `suprnova_live::resource::PermitPool::new` · crates/suprnova-live/src/resource/owner.rs:28
  - [ ] fn `suprnova_live::resource::PermitPool::try_acquire` · crates/suprnova-live/src/resource/owner.rs:39
  - [ ] fn `suprnova_live::resource::PermitPool::max_active` · crates/suprnova-live/src/resource/owner.rs:64
  - [ ] fn `suprnova_live::resource::PermitPool::active` · crates/suprnova-live/src/resource/owner.rs:70
  - [ ] fn `suprnova_live::resource::PermitPool::available` · crates/suprnova-live/src/resource/owner.rs:76
- [ ] struct `suprnova_live::resource::ResourceOwner` · crates/suprnova-live/src/resource/owner.rs:379
  - [ ] fn `suprnova_live::resource::ResourceOwner::new` · crates/suprnova-live/src/resource/owner.rs:387
  - [ ] fn `suprnova_live::resource::ResourceOwner::queue` · crates/suprnova-live/src/resource/owner.rs:396
  - [ ] fn `suprnova_live::resource::ResourceOwner::cancellation` · crates/suprnova-live/src/resource/owner.rs:402
  - [ ] fn `suprnova_live::resource::ResourceOwner::retire` · crates/suprnova-live/src/resource/owner.rs:411
- [ ] struct `suprnova_live::resource::ResourceQueue` · crates/suprnova-live/src/resource/owner.rs:130
  - [ ] fn `suprnova_live::resource::ResourceQueue::try_push` · crates/suprnova-live/src/resource/owner.rs:150
  - [ ] fn `suprnova_live::resource::ResourceQueue::try_push_batch` · crates/suprnova-live/src/resource/owner.rs:169
  - [ ] fn `suprnova_live::resource::ResourceQueue::try_replace_back` · crates/suprnova-live/src/resource/owner.rs:279
  - [ ] fn `suprnova_live::resource::ResourceQueue::pop` · crates/suprnova-live/src/resource/owner.rs:298
  - [ ] fn `suprnova_live::resource::ResourceQueue::len` · crates/suprnova-live/src/resource/owner.rs:322
  - [ ] fn `suprnova_live::resource::ResourceQueue::is_empty` · crates/suprnova-live/src/resource/owner.rs:328
  - [ ] fn `suprnova_live::resource::ResourceQueue::retained_bytes` · crates/suprnova-live/src/resource/owner.rs:334
  - [ ] fn `suprnova_live::resource::ResourceQueue::bounds` · crates/suprnova-live/src/resource/owner.rs:340
  - [ ] fn `suprnova_live::resource::ResourceQueue::is_retired` · crates/suprnova-live/src/resource/owner.rs:346

### `suprnova_live::resource::queue` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::resource::BoundedQueue` · crates/suprnova-live/src/resource/queue.rs:149
  - [ ] fn `suprnova_live::resource::BoundedQueue::new` · crates/suprnova-live/src/resource/queue.rs:171
  - [ ] fn `suprnova_live::resource::BoundedQueue::try_push` · crates/suprnova-live/src/resource/queue.rs:184
  - [ ] fn `suprnova_live::resource::BoundedQueue::try_replace_back` · crates/suprnova-live/src/resource/queue.rs:194
  - [ ] fn `suprnova_live::resource::BoundedQueue::pop` · crates/suprnova-live/src/resource/queue.rs:404
  - [ ] fn `suprnova_live::resource::BoundedQueue::bounds` · crates/suprnova-live/src/resource/queue.rs:415
  - [ ] fn `suprnova_live::resource::BoundedQueue::len` · crates/suprnova-live/src/resource/queue.rs:421
  - [ ] fn `suprnova_live::resource::BoundedQueue::is_empty` · crates/suprnova-live/src/resource/queue.rs:427
  - [ ] fn `suprnova_live::resource::BoundedQueue::retained_bytes` · crates/suprnova-live/src/resource/queue.rs:433
  - [ ] fn `suprnova_live::resource::BoundedQueue::is_retired` · crates/suprnova-live/src/resource/queue.rs:439
- [ ] struct `suprnova_live::resource::Retirement` · crates/suprnova-live/src/resource/queue.rs:465
  - Public fields: `canceled`, `drained_items`, `drained_bytes`
  - [ ] fn `suprnova_live::resource::Retirement::already_retired` · crates/suprnova-live/src/resource/queue.rs:477
- [ ] enum `suprnova_live::resource::ResourceDiagnostic` · crates/suprnova-live/src/resource/queue.rs:11
  - Variants: `Accepted`, `Released`, `ItemsExceeded`, `BytesExceeded`, `PermitsExceeded`, `Canceled`, `Retired`
  - [ ] const `suprnova_live::resource::ResourceDiagnostic::ALL` · crates/suprnova-live/src/resource/queue.rs:30
  - [ ] fn `suprnova_live::resource::ResourceDiagnostic::as_str` · crates/suprnova-live/src/resource/queue.rs:42
- [ ] enum `suprnova_live::resource::ResourceError` · crates/suprnova-live/src/resource/queue.rs:57
  - Variants: `ItemsExceeded`, `BytesExceeded`, `PermitsExceeded`, `Retired`
  - [ ] fn `suprnova_live::resource::ResourceError::diagnostic` · crates/suprnova-live/src/resource/queue.rs:71
  - [ ] fn `suprnova_live::resource::ResourceError::as_str` · crates/suprnova-live/src/resource/queue.rs:82

## snapshot

### `suprnova_live::snapshot::codec` (private module; items are public through re-exports)

- [ ] fn `suprnova_live::snapshot::verify_instance` · crates/suprnova-live/src/snapshot/codec.rs:199
- [ ] fn `suprnova_live::snapshot::verify_seed` · crates/suprnova-live/src/snapshot/codec.rs:170

### `suprnova_live::snapshot::composition` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::snapshot::CompositionChildLineageV1` · crates/suprnova-live/src/snapshot/composition.rs:213
  - [ ] fn `suprnova_live::snapshot::CompositionChildLineageV1::new` · crates/suprnova-live/src/snapshot/composition.rs:217
  - [ ] fn `suprnova_live::snapshot::CompositionChildLineageV1::parent_instance` · crates/suprnova-live/src/snapshot/composition.rs:238
  - [ ] fn `suprnova_live::snapshot::CompositionChildLineageV1::parent_revision` · crates/suprnova-live/src/snapshot/composition.rs:244
  - [ ] fn `suprnova_live::snapshot::CompositionChildLineageV1::child_key` · crates/suprnova-live/src/snapshot/composition.rs:250
  - [ ] fn `suprnova_live::snapshot::CompositionChildLineageV1::child_contract` · crates/suprnova-live/src/snapshot/composition.rs:256
  - [ ] fn `suprnova_live::snapshot::CompositionChildLineageV1::child_instance` · crates/suprnova-live/src/snapshot/composition.rs:262
  - [ ] fn `suprnova_live::snapshot::CompositionChildLineageV1::depth` · crates/suprnova-live/src/snapshot/composition.rs:268
- [ ] struct `suprnova_live::snapshot::CompositionLineageV1` · crates/suprnova-live/src/snapshot/composition.rs:275
  - [ ] fn `suprnova_live::snapshot::CompositionLineageV1::new` · crates/suprnova-live/src/snapshot/composition.rs:282
  - [ ] fn `suprnova_live::snapshot::CompositionLineageV1::owner` · crates/suprnova-live/src/snapshot/composition.rs:322
  - [ ] fn `suprnova_live::snapshot::CompositionLineageV1::children` · crates/suprnova-live/src/snapshot/composition.rs:328
- [ ] struct `suprnova_live::snapshot::CompositionOwnerLineageV1` · crates/suprnova-live/src/snapshot/composition.rs:151
  - [ ] fn `suprnova_live::snapshot::CompositionOwnerLineageV1::new` · crates/suprnova-live/src/snapshot/composition.rs:155
  - [ ] fn `suprnova_live::snapshot::CompositionOwnerLineageV1::parent_instance` · crates/suprnova-live/src/snapshot/composition.rs:176
  - [ ] fn `suprnova_live::snapshot::CompositionOwnerLineageV1::parent_revision` · crates/suprnova-live/src/snapshot/composition.rs:182
  - [ ] fn `suprnova_live::snapshot::CompositionOwnerLineageV1::child_key` · crates/suprnova-live/src/snapshot/composition.rs:188
  - [ ] fn `suprnova_live::snapshot::CompositionOwnerLineageV1::child_contract` · crates/suprnova-live/src/snapshot/composition.rs:194
  - [ ] fn `suprnova_live::snapshot::CompositionOwnerLineageV1::child_instance` · crates/suprnova-live/src/snapshot/composition.rs:200
  - [ ] fn `suprnova_live::snapshot::CompositionOwnerLineageV1::depth` · crates/suprnova-live/src/snapshot/composition.rs:206
- [ ] const `suprnova_live::snapshot::COMPOSITION_LINEAGE_EXTENSION_V1` · crates/suprnova-live/src/snapshot/composition.rs:30
- [ ] const `suprnova_live::snapshot::MAX_COMPOSITION_LINEAGE_BYTES_V1` · crates/suprnova-live/src/snapshot/composition.rs:36
- [ ] const `suprnova_live::snapshot::MAX_COMPOSITION_LINEAGE_CHILDREN_V1` · crates/suprnova-live/src/snapshot/composition.rs:32
- [ ] const `suprnova_live::snapshot::MAX_COMPOSITION_LINEAGE_DEPTH_V1` · crates/suprnova-live/src/snapshot/composition.rs:34

### `suprnova_live::snapshot::error` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::snapshot::SnapshotError` · crates/suprnova-live/src/snapshot/error.rs:94
  - [ ] fn `suprnova_live::snapshot::SnapshotError::kind` · crates/suprnova-live/src/snapshot/error.rs:105
- [ ] enum `suprnova_live::snapshot::SnapshotErrorKind` · crates/suprnova-live/src/snapshot/error.rs:8
  - Variants: `InputTooLarge`, `InputTooDeep`, `TooManyEntries`, `DuplicateField`, `InvalidEnvelope`, `WrongForm`, `UnsupportedSchema`, `SignatureInvalid`, `SigningKeyMismatch`, `BindingMismatch`, `CompatibilityMismatch`, `IssuedInFuture`, `Expired`, `ValidityTooLong`, `InvalidStateShape`, `UnknownStateField`, `MissingStateField`, `ForbiddenStateField`, `InvalidStateCodec`, `InvalidSchema`, `TooManyGenerations`, `InvalidExtension`, `DehydrationFailed`, `HydrationFailed`
  - [ ] fn `suprnova_live::snapshot::SnapshotErrorKind::as_str` · crates/suprnova-live/src/snapshot/error.rs:62

### `suprnova_live::snapshot::limits` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::snapshot::SnapshotLimits` · crates/suprnova-live/src/snapshot/limits.rs:8
  - [ ] fn `suprnova_live::snapshot::SnapshotLimits::new` · crates/suprnova-live/src/snapshot/limits.rs:19
  - [ ] fn `suprnova_live::snapshot::SnapshotLimits::input` · crates/suprnova-live/src/snapshot/limits.rs:48

### `suprnova_live::snapshot::schema` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::snapshot::ComponentContract` · crates/suprnova-live/src/snapshot/schema.rs:127
  - [ ] fn `suprnova_live::snapshot::ComponentContract::new` · crates/suprnova-live/src/snapshot/schema.rs:137
  - [ ] fn `suprnova_live::snapshot::ComponentContract::name` · crates/suprnova-live/src/snapshot/schema.rs:158
  - [ ] fn `suprnova_live::snapshot::ComponentContract::contract_digest` · crates/suprnova-live/src/snapshot/schema.rs:164
- [ ] struct `suprnova_live::snapshot::ExpectedInstanceV1` · crates/suprnova-live/src/snapshot/schema.rs:653
  - [ ] fn `suprnova_live::snapshot::ExpectedInstanceV1::new` · crates/suprnova-live/src/snapshot/schema.rs:665
- [ ] struct `suprnova_live::snapshot::ExpectedSeedV1` · crates/suprnova-live/src/snapshot/schema.rs:603
  - [ ] fn `suprnova_live::snapshot::ExpectedSeedV1::new` · crates/suprnova-live/src/snapshot/schema.rs:614
- [ ] struct `suprnova_live::snapshot::GenerationMemo` · crates/suprnova-live/src/snapshot/schema.rs:181
  - [ ] fn `suprnova_live::snapshot::GenerationMemo::new` · crates/suprnova-live/src/snapshot/schema.rs:189
- [ ] struct `suprnova_live::snapshot::InstanceBodyV1` · crates/suprnova-live/src/snapshot/schema.rs:451
  - [ ] fn `suprnova_live::snapshot::InstanceBodyV1::sign` · crates/suprnova-live/src/snapshot/codec.rs:133
  - [ ] fn `suprnova_live::snapshot::InstanceBodyV1::new` · crates/suprnova-live/src/snapshot/schema.rs:473
  - [ ] fn `suprnova_live::snapshot::InstanceBodyV1::instance_id` · crates/suprnova-live/src/snapshot/schema.rs:528
  - [ ] fn `suprnova_live::snapshot::InstanceBodyV1::revision` · crates/suprnova-live/src/snapshot/schema.rs:534
  - [ ] fn `suprnova_live::snapshot::InstanceBodyV1::expires_at` · crates/suprnova-live/src/snapshot/schema.rs:540
  - [ ] fn `suprnova_live::snapshot::InstanceBodyV1::composition_lineage` · crates/suprnova-live/src/snapshot/schema.rs:546
- [ ] struct `suprnova_live::snapshot::InstanceFieldsV1` · crates/suprnova-live/src/snapshot/schema.rs:395
  - Public fields: `component`, `build_id`, `route`, `slot`, `key_id`, `scope`, `instance_id`, `revision`, `issued_at`, `expires_at`, `state`, `memo`, `extensions`
  - [ ] fn `suprnova_live::snapshot::InstanceFieldsV1::set_composition_lineage` · crates/suprnova-live/src/snapshot/schema.rs:426
- [ ] struct `suprnova_live::snapshot::MountedDocumentPath` · crates/suprnova-live/src/snapshot/schema.rs:30
  - [ ] fn `suprnova_live::snapshot::MountedDocumentPath::parse` · crates/suprnova-live/src/snapshot/schema.rs:34
  - [ ] fn `suprnova_live::snapshot::MountedDocumentPath::as_str` · crates/suprnova-live/src/snapshot/schema.rs:52
- [ ] struct `suprnova_live::snapshot::SeedBodyV1` · crates/suprnova-live/src/snapshot/schema.rs:236
  - [ ] fn `suprnova_live::snapshot::SeedBodyV1::sign` · crates/suprnova-live/src/snapshot/codec.rs:117
  - [ ] fn `suprnova_live::snapshot::SeedBodyV1::new` · crates/suprnova-live/src/snapshot/schema.rs:256
  - [ ] fn `suprnova_live::snapshot::SeedBodyV1::component` · crates/suprnova-live/src/snapshot/schema.rs:318
  - [ ] fn `suprnova_live::snapshot::SeedBodyV1::route` · crates/suprnova-live/src/snapshot/schema.rs:324
  - [ ] fn `suprnova_live::snapshot::SeedBodyV1::slot` · crates/suprnova-live/src/snapshot/schema.rs:330
  - [ ] fn `suprnova_live::snapshot::SeedBodyV1::issued_at` · crates/suprnova-live/src/snapshot/schema.rs:336
  - [ ] fn `suprnova_live::snapshot::SeedBodyV1::max_age_ms` · crates/suprnova-live/src/snapshot/schema.rs:342
  - [ ] fn `suprnova_live::snapshot::SeedBodyV1::refresh_on_promote` · crates/suprnova-live/src/snapshot/schema.rs:348
  - [ ] fn `suprnova_live::snapshot::SeedBodyV1::advisory_generations` · crates/suprnova-live/src/snapshot/schema.rs:354
- [ ] struct `suprnova_live::snapshot::SeedFieldsV1` · crates/suprnova-live/src/snapshot/schema.rs:199
  - Public fields: `component`, `build_id`, `route`, `slot`, `key_id`, `issued_at`, `max_age_ms`, `mount`, `state`, `memo`, `advisory_generations`, `refresh_on_promote`, `extensions`
- [ ] enum `suprnova_live::snapshot::SnapshotForm` · crates/suprnova-live/src/snapshot/schema.rs:118
  - Variants: `Seed`, `Instance`
- [ ] const `suprnova_live::snapshot::SNAPSHOT_SCHEMA_V1` · crates/suprnova-live/src/snapshot/schema.rs:20

### `suprnova_live::snapshot::state`

- [ ] fn `suprnova_live::snapshot::state::decode_bytes` · crates/suprnova-live/src/snapshot/state.rs:405
- [ ] fn `suprnova_live::snapshot::state::decode_i64` · crates/suprnova-live/src/snapshot/state.rs:360
- [ ] fn `suprnova_live::snapshot::state::decode_u64` · crates/suprnova-live/src/snapshot/state.rs:382
- [ ] fn `suprnova_live::snapshot::state::dehydrate` · crates/suprnova-live/src/snapshot/state.rs:251
- [ ] fn `suprnova_live::snapshot::state::encode_bytes` · crates/suprnova-live/src/snapshot/state.rs:397
- [ ] fn `suprnova_live::snapshot::state::encode_i64` · crates/suprnova-live/src/snapshot/state.rs:355
- [ ] fn `suprnova_live::snapshot::state::encode_u64` · crates/suprnova-live/src/snapshot/state.rs:377
- [ ] struct `suprnova_live::snapshot::state::FieldSpec` · crates/suprnova-live/src/snapshot/state.rs:65
  - [ ] fn `suprnova_live::snapshot::state::FieldSpec::new` · crates/suprnova-live/src/snapshot/state.rs:74
- [ ] struct `suprnova_live::snapshot::SnapshotSchemaSet` · crates/suprnova-live/src/snapshot/state.rs:184 (also `suprnova_live::snapshot::state::SnapshotSchemaSet`)
  - [ ] fn `suprnova_live::snapshot::SnapshotSchemaSet::new` · crates/suprnova-live/src/snapshot/state.rs:192
  - [ ] fn `suprnova_live::snapshot::SnapshotSchemaSet::state` · crates/suprnova-live/src/snapshot/state.rs:202
  - [ ] fn `suprnova_live::snapshot::SnapshotSchemaSet::memo` · crates/suprnova-live/src/snapshot/state.rs:208
  - [ ] fn `suprnova_live::snapshot::SnapshotSchemaSet::mount` · crates/suprnova-live/src/snapshot/state.rs:214
- [ ] struct `suprnova_live::snapshot::state::StateSchema` · crates/suprnova-live/src/snapshot/state.rs:99
  - [ ] fn `suprnova_live::snapshot::state::StateSchema::new` · crates/suprnova-live/src/snapshot/state.rs:106
  - [ ] fn `suprnova_live::snapshot::state::StateSchema::version` · crates/suprnova-live/src/snapshot/state.rs:124
  - [ ] fn `suprnova_live::snapshot::state::StateSchema::validate` · crates/suprnova-live/src/snapshot/state.rs:129
- [ ] enum `suprnova_live::snapshot::state::FieldCategory` · crates/suprnova-live/src/snapshot/state.rs:20
  - Variants: `State`, `Public`, `Model`, `Locked`, `ServerOnly`, `Session`, `Computed`, `Transient`, `Secret`
- [ ] enum `suprnova_live::snapshot::state::StateCodec` · crates/suprnova-live/src/snapshot/state.rs:43
  - Variants: `Json`, `I64Decimal`, `U64Decimal`, `BytesBase64Url`
- [ ] enum `suprnova_live::snapshot::state::StateExposure` · crates/suprnova-live/src/snapshot/state.rs:56
  - Variants: `PublicSeed`, `Instanced`

### `suprnova_live::snapshot::verified` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::snapshot::VerifiedInstanceV1` · crates/suprnova-live/src/snapshot/verified.rs:63
  - [ ] fn `suprnova_live::snapshot::VerifiedInstanceV1::body` · crates/suprnova-live/src/snapshot/verified.rs:74
  - [ ] fn `suprnova_live::snapshot::VerifiedInstanceV1::mounted_document_path` · crates/suprnova-live/src/snapshot/verified.rs:79
  - [ ] fn `suprnova_live::snapshot::VerifiedInstanceV1::hydrate_state` · crates/suprnova-live/src/snapshot/verified.rs:84
  - [ ] fn `suprnova_live::snapshot::VerifiedInstanceV1::hydrate_memo` · crates/suprnova-live/src/snapshot/verified.rs:92
- [ ] struct `suprnova_live::snapshot::VerifiedSeedV1` · crates/suprnova-live/src/snapshot/verified.rs:11
  - [ ] fn `suprnova_live::snapshot::VerifiedSeedV1::body` · crates/suprnova-live/src/snapshot/verified.rs:22
  - [ ] fn `suprnova_live::snapshot::VerifiedSeedV1::mounted_document_path` · crates/suprnova-live/src/snapshot/verified.rs:27
  - [ ] fn `suprnova_live::snapshot::VerifiedSeedV1::hydrate_state` · crates/suprnova-live/src/snapshot/verified.rs:32
  - [ ] fn `suprnova_live::snapshot::VerifiedSeedV1::hydrate_memo` · crates/suprnova-live/src/snapshot/verified.rs:40
  - [ ] fn `suprnova_live::snapshot::VerifiedSeedV1::hydrate_mount` · crates/suprnova-live/src/snapshot/verified.rs:48

## state

### `suprnova_live::state::codec` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::state::BindingIssue` · crates/suprnova-live/src/state/codec.rs:58
  - [ ] fn `suprnova_live::state::BindingIssue::kind` · crates/suprnova-live/src/state/codec.rs:75
  - [ ] fn `suprnova_live::state::BindingIssue::path` · crates/suprnova-live/src/state/codec.rs:81
- [ ] enum `suprnova_live::state::BindingIssueKind` · crates/suprnova-live/src/state/codec.rs:23
  - Variants: `InvalidType`, `InvalidValue`, `InputTooLarge`, `InputTooDeep`, `TooManyEntries`, `StringTooLong`, `TypeMismatch`
  - [ ] fn `suprnova_live::state::BindingIssueKind::as_str` · crates/suprnova-live/src/state/codec.rs:43
- [ ] enum `suprnova_live::state::ModelCodec` · crates/suprnova-live/src/state/codec.rs:104
  - Variants: `String`, `Boolean`, `I64`, `U64`, `F64`, `Json`, `Date`, `DateTime`, `Uuid`, `Enumeration`, `List`, `Map`
  - [ ] fn `suprnova_live::state::ModelCodec::list` · crates/suprnova-live/src/state/codec.rs:134
  - [ ] fn `suprnova_live::state::ModelCodec::map` · crates/suprnova-live/src/state/codec.rs:140
  - [ ] fn `suprnova_live::state::ModelCodec::enumeration` · crates/suprnova-live/src/state/codec.rs:145
  - [ ] fn `suprnova_live::state::ModelCodec::decode` · crates/suprnova-live/src/state/codec.rs:170
  - [ ] fn `suprnova_live::state::ModelCodec::encode` · crates/suprnova-live/src/state/codec.rs:191

### `suprnova_live::state::path` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::state::ModelPath` · crates/suprnova-live/src/state/path.rs:70
  - [ ] fn `suprnova_live::state::ModelPath::parse` · crates/suprnova-live/src/state/path.rs:77
  - [ ] fn `suprnova_live::state::ModelPath::as_str` · crates/suprnova-live/src/state/path.rs:110
- [ ] struct `suprnova_live::state::PathError` · crates/suprnova-live/src/state/path.rs:38
  - [ ] fn `suprnova_live::state::PathError::kind` · crates/suprnova-live/src/state/path.rs:49
- [ ] enum `suprnova_live::state::PathErrorKind` · crates/suprnova-live/src/state/path.rs:12
  - Variants: `Malformed`, `TooLong`, `TooDeep`, `UnstableCollectionPath`
  - [ ] fn `suprnova_live::state::PathErrorKind::as_str` · crates/suprnova-live/src/state/path.rs:26

### `suprnova_live::state::proposal` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::state::ModelBindingSchema` · crates/suprnova-live/src/state/proposal.rs:254
  - [ ] fn `suprnova_live::state::ModelBindingSchema::new` · crates/suprnova-live/src/state/proposal.rs:260
- [ ] struct `suprnova_live::state::ModelFieldBinding` · crates/suprnova-live/src/state/proposal.rs:213
  - [ ] fn `suprnova_live::state::ModelFieldBinding::new` · crates/suprnova-live/src/state/proposal.rs:221
  - [ ] fn `suprnova_live::state::ModelFieldBinding::path` · crates/suprnova-live/src/state/proposal.rs:235
  - [ ] fn `suprnova_live::state::ModelFieldBinding::category` · crates/suprnova-live/src/state/proposal.rs:241
  - [ ] fn `suprnova_live::state::ModelFieldBinding::codec` · crates/suprnova-live/src/state/proposal.rs:247
- [ ] struct `suprnova_live::state::ProposalBatch` · crates/suprnova-live/src/state/proposal.rs:280
  - [ ] fn `suprnova_live::state::ProposalBatch::is_empty` · crates/suprnova-live/src/state/proposal.rs:290
  - [ ] fn `suprnova_live::state::ProposalBatch::prepare` · crates/suprnova-live/src/state/proposal.rs:295
  - [ ] fn `suprnova_live::state::ProposalBatch::issues` · crates/suprnova-live/src/state/proposal.rs:361
  - [ ] fn `suprnova_live::state::ProposalBatch::proposed` · crates/suprnova-live/src/state/proposal.rs:367
  - [ ] fn `suprnova_live::state::ProposalBatch::apply_required` · crates/suprnova-live/src/state/proposal.rs:386
  - [ ] fn `suprnova_live::state::ProposalBatch::apply_optional` · crates/suprnova-live/src/state/proposal.rs:410
- [ ] struct `suprnova_live::state::ProposalError` · crates/suprnova-live/src/state/proposal.rs:112
  - [ ] fn `suprnova_live::state::ProposalError::kind` · crates/suprnova-live/src/state/proposal.rs:123
- [ ] struct `suprnova_live::state::ProposalLimitError` · crates/suprnova-live/src/state/proposal.rs:138
- [ ] struct `suprnova_live::state::ProposalLimits` · crates/suprnova-live/src/state/proposal.rs:150
  - [ ] fn `suprnova_live::state::ProposalLimits::new` · crates/suprnova-live/src/state/proposal.rs:158
  - [ ] fn `suprnova_live::state::ProposalLimits::input_limits` · crates/suprnova-live/src/state/proposal.rs:179
- [ ] struct `suprnova_live::state::RawModelProposal` · crates/suprnova-live/src/state/proposal.rs:195
  - [ ] fn `suprnova_live::state::RawModelProposal::new` · crates/suprnova-live/src/state/proposal.rs:203
- [ ] enum `suprnova_live::state::ProposalApplication` · crates/suprnova-live/src/state/proposal.rs:33
  - Variants: `Missing`, `Null`, `Invalid`, `Applied`
  - [ ] fn `suprnova_live::state::ProposalApplication::into_issue` · crates/suprnova-live/src/state/proposal.rs:49
- [ ] enum `suprnova_live::state::ProposalErrorKind` · crates/suprnova-live/src/state/proposal.rs:59
  - Variants: `InvalidSchema`, `TooManyProposals`, `TooManyIssues`, `MalformedPath`, `UnstableCollectionPath`, `UnknownField`, `ForbiddenField`, `DuplicatePath`, `ConflictingPaths`, `InputTooLarge`, `InputTooDeep`, `TooManyEntries`, `StringTooLong`
  - [ ] fn `suprnova_live::state::ProposalErrorKind::as_str` · crates/suprnova-live/src/state/proposal.rs:91
- [ ] enum `suprnova_live::state::ProposedValue` · crates/suprnova-live/src/state/proposal.rs:20
  - Variants: `Missing`, `Null`, `Invalid`, `Valid`

### `suprnova_live::state::session` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::state::SessionError` · crates/suprnova-live/src/state/session.rs:48
  - [ ] fn `suprnova_live::state::SessionError::host_failure` · crates/suprnova-live/src/state/session.rs:59
  - [ ] fn `suprnova_live::state::SessionError::kind` · crates/suprnova-live/src/state/session.rs:65
- [ ] struct `suprnova_live::state::SessionField` · crates/suprnova-live/src/state/session.rs:80
  - [ ] fn `suprnova_live::state::SessionField::from_metadata` · crates/suprnova-live/src/state/session.rs:87
  - [ ] fn `suprnova_live::state::SessionField::name` · crates/suprnova-live/src/state/session.rs:103
  - [ ] fn `suprnova_live::state::SessionField::codec` · crates/suprnova-live/src/state/session.rs:109
- [ ] struct `suprnova_live::state::SessionIntent` · crates/suprnova-live/src/state/session.rs:168
  - [ ] fn `suprnova_live::state::SessionIntent::set` · crates/suprnova-live/src/state/session.rs:176
  - [ ] fn `suprnova_live::state::SessionIntent::remove` · crates/suprnova-live/src/state/session.rs:194
  - [ ] fn `suprnova_live::state::SessionIntent::field` · crates/suprnova-live/src/state/session.rs:204
  - [ ] fn `suprnova_live::state::SessionIntent::kind` · crates/suprnova-live/src/state/session.rs:210
  - [ ] fn `suprnova_live::state::SessionIntent::value` · crates/suprnova-live/src/state/session.rs:216
- [ ] struct `suprnova_live::state::SessionIntents` · crates/suprnova-live/src/state/session.rs:234
  - [ ] fn `suprnova_live::state::SessionIntents::new` · crates/suprnova-live/src/state/session.rs:241
  - [ ] fn `suprnova_live::state::SessionIntents::push` · crates/suprnova-live/src/state/session.rs:252
  - [ ] fn `suprnova_live::state::SessionIntents::as_slice` · crates/suprnova-live/src/state/session.rs:262
- [ ] struct `suprnova_live::state::SessionValue` · crates/suprnova-live/src/state/session.rs:116
  - [ ] fn `suprnova_live::state::SessionValue::from_canonical` · crates/suprnova-live/src/state/session.rs:120
  - [ ] fn `suprnova_live::state::SessionValue::decode` · crates/suprnova-live/src/state/session.rs:133
  - [ ] fn `suprnova_live::state::SessionValue::canonical` · crates/suprnova-live/src/state/session.rs:146
- [ ] enum `suprnova_live::state::SessionErrorKind` · crates/suprnova-live/src/state/session.rs:22
  - Variants: `InvalidField`, `InvalidValue`, `CapacityExceeded`, `HostFailure`
  - [ ] fn `suprnova_live::state::SessionErrorKind::as_str` · crates/suprnova-live/src/state/session.rs:36
- [ ] enum `suprnova_live::state::SessionIntentKind` · crates/suprnova-live/src/state/session.rs:159
  - Variants: `Set`, `Remove`
- [ ] trait `suprnova_live::state::SessionPort` · crates/suprnova-live/src/state/session.rs:269
  - [ ] fn `suprnova_live::state::SessionPort::read` · crates/suprnova-live/src/state/session.rs:271 (required)
  - [ ] fn `suprnova_live::state::SessionPort::apply` · crates/suprnova-live/src/state/session.rs:274 (required)

### `suprnova_live::state::timing` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::state::TimingError` · crates/suprnova-live/src/state/timing.rs:77
  - [ ] fn `suprnova_live::state::TimingError::kind` · crates/suprnova-live/src/state/timing.rs:84
- [ ] enum `suprnova_live::state::BindingTiming` · crates/suprnova-live/src/state/timing.rs:14
  - Variants: `Immediate`, `Change`, `Blur`, `Submit`, `Debounce`
  - [ ] fn `suprnova_live::state::BindingTiming::debounce` · crates/suprnova-live/src/state/timing.rs:31
  - [ ] fn `suprnova_live::state::BindingTiming::debounce_millis` · crates/suprnova-live/src/state/timing.rs:42
- [ ] enum `suprnova_live::state::TimingErrorKind` · crates/suprnova-live/src/state/timing.rs:70
  - Variants: `InvalidDebounce`
- [ ] const `suprnova_live::state::DEBOUNCE_MILLIS` · crates/suprnova-live/src/state/timing.rs:10

### `suprnova_live::state::url` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::state::UrlBinding` · crates/suprnova-live/src/state/url.rs:89
  - [ ] fn `suprnova_live::state::UrlBinding::new` · crates/suprnova-live/src/state/url.rs:99
  - [ ] fn `suprnova_live::state::UrlBinding::query_key` · crates/suprnova-live/src/state/url.rs:137
  - [ ] fn `suprnova_live::state::UrlBinding::category` · crates/suprnova-live/src/state/url.rs:143
  - [ ] fn `suprnova_live::state::UrlBinding::codec` · crates/suprnova-live/src/state/url.rs:149
  - [ ] fn `suprnova_live::state::UrlBinding::mode` · crates/suprnova-live/src/state/url.rs:155
  - [ ] fn `suprnova_live::state::UrlBinding::omit_default` · crates/suprnova-live/src/state/url.rs:161
  - [ ] fn `suprnova_live::state::UrlBinding::encode` · crates/suprnova-live/src/state/url.rs:166
  - [ ] fn `suprnova_live::state::UrlBinding::decode` · crates/suprnova-live/src/state/url.rs:189
  - [ ] fn `suprnova_live::state::UrlBinding::encode_if_changed` · crates/suprnova-live/src/state/url.rs:211
- [ ] struct `suprnova_live::state::UrlBindingSet` · crates/suprnova-live/src/state/url.rs:226
  - [ ] fn `suprnova_live::state::UrlBindingSet::new` · crates/suprnova-live/src/state/url.rs:232
  - [ ] fn `suprnova_live::state::UrlBindingSet::get` · crates/suprnova-live/src/state/url.rs:248
- [ ] struct `suprnova_live::state::UrlError` · crates/suprnova-live/src/state/url.rs:63
  - [ ] fn `suprnova_live::state::UrlError::kind` · crates/suprnova-live/src/state/url.rs:74
- [ ] enum `suprnova_live::state::UrlBindingMode` · crates/suprnova-live/src/state/url.rs:19
  - Variants: `Reflect`, `Navigate`
- [ ] enum `suprnova_live::state::UrlErrorKind` · crates/suprnova-live/src/state/url.rs:28
  - Variants: `InvalidQueryKey`, `ForbiddenCategory`, `UnsupportedCodec`, `DuplicateQueryKey`, `DuplicateField`, `InvalidValue`, `InputTooLarge`
  - [ ] fn `suprnova_live::state::UrlErrorKind::as_str` · crates/suprnova-live/src/state/url.rs:48

## telemetry

### `suprnova_live::telemetry`

- [ ] struct `suprnova_live::telemetry::TelemetryLabels` · crates/suprnova-live/src/telemetry.rs:93
  - [ ] fn `suprnova_live::telemetry::TelemetryLabels::new` · crates/suprnova-live/src/telemetry.rs:105
  - [ ] fn `suprnova_live::telemetry::TelemetryLabels::to_pairs` · crates/suprnova-live/src/telemetry.rs:125
- [ ] enum `suprnova_live::telemetry::TelemetryEvent` · crates/suprnova-live/src/telemetry.rs:10
  - Variants: `CanonicalParsing`, `SnapshotVerification`, `SeedPromotion`, `LedgerClaim`, `RequestProtocol`, `ResponseProtocol`, `ResponseOrdering`
  - [ ] const `suprnova_live::telemetry::TelemetryEvent::ALL` · crates/suprnova-live/src/telemetry.rs:29
  - [ ] fn `suprnova_live::telemetry::TelemetryEvent::as_str` · crates/suprnova-live/src/telemetry.rs:41
- [ ] enum `suprnova_live::telemetry::TelemetryOutcome` · crates/suprnova-live/src/telemetry.rs:56
  - Variants: `Accepted`, `Duplicate`, `Rejected`, `RefreshRequired`, `Failed`
  - [ ] const `suprnova_live::telemetry::TelemetryOutcome::ALL` · crates/suprnova-live/src/telemetry.rs:71
  - [ ] fn `suprnova_live::telemetry::TelemetryOutcome::as_str` · crates/suprnova-live/src/telemetry.rs:81

## upload

### `suprnova_live::upload::cleanup` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::upload::BoundedBackoff` · crates/suprnova-live/src/upload/cleanup.rs:66
  - [ ] fn `suprnova_live::upload::BoundedBackoff::new` · crates/suprnova-live/src/upload/cleanup.rs:74
  - [ ] fn `suprnova_live::upload::BoundedBackoff::initial` · crates/suprnova-live/src/upload/cleanup.rs:97
  - [ ] fn `suprnova_live::upload::BoundedBackoff::maximum` · crates/suprnova-live/src/upload/cleanup.rs:103
  - [ ] fn `suprnova_live::upload::BoundedBackoff::orphan_after` · crates/suprnova-live/src/upload/cleanup.rs:109
- [ ] struct `suprnova_live::upload::CleanupBatchRequest` · crates/suprnova-live/src/upload/cleanup.rs:178
  - [ ] fn `suprnova_live::upload::CleanupBatchRequest::lease_id` · crates/suprnova-live/src/upload/cleanup.rs:206
  - [ ] fn `suprnova_live::upload::CleanupBatchRequest::now` · crates/suprnova-live/src/upload/cleanup.rs:212
  - [ ] fn `suprnova_live::upload::CleanupBatchRequest::lease_expires_at` · crates/suprnova-live/src/upload/cleanup.rs:218
  - [ ] fn `suprnova_live::upload::CleanupBatchRequest::max_items` · crates/suprnova-live/src/upload/cleanup.rs:224
  - [ ] fn `suprnova_live::upload::CleanupBatchRequest::max_bytes` · crates/suprnova-live/src/upload/cleanup.rs:230
- [ ] struct `suprnova_live::upload::CleanupClaim` · crates/suprnova-live/src/upload/cleanup.rs:237
  - [ ] fn `suprnova_live::upload::CleanupClaim::from_store` · crates/suprnova-live/src/upload/cleanup.rs:250
  - [ ] fn `suprnova_live::upload::CleanupClaim::handle` · crates/suprnova-live/src/upload/cleanup.rs:281
  - [ ] fn `suprnova_live::upload::CleanupClaim::revision` · crates/suprnova-live/src/upload/cleanup.rs:287
  - [ ] fn `suprnova_live::upload::CleanupClaim::created_at` · crates/suprnova-live/src/upload/cleanup.rs:293
  - [ ] fn `suprnova_live::upload::CleanupClaim::retained_bytes` · crates/suprnova-live/src/upload/cleanup.rs:299
  - [ ] fn `suprnova_live::upload::CleanupClaim::lease_id` · crates/suprnova-live/src/upload/cleanup.rs:305
  - [ ] fn `suprnova_live::upload::CleanupClaim::lease_expires_at` · crates/suprnova-live/src/upload/cleanup.rs:311
  - [ ] fn `suprnova_live::upload::CleanupClaim::failed_attempts` · crates/suprnova-live/src/upload/cleanup.rs:317
  - [ ] fn `suprnova_live::upload::CleanupClaim::orphaned` · crates/suprnova-live/src/upload/cleanup.rs:323
- [ ] struct `suprnova_live::upload::CleanupCompletion` · crates/suprnova-live/src/upload/cleanup.rs:366
  - [ ] fn `suprnova_live::upload::CleanupCompletion::handle` · crates/suprnova-live/src/upload/cleanup.rs:387
  - [ ] fn `suprnova_live::upload::CleanupCompletion::revision` · crates/suprnova-live/src/upload/cleanup.rs:393
  - [ ] fn `suprnova_live::upload::CleanupCompletion::lease_id` · crates/suprnova-live/src/upload/cleanup.rs:399
  - [ ] fn `suprnova_live::upload::CleanupCompletion::completed_at` · crates/suprnova-live/src/upload/cleanup.rs:405
  - [ ] fn `suprnova_live::upload::CleanupCompletion::kind` · crates/suprnova-live/src/upload/cleanup.rs:411
- [ ] struct `suprnova_live::upload::CleanupLeaseId` · crates/suprnova-live/src/upload/cleanup.rs:32
  - [ ] fn `suprnova_live::upload::CleanupLeaseId::parse` · crates/suprnova-live/src/upload/cleanup.rs:36
  - [ ] fn `suprnova_live::upload::CleanupLeaseId::as_str` · crates/suprnova-live/src/upload/cleanup.rs:53
- [ ] struct `suprnova_live::upload::CleanupPolicy` · crates/suprnova-live/src/upload/cleanup.rs:124
  - [ ] fn `suprnova_live::upload::CleanupPolicy::new` · crates/suprnova-live/src/upload/cleanup.rs:133
  - [ ] fn `suprnova_live::upload::CleanupPolicy::batch_items` · crates/suprnova-live/src/upload/cleanup.rs:153
  - [ ] fn `suprnova_live::upload::CleanupPolicy::batch_bytes` · crates/suprnova-live/src/upload/cleanup.rs:159
  - [ ] fn `suprnova_live::upload::CleanupPolicy::lease` · crates/suprnova-live/src/upload/cleanup.rs:165
  - [ ] fn `suprnova_live::upload::CleanupPolicy::retry` · crates/suprnova-live/src/upload/cleanup.rs:171
- [ ] struct `suprnova_live::upload::CleanupRunOutcome` · crates/suprnova-live/src/upload/cleanup.rs:453
  - [ ] fn `suprnova_live::upload::CleanupRunOutcome::disposition` · crates/suprnova-live/src/upload/cleanup.rs:494
  - [ ] fn `suprnova_live::upload::CleanupRunOutcome::claimed` · crates/suprnova-live/src/upload/cleanup.rs:500
  - [ ] fn `suprnova_live::upload::CleanupRunOutcome::reclaimed` · crates/suprnova-live/src/upload/cleanup.rs:506
  - [ ] fn `suprnova_live::upload::CleanupRunOutcome::reclaimed_bytes` · crates/suprnova-live/src/upload/cleanup.rs:512
  - [ ] fn `suprnova_live::upload::CleanupRunOutcome::retry_scheduled` · crates/suprnova-live/src/upload/cleanup.rs:518
  - [ ] fn `suprnova_live::upload::CleanupRunOutcome::orphaned` · crates/suprnova-live/src/upload/cleanup.rs:524
  - [ ] fn `suprnova_live::upload::CleanupRunOutcome::deferred` · crates/suprnova-live/src/upload/cleanup.rs:530
- [ ] struct `suprnova_live::upload::UploadCleanupService` · crates/suprnova-live/src/upload/cleanup.rs:536
  - [ ] fn `suprnova_live::upload::UploadCleanupService::new` · crates/suprnova-live/src/upload/cleanup.rs:550
  - [ ] fn `suprnova_live::upload::UploadCleanupService::with_metrics` · crates/suprnova-live/src/upload/cleanup.rs:587
  - [ ] fn `suprnova_live::upload::UploadCleanupService::cancel` · crates/suprnova-live/src/upload/cleanup.rs:593
  - [ ] fn `suprnova_live::upload::UploadCleanupService::run_once` · crates/suprnova-live/src/upload/cleanup.rs:598
- [ ] enum `suprnova_live::upload::CleanupCompletionKind` · crates/suprnova-live/src/upload/cleanup.rs:347
  - Variants: `Reclaimed`, `Retry`, `Deferred`
- [ ] enum `suprnova_live::upload::CleanupDisposition` · crates/suprnova-live/src/upload/cleanup.rs:442
  - Variants: `Idle`, `Complete`, `Deferred`
- [ ] enum `suprnova_live::upload::CleanupLedgerDisposition` · crates/suprnova-live/src/upload/cleanup.rs:418
  - Variants: `Applied`, `Stale`
- [ ] trait `suprnova_live::upload::UploadCleanupLedger` · crates/suprnova-live/src/upload/cleanup.rs:426
  - [ ] fn `suprnova_live::upload::UploadCleanupLedger::claim_cleanup` · crates/suprnova-live/src/upload/cleanup.rs:428 (required)
  - [ ] fn `suprnova_live::upload::UploadCleanupLedger::complete_cleanup` · crates/suprnova-live/src/upload/cleanup.rs:434 (required)

### `suprnova_live::upload::direct_provider` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::upload::BoundedHeaders` · crates/suprnova-live/src/upload/direct_provider.rs:122
  - [ ] fn `suprnova_live::upload::BoundedHeaders::parse` · crates/suprnova-live/src/upload/direct_provider.rs:128
  - [ ] fn `suprnova_live::upload::BoundedHeaders::iter` · crates/suprnova-live/src/upload/direct_provider.rs:159
- [ ] struct `suprnova_live::upload::DirectPartReference` · crates/suprnova-live/src/upload/direct_provider.rs:217
  - [ ] fn `suprnova_live::upload::DirectPartReference::parse` · crates/suprnova-live/src/upload/direct_provider.rs:221
  - [ ] fn `suprnova_live::upload::DirectPartReference::as_str` · crates/suprnova-live/src/upload/direct_provider.rs:234
- [ ] struct `suprnova_live::upload::DirectTransferInstruction` · crates/suprnova-live/src/upload/direct_provider.rs:247
  - [ ] fn `suprnova_live::upload::DirectTransferInstruction::new` · crates/suprnova-live/src/upload/direct_provider.rs:263
  - [ ] fn `suprnova_live::upload::DirectTransferInstruction::method` · crates/suprnova-live/src/upload/direct_provider.rs:298
  - [ ] fn `suprnova_live::upload::DirectTransferInstruction::endpoint` · crates/suprnova-live/src/upload/direct_provider.rs:304
  - [ ] fn `suprnova_live::upload::DirectTransferInstruction::required_headers` · crates/suprnova-live/src/upload/direct_provider.rs:310
  - [ ] fn `suprnova_live::upload::DirectTransferInstruction::part` · crates/suprnova-live/src/upload/direct_provider.rs:316
  - [ ] fn `suprnova_live::upload::DirectTransferInstruction::reference` · crates/suprnova-live/src/upload/direct_provider.rs:322
  - [ ] fn `suprnova_live::upload::DirectTransferInstruction::expires_at` · crates/suprnova-live/src/upload/direct_provider.rs:328
  - [ ] fn `suprnova_live::upload::DirectTransferInstruction::maximum_bytes` · crates/suprnova-live/src/upload/direct_provider.rs:334
  - [ ] fn `suprnova_live::upload::DirectTransferInstruction::is_current` · crates/suprnova-live/src/upload/direct_provider.rs:340
  - [ ] fn `suprnova_live::upload::DirectTransferInstruction::is_constrained` · crates/suprnova-live/src/upload/direct_provider.rs:346
- [ ] struct `suprnova_live::upload::ReportDirectPart` · crates/suprnova-live/src/upload/direct_provider.rs:401
  - [ ] fn `suprnova_live::upload::ReportDirectPart::new` · crates/suprnova-live/src/upload/direct_provider.rs:411
  - [ ] fn `suprnova_live::upload::ReportDirectPart::handle` · crates/suprnova-live/src/upload/direct_provider.rs:427
  - [ ] fn `suprnova_live::upload::ReportDirectPart::part` · crates/suprnova-live/src/upload/direct_provider.rs:433
  - [ ] fn `suprnova_live::upload::ReportDirectPart::reference` · crates/suprnova-live/src/upload/direct_provider.rs:439
  - [ ] fn `suprnova_live::upload::ReportDirectPart::observed_at` · crates/suprnova-live/src/upload/direct_provider.rs:445
- [ ] struct `suprnova_live::upload::TrustedProviderOrigin` · crates/suprnova-live/src/upload/direct_provider.rs:22
  - [ ] fn `suprnova_live::upload::TrustedProviderOrigin::parse` · crates/suprnova-live/src/upload/direct_provider.rs:28
  - [ ] fn `suprnova_live::upload::TrustedProviderOrigin::as_str` · crates/suprnova-live/src/upload/direct_provider.rs:58
- [ ] struct `suprnova_live::upload::TrustedProviderUrl` · crates/suprnova-live/src/upload/direct_provider.rs:71
  - [ ] fn `suprnova_live::upload::TrustedProviderUrl::parse` · crates/suprnova-live/src/upload/direct_provider.rs:77
  - [ ] fn `suprnova_live::upload::TrustedProviderUrl::as_str` · crates/suprnova-live/src/upload/direct_provider.rs:92
- [ ] struct `suprnova_live::upload::UploadPart` · crates/suprnova-live/src/upload/direct_provider.rs:177
  - [ ] fn `suprnova_live::upload::UploadPart::new` · crates/suprnova-live/src/upload/direct_provider.rs:185
  - [ ] fn `suprnova_live::upload::UploadPart::index` · crates/suprnova-live/src/upload/direct_provider.rs:198
  - [ ] fn `suprnova_live::upload::UploadPart::offset` · crates/suprnova-live/src/upload/direct_provider.rs:204
  - [ ] fn `suprnova_live::upload::UploadPart::bytes` · crates/suprnova-live/src/upload/direct_provider.rs:210
- [ ] enum `suprnova_live::upload::TransferInstruction` · crates/suprnova-live/src/upload/direct_provider.rs:365
  - Variants: `ReverseProxy`, `Direct`
  - [ ] fn `suprnova_live::upload::TransferInstruction::is_constrained` · crates/suprnova-live/src/upload/direct_provider.rs:382
  - [ ] fn `suprnova_live::upload::TransferInstruction::as_direct` · crates/suprnova-live/src/upload/direct_provider.rs:391
- [ ] enum `suprnova_live::upload::TransferMethod` · crates/suprnova-live/src/upload/direct_provider.rs:105
  - Variants: `Put`
  - [ ] fn `suprnova_live::upload::TransferMethod::as_str` · crates/suprnova-live/src/upload/direct_provider.rs:113

### `suprnova_live::upload::finalize` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::upload::DurableUpload` · crates/suprnova-live/src/upload/finalize.rs:182
  - [ ] fn `suprnova_live::upload::DurableUpload::new` · crates/suprnova-live/src/upload/finalize.rs:193
  - [ ] fn `suprnova_live::upload::DurableUpload::id` · crates/suprnova-live/src/upload/finalize.rs:205
  - [ ] fn `suprnova_live::upload::DurableUpload::handle` · crates/suprnova-live/src/upload/finalize.rs:211
- [ ] struct `suprnova_live::upload::DurableUploadId` · crates/suprnova-live/src/upload/finalize.rs:46
  - [ ] fn `suprnova_live::upload::DurableUploadId::parse` · crates/suprnova-live/src/upload/finalize.rs:50
  - [ ] fn `suprnova_live::upload::DurableUploadId::as_str` · crates/suprnova-live/src/upload/finalize.rs:56
- [ ] struct `suprnova_live::upload::FailedFinalize` · crates/suprnova-live/src/upload/finalize.rs:240
  - [ ] fn `suprnova_live::upload::FailedFinalize::prepared` · crates/suprnova-live/src/upload/finalize.rs:252
  - [ ] fn `suprnova_live::upload::FailedFinalize::stage` · crates/suprnova-live/src/upload/finalize.rs:258
- [ ] struct `suprnova_live::upload::FinalizeRequest` · crates/suprnova-live/src/upload/finalize.rs:69
  - [ ] fn `suprnova_live::upload::FinalizeRequest::evidence` · crates/suprnova-live/src/upload/finalize.rs:93
  - [ ] fn `suprnova_live::upload::FinalizeRequest::action` · crates/suprnova-live/src/upload/finalize.rs:99
  - [ ] fn `suprnova_live::upload::FinalizeRequest::idempotency_key` · crates/suprnova-live/src/upload/finalize.rs:105
  - [ ] fn `suprnova_live::upload::FinalizeRequest::requested_at` · crates/suprnova-live/src/upload/finalize.rs:111
- [ ] struct `suprnova_live::upload::FinalizeToken` · crates/suprnova-live/src/upload/finalize.rs:23
  - [ ] fn `suprnova_live::upload::FinalizeToken::parse` · crates/suprnova-live/src/upload/finalize.rs:27
  - [ ] fn `suprnova_live::upload::FinalizeToken::as_str` · crates/suprnova-live/src/upload/finalize.rs:33
- [ ] struct `suprnova_live::upload::FinalizeUploadOutcome` · crates/suprnova-live/src/upload/finalize.rs:339
  - [ ] fn `suprnova_live::upload::FinalizeUploadOutcome::disposition` · crates/suprnova-live/src/upload/finalize.rs:348
  - [ ] fn `suprnova_live::upload::FinalizeUploadOutcome::durable` · crates/suprnova-live/src/upload/finalize.rs:354
  - [ ] fn `suprnova_live::upload::FinalizeUploadOutcome::revision` · crates/suprnova-live/src/upload/finalize.rs:360
- [ ] struct `suprnova_live::upload::FinalizeUploadRequest` · crates/suprnova-live/src/upload/finalize.rs:291
  - [ ] fn `suprnova_live::upload::FinalizeUploadRequest::new` · crates/suprnova-live/src/upload/finalize.rs:303
- [ ] struct `suprnova_live::upload::PreparedFinalize` · crates/suprnova-live/src/upload/finalize.rs:124
  - [ ] fn `suprnova_live::upload::PreparedFinalize::new` · crates/suprnova-live/src/upload/finalize.rs:136
  - [ ] fn `suprnova_live::upload::PreparedFinalize::token` · crates/suprnova-live/src/upload/finalize.rs:149
  - [ ] fn `suprnova_live::upload::PreparedFinalize::handle` · crates/suprnova-live/src/upload/finalize.rs:155
  - [ ] fn `suprnova_live::upload::PreparedFinalize::ready_revision` · crates/suprnova-live/src/upload/finalize.rs:161
- [ ] struct `suprnova_live::upload::ReadyUploadProposal` · crates/suprnova-live/src/upload/finalize.rs:595
  - [ ] fn `suprnova_live::upload::ReadyUploadProposal::handle` · crates/suprnova-live/src/upload/finalize.rs:602
  - [ ] fn `suprnova_live::upload::ReadyUploadProposal::ready_revision` · crates/suprnova-live/src/upload/finalize.rs:608
- [ ] struct `suprnova_live::upload::UploadFinalizationService` · crates/suprnova-live/src/upload/finalize.rs:366
  - [ ] fn `suprnova_live::upload::UploadFinalizationService::new` · crates/suprnova-live/src/upload/finalize.rs:375
  - [ ] fn `suprnova_live::upload::UploadFinalizationService::authorize_ready_proposal` · crates/suprnova-live/src/upload/finalize.rs:389
  - [ ] fn `suprnova_live::upload::UploadFinalizationService::finalize` · crates/suprnova-live/src/upload/finalize.rs:444
- [ ] enum `suprnova_live::upload::FinalizeDisposition` · crates/suprnova-live/src/upload/finalize.rs:330
  - Variants: `Finalized`, `ExistingOutcome`
- [ ] enum `suprnova_live::upload::FinalizeFailureStage` · crates/suprnova-live/src/upload/finalize.rs:231
  - Variants: `Commit`, `InvalidPreparation`
- [ ] trait `suprnova_live::upload::UploadFinalizer` · crates/suprnova-live/src/upload/finalize.rs:264
  - [ ] fn `suprnova_live::upload::UploadFinalizer::prepare` · crates/suprnova-live/src/upload/finalize.rs:266 (required)
  - [ ] fn `suprnova_live::upload::UploadFinalizer::commit` · crates/suprnova-live/src/upload/finalize.rs:272 (required)
  - [ ] fn `suprnova_live::upload::UploadFinalizer::compensate` · crates/suprnova-live/src/upload/finalize.rs:278 (required)
  - [ ] fn `suprnova_live::upload::UploadFinalizer::reconcile` · crates/suprnova-live/src/upload/finalize.rs:284 (required)

### `suprnova_live::upload::identity` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::upload::IssuedTransferGrant` · crates/suprnova-live/src/upload/identity.rs:376
  - [ ] fn `suprnova_live::upload::IssuedTransferGrant::handle` · crates/suprnova-live/src/upload/identity.rs:384
  - [ ] fn `suprnova_live::upload::IssuedTransferGrant::grant` · crates/suprnova-live/src/upload/identity.rs:390
- [ ] struct `suprnova_live::upload::TransferGrant` · crates/suprnova-live/src/upload/identity.rs:260
  - [ ] fn `suprnova_live::upload::TransferGrant::parse` · crates/suprnova-live/src/upload/identity.rs:264
  - [ ] fn `suprnova_live::upload::TransferGrant::expose_bearer` · crates/suprnova-live/src/upload/identity.rs:274
- [ ] struct `suprnova_live::upload::TransferGrantCodec` · crates/suprnova-live/src/upload/identity.rs:453
  - [ ] fn `suprnova_live::upload::TransferGrantCodec::new` · crates/suprnova-live/src/upload/identity.rs:460
  - [ ] fn `suprnova_live::upload::TransferGrantCodec::issue` · crates/suprnova-live/src/upload/identity.rs:465
  - [ ] fn `suprnova_live::upload::TransferGrantCodec::verify` · crates/suprnova-live/src/upload/identity.rs:498
- [ ] struct `suprnova_live::upload::TransferGrantRequest` · crates/suprnova-live/src/upload/identity.rs:353
  - [ ] fn `suprnova_live::upload::TransferGrantRequest::new` · crates/suprnova-live/src/upload/identity.rs:361
- [ ] struct `suprnova_live::upload::TransferGrantScope` · crates/suprnova-live/src/upload/identity.rs:287
  - [ ] fn `suprnova_live::upload::TransferGrantScope::new` · crates/suprnova-live/src/upload/identity.rs:298
  - [ ] fn `suprnova_live::upload::TransferGrantScope::handle` · crates/suprnova-live/src/upload/identity.rs:316
  - [ ] fn `suprnova_live::upload::TransferGrantScope::component` · crates/suprnova-live/src/upload/identity.rs:322
  - [ ] fn `suprnova_live::upload::TransferGrantScope::field` · crates/suprnova-live/src/upload/identity.rs:328
  - [ ] fn `suprnova_live::upload::TransferGrantScope::host_scope` · crates/suprnova-live/src/upload/identity.rs:334
  - [ ] fn `suprnova_live::upload::TransferGrantScope::upload_protocol` · crates/suprnova-live/src/upload/identity.rs:340
- [ ] struct `suprnova_live::upload::UploadError` · crates/suprnova-live/src/upload/identity.rs:170
  - [ ] fn `suprnova_live::upload::UploadError::new` · crates/suprnova-live/src/upload/identity.rs:177
  - [ ] fn `suprnova_live::upload::UploadError::kind` · crates/suprnova-live/src/upload/identity.rs:183
- [ ] struct `suprnova_live::upload::UploadHandle` · crates/suprnova-live/src/upload/identity.rs:204
  - [ ] fn `suprnova_live::upload::UploadHandle::parse` · crates/suprnova-live/src/upload/identity.rs:208
- [ ] struct `suprnova_live::upload::VerifiedTransferGrant` · crates/suprnova-live/src/upload/identity.rs:403
  - [ ] fn `suprnova_live::upload::VerifiedTransferGrant::handle` · crates/suprnova-live/src/upload/identity.rs:411
  - [ ] fn `suprnova_live::upload::VerifiedTransferGrant::component` · crates/suprnova-live/src/upload/identity.rs:417
  - [ ] fn `suprnova_live::upload::VerifiedTransferGrant::field` · crates/suprnova-live/src/upload/identity.rs:423
  - [ ] fn `suprnova_live::upload::VerifiedTransferGrant::scope` · crates/suprnova-live/src/upload/identity.rs:429
  - [ ] fn `suprnova_live::upload::VerifiedTransferGrant::expires_at` · crates/suprnova-live/src/upload/identity.rs:435
  - [ ] fn `suprnova_live::upload::VerifiedTransferGrant::upload_protocol` · crates/suprnova-live/src/upload/identity.rs:441
- [ ] enum `suprnova_live::upload::UploadErrorKind` · crates/suprnova-live/src/upload/identity.rs:39
  - Variants: `InvalidHandle`, `InvalidGrantEncoding`, `InvalidGrant`, `GrantExpired`, `ScopeMismatch`, `UnsupportedProtocol`, `InputTooLarge`, `DuplicateField`, `UnsupportedOperation`, `UnknownField`, `MissingField`, `InvalidField`, `UploadConflict`, `InvalidTransition`, `RevisionExhausted`, `IdempotencyHistoryFull`, `RequestAuthorityExpired`, `AuthorizationUnavailable`, `AuthorizationDenied`, `LedgerUnavailable`, `CreationRateExceeded`, `PendingLimitExceeded`, `FileCountExceeded`, `UploadExpired`, `ServiceRetired`, `RandomUnavailable`, `ProviderUnavailable`, `CleanupTimedOut`, `StorageConflict`, `BodyInterrupted`, `ChecksumMismatch`, `IncompleteTransfer`, `MediaHeaderUnproved`, `ValidationEvidenceUnavailable`, `FinalizationFailed`, `CompensationFailed`, `ReconciliationRequired`, `TransferCanceled`, `ResourceExhausted`
  - [ ] fn `suprnova_live::upload::UploadErrorKind::as_str` · crates/suprnova-live/src/upload/identity.rs:123

### `suprnova_live::upload::ledger` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::upload::ConditionalTransition` · crates/suprnova-live/src/upload/ledger.rs:223
  - [ ] fn `suprnova_live::upload::ConditionalTransition::new` · crates/suprnova-live/src/upload/ledger.rs:232
  - [ ] fn `suprnova_live::upload::ConditionalTransition::authority` · crates/suprnova-live/src/upload/ledger.rs:246
  - [ ] fn `suprnova_live::upload::ConditionalTransition::transition` · crates/suprnova-live/src/upload/ledger.rs:252
  - [ ] fn `suprnova_live::upload::ConditionalTransition::admitted_at` · crates/suprnova-live/src/upload/ledger.rs:258
- [ ] struct `suprnova_live::upload::UploadCreateCommand` · crates/suprnova-live/src/upload/ledger.rs:118
  - [ ] fn `suprnova_live::upload::UploadCreateCommand::new` · crates/suprnova-live/src/upload/ledger.rs:130
  - [ ] fn `suprnova_live::upload::UploadCreateCommand::record` · crates/suprnova-live/src/upload/ledger.rs:150
  - [ ] fn `suprnova_live::upload::UploadCreateCommand::idempotency_key` · crates/suprnova-live/src/upload/ledger.rs:156
  - [ ] fn `suprnova_live::upload::UploadCreateCommand::admitted_at` · crates/suprnova-live/src/upload/ledger.rs:162
  - [ ] fn `suprnova_live::upload::UploadCreateCommand::limits` · crates/suprnova-live/src/upload/ledger.rs:168
  - [ ] fn `suprnova_live::upload::UploadCreateCommand::declared_bytes` · crates/suprnova-live/src/upload/ledger.rs:174
  - [ ] fn `suprnova_live::upload::UploadCreateCommand::policy` · crates/suprnova-live/src/upload/ledger.rs:180
- [ ] struct `suprnova_live::upload::UploadLedgerCreateOutcome` · crates/suprnova-live/src/upload/ledger.rs:193
  - [ ] fn `suprnova_live::upload::UploadLedgerCreateOutcome::new` · crates/suprnova-live/src/upload/ledger.rs:201
  - [ ] fn `suprnova_live::upload::UploadLedgerCreateOutcome::disposition` · crates/suprnova-live/src/upload/ledger.rs:210
  - [ ] fn `suprnova_live::upload::UploadLedgerCreateOutcome::record` · crates/suprnova-live/src/upload/ledger.rs:216
- [ ] struct `suprnova_live::upload::UploadRecord` · crates/suprnova-live/src/upload/ledger.rs:20
  - [ ] fn `suprnova_live::upload::UploadRecord::new` · crates/suprnova-live/src/upload/ledger.rs:30
  - [ ] fn `suprnova_live::upload::UploadRecord::authority` · crates/suprnova-live/src/upload/ledger.rs:51
  - [ ] fn `suprnova_live::upload::UploadRecord::state` · crates/suprnova-live/src/upload/ledger.rs:57
  - [ ] fn `suprnova_live::upload::UploadRecord::revision` · crates/suprnova-live/src/upload/ledger.rs:63
  - [ ] fn `suprnova_live::upload::UploadRecord::created_at` · crates/suprnova-live/src/upload/ledger.rs:69
  - [ ] fn `suprnova_live::upload::UploadRecord::expires_at` · crates/suprnova-live/src/upload/ledger.rs:75
  - [ ] fn `suprnova_live::upload::UploadRecord::with_outcome` · crates/suprnova-live/src/upload/ledger.rs:80
- [ ] enum `suprnova_live::upload::ConditionalUploadCreate` · crates/suprnova-live/src/upload/ledger.rs:109
  - Variants: `Created`, `ExistingOutcome`
- [ ] trait `suprnova_live::upload::UploadLedger` · crates/suprnova-live/src/upload/ledger.rs:270
  - [ ] fn `suprnova_live::upload::UploadLedger::create` · crates/suprnova-live/src/upload/ledger.rs:272 (required)
  - [ ] fn `suprnova_live::upload::UploadLedger::load` · crates/suprnova-live/src/upload/ledger.rs:278 (required)
  - [ ] fn `suprnova_live::upload::UploadLedger::transition` · crates/suprnova-live/src/upload/ledger.rs:284 (required)
- [ ] type `suprnova_live::upload::UploadFuture` · crates/suprnova-live/src/upload/ledger.rs:16

### `suprnova_live::upload::policy` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::upload::AcceptedUploadType` · crates/suprnova-live/src/upload/policy.rs:69
  - [ ] fn `suprnova_live::upload::AcceptedUploadType::application` · crates/suprnova-live/src/upload/policy.rs:76
  - [ ] fn `suprnova_live::upload::AcceptedUploadType::media_type` · crates/suprnova-live/src/upload/policy.rs:109
  - [ ] fn `suprnova_live::upload::AcceptedUploadType::extensions` · crates/suprnova-live/src/upload/policy.rs:115
- [ ] struct `suprnova_live::upload::AuthoritativeUploadType` · crates/suprnova-live/src/upload/policy.rs:42
  - [ ] fn `suprnova_live::upload::AuthoritativeUploadType::application` · crates/suprnova-live/src/upload/policy.rs:46
  - [ ] fn `suprnova_live::upload::AuthoritativeUploadType::media_type` · crates/suprnova-live/src/upload/policy.rs:56
- [ ] struct `suprnova_live::upload::UploadDimensionLimits` · crates/suprnova-live/src/upload/policy.rs:193
  - [ ] fn `suprnova_live::upload::UploadDimensionLimits::new` · crates/suprnova-live/src/upload/policy.rs:201
  - [ ] fn `suprnova_live::upload::UploadDimensionLimits::maximum_width` · crates/suprnova-live/src/upload/policy.rs:218
  - [ ] fn `suprnova_live::upload::UploadDimensionLimits::maximum_height` · crates/suprnova-live/src/upload/policy.rs:224
  - [ ] fn `suprnova_live::upload::UploadDimensionLimits::maximum_pixels` · crates/suprnova-live/src/upload/policy.rs:230
- [ ] struct `suprnova_live::upload::UploadFieldPolicy` · crates/suprnova-live/src/upload/policy.rs:237
  - [ ] fn `suprnova_live::upload::UploadFieldPolicy::new` · crates/suprnova-live/src/upload/policy.rs:254
  - [ ] fn `suprnova_live::upload::UploadFieldPolicy::new_with_accepted_types` · crates/suprnova-live/src/upload/policy.rs:279
  - [ ] fn `suprnova_live::upload::UploadFieldPolicy::maximum_files` · crates/suprnova-live/src/upload/policy.rs:322
  - [ ] fn `suprnova_live::upload::UploadFieldPolicy::maximum_file_bytes` · crates/suprnova-live/src/upload/policy.rs:328
  - [ ] fn `suprnova_live::upload::UploadFieldPolicy::replacement` · crates/suprnova-live/src/upload/policy.rs:334
  - [ ] fn `suprnova_live::upload::UploadFieldPolicy::accepted_types` · crates/suprnova-live/src/upload/policy.rs:340
  - [ ] fn `suprnova_live::upload::UploadFieldPolicy::dimensions` · crates/suprnova-live/src/upload/policy.rs:346
  - [ ] fn `suprnova_live::upload::UploadFieldPolicy::scan` · crates/suprnova-live/src/upload/policy.rs:352
  - [ ] fn `suprnova_live::upload::UploadFieldPolicy::finalize_action` · crates/suprnova-live/src/upload/policy.rs:358
  - [ ] fn `suprnova_live::upload::UploadFieldPolicy::contract_digest` · crates/suprnova-live/src/upload/policy.rs:364
- [ ] enum `suprnova_live::upload::ScanFailurePolicy` · crates/suprnova-live/src/upload/policy.rs:161
  - Variants: `Retry`, `Reject`
- [ ] enum `suprnova_live::upload::UploadMediaType` · crates/suprnova-live/src/upload/policy.rs:16
  - Variants: `Gif`, `Jpeg`, `Png`, `Webp`
  - [ ] fn `suprnova_live::upload::UploadMediaType::media_type` · crates/suprnova-live/src/upload/policy.rs:30
- [ ] enum `suprnova_live::upload::UploadReplacementPolicy` · crates/suprnova-live/src/upload/policy.rs:143
  - Variants: `RetirePrevious`, `PreservePrevious`
- [ ] enum `suprnova_live::upload::UploadScanPolicy` · crates/suprnova-live/src/upload/policy.rs:179
  - Variants: `Disabled`, `Required`

### `suprnova_live::upload::protocol` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::upload::CancelUpload` · crates/suprnova-live/src/upload/protocol.rs:253
  - [ ] fn `suprnova_live::upload::CancelUpload::handle` · crates/suprnova-live/src/upload/protocol.rs:262
  - [ ] fn `suprnova_live::upload::CancelUpload::expected_revision` · crates/suprnova-live/src/upload/protocol.rs:268
  - [ ] fn `suprnova_live::upload::CancelUpload::idempotency_key` · crates/suprnova-live/src/upload/protocol.rs:274
- [ ] struct `suprnova_live::upload::CompleteUpload` · crates/suprnova-live/src/upload/protocol.rs:218
  - [ ] fn `suprnova_live::upload::CompleteUpload::handle` · crates/suprnova-live/src/upload/protocol.rs:228
  - [ ] fn `suprnova_live::upload::CompleteUpload::expected_revision` · crates/suprnova-live/src/upload/protocol.rs:234
  - [ ] fn `suprnova_live::upload::CompleteUpload::idempotency_key` · crates/suprnova-live/src/upload/protocol.rs:240
  - [ ] fn `suprnova_live::upload::CompleteUpload::whole_checksum` · crates/suprnova-live/src/upload/protocol.rs:246
- [ ] struct `suprnova_live::upload::CreateUpload` · crates/suprnova-live/src/upload/protocol.rs:127
  - [ ] fn `suprnova_live::upload::CreateUpload::expected_revision` · crates/suprnova-live/src/upload/protocol.rs:136
  - [ ] fn `suprnova_live::upload::CreateUpload::field` · crates/suprnova-live/src/upload/protocol.rs:142
  - [ ] fn `suprnova_live::upload::CreateUpload::idempotency_key` · crates/suprnova-live/src/upload/protocol.rs:148
- [ ] struct `suprnova_live::upload::PutChunk` · crates/suprnova-live/src/upload/protocol.rs:155
  - [ ] fn `suprnova_live::upload::PutChunk::handle` · crates/suprnova-live/src/upload/protocol.rs:167
  - [ ] fn `suprnova_live::upload::PutChunk::expected_revision` · crates/suprnova-live/src/upload/protocol.rs:173
  - [ ] fn `suprnova_live::upload::PutChunk::idempotency_key` · crates/suprnova-live/src/upload/protocol.rs:179
  - [ ] fn `suprnova_live::upload::PutChunk::chunk_index` · crates/suprnova-live/src/upload/protocol.rs:185
  - [ ] fn `suprnova_live::upload::PutChunk::size` · crates/suprnova-live/src/upload/protocol.rs:191
  - [ ] fn `suprnova_live::upload::PutChunk::checksum` · crates/suprnova-live/src/upload/protocol.rs:197
- [ ] struct `suprnova_live::upload::ReacquireUpload` · crates/suprnova-live/src/upload/protocol.rs:281
  - [ ] fn `suprnova_live::upload::ReacquireUpload::handle` · crates/suprnova-live/src/upload/protocol.rs:288
- [ ] struct `suprnova_live::upload::StatusUpload` · crates/suprnova-live/src/upload/protocol.rs:204
  - [ ] fn `suprnova_live::upload::StatusUpload::handle` · crates/suprnova-live/src/upload/protocol.rs:211
- [ ] struct `suprnova_live::upload::UploadChecksum` · crates/suprnova-live/src/upload/protocol.rs:97
  - [ ] fn `suprnova_live::upload::UploadChecksum::parse` · crates/suprnova-live/src/upload/protocol.rs:101
  - [ ] fn `suprnova_live::upload::UploadChecksum::as_str` · crates/suprnova-live/src/upload/protocol.rs:114
- [ ] struct `suprnova_live::upload::UploadIdempotencyKey` · crates/suprnova-live/src/upload/protocol.rs:66
  - [ ] fn `suprnova_live::upload::UploadIdempotencyKey::parse` · crates/suprnova-live/src/upload/protocol.rs:70
  - [ ] fn `suprnova_live::upload::UploadIdempotencyKey::as_str` · crates/suprnova-live/src/upload/protocol.rs:84
- [ ] struct `suprnova_live::upload::UploadProtocolCodec` · crates/suprnova-live/src/upload/protocol.rs:327
  - [ ] fn `suprnova_live::upload::UploadProtocolCodec::new` · crates/suprnova-live/src/upload/protocol.rs:333
  - [ ] fn `suprnova_live::upload::UploadProtocolCodec::v1` · crates/suprnova-live/src/upload/protocol.rs:346
  - [ ] fn `suprnova_live::upload::UploadProtocolCodec::decode` · crates/suprnova-live/src/upload/protocol.rs:353
- [ ] struct `suprnova_live::upload::UploadRevision` · crates/suprnova-live/src/upload/protocol.rs:20
  - [ ] fn `suprnova_live::upload::UploadRevision::new` · crates/suprnova-live/src/upload/protocol.rs:25
  - [ ] fn `suprnova_live::upload::UploadRevision::initial` · crates/suprnova-live/src/upload/protocol.rs:31
  - [ ] fn `suprnova_live::upload::UploadRevision::parse` · crates/suprnova-live/src/upload/protocol.rs:36
  - [ ] fn `suprnova_live::upload::UploadRevision::get` · crates/suprnova-live/src/upload/protocol.rs:52
- [ ] enum `suprnova_live::upload::UploadOperation` · crates/suprnova-live/src/upload/protocol.rs:295
  - Variants: `Create`, `PutChunk`, `Status`, `Complete`, `Cancel`, `Reacquire`
  - [ ] fn `suprnova_live::upload::UploadOperation::name` · crates/suprnova-live/src/upload/protocol.rs:313
- [ ] const `suprnova_live::upload::SUPPORTED_UPLOAD_PROTOCOL_VERSIONS` · crates/suprnova-live/src/upload/protocol.rs:11

### `suprnova_live::upload::provider` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::upload::CheckpointChunk` · crates/suprnova-live/src/upload/provider.rs:448
  - [ ] fn `suprnova_live::upload::CheckpointChunk::new` · crates/suprnova-live/src/upload/provider.rs:457
  - [ ] fn `suprnova_live::upload::CheckpointChunk::index` · crates/suprnova-live/src/upload/provider.rs:476
  - [ ] fn `suprnova_live::upload::CheckpointChunk::offset` · crates/suprnova-live/src/upload/provider.rs:482
  - [ ] fn `suprnova_live::upload::CheckpointChunk::size` · crates/suprnova-live/src/upload/provider.rs:488
  - [ ] fn `suprnova_live::upload::CheckpointChunk::checksum` · crates/suprnova-live/src/upload/provider.rs:494
- [ ] struct `suprnova_live::upload::ChunkReceipt` · crates/suprnova-live/src/upload/provider.rs:226
  - [ ] fn `suprnova_live::upload::ChunkReceipt::for_direct_part` · crates/suprnova-live/src/upload/provider.rs:237
  - [ ] fn `suprnova_live::upload::ChunkReceipt::index` · crates/suprnova-live/src/upload/provider.rs:253
  - [ ] fn `suprnova_live::upload::ChunkReceipt::offset` · crates/suprnova-live/src/upload/provider.rs:259
  - [ ] fn `suprnova_live::upload::ChunkReceipt::bytes` · crates/suprnova-live/src/upload/provider.rs:265
  - [ ] fn `suprnova_live::upload::ChunkReceipt::disposition` · crates/suprnova-live/src/upload/provider.rs:271
  - [ ] fn `suprnova_live::upload::ChunkReceipt::next_instruction` · crates/suprnova-live/src/upload/provider.rs:277
- [ ] struct `suprnova_live::upload::IntegrityEvidence` · crates/suprnova-live/src/upload/provider.rs:361
  - [ ] fn `suprnova_live::upload::IntegrityEvidence::from_provider` · crates/suprnova-live/src/upload/provider.rs:369
  - [ ] fn `suprnova_live::upload::IntegrityEvidence::bytes` · crates/suprnova-live/src/upload/provider.rs:375
  - [ ] fn `suprnova_live::upload::IntegrityEvidence::checksum` · crates/suprnova-live/src/upload/provider.rs:381
- [ ] struct `suprnova_live::upload::PrepareTransfer` · crates/suprnova-live/src/upload/provider.rs:41
  - [ ] fn `suprnova_live::upload::PrepareTransfer::new` · crates/suprnova-live/src/upload/provider.rs:51
  - [ ] fn `suprnova_live::upload::PrepareTransfer::handle` · crates/suprnova-live/src/upload/provider.rs:67
  - [ ] fn `suprnova_live::upload::PrepareTransfer::expected_bytes` · crates/suprnova-live/src/upload/provider.rs:73
  - [ ] fn `suprnova_live::upload::PrepareTransfer::client_name` · crates/suprnova-live/src/upload/provider.rs:79
  - [ ] fn `suprnova_live::upload::PrepareTransfer::created_at` · crates/suprnova-live/src/upload/provider.rs:85
- [ ] struct `suprnova_live::upload::ProviderRetirementError` · crates/suprnova-live/src/upload/provider.rs:741
  - [ ] fn `suprnova_live::upload::ProviderRetirementError::kind` · crates/suprnova-live/src/upload/provider.rs:753
  - [ ] fn `suprnova_live::upload::ProviderRetirementError::status` · crates/suprnova-live/src/upload/provider.rs:759
- [ ] struct `suprnova_live::upload::ProviderRetirementStatus` · crates/suprnova-live/src/upload/provider.rs:678
  - [ ] fn `suprnova_live::upload::ProviderRetirementStatus::active_operations` · crates/suprnova-live/src/upload/provider.rs:716
  - [ ] fn `suprnova_live::upload::ProviderRetirementStatus::owned_transfers` · crates/suprnova-live/src/upload/provider.rs:722
  - [ ] fn `suprnova_live::upload::ProviderRetirementStatus::active_descriptors` · crates/suprnova-live/src/upload/provider.rs:728
  - [ ] fn `suprnova_live::upload::ProviderRetirementStatus::active_chunks` · crates/suprnova-live/src/upload/provider.rs:734
- [ ] struct `suprnova_live::upload::ProviderTransferAccounting` · crates/suprnova-live/src/upload/provider.rs:687
  - [ ] fn `suprnova_live::upload::ProviderTransferAccounting::accepted_chunk_records` · crates/suprnova-live/src/upload/provider.rs:696
  - [ ] fn `suprnova_live::upload::ProviderTransferAccounting::committed_bytes` · crates/suprnova-live/src/upload/provider.rs:702
  - [ ] fn `suprnova_live::upload::ProviderTransferAccounting::pending_chunk` · crates/suprnova-live/src/upload/provider.rs:708
- [ ] struct `suprnova_live::upload::QuarantinedFileProvider` · crates/suprnova-live/src/upload/provider.rs:664
  - Implements: `suprnova_live::upload::ReverseProxyUploadProvider`, `suprnova_live::upload::UploadProvider`
  - [ ] fn `suprnova_live::upload::QuarantinedFileProvider::new` · crates/suprnova-live/src/upload/provider.rs:1173
  - [ ] fn `suprnova_live::upload::QuarantinedFileProvider::new_with_retirement_wait_steps` · crates/suprnova-live/src/upload/provider.rs:1181
  - [ ] fn `suprnova_live::upload::QuarantinedFileProvider::checkpoint` · crates/suprnova-live/src/upload/provider.rs:1214
  - [ ] fn `suprnova_live::upload::QuarantinedFileProvider::transfer_accounting` · crates/suprnova-live/src/upload/provider.rs:1238
  - [ ] fn `suprnova_live::upload::QuarantinedFileProvider::recover` · crates/suprnova-live/src/upload/provider.rs:1254
  - [ ] fn `suprnova_live::upload::QuarantinedFileProvider::descriptor_permits` · crates/suprnova-live/src/upload/provider.rs:1302
  - [ ] fn `suprnova_live::upload::QuarantinedFileProvider::chunk_permits` · crates/suprnova-live/src/upload/provider.rs:1308
  - [ ] fn `suprnova_live::upload::QuarantinedFileProvider::retire` · crates/suprnova-live/src/upload/provider.rs:1313
  - [ ] fn `suprnova_live::upload::QuarantinedFileProvider::retire_and_cleanup` · crates/suprnova-live/src/upload/provider.rs:1319
  - [ ] fn `suprnova_live::upload::QuarantinedFileProvider::retirement_status` · crates/suprnova-live/src/upload/provider.rs:1365
- [ ] struct `suprnova_live::upload::ReadUpload` · crates/suprnova-live/src/upload/provider.rs:317
  - [ ] fn `suprnova_live::upload::ReadUpload::new` · crates/suprnova-live/src/upload/provider.rs:326
  - [ ] fn `suprnova_live::upload::ReadUpload::handle` · crates/suprnova-live/src/upload/provider.rs:336
  - [ ] fn `suprnova_live::upload::ReadUpload::offset` · crates/suprnova-live/src/upload/provider.rs:342
  - [ ] fn `suprnova_live::upload::ReadUpload::maximum_bytes` · crates/suprnova-live/src/upload/provider.rs:348
- [ ] struct `suprnova_live::upload::TransferCheckpoint` · crates/suprnova-live/src/upload/provider.rs:581
  - [ ] fn `suprnova_live::upload::TransferCheckpoint::new` · crates/suprnova-live/src/upload/provider.rs:593
  - [ ] fn `suprnova_live::upload::TransferCheckpoint::handle` · crates/suprnova-live/src/upload/provider.rs:623
  - [ ] fn `suprnova_live::upload::TransferCheckpoint::object` · crates/suprnova-live/src/upload/provider.rs:629
  - [ ] fn `suprnova_live::upload::TransferCheckpoint::expected_bytes` · crates/suprnova-live/src/upload/provider.rs:635
  - [ ] fn `suprnova_live::upload::TransferCheckpoint::created_at` · crates/suprnova-live/src/upload/provider.rs:641
  - [ ] fn `suprnova_live::upload::TransferCheckpoint::committed_bytes` · crates/suprnova-live/src/upload/provider.rs:647
  - [ ] fn `suprnova_live::upload::TransferCheckpoint::chunks` · crates/suprnova-live/src/upload/provider.rs:652
- [ ] struct `suprnova_live::upload::TransferPlan` · crates/suprnova-live/src/upload/provider.rs:107
  - [ ] fn `suprnova_live::upload::TransferPlan::direct` · crates/suprnova-live/src/upload/provider.rs:129
  - [ ] fn `suprnova_live::upload::TransferPlan::handle` · crates/suprnova-live/src/upload/provider.rs:157
  - [ ] fn `suprnova_live::upload::TransferPlan::maximum_chunk_bytes` · crates/suprnova-live/src/upload/provider.rs:163
  - [ ] fn `suprnova_live::upload::TransferPlan::disposition` · crates/suprnova-live/src/upload/provider.rs:169
  - [ ] fn `suprnova_live::upload::TransferPlan::instructions` · crates/suprnova-live/src/upload/provider.rs:174
- [ ] struct `suprnova_live::upload::VerifyTransfer` · crates/suprnova-live/src/upload/provider.rs:284
  - [ ] fn `suprnova_live::upload::VerifyTransfer::new` · crates/suprnova-live/src/upload/provider.rs:292
  - [ ] fn `suprnova_live::upload::VerifyTransfer::handle` · crates/suprnova-live/src/upload/provider.rs:298
  - [ ] fn `suprnova_live::upload::VerifyTransfer::checksum` · crates/suprnova-live/src/upload/provider.rs:304
- [ ] struct `suprnova_live::upload::WriteChunk` · crates/suprnova-live/src/upload/provider.rs:181
  - [ ] fn `suprnova_live::upload::WriteChunk::new` · crates/suprnova-live/src/upload/provider.rs:192
- [ ] enum `suprnova_live::upload::ChunkDisposition` · crates/suprnova-live/src/upload/provider.rs:217
  - Variants: `Stored`, `ExistingOutcome`
- [ ] enum `suprnova_live::upload::TransferDisposition` · crates/suprnova-live/src/upload/provider.rs:98
  - Variants: `Prepared`, `ExistingOutcome`
- [ ] trait `suprnova_live::upload::ChunkBody` · crates/suprnova-live/src/upload/provider.rs:31
  - [ ] fn `suprnova_live::upload::ChunkBody::next_chunk` · crates/suprnova-live/src/upload/provider.rs:33 (required)
- [ ] trait `suprnova_live::upload::DirectUploadProvider` · crates/suprnova-live/src/upload/provider.rs:430
  - [ ] fn `suprnova_live::upload::DirectUploadProvider::report_part` · crates/suprnova-live/src/upload/provider.rs:432 (required)
- [ ] trait `suprnova_live::upload::ReverseProxyUploadProvider` · crates/suprnova-live/src/upload/provider.rs:420
  - Implemented here by: `upload::QuarantinedFileProvider`
  - [ ] fn `suprnova_live::upload::ReverseProxyUploadProvider::write_chunk` · crates/suprnova-live/src/upload/provider.rs:422 (required)
- [ ] trait `suprnova_live::upload::UploadProvider` · crates/suprnova-live/src/upload/provider.rs:387
  - Implemented here by: `upload::QuarantinedFileProvider`
  - [ ] fn `suprnova_live::upload::UploadProvider::prepare` · crates/suprnova-live/src/upload/provider.rs:389 (required)
  - [ ] fn `suprnova_live::upload::UploadProvider::verify` · crates/suprnova-live/src/upload/provider.rs:395 (required)
  - [ ] fn `suprnova_live::upload::UploadProvider::read` · crates/suprnova-live/src/upload/provider.rs:401 (required)
  - [ ] fn `suprnova_live::upload::UploadProvider::cancel` · crates/suprnova-live/src/upload/provider.rs:407 (required)
  - [ ] fn `suprnova_live::upload::UploadProvider::expire` · crates/suprnova-live/src/upload/provider.rs:410 (provided)
  - [ ] fn `suprnova_live::upload::UploadProvider::cleanup` · crates/suprnova-live/src/upload/provider.rs:415 (required)

### `suprnova_live::upload::quarantine` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::upload::QuarantineCompletion` · crates/suprnova-live/src/upload/quarantine.rs:148
  - [ ] fn `suprnova_live::upload::QuarantineCompletion::complete` · crates/suprnova-live/src/upload/quarantine.rs:155
- [ ] struct `suprnova_live::upload::QuarantineObject` · crates/suprnova-live/src/upload/quarantine.rs:22
  - [ ] fn `suprnova_live::upload::QuarantineObject::generate` · crates/suprnova-live/src/upload/quarantine.rs:26
  - [ ] fn `suprnova_live::upload::QuarantineObject::parse_storage_key` · crates/suprnova-live/src/upload/quarantine.rs:39
  - [ ] fn `suprnova_live::upload::QuarantineObject::storage_key` · crates/suprnova-live/src/upload/quarantine.rs:53
- [ ] struct `suprnova_live::upload::QuarantineOperation` · crates/suprnova-live/src/upload/quarantine.rs:84
  - [ ] fn `suprnova_live::upload::QuarantineOperation::pending` · crates/suprnova-live/src/upload/quarantine.rs:91
  - [ ] fn `suprnova_live::upload::QuarantineOperation::ready` · crates/suprnova-live/src/upload/quarantine.rs:110
- [ ] enum `suprnova_live::upload::RemoveDisposition` · crates/suprnova-live/src/upload/quarantine.rs:66
  - Variants: `Removed`, `AlreadyAbsent`
- [ ] trait `suprnova_live::upload::QuarantineStore` · crates/suprnova-live/src/upload/quarantine.rs:192
  - [ ] fn `suprnova_live::upload::QuarantineStore::create_exclusive` · crates/suprnova-live/src/upload/quarantine.rs:194 (required)
  - [ ] fn `suprnova_live::upload::QuarantineStore::write_at` · crates/suprnova-live/src/upload/quarantine.rs:197 (required)
  - [ ] fn `suprnova_live::upload::QuarantineStore::sync` · crates/suprnova-live/src/upload/quarantine.rs:205 (required)
  - [ ] fn `suprnova_live::upload::QuarantineStore::read_at` · crates/suprnova-live/src/upload/quarantine.rs:208 (required)
  - [ ] fn `suprnova_live::upload::QuarantineStore::read_prefix` · crates/suprnova-live/src/upload/quarantine.rs:216 (provided)
  - [ ] fn `suprnova_live::upload::QuarantineStore::remove` · crates/suprnova-live/src/upload/quarantine.rs:225 (required)
- [ ] type `suprnova_live::upload::QuarantineBytes` · crates/suprnova-live/src/upload/quarantine.rs:18

### `suprnova_live::upload::service` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::upload::UploadAuthorizationRequest` · crates/suprnova-live/src/upload/service.rs:83
  - [ ] fn `suprnova_live::upload::UploadAuthorizationRequest::component` · crates/suprnova-live/src/upload/service.rs:107
  - [ ] fn `suprnova_live::upload::UploadAuthorizationRequest::field` · crates/suprnova-live/src/upload/service.rs:113
  - [ ] fn `suprnova_live::upload::UploadAuthorizationRequest::handle` · crates/suprnova-live/src/upload/service.rs:119
  - [ ] fn `suprnova_live::upload::UploadAuthorizationRequest::control` · crates/suprnova-live/src/upload/service.rs:125
- [ ] struct `suprnova_live::upload::UploadCreateOutcome` · crates/suprnova-live/src/upload/service.rs:283
  - [ ] fn `suprnova_live::upload::UploadCreateOutcome::disposition` · crates/suprnova-live/src/upload/service.rs:292
  - [ ] fn `suprnova_live::upload::UploadCreateOutcome::record` · crates/suprnova-live/src/upload/service.rs:298
  - [ ] fn `suprnova_live::upload::UploadCreateOutcome::grant` · crates/suprnova-live/src/upload/service.rs:304
- [ ] struct `suprnova_live::upload::UploadCreationRequest` · crates/suprnova-live/src/upload/service.rs:153
  - [ ] fn `suprnova_live::upload::UploadCreationRequest::new` · crates/suprnova-live/src/upload/service.rs:165
  - [ ] fn `suprnova_live::upload::UploadCreationRequest::handle` · crates/suprnova-live/src/upload/service.rs:185
  - [ ] fn `suprnova_live::upload::UploadCreationRequest::field` · crates/suprnova-live/src/upload/service.rs:191
  - [ ] fn `suprnova_live::upload::UploadCreationRequest::idempotency_key` · crates/suprnova-live/src/upload/service.rs:197
  - [ ] fn `suprnova_live::upload::UploadCreationRequest::expires_at` · crates/suprnova-live/src/upload/service.rs:203
  - [ ] fn `suprnova_live::upload::UploadCreationRequest::declared_bytes` · crates/suprnova-live/src/upload/service.rs:209
  - [ ] fn `suprnova_live::upload::UploadCreationRequest::policy` · crates/suprnova-live/src/upload/service.rs:215
- [ ] struct `suprnova_live::upload::UploadReacquireOutcome` · crates/suprnova-live/src/upload/service.rs:316
  - [ ] fn `suprnova_live::upload::UploadReacquireOutcome::record` · crates/suprnova-live/src/upload/service.rs:324
  - [ ] fn `suprnova_live::upload::UploadReacquireOutcome::grant` · crates/suprnova-live/src/upload/service.rs:330
- [ ] struct `suprnova_live::upload::UploadReacquireRequest` · crates/suprnova-live/src/upload/service.rs:228
  - [ ] fn `suprnova_live::upload::UploadReacquireRequest::new` · crates/suprnova-live/src/upload/service.rs:237
- [ ] struct `suprnova_live::upload::UploadService` · crates/suprnova-live/src/upload/service.rs:342
  - [ ] fn `suprnova_live::upload::UploadService::new` · crates/suprnova-live/src/upload/service.rs:352
  - [ ] fn `suprnova_live::upload::UploadService::create` · crates/suprnova-live/src/upload/service.rs:374
  - [ ] fn `suprnova_live::upload::UploadService::transition` · crates/suprnova-live/src/upload/service.rs:430
  - [ ] fn `suprnova_live::upload::UploadService::status` · crates/suprnova-live/src/upload/service.rs:460
  - [ ] fn `suprnova_live::upload::UploadService::reacquire` · crates/suprnova-live/src/upload/service.rs:495
  - [ ] fn `suprnova_live::upload::UploadService::cancellation` · crates/suprnova-live/src/upload/service.rs:600
  - [ ] fn `suprnova_live::upload::UploadService::transfer_permits` · crates/suprnova-live/src/upload/service.rs:606
  - [ ] fn `suprnova_live::upload::UploadService::retire` · crates/suprnova-live/src/upload/service.rs:611
- [ ] struct `suprnova_live::upload::UploadTransitionAdmission` · crates/suprnova-live/src/upload/service.rs:254
  - [ ] fn `suprnova_live::upload::UploadTransitionAdmission::new` · crates/suprnova-live/src/upload/service.rs:263
- [ ] enum `suprnova_live::upload::UploadAuthorizationDecision` · crates/suprnova-live/src/upload/service.rs:74
  - Variants: `Allow`, `Deny`
- [ ] enum `suprnova_live::upload::UploadControlKind` · crates/suprnova-live/src/upload/service.rs:23
  - Variants: `Create`, `Status`, `Reacquire`, `Queue`, `BeginTransfer`, `PutChunk`, `Complete`, `Accept`, `BeginFinalize`, `CommitFinalize`, `Cancel`, `Reject`, `Expire`, `Fail`
- [ ] trait `suprnova_live::upload::UploadAuthorizationPort` · crates/suprnova-live/src/upload/service.rs:143
  - [ ] fn `suprnova_live::upload::UploadAuthorizationPort::authorize` · crates/suprnova-live/src/upload/service.rs:145 (required)

### `suprnova_live::upload::state` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::upload::AcceptedChunk` · crates/suprnova-live/src/upload/state.rs:118
  - [ ] fn `suprnova_live::upload::AcceptedChunk::new` · crates/suprnova-live/src/upload/state.rs:126
  - [ ] fn `suprnova_live::upload::AcceptedChunk::index` · crates/suprnova-live/src/upload/state.rs:139
  - [ ] fn `suprnova_live::upload::AcceptedChunk::size` · crates/suprnova-live/src/upload/state.rs:145
  - [ ] fn `suprnova_live::upload::AcceptedChunk::checksum` · crates/suprnova-live/src/upload/state.rs:151
- [ ] struct `suprnova_live::upload::TransitionOutcome` · crates/suprnova-live/src/upload/state.rs:245
  - [ ] fn `suprnova_live::upload::TransitionOutcome::disposition` · crates/suprnova-live/src/upload/state.rs:254
  - [ ] fn `suprnova_live::upload::TransitionOutcome::state` · crates/suprnova-live/src/upload/state.rs:260
  - [ ] fn `suprnova_live::upload::TransitionOutcome::revision` · crates/suprnova-live/src/upload/state.rs:266
- [ ] struct `suprnova_live::upload::UploadStateMachine` · crates/suprnova-live/src/upload/state.rs:280
  - [ ] fn `suprnova_live::upload::UploadStateMachine::new` · crates/suprnova-live/src/upload/state.rs:291
  - [ ] fn `suprnova_live::upload::UploadStateMachine::with_outcome_limit` · crates/suprnova-live/src/upload/state.rs:302
  - [ ] fn `suprnova_live::upload::UploadStateMachine::state` · crates/suprnova-live/src/upload/state.rs:322
  - [ ] fn `suprnova_live::upload::UploadStateMachine::revision` · crates/suprnova-live/src/upload/state.rs:328
  - [ ] fn `suprnova_live::upload::UploadStateMachine::apply` · crates/suprnova-live/src/upload/state.rs:333
  - [ ] fn `suprnova_live::upload::UploadStateMachine::expire_for_cleanup` · crates/suprnova-live/src/upload/state.rs:385
- [ ] struct `suprnova_live::upload::UploadTransitionRequest` · crates/suprnova-live/src/upload/state.rs:185
  - [ ] fn `suprnova_live::upload::UploadTransitionRequest::new` · crates/suprnova-live/src/upload/state.rs:195
  - [ ] fn `suprnova_live::upload::UploadTransitionRequest::handle` · crates/suprnova-live/src/upload/state.rs:211
  - [ ] fn `suprnova_live::upload::UploadTransitionRequest::expected_revision` · crates/suprnova-live/src/upload/state.rs:217
  - [ ] fn `suprnova_live::upload::UploadTransitionRequest::idempotency_key` · crates/suprnova-live/src/upload/state.rs:223
  - [ ] fn `suprnova_live::upload::UploadTransitionRequest::transition` · crates/suprnova-live/src/upload/state.rs:229
- [ ] enum `suprnova_live::upload::TransitionDisposition` · crates/suprnova-live/src/upload/state.rs:236
  - Variants: `Applied`, `ExistingOutcome`
- [ ] enum `suprnova_live::upload::UploadState` · crates/suprnova-live/src/upload/state.rs:15
  - Variants: `Created`, `Queued`, `Transferring`, `Verifying`, `Ready`, `Finalizing`, `Finalized`, `Rejected`, `Canceled`, `Expired`, `Failed`
  - [ ] const `suprnova_live::upload::UploadState::ALL` · crates/suprnova-live/src/upload/state.rs:42
  - [ ] fn `suprnova_live::upload::UploadState::parse` · crates/suprnova-live/src/upload/state.rs:57
  - [ ] fn `suprnova_live::upload::UploadState::as_str` · crates/suprnova-live/src/upload/state.rs:76
  - [ ] fn `suprnova_live::upload::UploadState::is_terminal` · crates/suprnova-live/src/upload/state.rs:94
  - [ ] fn `suprnova_live::upload::UploadState::rank` · crates/suprnova-live/src/upload/state.rs:103
- [ ] enum `suprnova_live::upload::UploadTransition` · crates/suprnova-live/src/upload/state.rs:158
  - Variants: `Queue`, `BeginTransfer`, `PutChunk`, `Complete`, `Accept`, `BeginFinalize`, `CommitFinalize`, `Cancel`, `Reject`, `Expire`, `Fail`

### `suprnova_live::upload::telemetry` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::upload::CleanupMetrics` · crates/suprnova-live/src/upload/telemetry.rs:136
  - [ ] fn `suprnova_live::upload::CleanupMetrics::age_bucket` · crates/suprnova-live/src/upload/telemetry.rs:163
  - [ ] fn `suprnova_live::upload::CleanupMetrics::volume_bucket` · crates/suprnova-live/src/upload/telemetry.rs:169
  - [ ] fn `suprnova_live::upload::CleanupMetrics::outcome` · crates/suprnova-live/src/upload/telemetry.rs:175
  - [ ] fn `suprnova_live::upload::CleanupMetrics::retry_bucket` · crates/suprnova-live/src/upload/telemetry.rs:181
  - [ ] fn `suprnova_live::upload::CleanupMetrics::orphaned` · crates/suprnova-live/src/upload/telemetry.rs:187
- [ ] enum `suprnova_live::upload::CleanupOutcome` · crates/suprnova-live/src/upload/telemetry.rs:77
  - Variants: `Reclaimed`, `RetryScheduled`, `Deferred`, `LeaseLost`
  - [ ] const `suprnova_live::upload::CleanupOutcome::ALL` · crates/suprnova-live/src/upload/telemetry.rs:90
- [ ] enum `suprnova_live::upload::RetryBucket` · crates/suprnova-live/src/upload/telemetry.rs:100
  - Variants: `None`, `One`, `TwoToFour`, `FiveToEight`, `NineOrMore`
  - [ ] const `suprnova_live::upload::RetryBucket::ALL` · crates/suprnova-live/src/upload/telemetry.rs:115
- [ ] enum `suprnova_live::upload::UploadAgeBucket` · crates/suprnova-live/src/upload/telemetry.rs:9
  - Variants: `UnderMinute`, `UnderHour`, `UnderDay`, `DayOrOlder`
  - [ ] const `suprnova_live::upload::UploadAgeBucket::ALL` · crates/suprnova-live/src/upload/telemetry.rs:22
- [ ] enum `suprnova_live::upload::UploadVolumeBucket` · crates/suprnova-live/src/upload/telemetry.rs:41
  - Variants: `Empty`, `UpTo64KiB`, `UpTo1MiB`, `UpTo64MiB`, `Over64MiB`
  - [ ] const `suprnova_live::upload::UploadVolumeBucket::ALL` · crates/suprnova-live/src/upload/telemetry.rs:56
- [ ] trait `suprnova_live::upload::CleanupMetricSink` · crates/suprnova-live/src/upload/telemetry.rs:193
  - [ ] fn `suprnova_live::upload::CleanupMetricSink::record` · crates/suprnova-live/src/upload/telemetry.rs:195 (required)

### `suprnova_live::upload::validation` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::upload::ApplicationValidationInput` · crates/suprnova-live/src/upload/validation.rs:523
  - [ ] fn `suprnova_live::upload::ApplicationValidationInput::upload` · crates/suprnova-live/src/upload/validation.rs:547
  - [ ] fn `suprnova_live::upload::ApplicationValidationInput::content` · crates/suprnova-live/src/upload/validation.rs:553
  - [ ] fn `suprnova_live::upload::ApplicationValidationInput::started_at` · crates/suprnova-live/src/upload/validation.rs:559
  - [ ] fn `suprnova_live::upload::ApplicationValidationInput::deadline` · crates/suprnova-live/src/upload/validation.rs:565
- [ ] struct `suprnova_live::upload::ClientUploadMetadata` · crates/suprnova-live/src/upload/validation.rs:27
  - [ ] fn `suprnova_live::upload::ClientUploadMetadata::new` · crates/suprnova-live/src/upload/validation.rs:34
  - [ ] fn `suprnova_live::upload::ClientUploadMetadata::display_name` · crates/suprnova-live/src/upload/validation.rs:72
  - [ ] fn `suprnova_live::upload::ClientUploadMetadata::claimed_media_type` · crates/suprnova-live/src/upload/validation.rs:78
- [ ] struct `suprnova_live::upload::MediaDimensions` · crates/suprnova-live/src/upload/validation.rs:132
  - [ ] fn `suprnova_live::upload::MediaDimensions::new` · crates/suprnova-live/src/upload/validation.rs:140
  - [ ] fn `suprnova_live::upload::MediaDimensions::width` · crates/suprnova-live/src/upload/validation.rs:156
  - [ ] fn `suprnova_live::upload::MediaDimensions::height` · crates/suprnova-live/src/upload/validation.rs:162
  - [ ] fn `suprnova_live::upload::MediaDimensions::pixels` · crates/suprnova-live/src/upload/validation.rs:168
- [ ] struct `suprnova_live::upload::MediaHeaderProbe` · crates/suprnova-live/src/upload/validation.rs:174
  - [ ] fn `suprnova_live::upload::MediaHeaderProbe::classify` · crates/suprnova-live/src/upload/validation.rs:179
  - [ ] fn `suprnova_live::upload::MediaHeaderProbe::prefix_limit` · crates/suprnova-live/src/upload/validation.rs:195
  - [ ] fn `suprnova_live::upload::MediaHeaderProbe::probe` · crates/suprnova-live/src/upload/validation.rs:206
- [ ] struct `suprnova_live::upload::ScanInput` · crates/suprnova-live/src/upload/validation.rs:468
  - [ ] fn `suprnova_live::upload::ScanInput::upload` · crates/suprnova-live/src/upload/validation.rs:492
  - [ ] fn `suprnova_live::upload::ScanInput::content` · crates/suprnova-live/src/upload/validation.rs:498
  - [ ] fn `suprnova_live::upload::ScanInput::started_at` · crates/suprnova-live/src/upload/validation.rs:504
  - [ ] fn `suprnova_live::upload::ScanInput::deadline` · crates/suprnova-live/src/upload/validation.rs:510
- [ ] struct `suprnova_live::upload::ScanReason` · crates/suprnova-live/src/upload/validation.rs:224
  - [ ] fn `suprnova_live::upload::ScanReason::parse` · crates/suprnova-live/src/upload/validation.rs:228
  - [ ] fn `suprnova_live::upload::ScanReason::as_str` · crates/suprnova-live/src/upload/validation.rs:245
- [ ] struct `suprnova_live::upload::UploadContent` · crates/suprnova-live/src/upload/validation.rs:386
  - [ ] fn `suprnova_live::upload::UploadContent::read` · crates/suprnova-live/src/upload/validation.rs:412
  - [ ] fn `suprnova_live::upload::UploadContent::total_bytes` · crates/suprnova-live/src/upload/validation.rs:443
  - [ ] fn `suprnova_live::upload::UploadContent::maximum_read_bytes` · crates/suprnova-live/src/upload/validation.rs:449
  - [ ] fn `suprnova_live::upload::UploadContent::deadline` · crates/suprnova-live/src/upload/validation.rs:455
- [ ] struct `suprnova_live::upload::UploadInspection` · crates/suprnova-live/src/upload/validation.rs:285
  - [ ] fn `suprnova_live::upload::UploadInspection::from_store` · crates/suprnova-live/src/upload/validation.rs:302
  - [ ] fn `suprnova_live::upload::UploadInspection::handle` · crates/suprnova-live/src/upload/validation.rs:337
  - [ ] fn `suprnova_live::upload::UploadInspection::client` · crates/suprnova-live/src/upload/validation.rs:343
  - [ ] fn `suprnova_live::upload::UploadInspection::detected_type` · crates/suprnova-live/src/upload/validation.rs:349
  - [ ] fn `suprnova_live::upload::UploadInspection::authoritative_type` · crates/suprnova-live/src/upload/validation.rs:355
  - [ ] fn `suprnova_live::upload::UploadInspection::bytes` · crates/suprnova-live/src/upload/validation.rs:361
  - [ ] fn `suprnova_live::upload::UploadInspection::checksum` · crates/suprnova-live/src/upload/validation.rs:367
  - [ ] fn `suprnova_live::upload::UploadInspection::dimensions` · crates/suprnova-live/src/upload/validation.rs:373
  - [ ] fn `suprnova_live::upload::UploadInspection::inspected_at` · crates/suprnova-live/src/upload/validation.rs:379
- [ ] struct `suprnova_live::upload::UploadValidationOutcome` · crates/suprnova-live/src/upload/validation.rs:773
  - [ ] fn `suprnova_live::upload::UploadValidationOutcome::disposition` · crates/suprnova-live/src/upload/validation.rs:783
  - [ ] fn `suprnova_live::upload::UploadValidationOutcome::evidence` · crates/suprnova-live/src/upload/validation.rs:789
  - [ ] fn `suprnova_live::upload::UploadValidationOutcome::reason` · crates/suprnova-live/src/upload/validation.rs:795
  - [ ] fn `suprnova_live::upload::UploadValidationOutcome::transition` · crates/suprnova-live/src/upload/validation.rs:801
- [ ] struct `suprnova_live::upload::UploadValidationRequest` · crates/suprnova-live/src/upload/validation.rs:719
  - [ ] fn `suprnova_live::upload::UploadValidationRequest::new` · crates/suprnova-live/src/upload/validation.rs:737
- [ ] struct `suprnova_live::upload::UploadValidationService` · crates/suprnova-live/src/upload/validation.rs:807
  - [ ] fn `suprnova_live::upload::UploadValidationService::new` · crates/suprnova-live/src/upload/validation.rs:818
  - [ ] fn `suprnova_live::upload::UploadValidationService::validate` · crates/suprnova-live/src/upload/validation.rs:843
- [ ] struct `suprnova_live::upload::ValidatedUpload` · crates/suprnova-live/src/upload/validation.rs:621
  - [ ] fn `suprnova_live::upload::ValidatedUpload::from_store` · crates/suprnova-live/src/upload/validation.rs:630
  - [ ] fn `suprnova_live::upload::ValidatedUpload::handle` · crates/suprnova-live/src/upload/validation.rs:649
  - [ ] fn `suprnova_live::upload::ValidatedUpload::authority` · crates/suprnova-live/src/upload/validation.rs:655
  - [ ] fn `suprnova_live::upload::ValidatedUpload::ready_revision` · crates/suprnova-live/src/upload/validation.rs:661
  - [ ] fn `suprnova_live::upload::ValidatedUpload::policy_digest` · crates/suprnova-live/src/upload/validation.rs:667
  - [ ] fn `suprnova_live::upload::ValidatedUpload::inspection` · crates/suprnova-live/src/upload/validation.rs:673
- [ ] enum `suprnova_live::upload::ApplicationValidationDecision` · crates/suprnova-live/src/upload/validation.rs:271
  - Variants: `Allow`, `AllowAs`, `Reject`
- [ ] enum `suprnova_live::upload::DetectedUploadType` · crates/suprnova-live/src/upload/validation.rs:103
  - Variants: `Gif`, `Jpeg`, `Png`, `Webp`, `Unknown`
  - [ ] fn `suprnova_live::upload::DetectedUploadType::media_type` · crates/suprnova-live/src/upload/validation.rs:119
- [ ] enum `suprnova_live::upload::ScanDisposition` · crates/suprnova-live/src/upload/validation.rs:258
  - Variants: `Clean`, `Rejected`, `Unavailable`, `TimedOut`
- [ ] enum `suprnova_live::upload::UploadRejectionReason` · crates/suprnova-live/src/upload/validation.rs:596
  - Variants: `SizeMismatch`, `IntegrityMismatch`, `TypeMismatch`, `MediaHeaderUnproved`, `DimensionsExceeded`, `PixelsExceeded`, `ScanRejected`, `ScanTimedOut`, `ScanUnavailable`, `ApplicationRejected`
- [ ] enum `suprnova_live::upload::UploadValidationDisposition` · crates/suprnova-live/src/upload/validation.rs:762
  - Variants: `Ready`, `Rejected`, `Retry`
- [ ] enum `suprnova_live::upload::ValidationStoreDisposition` · crates/suprnova-live/src/upload/validation.rs:692
  - Variants: `Stored`, `ExistingOutcome`
- [ ] trait `suprnova_live::upload::UploadApplicationValidator` · crates/suprnova-live/src/upload/validation.rs:586
  - [ ] fn `suprnova_live::upload::UploadApplicationValidator::validate` · crates/suprnova-live/src/upload/validation.rs:588 (required)
- [ ] trait `suprnova_live::upload::UploadScanner` · crates/suprnova-live/src/upload/validation.rs:577
  - [ ] fn `suprnova_live::upload::UploadScanner::scan` · crates/suprnova-live/src/upload/validation.rs:579 (required)
- [ ] trait `suprnova_live::upload::UploadValidationStore` · crates/suprnova-live/src/upload/validation.rs:700
  - [ ] fn `suprnova_live::upload::UploadValidationStore::put` · crates/suprnova-live/src/upload/validation.rs:702 (required)
  - [ ] fn `suprnova_live::upload::UploadValidationStore::load` · crates/suprnova-live/src/upload/validation.rs:708 (required)
  - [ ] fn `suprnova_live::upload::UploadValidationStore::remove` · crates/suprnova-live/src/upload/validation.rs:714 (required)

## validation

### `suprnova_live::validation::engine` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::validation::ValidationEngine` · crates/suprnova-live/src/validation/engine.rs:64
  - [ ] fn `suprnova_live::validation::ValidationEngine::new` · crates/suprnova-live/src/validation/engine.rs:70
  - [ ] fn `suprnova_live::validation::ValidationEngine::validate` · crates/suprnova-live/src/validation/engine.rs:85
- [ ] struct `suprnova_live::validation::ValidationEngineError` · crates/suprnova-live/src/validation/engine.rs:28
  - [ ] fn `suprnova_live::validation::ValidationEngineError::kind` · crates/suprnova-live/src/validation/engine.rs:39
- [ ] enum `suprnova_live::validation::ValidationEngineErrorKind` · crates/suprnova-live/src/validation/engine.rs:17
  - Variants: `InvalidSelection`, `ProviderFailure`, `TooManyIssues`

### `suprnova_live::validation::error_bag` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::validation::ErrorBag` · crates/suprnova-live/src/validation/error_bag.rs:100
  - [ ] fn `suprnova_live::validation::ErrorBag::from_issues` · crates/suprnova-live/src/validation/error_bag.rs:106
  - [ ] fn `suprnova_live::validation::ErrorBag::issues` · crates/suprnova-live/src/validation/error_bag.rs:115
  - [ ] fn `suprnova_live::validation::ErrorBag::len` · crates/suprnova-live/src/validation/error_bag.rs:121
  - [ ] fn `suprnova_live::validation::ErrorBag::is_empty` · crates/suprnova-live/src/validation/error_bag.rs:127
- [ ] struct `suprnova_live::validation::ValidationIssue` · crates/suprnova-live/src/validation/error_bag.rs:43
  - [ ] fn `suprnova_live::validation::ValidationIssue::new` · crates/suprnova-live/src/validation/error_bag.rs:51
  - [ ] fn `suprnova_live::validation::ValidationIssue::path` · crates/suprnova-live/src/validation/error_bag.rs:57
  - [ ] fn `suprnova_live::validation::ValidationIssue::message` · crates/suprnova-live/src/validation/error_bag.rs:63
- [ ] struct `suprnova_live::validation::ValidationMessageId` · crates/suprnova-live/src/validation/error_bag.rs:12
  - [ ] fn `suprnova_live::validation::ValidationMessageId::parse` · crates/suprnova-live/src/validation/error_bag.rs:16
  - [ ] fn `suprnova_live::validation::ValidationMessageId::as_str` · crates/suprnova-live/src/validation/error_bag.rs:30
- [ ] enum `suprnova_live::validation::BagPolicy` · crates/suprnova-live/src/validation/error_bag.rs:80
  - Variants: `Clear`, `Retain`, `Replace`
- [ ] enum `suprnova_live::validation::ValidationStatus` · crates/suprnova-live/src/validation/error_bag.rs:91
  - Variants: `Valid`, `Invalid`

### `suprnova_live::validation::port` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::validation::ValidationPortError` · crates/suprnova-live/src/validation/port.rs:144
  - [ ] fn `suprnova_live::validation::ValidationPortError::unavailable` · crates/suprnova-live/src/validation/port.rs:149
- [ ] struct `suprnova_live::validation::ValidationRequest` · crates/suprnova-live/src/validation/port.rs:35
  - [ ] fn `suprnova_live::validation::ValidationRequest::new` · crates/suprnova-live/src/validation/port.rs:48
  - [ ] fn `suprnova_live::validation::ValidationRequest::component` · crates/suprnova-live/src/validation/port.rs:67
  - [ ] fn `suprnova_live::validation::ValidationRequest::with_action` · crates/suprnova-live/src/validation/port.rs:73
  - [ ] fn `suprnova_live::validation::ValidationRequest::with_prepared_arguments` · crates/suprnova-live/src/validation/port.rs:80
  - [ ] fn `suprnova_live::validation::ValidationRequest::with_target` · crates/suprnova-live/src/validation/port.rs:87
  - [ ] fn `suprnova_live::validation::ValidationRequest::selection` · crates/suprnova-live/src/validation/port.rs:94
  - [ ] fn `suprnova_live::validation::ValidationRequest::state` · crates/suprnova-live/src/validation/port.rs:100
  - [ ] fn `suprnova_live::validation::ValidationRequest::arguments` · crates/suprnova-live/src/validation/port.rs:106
  - [ ] fn `suprnova_live::validation::ValidationRequest::decode_argument` · crates/suprnova-live/src/validation/port.rs:111
  - [ ] fn `suprnova_live::validation::ValidationRequest::action` · crates/suprnova-live/src/validation/port.rs:123
  - [ ] fn `suprnova_live::validation::ValidationRequest::target_mut` · crates/suprnova-live/src/validation/port.rs:128
- [ ] enum `suprnova_live::validation::ValidationSelection` · crates/suprnova-live/src/validation/port.rs:21
  - Variants: `None`, `Selected`, `WholeComponent`, `ActionArguments`, `ComponentAndArguments`
- [ ] trait `suprnova_live::validation::ValidationPort` · crates/suprnova-live/src/validation/port.rs:134
  - [ ] fn `suprnova_live::validation::ValidationPort::validate` · crates/suprnova-live/src/validation/port.rs:136 (required)
- [ ] type `suprnova_live::validation::ValidationFuture` · crates/suprnova-live/src/validation/port.rs:17

## view

### `suprnova_live::view`

- [ ] struct `suprnova_live::view::ViewRenderer` · crates/suprnova-live/src/view/mod.rs:51
  - [ ] fn `suprnova_live::view::ViewRenderer::new` · crates/suprnova-live/src/view/mod.rs:57
  - [ ] fn `suprnova_live::view::ViewRenderer::render_document` · crates/suprnova-live/src/view/mod.rs:69
  - [ ] fn `suprnova_live::view::ViewRenderer::render_island` · crates/suprnova-live/src/view/mod.rs:104
  - [ ] fn `suprnova_live::view::ViewRenderer::render_component_fragment` · crates/suprnova-live/src/view/mod.rs:123
  - [ ] fn `suprnova_live::view::ViewRenderer::validate_island_fragment` · crates/suprnova-live/src/view/mod.rs:141
  - [ ] fn `suprnova_live::view::ViewRenderer::validate_island_output` · crates/suprnova-live/src/view/mod.rs:170

### `suprnova_live::view::charts`

- [ ] fn `suprnova_live::view::charts::render_chart` · crates/suprnova-live/src/view/charts.rs:124
- [ ] struct `suprnova_live::view::charts::ChartError` · crates/suprnova-live/src/view/charts.rs:72
  - [ ] fn `suprnova_live::view::charts::ChartError::kind` · crates/suprnova-live/src/view/charts.rs:83
- [ ] struct `suprnova_live::view::charts::ChartSeries` · crates/suprnova-live/src/view/charts.rs:39
  - [ ] fn `suprnova_live::view::charts::ChartSeries::new` · crates/suprnova-live/src/view/charts.rs:48
- [ ] enum `suprnova_live::view::charts::ChartErrorKind` · crates/suprnova-live/src/view/charts.rs:58
  - Variants: `Shape`, `Input`, `TooLarge`, `Render`
- [ ] enum `suprnova_live::view::charts::ChartKind` · crates/suprnova-live/src/view/charts.rs:30
  - Variants: `Bar`, `Line`

### `suprnova_live::view::contract` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::view::AssetSet` · crates/suprnova-live/src/view/contract.rs:144
  - [ ] fn `suprnova_live::view::AssetSet::empty` · crates/suprnova-live/src/view/contract.rs:151
  - [ ] fn `suprnova_live::view::AssetSet::new` · crates/suprnova-live/src/view/contract.rs:159
  - [ ] fn `suprnova_live::view::AssetSet::is_empty` · crates/suprnova-live/src/view/contract.rs:167
  - [ ] fn `suprnova_live::view::AssetSet::len` · crates/suprnova-live/src/view/contract.rs:173
- [ ] struct `suprnova_live::view::ChildMount` · crates/suprnova-live/src/view/contract.rs:248
  - [ ] fn `suprnova_live::view::ChildMount::new` · crates/suprnova-live/src/view/contract.rs:263
  - [ ] fn `suprnova_live::view::ChildMount::surviving` · crates/suprnova-live/src/view/contract.rs:273
  - [ ] fn `suprnova_live::view::ChildMount::pending_parameters` · crates/suprnova-live/src/view/contract.rs:283
  - [ ] fn `suprnova_live::view::ChildMount::slot` · crates/suprnova-live/src/view/contract.rs:293
  - [ ] fn `suprnova_live::view::ChildMount::component` · crates/suprnova-live/src/view/contract.rs:299
- [ ] struct `suprnova_live::view::MountMetadata` · crates/suprnova-live/src/view/contract.rs:189
  - [ ] fn `suprnova_live::view::MountMetadata::new` · crates/suprnova-live/src/view/contract.rs:198
  - [ ] fn `suprnova_live::view::MountMetadata::slot` · crates/suprnova-live/src/view/contract.rs:217
  - [ ] fn `suprnova_live::view::MountMetadata::component` · crates/suprnova-live/src/view/contract.rs:223
  - [ ] fn `suprnova_live::view::MountMetadata::snapshot_kind` · crates/suprnova-live/src/view/contract.rs:229
  - [ ] fn `suprnova_live::view::MountMetadata::signed_snapshot` · crates/suprnova-live/src/view/contract.rs:235
- [ ] struct `suprnova_live::view::RenderLimits` · crates/suprnova-live/src/view/contract.rs:22
  - [ ] fn `suprnova_live::view::RenderLimits::new` · crates/suprnova-live/src/view/contract.rs:32
  - [ ] fn `suprnova_live::view::RenderLimits::standard` · crates/suprnova-live/src/view/contract.rs:58
- [ ] enum `suprnova_live::view::MountSnapshotKind` · crates/suprnova-live/src/view/contract.rs:180
  - Variants: `PublicSeed`, `Instance`
- [ ] trait `suprnova_live::view::ViewTemplate` · crates/suprnova-live/src/view/contract.rs:124
  - [ ] fn `suprnova_live::view::ViewTemplate::render_view` · crates/suprnova-live/src/view/contract.rs:126 (required)

### `suprnova_live::view::document` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::view::CanonicalDocumentConformance` · crates/suprnova-live/src/view/document.rs:269
  - [ ] fn `suprnova_live::view::CanonicalDocumentConformance::project` · crates/suprnova-live/src/view/document.rs:274
- [ ] struct `suprnova_live::view::CanonicalDocumentIntent` · crates/suprnova-live/src/view/document.rs:216
  - [ ] fn `suprnova_live::view::CanonicalDocumentIntent::status` · crates/suprnova-live/src/view/document.rs:238
  - [ ] fn `suprnova_live::view::CanonicalDocumentIntent::response` · crates/suprnova-live/src/view/document.rs:244
  - [ ] fn `suprnova_live::view::CanonicalDocumentIntent::body` · crates/suprnova-live/src/view/document.rs:250
  - [ ] fn `suprnova_live::view::CanonicalDocumentIntent::representation_length` · crates/suprnova-live/src/view/document.rs:256
  - [ ] fn `suprnova_live::view::CanonicalDocumentIntent::validator` · crates/suprnova-live/src/view/document.rs:262
- [ ] struct `suprnova_live::view::CanonicalDocumentRequest` · crates/suprnova-live/src/view/document.rs:189
  - [ ] fn `suprnova_live::view::CanonicalDocumentRequest::get` · crates/suprnova-live/src/view/document.rs:197
  - [ ] fn `suprnova_live::view::CanonicalDocumentRequest::head` · crates/suprnova-live/src/view/document.rs:206
- [ ] struct `suprnova_live::view::DocumentRender` · crates/suprnova-live/src/view/document.rs:148
  - Public fields: `body`, `response`, `assets`, `mounts`
- [ ] struct `suprnova_live::view::DocumentResponseIntent` · crates/suprnova-live/src/view/document.rs:37
  - [ ] fn `suprnova_live::view::DocumentResponseIntent::html` · crates/suprnova-live/src/view/document.rs:58
  - [ ] fn `suprnova_live::view::DocumentResponseIntent::with_header` · crates/suprnova-live/src/view/document.rs:73
  - [ ] fn `suprnova_live::view::DocumentResponseIntent::with_cache` · crates/suprnova-live/src/view/document.rs:89
  - [ ] fn `suprnova_live::view::DocumentResponseIntent::status` · crates/suprnova-live/src/view/document.rs:96
  - [ ] fn `suprnova_live::view::DocumentResponseIntent::headers` · crates/suprnova-live/src/view/document.rs:102
  - [ ] fn `suprnova_live::view::DocumentResponseIntent::cache` · crates/suprnova-live/src/view/document.rs:108
  - [ ] fn `suprnova_live::view::DocumentResponseIntent::media_type` · crates/suprnova-live/src/view/document.rs:114
- [ ] struct `suprnova_live::view::DocumentValidator` · crates/suprnova-live/src/view/document.rs:173
- [ ] enum `suprnova_live::view::DocumentCachePolicy` · crates/suprnova-live/src/view/document.rs:19
  - Variants: `Private`, `NoStore`, `Public`
- [ ] enum `suprnova_live::view::DocumentMediaType` · crates/suprnova-live/src/view/document.rs:30
  - Variants: `HtmlUtf8`

### `suprnova_live::view::error` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::view::ViewError` · crates/suprnova-live/src/view/error.rs:69
  - [ ] fn `suprnova_live::view::ViewError::kind` · crates/suprnova-live/src/view/error.rs:88
- [ ] enum `suprnova_live::view::ViewErrorKind` · crates/suprnova-live/src/view/error.rs:10
  - Variants: `InvalidLimits`, `TemplateRenderFailed`, `MissingViewData`, `InvalidViewData`, `BodyTooLarge`, `TooManyAssets`, `TooManyMounts`, `TooManyChildren`, `InvalidMountMetadata`, `InvalidDocument`, `MountMetadataMismatch`, `MissingIslandRoot`, `MultipleIslandRoots`, `ExecutableMountMetadata`, `ForbiddenResponseIntent`
  - [ ] fn `suprnova_live::view::ViewErrorKind::as_str` · crates/suprnova-live/src/view/error.rs:46

### `suprnova_live::view::island` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::view::IslandRender` · crates/suprnova-live/src/view/island.rs:37
  - Public fields: `body`, `assets`, `children`

### `suprnova_live::view::live_key` (private module; items are public through re-exports)

- [ ] fn `suprnova_live::view::check_live_key` · crates/suprnova-live/src/view/live_key.rs:70
- [ ] fn `suprnova_live::view::live_key_digest` · crates/suprnova-live/src/view/live_key.rs:93
- [ ] struct `suprnova_live::view::LiveKeyError` · crates/suprnova-live/src/view/live_key.rs:31
  - [ ] fn `suprnova_live::view::LiveKeyError::kind` · crates/suprnova-live/src/view/live_key.rs:38
- [ ] enum `suprnova_live::view::LiveKeyErrorKind` · crates/suprnova-live/src/view/live_key.rs:19
  - Variants: `Empty`, `TooLong`, `ForbiddenByte`

### `suprnova_live::view::live_key::filters` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::view::filters::live_key` · crates/suprnova-live/src/view/live_key.rs:119
  - [ ] fn `suprnova_live::view::filters::live_key::execute` · crates/suprnova-live/src/view/live_key.rs:120
- [ ] struct `suprnova_live::view::filters::live_key_digest` · crates/suprnova-live/src/view/live_key.rs:128
  - [ ] fn `suprnova_live::view::filters::live_key_digest::execute` · crates/suprnova-live/src/view/live_key.rs:129

### `suprnova_live::view::trusted_html` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::view::RegisteredSanitizer` · crates/suprnova-live/src/view/trusted_html.rs:78
  - [ ] fn `suprnova_live::view::RegisteredSanitizer::new` · crates/suprnova-live/src/view/trusted_html.rs:86
  - [ ] fn `suprnova_live::view::RegisteredSanitizer::sanitize` · crates/suprnova-live/src/view/trusted_html.rs:94
- [ ] struct `suprnova_live::view::SanitizerFailure` · crates/suprnova-live/src/view/trusted_html.rs:75
- [ ] struct `suprnova_live::view::SanitizerId` · crates/suprnova-live/src/view/trusted_html.rs:48
  - [ ] fn `suprnova_live::view::SanitizerId::parse` · crates/suprnova-live/src/view/trusted_html.rs:52
- [ ] struct `suprnova_live::view::TrustedHtml` · crates/suprnova-live/src/view/trusted_html.rs:180
  - [ ] fn `suprnova_live::view::TrustedHtml::framework_static` · crates/suprnova-live/src/view/trusted_html.rs:188
  - [ ] fn `suprnova_live::view::TrustedHtml::framework_generated` · crates/suprnova-live/src/view/trusted_html.rs:209
  - [ ] fn `suprnova_live::view::TrustedHtml::reason` · crates/suprnova-live/src/view/trusted_html.rs:227
  - [ ] fn `suprnova_live::view::TrustedHtml::as_str` · crates/suprnova-live/src/view/trusted_html.rs:233
- [ ] struct `suprnova_live::view::TrustedMarkupError` · crates/suprnova-live/src/view/trusted_html.rs:305
  - [ ] fn `suprnova_live::view::TrustedMarkupError::kind` · crates/suprnova-live/src/view/trusted_html.rs:316
- [ ] struct `suprnova_live::view::TrustedMarkupReason` · crates/suprnova-live/src/view/trusted_html.rs:15
  - [ ] fn `suprnova_live::view::TrustedMarkupReason::new` · crates/suprnova-live/src/view/trusted_html.rs:19
  - [ ] fn `suprnova_live::view::TrustedMarkupReason::as_str` · crates/suprnova-live/src/view/trusted_html.rs:35
- [ ] enum `suprnova_live::view::TrustedMarkupErrorKind` · crates/suprnova-live/src/view/trusted_html.rs:292
  - Variants: `InvalidReason`, `InvalidSanitizer`, `MarkupTooLarge`, `SanitizationFailed`

### `suprnova_live::view::trusted_html::filters` (private module; items are public through re-exports)

- [ ] struct `suprnova_live::view::filters::trusted_html` · crates/suprnova-live/src/view/trusted_html.rs:281
  - [ ] fn `suprnova_live::view::filters::trusted_html::execute` · crates/suprnova-live/src/view/trusted_html.rs:282

## Public but unnameable

Reachable from the public API (as a return type, field or supertrait) but not importable by any public path.

- [ ] struct `suprnova_live::validation::error_bag::ValidationBagError` · crates/suprnova-live/src/validation/error_bag.rs:172 (public, but no public path names it)
- [ ] trait `suprnova_live::view::contract::sealed::Sealed` · crates/suprnova-live/src/view/contract.rs:102 (sealed trait: a supertrait that stops implementations outside the crate)
