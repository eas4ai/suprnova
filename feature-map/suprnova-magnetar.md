# Suprnova feature map: `suprnova-magnetar` Rust API

Source: `crates/suprnova-magnetar/` at d03b4f1, rustdoc JSON (all features), cross-checked against a default-features build.

A checked box means the documentation for that item has been remediated
against the source. Items are listed under their shortest public path;
`also` names the other paths the same item is reachable by.

## Counts

- Top-level items (including re-exports): 527
- Members (methods, associated consts and types): 643
- By kind: constant 29, enum 98, function 39, struct 289, trait 63, type_alias 8, unnameable 1

## abuse

### `magnetar::abuse`

- [ ] struct `magnetar::abuse::AbusePolicy` · crates/suprnova-magnetar/src/abuse.rs:12 (also `magnetar::plugin::AbusePolicy`)
  - Public fields: `max_requests`, `window`
  - [ ] fn `magnetar::abuse::AbusePolicy::validate` · crates/suprnova-magnetar/src/abuse.rs:21
- [ ] enum `magnetar::abuse::Permit` · crates/suprnova-magnetar/src/abuse.rs:40 (also `magnetar::plugin::Permit`)
  - Variants: `Allowed`, `Rejected`
- [ ] trait `magnetar::abuse::AbuseLimiter` · crates/suprnova-magnetar/src/abuse.rs:56 (also `magnetar::plugin::AbuseLimiter`)
  - Implemented here by: `drivers::redis_abuse::RedisAbuseLimiter`
  - [ ] fn `magnetar::abuse::AbuseLimiter::acquire` · crates/suprnova-magnetar/src/abuse.rs:61 (required)

## auth

### `magnetar::auth::factor_gate` (private module; items are public through re-exports)

- [ ] struct `magnetar::auth::OpaqueFactorGate` · crates/suprnova-magnetar/src/auth/factor_gate.rs:142
  - [ ] fn `magnetar::auth::OpaqueFactorGate::new` · crates/suprnova-magnetar/src/auth/factor_gate.rs:178
- [ ] struct `magnetar::auth::PreparedFactorProof` · crates/suprnova-magnetar/src/auth/factor_gate.rs:39
  - [ ] fn `magnetar::auth::PreparedFactorProof::valid` · crates/suprnova-magnetar/src/auth/factor_gate.rs:46
  - [ ] fn `magnetar::auth::PreparedFactorProof::invalid` · crates/suprnova-magnetar/src/auth/factor_gate.rs:55
- [ ] enum `magnetar::auth::SignInDecision` · crates/suprnova-magnetar/src/auth/factor_gate.rs:24
  - Variants: `SessionAllowed`, `FactorRequired`
- [ ] trait `magnetar::auth::FactorGate` · crates/suprnova-magnetar/src/auth/factor_gate.rs:124 (also `magnetar::plugin::FactorGate`)
  - Implemented here by: `auth::OpaqueFactorGate`
  - [ ] fn `magnetar::auth::FactorGate::complete_sign_in` · crates/suprnova-magnetar/src/auth/factor_gate.rs:127 (required)
  - [ ] fn `magnetar::auth::FactorGate::complete_challenge` · crates/suprnova-magnetar/src/auth/factor_gate.rs:134 (required)
- [ ] trait `magnetar::auth::FactorVerifier` · crates/suprnova-magnetar/src/auth/factor_gate.rs:88
  - Implemented here by: `two_factor::TwoFactorService`
  - [ ] type `magnetar::auth::FactorVerifier::PreparedProof` · crates/suprnova-magnetar/src/auth/factor_gate.rs:90
  - [ ] fn `magnetar::auth::FactorVerifier::has_confirmed_enrollment` · crates/suprnova-magnetar/src/auth/factor_gate.rs:93 (required)
  - [ ] fn `magnetar::auth::FactorVerifier::prepare_code` · crates/suprnova-magnetar/src/auth/factor_gate.rs:96 (required)
  - [ ] fn `magnetar::auth::FactorVerifier::claim_prepared` · crates/suprnova-magnetar/src/auth/factor_gate.rs:107 (required)
  - [ ] fn `magnetar::auth::FactorVerifier::cancel_prepared` · crates/suprnova-magnetar/src/auth/factor_gate.rs:113 (provided)
- [ ] const `magnetar::auth::TWO_FACTOR_CHALLENGE_KIND` · crates/suprnova-magnetar/src/auth/factor_gate.rs:17

### `magnetar::auth::primary` (private module; items are public through re-exports)

- [ ] struct `magnetar::auth::AuthenticationContext` · crates/suprnova-magnetar/src/auth/primary.rs:64
  - Public fields: `metadata`, `auth_epoch`, `authenticated_at`
  - [ ] fn `magnetar::auth::AuthenticationContext::new` · crates/suprnova-magnetar/src/auth/primary.rs:76
- [ ] struct `magnetar::auth::VerifiedPrincipal` · crates/suprnova-magnetar/src/auth/primary.rs:95
  - [ ] fn `magnetar::auth::VerifiedPrincipal::user_id` · crates/suprnova-magnetar/src/auth/primary.rs:104
  - [ ] fn `magnetar::auth::VerifiedPrincipal::method` · crates/suprnova-magnetar/src/auth/primary.rs:110
  - [ ] fn `magnetar::auth::VerifiedPrincipal::context` · crates/suprnova-magnetar/src/auth/primary.rs:116
- [ ] enum `magnetar::auth::PrimaryCredential` · crates/suprnova-magnetar/src/auth/primary.rs:31
  - Variants: `Password`, `MagicLink`, `Passkey`, `OAuth`, `DeviceApproval`
- [ ] enum `magnetar::auth::SignInMethod` · crates/suprnova-magnetar/src/auth/primary.rs:11
  - Variants: `Password`, `MagicLink`, `Passkey`, `OAuth`, `Remembered`, `DeviceApproval`
- [ ] trait `magnetar::auth::PrimaryAuth` · crates/suprnova-magnetar/src/auth/primary.rs:154
  - [ ] fn `magnetar::auth::PrimaryAuth::verify` · crates/suprnova-magnetar/src/auth/primary.rs:156 (required)

### `magnetar::auth::reauth`

- [ ] fn `magnetar::auth::reauth::validate_reauth` · crates/suprnova-magnetar/src/auth/reauth.rs:50
- [ ] struct `magnetar::auth::reauth::ReauthCapability` · crates/suprnova-magnetar/src/auth/reauth.rs:27
  - [ ] fn `magnetar::auth::reauth::ReauthCapability::owner_user_id` · crates/suprnova-magnetar/src/auth/reauth.rs:35
  - [ ] fn `magnetar::auth::reauth::ReauthCapability::password_confirmed_at` · crates/suprnova-magnetar/src/auth/reauth.rs:41
- [ ] struct `magnetar::auth::reauth::ReauthStamp` · crates/suprnova-magnetar/src/auth/reauth.rs:15
  - Public fields: `owner_user_id`, `password_confirmed_at`
- [ ] const `magnetar::auth::reauth::REAUTH_WINDOW` · crates/suprnova-magnetar/src/auth/reauth.rs:8

## broker

### `magnetar::broker` (feature: `oauth`, off by default)

- [ ] fn `magnetar::broker::record_id_for_linked_account` · crates/suprnova-magnetar/src/broker/mod.rs:329
- [ ] struct `magnetar::broker::AccessToken` · crates/suprnova-magnetar/src/broker/mod.rs:94
  - Public fields: `value`, `token_type`, `expires_at`, `scopes`
- [ ] struct `magnetar::broker::BrokerConfig` · crates/suprnova-magnetar/src/broker/mod.rs:44
  - Public fields: `single_flight`, `provider_call_timeout`, `lease_grace`, `poll_interval`, `m2m_cache`
- [ ] struct `magnetar::broker::RefreshRequest` · crates/suprnova-magnetar/src/broker/mod.rs:86
  - Public fields: `record_id`, `presented_generation`
- [ ] struct `magnetar::broker::TokenBrokerService` · crates/suprnova-magnetar/src/broker/mod.rs:266
  - [ ] fn `magnetar::broker::TokenBrokerService::new` · crates/suprnova-magnetar/src/broker/mod.rs:279
  - [ ] fn `magnetar::broker::TokenBrokerService::with_reuse_hook` · crates/suprnova-magnetar/src/broker/mod.rs:300
  - [ ] fn `magnetar::broker::TokenBrokerService::provision_linked_account` · crates/suprnova-magnetar/src/broker/mod.rs:310
- [ ] enum `magnetar::broker::BrokerError` · crates/suprnova-magnetar/src/broker/mod.rs:136
  - Variants: `NotFound`, `Revoked`, `UnknownProvider`, `Retriable`, `Terminal`, `LeaseTimeout`, `Storage`
- [ ] trait `magnetar::broker::ReuseHook` · crates/suprnova-magnetar/src/broker/mod.rs:126
  - [ ] fn `magnetar::broker::ReuseHook::on_reuse_detected` · crates/suprnova-magnetar/src/broker/mod.rs:128 (required)
- [ ] trait `magnetar::broker::TokenBroker` · crates/suprnova-magnetar/src/broker/mod.rs:249
  - Implemented here by: `broker::TokenBrokerService`
  - [ ] fn `magnetar::broker::TokenBroker::access_token` · crates/suprnova-magnetar/src/broker/mod.rs:253 (required)
  - [ ] fn `magnetar::broker::TokenBroker::refresh` · crates/suprnova-magnetar/src/broker/mod.rs:257 (required)
  - [ ] fn `magnetar::broker::TokenBroker::client_credentials` · crates/suprnova-magnetar/src/broker/mod.rs:261 (required)
- [ ] type `magnetar::broker::BrokerResult` · crates/suprnova-magnetar/src/broker/mod.rs:243

### `magnetar::broker::cache` (feature: `oauth`, off by default)

- [ ] fn `magnetar::broker::cache::needs_refresh` · crates/suprnova-magnetar/src/broker/cache.rs:112
- [ ] struct `magnetar::broker::M2MCacheConfig` · crates/suprnova-magnetar/src/broker/cache.rs:84 (also `magnetar::broker::cache::M2MCacheConfig`)
  - Public fields: `refresh_before`, `jitter`
- [ ] struct `magnetar::broker::M2MCacheKey` · crates/suprnova-magnetar/src/broker/cache.rs:24 (also `magnetar::broker::cache::M2MCacheKey`)
  - Public fields: `provider`, `client_id`, `scopes`
  - [ ] fn `magnetar::broker::M2MCacheKey::new` · crates/suprnova-magnetar/src/broker/cache.rs:38
  - [ ] fn `magnetar::broker::M2MCacheKey::normalized_scopes` · crates/suprnova-magnetar/src/broker/cache.rs:53
  - [ ] fn `magnetar::broker::M2MCacheKey::record_id` · crates/suprnova-magnetar/src/broker/cache.rs:64

### `magnetar::broker::singleflight` (feature: `oauth`, off by default)

- [ ] struct `magnetar::broker::singleflight::SingleFlight` · crates/suprnova-magnetar/src/broker/singleflight.rs:36
  - [ ] fn `magnetar::broker::singleflight::SingleFlight::new` · crates/suprnova-magnetar/src/broker/singleflight.rs:43
  - [ ] fn `magnetar::broker::singleflight::SingleFlight::run` · crates/suprnova-magnetar/src/broker/singleflight.rs:48

## crypto

### `magnetar::crypto`

- [ ] struct `magnetar::crypto::AeadEncryptor` · crates/suprnova-magnetar/src/crypto.rs:71
  - [ ] fn `magnetar::crypto::AeadEncryptor::new` · crates/suprnova-magnetar/src/crypto.rs:78
- [ ] enum `magnetar::crypto::CryptoPurpose` · crates/suprnova-magnetar/src/crypto.rs:27
  - Variants: `CeremonyState`, `TwoFactorSecret`, `TwoFactorRecovery`, `ProviderToken`, `RefreshToken`, `SessionGrant`
  - [ ] fn `magnetar::crypto::CryptoPurpose::label` · crates/suprnova-magnetar/src/crypto.rs:47
  - [ ] fn `magnetar::crypto::CryptoPurpose::label_str` · crates/suprnova-magnetar/src/crypto.rs:53
- [ ] trait `magnetar::crypto::Encryptor` · crates/suprnova-magnetar/src/crypto.rs:134
  - Implemented here by: `crypto::AeadEncryptor`
  - [ ] fn `magnetar::crypto::Encryptor::encrypt` · crates/suprnova-magnetar/src/crypto.rs:136 (required)
  - [ ] fn `magnetar::crypto::Encryptor::decrypt` · crates/suprnova-magnetar/src/crypto.rs:139 (required)

## default_first_email_proof

### `magnetar::default_first_email_proof`

- [ ] struct `magnetar::default_first_email_proof::SqlFirstEmailProofStore` · crates/suprnova-magnetar/src/default_first_email_proof.rs:45
  - [ ] fn `magnetar::default_first_email_proof::SqlFirstEmailProofStore::new` · crates/suprnova-magnetar/src/default_first_email_proof.rs:54

## default_migration

### `magnetar::default_migration` (feature: `migration`, off by default)

- [ ] struct `magnetar::default_migration::DefaultMigrationBindings` · crates/suprnova-magnetar/src/default_migration.rs:25
  - [ ] fn `magnetar::default_migration::DefaultMigrationBindings::new` · crates/suprnova-magnetar/src/default_migration.rs:33
  - [ ] fn `magnetar::default_migration::DefaultMigrationBindings::sharing_source_database` · crates/suprnova-magnetar/src/default_migration.rs:45

## default_schema

### `magnetar::default_schema`

- [ ] fn `magnetar::default_schema::database` · crates/suprnova-magnetar/src/default_schema.rs:2081
- [ ] fn `magnetar::default_schema::migrate` · crates/suprnova-magnetar/src/default_schema.rs:1574
- [ ] struct `magnetar::default_schema::DefaultAuthSchema` · crates/suprnova-magnetar/src/default_schema.rs:186

### `magnetar::default_schema::accounts`

- [ ] struct `magnetar::default_schema::accounts::ActiveModel` · crates/suprnova-magnetar/src/default_schema.rs:64
  - Public fields: `id`, `user_id`, `provider`, `provider_account_id`, `created_at`, `updated_at`
- [ ] struct `magnetar::default_schema::accounts::ColumnIter` · crates/suprnova-magnetar/src/default_schema.rs:64
- [ ] struct `magnetar::default_schema::accounts::Entity` · crates/suprnova-magnetar/src/default_schema.rs:64
- [ ] struct `magnetar::default_schema::accounts::Model` · crates/suprnova-magnetar/src/default_schema.rs:64
  - Public fields: `id`, `user_id`, `provider`, `provider_account_id`, `created_at`, `updated_at`
  - [ ] fn `magnetar::default_schema::accounts::Model::into_ex` · crates/suprnova-magnetar/src/default_schema.rs:64
- [ ] struct `magnetar::default_schema::accounts::PrimaryKeyIter` · crates/suprnova-magnetar/src/default_schema.rs:64
- [ ] struct `magnetar::default_schema::accounts::RelationIter` · crates/suprnova-magnetar/src/default_schema.rs:64
- [ ] enum `magnetar::default_schema::accounts::Column` · crates/suprnova-magnetar/src/default_schema.rs:64
  - Variants: `Id`, `UserId`, `Provider`, `ProviderAccountId`, `CreatedAt`, `UpdatedAt`
- [ ] enum `magnetar::default_schema::accounts::PrimaryKey` · crates/suprnova-magnetar/src/default_schema.rs:64
  - Variants: `Id`
- [ ] enum `magnetar::default_schema::accounts::Relation` · crates/suprnova-magnetar/src/default_schema.rs:64

### `magnetar::default_schema::ceremonies`

- [ ] struct `magnetar::default_schema::ceremonies::ActiveModel` · crates/suprnova-magnetar/src/default_schema.rs:82
  - Public fields: `id`, `kind`, `selector`, `payload`, `state`, `expires_at`, `used_at`
- [ ] struct `magnetar::default_schema::ceremonies::ColumnIter` · crates/suprnova-magnetar/src/default_schema.rs:82
- [ ] struct `magnetar::default_schema::ceremonies::Entity` · crates/suprnova-magnetar/src/default_schema.rs:82
- [ ] struct `magnetar::default_schema::ceremonies::Model` · crates/suprnova-magnetar/src/default_schema.rs:82
  - Public fields: `id`, `kind`, `selector`, `payload`, `state`, `expires_at`, `used_at`
  - [ ] fn `magnetar::default_schema::ceremonies::Model::into_ex` · crates/suprnova-magnetar/src/default_schema.rs:82
- [ ] struct `magnetar::default_schema::ceremonies::PrimaryKeyIter` · crates/suprnova-magnetar/src/default_schema.rs:82
- [ ] struct `magnetar::default_schema::ceremonies::RelationIter` · crates/suprnova-magnetar/src/default_schema.rs:82
- [ ] enum `magnetar::default_schema::ceremonies::Column` · crates/suprnova-magnetar/src/default_schema.rs:82
  - Variants: `Id`, `Kind`, `Selector`, `Payload`, `State`, `ExpiresAt`, `UsedAt`
- [ ] enum `magnetar::default_schema::ceremonies::PrimaryKey` · crates/suprnova-magnetar/src/default_schema.rs:82
  - Variants: `Id`
- [ ] enum `magnetar::default_schema::ceremonies::Relation` · crates/suprnova-magnetar/src/default_schema.rs:82

### `magnetar::default_schema::lifecycle_deliveries`

- [ ] struct `magnetar::default_schema::lifecycle_deliveries::ActiveModel` · crates/suprnova-magnetar/src/default_schema.rs:137
  - Public fields: `mutation_id`, `lease_id`, `lease_until`, `delivered_at`
- [ ] struct `magnetar::default_schema::lifecycle_deliveries::ColumnIter` · crates/suprnova-magnetar/src/default_schema.rs:137
- [ ] struct `magnetar::default_schema::lifecycle_deliveries::Entity` · crates/suprnova-magnetar/src/default_schema.rs:137
- [ ] struct `magnetar::default_schema::lifecycle_deliveries::Model` · crates/suprnova-magnetar/src/default_schema.rs:137
  - Public fields: `mutation_id`, `lease_id`, `lease_until`, `delivered_at`
  - [ ] fn `magnetar::default_schema::lifecycle_deliveries::Model::into_ex` · crates/suprnova-magnetar/src/default_schema.rs:137
- [ ] struct `magnetar::default_schema::lifecycle_deliveries::PrimaryKeyIter` · crates/suprnova-magnetar/src/default_schema.rs:137
- [ ] struct `magnetar::default_schema::lifecycle_deliveries::RelationIter` · crates/suprnova-magnetar/src/default_schema.rs:137
- [ ] enum `magnetar::default_schema::lifecycle_deliveries::Column` · crates/suprnova-magnetar/src/default_schema.rs:137
  - Variants: `MutationId`, `LeaseId`, `LeaseUntil`, `DeliveredAt`
- [ ] enum `magnetar::default_schema::lifecycle_deliveries::PrimaryKey` · crates/suprnova-magnetar/src/default_schema.rs:137
  - Variants: `MutationId`
- [ ] enum `magnetar::default_schema::lifecycle_deliveries::Relation` · crates/suprnova-magnetar/src/default_schema.rs:137

### `magnetar::default_schema::lockouts`

- [ ] struct `magnetar::default_schema::lockouts::ActiveModel` · crates/suprnova-magnetar/src/default_schema.rs:91
  - Public fields: `id`, `identity`, `attempted_at`, `ip_address`, `migration_source_id`, `locked_at`, `reason`
- [ ] struct `magnetar::default_schema::lockouts::ColumnIter` · crates/suprnova-magnetar/src/default_schema.rs:91
- [ ] struct `magnetar::default_schema::lockouts::Entity` · crates/suprnova-magnetar/src/default_schema.rs:91
- [ ] struct `magnetar::default_schema::lockouts::Model` · crates/suprnova-magnetar/src/default_schema.rs:91
  - Public fields: `id`, `identity`, `attempted_at`, `ip_address`, `migration_source_id`, `locked_at`, `reason`
  - [ ] fn `magnetar::default_schema::lockouts::Model::into_ex` · crates/suprnova-magnetar/src/default_schema.rs:91
- [ ] struct `magnetar::default_schema::lockouts::PrimaryKeyIter` · crates/suprnova-magnetar/src/default_schema.rs:91
- [ ] struct `magnetar::default_schema::lockouts::RelationIter` · crates/suprnova-magnetar/src/default_schema.rs:91
- [ ] enum `magnetar::default_schema::lockouts::Column` · crates/suprnova-magnetar/src/default_schema.rs:91
  - Variants: `Id`, `Identity`, `AttemptedAt`, `IpAddress`, `MigrationSourceId`, `LockedAt`, `Reason`
- [ ] enum `magnetar::default_schema::lockouts::PrimaryKey` · crates/suprnova-magnetar/src/default_schema.rs:91
  - Variants: `Id`
- [ ] enum `magnetar::default_schema::lockouts::Relation` · crates/suprnova-magnetar/src/default_schema.rs:91

### `magnetar::default_schema::methods`

- [ ] struct `magnetar::default_schema::methods::ActiveModel` · crates/suprnova-magnetar/src/default_schema.rs:57
  - Public fields: `id`, `user_id`, `credential_id`, `public_key`, `created_at`
- [ ] struct `magnetar::default_schema::methods::ColumnIter` · crates/suprnova-magnetar/src/default_schema.rs:57
- [ ] struct `magnetar::default_schema::methods::Entity` · crates/suprnova-magnetar/src/default_schema.rs:57
- [ ] struct `magnetar::default_schema::methods::Model` · crates/suprnova-magnetar/src/default_schema.rs:57
  - Public fields: `id`, `user_id`, `credential_id`, `public_key`, `created_at`
  - [ ] fn `magnetar::default_schema::methods::Model::into_ex` · crates/suprnova-magnetar/src/default_schema.rs:57
- [ ] struct `magnetar::default_schema::methods::PrimaryKeyIter` · crates/suprnova-magnetar/src/default_schema.rs:57
- [ ] struct `magnetar::default_schema::methods::RelationIter` · crates/suprnova-magnetar/src/default_schema.rs:57
- [ ] enum `magnetar::default_schema::methods::Column` · crates/suprnova-magnetar/src/default_schema.rs:57
  - Variants: `Id`, `UserId`, `CredentialId`, `PublicKey`, `CreatedAt`
- [ ] enum `magnetar::default_schema::methods::PrimaryKey` · crates/suprnova-magnetar/src/default_schema.rs:57
  - Variants: `Id`
- [ ] enum `magnetar::default_schema::methods::Relation` · crates/suprnova-magnetar/src/default_schema.rs:57

### `magnetar::default_schema::migration_identities`

- [ ] struct `magnetar::default_schema::migration_identities::ActiveModel` · crates/suprnova-magnetar/src/default_schema.rs:148
  - Public fields: `id`, `plan_id`, `source_user_id`, `app_user_id`
- [ ] struct `magnetar::default_schema::migration_identities::ColumnIter` · crates/suprnova-magnetar/src/default_schema.rs:148
- [ ] struct `magnetar::default_schema::migration_identities::Entity` · crates/suprnova-magnetar/src/default_schema.rs:148
- [ ] struct `magnetar::default_schema::migration_identities::Model` · crates/suprnova-magnetar/src/default_schema.rs:148
  - Public fields: `id`, `plan_id`, `source_user_id`, `app_user_id`
  - [ ] fn `magnetar::default_schema::migration_identities::Model::into_ex` · crates/suprnova-magnetar/src/default_schema.rs:148
- [ ] struct `magnetar::default_schema::migration_identities::PrimaryKeyIter` · crates/suprnova-magnetar/src/default_schema.rs:148
- [ ] struct `magnetar::default_schema::migration_identities::RelationIter` · crates/suprnova-magnetar/src/default_schema.rs:148
- [ ] enum `magnetar::default_schema::migration_identities::Column` · crates/suprnova-magnetar/src/default_schema.rs:148
  - Variants: `Id`, `PlanId`, `SourceUserId`, `AppUserId`
- [ ] enum `magnetar::default_schema::migration_identities::PrimaryKey` · crates/suprnova-magnetar/src/default_schema.rs:148
  - Variants: `Id`
- [ ] enum `magnetar::default_schema::migration_identities::Relation` · crates/suprnova-magnetar/src/default_schema.rs:148

### `magnetar::default_schema::migration_runs`

- [ ] struct `magnetar::default_schema::migration_runs::ActiveModel` · crates/suprnova-magnetar/src/default_schema.rs:143
  - Public fields: `plan_id`, `imports_committed`, `completed_at`
- [ ] struct `magnetar::default_schema::migration_runs::ColumnIter` · crates/suprnova-magnetar/src/default_schema.rs:143
- [ ] struct `magnetar::default_schema::migration_runs::Entity` · crates/suprnova-magnetar/src/default_schema.rs:143
- [ ] struct `magnetar::default_schema::migration_runs::Model` · crates/suprnova-magnetar/src/default_schema.rs:143
  - Public fields: `plan_id`, `imports_committed`, `completed_at`
  - [ ] fn `magnetar::default_schema::migration_runs::Model::into_ex` · crates/suprnova-magnetar/src/default_schema.rs:143
- [ ] struct `magnetar::default_schema::migration_runs::PrimaryKeyIter` · crates/suprnova-magnetar/src/default_schema.rs:143
- [ ] struct `magnetar::default_schema::migration_runs::RelationIter` · crates/suprnova-magnetar/src/default_schema.rs:143
- [ ] enum `magnetar::default_schema::migration_runs::Column` · crates/suprnova-magnetar/src/default_schema.rs:143
  - Variants: `PlanId`, `ImportsCommitted`, `CompletedAt`
- [ ] enum `magnetar::default_schema::migration_runs::PrimaryKey` · crates/suprnova-magnetar/src/default_schema.rs:143
  - Variants: `PlanId`
- [ ] enum `magnetar::default_schema::migration_runs::Relation` · crates/suprnova-magnetar/src/default_schema.rs:143

### `magnetar::default_schema::migration_state`

