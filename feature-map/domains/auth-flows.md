# Feature map: `manual/auth-flows.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 108 checked.

## Endpoints and tables

### Magnetar default schema

- [ ] table `app_users (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:34
- [ ] table `auth_sessions (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:46
- [ ] table `auth_methods (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:57
- [ ] table `auth_linked_accounts (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:64
- [ ] table `auth_tokens (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:72
- [ ] table `auth_ceremonies (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:82
- [ ] table `auth_lockouts (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:91
- [ ] table `auth_two_factor (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:100
- [ ] table `auth_remember_tokens (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:113
- [ ] table `auth_provider_tokens (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:121
- [ ] table `auth_lifecycle_deliveries (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:137
- [ ] table `auth_migration_runs (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:143
- [ ] table `auth_migration_identities (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:148
- [ ] table `magnetar_migration_state (magnetar)` · crates/suprnova-magnetar/src/default_schema.rs:154

### framework fn `auth_flows::create_auth_flow_tokens_table`

- [ ] table `auth_flow_tokens (framework)` · framework/src/auth_flows/token_store.rs:119

### framework migration `auth_flows::two_factor::migration::Migration`

- [ ] table `two_factor_credentials (framework)` · framework/src/auth_flows/two_factor/migration.rs:42

## Rust API: suprnova

### `suprnova::auth_flows::brute_force` (feature: `database-sqlite` or `database-postgres` or `database-mysql`)

- [ ] struct `suprnova::BruteForce` · framework/src/auth_flows/brute_force.rs:105 (also `suprnova::auth_flows::BruteForce`, `suprnova::auth_flows::brute_force::BruteForce`)
  - [ ] fn `suprnova::BruteForce::record_failed_attempt` · framework/src/auth_flows/brute_force.rs:172
  - [ ] fn `suprnova::BruteForce::get_lockout_status` · framework/src/auth_flows/brute_force.rs:198
  - [ ] fn `suprnova::BruteForce::is_locked` · framework/src/auth_flows/brute_force.rs:204
  - [ ] fn `suprnova::BruteForce::reset_attempts` · framework/src/auth_flows/brute_force.rs:216
  - [ ] fn `suprnova::BruteForce::reset_admitted_attempt` · framework/src/auth_flows/brute_force.rs:222
  - [ ] fn `suprnova::BruteForce::unlock_account` · framework/src/auth_flows/brute_force.rs:237
- [ ] struct `suprnova::LoginThrottleMiddleware` · framework/src/auth_flows/brute_force.rs:396 (also `suprnova::auth_flows::LoginThrottleMiddleware`, `suprnova::auth_flows::brute_force::LoginThrottleMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::LoginThrottleMiddleware::new` · framework/src/auth_flows/brute_force.rs:410
  - [ ] fn `suprnova::LoginThrottleMiddleware::on_backend_error` · framework/src/auth_flows/brute_force.rs:429
- [ ] enum `suprnova::auth_flows::BackendErrorPolicy` · framework/src/auth_flows/brute_force.rs:376 (also `suprnova::auth_flows::LoginThrottleBackendErrorPolicy`, `suprnova::auth_flows::brute_force::BackendErrorPolicy`)
  - Variants: `FailOpen`, `FailClosed`

### `suprnova::auth_flows::email_verified_middleware`

- [ ] struct `suprnova::EnsureEmailVerifiedMiddleware` · framework/src/auth_flows/email_verified_middleware.rs:57 (also `suprnova::auth_flows::EnsureEmailVerifiedMiddleware`, `suprnova::auth_flows::email_verified_middleware::EnsureEmailVerifiedMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::EnsureEmailVerifiedMiddleware::new` · framework/src/auth_flows/email_verified_middleware.rs:70
  - [ ] fn `suprnova::EnsureEmailVerifiedMiddleware::redirect_to` · framework/src/auth_flows/email_verified_middleware.rs:79

### `suprnova::auth_flows::email_verify`

- [ ] struct `suprnova::EmailVerification` · framework/src/auth_flows/email_verify.rs:79 (also `suprnova::auth_flows::EmailVerification`, `suprnova::auth_flows::email_verify::EmailVerification`)
  - [ ] fn `suprnova::EmailVerification::send_link` · framework/src/auth_flows/email_verify.rs:96
  - [ ] fn `suprnova::EmailVerification::resend` · framework/src/auth_flows/email_verify.rs:164
  - [ ] fn `suprnova::EmailVerification::check` · framework/src/auth_flows/email_verify.rs:185
  - [ ] fn `suprnova::EmailVerification::verify` · framework/src/auth_flows/email_verify.rs:215

### `suprnova::auth_flows::events`

- [ ] struct `suprnova::auth_flows::AccountLocked` · framework/src/auth_flows/events.rs:109 (also `suprnova::auth_flows::events::AccountLocked`)
  - Public fields: `email`, `failed_attempts`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::auth_flows::AccountUnlocked` · framework/src/auth_flows/events.rs:133 (also `suprnova::auth_flows::events::AccountUnlocked`)
  - Public fields: `email`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::auth_flows::EmailVerified` · framework/src/auth_flows/events.rs:39 (also `suprnova::auth_flows::events::EmailVerified`)
  - Public fields: `user_id`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::auth_flows::PasswordResetCompleted` · framework/src/auth_flows/events.rs:86 (also `suprnova::auth_flows::events::PasswordResetCompleted`)
  - Public fields: `user_id`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::PasswordResetLinkSent` · framework/src/auth_flows/events.rs:65 (also `suprnova::auth_flows::PasswordResetLinkSent`, `suprnova::auth_flows::events::PasswordResetLinkSent`)
  - Public fields: `user_id`, `email`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::TwoFactorChallenged` · framework/src/auth_flows/events.rs:174 (also `suprnova::auth_flows::TwoFactorChallenged`, `suprnova::auth_flows::events::TwoFactorChallenged`)
  - Public fields: `user_id`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::TwoFactorChallengeFailed` · framework/src/auth_flows/events.rs:199 (also `suprnova::auth_flows::TwoFactorChallengeFailed`, `suprnova::auth_flows::events::TwoFactorChallengeFailed`)
  - Public fields: `user_id`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::auth_flows::TwoFactorDisabled` · framework/src/auth_flows/events.rs:219 (also `suprnova::auth_flows::events::TwoFactorDisabled`)
  - Public fields: `user_id`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::auth_flows::TwoFactorEnrolled` · framework/src/auth_flows/events.rs:152 (also `suprnova::auth_flows::events::TwoFactorEnrolled`)
  - Public fields: `user_id`
  - Implements: `suprnova::Event`

### `suprnova::auth_flows::mail`

- [ ] struct `suprnova::EmailVerificationMail` · framework/src/auth_flows/mail.rs:66 (also `suprnova::auth_flows::EmailVerificationMail`, `suprnova::auth_flows::mail::EmailVerificationMail`)
  - Public fields: `to_address`, `user_name`, `verification_link`, `app_name`, `from_address`
  - Implements: `suprnova::Mailable`
- [ ] struct `suprnova::PasswordChangedMail` · framework/src/auth_flows/mail.rs:206 (also `suprnova::auth_flows::PasswordChangedMail`, `suprnova::auth_flows::mail::PasswordChangedMail`)
  - Public fields: `to_address`, `user_name`, `app_name`, `from_address`
  - Implements: `suprnova::Mailable`
- [ ] struct `suprnova::PasswordResetMail` · framework/src/auth_flows/mail.rs:139 (also `suprnova::auth_flows::PasswordResetMail`, `suprnova::auth_flows::mail::PasswordResetMail`)
  - Public fields: `to_address`, `user_name`, `reset_link`, `app_name`, `from_address`
  - Implements: `suprnova::Mailable`

### `suprnova::auth_flows::password_reset`

- [ ] struct `suprnova::PasswordReset` · framework/src/auth_flows/password_reset.rs:67 (also `suprnova::auth_flows::PasswordReset`, `suprnova::auth_flows::password_reset::PasswordReset`)
  - [ ] fn `suprnova::PasswordReset::send_link` · framework/src/auth_flows/password_reset.rs:114
  - [ ] fn `suprnova::PasswordReset::check` · framework/src/auth_flows/password_reset.rs:156
  - [ ] fn `suprnova::PasswordReset::complete` · framework/src/auth_flows/password_reset.rs:199
  - [ ] fn `suprnova::PasswordReset::complete_with_outcome` · framework/src/auth_flows/password_reset.rs:223
- [ ] struct `suprnova::auth_flows::password_reset::PasswordResetOutcome` · framework/src/auth_flows/password_reset.rs:80
  - Public fields: `user_id`, `sessions_revoked`, `remember_tokens_revoked`

### `suprnova::auth_flows::token_store::entity`

- [ ] struct `suprnova::auth_flows::token_store::entity::ActiveModel` · framework/src/auth_flows/token_store.rs:386
  - Public fields: `id`, `user_id`, `token_hash`, `purpose`, `expires_at`, `used_at`, `created_at`
- [ ] struct `suprnova::auth_flows::token_store::entity::ColumnIter` · framework/src/auth_flows/token_store.rs:386
- [ ] struct `suprnova::auth_flows::token_store::entity::Entity` · framework/src/auth_flows/token_store.rs:386
- [ ] struct `suprnova::auth_flows::token_store::entity::Model` · framework/src/auth_flows/token_store.rs:388
  - Public fields: `id`, `user_id`, `token_hash`, `purpose`, `expires_at`, `used_at`, `created_at`
  - [ ] fn `suprnova::auth_flows::token_store::entity::Model::into_ex` · framework/src/auth_flows/token_store.rs:386
- [ ] struct `suprnova::auth_flows::token_store::entity::PrimaryKeyIter` · framework/src/auth_flows/token_store.rs:386
- [ ] struct `suprnova::auth_flows::token_store::entity::RelationIter` · framework/src/auth_flows/token_store.rs:409
- [ ] enum `suprnova::auth_flows::token_store::entity::Column` · framework/src/auth_flows/token_store.rs:386
  - Variants: `Id`, `UserId`, `TokenHash`, `Purpose`, `ExpiresAt`, `UsedAt`, `CreatedAt`
- [ ] enum `suprnova::auth_flows::token_store::entity::PrimaryKey` · framework/src/auth_flows/token_store.rs:386
  - Variants: `Id`
- [ ] enum `suprnova::auth_flows::token_store::entity::Relation` · framework/src/auth_flows/token_store.rs:410

### `suprnova::auth_flows::token_store`

- [ ] fn `suprnova::auth_flows::create_auth_flow_tokens_table` · framework/src/auth_flows/token_store.rs:81 (also `suprnova::auth_flows::token_store::create_auth_flow_tokens_table`)
- [ ] struct `suprnova::auth_flows::token_store::TokenStore` · framework/src/auth_flows/token_store.rs:162
  - [ ] fn `suprnova::auth_flows::token_store::TokenStore::issue` · framework/src/auth_flows/token_store.rs:171
  - [ ] fn `suprnova::auth_flows::token_store::TokenStore::check` · framework/src/auth_flows/token_store.rs:203
  - [ ] fn `suprnova::auth_flows::token_store::TokenStore::owner` · framework/src/auth_flows/token_store.rs:221
  - [ ] fn `suprnova::auth_flows::token_store::TokenStore::consume` · framework/src/auth_flows/token_store.rs:274
  - [ ] fn `suprnova::auth_flows::token_store::TokenStore::prune_expired` · framework/src/auth_flows/token_store.rs:359
- [ ] enum `suprnova::auth_flows::TokenPurpose` · framework/src/auth_flows/token_store.rs:27 (also `suprnova::auth_flows::token_store::TokenPurpose`)
  - Variants: `EmailVerification`, `PasswordReset`, `MagicLink`
  - [ ] fn `suprnova::auth_flows::TokenPurpose::as_str` · framework/src/auth_flows/token_store.rs:40
  - [ ] fn `suprnova::auth_flows::TokenPurpose::default_ttl` · framework/src/auth_flows/token_store.rs:51

### `suprnova::auth_flows::two_factor::entity`

- [ ] struct `suprnova::auth_flows::two_factor::entity::ActiveModel` · framework/src/auth_flows/two_factor/entity.rs:13
  - Public fields: `user_id`, `secret`, `confirmed_at`, `recovery_codes`, `last_used_timestep`, `created_at`, `updated_at`
- [ ] struct `suprnova::auth_flows::two_factor::entity::ColumnIter` · framework/src/auth_flows/two_factor/entity.rs:13
- [ ] struct `suprnova::auth_flows::two_factor::entity::Entity` · framework/src/auth_flows/two_factor/entity.rs:13
- [ ] struct `suprnova::auth_flows::two_factor::entity::Model` · framework/src/auth_flows/two_factor/entity.rs:15
  - Public fields: `user_id`, `secret`, `confirmed_at`, `recovery_codes`, `last_used_timestep`, `created_at`, `updated_at`
  - [ ] fn `suprnova::auth_flows::two_factor::entity::Model::into_ex` · framework/src/auth_flows/two_factor/entity.rs:13
- [ ] struct `suprnova::auth_flows::two_factor::entity::PrimaryKeyIter` · framework/src/auth_flows/two_factor/entity.rs:13
- [ ] struct `suprnova::auth_flows::two_factor::entity::RelationIter` · framework/src/auth_flows/two_factor/entity.rs:49
- [ ] enum `suprnova::auth_flows::two_factor::entity::Column` · framework/src/auth_flows/two_factor/entity.rs:13
  - Variants: `UserId`, `Secret`, `ConfirmedAt`, `RecoveryCodes`, `LastUsedTimestep`, `CreatedAt`, `UpdatedAt`
- [ ] enum `suprnova::auth_flows::two_factor::entity::PrimaryKey` · framework/src/auth_flows/two_factor/entity.rs:13
  - Variants: `UserId`
- [ ] enum `suprnova::auth_flows::two_factor::entity::Relation` · framework/src/auth_flows/two_factor/entity.rs:50

### `suprnova::auth_flows::two_factor::migration_replay`

- [ ] struct `suprnova::auth_flows::two_factor::migration_replay::Migration` · framework/src/auth_flows/two_factor/migration_replay.rs:15

### `suprnova::auth_flows::two_factor::migration`

- [ ] struct `suprnova::auth_flows::two_factor::migration::Migration` · framework/src/auth_flows/two_factor/migration.rs:11

### `suprnova::auth_flows::two_factor::recovery`

- [ ] fn `suprnova::auth_flows::two_factor::recovery::consume` · framework/src/auth_flows/two_factor/recovery.rs:71
- [ ] fn `suprnova::auth_flows::two_factor::recovery::generate` · framework/src/auth_flows/two_factor/recovery.rs:55

### `suprnova::auth_flows::two_factor_challenge_middleware`

- [ ] struct `suprnova::TwoFactorChallengeMiddleware` · framework/src/auth_flows/two_factor_challenge_middleware.rs:75 (also `suprnova::auth_flows::TwoFactorChallengeMiddleware`, `suprnova::auth_flows::two_factor_challenge_middleware::TwoFactorChallengeMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::TwoFactorChallengeMiddleware::new` · framework/src/auth_flows/two_factor_challenge_middleware.rs:87
  - [ ] fn `suprnova::TwoFactorChallengeMiddleware::redirect_to` · framework/src/auth_flows/two_factor_challenge_middleware.rs:96

### `suprnova::auth_flows::two_factor`

- [ ] struct `suprnova::EnrollmentResponse` · framework/src/auth_flows/two_factor/mod.rs:74 (also `suprnova::auth_flows::EnrollmentResponse`, `suprnova::auth_flows::two_factor::EnrollmentResponse`)
  - Public fields: `otpauth_url`, `qr_code_svg`, `recovery_codes`
- [ ] struct `suprnova::TwoFactor` · framework/src/auth_flows/two_factor/mod.rs:60 (also `suprnova::auth_flows::TwoFactor`, `suprnova::auth_flows::two_factor::TwoFactor`)
  - [ ] fn `suprnova::TwoFactor::enroll` · framework/src/auth_flows/two_factor/mod.rs:141
  - [ ] fn `suprnova::TwoFactor::re_enroll` · framework/src/auth_flows/two_factor/mod.rs:167
  - [ ] fn `suprnova::TwoFactor::confirm` · framework/src/auth_flows/two_factor/mod.rs:283
  - [ ] fn `suprnova::TwoFactor::verify` · framework/src/auth_flows/two_factor/mod.rs:365
  - [ ] fn `suprnova::TwoFactor::consume_recovery_code` · framework/src/auth_flows/two_factor/mod.rs:452
  - [ ] fn `suprnova::TwoFactor::is_enabled` · framework/src/auth_flows/two_factor/mod.rs:549
  - [ ] fn `suprnova::TwoFactor::is_enabled_by_id` · framework/src/auth_flows/two_factor/mod.rs:561
  - [ ] fn `suprnova::TwoFactor::start_challenge` · framework/src/auth_flows/two_factor/mod.rs:630
  - [ ] fn `suprnova::TwoFactor::pending_user_id` · framework/src/auth_flows/two_factor/mod.rs:679
  - [ ] fn `suprnova::TwoFactor::cancel_challenge` · framework/src/auth_flows/two_factor/mod.rs:693
  - [ ] fn `suprnova::TwoFactor::complete_challenge` · framework/src/auth_flows/two_factor/mod.rs:769
  - [ ] fn `suprnova::TwoFactor::regenerate_recovery_codes` · framework/src/auth_flows/two_factor/mod.rs:990
  - [ ] fn `suprnova::TwoFactor::disable` · framework/src/auth_flows/two_factor/mod.rs:1069
- [ ] trait `suprnova::TwoFactorUser` · framework/src/auth_flows/two_factor/mod.rs:112 (also `suprnova::auth_flows::TwoFactorUser`, `suprnova::auth_flows::two_factor::TwoFactorUser`)
  - [ ] fn `suprnova::TwoFactorUser::user_id` · framework/src/auth_flows/two_factor/mod.rs:114 (required)
  - [ ] fn `suprnova::TwoFactorUser::email` · framework/src/auth_flows/two_factor/mod.rs:117 (required)