- [ ] struct `magnetar::default_schema::migration_state::ActiveModel` · crates/suprnova-magnetar/src/default_schema.rs:154
  - Public fields: `key`, `value`
- [ ] struct `magnetar::default_schema::migration_state::ColumnIter` · crates/suprnova-magnetar/src/default_schema.rs:154
- [ ] struct `magnetar::default_schema::migration_state::Entity` · crates/suprnova-magnetar/src/default_schema.rs:154
- [ ] struct `magnetar::default_schema::migration_state::Model` · crates/suprnova-magnetar/src/default_schema.rs:154
  - Public fields: `key`, `value`
  - [ ] fn `magnetar::default_schema::migration_state::Model::into_ex` · crates/suprnova-magnetar/src/default_schema.rs:154
- [ ] struct `magnetar::default_schema::migration_state::PrimaryKeyIter` · crates/suprnova-magnetar/src/default_schema.rs:154
- [ ] struct `magnetar::default_schema::migration_state::RelationIter` · crates/suprnova-magnetar/src/default_schema.rs:154
- [ ] enum `magnetar::default_schema::migration_state::Column` · crates/suprnova-magnetar/src/default_schema.rs:154
  - Variants: `Key`, `Value`
- [ ] enum `magnetar::default_schema::migration_state::PrimaryKey` · crates/suprnova-magnetar/src/default_schema.rs:154
  - Variants: `Key`
- [ ] enum `magnetar::default_schema::migration_state::Relation` · crates/suprnova-magnetar/src/default_schema.rs:154

### `magnetar::default_schema::provider_tokens`

- [ ] struct `magnetar::default_schema::provider_tokens::ActiveModel` · crates/suprnova-magnetar/src/default_schema.rs:121
  - Public fields: `id`, `provider`, `access_ciphertext`, `refresh_ciphertext`, `raw_payload_ciphertext`, `token_type`, `scopes`, `access_expires_at`, `generation`, `claim_id`, `claim_deadline`, `revoked_at`, `revoked_reused`, `created_at`
- [ ] struct `magnetar::default_schema::provider_tokens::ColumnIter` · crates/suprnova-magnetar/src/default_schema.rs:121
- [ ] struct `magnetar::default_schema::provider_tokens::Entity` · crates/suprnova-magnetar/src/default_schema.rs:121
- [ ] struct `magnetar::default_schema::provider_tokens::Model` · crates/suprnova-magnetar/src/default_schema.rs:121
  - Public fields: `id`, `provider`, `access_ciphertext`, `refresh_ciphertext`, `raw_payload_ciphertext`, `token_type`, `scopes`, `access_expires_at`, `generation`, `claim_id`, `claim_deadline`, `revoked_at`, `revoked_reused`, `created_at`
  - [ ] fn `magnetar::default_schema::provider_tokens::Model::into_ex` · crates/suprnova-magnetar/src/default_schema.rs:121
- [ ] struct `magnetar::default_schema::provider_tokens::PrimaryKeyIter` · crates/suprnova-magnetar/src/default_schema.rs:121
- [ ] struct `magnetar::default_schema::provider_tokens::RelationIter` · crates/suprnova-magnetar/src/default_schema.rs:121
- [ ] enum `magnetar::default_schema::provider_tokens::Column` · crates/suprnova-magnetar/src/default_schema.rs:121
  - Variants: `Id`, `Provider`, `AccessCiphertext`, `RefreshCiphertext`, `RawPayloadCiphertext`, `TokenType`, `Scopes`, `AccessExpiresAt`, `Generation`, `ClaimId`, `ClaimDeadline`, `RevokedAt`, `RevokedReused`, `CreatedAt`
- [ ] enum `magnetar::default_schema::provider_tokens::PrimaryKey` · crates/suprnova-magnetar/src/default_schema.rs:121
  - Variants: `Id`
- [ ] enum `magnetar::default_schema::provider_tokens::Relation` · crates/suprnova-magnetar/src/default_schema.rs:121

### `magnetar::default_schema::remembers`

- [ ] struct `magnetar::default_schema::remembers::ActiveModel` · crates/suprnova-magnetar/src/default_schema.rs:113
  - Public fields: `id`, `selector`, `user_id`, `auth_epoch`, `verifier_hash`, `expires_at`
- [ ] struct `magnetar::default_schema::remembers::ColumnIter` · crates/suprnova-magnetar/src/default_schema.rs:113
- [ ] struct `magnetar::default_schema::remembers::Entity` · crates/suprnova-magnetar/src/default_schema.rs:113
- [ ] struct `magnetar::default_schema::remembers::Model` · crates/suprnova-magnetar/src/default_schema.rs:113
  - Public fields: `id`, `selector`, `user_id`, `auth_epoch`, `verifier_hash`, `expires_at`
  - [ ] fn `magnetar::default_schema::remembers::Model::into_ex` · crates/suprnova-magnetar/src/default_schema.rs:113
- [ ] struct `magnetar::default_schema::remembers::PrimaryKeyIter` · crates/suprnova-magnetar/src/default_schema.rs:113
- [ ] struct `magnetar::default_schema::remembers::RelationIter` · crates/suprnova-magnetar/src/default_schema.rs:113
- [ ] enum `magnetar::default_schema::remembers::Column` · crates/suprnova-magnetar/src/default_schema.rs:113
  - Variants: `Id`, `Selector`, `UserId`, `AuthEpoch`, `VerifierHash`, `ExpiresAt`
- [ ] enum `magnetar::default_schema::remembers::PrimaryKey` · crates/suprnova-magnetar/src/default_schema.rs:113
  - Variants: `Id`
- [ ] enum `magnetar::default_schema::remembers::Relation` · crates/suprnova-magnetar/src/default_schema.rs:113

### `magnetar::default_schema::sessions`

- [ ] struct `magnetar::default_schema::sessions::ActiveModel` · crates/suprnova-magnetar/src/default_schema.rs:46
  - Public fields: `id`, `user_id`, `auth_epoch`, `token_digest`, `token_hash`, `user_agent`, `ip_address`, `expires_at`, `revoked_at`
- [ ] struct `magnetar::default_schema::sessions::ColumnIter` · crates/suprnova-magnetar/src/default_schema.rs:46
- [ ] struct `magnetar::default_schema::sessions::Entity` · crates/suprnova-magnetar/src/default_schema.rs:46
- [ ] struct `magnetar::default_schema::sessions::Model` · crates/suprnova-magnetar/src/default_schema.rs:46
  - Public fields: `id`, `user_id`, `auth_epoch`, `token_digest`, `token_hash`, `user_agent`, `ip_address`, `expires_at`, `revoked_at`
  - [ ] fn `magnetar::default_schema::sessions::Model::into_ex` · crates/suprnova-magnetar/src/default_schema.rs:46
- [ ] struct `magnetar::default_schema::sessions::PrimaryKeyIter` · crates/suprnova-magnetar/src/default_schema.rs:46
- [ ] struct `magnetar::default_schema::sessions::RelationIter` · crates/suprnova-magnetar/src/default_schema.rs:46
- [ ] enum `magnetar::default_schema::sessions::Column` · crates/suprnova-magnetar/src/default_schema.rs:46
  - Variants: `Id`, `UserId`, `AuthEpoch`, `TokenDigest`, `TokenHash`, `UserAgent`, `IpAddress`, `ExpiresAt`, `RevokedAt`
- [ ] enum `magnetar::default_schema::sessions::PrimaryKey` · crates/suprnova-magnetar/src/default_schema.rs:46
  - Variants: `Id`
- [ ] enum `magnetar::default_schema::sessions::Relation` · crates/suprnova-magnetar/src/default_schema.rs:46

### `magnetar::default_schema::sql_stores`

- [ ] struct `magnetar::default_schema::sql_stores::SqlRememberStore` · crates/suprnova-magnetar/src/default_schema.rs:917
  - Public tuple fields: 1
- [ ] struct `magnetar::default_schema::sql_stores::SqlSessionStore` · crates/suprnova-magnetar/src/default_schema.rs:840
  - Public tuple fields: 1

### `magnetar::default_schema::sql_two_factor` (feature: `two-factor`, off by default)

- [ ] struct `magnetar::default_schema::sql_two_factor::SqlTwoFactorStore` · crates/suprnova-magnetar/src/default_schema.rs:1384
  - Public tuple fields: 1

### `magnetar::default_schema::tokens`

- [ ] struct `magnetar::default_schema::tokens::ActiveModel` · crates/suprnova-magnetar/src/default_schema.rs:72
  - Public fields: `id`, `user_id`, `purpose`, `digest`, `expires_at`, `used_at`, `created_at`, `updated_at`
- [ ] struct `magnetar::default_schema::tokens::ColumnIter` · crates/suprnova-magnetar/src/default_schema.rs:72
- [ ] struct `magnetar::default_schema::tokens::Entity` · crates/suprnova-magnetar/src/default_schema.rs:72
- [ ] struct `magnetar::default_schema::tokens::Model` · crates/suprnova-magnetar/src/default_schema.rs:72
  - Public fields: `id`, `user_id`, `purpose`, `digest`, `expires_at`, `used_at`, `created_at`, `updated_at`
  - [ ] fn `magnetar::default_schema::tokens::Model::into_ex` · crates/suprnova-magnetar/src/default_schema.rs:72
- [ ] struct `magnetar::default_schema::tokens::PrimaryKeyIter` · crates/suprnova-magnetar/src/default_schema.rs:72
- [ ] struct `magnetar::default_schema::tokens::RelationIter` · crates/suprnova-magnetar/src/default_schema.rs:72
- [ ] enum `magnetar::default_schema::tokens::Column` · crates/suprnova-magnetar/src/default_schema.rs:72
  - Variants: `Id`, `UserId`, `Purpose`, `Digest`, `ExpiresAt`, `UsedAt`, `CreatedAt`, `UpdatedAt`
- [ ] enum `magnetar::default_schema::tokens::PrimaryKey` · crates/suprnova-magnetar/src/default_schema.rs:72
  - Variants: `Id`
- [ ] enum `magnetar::default_schema::tokens::Relation` · crates/suprnova-magnetar/src/default_schema.rs:72

### `magnetar::default_schema::two_factor`

- [ ] struct `magnetar::default_schema::two_factor::ActiveModel` · crates/suprnova-magnetar/src/default_schema.rs:100
  - Public fields: `user_id`, `secret`, `recovery_codes`, `enrollment_auth_epoch`, `enrollment_session_id`, `enrollment_expires_at`, `rotation_pending`, `confirmed_at`, `last_used_timestep`, `created_at`, `updated_at`
- [ ] struct `magnetar::default_schema::two_factor::ColumnIter` · crates/suprnova-magnetar/src/default_schema.rs:100
- [ ] struct `magnetar::default_schema::two_factor::Entity` · crates/suprnova-magnetar/src/default_schema.rs:100
- [ ] struct `magnetar::default_schema::two_factor::Model` · crates/suprnova-magnetar/src/default_schema.rs:100
  - Public fields: `user_id`, `secret`, `recovery_codes`, `enrollment_auth_epoch`, `enrollment_session_id`, `enrollment_expires_at`, `rotation_pending`, `confirmed_at`, `last_used_timestep`, `created_at`, `updated_at`
  - [ ] fn `magnetar::default_schema::two_factor::Model::into_ex` · crates/suprnova-magnetar/src/default_schema.rs:100
- [ ] struct `magnetar::default_schema::two_factor::PrimaryKeyIter` · crates/suprnova-magnetar/src/default_schema.rs:100
- [ ] struct `magnetar::default_schema::two_factor::RelationIter` · crates/suprnova-magnetar/src/default_schema.rs:100
- [ ] enum `magnetar::default_schema::two_factor::Column` · crates/suprnova-magnetar/src/default_schema.rs:100
  - Variants: `UserId`, `Secret`, `RecoveryCodes`, `EnrollmentAuthEpoch`, `EnrollmentSessionId`, `EnrollmentExpiresAt`, `RotationPending`, `ConfirmedAt`, `LastUsedTimestep`, `CreatedAt`, `UpdatedAt`
- [ ] enum `magnetar::default_schema::two_factor::PrimaryKey` · crates/suprnova-magnetar/src/default_schema.rs:100
  - Variants: `UserId`
- [ ] enum `magnetar::default_schema::two_factor::Relation` · crates/suprnova-magnetar/src/default_schema.rs:100

### `magnetar::default_schema::users`

- [ ] struct `magnetar::default_schema::users::ActiveModel` · crates/suprnova-magnetar/src/default_schema.rs:34
  - Public fields: `id`, `email`, `name`, `password_hash`, `remember_token`, `email_verified_at`, `locked_at`, `auth_epoch`, `created_at`, `updated_at`
- [ ] struct `magnetar::default_schema::users::ColumnIter` · crates/suprnova-magnetar/src/default_schema.rs:34
- [ ] struct `magnetar::default_schema::users::Entity` · crates/suprnova-magnetar/src/default_schema.rs:34
- [ ] struct `magnetar::default_schema::users::Model` · crates/suprnova-magnetar/src/default_schema.rs:34
  - Public fields: `id`, `email`, `name`, `password_hash`, `remember_token`, `email_verified_at`, `locked_at`, `auth_epoch`, `created_at`, `updated_at`
  - [ ] fn `magnetar::default_schema::users::Model::into_ex` · crates/suprnova-magnetar/src/default_schema.rs:34
- [ ] struct `magnetar::default_schema::users::PrimaryKeyIter` · crates/suprnova-magnetar/src/default_schema.rs:34
- [ ] struct `magnetar::default_schema::users::RelationIter` · crates/suprnova-magnetar/src/default_schema.rs:34
- [ ] enum `magnetar::default_schema::users::Column` · crates/suprnova-magnetar/src/default_schema.rs:34
  - Variants: `Id`, `Email`, `Name`, `PasswordHash`, `RememberToken`, `EmailVerifiedAt`, `LockedAt`, `AuthEpoch`, `CreatedAt`, `UpdatedAt`
- [ ] enum `magnetar::default_schema::users::PrimaryKey` · crates/suprnova-magnetar/src/default_schema.rs:34
  - Variants: `Id`
- [ ] enum `magnetar::default_schema::users::Relation` · crates/suprnova-magnetar/src/default_schema.rs:34

## drivers

### `magnetar::drivers::redis_abuse` (feature: `redis`, off by default)

- [ ] struct `magnetar::drivers::redis_abuse::RedisAbuseLimiter` · crates/suprnova-magnetar/src/drivers/redis_abuse.rs:49
  - [ ] fn `magnetar::drivers::redis_abuse::RedisAbuseLimiter::new` · crates/suprnova-magnetar/src/drivers/redis_abuse.rs:56
  - [ ] fn `magnetar::drivers::redis_abuse::RedisAbuseLimiter::redis_key_for` · crates/suprnova-magnetar/src/drivers/redis_abuse.rs:65
  - [ ] fn `magnetar::drivers::redis_abuse::RedisAbuseLimiter::normalize_identity` · crates/suprnova-magnetar/src/drivers/redis_abuse.rs:88
  - [ ] fn `magnetar::drivers::redis_abuse::RedisAbuseLimiter::acquire_identity` · crates/suprnova-magnetar/src/drivers/redis_abuse.rs:93
- [ ] struct `magnetar::drivers::redis_abuse::RedisConnection` · crates/suprnova-magnetar/src/drivers/redis_abuse.rs:142
  - [ ] fn `magnetar::drivers::redis_abuse::RedisConnection::new` · crates/suprnova-magnetar/src/drivers/redis_abuse.rs:150
  - [ ] fn `magnetar::drivers::redis_abuse::RedisConnection::connect` · crates/suprnova-magnetar/src/drivers/redis_abuse.rs:154
- [ ] struct `magnetar::drivers::redis_abuse::RedisPermitState` · crates/suprnova-magnetar/src/drivers/redis_abuse.rs:30
  - Public fields: `count`, `remaining`
- [ ] trait `magnetar::drivers::redis_abuse::RedisAbuseConnection` · crates/suprnova-magnetar/src/drivers/redis_abuse.rs:39
  - Implemented here by: `drivers::redis_abuse::RedisConnection`
  - [ ] fn `magnetar::drivers::redis_abuse::RedisAbuseConnection::increment_window` · crates/suprnova-magnetar/src/drivers/redis_abuse.rs:41 (required)

## error

### `magnetar::error`

- [ ] enum `magnetar::Error` · crates/suprnova-magnetar/src/error.rs:7 (also `magnetar::error::Error`)
  - Variants: `InvalidInput`, `NotFound`, `Conflict`, `DependencyUnavailable`, `Internal`
- [ ] type `magnetar::Result` · crates/suprnova-magnetar/src/error.rs:69 (also `magnetar::error::Result`)

## first_email_proof

### `magnetar::first_email_proof`

- [ ] struct `magnetar::first_email_proof::FirstEmailProofCommit` · crates/suprnova-magnetar/src/first_email_proof.rs:81
  - Public fields: `user_id`, `kind`, `first_proof`, `auth_epoch`, `provider_account_id`, `revoked_sessions`, `revoked_remember_rows`
- [ ] struct `magnetar::first_email_proof::NewVerifiedProviderAccount` · crates/suprnova-magnetar/src/first_email_proof.rs:126
  - Public fields: `provider`, `provider_account_id`, `email`
- [ ] struct `magnetar::first_email_proof::VerifiedProviderAccountCommit` · crates/suprnova-magnetar/src/first_email_proof.rs:137
  - Public fields: `user_id`, `auth_epoch`
- [ ] enum `magnetar::first_email_proof::FirstEmailProofKind` · crates/suprnova-magnetar/src/first_email_proof.rs:66
  - Variants: `PasswordReset`, `MagicLink`, `OAuthEmailCompletion`
- [ ] enum `magnetar::first_email_proof::FirstEmailProofMutation` · crates/suprnova-magnetar/src/first_email_proof.rs:17
  - Variants: `PasswordReset`, `MagicLink`, `OAuthEmailCompletion`
  - [ ] fn `magnetar::first_email_proof::FirstEmailProofMutation::kind` · crates/suprnova-magnetar/src/first_email_proof.rs:42
- [ ] enum `magnetar::first_email_proof::FirstEmailProofOutcome` · crates/suprnova-magnetar/src/first_email_proof.rs:100
  - Variants: `Committed`, `ExplicitLinkRequired`
  - [ ] fn `magnetar::first_email_proof::FirstEmailProofOutcome::into_commit` · crates/suprnova-magnetar/src/first_email_proof.rs:113
- [ ] trait `magnetar::first_email_proof::FirstEmailProofStore` · crates/suprnova-magnetar/src/first_email_proof.rs:146
  - Implemented here by: `default_first_email_proof::SqlFirstEmailProofStore`
  - [ ] fn `magnetar::first_email_proof::FirstEmailProofStore::apply` · crates/suprnova-magnetar/src/first_email_proof.rs:148 (required)
  - [ ] fn `magnetar::first_email_proof::FirstEmailProofStore::create_verified_provider_account` · crates/suprnova-magnetar/src/first_email_proof.rs:151 (required)

## mail

### `magnetar::mail`

- [ ] fn `magnetar::mail::email_verification` · crates/suprnova-magnetar/src/mail.rs:27
- [ ] fn `magnetar::mail::magic_link` · crates/suprnova-magnetar/src/mail.rs:63
- [ ] fn `magnetar::mail::oauth_email_completion` · crates/suprnova-magnetar/src/mail.rs:76
- [ ] fn `magnetar::mail::password_changed` · crates/suprnova-magnetar/src/mail.rs:53
- [ ] fn `magnetar::mail::password_reset` · crates/suprnova-magnetar/src/mail.rs:40
- [ ] const `magnetar::mail::EMAIL_VERIFICATION` · crates/suprnova-magnetar/src/mail.rs:15
- [ ] const `magnetar::mail::MAGIC_LINK` · crates/suprnova-magnetar/src/mail.rs:21
- [ ] const `magnetar::mail::OAUTH_EMAIL_COMPLETION` · crates/suprnova-magnetar/src/mail.rs:23
- [ ] const `magnetar::mail::PASSWORD_CHANGED` · crates/suprnova-magnetar/src/mail.rs:19
- [ ] const `magnetar::mail::PASSWORD_RESET` · crates/suprnova-magnetar/src/mail.rs:17

## migration

### `magnetar::migration` (feature: `migration`, off by default)

- [ ] struct `magnetar::migration::MigrationEngine` · crates/suprnova-magnetar/src/migration/mod.rs:215
  - [ ] fn `magnetar::migration::MigrationEngine::new` · crates/suprnova-magnetar/src/migration/mod.rs:226
  - [ ] fn `magnetar::migration::MigrationEngine::with_recovery` · crates/suprnova-magnetar/src/migration/mod.rs:235
  - [ ] fn `magnetar::migration::MigrationEngine::dry_run_optional` · crates/suprnova-magnetar/src/migration/mod.rs:241
  - [ ] fn `magnetar::migration::MigrationEngine::apply_mysql` · crates/suprnova-magnetar/src/migration/mod.rs:540
- [ ] struct `magnetar::migration::MySqlMigrationFailure` · crates/suprnova-magnetar/src/migration/mod.rs:197
  - Public fields: `journal`, `error`, `write_barrier_held`
- [ ] struct `magnetar::migration::MySqlMigrationReport` · crates/suprnova-magnetar/src/migration/mod.rs:188
  - Public fields: `migration`, `journal`
- [ ] trait `magnetar::migration::MigrationBindings` · crates/suprnova-magnetar/src/migration/mod.rs:119
  - Implemented here by: `default_migration::DefaultMigrationBindings`
  - [ ] fn `magnetar::migration::MigrationBindings::app_users` · crates/suprnova-magnetar/src/migration/mod.rs:122 (required)
  - [ ] fn `magnetar::migration::MigrationBindings::shares_source_database` · crates/suprnova-magnetar/src/migration/mod.rs:125 (provided)
  - [ ] fn `magnetar::migration::MigrationBindings::app_users_in_source` · crates/suprnova-magnetar/src/migration/mod.rs:130 (provided)
  - [ ] fn `magnetar::migration::MigrationBindings::begin_transaction` · crates/suprnova-magnetar/src/migration/mod.rs:141 (required)
  - [ ] fn `magnetar::migration::MigrationBindings::mark_migration_completed` · crates/suprnova-magnetar/src/migration/mod.rs:148 (provided)
  - [ ] fn `magnetar::migration::MigrationBindings::migration_target_tables` · crates/suprnova-magnetar/src/migration/mod.rs:156 (provided)
- [ ] trait `magnetar::migration::MigrationRecovery` · crates/suprnova-magnetar/src/migration/mod.rs:179
  - Implemented here by: `migration::mysql_swap::MySqlSwapRecovery`
  - [ ] fn `magnetar::migration::MigrationRecovery::abort` · crates/suprnova-magnetar/src/migration/mod.rs:181 (required)
  - [ ] fn `magnetar::migration::MigrationRecovery::restore` · crates/suprnova-magnetar/src/migration/mod.rs:183 (required)
- [ ] trait `magnetar::migration::MigrationRunner` · crates/suprnova-magnetar/src/migration/mod.rs:163
  - Implemented here by: `migration::MigrationEngine`
  - [ ] fn `magnetar::migration::MigrationRunner::detect_shape` · crates/suprnova-magnetar/src/migration/mod.rs:165 (required)
  - [ ] fn `magnetar::migration::MigrationRunner::dry_run` · crates/suprnova-magnetar/src/migration/mod.rs:168 (required)
  - [ ] fn `magnetar::migration::MigrationRunner::apply` · crates/suprnova-magnetar/src/migration/mod.rs:170 (required)
  - [ ] fn `magnetar::migration::MigrationRunner::abort` · crates/suprnova-magnetar/src/migration/mod.rs:172 (required)
  - [ ] fn `magnetar::migration::MigrationRunner::restore` · crates/suprnova-magnetar/src/migration/mod.rs:174 (required)
- [ ] trait `magnetar::migration::MigrationTransaction` · crates/suprnova-magnetar/src/migration/mod.rs:54
  - [ ] fn `magnetar::migration::MigrationTransaction::app_users` · crates/suprnova-magnetar/src/migration/mod.rs:56 (required)
  - [ ] fn `magnetar::migration::MigrationTransaction::import_user` · crates/suprnova-magnetar/src/migration/mod.rs:62 (required)
  - [ ] fn `magnetar::migration::MigrationTransaction::bind_external_identity` · crates/suprnova-magnetar/src/migration/mod.rs:65 (required)
  - [ ] fn `magnetar::migration::MigrationTransaction::import_passkey` · crates/suprnova-magnetar/src/migration/mod.rs:75 (required)
  - [ ] fn `magnetar::migration::MigrationTransaction::import_durable_record` · crates/suprnova-magnetar/src/migration/mod.rs:83 (required)
  - [ ] fn `magnetar::migration::MigrationTransaction::imports_committed` · crates/suprnova-magnetar/src/migration/mod.rs:86 (required)
  - [ ] fn `magnetar::migration::MigrationTransaction::resolved_app_user_id` · crates/suprnova-magnetar/src/migration/mod.rs:89 (required)
  - [ ] fn `magnetar::migration::MigrationTransaction::record_identity_resolution` · crates/suprnova-magnetar/src/migration/mod.rs:96 (required)
  - [ ] fn `magnetar::migration::MigrationTransaction::mark_imports_committed` · crates/suprnova-magnetar/src/migration/mod.rs:104 (required)
  - [ ] fn `magnetar::migration::MigrationTransaction::commit` · crates/suprnova-magnetar/src/migration/mod.rs:107 (required)
  - [ ] fn `magnetar::migration::MigrationTransaction::rollback` · crates/suprnova-magnetar/src/migration/mod.rs:110 (required)

### `magnetar::migration::fingerprint` (feature: `migration`, off by default)

- [ ] struct `magnetar::migration::fingerprint::SourceTableFingerprint` · crates/suprnova-magnetar/src/migration/fingerprint.rs:23
  - Public fields: `table`, `fingerprint`, `schema_digest`
- [ ] struct `magnetar::migration::fingerprint::TableFingerprint` · crates/suprnova-magnetar/src/migration/fingerprint.rs:12
  - Public fields: `row_count`, `fields`, `digest`
  - [ ] fn `magnetar::migration::fingerprint::TableFingerprint::from_rows` · crates/suprnova-magnetar/src/migration/fingerprint.rs:37

### `magnetar::migration::identity_map` (private module; items are public through re-exports)

- [ ] struct `magnetar::migration::AppUser` · crates/suprnova-magnetar/src/migration/identity_map.rs:12 (feature: `migration`, off by default)
  - Public fields: `id`, `email`, `auth_epoch`, `session_version`
- [ ] struct `magnetar::migration::ExternalIdentity` · crates/suprnova-magnetar/src/migration/identity_map.rs:25 (feature: `migration`, off by default)
  - Public fields: `provider`, `external_user_id`, `app_user_id`
- [ ] struct `magnetar::migration::IdentityMapPlan` · crates/suprnova-magnetar/src/migration/identity_map.rs:83 (feature: `migration`, off by default)
  - Public fields: `entries`
  - [ ] fn `magnetar::migration::IdentityMapPlan::existing_app_user_ids` · crates/suprnova-magnetar/src/migration/identity_map.rs:90
  - [ ] fn `magnetar::migration::IdentityMapPlan::pending_creates` · crates/suprnova-magnetar/src/migration/identity_map.rs:101
- [ ] struct `magnetar::migration::ImportedPasskey` · crates/suprnova-magnetar/src/migration/identity_map.rs:36 (feature: `migration`, off by default)
  - Public fields: `app_user_id`, `credential_id`, `data_json`
- [ ] enum `magnetar::migration::IdentityMapEntry` · crates/suprnova-magnetar/src/migration/identity_map.rs:47 (feature: `migration`, off by default)
  - Variants: `Existing`, `Create`
  - [ ] fn `magnetar::migration::IdentityMapEntry::source_user_id` · crates/suprnova-magnetar/src/migration/identity_map.rs:72

### `magnetar::migration::mysql_swap` (feature: `migration`, off by default)

- [ ] struct `magnetar::migration::mysql_swap::MySqlShadowSwap` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:204
  - [ ] fn `magnetar::migration::mysql_swap::MySqlShadowSwap::resume` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:342
  - [ ] fn `magnetar::migration::mysql_swap::MySqlShadowSwap::abort` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:460
  - [ ] fn `magnetar::migration::mysql_swap::MySqlShadowSwap::restore` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:480
- [ ] struct `magnetar::migration::mysql_swap::MySqlSwapRecovery` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:146
  - [ ] fn `magnetar::migration::mysql_swap::MySqlSwapRecovery::new` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:153
  - [ ] fn `magnetar::migration::mysql_swap::MySqlSwapRecovery::journal` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:163
- [ ] struct `magnetar::migration::mysql_swap::RenameJournalEntry` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:73
  - Public fields: `from`, `to`, `state`
- [ ] struct `magnetar::migration::RestoreReport` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:103 (also `magnetar::migration::mysql_swap::RestoreReport`)
  - Public fields: `restored_tables`
- [ ] struct `magnetar::migration::mysql_swap::SwapFailure` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:110
  - Public fields: `journal`, `error`
- [ ] struct `magnetar::migration::SwapJournal` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:84 (also `magnetar::migration::mysql_swap::SwapJournal`)
  - Public fields: `tables`, `source_fingerprints`, `source_schema_digests`, `prepared_shadows`, `renames`, `cleanup`, `phase`
- [ ] struct `magnetar::migration::SwapTable` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:15 (also `magnetar::migration::mysql_swap::SwapTable`)
  - Public fields: `active`, `shadow`, `backup`
  - [ ] fn `magnetar::migration::SwapTable::new` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:26
- [ ] enum `magnetar::migration::mysql_swap::JournalPhase` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:53
  - Variants: `Preparing`, `Prepared`, `CuttingOver`, `Complete`, `Restoring`, `Restored`, `Aborted`
- [ ] enum `magnetar::migration::mysql_swap::RenameState` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:37
  - Variants: `Pending`, `Prepared`, `Completed`, `RestorePrepared`, `Restored`
- [ ] trait `magnetar::migration::mysql_swap::MySqlSwapBackend` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:120
  - [ ] fn `magnetar::migration::mysql_swap::MySqlSwapBackend::acquire_write_barrier` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:122 (required)
  - [ ] fn `magnetar::migration::mysql_swap::MySqlSwapBackend::write_barrier_held` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:124 (required)
  - [ ] fn `magnetar::migration::mysql_swap::MySqlSwapBackend::release_write_barrier` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:126 (required)
  - [ ] fn `magnetar::migration::mysql_swap::MySqlSwapBackend::copy_to_shadow` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:128 (required)
  - [ ] fn `magnetar::migration::mysql_swap::MySqlSwapBackend::fingerprint` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:130 (required)
  - [ ] fn `magnetar::migration::mysql_swap::MySqlSwapBackend::schema_digest` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:132 (required)
  - [ ] fn `magnetar::migration::mysql_swap::MySqlSwapBackend::table_exists` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:134 (required)
  - [ ] fn `magnetar::migration::mysql_swap::MySqlSwapBackend::rename` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:136 (required)
  - [ ] fn `magnetar::migration::mysql_swap::MySqlSwapBackend::remove_shadow` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:138 (required)
  - [ ] fn `magnetar::migration::mysql_swap::MySqlSwapBackend::apply_cleanup` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:140 (required)
  - [ ] fn `magnetar::migration::mysql_swap::MySqlSwapBackend::persist_journal` · crates/suprnova-magnetar/src/migration/mysql_swap.rs:142 (required)

### `magnetar::migration::plan` (private module; items are public through re-exports)

- [ ] struct `magnetar::migration::FieldMapping` · crates/suprnova-magnetar/src/migration/plan.rs:83 (feature: `migration`, off by default)
  - Public fields: `source`, `destination`, `semantics`
- [ ] struct `magnetar::migration::MigrationPlan` · crates/suprnova-magnetar/src/migration/plan.rs:147 (feature: `migration`, off by default)
  - Public fields: `plan_id`, `source`, `confirmation`, `normalized_collisions`, `warnings`, `source_row_counts`, `source_fingerprints`, `field_mappings`, `backend_strategy`, `table_operations`, `identity_map`, `sacrificeable_cleanup`
- [ ] struct `magnetar::migration::MigrationReport` · crates/suprnova-magnetar/src/migration/plan.rs:423 (feature: `migration`, off by default)
  - Public fields: `source`, `identity_mappings`, `cleanup`, `cleanup_statements`
- [ ] struct `magnetar::migration::SacrificeableCleanup` · crates/suprnova-magnetar/src/migration/plan.rs:34 (feature: `migration`, off by default)
  - Public fields: `targets`
  - [ ] fn `magnetar::migration::SacrificeableCleanup::invalidates` · crates/suprnova-magnetar/src/migration/plan.rs:65
- [ ] struct `magnetar::migration::SourceRowCount` · crates/suprnova-magnetar/src/migration/plan.rs:74 (feature: `migration`, off by default)
  - Public fields: `table`, `rows`
- [ ] struct `magnetar::migration::TableOperation` · crates/suprnova-magnetar/src/migration/plan.rs:11 (feature: `migration`, off by default)
  - Public fields: `target`, `kind`
- [ ] enum `magnetar::migration::BackendStrategy` · crates/suprnova-magnetar/src/migration/plan.rs:103 (feature: `migration`, off by default)
  - Variants: `Transactional`, `MySqlShadowSwap`, `Unsupported`
  - [ ] fn `magnetar::migration::BackendStrategy::for_backend` · crates/suprnova-magnetar/src/migration/plan.rs:126
- [ ] enum `magnetar::migration::MigrationBackend` · crates/suprnova-magnetar/src/migration/plan.rs:94 (feature: `migration`, off by default)
  - Variants: `Sqlite`, `Postgres`
- [ ] enum `magnetar::migration::TableOperationKind` · crates/suprnova-magnetar/src/migration/plan.rs:20 (feature: `migration`, off by default)
  - Variants: `Import`, `Preserve`, `Invalidate`

### `magnetar::migration::preflight` (private module; items are public through re-exports)

- [ ] struct `magnetar::migration::CollisionGroup` · crates/suprnova-magnetar/src/migration/preflight.rs:27 (feature: `migration`, off by default)
  - Public fields: `normalized_email`, `owners`
- [ ] struct `magnetar::migration::CollisionOwner` · crates/suprnova-magnetar/src/migration/preflight.rs:16 (feature: `migration`, off by default)
  - Public fields: `table`, `primary_key`, `email`

### `magnetar::migration::records` (private module; items are public through re-exports)

- [ ] struct `magnetar::migration::ImportedFailedLoginAttempt` · crates/suprnova-magnetar/src/migration/records.rs:68 (feature: `migration`, off by default)
  - Public fields: `source_record_id`, `email`, `ip_address`, `attempted_at`
- [ ] struct `magnetar::migration::ImportedLinkedAccount` · crates/suprnova-magnetar/src/migration/records.rs:34 (feature: `migration`, off by default)
  - Public fields: `app_user_id`, `provider`, `subject`, `created_at`, `updated_at`
- [ ] struct `magnetar::migration::ImportedSecureToken` · crates/suprnova-magnetar/src/migration/records.rs:49 (feature: `migration`, off by default)
  - Public fields: `app_user_id`, `token`, `purpose`, `used_at`, `expires_at`, `created_at`, `updated_at`
- [ ] struct `magnetar::migration::ImportedTwoFactorCredential` · crates/suprnova-magnetar/src/migration/records.rs:81 (feature: `migration`, off by default)
  - Public fields: `app_user_id`, `secret`, `confirmed_at`, `recovery_codes`, `last_used_timestep`, `created_at`, `updated_at`
- [ ] struct `magnetar::migration::ImportedUser` · crates/suprnova-magnetar/src/migration/records.rs:7 (feature: `migration`, off by default)
  - Public fields: `source_user_id`, `preferred_app_user_id`, `email`, `name`, `password_hash`, `email_verified_at`, `locked_at`, `created_at`, `updated_at`, `auth_epoch`, `session_version`
- [ ] enum `magnetar::migration::DurableAuthRecord` · crates/suprnova-magnetar/src/migration/records.rs:101 (feature: `migration`, off by default)
  - Variants: `LinkedAccount`, `SecureToken`, `FailedLoginAttempt`, `TwoFactorCredential`

### `magnetar::migration::schema_guards` (feature: `migration`, off by default)

- [ ] fn `magnetar::migration::schema_guards::create_index_if_missing` · crates/suprnova-magnetar/src/migration/schema_guards.rs:141
- [ ] fn `magnetar::migration::schema_guards::has_column` · crates/suprnova-magnetar/src/migration/schema_guards.rs:50
- [ ] fn `magnetar::migration::schema_guards::has_columns` · crates/suprnova-magnetar/src/migration/schema_guards.rs:88
- [ ] fn `magnetar::migration::schema_guards::has_index` · crates/suprnova-magnetar/src/migration/schema_guards.rs:103
- [ ] fn `magnetar::migration::schema_guards::has_table` · crates/suprnova-magnetar/src/migration/schema_guards.rs:21
- [ ] struct `magnetar::migration::schema_guards::GuardReport` · crates/suprnova-magnetar/src/migration/schema_guards.rs:15
  - Public fields: `statements`

### `magnetar::migration::shape` (private module; items are public through re-exports)

- [ ] struct `magnetar::migration::ShapeConfirmation` · crates/suprnova-magnetar/src/migration/shape.rs:58 (feature: `migration`, off by default)
  - Public fields: `detected`, `operator_selected`
- [ ] enum `magnetar::migration::SourceShape` · crates/suprnova-magnetar/src/migration/shape.rs:9 (feature: `migration`, off by default)
  - Variants: `Torii`, `SuprnovaWeb`, `SuprnovaApi`, `Magnetar`
  - [ ] fn `magnetar::migration::SourceShape::cli_value` · crates/suprnova-magnetar/src/migration/shape.rs:22
  - [ ] fn `magnetar::migration::SourceShape::parse_cli` · crates/suprnova-magnetar/src/migration/shape.rs:32

### `magnetar::migration::upgrade_guide` (feature: `migration`, off by default)

- [ ] struct `magnetar::migration::upgrade_guide::UpgradeGuide` · crates/suprnova-magnetar/src/migration/upgrade_guide.rs:7
  - [ ] fn `magnetar::migration::upgrade_guide::UpgradeGuide::confirmation_flag` · crates/suprnova-magnetar/src/migration/upgrade_guide.rs:11
  - [ ] fn `magnetar::migration::upgrade_guide::UpgradeGuide::supported_source_shapes` · crates/suprnova-magnetar/src/migration/upgrade_guide.rs:16
  - [ ] fn `magnetar::migration::upgrade_guide::UpgradeGuide::confirmation_argument` · crates/suprnova-magnetar/src/migration/upgrade_guide.rs:26
  - [ ] fn `magnetar::migration::upgrade_guide::UpgradeGuide::preflight_warning` · crates/suprnova-magnetar/src/migration/upgrade_guide.rs:31

## oauth

### `magnetar::oauth::authorization` (feature: `oauth`, off by default)

- [ ] struct `magnetar::oauth::OAuthAuthorizationConfig` · crates/suprnova-magnetar/src/oauth/authorization.rs:138 (also `magnetar::oauth::authorization::OAuthAuthorizationConfig`)
  - Public fields: `begin_policy`
- [ ] struct `magnetar::oauth::OAuthAuthorizationService` · crates/suprnova-magnetar/src/oauth/authorization.rs:201 (also `magnetar::oauth::authorization::OAuthAuthorizationService`)
  - [ ] fn `magnetar::oauth::OAuthAuthorizationService::new` · crates/suprnova-magnetar/src/oauth/authorization.rs:211
  - [ ] fn `magnetar::oauth::OAuthAuthorizationService::begin` · crates/suprnova-magnetar/src/oauth/authorization.rs:244
  - [ ] fn `magnetar::oauth::OAuthAuthorizationService::complete` · crates/suprnova-magnetar/src/oauth/authorization.rs:336
- [ ] struct `magnetar::oauth::OAuthBeginInput` · crates/suprnova-magnetar/src/oauth/authorization.rs:73 (also `magnetar::oauth::authorization::OAuthBeginInput`)
  - Public fields: `provider`, `intent`, `actor`, `binding`
- [ ] struct `magnetar::oauth::OAuthBegun` · crates/suprnova-magnetar/src/oauth/authorization.rs:123 (also `magnetar::oauth::authorization::OAuthBegun`)
  - Public fields: `selector`, `code_challenge`, `nonce`
- [ ] struct `magnetar::oauth::OAuthCallbackInput` · crates/suprnova-magnetar/src/oauth/authorization.rs:85 (also `magnetar::oauth::authorization::OAuthCallbackInput`)
  - Public fields: `state`, `provider`, `host_session_digest`
- [ ] struct `magnetar::oauth::OAuthCeremony` · crates/suprnova-magnetar/src/oauth/authorization.rs:99 (also `magnetar::oauth::authorization::OAuthCeremony`)
  - Public fields: `selector`, `provider`, `verifier`, `nonce`, `intent`, `actor`, `binding`
- [ ] enum `magnetar::oauth::CeremonyBinding` · crates/suprnova-magnetar/src/oauth/authorization.rs:49 (also `magnetar::oauth::authorization::CeremonyBinding`)
  - Variants: `HostSessionDigest`, `StateOnly`
- [ ] enum `magnetar::oauth::OAuthIntent` · crates/suprnova-magnetar/src/oauth/authorization.rs:61 (also `magnetar::oauth::authorization::OAuthIntent`)
  - Variants: `SignIn`, `Link`
- [ ] const `magnetar::oauth::OAUTH_AUTHORIZATION_KIND` · crates/suprnova-magnetar/src/oauth/authorization.rs:38 (also `magnetar::oauth::authorization::OAUTH_AUTHORIZATION_KIND`)
- [ ] const `magnetar::oauth::OAUTH_BEGIN_PURPOSE` · crates/suprnova-magnetar/src/oauth/authorization.rs:45 (also `magnetar::oauth::authorization::OAUTH_BEGIN_PURPOSE`)
- [ ] const `magnetar::oauth::OAUTH_STATE_TTL` · crates/suprnova-magnetar/src/oauth/authorization.rs:42 (also `magnetar::oauth::authorization::OAUTH_STATE_TTL`)

### `magnetar::oauth::device` (feature: `oauth`, off by default; `device-authorization`, off by default)

- [ ] struct `magnetar::oauth::device::DeviceAuthorizationConfig` · crates/suprnova-magnetar/src/oauth/device.rs:108
  - Public fields: `code_ttl`, `poll_interval`, `verification_uri`, `poll_abuse_policy`
- [ ] struct `magnetar::oauth::device::DeviceAuthorizationService` · crates/suprnova-magnetar/src/oauth/device.rs:169
  - [ ] fn `magnetar::oauth::device::DeviceAuthorizationService::new` · crates/suprnova-magnetar/src/oauth/device.rs:187
  - [ ] fn `magnetar::oauth::device::DeviceAuthorizationService::issue_code` · crates/suprnova-magnetar/src/oauth/device.rs:210
  - [ ] fn `magnetar::oauth::device::DeviceAuthorizationService::verify` · crates/suprnova-magnetar/src/oauth/device.rs:266
  - [ ] fn `magnetar::oauth::device::DeviceAuthorizationService::approve` · crates/suprnova-magnetar/src/oauth/device.rs:290
  - [ ] fn `magnetar::oauth::device::DeviceAuthorizationService::complete_approval` · crates/suprnova-magnetar/src/oauth/device.rs:349
  - [ ] fn `magnetar::oauth::device::DeviceAuthorizationService::deny` · crates/suprnova-magnetar/src/oauth/device.rs:438
  - [ ] fn `magnetar::oauth::device::DeviceAuthorizationService::poll` · crates/suprnova-magnetar/src/oauth/device.rs:460
- [ ] struct `magnetar::oauth::device::DeviceCodeResponse` · crates/suprnova-magnetar/src/oauth/device.rs:39
  - Public fields: `device_code`, `user_code`, `verification_uri`, `verification_uri_complete`, `expires_in`, `interval`
- [ ] struct `magnetar::oauth::device::DeviceDisplay` · crates/suprnova-magnetar/src/oauth/device.rs:69
  - Public fields: `status`, `expires_at`
- [ ] enum `magnetar::oauth::device::DeviceApprovalOutcome` · crates/suprnova-magnetar/src/oauth/device.rs:78
  - Variants: `Approved`, `FactorRequired`
- [ ] enum `magnetar::oauth::device::DeviceCeremonyStatus` · crates/suprnova-magnetar/src/oauth/device.rs:56
  - Variants: `Pending`, `Approved`, `Denied`, `Issued`
- [ ] enum `magnetar::oauth::device::DevicePollOutcome` · crates/suprnova-magnetar/src/oauth/device.rs:90
  - Variants: `AuthorizationPending`, `SlowDown`, `AccessDenied`, `ExpiredToken`, `Success`

### `magnetar::oauth::email_completion` (feature: `oauth`, off by default)

- [ ] struct `magnetar::oauth::EmailCompletionConfig` · crates/suprnova-magnetar/src/oauth/email_completion.rs:74 (also `magnetar::oauth::email_completion::EmailCompletionConfig`)
  - Public fields: `resend_policy`
- [ ] struct `magnetar::oauth::EmailCompletionService` · crates/suprnova-magnetar/src/oauth/email_completion.rs:92 (also `magnetar::oauth::email_completion::EmailCompletionService`)
  - [ ] fn `magnetar::oauth::EmailCompletionService::new` · crates/suprnova-magnetar/src/oauth/email_completion.rs:107
  - [ ] fn `magnetar::oauth::EmailCompletionService::request` · crates/suprnova-magnetar/src/oauth/email_completion.rs:140
  - [ ] fn `magnetar::oauth::EmailCompletionService::resend` · crates/suprnova-magnetar/src/oauth/email_completion.rs:159
  - [ ] fn `magnetar::oauth::EmailCompletionService::consume` · crates/suprnova-magnetar/src/oauth/email_completion.rs:260
- [ ] const `magnetar::oauth::OAUTH_EMAIL_COMPLETION_PURPOSE` · crates/suprnova-magnetar/src/oauth/email_completion.rs:47 (also `magnetar::oauth::email_completion::OAUTH_EMAIL_COMPLETION_PURPOSE`)
- [ ] const `magnetar::oauth::OAUTH_EMAIL_COMPLETION_TTL` · crates/suprnova-magnetar/src/oauth/email_completion.rs:50 (also `magnetar::oauth::email_completion::OAUTH_EMAIL_COMPLETION_TTL`)

### `magnetar::oauth::errors` (feature: `oauth`, off by default)

- [ ] struct `magnetar::oauth::OAuthErrorTraceContext` · crates/suprnova-magnetar/src/oauth/errors.rs:41 (also `magnetar::oauth::errors::OAuthErrorTraceContext`)
  - Public fields: `class`, `provider`, `grant`, `ceremony_kind`, `correlation_id`
- [ ] enum `magnetar::oauth::OAuthErrorClass` · crates/suprnova-magnetar/src/oauth/errors.rs:15 (also `magnetar::oauth::errors::OAuthErrorClass`)
  - Variants: `ClientError`, `IdentityError`, `UpstreamError`, `ServerError`
  - [ ] fn `magnetar::oauth::OAuthErrorClass::status` · crates/suprnova-magnetar/src/oauth/errors.rs:29
- [ ] enum `magnetar::oauth::OAuthProtocolError` · crates/suprnova-magnetar/src/oauth/errors.rs:56 (also `magnetar::oauth::errors::OAuthProtocolError`)
  - Variants: `InvalidRequestShape`, `MalformedTokenResponse`, `MalformedProviderResponse`, `ProviderReportedError`, `IdentityVerificationFailed`, `UpstreamUnavailable`, `ProviderConfiguration`
  - [ ] fn `magnetar::oauth::OAuthProtocolError::class` · crates/suprnova-magnetar/src/oauth/errors.rs:113
  - [ ] fn `magnetar::oauth::OAuthProtocolError::provider` · crates/suprnova-magnetar/src/oauth/errors.rs:126
  - [ ] fn `magnetar::oauth::OAuthProtocolError::trace_context` · crates/suprnova-magnetar/src/oauth/errors.rs:138
- [ ] type `magnetar::oauth::OAuthResult` · crates/suprnova-magnetar/src/oauth/errors.rs:10 (also `magnetar::oauth::errors::OAuthResult`)

### `magnetar::oauth::grants::authorization_code` (feature: `oauth`, off by default)

- [ ] fn `magnetar::oauth::grants::authorization_code::execute` · crates/suprnova-magnetar/src/oauth/grants/authorization_code.rs:60
- [ ] fn `magnetar::oauth::grants::authorization_code::execute_with_raw` · crates/suprnova-magnetar/src/oauth/grants/authorization_code.rs:87
- [ ] struct `magnetar::oauth::grants::authorization_code::AuthorizationCodeResult` · crates/suprnova-magnetar/src/oauth/grants/authorization_code.rs:19
  - Public fields: `response`
  - [ ] fn `magnetar::oauth::grants::authorization_code::AuthorizationCodeResult::raw_body` · crates/suprnova-magnetar/src/oauth/grants/authorization_code.rs:32

### `magnetar::oauth::grants::client_credentials` (feature: `oauth`, off by default)

- [ ] fn `magnetar::oauth::grants::client_credentials::execute` · crates/suprnova-magnetar/src/oauth/grants/client_credentials.rs:42

### `magnetar::oauth::grants::jwt_bearer` (feature: `oauth`, off by default)

- [ ] fn `magnetar::oauth::grants::jwt_bearer::execute` · crates/suprnova-magnetar/src/oauth/grants/jwt_bearer.rs:63
- [ ] struct `magnetar::oauth::grants::jwt_bearer::JwtBearerAssertion` · crates/suprnova-magnetar/src/oauth/grants/jwt_bearer.rs:23
  - Public fields: `issuer`, `subject`, `audience`, `key_id`
- [ ] struct `magnetar::oauth::grants::jwt_bearer::JwtBearerSigningKey` · crates/suprnova-magnetar/src/oauth/grants/jwt_bearer.rs:39
  - Public fields: `algorithm`, `encoding_key`

### `magnetar::oauth::grants::refresh` (feature: `oauth`, off by default)

- [ ] fn `magnetar::oauth::grants::refresh::execute` · crates/suprnova-magnetar/src/oauth/grants/refresh.rs:65

### `magnetar::oauth::grants::revocation` (feature: `oauth`, off by default)

- [ ] fn `magnetar::oauth::grants::revocation::execute` · crates/suprnova-magnetar/src/oauth/grants/revocation.rs:24

### `magnetar::oauth::identity` (feature: `oauth`, off by default)

- [ ] struct `magnetar::oauth::IdentityResolver` · crates/suprnova-magnetar/src/oauth/identity.rs:182 (also `magnetar::oauth::identity::IdentityResolver`)
  - [ ] fn `magnetar::oauth::IdentityResolver::new` · crates/suprnova-magnetar/src/oauth/identity.rs:194
  - [ ] fn `magnetar::oauth::IdentityResolver::resolve` · crates/suprnova-magnetar/src/oauth/identity.rs:222
- [ ] struct `magnetar::oauth::VerifiedProviderIdentity` · crates/suprnova-magnetar/src/oauth/identity.rs:55 (also `magnetar::oauth::identity::VerifiedProviderIdentity`)
  - Public fields: `provider`, `subject`, `email`, `email_verified`, `display_name`
- [ ] enum `magnetar::oauth::AutoLinkPolicy` · crates/suprnova-magnetar/src/oauth/identity.rs:76 (also `magnetar::oauth::identity::AutoLinkPolicy`)
  - Variants: `ExplicitLinkRequired`, `AutoLink`
- [ ] enum `magnetar::oauth::IdentityOutcome` · crates/suprnova-magnetar/src/oauth/identity.rs:85 (also `magnetar::oauth::identity::IdentityOutcome`)
  - Variants: `SignIn`, `Create`, `Link`, `ExplicitLinkRequired`, `EmailCompletionRequired`
- [ ] const `magnetar::oauth::OAUTH_PENDING_IDENTITY_KIND` · crates/suprnova-magnetar/src/oauth/identity.rs:44 (also `magnetar::oauth::identity::OAUTH_PENDING_IDENTITY_KIND`)
- [ ] const `magnetar::oauth::identity::OAUTH_PENDING_IDENTITY_TTL` · crates/suprnova-magnetar/src/oauth/identity.rs:49

### `magnetar::oauth::protocol` (feature: `oauth`, off by default)

- [ ] fn `magnetar::oauth::parse_token_response_body` · crates/suprnova-magnetar/src/oauth/protocol.rs:162 (also `magnetar::oauth::protocol::parse_token_response_body`)
- [ ] struct `magnetar::oauth::TokenErrorResponse` · crates/suprnova-magnetar/src/oauth/protocol.rs:52 (also `magnetar::oauth::protocol::TokenErrorResponse`)
  - Public fields: `error`, `error_description`, `error_uri`
- [ ] struct `magnetar::oauth::TokenSuccessResponse` · crates/suprnova-magnetar/src/oauth/protocol.rs:30 (also `magnetar::oauth::protocol::TokenSuccessResponse`)
  - Public fields: `access_token`, `token_type`, `expires_in`, `refresh_token`, `id_token`, `scope`
- [ ] enum `magnetar::oauth::OAuthErrorCode` · crates/suprnova-magnetar/src/oauth/protocol.rs:65 (also `magnetar::oauth::protocol::OAuthErrorCode`)
  - Variants: `InvalidRequest`, `InvalidClient`, `InvalidGrant`, `UnauthorizedClient`, `UnsupportedGrantType`, `InvalidScope`, `Unknown`
  - [ ] fn `magnetar::oauth::OAuthErrorCode::wire_str` · crates/suprnova-magnetar/src/oauth/protocol.rs:123
- [ ] enum `magnetar::oauth::TokenResponseBody` · crates/suprnova-magnetar/src/oauth/protocol.rs:148 (also `magnetar::oauth::protocol::TokenResponseBody`)
  - Variants: `Error`, `Success`

### `magnetar::oauth::provider` (feature: `oauth`, off by default)

- [ ] struct `magnetar::oauth::ClientAuthenticationMaterial` · crates/suprnova-magnetar/src/oauth/provider.rs:268 (also `magnetar::oauth::provider::ClientAuthenticationMaterial`)
  - Public fields: `params`, `headers`
- [ ] struct `magnetar::oauth::EndpointOverrides` · crates/suprnova-magnetar/src/oauth/provider.rs:64 (also `magnetar::oauth::provider::EndpointOverrides`)
  - Public fields: `authorization_endpoint`, `token_endpoint`, `userinfo_endpoint`, `revocation_endpoint`, `device_authorization_endpoint`, `device_token_endpoint`
- [ ] struct `magnetar::oauth::OAuthProviderRegistry` · crates/suprnova-magnetar/src/oauth/provider.rs:370 (also `magnetar::oauth::provider::OAuthProviderRegistry`)
  - [ ] fn `magnetar::oauth::OAuthProviderRegistry::new` · crates/suprnova-magnetar/src/oauth/provider.rs:377
  - [ ] fn `magnetar::oauth::OAuthProviderRegistry::register` · crates/suprnova-magnetar/src/oauth/provider.rs:390
  - [ ] fn `magnetar::oauth::OAuthProviderRegistry::get` · crates/suprnova-magnetar/src/oauth/provider.rs:404
  - [ ] fn `magnetar::oauth::OAuthProviderRegistry::names` · crates/suprnova-magnetar/src/oauth/provider.rs:410
- [ ] struct `magnetar::oauth::RefreshPolicy` · crates/suprnova-magnetar/src/oauth/provider.rs:194 (also `magnetar::oauth::provider::RefreshPolicy`)
  - Public fields: `supported`, `token_client_authentication`, `extra_authorization_params`, `required_scopes`, `requires_reconsent_for_reissue`, `invalid_grant_meaning`
- [ ] struct `magnetar::oauth::RevocationRequest` · crates/suprnova-magnetar/src/oauth/provider.rs:231 (also `magnetar::oauth::provider::RevocationRequest`)
  - Public fields: `method`, `endpoint`, `placement`, `params`, `headers`
- [ ] enum `magnetar::oauth::ClientAuthentication` · crates/suprnova-magnetar/src/oauth/provider.rs:149 (also `magnetar::oauth::provider::ClientAuthentication`)
  - Variants: `RequestBody`, `HttpBasic`, `SignedJwt`
- [ ] enum `magnetar::oauth::InvalidGrantMeaning` · crates/suprnova-magnetar/src/oauth/provider.rs:179 (also `magnetar::oauth::provider::InvalidGrantMeaning`)
  - Variants: `ReuseOrExternalRevocation`, `OrdinaryRevocation`
- [ ] enum `magnetar::oauth::ParamPlacement` · crates/suprnova-magnetar/src/oauth/provider.rs:217 (also `magnetar::oauth::provider::ParamPlacement`)
  - Variants: `Body`, `Query`
- [ ] enum `magnetar::oauth::ProviderResponse` · crates/suprnova-magnetar/src/oauth/provider.rs:117 (also `magnetar::oauth::provider::ProviderResponse`)
  - Variants: `UserInfo`, `AppleIdToken`
- [ ] enum `magnetar::oauth::TokenHint` · crates/suprnova-magnetar/src/oauth/provider.rs:87 (also `magnetar::oauth::provider::TokenHint`)
  - Variants: `Access`, `Refresh`
  - [ ] fn `magnetar::oauth::TokenHint::wire_value` · crates/suprnova-magnetar/src/oauth/provider.rs:97
- [ ] trait `magnetar::oauth::OAuthProvider` · crates/suprnova-magnetar/src/oauth/provider.rs:293 (also `magnetar::oauth::provider::OAuthProvider`)
  - Implemented here by: `plugins::oauth_apple::AppleOAuthProvider`, `plugins::oauth_facebook::FacebookOAuthProvider`, `plugins::oauth_google::GoogleOAuthProvider`, `plugins::oauth_tiktok::TikTokOAuthProvider`, `plugins::oauth_x::XOAuthProvider`
  - [ ] fn `magnetar::oauth::OAuthProvider::name` · crates/suprnova-magnetar/src/oauth/provider.rs:296 (required)
  - [ ] fn `magnetar::oauth::OAuthProvider::authorization_shape` · crates/suprnova-magnetar/src/oauth/provider.rs:300 (required)
  - [ ] fn `magnetar::oauth::OAuthProvider::token_shape` · crates/suprnova-magnetar/src/oauth/provider.rs:304 (required)
  - [ ] fn `magnetar::oauth::OAuthProvider::resolve_identity` · crates/suprnova-magnetar/src/oauth/provider.rs:307 (required)
  - [ ] fn `magnetar::oauth::OAuthProvider::revoke` · crates/suprnova-magnetar/src/oauth/provider.rs:310 (required)
  - [ ] fn `magnetar::oauth::OAuthProvider::client_id` · crates/suprnova-magnetar/src/oauth/provider.rs:319 (required)
  - [ ] fn `magnetar::oauth::OAuthProvider::token_endpoint` · crates/suprnova-magnetar/src/oauth/provider.rs:325 (required)
  - [ ] fn `magnetar::oauth::OAuthProvider::authorization_endpoint` · crates/suprnova-magnetar/src/oauth/provider.rs:333 (required)
  - [ ] fn `magnetar::oauth::OAuthProvider::userinfo_endpoint` · crates/suprnova-magnetar/src/oauth/provider.rs:342 (required)
  - [ ] fn `magnetar::oauth::OAuthProvider::userinfo_headers` · crates/suprnova-magnetar/src/oauth/provider.rs:348 (provided)
  - [ ] fn `magnetar::oauth::OAuthProvider::refresh_policy` · crates/suprnova-magnetar/src/oauth/provider.rs:352 (required)
  - [ ] fn `magnetar::oauth::OAuthProvider::client_authentication` · crates/suprnova-magnetar/src/oauth/provider.rs:360 (required)
- [ ] trait `magnetar::oauth::RevocationTransport` · crates/suprnova-magnetar/src/oauth/provider.rs:281 (also `magnetar::oauth::provider::RevocationTransport`)
  - [ ] fn `magnetar::oauth::RevocationTransport::send` · crates/suprnova-magnetar/src/oauth/provider.rs:285 (required)
- [ ] type `magnetar::oauth::ProviderIdentity` · crates/suprnova-magnetar/src/oauth/provider.rs:41 (also `magnetar::oauth::provider::ProviderIdentity`)

### `magnetar::oauth::request_shape` (feature: `oauth`, off by default)

- [ ] fn `magnetar::oauth::render_authorization_request` · crates/suprnova-magnetar/src/oauth/request_shape.rs:175 (also `magnetar::oauth::request_shape::render_authorization_request`)
- [ ] fn `magnetar::oauth::render_token_request` · crates/suprnova-magnetar/src/oauth/request_shape.rs:247 (also `magnetar::oauth::request_shape::render_token_request`)
- [ ] struct `magnetar::oauth::AuthorizationRequestParams` · crates/suprnova-magnetar/src/oauth/request_shape.rs:119 (also `magnetar::oauth::request_shape::AuthorizationRequestParams`)
  - Public fields: `client_id`, `redirect_uri`, `scopes`, `state`, `code_challenge`, `nonce`
- [ ] struct `magnetar::oauth::AuthorizationRequestShape` · crates/suprnova-magnetar/src/oauth/request_shape.rs:39 (also `magnetar::oauth::request_shape::AuthorizationRequestShape`)
  - Public fields: `client_id_param`, `scope_delimiter`, `always_send_scope`, `pkce`, `response_mode`, `requires_nonce`
- [ ] struct `magnetar::oauth::TokenRequestParams` · crates/suprnova-magnetar/src/oauth/request_shape.rs:143 (also `magnetar::oauth::request_shape::TokenRequestParams`)
  - Public fields: `client_id`, `code`, `redirect_uri`, `code_verifier`, `scopes`
- [ ] struct `magnetar::oauth::TokenRequestShape` · crates/suprnova-magnetar/src/oauth/request_shape.rs:81 (also `magnetar::oauth::request_shape::TokenRequestShape`)
  - Public fields: `client_id_param`, `scope_delimiter`, `always_send_scope`, `accept_http_success_error_body`
- [ ] enum `magnetar::oauth::PkcePosture` · crates/suprnova-magnetar/src/oauth/request_shape.rs:26 (also `magnetar::oauth::request_shape::PkcePosture`)
  - Variants: `Required`, `Disabled`

## passkey

### `magnetar::passkey` (feature: `passkey`, off by default)

- [ ] struct `magnetar::passkey::BegunAuthentication` · crates/suprnova-magnetar/src/passkey/mod.rs:92
  - Public fields: `selector`, `options`
- [ ] struct `magnetar::passkey::BegunRegistration` · crates/suprnova-magnetar/src/passkey/mod.rs:83
  - Public fields: `selector`, `options`
- [ ] struct `magnetar::passkey::PasskeyAuthService` · crates/suprnova-magnetar/src/passkey/mod.rs:115
  - [ ] fn `magnetar::passkey::PasskeyAuthService::new` · crates/suprnova-magnetar/src/passkey/mod.rs:127
  - [ ] fn `magnetar::passkey::PasskeyAuthService::begin_registration` · crates/suprnova-magnetar/src/passkey/mod.rs:162
  - [ ] fn `magnetar::passkey::PasskeyAuthService::finish_registration` · crates/suprnova-magnetar/src/passkey/mod.rs:273
  - [ ] fn `magnetar::passkey::PasskeyAuthService::begin_authentication` · crates/suprnova-magnetar/src/passkey/mod.rs:336
  - [ ] fn `magnetar::passkey::PasskeyAuthService::finish_authentication` · crates/suprnova-magnetar/src/passkey/mod.rs:375
  - [ ] fn `magnetar::passkey::PasskeyAuthService::list` · crates/suprnova-magnetar/src/passkey/mod.rs:453
- [ ] struct `magnetar::passkey::PasskeyConfig` · crates/suprnova-magnetar/src/passkey/mod.rs:48
  - Public fields: `rp_id`, `rp_origin`
- [ ] struct `magnetar::passkey::PasskeySummary` · crates/suprnova-magnetar/src/passkey/mod.rs:101
  - Public fields: `passkey_id`, `credential_id`, `name`, `created_at`, `last_used_at`
- [ ] struct `magnetar::passkey::RegistrationIntent` · crates/suprnova-magnetar/src/passkey/mod.rs:69
  - Public fields: `email`, `actor`, `reauthenticated_at`

### `magnetar::passkey::ceremony` (feature: `passkey`, off by default)

- [ ] const `magnetar::passkey::ceremony::AUTHENTICATION_KIND` · crates/suprnova-magnetar/src/passkey/ceremony.rs:37
- [ ] const `magnetar::passkey::ceremony::CEREMONY_TTL_MINUTES` · crates/suprnova-magnetar/src/passkey/ceremony.rs:39
- [ ] const `magnetar::passkey::ceremony::REGISTRATION_KIND` · crates/suprnova-magnetar/src/passkey/ceremony.rs:35

### `magnetar::passkey::envelope` (feature: `passkey`, off by default)

- [ ] struct `magnetar::passkey::envelope::PasskeyEnvelope` · crates/suprnova-magnetar/src/passkey/envelope.rs:21
  - [ ] fn `magnetar::passkey::envelope::PasskeyEnvelope::for_new_credential` · crates/suprnova-magnetar/src/passkey/envelope.rs:28
  - [ ] fn `magnetar::passkey::envelope::PasskeyEnvelope::parse` · crates/suprnova-magnetar/src/passkey/envelope.rs:44
  - [ ] fn `magnetar::passkey::envelope::PasskeyEnvelope::credential_id_b64` · crates/suprnova-magnetar/src/passkey/envelope.rs:53
  - [ ] fn `magnetar::passkey::envelope::PasskeyEnvelope::name` · crates/suprnova-magnetar/src/passkey/envelope.rs:62
  - [ ] fn `magnetar::passkey::envelope::PasskeyEnvelope::last_used_at` · crates/suprnova-magnetar/src/passkey/envelope.rs:68
  - [ ] fn `magnetar::passkey::envelope::PasskeyEnvelope::passkey` · crates/suprnova-magnetar/src/passkey/envelope.rs:73
  - [ ] fn `magnetar::passkey::envelope::PasskeyEnvelope::with_updated_credential` · crates/suprnova-magnetar/src/passkey/envelope.rs:87
  - [ ] fn `magnetar::passkey::envelope::PasskeyEnvelope::to_json` · crates/suprnova-magnetar/src/passkey/envelope.rs:102

## password

### `magnetar::password`

- [ ] fn `magnetar::password::normalize_email` · crates/suprnova-magnetar/src/password/mod.rs:26
- [ ] fn `magnetar::password::validate_password` · crates/suprnova-magnetar/src/password/mod.rs:32

### `magnetar::password::hash`

- [ ] struct `magnetar::password::AttemptVerdict` · crates/suprnova-magnetar/src/password/hash.rs:164 (also `magnetar::password::hash::AttemptVerdict`)
  - Public fields: `valid`, `rehash`
- [ ] struct `magnetar::password::HashWorkProfile` · crates/suprnova-magnetar/src/password/hash.rs:57 (also `magnetar::password::hash::HashWorkProfile`)
  - Public fields: `algorithm`, `parameters`
- [ ] struct `magnetar::password::PasswordHashConfig` · crates/suprnova-magnetar/src/password/hash.rs:96 (also `magnetar::password::hash::PasswordHashConfig`)
  - Public fields: `bcrypt_cost`, `argon2_memory_kib`, `argon2_iterations`, `argon2_parallelism`
  - [ ] fn `magnetar::password::PasswordHashConfig::bcrypt_profile` · crates/suprnova-magnetar/src/password/hash.rs:124
  - [ ] fn `magnetar::password::PasswordHashConfig::argon2_target` · crates/suprnova-magnetar/src/password/hash.rs:135
- [ ] struct `magnetar::password::PasswordVerifier` · crates/suprnova-magnetar/src/password/hash.rs:172 (also `magnetar::password::hash::PasswordVerifier`)
  - [ ] fn `magnetar::password::PasswordVerifier::new` · crates/suprnova-magnetar/src/password/hash.rs:185
  - [ ] fn `magnetar::password::PasswordVerifier::config` · crates/suprnova-magnetar/src/password/hash.rs:199
  - [ ] fn `magnetar::password::PasswordVerifier::verify_attempt` · crates/suprnova-magnetar/src/password/hash.rs:209
  - [ ] fn `magnetar::password::PasswordVerifier::verify_work_only` · crates/suprnova-magnetar/src/password/hash.rs:236
  - [ ] fn `magnetar::password::PasswordVerifier::mint_target` · crates/suprnova-magnetar/src/password/hash.rs:301
- [ ] struct `magnetar::password::StandardPasswordHashDriver` · crates/suprnova-magnetar/src/password/hash.rs:374 (also `magnetar::password::hash::StandardPasswordHashDriver`)
- [ ] struct `magnetar::password::VerificationCall` · crates/suprnova-magnetar/src/password/hash.rs:74 (also `magnetar::password::hash::VerificationCall`)
  - Public fields: `provenance`, `profile`, `password`, `hash`
- [ ] enum `magnetar::password::CallProvenance` · crates/suprnova-magnetar/src/password/hash.rs:66 (also `magnetar::password::hash::CallProvenance`)
  - Variants: `Stored`, `Dummy`
- [ ] enum `magnetar::password::HashAlgorithm` · crates/suprnova-magnetar/src/password/hash.rs:29 (also `magnetar::password::hash::HashAlgorithm`)
  - Variants: `Bcrypt`, `Argon2`
- [ ] enum `magnetar::password::HashParameters` · crates/suprnova-magnetar/src/password/hash.rs:38 (also `magnetar::password::hash::HashParameters`)
  - Variants: `Bcrypt`, `Argon2`
- [ ] enum `magnetar::password::RehashOutcome` · crates/suprnova-magnetar/src/password/hash.rs:149 (also `magnetar::password::hash::RehashOutcome`)
  - Variants: `NotNeeded`, `Upgraded`, `Failed`
- [ ] trait `magnetar::password::PasswordHashDriver` · crates/suprnova-magnetar/src/password/hash.rs:87 (also `magnetar::password::hash::PasswordHashDriver`)
  - Implemented here by: `password::StandardPasswordHashDriver`
  - [ ] fn `magnetar::password::PasswordHashDriver::verify` · crates/suprnova-magnetar/src/password/hash.rs:89 (required)
  - [ ] fn `magnetar::password::PasswordHashDriver::mint` · crates/suprnova-magnetar/src/password/hash.rs:91 (required)
- [ ] const `magnetar::password::hash::MAX_BCRYPT_PASSWORD_BYTES` · crates/suprnova-magnetar/src/password/hash.rs:25

### `magnetar::password::lockout`

- [ ] struct `magnetar::password::AttemptAdmission` · crates/suprnova-magnetar/src/password/lockout.rs:114 (also `magnetar::password::lockout::AttemptAdmission`)
  - Public fields: `admitted`, `status`, `locked_event`
  - [ ] fn `magnetar::password::AttemptAdmission::from_parts` · crates/suprnova-magnetar/src/password/lockout.rs:149
  - [ ] fn `magnetar::password::AttemptAdmission::from_parts_with_event` · crates/suprnova-magnetar/src/password/lockout.rs:164
  - [ ] fn `magnetar::password::AttemptAdmission::reservation` · crates/suprnova-magnetar/src/password/lockout.rs:180
- [ ] struct `magnetar::password::AttemptReservationToken` · crates/suprnova-magnetar/src/password/lockout.rs:130 (also `magnetar::password::lockout::AttemptReservationToken`)
  - [ ] fn `magnetar::password::AttemptReservationToken::new` · crates/suprnova-magnetar/src/password/lockout.rs:138
- [ ] struct `magnetar::password::FailedAttempt` · crates/suprnova-magnetar/src/password/lockout.rs:103 (also `magnetar::password::lockout::FailedAttempt`)
  - Public fields: `status`, `locked_event`
- [ ] struct `magnetar::password::LockoutConfig` · crates/suprnova-magnetar/src/password/lockout.rs:32 (also `magnetar::password::lockout::LockoutConfig`)
  - Public fields: `enabled`, `max_failed_attempts`, `lockout_period`, `retention_period`, `backend_error_policy`
  - [ ] fn `magnetar::password::LockoutConfig::disabled` · crates/suprnova-magnetar/src/password/lockout.rs:62
- [ ] struct `magnetar::password::LockoutService` · crates/suprnova-magnetar/src/password/lockout.rs:186 (also `magnetar::password::lockout::LockoutService`)
  - [ ] fn `magnetar::password::LockoutService::new` · crates/suprnova-magnetar/src/password/lockout.rs:194
  - [ ] fn `magnetar::password::LockoutService::config` · crates/suprnova-magnetar/src/password/lockout.rs:208
  - [ ] fn `magnetar::password::LockoutService::status` · crates/suprnova-magnetar/src/password/lockout.rs:213
  - [ ] fn `magnetar::password::LockoutService::guarded_status` · crates/suprnova-magnetar/src/password/lockout.rs:224
  - [ ] fn `magnetar::password::LockoutService::is_locked` · crates/suprnova-magnetar/src/password/lockout.rs:241
  - [ ] fn `magnetar::password::LockoutService::record_failed_attempt` · crates/suprnova-magnetar/src/password/lockout.rs:247
  - [ ] fn `magnetar::password::LockoutService::admit_attempt` · crates/suprnova-magnetar/src/password/lockout.rs:284
  - [ ] fn `magnetar::password::LockoutService::cancel_attempt` · crates/suprnova-magnetar/src/password/lockout.rs:330
  - [ ] fn `magnetar::password::LockoutService::finalize_failed_attempt` · crates/suprnova-magnetar/src/password/lockout.rs:350
  - [ ] fn `magnetar::password::LockoutService::reset_admitted_attempts` · crates/suprnova-magnetar/src/password/lockout.rs:386
  - [ ] fn `magnetar::password::LockoutService::reset_attempts` · crates/suprnova-magnetar/src/password/lockout.rs:402
  - [ ] fn `magnetar::password::LockoutService::unlock_account` · crates/suprnova-magnetar/src/password/lockout.rs:411
  - [ ] fn `magnetar::password::LockoutService::cleanup_expired_attempts` · crates/suprnova-magnetar/src/password/lockout.rs:420
- [ ] struct `magnetar::password::LockoutStatus` · crates/suprnova-magnetar/src/password/lockout.rs:72 (also `magnetar::password::lockout::LockoutStatus`)
  - Public fields: `identity`, `failed_attempts`, `is_locked`, `locked_until`
  - [ ] fn `magnetar::password::LockoutStatus::retry_after_seconds` · crates/suprnova-magnetar/src/password/lockout.rs:86
- [ ] enum `magnetar::password::BackendErrorPolicy` · crates/suprnova-magnetar/src/password/lockout.rs:22 (also `magnetar::password::lockout::BackendErrorPolicy`)
  - Variants: `FailClosed`, `FailOpen`

## plugin

### `magnetar::plugin::context`

- [ ] struct `magnetar::plugin::BearerCredential` · crates/suprnova-magnetar/src/plugin/context.rs:29 (also `magnetar::plugin::context::BearerCredential`)
  - [ ] fn `magnetar::plugin::BearerCredential::new` · crates/suprnova-magnetar/src/plugin/context.rs:33
- [ ] struct `magnetar::plugin::HookContext` · crates/suprnova-magnetar/src/plugin/context.rs:304 (also `magnetar::plugin::context::HookContext`)
  - Public fields: `plugin`
  - [ ] fn `magnetar::plugin::HookContext::new` · crates/suprnova-magnetar/src/plugin/context.rs:311
- [ ] struct `magnetar::plugin::HttpRequest` · crates/suprnova-magnetar/src/plugin/context.rs:89 (also `magnetar::plugin::context::HttpRequest`)
  - Public fields: `method`, `url`, `headers`, `body`
- [ ] struct `magnetar::plugin::HttpResponse` · crates/suprnova-magnetar/src/plugin/context.rs:129 (also `magnetar::plugin::context::HttpResponse`)
  - Public fields: `status`, `headers`, `body`
- [ ] struct `magnetar::plugin::InitContext` · crates/suprnova-magnetar/src/plugin/context.rs:255 (also `magnetar::plugin::context::InitContext`)
  - Public fields: `plugin`
  - [ ] fn `magnetar::plugin::InitContext::new` · crates/suprnova-magnetar/src/plugin/context.rs:262
- [ ] struct `magnetar::plugin::MailMessage` · crates/suprnova-magnetar/src/plugin/context.rs:64 (also `magnetar::plugin::context::MailMessage`)
  - Public fields: `name`, `recipient`, `payload`
- [ ] struct `magnetar::plugin::PluginContext` · crates/suprnova-magnetar/src/plugin/context.rs:175 (also `magnetar::plugin::context::PluginContext`)
  - [ ] fn `magnetar::plugin::PluginContext::new` · crates/suprnova-magnetar/src/plugin/context.rs:189
  - [ ] fn `magnetar::plugin::PluginContext::storage` · crates/suprnova-magnetar/src/plugin/context.rs:212
  - [ ] fn `magnetar::plugin::PluginContext::sessions` · crates/suprnova-magnetar/src/plugin/context.rs:216
  - [ ] fn `magnetar::plugin::PluginContext::factor_gate` · crates/suprnova-magnetar/src/plugin/context.rs:229
  - [ ] fn `magnetar::plugin::PluginContext::encryptor` · crates/suprnova-magnetar/src/plugin/context.rs:233
  - [ ] fn `magnetar::plugin::PluginContext::abuse_limiter` · crates/suprnova-magnetar/src/plugin/context.rs:237
  - [ ] fn `magnetar::plugin::PluginContext::mail` · crates/suprnova-magnetar/src/plugin/context.rs:241
  - [ ] fn `magnetar::plugin::PluginContext::http` · crates/suprnova-magnetar/src/plugin/context.rs:245
  - [ ] fn `magnetar::plugin::PluginContext::links` · crates/suprnova-magnetar/src/plugin/context.rs:249
- [ ] struct `magnetar::plugin::RequestContext` · crates/suprnova-magnetar/src/plugin/context.rs:268 (also `magnetar::plugin::context::RequestContext`)
  - Public fields: `plugin`, `request`, `session`
  - [ ] fn `magnetar::plugin::RequestContext::new` · crates/suprnova-magnetar/src/plugin/context.rs:279
  - [ ] fn `magnetar::plugin::RequestContext::with_session` · crates/suprnova-magnetar/src/plugin/context.rs:291
- [ ] enum `magnetar::plugin::BeforeRequest` · crates/suprnova-magnetar/src/plugin/context.rs:18 (also `magnetar::plugin::context::BeforeRequest`)
  - Variants: `Continue`, `Respond`, `Bind`
- [ ] trait `magnetar::plugin::AuthStorage` · crates/suprnova-magnetar/src/plugin/context.rs:44 (also `magnetar::plugin::context::AuthStorage`)
- [ ] trait `magnetar::plugin::Encryptor` · crates/suprnova-magnetar/src/plugin/context.rs:55 (also `magnetar::plugin::context::Encryptor`)
  - [ ] fn `magnetar::plugin::Encryptor::encrypt` · crates/suprnova-magnetar/src/plugin/context.rs:57 (required)
  - [ ] fn `magnetar::plugin::Encryptor::decrypt` · crates/suprnova-magnetar/src/plugin/context.rs:59 (required)
- [ ] trait `magnetar::plugin::HttpTransport` · crates/suprnova-magnetar/src/plugin/context.rs:161 (also `magnetar::plugin::context::HttpTransport`)
  - [ ] fn `magnetar::plugin::HttpTransport::send` · crates/suprnova-magnetar/src/plugin/context.rs:163 (required)
- [ ] trait `magnetar::plugin::LinkGenerator` · crates/suprnova-magnetar/src/plugin/context.rs:168 (also `magnetar::plugin::context::LinkGenerator`)
  - [ ] fn `magnetar::plugin::LinkGenerator::url_for` · crates/suprnova-magnetar/src/plugin/context.rs:170 (required)
- [ ] trait `magnetar::plugin::MailDriver` · crates/suprnova-magnetar/src/plugin/context.rs:75 (also `magnetar::plugin::context::MailDriver`)
  - [ ] fn `magnetar::plugin::MailDriver::send` · crates/suprnova-magnetar/src/plugin/context.rs:77 (required)

### `magnetar::plugin::effects`

- [ ] struct `magnetar::plugin::EffectResponse` · crates/suprnova-magnetar/src/plugin/effects.rs:44 (also `magnetar::plugin::effects::EffectResponse`)
  - Public fields: `status`, `headers`, `body`, `effects`
  - [ ] fn `magnetar::plugin::EffectResponse::ok` · crates/suprnova-magnetar/src/plugin/effects.rs:68
  - [ ] fn `magnetar::plugin::EffectResponse::json` · crates/suprnova-magnetar/src/plugin/effects.rs:72
  - [ ] fn `magnetar::plugin::EffectResponse::with_effect` · crates/suprnova-magnetar/src/plugin/effects.rs:79
- [ ] enum `magnetar::plugin::Effect` · crates/suprnova-magnetar/src/plugin/effects.rs:13 (also `magnetar::plugin::effects::Effect`)
  - Variants: `EstablishSession`, `ClearSession`, `IssueRemember`, `Redirect`, `SetStatus`, `SetHeader`, `Json`
- [ ] type `magnetar::plugin::PluginResponse` · crates/suprnova-magnetar/src/plugin/effects.rs:85 (also `magnetar::plugin::effects::PluginResponse`)
- [ ] type `magnetar::plugin::ResponseEffect` · crates/suprnova-magnetar/src/plugin/effects.rs:9 (also `magnetar::plugin::effects::ResponseEffect`)

### `magnetar::plugin::error`

- [ ] enum `magnetar::plugin::PluginError` · crates/suprnova-magnetar/src/plugin/error.rs:10 (also `magnetar::plugin::error::PluginError`)
  - Variants: `InvalidComposition`, `RouteNotFound`, `Request`, `LifecyclePanic`, `Foundation`
- [ ] type `magnetar::plugin::PluginResult` · crates/suprnova-magnetar/src/plugin/error.rs:6 (also `magnetar::plugin::error::PluginResult`)

### `magnetar::plugin::hooks`

- [ ] struct `magnetar::plugin::LifecycleEvent` · crates/suprnova-magnetar/src/plugin/hooks.rs:24 (also `magnetar::plugin::hooks::LifecycleEvent`)
  - Public fields: `mutation_id`, `kind`, `user_id`
  - [ ] fn `magnetar::plugin::LifecycleEvent::new` · crates/suprnova-magnetar/src/plugin/hooks.rs:36
- [ ] enum `magnetar::plugin::LifecycleEventKind` · crates/suprnova-magnetar/src/plugin/hooks.rs:11 (also `magnetar::plugin::hooks::LifecycleEventKind`)
  - Variants: `UserCreated`, `UserDeleted`, `SessionCreated`, `SessionDeleted`
- [ ] trait `magnetar::plugin::DurableLifecycleDelivery` · crates/suprnova-magnetar/src/plugin/hooks.rs:66 (also `magnetar::plugin::hooks::DurableLifecycleDelivery`)
  - [ ] fn `magnetar::plugin::DurableLifecycleDelivery::enqueue` · crates/suprnova-magnetar/src/plugin/hooks.rs:68 (required)
- [ ] trait `magnetar::plugin::LifecycleHook` · crates/suprnova-magnetar/src/plugin/hooks.rs:51 (also `magnetar::plugin::hooks::LifecycleHook`)
  - [ ] fn `magnetar::plugin::LifecycleHook::on_event` · crates/suprnova-magnetar/src/plugin/hooks.rs:53 (required)

### `magnetar::plugin::registry`

- [ ] struct `magnetar::plugin::PluginRegistry` · crates/suprnova-magnetar/src/plugin/registry.rs:71 (also `magnetar::plugin::registry::PluginRegistry`)
  - [ ] fn `magnetar::plugin::PluginRegistry::new` · crates/suprnova-magnetar/src/plugin/registry.rs:79
  - [ ] fn `magnetar::plugin::PluginRegistry::from_plugins` · crates/suprnova-magnetar/src/plugin/registry.rs:87
  - [ ] fn `magnetar::plugin::PluginRegistry::init` · crates/suprnova-magnetar/src/plugin/registry.rs:99
  - [ ] fn `magnetar::plugin::PluginRegistry::before_request` · crates/suprnova-magnetar/src/plugin/registry.rs:107
  - [ ] fn `magnetar::plugin::PluginRegistry::handle` · crates/suprnova-magnetar/src/plugin/registry.rs:119
  - [ ] fn `magnetar::plugin::PluginRegistry::handle_bound` · crates/suprnova-magnetar/src/plugin/registry.rs:124
  - [ ] fn `magnetar::plugin::PluginRegistry::handle_web_binding` · crates/suprnova-magnetar/src/plugin/registry.rs:142
  - [ ] fn `magnetar::plugin::PluginRegistry::dispatch_lifecycle` · crates/suprnova-magnetar/src/plugin/registry.rs:176
  - [ ] fn `magnetar::plugin::PluginRegistry::take_lifecycle_errors` · crates/suprnova-magnetar/src/plugin/registry.rs:210
  - [ ] fn `magnetar::plugin::PluginRegistry::route_names` · crates/suprnova-magnetar/src/plugin/registry.rs:220
- [ ] struct `magnetar::plugin::PluginRegistryBuilder` · crates/suprnova-magnetar/src/plugin/registry.rs:229 (also `magnetar::plugin::registry::PluginRegistryBuilder`)
  - [ ] fn `magnetar::plugin::PluginRegistryBuilder::register` · crates/suprnova-magnetar/src/plugin/registry.rs:236
  - [ ] fn `magnetar::plugin::PluginRegistryBuilder::register_arc` · crates/suprnova-magnetar/src/plugin/registry.rs:245
  - [ ] fn `magnetar::plugin::PluginRegistryBuilder::build` · crates/suprnova-magnetar/src/plugin/registry.rs:251
- [ ] trait `magnetar::plugin::ErasedPluginFacade` · crates/suprnova-magnetar/src/plugin/registry.rs:18 (also `magnetar::plugin::registry::ErasedPluginFacade`)
  - Implemented here by: `plugin::PluginRegistry`
  - [ ] fn `magnetar::plugin::ErasedPluginFacade::before_request` · crates/suprnova-magnetar/src/plugin/registry.rs:20 (required)
  - [ ] fn `magnetar::plugin::ErasedPluginFacade::handle` · crates/suprnova-magnetar/src/plugin/registry.rs:22 (required)
  - [ ] fn `magnetar::plugin::ErasedPluginFacade::handle_bound` · crates/suprnova-magnetar/src/plugin/registry.rs:24 (required)
  - [ ] fn `magnetar::plugin::ErasedPluginFacade::handle_web_binding` · crates/suprnova-magnetar/src/plugin/registry.rs:30 (required)
  - [ ] fn `magnetar::plugin::ErasedPluginFacade::dispatch_lifecycle` · crates/suprnova-magnetar/src/plugin/registry.rs:36 (required)
  - [ ] fn `magnetar::plugin::ErasedPluginFacade::route_names` · crates/suprnova-magnetar/src/plugin/registry.rs:38 (required)
- [ ] trait `magnetar::plugin::Plugin` · crates/suprnova-magnetar/src/plugin/registry.rs:44 (also `magnetar::plugin::registry::Plugin`)
  - Implemented here by: `plugins::device_authorization::DeviceAuthorizationPlugin`, `plugins::email_verification::EmailVerificationPlugin`, `plugins::magic_link::MagicLinkPlugin`, `plugins::passkey::PasskeyPlugin`, `plugins::password::PasswordPlugin`, `plugins::password_management::PasswordManagementPlugin`, `plugins::two_factor::TwoFactorPlugin`
  - [ ] fn `magnetar::plugin::Plugin::name` · crates/suprnova-magnetar/src/plugin/registry.rs:46 (required)
  - [ ] fn `magnetar::plugin::Plugin::routes` · crates/suprnova-magnetar/src/plugin/registry.rs:48 (required)
  - [ ] fn `magnetar::plugin::Plugin::init` · crates/suprnova-magnetar/src/plugin/registry.rs:50 (provided)
  - [ ] fn `magnetar::plugin::Plugin::before_request` · crates/suprnova-magnetar/src/plugin/registry.rs:54 (provided)
  - [ ] fn `magnetar::plugin::Plugin::handle` · crates/suprnova-magnetar/src/plugin/registry.rs:58 (required)
  - [ ] fn `magnetar::plugin::Plugin::lifecycle_hooks` · crates/suprnova-magnetar/src/plugin/registry.rs:60 (provided)

### `magnetar::plugin::routes`

- [ ] struct `magnetar::plugin::RouteDescriptor` · crates/suprnova-magnetar/src/plugin/routes.rs:7 (also `magnetar::plugin::routes::RouteDescriptor`)
  - Public fields: `method`, `path`, `name`, `feature`, `enabled`
  - [ ] fn `magnetar::plugin::RouteDescriptor::new` · crates/suprnova-magnetar/src/plugin/routes.rs:22
  - [ ] fn `magnetar::plugin::RouteDescriptor::with_feature` · crates/suprnova-magnetar/src/plugin/routes.rs:32
  - [ ] fn `magnetar::plugin::RouteDescriptor::disabled` · crates/suprnova-magnetar/src/plugin/routes.rs:37
  - [ ] fn `magnetar::plugin::RouteDescriptor::match_path` · crates/suprnova-magnetar/src/plugin/routes.rs:43
  - [ ] fn `magnetar::plugin::RouteDescriptor::overlaps` · crates/suprnova-magnetar/src/plugin/routes.rs:67

### `magnetar::plugin::wire`

- [ ] struct `magnetar::plugin::WireRequest` · crates/suprnova-magnetar/src/plugin/wire.rs:71 (also `magnetar::plugin::wire::WireRequest`)
  - Public fields: `method`, `path`, `path_params`, `query`, `headers`, `body`
  - [ ] fn `magnetar::plugin::WireRequest::new` · crates/suprnova-magnetar/src/plugin/wire.rs:88
- [ ] struct `magnetar::plugin::WireResponse` · crates/suprnova-magnetar/src/plugin/wire.rs:99 (also `magnetar::plugin::wire::WireResponse`)
  - Public tuple fields: 1
  - [ ] fn `magnetar::plugin::WireResponse::from_effects` · crates/suprnova-magnetar/src/plugin/wire.rs:103
  - [ ] fn `magnetar::plugin::WireResponse::ok` · crates/suprnova-magnetar/src/plugin/wire.rs:107
  - [ ] fn `magnetar::plugin::WireResponse::json` · crates/suprnova-magnetar/src/plugin/wire.rs:111
  - [ ] fn `magnetar::plugin::WireResponse::effects` · crates/suprnova-magnetar/src/plugin/wire.rs:115
  - [ ] fn `magnetar::plugin::WireResponse::into_effects` · crates/suprnova-magnetar/src/plugin/wire.rs:119
- [ ] enum `magnetar::plugin::Method` · crates/suprnova-magnetar/src/plugin/wire.rs:11 (also `magnetar::plugin::wire::Method`)
  - Variants: `Get`, `Post`, `Put`, `Patch`, `Delete`, `Head`, `Options`, `Other`
  - [ ] fn `magnetar::plugin::Method::parse` · crates/suprnova-magnetar/src/plugin/wire.rs:33
- [ ] enum `magnetar::plugin::WireBody` · crates/suprnova-magnetar/src/plugin/wire.rs:52 (also `magnetar::plugin::wire::WireBody`)
  - Variants: `Empty`, `Json`, `Form`, `Bytes`
- [ ] type `magnetar::plugin::HttpMethod` · crates/suprnova-magnetar/src/plugin/wire.rs:48 (also `magnetar::plugin::wire::HttpMethod`)

## plugins

### `magnetar::plugins::device_authorization` (feature: `device-authorization`, off by default)

- [ ] struct `magnetar::plugins::device_authorization::DeviceAuthorizationPlugin` · crates/suprnova-magnetar/src/plugins/device_authorization.rs:45
  - [ ] fn `magnetar::plugins::device_authorization::DeviceAuthorizationPlugin::new` · crates/suprnova-magnetar/src/plugins/device_authorization.rs:53
- [ ] struct `magnetar::plugins::device_authorization::DeviceAuthorizationPluginConfig` · crates/suprnova-magnetar/src/plugins/device_authorization.rs:25
  - Public fields: `route_prefix`, `issue_policy`

### `magnetar::plugins::email_verification` (feature: `email-verification`, off by default)

- [ ] struct `magnetar::plugins::email_verification::EmailVerificationPlugin` · crates/suprnova-magnetar/src/plugins/email_verification.rs:174
  - [ ] fn `magnetar::plugins::email_verification::EmailVerificationPlugin::new` · crates/suprnova-magnetar/src/plugins/email_verification.rs:181
- [ ] struct `magnetar::plugins::email_verification::EmailVerificationPluginConfig` · crates/suprnova-magnetar/src/plugins/email_verification.rs:156
  - Public fields: `resend_policy`
- [ ] struct `magnetar::plugins::email_verification::EmailVerificationService` · crates/suprnova-magnetar/src/plugins/email_verification.rs:37
  - [ ] fn `magnetar::plugins::email_verification::EmailVerificationService::new` · crates/suprnova-magnetar/src/plugins/email_verification.rs:48
  - [ ] fn `magnetar::plugins::email_verification::EmailVerificationService::send_link` · crates/suprnova-magnetar/src/plugins/email_verification.rs:66
  - [ ] fn `magnetar::plugins::email_verification::EmailVerificationService::resend` · crates/suprnova-magnetar/src/plugins/email_verification.rs:102
  - [ ] fn `magnetar::plugins::email_verification::EmailVerificationService::check` · crates/suprnova-magnetar/src/plugins/email_verification.rs:112
  - [ ] fn `magnetar::plugins::email_verification::EmailVerificationService::verify` · crates/suprnova-magnetar/src/plugins/email_verification.rs:121
- [ ] const `magnetar::plugins::email_verification::EMAIL_VERIFICATION_PURPOSE` · crates/suprnova-magnetar/src/plugins/email_verification.rs:30
- [ ] const `magnetar::plugins::email_verification::EMAIL_VERIFICATION_TTL` · crates/suprnova-magnetar/src/plugins/email_verification.rs:33

### `magnetar::plugins::magic_link` (feature: `magic-link`, off by default)

- [ ] struct `magnetar::plugins::magic_link::MagicLinkPlugin` · crates/suprnova-magnetar/src/plugins/magic_link.rs:209
  - [ ] fn `magnetar::plugins::magic_link::MagicLinkPlugin::new` · crates/suprnova-magnetar/src/plugins/magic_link.rs:218
- [ ] struct `magnetar::plugins::magic_link::MagicLinkPluginConfig` · crates/suprnova-magnetar/src/plugins/magic_link.rs:192
  - Public fields: `send_policy`
- [ ] struct `magnetar::plugins::magic_link::MagicLinkService` · crates/suprnova-magnetar/src/plugins/magic_link.rs:80
  - [ ] fn `magnetar::plugins::magic_link::MagicLinkService::new` · crates/suprnova-magnetar/src/plugins/magic_link.rs:91
  - [ ] fn `magnetar::plugins::magic_link::MagicLinkService::issue` · crates/suprnova-magnetar/src/plugins/magic_link.rs:111
  - [ ] fn `magnetar::plugins::magic_link::MagicLinkService::consume` · crates/suprnova-magnetar/src/plugins/magic_link.rs:161
- [ ] enum `magnetar::plugins::magic_link::MagicLinkIssued` · crates/suprnova-magnetar/src/plugins/magic_link.rs:63
  - Variants: `Minted`, `Suppressed`
- [ ] enum `magnetar::plugins::magic_link::RegistrationPolicy` · crates/suprnova-magnetar/src/plugins/magic_link.rs:46
  - Variants: `Open`, `ExistingOnly`
- [ ] const `magnetar::plugins::magic_link::MAGIC_LINK_PURPOSE` · crates/suprnova-magnetar/src/plugins/magic_link.rs:39
- [ ] const `magnetar::plugins::magic_link::MAGIC_LINK_TTL` · crates/suprnova-magnetar/src/plugins/magic_link.rs:42

### `magnetar::plugins::oauth_apple` (feature: `oauth-apple`, off by default)

- [ ] struct `magnetar::plugins::oauth_apple::AppleClaims` · crates/suprnova-magnetar/src/plugins/oauth_apple.rs:131
  - Public fields: `subject`, `email`, `email_verified`, `is_private_email`
- [ ] struct `magnetar::plugins::oauth_apple::AppleOAuthProvider` · crates/suprnova-magnetar/src/plugins/oauth_apple.rs:280
  - [ ] fn `magnetar::plugins::oauth_apple::AppleOAuthProvider::new` · crates/suprnova-magnetar/src/plugins/oauth_apple.rs:310
- [ ] struct `magnetar::plugins::oauth_apple::AppleProviderConfig` · crates/suprnova-magnetar/src/plugins/oauth_apple.rs:109
  - Public fields: `client_id`, `team_id`, `key_id`, `private_key_pem`, `redirect_uri`, `scopes`, `endpoints`
- [ ] struct `magnetar::plugins::oauth_apple::LiveApplePublicKeySource` · crates/suprnova-magnetar/src/plugins/oauth_apple.rs:163
  - [ ] fn `magnetar::plugins::oauth_apple::LiveApplePublicKeySource::new` · crates/suprnova-magnetar/src/plugins/oauth_apple.rs:175
- [ ] trait `magnetar::plugins::oauth_apple::ApplePublicKeySource` · crates/suprnova-magnetar/src/plugins/oauth_apple.rs:148
  - Implemented here by: `plugins::oauth_apple::LiveApplePublicKeySource`
  - [ ] fn `magnetar::plugins::oauth_apple::ApplePublicKeySource::verify` · crates/suprnova-magnetar/src/plugins/oauth_apple.rs:151 (required)

### `magnetar::plugins::oauth_facebook` (feature: `oauth-facebook`, off by default)

- [ ] struct `magnetar::plugins::oauth_facebook::FacebookOAuthProvider` · crates/suprnova-magnetar/src/plugins/oauth_facebook.rs:125
  - [ ] fn `magnetar::plugins::oauth_facebook::FacebookOAuthProvider::new` · crates/suprnova-magnetar/src/plugins/oauth_facebook.rs:138
- [ ] struct `magnetar::plugins::oauth_facebook::FacebookProviderConfig` · crates/suprnova-magnetar/src/plugins/oauth_facebook.rs:82
  - Public fields: `client_id`, `client_secret`, `redirect_uri`, `scopes`, `graph_api_version`, `endpoints`
- [ ] const `magnetar::plugins::oauth_facebook::DEFAULT_GRAPH_API_VERSION` · crates/suprnova-magnetar/src/plugins/oauth_facebook.rs:78

### `magnetar::plugins::oauth_google` (feature: `oauth-google`, off by default)

- [ ] struct `magnetar::plugins::oauth_google::GoogleOAuthProvider` · crates/suprnova-magnetar/src/plugins/oauth_google.rs:81
  - [ ] fn `magnetar::plugins::oauth_google::GoogleOAuthProvider::new` · crates/suprnova-magnetar/src/plugins/oauth_google.rs:94
- [ ] struct `magnetar::plugins::oauth_google::GoogleProviderConfig` · crates/suprnova-magnetar/src/plugins/oauth_google.rs:59
  - Public fields: `client_id`, `client_secret`, `redirect_uri`, `scopes`, `endpoints`

### `magnetar::plugins::oauth_tiktok` (feature: `oauth-tiktok`, off by default)

- [ ] struct `magnetar::plugins::oauth_tiktok::TikTokOAuthProvider` · crates/suprnova-magnetar/src/plugins/oauth_tiktok.rs:123
  - [ ] fn `magnetar::plugins::oauth_tiktok::TikTokOAuthProvider::new` · crates/suprnova-magnetar/src/plugins/oauth_tiktok.rs:132
- [ ] struct `magnetar::plugins::oauth_tiktok::TikTokProviderConfig` · crates/suprnova-magnetar/src/plugins/oauth_tiktok.rs:82
  - Public fields: `client_id`, `client_secret`, `redirect_uri`, `scopes`, `endpoints`

### `magnetar::plugins::oauth_x` (feature: `oauth-x`, off by default)

- [ ] struct `magnetar::plugins::oauth_x::XOAuthProvider` · crates/suprnova-magnetar/src/plugins/oauth_x.rs:85
  - [ ] fn `magnetar::plugins::oauth_x::XOAuthProvider::new` · crates/suprnova-magnetar/src/plugins/oauth_x.rs:94
- [ ] struct `magnetar::plugins::oauth_x::XProviderConfig` · crates/suprnova-magnetar/src/plugins/oauth_x.rs:59
  - Public fields: `client_id`, `client_secret`, `redirect_uri`, `scopes`, `endpoints`

### `magnetar::plugins::passkey` (feature: `passkey`, off by default)

- [ ] struct `magnetar::plugins::passkey::NoReauth` · crates/suprnova-magnetar/src/plugins/passkey.rs:273
- [ ] struct `magnetar::plugins::passkey::PasskeyPlugin` · crates/suprnova-magnetar/src/plugins/passkey.rs:69
  - [ ] fn `magnetar::plugins::passkey::PasskeyPlugin::new` · crates/suprnova-magnetar/src/plugins/passkey.rs:78
- [ ] struct `magnetar::plugins::passkey::PasskeyPluginConfig` · crates/suprnova-magnetar/src/plugins/passkey.rs:46
  - Public fields: `register_policy`, `login_policy`
- [ ] trait `magnetar::plugins::passkey::ReauthSource` · crates/suprnova-magnetar/src/plugins/passkey.rs:36
  - Implemented here by: `plugins::passkey::NoReauth`
  - [ ] fn `magnetar::plugins::passkey::ReauthSource::password_confirmed_at` · crates/suprnova-magnetar/src/plugins/passkey.rs:38 (required)

### `magnetar::plugins::password` (feature: `password`)

- [ ] struct `magnetar::plugins::password::PasswordAttempt` · crates/suprnova-magnetar/src/plugins/password.rs:62
  - Public fields: `email`, `password`, `metadata`
- [ ] struct `magnetar::plugins::password::PasswordAuthService` · crates/suprnova-magnetar/src/plugins/password.rs:124
  - [ ] fn `magnetar::plugins::password::PasswordAuthService::new` · crates/suprnova-magnetar/src/plugins/password.rs:133
- [ ] struct `magnetar::plugins::password::PasswordPlugin` · crates/suprnova-magnetar/src/plugins/password.rs:366
  - [ ] fn `magnetar::plugins::password::PasswordPlugin::new` · crates/suprnova-magnetar/src/plugins/password.rs:376
- [ ] struct `magnetar::plugins::password::PasswordPluginConfig` · crates/suprnova-magnetar/src/plugins/password.rs:335
  - Public fields: `send_verification_on_register`, `establish_session_on_register`, `register_policy`, `login_policy`
- [ ] struct `magnetar::plugins::password::RegisterInput` · crates/suprnova-magnetar/src/plugins/password.rs:35
  - Public fields: `email`, `password`
- [ ] enum `magnetar::plugins::password::RegistrationOutcome` · crates/suprnova-magnetar/src/plugins/password.rs:46
  - Variants: `Created`, `Existing`
- [ ] enum `magnetar::plugins::password::RehashReport` · crates/suprnova-magnetar/src/plugins/password.rs:73
  - Variants: `NotNeeded`, `Upgraded`, `Failed`
- [ ] trait `magnetar::plugins::password::PasswordAuthProvider` · crates/suprnova-magnetar/src/plugins/password.rs:88
  - Implemented here by: `plugins::password::PasswordAuthService`
  - [ ] fn `magnetar::plugins::password::PasswordAuthProvider::register` · crates/suprnova-magnetar/src/plugins/password.rs:91 (required)
  - [ ] fn `magnetar::plugins::password::PasswordAuthProvider::authenticate` · crates/suprnova-magnetar/src/plugins/password.rs:93 (required)
  - [ ] fn `magnetar::plugins::password::PasswordAuthProvider::authenticate_with_outcome` · crates/suprnova-magnetar/src/plugins/password.rs:96 (required)
  - [ ] fn `magnetar::plugins::password::PasswordAuthProvider::perform_authentication_work` · crates/suprnova-magnetar/src/plugins/password.rs:103 (required)
  - [ ] fn `magnetar::plugins::password::PasswordAuthProvider::change_password` · crates/suprnova-magnetar/src/plugins/password.rs:106 (required)
  - [ ] fn `magnetar::plugins::password::PasswordAuthProvider::set_password` · crates/suprnova-magnetar/src/plugins/password.rs:114 (required)
  - [ ] fn `magnetar::plugins::password::PasswordAuthProvider::remove_password` · crates/suprnova-magnetar/src/plugins/password.rs:118 (required)
  - [ ] fn `magnetar::plugins::password::PasswordAuthProvider::has_password` · crates/suprnova-magnetar/src/plugins/password.rs:120 (required)
- [ ] trait `magnetar::plugins::password::RegistrationVerification` · crates/suprnova-magnetar/src/plugins/password.rs:328
  - Implemented here by: `plugins::email_verification::EmailVerificationService`
  - [ ] fn `magnetar::plugins::password::RegistrationVerification::send_for_new_user` · crates/suprnova-magnetar/src/plugins/password.rs:330 (required)

### `magnetar::plugins::password_management` (feature: `password-management`, off by default)

- [ ] struct `magnetar::plugins::password_management::PasswordManagementPlugin` · crates/suprnova-magnetar/src/plugins/password_management.rs:261
  - [ ] fn `magnetar::plugins::password_management::PasswordManagementPlugin::new` · crates/suprnova-magnetar/src/plugins/password_management.rs:268
- [ ] struct `magnetar::plugins::password_management::PasswordManagementPluginConfig` · crates/suprnova-magnetar/src/plugins/password_management.rs:244
  - Public fields: `forgot_policy`
- [ ] struct `magnetar::plugins::password_management::PasswordManagementService` · crates/suprnova-magnetar/src/plugins/password_management.rs:54
  - [ ] fn `magnetar::plugins::password_management::PasswordManagementService::new` · crates/suprnova-magnetar/src/plugins/password_management.rs:68
  - [ ] fn `magnetar::plugins::password_management::PasswordManagementService::send_link` · crates/suprnova-magnetar/src/plugins/password_management.rs:91
  - [ ] fn `magnetar::plugins::password_management::PasswordManagementService::check` · crates/suprnova-magnetar/src/plugins/password_management.rs:129
  - [ ] fn `magnetar::plugins::password_management::PasswordManagementService::complete` · crates/suprnova-magnetar/src/plugins/password_management.rs:138
  - [ ] fn `magnetar::plugins::password_management::PasswordManagementService::complete_with_outcome` · crates/suprnova-magnetar/src/plugins/password_management.rs:148
- [ ] struct `magnetar::plugins::password_management::PasswordResetFlowOutcome` · crates/suprnova-magnetar/src/plugins/password_management.rs:39
  - Public fields: `user_id`, `auth_epoch`, `revoked_sessions`, `remember_rows_revoked`, `lockout_cleared`
- [ ] const `magnetar::plugins::password_management::PASSWORD_RESET_TTL` · crates/suprnova-magnetar/src/plugins/password_management.rs:30

### `magnetar::plugins::shared` (private module; items are public through re-exports)

- [ ] fn `magnetar::plugins::abuse_key` · crates/suprnova-magnetar/src/plugins/mod.rs:148 (feature: `password` or `email-verification` or `password-management` or `magic-link` or `passkey` or `two-factor` or `device-authorization`)

### `magnetar::plugins::two_factor` (feature: `two-factor`, off by default)

- [ ] struct `magnetar::plugins::two_factor::TwoFactorPlugin` · crates/suprnova-magnetar/src/plugins/two_factor.rs:30
  - [ ] fn `magnetar::plugins::two_factor::TwoFactorPlugin::new` · crates/suprnova-magnetar/src/plugins/two_factor.rs:38

## schema

### `magnetar::schema`

- [ ] trait `magnetar::schema::AuthSchema` · crates/suprnova-magnetar/src/schema.rs:44
  - Implemented here by: `default_schema::DefaultAuthSchema`
  - [ ] type `magnetar::schema::AuthSchema::User` · crates/suprnova-magnetar/src/schema.rs:46
  - [ ] type `magnetar::schema::AuthSchema::Session` · crates/suprnova-magnetar/src/schema.rs:48
  - [ ] type `magnetar::schema::AuthSchema::LinkedAccount` · crates/suprnova-magnetar/src/schema.rs:50
  - [ ] type `magnetar::schema::AuthSchema::Passkey` · crates/suprnova-magnetar/src/schema.rs:52
  - [ ] type `magnetar::schema::AuthSchema::Token` · crates/suprnova-magnetar/src/schema.rs:54
  - [ ] type `magnetar::schema::AuthSchema::Ceremony` · crates/suprnova-magnetar/src/schema.rs:56
  - [ ] type `magnetar::schema::AuthSchema::Lockout` · crates/suprnova-magnetar/src/schema.rs:58
  - [ ] type `magnetar::schema::AuthSchema::TokenRecord` · crates/suprnova-magnetar/src/schema.rs:60
- [ ] trait `magnetar::schema::BrokerSchema` · crates/suprnova-magnetar/src/schema.rs:72
  - Implemented here by: `default_schema::DefaultAuthSchema`
  - [ ] type `magnetar::schema::BrokerSchema::ProviderToken` · crates/suprnova-magnetar/src/schema.rs:74

### `magnetar::schema::account`

- [ ] trait `magnetar::schema::LinkedAccountFields` · crates/suprnova-magnetar/src/schema/account.rs:18 (also `magnetar::schema::account::LinkedAccountBinding`, `magnetar::schema::account::LinkedAccountFields`)
  - Implemented here by: `default_schema::accounts::Entity`
  - [ ] fn `magnetar::schema::LinkedAccountFields::read_account_id` · crates/suprnova-magnetar/src/schema/account.rs:20 (required)
  - [ ] fn `magnetar::schema::LinkedAccountFields::account_id_column` · crates/suprnova-magnetar/src/schema/account.rs:22 (required)
  - [ ] fn `magnetar::schema::LinkedAccountFields::write_account_id` · crates/suprnova-magnetar/src/schema/account.rs:24 (required)
  - [ ] fn `magnetar::schema::LinkedAccountFields::read_user_id` · crates/suprnova-magnetar/src/schema/account.rs:26 (required)
  - [ ] fn `magnetar::schema::LinkedAccountFields::user_id_column` · crates/suprnova-magnetar/src/schema/account.rs:28 (required)
  - [ ] fn `magnetar::schema::LinkedAccountFields::user_id_value` · crates/suprnova-magnetar/src/schema/account.rs:30 (provided)
  - [ ] fn `magnetar::schema::LinkedAccountFields::write_user_id` · crates/suprnova-magnetar/src/schema/account.rs:34 (required)
  - [ ] fn `magnetar::schema::LinkedAccountFields::read_provider` · crates/suprnova-magnetar/src/schema/account.rs:36 (required)
  - [ ] fn `magnetar::schema::LinkedAccountFields::provider_column` · crates/suprnova-magnetar/src/schema/account.rs:38 (required)
  - [ ] fn `magnetar::schema::LinkedAccountFields::write_provider` · crates/suprnova-magnetar/src/schema/account.rs:40 (required)
  - [ ] fn `magnetar::schema::LinkedAccountFields::read_provider_account_id` · crates/suprnova-magnetar/src/schema/account.rs:42 (required)
  - [ ] fn `magnetar::schema::LinkedAccountFields::provider_account_id_column` · crates/suprnova-magnetar/src/schema/account.rs:44 (required)
  - [ ] fn `magnetar::schema::LinkedAccountFields::write_provider_account_id` · crates/suprnova-magnetar/src/schema/account.rs:46 (required)
  - [ ] fn `magnetar::schema::LinkedAccountFields::read_access_token` · crates/suprnova-magnetar/src/schema/account.rs:48 (required)
  - [ ] fn `magnetar::schema::LinkedAccountFields::read_refresh_token` · crates/suprnova-magnetar/src/schema/account.rs:50 (required)
  - [ ] fn `magnetar::schema::LinkedAccountFields::read_expires_at` · crates/suprnova-magnetar/src/schema/account.rs:52 (required)

### `magnetar::schema::ceremony`

- [ ] trait `magnetar::schema::CeremonyFields` · crates/suprnova-magnetar/src/schema/ceremony.rs:8 (also `magnetar::schema::ceremony::CeremonyBinding`, `magnetar::schema::ceremony::CeremonyFields`)
  - Implemented here by: `default_schema::ceremonies::Entity`
  - [ ] fn `magnetar::schema::CeremonyFields::read_ceremony_id` · crates/suprnova-magnetar/src/schema/ceremony.rs:10 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::ceremony_id_column` · crates/suprnova-magnetar/src/schema/ceremony.rs:12 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::read_kind` · crates/suprnova-magnetar/src/schema/ceremony.rs:14 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::kind_column` · crates/suprnova-magnetar/src/schema/ceremony.rs:16 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::kind_column_name` · crates/suprnova-magnetar/src/schema/ceremony.rs:18 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::read_selector` · crates/suprnova-magnetar/src/schema/ceremony.rs:20 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::selector_column` · crates/suprnova-magnetar/src/schema/ceremony.rs:22 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::selector_column_name` · crates/suprnova-magnetar/src/schema/ceremony.rs:24 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::read_payload` · crates/suprnova-magnetar/src/schema/ceremony.rs:26 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::read_state` · crates/suprnova-magnetar/src/schema/ceremony.rs:28 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::state_column` · crates/suprnova-magnetar/src/schema/ceremony.rs:30 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::state_column_name` · crates/suprnova-magnetar/src/schema/ceremony.rs:32 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::read_expires_at` · crates/suprnova-magnetar/src/schema/ceremony.rs:34 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::expires_at_column` · crates/suprnova-magnetar/src/schema/ceremony.rs:36 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::read_used_at` · crates/suprnova-magnetar/src/schema/ceremony.rs:38 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::used_at_column` · crates/suprnova-magnetar/src/schema/ceremony.rs:40 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::write_state` · crates/suprnova-magnetar/src/schema/ceremony.rs:42 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::write_used_at` · crates/suprnova-magnetar/src/schema/ceremony.rs:44 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::write_ceremony_id` · crates/suprnova-magnetar/src/schema/ceremony.rs:46 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::write_kind` · crates/suprnova-magnetar/src/schema/ceremony.rs:48 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::write_selector` · crates/suprnova-magnetar/src/schema/ceremony.rs:50 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::write_payload` · crates/suprnova-magnetar/src/schema/ceremony.rs:52 (required)
  - [ ] fn `magnetar::schema::CeremonyFields::write_expires_at` · crates/suprnova-magnetar/src/schema/ceremony.rs:54 (required)

### `magnetar::schema::entity`

- [ ] trait `magnetar::schema::EntityBinding` · crates/suprnova-magnetar/src/schema/entity.rs:4 (also `magnetar::schema::entity::EntityBinding`)
  - Implemented here by: `default_schema::accounts::Entity`, `default_schema::ceremonies::Entity`, `default_schema::lifecycle_deliveries::Entity`, `default_schema::lockouts::Entity`, `default_schema::methods::Entity`, `default_schema::migration_identities::Entity`, `default_schema::migration_runs::Entity`, `default_schema::migration_state::Entity`, `default_schema::provider_tokens::Entity`, `default_schema::remembers::Entity`, `default_schema::sessions::Entity`, `default_schema::tokens::Entity`, `default_schema::users::Entity`
  - [ ] type `magnetar::schema::EntityBinding::Entity` · crates/suprnova-magnetar/src/schema/entity.rs:6
  - [ ] type `magnetar::schema::EntityBinding::Column` · crates/suprnova-magnetar/src/schema/entity.rs:8
  - [ ] type `magnetar::schema::EntityBinding::PrimaryKey` · crates/suprnova-magnetar/src/schema/entity.rs:10
  - [ ] type `magnetar::schema::EntityBinding::Model` · crates/suprnova-magnetar/src/schema/entity.rs:12
  - [ ] type `magnetar::schema::EntityBinding::ActiveModel` · crates/suprnova-magnetar/src/schema/entity.rs:20

### `magnetar::schema::lockout`

- [ ] trait `magnetar::schema::LockoutFields` · crates/suprnova-magnetar/src/schema/lockout.rs:25 (also `magnetar::schema::lockout::LockoutFields`)
  - Implemented here by: `default_schema::lockouts::Entity`
  - [ ] fn `magnetar::schema::LockoutFields::read_lockout_id` · crates/suprnova-magnetar/src/schema/lockout.rs:27 (required)
  - [ ] fn `magnetar::schema::LockoutFields::write_lockout_id` · crates/suprnova-magnetar/src/schema/lockout.rs:29 (required)
  - [ ] fn `magnetar::schema::LockoutFields::read_user_id` · crates/suprnova-magnetar/src/schema/lockout.rs:31 (required)
  - [ ] fn `magnetar::schema::LockoutFields::user_id_column` · crates/suprnova-magnetar/src/schema/lockout.rs:33 (required)
  - [ ] fn `magnetar::schema::LockoutFields::write_user_id` · crates/suprnova-magnetar/src/schema/lockout.rs:35 (required)
  - [ ] fn `magnetar::schema::LockoutFields::read_attempted_at` · crates/suprnova-magnetar/src/schema/lockout.rs:37 (required)
  - [ ] fn `magnetar::schema::LockoutFields::attempted_at_column` · crates/suprnova-magnetar/src/schema/lockout.rs:39 (required)
  - [ ] fn `magnetar::schema::LockoutFields::write_attempted_at` · crates/suprnova-magnetar/src/schema/lockout.rs:41 (required)
  - [ ] fn `magnetar::schema::LockoutFields::read_locked_at` · crates/suprnova-magnetar/src/schema/lockout.rs:43 (required)
  - [ ] fn `magnetar::schema::LockoutFields::read_reason` · crates/suprnova-magnetar/src/schema/lockout.rs:45 (required)
  - [ ] fn `magnetar::schema::LockoutFields::write_reason` · crates/suprnova-magnetar/src/schema/lockout.rs:51 (required)
  - [ ] fn `magnetar::schema::LockoutFields::write_locked_at` · crates/suprnova-magnetar/src/schema/lockout.rs:53 (required)

### `magnetar::schema::passkey`

- [ ] trait `magnetar::schema::PasskeyFields` · crates/suprnova-magnetar/src/schema/passkey.rs:12 (also `magnetar::schema::passkey::PasskeyFields`)
  - Implemented here by: `default_schema::methods::Entity`
  - [ ] fn `magnetar::schema::PasskeyFields::read_passkey_id` · crates/suprnova-magnetar/src/schema/passkey.rs:14 (required)
  - [ ] fn `magnetar::schema::PasskeyFields::passkey_id_column` · crates/suprnova-magnetar/src/schema/passkey.rs:16 (required)
  - [ ] fn `magnetar::schema::PasskeyFields::write_passkey_id` · crates/suprnova-magnetar/src/schema/passkey.rs:18 (required)
  - [ ] fn `magnetar::schema::PasskeyFields::read_user_id` · crates/suprnova-magnetar/src/schema/passkey.rs:20 (required)
  - [ ] fn `magnetar::schema::PasskeyFields::user_id_column` · crates/suprnova-magnetar/src/schema/passkey.rs:22 (required)
  - [ ] fn `magnetar::schema::PasskeyFields::user_id_value` · crates/suprnova-magnetar/src/schema/passkey.rs:24 (provided)
  - [ ] fn `magnetar::schema::PasskeyFields::write_user_id` · crates/suprnova-magnetar/src/schema/passkey.rs:28 (required)
  - [ ] fn `magnetar::schema::PasskeyFields::read_credential_id` · crates/suprnova-magnetar/src/schema/passkey.rs:30 (required)
  - [ ] fn `magnetar::schema::PasskeyFields::credential_id_column` · crates/suprnova-magnetar/src/schema/passkey.rs:32 (required)
  - [ ] fn `magnetar::schema::PasskeyFields::write_credential_id` · crates/suprnova-magnetar/src/schema/passkey.rs:34 (required)
  - [ ] fn `magnetar::schema::PasskeyFields::read_public_key` · crates/suprnova-magnetar/src/schema/passkey.rs:36 (required)
  - [ ] fn `magnetar::schema::PasskeyFields::write_public_key` · crates/suprnova-magnetar/src/schema/passkey.rs:38 (required)
  - [ ] fn `magnetar::schema::PasskeyFields::read_sign_count` · crates/suprnova-magnetar/src/schema/passkey.rs:40 (required)
  - [ ] fn `magnetar::schema::PasskeyFields::read_transports` · crates/suprnova-magnetar/src/schema/passkey.rs:42 (required)
  - [ ] fn `magnetar::schema::PasskeyFields::read_created_at` · crates/suprnova-magnetar/src/schema/passkey.rs:44 (required)

### `magnetar::schema::provider_token`

- [ ] trait `magnetar::schema::ProviderTokenFields` · crates/suprnova-magnetar/src/schema/provider_token.rs:19 (also `magnetar::schema::provider_token::ProviderTokenFields`)
  - Implemented here by: `default_schema::provider_tokens::Entity`
  - [ ] fn `magnetar::schema::ProviderTokenFields::read_id` · crates/suprnova-magnetar/src/schema/provider_token.rs:21 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::id_column` · crates/suprnova-magnetar/src/schema/provider_token.rs:23 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::write_id` · crates/suprnova-magnetar/src/schema/provider_token.rs:25 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::read_provider` · crates/suprnova-magnetar/src/schema/provider_token.rs:28 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::provider_column` · crates/suprnova-magnetar/src/schema/provider_token.rs:30 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::write_provider` · crates/suprnova-magnetar/src/schema/provider_token.rs:32 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::read_access_ciphertext` · crates/suprnova-magnetar/src/schema/provider_token.rs:36 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::access_ciphertext_column` · crates/suprnova-magnetar/src/schema/provider_token.rs:38 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::write_access_ciphertext` · crates/suprnova-magnetar/src/schema/provider_token.rs:40 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::read_refresh_ciphertext` · crates/suprnova-magnetar/src/schema/provider_token.rs:45 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::refresh_ciphertext_column` · crates/suprnova-magnetar/src/schema/provider_token.rs:47 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::write_refresh_ciphertext` · crates/suprnova-magnetar/src/schema/provider_token.rs:49 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::read_raw_payload_ciphertext` · crates/suprnova-magnetar/src/schema/provider_token.rs:54 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::raw_payload_ciphertext_column` · crates/suprnova-magnetar/src/schema/provider_token.rs:56 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::write_raw_payload_ciphertext` · crates/suprnova-magnetar/src/schema/provider_token.rs:58 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::read_token_type` · crates/suprnova-magnetar/src/schema/provider_token.rs:61 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::token_type_column` · crates/suprnova-magnetar/src/schema/provider_token.rs:63 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::write_token_type` · crates/suprnova-magnetar/src/schema/provider_token.rs:65 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::read_scopes` · crates/suprnova-magnetar/src/schema/provider_token.rs:68 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::scopes_column` · crates/suprnova-magnetar/src/schema/provider_token.rs:70 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::write_scopes` · crates/suprnova-magnetar/src/schema/provider_token.rs:72 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::read_access_expires_at` · crates/suprnova-magnetar/src/schema/provider_token.rs:75 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::access_expires_at_column` · crates/suprnova-magnetar/src/schema/provider_token.rs:77 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::write_access_expires_at` · crates/suprnova-magnetar/src/schema/provider_token.rs:79 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::read_generation` · crates/suprnova-magnetar/src/schema/provider_token.rs:82 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::generation_column` · crates/suprnova-magnetar/src/schema/provider_token.rs:84 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::write_generation` · crates/suprnova-magnetar/src/schema/provider_token.rs:86 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::read_claim_id` · crates/suprnova-magnetar/src/schema/provider_token.rs:89 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::claim_id_column` · crates/suprnova-magnetar/src/schema/provider_token.rs:91 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::write_claim_id` · crates/suprnova-magnetar/src/schema/provider_token.rs:93 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::read_claim_deadline` · crates/suprnova-magnetar/src/schema/provider_token.rs:96 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::claim_deadline_column` · crates/suprnova-magnetar/src/schema/provider_token.rs:98 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::write_claim_deadline` · crates/suprnova-magnetar/src/schema/provider_token.rs:100 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::read_revoked_at` · crates/suprnova-magnetar/src/schema/provider_token.rs:104 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::revoked_at_column` · crates/suprnova-magnetar/src/schema/provider_token.rs:106 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::write_revoked_at` · crates/suprnova-magnetar/src/schema/provider_token.rs:108 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::read_revoked_reused` · crates/suprnova-magnetar/src/schema/provider_token.rs:113 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::revoked_reused_column` · crates/suprnova-magnetar/src/schema/provider_token.rs:115 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::write_revoked_reused` · crates/suprnova-magnetar/src/schema/provider_token.rs:117 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::read_created_at` · crates/suprnova-magnetar/src/schema/provider_token.rs:120 (required)
  - [ ] fn `magnetar::schema::ProviderTokenFields::write_created_at` · crates/suprnova-magnetar/src/schema/provider_token.rs:122 (required)

### `magnetar::schema::session`

- [ ] trait `magnetar::schema::SessionFields` · crates/suprnova-magnetar/src/schema/session.rs:8 (also `magnetar::schema::session::SessionFields`)
  - Implemented here by: `default_schema::sessions::Entity`
  - [ ] fn `magnetar::schema::SessionFields::read_session_id` · crates/suprnova-magnetar/src/schema/session.rs:10 (required)
  - [ ] fn `magnetar::schema::SessionFields::session_id_column` · crates/suprnova-magnetar/src/schema/session.rs:12 (required)
  - [ ] fn `magnetar::schema::SessionFields::read_user_id` · crates/suprnova-magnetar/src/schema/session.rs:14 (required)
  - [ ] fn `magnetar::schema::SessionFields::user_id_column` · crates/suprnova-magnetar/src/schema/session.rs:16 (required)
  - [ ] fn `magnetar::schema::SessionFields::user_id_value` · crates/suprnova-magnetar/src/schema/session.rs:18 (provided)
  - [ ] fn `magnetar::schema::SessionFields::read_auth_epoch` · crates/suprnova-magnetar/src/schema/session.rs:22 (required)
  - [ ] fn `magnetar::schema::SessionFields::auth_epoch_column` · crates/suprnova-magnetar/src/schema/session.rs:24 (required)
  - [ ] fn `magnetar::schema::SessionFields::auth_epoch_value` · crates/suprnova-magnetar/src/schema/session.rs:26 (provided)
  - [ ] fn `magnetar::schema::SessionFields::write_auth_epoch` · crates/suprnova-magnetar/src/schema/session.rs:30 (required)
  - [ ] fn `magnetar::schema::SessionFields::read_token_digest` · crates/suprnova-magnetar/src/schema/session.rs:32 (required)
  - [ ] fn `magnetar::schema::SessionFields::read_expires_at` · crates/suprnova-magnetar/src/schema/session.rs:34 (required)
  - [ ] fn `magnetar::schema::SessionFields::read_revoked_at` · crates/suprnova-magnetar/src/schema/session.rs:36 (required)
  - [ ] fn `magnetar::schema::SessionFields::revoked_at_column` · crates/suprnova-magnetar/src/schema/session.rs:38 (required)
  - [ ] fn `magnetar::schema::SessionFields::write_revoked_at` · crates/suprnova-magnetar/src/schema/session.rs:40 (required)

### `magnetar::schema::token`

- [ ] trait `magnetar::schema::TokenFields` · crates/suprnova-magnetar/src/schema/token.rs:8 (also `magnetar::schema::token::TokenFields`)
  - Implemented here by: `default_schema::tokens::Entity`
  - [ ] fn `magnetar::schema::TokenFields::read_token_id` · crates/suprnova-magnetar/src/schema/token.rs:10 (required)
  - [ ] fn `magnetar::schema::TokenFields::token_id_column` · crates/suprnova-magnetar/src/schema/token.rs:12 (required)
  - [ ] fn `magnetar::schema::TokenFields::read_user_id` · crates/suprnova-magnetar/src/schema/token.rs:14 (required)
  - [ ] fn `magnetar::schema::TokenFields::user_id_column` · crates/suprnova-magnetar/src/schema/token.rs:16 (required)
  - [ ] fn `magnetar::schema::TokenFields::user_id_value` · crates/suprnova-magnetar/src/schema/token.rs:18 (provided)
  - [ ] fn `magnetar::schema::TokenFields::read_purpose` · crates/suprnova-magnetar/src/schema/token.rs:22 (required)
  - [ ] fn `magnetar::schema::TokenFields::purpose_column` · crates/suprnova-magnetar/src/schema/token.rs:24 (required)
  - [ ] fn `magnetar::schema::TokenFields::purpose_column_name` · crates/suprnova-magnetar/src/schema/token.rs:26 (required)
  - [ ] fn `magnetar::schema::TokenFields::read_digest` · crates/suprnova-magnetar/src/schema/token.rs:28 (required)
  - [ ] fn `magnetar::schema::TokenFields::digest_column` · crates/suprnova-magnetar/src/schema/token.rs:30 (required)
  - [ ] fn `magnetar::schema::TokenFields::digest_column_name` · crates/suprnova-magnetar/src/schema/token.rs:32 (required)
  - [ ] fn `magnetar::schema::TokenFields::read_expires_at` · crates/suprnova-magnetar/src/schema/token.rs:34 (required)
  - [ ] fn `magnetar::schema::TokenFields::expires_at_column` · crates/suprnova-magnetar/src/schema/token.rs:36 (required)
  - [ ] fn `magnetar::schema::TokenFields::read_used_at` · crates/suprnova-magnetar/src/schema/token.rs:38 (required)
  - [ ] fn `magnetar::schema::TokenFields::used_at_column` · crates/suprnova-magnetar/src/schema/token.rs:40 (required)
  - [ ] fn `magnetar::schema::TokenFields::used_at_column_name` · crates/suprnova-magnetar/src/schema/token.rs:42 (required)
  - [ ] fn `magnetar::schema::TokenFields::write_used_at` · crates/suprnova-magnetar/src/schema/token.rs:44 (required)
  - [ ] fn `magnetar::schema::TokenFields::write_token_id` · crates/suprnova-magnetar/src/schema/token.rs:46 (required)
  - [ ] fn `magnetar::schema::TokenFields::write_user_id` · crates/suprnova-magnetar/src/schema/token.rs:48 (required)
  - [ ] fn `magnetar::schema::TokenFields::write_purpose` · crates/suprnova-magnetar/src/schema/token.rs:50 (required)
  - [ ] fn `magnetar::schema::TokenFields::write_digest` · crates/suprnova-magnetar/src/schema/token.rs:52 (required)
  - [ ] fn `magnetar::schema::TokenFields::write_expires_at` · crates/suprnova-magnetar/src/schema/token.rs:54 (required)
- [ ] trait `magnetar::schema::TokenRecordFields` · crates/suprnova-magnetar/src/schema/token.rs:58 (also `magnetar::schema::token::TokenRecordFields`)
  - [ ] fn `magnetar::schema::TokenRecordFields::read_record_id` · crates/suprnova-magnetar/src/schema/token.rs:60 (required)
  - [ ] fn `magnetar::schema::TokenRecordFields::read_token_id` · crates/suprnova-magnetar/src/schema/token.rs:62 (required)
  - [ ] fn `magnetar::schema::TokenRecordFields::read_user_id` · crates/suprnova-magnetar/src/schema/token.rs:64 (required)
  - [ ] fn `magnetar::schema::TokenRecordFields::read_purpose` · crates/suprnova-magnetar/src/schema/token.rs:66 (required)
  - [ ] fn `magnetar::schema::TokenRecordFields::read_digest` · crates/suprnova-magnetar/src/schema/token.rs:68 (required)

### `magnetar::schema::user`

- [ ] fn `magnetar::schema::password_hash_for_verifier` · crates/suprnova-magnetar/src/schema/user.rs:85 (also `magnetar::schema::user::password_hash_for_verifier`)
- [ ] trait `magnetar::schema::SessionEpoch` · crates/suprnova-magnetar/src/schema/user.rs:66 (also `magnetar::schema::user::SessionEpoch`)
  - Implemented here by: `default_schema::users::Entity`
  - [ ] fn `magnetar::schema::SessionEpoch::auth_epoch` · crates/suprnova-magnetar/src/schema/user.rs:68 (required)
  - [ ] fn `magnetar::schema::SessionEpoch::auth_epoch_column` · crates/suprnova-magnetar/src/schema/user.rs:70 (required)
  - [ ] fn `magnetar::schema::SessionEpoch::auth_epoch_value` · crates/suprnova-magnetar/src/schema/user.rs:72 (provided)
  - [ ] fn `magnetar::schema::SessionEpoch::write_auth_epoch` · crates/suprnova-magnetar/src/schema/user.rs:76 (required)
- [ ] trait `magnetar::schema::UserBinding` · crates/suprnova-magnetar/src/schema/user.rs:17 (also `magnetar::schema::UserFields`, `magnetar::schema::user::UserBinding`, `magnetar::schema::user::UserFields`)
  - Implemented here by: `default_schema::users::Entity`
  - [ ] fn `magnetar::schema::UserBinding::read_user_id` · crates/suprnova-magnetar/src/schema/user.rs:19 (required)
  - [ ] fn `magnetar::schema::UserBinding::user_id_column` · crates/suprnova-magnetar/src/schema/user.rs:21 (required)
  - [ ] fn `magnetar::schema::UserBinding::user_id_value` · crates/suprnova-magnetar/src/schema/user.rs:23 (provided)
  - [ ] fn `magnetar::schema::UserBinding::write_user_id` · crates/suprnova-magnetar/src/schema/user.rs:30 (required)
  - [ ] fn `magnetar::schema::UserBinding::read_email` · crates/suprnova-magnetar/src/schema/user.rs:32 (required)
  - [ ] fn `magnetar::schema::UserBinding::email_column` · crates/suprnova-magnetar/src/schema/user.rs:34 (required)
  - [ ] fn `magnetar::schema::UserBinding::write_email` · crates/suprnova-magnetar/src/schema/user.rs:36 (required)
  - [ ] fn `magnetar::schema::UserBinding::read_password_hash` · crates/suprnova-magnetar/src/schema/user.rs:38 (required)
  - [ ] fn `magnetar::schema::UserBinding::password_hash_column` · crates/suprnova-magnetar/src/schema/user.rs:40 (required)
  - [ ] fn `magnetar::schema::UserBinding::write_password_hash` · crates/suprnova-magnetar/src/schema/user.rs:42 (required)
  - [ ] fn `magnetar::schema::UserBinding::read_locked_at` · crates/suprnova-magnetar/src/schema/user.rs:44 (required)
  - [ ] fn `magnetar::schema::UserBinding::locked_at_column` · crates/suprnova-magnetar/src/schema/user.rs:46 (required)
  - [ ] fn `magnetar::schema::UserBinding::write_locked_at` · crates/suprnova-magnetar/src/schema/user.rs:48 (required)
- [ ] trait `magnetar::schema::UserOptionalFields` · crates/suprnova-magnetar/src/schema/user.rs:52 (also `magnetar::schema::user::UserOptionalFields`)
  - Implemented here by: `default_schema::users::Entity`
  - [ ] fn `magnetar::schema::UserOptionalFields::read_name` · crates/suprnova-magnetar/src/schema/user.rs:54 (required)
  - [ ] fn `magnetar::schema::UserOptionalFields::read_email_verified_at` · crates/suprnova-magnetar/src/schema/user.rs:56 (required)
  - [ ] fn `magnetar::schema::UserOptionalFields::write_email_verified_at` · crates/suprnova-magnetar/src/schema/user.rs:58 (required)
  - [ ] fn `magnetar::schema::UserOptionalFields::read_remember_token` · crates/suprnova-magnetar/src/schema/user.rs:60 (required)
  - [ ] fn `magnetar::schema::UserOptionalFields::write_remember_token` · crates/suprnova-magnetar/src/schema/user.rs:62 (required)
- [ ] const `magnetar::schema::NOT_NULL_PASSWORD_EMPTY_SENTINEL` · crates/suprnova-magnetar/src/schema/user.rs:14 (also `magnetar::schema::user::NOT_NULL_PASSWORD_EMPTY_SENTINEL`)

## sessions

### `magnetar::sessions`

- [ ] struct `magnetar::sessions::HostSessionApproval` · crates/suprnova-magnetar/src/sessions/mod.rs:168
  - [ ] fn `magnetar::sessions::HostSessionApproval::authenticated` · crates/suprnova-magnetar/src/sessions/mod.rs:172
- [ ] struct `magnetar::sessions::SessionMetadata` · crates/suprnova-magnetar/src/sessions/mod.rs:28
  - Public fields: `user_agent`, `ip_address`
- [ ] struct `magnetar::sessions::SessionSummary` · crates/suprnova-magnetar/src/sessions/mod.rs:123
  - Public fields: `session_id`, `user_id`, `expires_at`, `metadata`
- [ ] struct `magnetar::sessions::VerifiedSession` · crates/suprnova-magnetar/src/sessions/mod.rs:49
  - [ ] fn `magnetar::sessions::VerifiedSession::carrier` · crates/suprnova-magnetar/src/sessions/mod.rs:86
  - [ ] fn `magnetar::sessions::VerifiedSession::session_id` · crates/suprnova-magnetar/src/sessions/mod.rs:92
  - [ ] fn `magnetar::sessions::VerifiedSession::user_id` · crates/suprnova-magnetar/src/sessions/mod.rs:98
  - [ ] fn `magnetar::sessions::VerifiedSession::auth_epoch` · crates/suprnova-magnetar/src/sessions/mod.rs:104
  - [ ] fn `magnetar::sessions::VerifiedSession::expires_at` · crates/suprnova-magnetar/src/sessions/mod.rs:110
  - [ ] fn `magnetar::sessions::VerifiedSession::metadata` · crates/suprnova-magnetar/src/sessions/mod.rs:116
- [ ] enum `magnetar::sessions::SessionCarrier` · crates/suprnova-magnetar/src/sessions/mod.rs:37
  - Variants: `Opaque`, `Jwt`
- [ ] trait `magnetar::sessions::SessionQueries` · crates/suprnova-magnetar/src/sessions/mod.rs:143
  - Implemented here by: `sessions::JwtSessionProvider`, `sessions::OpaqueSessionProvider`
  - [ ] fn `magnetar::sessions::SessionQueries::verify_bearer` · crates/suprnova-magnetar/src/sessions/mod.rs:145 (required)
  - [ ] fn `magnetar::sessions::SessionQueries::resolve_web_binding` · crates/suprnova-magnetar/src/sessions/mod.rs:147 (required)
  - [ ] fn `magnetar::sessions::SessionQueries::revoke_all_for_user` · crates/suprnova-magnetar/src/sessions/mod.rs:153 (required)
  - [ ] fn `magnetar::sessions::SessionQueries::revoke_session` · crates/suprnova-magnetar/src/sessions/mod.rs:161 (required)
  - [ ] fn `magnetar::sessions::SessionQueries::list_for_user` · crates/suprnova-magnetar/src/sessions/mod.rs:163 (required)

### `magnetar::sessions::grant`

- [ ] struct `magnetar::sessions::BearerSession` · crates/suprnova-magnetar/src/sessions/grant.rs:20 (also `magnetar::sessions::grant::BearerSession`)
  - [ ] fn `magnetar::sessions::BearerSession::session_id` · crates/suprnova-magnetar/src/sessions/grant.rs:30
  - [ ] fn `magnetar::sessions::BearerSession::user_id` · crates/suprnova-magnetar/src/sessions/grant.rs:34
  - [ ] fn `magnetar::sessions::BearerSession::expires_at` · crates/suprnova-magnetar/src/sessions/grant.rs:38
  - [ ] fn `magnetar::sessions::BearerSession::metadata` · crates/suprnova-magnetar/src/sessions/grant.rs:42
  - [ ] fn `magnetar::sessions::BearerSession::expose_token_once` · crates/suprnova-magnetar/src/sessions/grant.rs:46
- [ ] struct `magnetar::sessions::SessionGrant` · crates/suprnova-magnetar/src/sessions/grant.rs:66 (also `magnetar::sessions::grant::SessionGrant`)
  - [ ] fn `magnetar::sessions::SessionGrant::session_id` · crates/suprnova-magnetar/src/sessions/grant.rs:125
  - [ ] fn `magnetar::sessions::SessionGrant::user_id` · crates/suprnova-magnetar/src/sessions/grant.rs:129
  - [ ] fn `magnetar::sessions::SessionGrant::expires_at` · crates/suprnova-magnetar/src/sessions/grant.rs:133
  - [ ] fn `magnetar::sessions::SessionGrant::metadata` · crates/suprnova-magnetar/src/sessions/grant.rs:137
  - [ ] fn `magnetar::sessions::SessionGrant::web_binding` · crates/suprnova-magnetar/src/sessions/grant.rs:141
  - [ ] fn `magnetar::sessions::SessionGrant::into_bearer` · crates/suprnova-magnetar/src/sessions/grant.rs:148
- [ ] struct `magnetar::sessions::WebSessionBinding` · crates/suprnova-magnetar/src/sessions/grant.rs:11 (also `magnetar::sessions::grant::WebSessionBinding`)
  - Public fields: `session_id`, `token_digest`

### `magnetar::sessions::jwt`

- [ ] struct `magnetar::sessions::JwtConfig` · crates/suprnova-magnetar/src/sessions/jwt.rs:24 (also `magnetar::sessions::jwt::JwtConfig`)
  - Public fields: `issuer`, `signing_key`, `lifetime`
  - [ ] fn `magnetar::sessions::JwtConfig::new` · crates/suprnova-magnetar/src/sessions/jwt.rs:35
- [ ] struct `magnetar::sessions::JwtSessionProvider` · crates/suprnova-magnetar/src/sessions/jwt.rs:54 (also `magnetar::sessions::jwt::JwtSessionProvider`)
  - [ ] fn `magnetar::sessions::JwtSessionProvider::new` · crates/suprnova-magnetar/src/sessions/jwt.rs:70
- [ ] trait `magnetar::sessions::JwtEpochStore` · crates/suprnova-magnetar/src/sessions/jwt.rs:46 (also `magnetar::sessions::jwt::JwtEpochStore`)
  - Implemented here by: `storage::SeaOrmStorage`
  - [ ] fn `magnetar::sessions::JwtEpochStore::current_auth_epoch` · crates/suprnova-magnetar/src/sessions/jwt.rs:48 (required)
  - [ ] fn `magnetar::sessions::JwtEpochStore::bump_auth_epoch` · crates/suprnova-magnetar/src/sessions/jwt.rs:50 (required)

### `magnetar::sessions::opaque`

- [ ] struct `magnetar::sessions::OpaqueConfig` · crates/suprnova-magnetar/src/sessions/opaque.rs:65 (also `magnetar::sessions::opaque::OpaqueConfig`)
  - Public fields: `lifetime`
- [ ] struct `magnetar::sessions::OpaqueSessionProvider` · crates/suprnova-magnetar/src/sessions/opaque.rs:79 (also `magnetar::sessions::opaque::OpaqueSessionProvider`)
  - [ ] fn `magnetar::sessions::OpaqueSessionProvider::new` · crates/suprnova-magnetar/src/sessions/opaque.rs:95
- [ ] struct `magnetar::sessions::StoredSession` · crates/suprnova-magnetar/src/sessions/opaque.rs:19 (also `magnetar::sessions::opaque::StoredSession`)
  - Public fields: `session_id`, `user_id`, `auth_epoch`, `token_hash`, `token_digest`, `expires_at`, `revoked_at`, `metadata`
- [ ] trait `magnetar::sessions::OpaqueSessionStore` · crates/suprnova-magnetar/src/sessions/opaque.rs:40 (also `magnetar::sessions::opaque::OpaqueSessionStore`)
  - Implemented here by: `default_schema::sql_stores::SqlSessionStore`
  - [ ] fn `magnetar::sessions::OpaqueSessionStore::insert_session_if_epoch_current` · crates/suprnova-magnetar/src/sessions/opaque.rs:42 (required)
  - [ ] fn `magnetar::sessions::OpaqueSessionStore::find_by_token_hash` · crates/suprnova-magnetar/src/sessions/opaque.rs:44 (required)
  - [ ] fn `magnetar::sessions::OpaqueSessionStore::find_by_web_binding` · crates/suprnova-magnetar/src/sessions/opaque.rs:46 (required)
  - [ ] fn `magnetar::sessions::OpaqueSessionStore::revoke_all_sessions` · crates/suprnova-magnetar/src/sessions/opaque.rs:51 (required)
  - [ ] fn `magnetar::sessions::OpaqueSessionStore::revoke_session` · crates/suprnova-magnetar/src/sessions/opaque.rs:54 (required)
  - [ ] fn `magnetar::sessions::OpaqueSessionStore::list_active_sessions` · crates/suprnova-magnetar/src/sessions/opaque.rs:56 (required)

### `magnetar::sessions::remember`

- [ ] struct `magnetar::sessions::RememberAnomaly` · crates/suprnova-magnetar/src/sessions/remember.rs:151 (also `magnetar::sessions::remember::RememberAnomaly`)
  - Public fields: `kind`, `user_id`
- [ ] struct `magnetar::sessions::RememberCredential` · crates/suprnova-magnetar/src/sessions/remember.rs:117 (also `magnetar::sessions::remember::RememberCredential`)
  - [ ] fn `magnetar::sessions::RememberCredential::from_host` · crates/suprnova-magnetar/src/sessions/remember.rs:130
  - [ ] fn `magnetar::sessions::RememberCredential::expose_once` · crates/suprnova-magnetar/src/sessions/remember.rs:135
- [ ] struct `magnetar::sessions::RememberPostRotationFailure` · crates/suprnova-magnetar/src/sessions/remember.rs:482 (also `magnetar::sessions::remember::RememberPostRotationFailure`)
  - Public fields: `error`, `disposition`
- [ ] struct `magnetar::sessions::RememberRow` · crates/suprnova-magnetar/src/sessions/remember.rs:26 (also `magnetar::sessions::remember::RememberRow`)
  - Public fields: `id`, `selector`, `user_id`, `auth_epoch`, `verifier_hash`, `expires_at`
- [ ] struct `magnetar::sessions::RememberService` · crates/suprnova-magnetar/src/sessions/remember.rs:243 (also `magnetar::sessions::remember::RememberService`)
  - [ ] fn `magnetar::sessions::RememberService::new` · crates/suprnova-magnetar/src/sessions/remember.rs:261
  - [ ] fn `magnetar::sessions::RememberService::with_anomaly_hook` · crates/suprnova-magnetar/src/sessions/remember.rs:272
  - [ ] fn `magnetar::sessions::RememberService::issue_at_epoch` · crates/suprnova-magnetar/src/sessions/remember.rs:303
  - [ ] fn `magnetar::sessions::RememberService::rotate_at_epoch` · crates/suprnova-magnetar/src/sessions/remember.rs:317
  - [ ] fn `magnetar::sessions::RememberService::revoke_selector` · crates/suprnova-magnetar/src/sessions/remember.rs:372
  - [ ] fn `magnetar::sessions::RememberService::revoke_all_for_user` · crates/suprnova-magnetar/src/sessions/remember.rs:377
  - [ ] fn `magnetar::sessions::RememberService::prune_expired` · crates/suprnova-magnetar/src/sessions/remember.rs:382
- [ ] struct `magnetar::sessions::RememberSignInOutcome` · crates/suprnova-magnetar/src/sessions/remember.rs:439 (also `magnetar::sessions::remember::RememberSignInOutcome`)
  - Public fields: `session`, `replacement`
- [ ] struct `magnetar::sessions::RememberSignInService` · crates/suprnova-magnetar/src/sessions/remember.rs:514 (also `magnetar::sessions::remember::RememberSignInService`)
  - [ ] fn `magnetar::sessions::RememberSignInService::new` · crates/suprnova-magnetar/src/sessions/remember.rs:532
  - [ ] fn `magnetar::sessions::RememberSignInService::issue` · crates/suprnova-magnetar/src/sessions/remember.rs:545
  - [ ] fn `magnetar::sessions::RememberSignInService::issue_with_lifetime` · crates/suprnova-magnetar/src/sessions/remember.rs:551
  - [ ] fn `magnetar::sessions::RememberSignInService::sign_in` · crates/suprnova-magnetar/src/sessions/remember.rs:564
  - [ ] fn `magnetar::sessions::RememberSignInService::sign_in_with_lifetime` · crates/suprnova-magnetar/src/sessions/remember.rs:575
  - [ ] fn `magnetar::sessions::RememberSignInService::attempt_sign_in_with_lifetime` · crates/suprnova-magnetar/src/sessions/remember.rs:617
  - [ ] fn `magnetar::sessions::RememberSignInService::revoke_selector` · crates/suprnova-magnetar/src/sessions/remember.rs:739
  - [ ] fn `magnetar::sessions::RememberSignInService::revoke_all_for_user` · crates/suprnova-magnetar/src/sessions/remember.rs:744
- [ ] enum `magnetar::sessions::RememberAnomalyKind` · crates/suprnova-magnetar/src/sessions/remember.rs:142 (also `magnetar::sessions::remember::RememberAnomalyKind`)
  - Variants: `UnknownOrReusedSelector`, `VerifierMismatch`
- [ ] enum `magnetar::sessions::RememberPostRotationDisposition` · crates/suprnova-magnetar/src/sessions/remember.rs:473 (also `magnetar::sessions::remember::RememberPostRotationDisposition`)
  - Variants: `Retryable`, `Reject`
- [ ] enum `magnetar::sessions::RememberRotationAttempt` · crates/suprnova-magnetar/src/sessions/remember.rs:449 (also `magnetar::sessions::remember::RememberRotationAttempt`)
  - Variants: `Committed`, `NotCommitted`, `OutcomeUnknown`
- [ ] enum `magnetar::sessions::RememberSignInAttempt` · crates/suprnova-magnetar/src/sessions/remember.rs:492 (also `magnetar::sessions::remember::RememberSignInAttempt`)
  - Variants: `Authenticated`, `RotationCommitted`, `RotationOutcomeUnknown`
- [ ] trait `magnetar::sessions::RememberAnomalyHook` · crates/suprnova-magnetar/src/sessions/remember.rs:160 (also `magnetar::sessions::remember::RememberAnomalyHook`)
  - [ ] fn `magnetar::sessions::RememberAnomalyHook::on_anomaly` · crates/suprnova-magnetar/src/sessions/remember.rs:162 (required)
- [ ] trait `magnetar::sessions::RememberFacade` · crates/suprnova-magnetar/src/sessions/remember.rs:761 (also `magnetar::sessions::remember::RememberFacade`)
  - Implemented here by: `sessions::RememberSignInService`
  - [ ] fn `magnetar::sessions::RememberFacade::issue_now` · crates/suprnova-magnetar/src/sessions/remember.rs:763 (required)
  - [ ] fn `magnetar::sessions::RememberFacade::issue_with_lifetime` · crates/suprnova-magnetar/src/sessions/remember.rs:765 (required)
  - [ ] fn `magnetar::sessions::RememberFacade::sign_in_with_lifetime` · crates/suprnova-magnetar/src/sessions/remember.rs:772 (required)
  - [ ] fn `magnetar::sessions::RememberFacade::revoke_all` · crates/suprnova-magnetar/src/sessions/remember.rs:780 (required)
- [ ] trait `magnetar::sessions::RememberStore` · crates/suprnova-magnetar/src/sessions/remember.rs:56 (also `magnetar::sessions::remember::RememberStore`)
  - Implemented here by: `default_schema::sql_stores::SqlRememberStore`
  - [ ] fn `magnetar::sessions::RememberStore::insert_remember` · crates/suprnova-magnetar/src/sessions/remember.rs:58 (required)
  - [ ] fn `magnetar::sessions::RememberStore::find_for_rotation` · crates/suprnova-magnetar/src/sessions/remember.rs:60 (required)
  - [ ] fn `magnetar::sessions::RememberStore::consume_for_rotation` · crates/suprnova-magnetar/src/sessions/remember.rs:66 (required)
  - [ ] fn `magnetar::sessions::RememberStore::replace_for_rotation` · crates/suprnova-magnetar/src/sessions/remember.rs:77 (provided)
  - [ ] fn `magnetar::sessions::RememberStore::revoke_remember_selector` · crates/suprnova-magnetar/src/sessions/remember.rs:103 (provided)
  - [ ] fn `magnetar::sessions::RememberStore::revoke_all_remember` · crates/suprnova-magnetar/src/sessions/remember.rs:111 (required)
  - [ ] fn `magnetar::sessions::RememberStore::prune_expired_remember` · crates/suprnova-magnetar/src/sessions/remember.rs:113 (required)
- [ ] trait `magnetar::sessions::RememberTokenService` · crates/suprnova-magnetar/src/sessions/remember.rs:174 (also `magnetar::sessions::remember::RememberTokenService`)
  - Implemented here by: `sessions::RememberService`
  - [ ] fn `magnetar::sessions::RememberTokenService::default_lifetime` · crates/suprnova-magnetar/src/sessions/remember.rs:176 (required)
  - [ ] fn `magnetar::sessions::RememberTokenService::issue_at_epoch` · crates/suprnova-magnetar/src/sessions/remember.rs:178 (required)
  - [ ] fn `magnetar::sessions::RememberTokenService::rotate_at_epoch` · crates/suprnova-magnetar/src/sessions/remember.rs:186 (required)
  - [ ] fn `magnetar::sessions::RememberTokenService::attempt_rotation_at_epoch` · crates/suprnova-magnetar/src/sessions/remember.rs:199 (provided)
  - [ ] fn `magnetar::sessions::RememberTokenService::revoke_selector` · crates/suprnova-magnetar/src/sessions/remember.rs:228 (provided)
  - [ ] fn `magnetar::sessions::RememberTokenService::revoke_all_for_user` · crates/suprnova-magnetar/src/sessions/remember.rs:236 (required)

## storage

### `magnetar::storage`

- [ ] struct `magnetar::storage::AuthTransaction` · crates/suprnova-magnetar/src/storage/mod.rs:57
  - [ ] fn `magnetar::storage::AuthTransaction::new` · crates/suprnova-magnetar/src/storage/mod.rs:63
  - [ ] fn `magnetar::storage::AuthTransaction::connection` · crates/suprnova-magnetar/src/storage/mod.rs:68
- [ ] struct `magnetar::storage::Store` · crates/suprnova-magnetar/src/storage/mod.rs:117
  - [ ] fn `magnetar::storage::Store::new` · crates/suprnova-magnetar/src/storage/mod.rs:126
  - [ ] fn `magnetar::storage::Store::database` · crates/suprnova-magnetar/src/storage/mod.rs:134

### `magnetar::storage::accounts`

- [ ] struct `magnetar::storage::LinkedAccountRecord` · crates/suprnova-magnetar/src/storage/accounts.rs:27 (also `magnetar::storage::accounts::LinkedAccountRecord`)
  - Public fields: `account_id`, `user_id`, `provider`, `provider_account_id`
- [ ] struct `magnetar::storage::NewLinkedAccount` · crates/suprnova-magnetar/src/storage/accounts.rs:15 (also `magnetar::storage::accounts::NewLinkedAccount`)
  - Public fields: `user_id`, `provider`, `provider_account_id`
- [ ] trait `magnetar::storage::LinkedAccountInitializer` · crates/suprnova-magnetar/src/storage/accounts.rs:83 (also `magnetar::storage::accounts::LinkedAccountInitializer`)
  - Implemented here by: `storage::SeaOrmStorage`
  - [ ] fn `magnetar::storage::LinkedAccountInitializer::initialize` · crates/suprnova-magnetar/src/storage/accounts.rs:86 (required)
- [ ] trait `magnetar::storage::LinkedAccountStore` · crates/suprnova-magnetar/src/storage/accounts.rs:46 (also `magnetar::storage::accounts::LinkedAccountStore`)
  - Implemented here by: `storage::SeaOrmStorage`
  - [ ] fn `magnetar::storage::LinkedAccountStore::create` · crates/suprnova-magnetar/src/storage/accounts.rs:58 (required)
  - [ ] fn `magnetar::storage::LinkedAccountStore::validate_actor` · crates/suprnova-magnetar/src/storage/accounts.rs:67 (required)
  - [ ] fn `magnetar::storage::LinkedAccountStore::find_by_provider_subject` · crates/suprnova-magnetar/src/storage/accounts.rs:70 (required)

### `magnetar::storage::ceremonies`

- [ ] struct `magnetar::storage::CeremonyRecord` · crates/suprnova-magnetar/src/storage/ceremonies.rs:29 (also `magnetar::storage::ceremonies::CeremonyRecord`)
  - Public fields: `id`, `selector`, `kind`, `state`, `expires_at`, `payload`
- [ ] struct `magnetar::storage::NewCeremony` · crates/suprnova-magnetar/src/storage/ceremonies.rs:14 (also `magnetar::storage::ceremonies::NewCeremony`)
  - Public fields: `selector`, `kind`, `state`, `payload`, `expires_at`
- [ ] trait `magnetar::storage::CeremonyStore` · crates/suprnova-magnetar/src/storage/ceremonies.rs:46 (also `magnetar::storage::ceremonies::CeremonyStore`)
  - Implemented here by: `storage::SeaOrmStorage`
  - [ ] fn `magnetar::storage::CeremonyStore::create` · crates/suprnova-magnetar/src/storage/ceremonies.rs:48 (required)
  - [ ] fn `magnetar::storage::CeremonyStore::consume` · crates/suprnova-magnetar/src/storage/ceremonies.rs:50 (required)
  - [ ] fn `magnetar::storage::CeremonyStore::peek` · crates/suprnova-magnetar/src/storage/ceremonies.rs:52 (required)
  - [ ] fn `magnetar::storage::CeremonyStore::transition` · crates/suprnova-magnetar/src/storage/ceremonies.rs:54 (required)
  - [ ] fn `magnetar::storage::CeremonyStore::transition_and_consume` · crates/suprnova-magnetar/src/storage/ceremonies.rs:66 (provided)
  - [ ] fn `magnetar::storage::CeremonyStore::transition_and_consume_exact` · crates/suprnova-magnetar/src/storage/ceremonies.rs:87 (provided)

### `magnetar::storage::credential_writes` (private module; items are public through re-exports)

- [ ] fn `magnetar::storage::fenced_credential_write` · crates/suprnova-magnetar/src/storage/credential_writes.rs:110
- [ ] struct `magnetar::storage::CredentialActor` · crates/suprnova-magnetar/src/storage/credential_writes.rs:16
  - [ ] fn `magnetar::storage::CredentialActor::from_session` · crates/suprnova-magnetar/src/storage/credential_writes.rs:30
  - [ ] fn `magnetar::storage::CredentialActor::from_verified_primary` · crates/suprnova-magnetar/src/storage/credential_writes.rs:42
  - [ ] fn `magnetar::storage::CredentialActor::user_id` · crates/suprnova-magnetar/src/storage/credential_writes.rs:59
  - [ ] fn `magnetar::storage::CredentialActor::issuance_epoch` · crates/suprnova-magnetar/src/storage/credential_writes.rs:65
  - [ ] fn `magnetar::storage::CredentialActor::opaque_session_id` · crates/suprnova-magnetar/src/storage/credential_writes.rs:71
  - [ ] fn `magnetar::storage::CredentialActor::expires_at` · crates/suprnova-magnetar/src/storage/credential_writes.rs:77

### `magnetar::storage::device`

- [ ] struct `magnetar::storage::DeviceRecord` · crates/suprnova-magnetar/src/storage/device.rs:14 (also `magnetar::storage::device::DeviceRecord`)
  - Public fields: `id`, `selector`, `state`, `expires_at`, `payload`
- [ ] trait `magnetar::storage::DeviceStore` · crates/suprnova-magnetar/src/storage/device.rs:41 (also `magnetar::storage::device::DeviceStore`)
  - Implemented here by: `storage::SeaOrmStorage`
  - [ ] fn `magnetar::storage::DeviceStore::peek_device` · crates/suprnova-magnetar/src/storage/device.rs:43 (required)
  - [ ] fn `magnetar::storage::DeviceStore::transition_device` · crates/suprnova-magnetar/src/storage/device.rs:45 (required)
  - [ ] fn `magnetar::storage::DeviceStore::approve_device` · crates/suprnova-magnetar/src/storage/device.rs:47 (required)
  - [ ] fn `magnetar::storage::DeviceStore::deny_device` · crates/suprnova-magnetar/src/storage/device.rs:49 (required)

### `magnetar::storage::lockout`

- [ ] struct `magnetar::storage::AttemptFinalization` · crates/suprnova-magnetar/src/storage/lockout.rs:48 (also `magnetar::storage::lockout::AttemptFinalization`)
  - Public fields: `stats`, `locked_event`
- [ ] struct `magnetar::storage::AttemptReservation` · crates/suprnova-magnetar/src/storage/lockout.rs:33 (also `magnetar::storage::lockout::AttemptReservation`)
  - Public fields: `admitted`, `stats`, `reservation_id`, `locked_event`
- [ ] struct `magnetar::storage::AttemptStats` · crates/suprnova-magnetar/src/storage/lockout.rs:24 (also `magnetar::storage::lockout::AttemptStats`)
  - Public fields: `count`, `latest_at`
- [ ] trait `magnetar::storage::LockoutStore` · crates/suprnova-magnetar/src/storage/lockout.rs:111 (also `magnetar::storage::lockout::LockoutStore`)
  - Implemented here by: `storage::SeaOrmStorage`
  - [ ] fn `magnetar::storage::LockoutStore::record_attempt_and_stats` · crates/suprnova-magnetar/src/storage/lockout.rs:114 (required)
  - [ ] fn `magnetar::storage::LockoutStore::admit_attempt_and_stats` · crates/suprnova-magnetar/src/storage/lockout.rs:128 (provided)
  - [ ] fn `magnetar::storage::LockoutStore::cancel_attempt_reservation` · crates/suprnova-magnetar/src/storage/lockout.rs:143 (provided)
  - [ ] fn `magnetar::storage::LockoutStore::finalize_attempt_reservation` · crates/suprnova-magnetar/src/storage/lockout.rs:160 (provided)
  - [ ] fn `magnetar::storage::LockoutStore::reset_admitted_attempts` · crates/suprnova-magnetar/src/storage/lockout.rs:189 (provided)
  - [ ] fn `magnetar::storage::LockoutStore::attempt_stats` · crates/suprnova-magnetar/src/storage/lockout.rs:202 (required)
  - [ ] fn `magnetar::storage::LockoutStore::clear_attempts` · crates/suprnova-magnetar/src/storage/lockout.rs:208 (required)
  - [ ] fn `magnetar::storage::LockoutStore::cleanup_attempts_before` · crates/suprnova-magnetar/src/storage/lockout.rs:211 (required)

### `magnetar::storage::methods`

- [ ] trait `magnetar::storage::MethodStore` · crates/suprnova-magnetar/src/storage/methods.rs:17 (also `magnetar::storage::methods::MethodStore`)
  - Implemented here by: `storage::SeaOrmStorage`
  - [ ] fn `magnetar::storage::MethodStore::census` · crates/suprnova-magnetar/src/storage/methods.rs:20 (required)
  - [ ] fn `magnetar::storage::MethodStore::remove_password_if_not_last` · crates/suprnova-magnetar/src/storage/methods.rs:23 (required)
  - [ ] fn `magnetar::storage::MethodStore::remove_passkey_if_not_last` · crates/suprnova-magnetar/src/storage/methods.rs:26 (required)
  - [ ] fn `magnetar::storage::MethodStore::remove_linked_account_if_not_last` · crates/suprnova-magnetar/src/storage/methods.rs:33 (required)

### `magnetar::storage::migrations`

- [ ] struct `magnetar::storage::migrations::MigrationReport` · crates/suprnova-magnetar/src/storage/migrations/mod.rs:17
  - Public fields: `statements`

### `magnetar::storage::migrations::mysql`

- [ ] fn `magnetar::storage::migrations::mysql::apply` · crates/suprnova-magnetar/src/storage/migrations/mysql.rs:13

### `magnetar::storage::migrations::postgres`

- [ ] fn `magnetar::storage::migrations::postgres::apply` · crates/suprnova-magnetar/src/storage/migrations/postgres.rs:12

### `magnetar::storage::migrations::sqlite`

- [ ] fn `magnetar::storage::migrations::sqlite::apply` · crates/suprnova-magnetar/src/storage/migrations/sqlite.rs:12

### `magnetar::storage::passkeys`

- [ ] struct `magnetar::storage::PasskeyRow` · crates/suprnova-magnetar/src/storage/passkeys.rs:23 (also `magnetar::storage::passkeys::PasskeyRow`)
  - Public fields: `passkey_id`, `user_id`, `credential_id`, `envelope_json`, `created_at`
- [ ] trait `magnetar::storage::PasskeyStore` · crates/suprnova-magnetar/src/storage/passkeys.rs:38 (also `magnetar::storage::passkeys::PasskeyStore`)
  - Implemented here by: `storage::SeaOrmStorage`
  - [ ] fn `magnetar::storage::PasskeyStore::insert_passkey` · crates/suprnova-magnetar/src/storage/passkeys.rs:41 (required)
  - [ ] fn `magnetar::storage::PasskeyStore::passkeys_for_user` · crates/suprnova-magnetar/src/storage/passkeys.rs:48 (required)
  - [ ] fn `magnetar::storage::PasskeyStore::find_user_by_credential` · crates/suprnova-magnetar/src/storage/passkeys.rs:51 (required)
  - [ ] fn `magnetar::storage::PasskeyStore::update_passkey_envelope` · crates/suprnova-magnetar/src/storage/passkeys.rs:56 (required)

### `magnetar::storage::provider_tokens`

- [ ] fn `magnetar::storage::provider_tokens::exchange_claim_id` · crates/suprnova-magnetar/src/storage/provider_tokens.rs:34
- [ ] fn `magnetar::storage::provider_tokens::is_exchange_claim_id` · crates/suprnova-magnetar/src/storage/provider_tokens.rs:49
- [ ] struct `magnetar::storage::CommitProviderToken` · crates/suprnova-magnetar/src/storage/provider_tokens.rs:88 (also `magnetar::storage::provider_tokens::CommitProviderToken`)
  - Public fields: `access_ciphertext`, `refresh_ciphertext`, `raw_payload_ciphertext`, `token_type`, `scopes`, `access_expires_at`, `new_generation`
- [ ] struct `magnetar::storage::NewProviderToken` · crates/suprnova-magnetar/src/storage/provider_tokens.rs:79 (also `magnetar::storage::provider_tokens::NewProviderToken`)
  - Public fields: `id`, `provider`
- [ ] struct `magnetar::storage::ProviderTokenRow` · crates/suprnova-magnetar/src/storage/provider_tokens.rs:113 (also `magnetar::storage::provider_tokens::ProviderTokenRow`)
  - Public fields: `id`, `provider`, `access_ciphertext`, `refresh_ciphertext`, `raw_payload_ciphertext`, `token_type`, `scopes`, `access_expires_at`, `generation`, `claim_id`, `claim_deadline`, `revoked_at`, `revoked_reused`, `created_at`
  - [ ] fn `magnetar::storage::ProviderTokenRow::has_live_claim` · crates/suprnova-magnetar/src/storage/provider_tokens.rs:149
- [ ] trait `magnetar::storage::ProviderTokenStore` · crates/suprnova-magnetar/src/storage/provider_tokens.rs:165 (also `magnetar::storage::provider_tokens::ProviderTokenStore`)
  - Implemented here by: `storage::SeaOrmStorage`
  - [ ] fn `magnetar::storage::ProviderTokenStore::create_if_missing` · crates/suprnova-magnetar/src/storage/provider_tokens.rs:168 (required)
  - [ ] fn `magnetar::storage::ProviderTokenStore::read` · crates/suprnova-magnetar/src/storage/provider_tokens.rs:171 (required)
  - [ ] fn `magnetar::storage::ProviderTokenStore::claim` · crates/suprnova-magnetar/src/storage/provider_tokens.rs:178 (required)
  - [ ] fn `magnetar::storage::ProviderTokenStore::heartbeat` · crates/suprnova-magnetar/src/storage/provider_tokens.rs:188 (required)
  - [ ] fn `magnetar::storage::ProviderTokenStore::mark_exchange_started` · crates/suprnova-magnetar/src/storage/provider_tokens.rs:201 (provided)
  - [ ] fn `magnetar::storage::ProviderTokenStore::commit` · crates/suprnova-magnetar/src/storage/provider_tokens.rs:215 (required)
  - [ ] fn `magnetar::storage::ProviderTokenStore::commit_exchange` · crates/suprnova-magnetar/src/storage/provider_tokens.rs:227 (provided)
  - [ ] fn `magnetar::storage::ProviderTokenStore::mark_revoked_by_claim` · crates/suprnova-magnetar/src/storage/provider_tokens.rs:240 (required)
  - [ ] fn `magnetar::storage::ProviderTokenStore::mark_revoked_by_exchange` · crates/suprnova-magnetar/src/storage/provider_tokens.rs:250 (provided)
  - [ ] fn `magnetar::storage::ProviderTokenStore::revoke_family_if_unrevoked` · crates/suprnova-magnetar/src/storage/provider_tokens.rs:264 (required)
  - [ ] fn `magnetar::storage::ProviderTokenStore::delete` · crates/suprnova-magnetar/src/storage/provider_tokens.rs:269 (required)

### `magnetar::storage::seaorm` (private module; items are public through re-exports)

- [ ] struct `magnetar::storage::SeaOrmStorage` · crates/suprnova-magnetar/src/storage/seaorm.rs:12
  - [ ] fn `magnetar::storage::SeaOrmStorage::new` · crates/suprnova-magnetar/src/storage/seaorm.rs:19
  - [ ] fn `magnetar::storage::SeaOrmStorage::database` · crates/suprnova-magnetar/src/storage/seaorm.rs:27
  - [ ] fn `magnetar::storage::SeaOrmStorage::into_database` · crates/suprnova-magnetar/src/storage/seaorm.rs:32

### `magnetar::storage::tokens`

- [ ] struct `magnetar::storage::ConsumedToken` · crates/suprnova-magnetar/src/storage/tokens.rs:56 (also `magnetar::storage::tokens::ConsumedToken`)
  - Public fields: `token_id`, `user_id`, `purpose`, `expires_at`
- [ ] struct `magnetar::storage::IssuedToken` · crates/suprnova-magnetar/src/storage/tokens.rs:45 (also `magnetar::storage::tokens::IssuedToken`)
  - Public fields: `plaintext`, `token_id`, `expires_at`
- [ ] struct `magnetar::storage::IssueToken` · crates/suprnova-magnetar/src/storage/tokens.rs:23 (also `magnetar::storage::tokens::IssueToken`)
  - Public fields: `user_id`, `purpose`, `ttl`
- [ ] struct `magnetar::storage::PasswordResetCommit` · crates/suprnova-magnetar/src/storage/tokens.rs:104 (also `magnetar::storage::tokens::PasswordResetCommit`)
  - Public fields: `user_id`, `auth_epoch`, `revoked_sessions`
- [ ] struct `magnetar::storage::PasswordResetInput` · crates/suprnova-magnetar/src/storage/tokens.rs:75 (also `magnetar::storage::tokens::PasswordResetInput`)
  - Public fields: `token`, `new_password_hash`, `expected_user_id`
  - [ ] fn `magnetar::storage::PasswordResetInput::new` · crates/suprnova-magnetar/src/storage/tokens.rs:86
  - [ ] fn `magnetar::storage::PasswordResetInput::expecting_user` · crates/suprnova-magnetar/src/storage/tokens.rs:96
- [ ] struct `magnetar::storage::PresentedToken` · crates/suprnova-magnetar/src/storage/tokens.rs:34 (also `magnetar::storage::tokens::PresentedToken`)
  - Public tuple fields: 1
  - [ ] fn `magnetar::storage::PresentedToken::new` · crates/suprnova-magnetar/src/storage/tokens.rs:38
- [ ] trait `magnetar::storage::PasswordResetStore` · crates/suprnova-magnetar/src/storage/tokens.rs:133 (also `magnetar::storage::tokens::PasswordResetStore`)
  - Implemented here by: `storage::SeaOrmStorage`
  - [ ] fn `magnetar::storage::PasswordResetStore::apply_password_reset` · crates/suprnova-magnetar/src/storage/tokens.rs:136 (required)
- [ ] trait `magnetar::storage::TokenStore` · crates/suprnova-magnetar/src/storage/tokens.rs:115 (also `magnetar::storage::tokens::TokenStore`)
  - Implemented here by: `storage::SeaOrmStorage`
  - [ ] fn `magnetar::storage::TokenStore::issue` · crates/suprnova-magnetar/src/storage/tokens.rs:117 (required)
  - [ ] fn `magnetar::storage::TokenStore::check` · crates/suprnova-magnetar/src/storage/tokens.rs:119 (required)
  - [ ] fn `magnetar::storage::TokenStore::consume` · crates/suprnova-magnetar/src/storage/tokens.rs:121 (required)
  - [ ] fn `magnetar::storage::TokenStore::consume_in` · crates/suprnova-magnetar/src/storage/tokens.rs:123 (required)
- [ ] const `magnetar::storage::PASSWORD_RESET_PURPOSE` · crates/suprnova-magnetar/src/storage/tokens.rs:19 (also `magnetar::storage::tokens::PASSWORD_RESET_PURPOSE`)

### `magnetar::storage::users`

- [ ] struct `magnetar::storage::NewUser` · crates/suprnova-magnetar/src/storage/users.rs:43 (also `magnetar::storage::users::NewUser`)
  - Public fields: `email`, `password_hash`
- [ ] struct `magnetar::storage::UserRecord` · crates/suprnova-magnetar/src/storage/users.rs:25 (also `magnetar::storage::users::UserRecord`)
  - Public fields: `user_id`, `email`, `password_hash`, `email_verified_at`, `locked_at`, `auth_epoch`
- [ ] trait `magnetar::storage::UserStore` · crates/suprnova-magnetar/src/storage/users.rs:52 (also `magnetar::storage::users::UserStore`)
  - Implemented here by: `storage::SeaOrmStorage`
  - [ ] fn `magnetar::storage::UserStore::find_by_email` · crates/suprnova-magnetar/src/storage/users.rs:54 (required)
  - [ ] fn `magnetar::storage::UserStore::find_by_id` · crates/suprnova-magnetar/src/storage/users.rs:56 (required)
  - [ ] fn `magnetar::storage::UserStore::create_user` · crates/suprnova-magnetar/src/storage/users.rs:58 (required)
  - [ ] fn `magnetar::storage::UserStore::set_password_hash` · crates/suprnova-magnetar/src/storage/users.rs:60 (required)
  - [ ] fn `magnetar::storage::UserStore::mark_email_verified` · crates/suprnova-magnetar/src/storage/users.rs:62 (required)
  - [ ] fn `magnetar::storage::UserStore::lock_if_unlocked_by_email` · crates/suprnova-magnetar/src/storage/users.rs:67 (required)
  - [ ] fn `magnetar::storage::UserStore::set_locked_at_by_email` · crates/suprnova-magnetar/src/storage/users.rs:75 (required)

## two_factor

### `magnetar::two_factor` (feature: `two-factor`, off by default)

- [ ] struct `magnetar::two_factor::EnrollmentResponse` · crates/suprnova-magnetar/src/two_factor/mod.rs:53
  - Public fields: `otpauth_url`, `qr_code_svg`, `recovery_codes`
- [ ] struct `magnetar::two_factor::PreparedTwoFactorProof` · crates/suprnova-magnetar/src/two_factor/mod.rs:105
- [ ] struct `magnetar::two_factor::TwoFactorConfig` · crates/suprnova-magnetar/src/two_factor/mod.rs:35
  - Public fields: `issuer`
- [ ] struct `magnetar::two_factor::TwoFactorService` · crates/suprnova-magnetar/src/two_factor/mod.rs:125
  - [ ] fn `magnetar::two_factor::TwoFactorService::new` · crates/suprnova-magnetar/src/two_factor/mod.rs:135
  - [ ] fn `magnetar::two_factor::TwoFactorService::enroll` · crates/suprnova-magnetar/src/two_factor/mod.rs:156
  - [ ] fn `magnetar::two_factor::TwoFactorService::re_enroll` · crates/suprnova-magnetar/src/two_factor/mod.rs:179
  - [ ] fn `magnetar::two_factor::TwoFactorService::confirm` · crates/suprnova-magnetar/src/two_factor/mod.rs:219
  - [ ] fn `magnetar::two_factor::TwoFactorService::verify` · crates/suprnova-magnetar/src/two_factor/mod.rs:255
  - [ ] fn `magnetar::two_factor::TwoFactorService::consume_recovery_code` · crates/suprnova-magnetar/src/two_factor/mod.rs:276
  - [ ] fn `magnetar::two_factor::TwoFactorService::regenerate_recovery_codes` · crates/suprnova-magnetar/src/two_factor/mod.rs:329
  - [ ] fn `magnetar::two_factor::TwoFactorService::disable` · crates/suprnova-magnetar/src/two_factor/mod.rs:369
  - [ ] fn `magnetar::two_factor::TwoFactorService::is_enabled` · crates/suprnova-magnetar/src/two_factor/mod.rs:374

### `magnetar::two_factor::recovery` (feature: `two-factor`, off by default)

- [ ] fn `magnetar::two_factor::recovery::find_constant_time` · crates/suprnova-magnetar/src/two_factor/recovery.rs:36
- [ ] fn `magnetar::two_factor::recovery::generate` · crates/suprnova-magnetar/src/two_factor/recovery.rs:17
- [ ] const `magnetar::two_factor::recovery::RECOVERY_CODE_COUNT` · crates/suprnova-magnetar/src/two_factor/recovery.rs:13

### `magnetar::two_factor::store` (feature: `two-factor`, off by default)

- [ ] struct `magnetar::two_factor::TwoFactorRow` · crates/suprnova-magnetar/src/two_factor/store.rs:18 (also `magnetar::two_factor::store::TwoFactorRow`)
  - Public fields: `user_id`, `secret`, `recovery_codes`, `enrollment_auth_epoch`, `enrollment_session_id`, `enrollment_expires_at`, `rotation_pending`, `confirmed_at`, `last_used_timestep`
- [ ] enum `magnetar::two_factor::TwoFactorProofClaim` · crates/suprnova-magnetar/src/two_factor/store.rs:45 (also `magnetar::two_factor::store::TwoFactorProofClaim`)
  - Variants: `Invalid`, `Totp`, `Recovery`
- [ ] trait `magnetar::two_factor::TwoFactorStore` · crates/suprnova-magnetar/src/two_factor/store.rs:80 (also `magnetar::two_factor::store::TwoFactorStore`)
  - Implemented here by: `default_schema::sql_two_factor::SqlTwoFactorStore`
  - [ ] fn `magnetar::two_factor::TwoFactorStore::find_enrollment` · crates/suprnova-magnetar/src/two_factor/store.rs:82 (required)
  - [ ] fn `magnetar::two_factor::TwoFactorStore::begin_enrollment` · crates/suprnova-magnetar/src/two_factor/store.rs:85 (required)
  - [ ] fn `magnetar::two_factor::TwoFactorStore::set_confirmed` · crates/suprnova-magnetar/src/two_factor/store.rs:92 (required)
  - [ ] fn `magnetar::two_factor::TwoFactorStore::claim_timestep` · crates/suprnova-magnetar/src/two_factor/store.rs:97 (required)
  - [ ] fn `magnetar::two_factor::TwoFactorStore::swap_recovery_codes` · crates/suprnova-magnetar/src/two_factor/store.rs:100 (required)
  - [ ] fn `magnetar::two_factor::TwoFactorStore::rotate_enrollment` · crates/suprnova-magnetar/src/two_factor/store.rs:107 (required)
  - [ ] fn `magnetar::two_factor::TwoFactorStore::regenerate_recovery_codes` · crates/suprnova-magnetar/src/two_factor/store.rs:115 (required)
  - [ ] fn `magnetar::two_factor::TwoFactorStore::delete_enrollment` · crates/suprnova-magnetar/src/two_factor/store.rs:123 (required)

### `magnetar::two_factor::totp` (feature: `two-factor`, off by default)

- [ ] fn `magnetar::two_factor::totp::matched_step` · crates/suprnova-magnetar/src/two_factor/totp.rs:88
- [ ] fn `magnetar::two_factor::totp::provision` · crates/suprnova-magnetar/src/two_factor/totp.rs:52
- [ ] fn `magnetar::two_factor::totp::timestep_at` · crates/suprnova-magnetar/src/two_factor/totp.rs:77
- [ ] struct `magnetar::two_factor::totp::ProvisionedSecret` · crates/suprnova-magnetar/src/two_factor/totp.rs:22
  - Public fields: `secret_b32`, `otpauth_url`, `qr_code_svg`
- [ ] const `magnetar::two_factor::totp::SKEW_STEPS` · crates/suprnova-magnetar/src/two_factor/totp.rs:19
- [ ] const `magnetar::two_factor::totp::STEP_SECONDS` · crates/suprnova-magnetar/src/two_factor/totp.rs:17

## Public but unnameable

Reachable from the public API (as a return type, field or supertrait) but not importable by any public path.

- [ ] trait `magnetar::sessions::sealed::Sealed` · crates/suprnova-magnetar/src/sessions/mod.rs:135 (sealed trait: a supertrait that stops implementations outside the crate)
