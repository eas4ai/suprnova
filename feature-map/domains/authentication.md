# Feature map: `manual/authentication.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 372 checked.

## Rust API: suprnova

### `suprnova::auth::authenticatable`

- [ ] trait `suprnova::Authenticatable` · framework/src/auth/authenticatable.rs:61 (also `suprnova::auth::Authenticatable`, `suprnova::auth::authenticatable::Authenticatable`)
  - Implemented here by: `GenericUser`
  - [ ] fn `suprnova::Authenticatable::get_auth_identifier` · framework/src/auth/authenticatable.rs:70 (required)
  - [ ] fn `suprnova::Authenticatable::auth_identifier_name` · framework/src/auth/authenticatable.rs:75 (provided)
  - [ ] fn `suprnova::Authenticatable::auth_identifier` · framework/src/auth/authenticatable.rs:87 (provided)
  - [ ] fn `suprnova::Authenticatable::get_auth_password` · framework/src/auth/authenticatable.rs:97 (provided)
  - [ ] fn `suprnova::Authenticatable::as_any` · framework/src/auth/authenticatable.rs:105 (required)
  - [ ] fn `suprnova::Authenticatable::into_arc_any` · framework/src/auth/authenticatable.rs:136 (required)

### `suprnova::auth::config`

- [ ] struct `suprnova::AuthConfig` · framework/src/auth/config.rs:85 (also `suprnova::auth::AuthConfig`, `suprnova::auth::config::AuthConfig`)
  - Public fields: `default_guard`, `guards`
  - [ ] fn `suprnova::AuthConfig::new` · framework/src/auth/config.rs:109
  - [ ] fn `suprnova::AuthConfig::from_env` · framework/src/auth/config.rs:122
  - [ ] fn `suprnova::AuthConfig::guard` · framework/src/auth/config.rs:128
  - [ ] fn `suprnova::AuthConfig::guard_config` · framework/src/auth/config.rs:134
- [ ] struct `suprnova::GuardConfig` · framework/src/auth/config.rs:55 (also `suprnova::auth::GuardConfig`, `suprnova::auth::config::GuardConfig`)
  - Public fields: `driver`, `provider`
  - [ ] fn `suprnova::GuardConfig::session` · framework/src/auth/config.rs:64
  - [ ] fn `suprnova::GuardConfig::token` · framework/src/auth/config.rs:72
- [ ] enum `suprnova::GuardDriver` · framework/src/auth/config.rs:34 (also `suprnova::auth::GuardDriver`, `suprnova::auth::config::GuardDriver`)
  - Variants: `Session`, `Token`
  - [ ] fn `suprnova::GuardDriver::from_str_lenient` · framework/src/auth/config.rs:44

### `suprnova::auth::contract`

- [ ] struct `suprnova::Credentials` · framework/src/auth/contract.rs:28 (also `suprnova::auth::Credentials`, `suprnova::auth::contract::Credentials`)
  - Public tuple fields: 1
  - [ ] fn `suprnova::Credentials::new` · framework/src/auth/contract.rs:32
  - [ ] fn `suprnova::Credentials::password` · framework/src/auth/contract.rs:37
  - [ ] fn `suprnova::Credentials::insert` · framework/src/auth/contract.rs:44
  - [ ] fn `suprnova::Credentials::get_str` · framework/src/auth/contract.rs:50
  - [ ] fn `suprnova::Credentials::as_value` · framework/src/auth/contract.rs:55
- [ ] trait `suprnova::Guard` · framework/src/auth/contract.rs:67 (also `suprnova::auth::Guard`, `suprnova::auth::contract::Guard`)
  - Implemented here by: `SessionGuard`, `TokenGuard`
  - [ ] fn `suprnova::Guard::user` · framework/src/auth/contract.rs:69 (required)
  - [ ] fn `suprnova::Guard::id` · framework/src/auth/contract.rs:72 (required)
  - [ ] fn `suprnova::Guard::validate` · framework/src/auth/contract.rs:75 (required)
  - [ ] fn `suprnova::Guard::set_user` · framework/src/auth/contract.rs:82 (required)
  - [ ] fn `suprnova::Guard::has_user` · framework/src/auth/contract.rs:88 (required)
  - [ ] fn `suprnova::Guard::check` · framework/src/auth/contract.rs:91 (provided)
  - [ ] fn `suprnova::Guard::guest` · framework/src/auth/contract.rs:96 (provided)
- [ ] trait `suprnova::StatefulGuard` · framework/src/auth/contract.rs:114 (also `suprnova::auth::StatefulGuard`, `suprnova::auth::contract::StatefulGuard`)
  - Implemented here by: `SessionGuard`
  - [ ] fn `suprnova::StatefulGuard::attempt` · framework/src/auth/contract.rs:118 (required)
  - [ ] fn `suprnova::StatefulGuard::once` · framework/src/auth/contract.rs:126 (required)
  - [ ] fn `suprnova::StatefulGuard::login` · framework/src/auth/contract.rs:130 (required)
  - [ ] fn `suprnova::StatefulGuard::login_using_id` · framework/src/auth/contract.rs:139 (required)
  - [ ] fn `suprnova::StatefulGuard::once_using_id` · framework/src/auth/contract.rs:148 (required)
  - [ ] fn `suprnova::StatefulGuard::via_remember` · framework/src/auth/contract.rs:155 (required)
  - [ ] fn `suprnova::StatefulGuard::logout` · framework/src/auth/contract.rs:158 (required)

### `suprnova::auth::database_provider`

- [ ] struct `suprnova::DatabaseUserProvider` · framework/src/auth/database_provider.rs:77 (also `suprnova::auth::DatabaseUserProvider`, `suprnova::auth::database_provider::DatabaseUserProvider`)
  - Implements: `suprnova::UserProvider`
  - [ ] fn `suprnova::DatabaseUserProvider::new` · framework/src/auth/database_provider.rs:88
  - [ ] fn `suprnova::DatabaseUserProvider::identifier_column` · framework/src/auth/database_provider.rs:99
  - [ ] fn `suprnova::DatabaseUserProvider::password_column` · framework/src/auth/database_provider.rs:105
  - [ ] fn `suprnova::DatabaseUserProvider::credential_columns` · framework/src/auth/database_provider.rs:115
  - [ ] fn `suprnova::DatabaseUserProvider::with_id_parser` · framework/src/auth/database_provider.rs:129

### `suprnova::auth::eloquent_provider`

- [ ] struct `suprnova::EloquentUserProvider` · framework/src/auth/eloquent_provider.rs:51 (also `suprnova::auth::EloquentUserProvider`, `suprnova::auth::eloquent_provider::EloquentUserProvider`)
  - Implements: `suprnova::UserProvider`
  - [ ] fn `suprnova::EloquentUserProvider::new` · framework/src/auth/eloquent_provider.rs:66
  - [ ] fn `suprnova::EloquentUserProvider::identifier_column` · framework/src/auth/eloquent_provider.rs:77
  - [ ] fn `suprnova::EloquentUserProvider::credential_columns` · framework/src/auth/eloquent_provider.rs:85
  - [ ] fn `suprnova::EloquentUserProvider::with_id_parser` · framework/src/auth/eloquent_provider.rs:95

### `suprnova::auth::events`

- [ ] struct `suprnova::auth::events::Attempting` · framework/src/auth/events.rs:38
  - Public fields: `guard`, `remember`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::auth::events::Authenticated` · framework/src/auth/events.rs:55
  - Public fields: `guard`, `user_id`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::auth::events::Failed` · framework/src/auth/events.rs:107
  - Public fields: `guard`, `user_id`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::auth::events::Login` · framework/src/auth/events.rs:71
  - Public fields: `guard`, `user_id`, `remember`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::auth::events::Logout` · framework/src/auth/events.rs:88
  - Public fields: `guard`, `user_id`
  - Implements: `suprnova::Event`

### `suprnova::auth::generic_user`

- [ ] struct `suprnova::GenericUser` · framework/src/auth/generic_user.rs:20 (also `suprnova::auth::GenericUser`, `suprnova::auth::generic_user::GenericUser`)
  - Implements: `suprnova::Authenticatable`
  - [ ] fn `suprnova::GenericUser::new` · framework/src/auth/generic_user.rs:29
  - [ ] fn `suprnova::GenericUser::attribute` · framework/src/auth/generic_user.rs:42
  - [ ] fn `suprnova::GenericUser::attributes` · framework/src/auth/generic_user.rs:47

### `suprnova::auth::guard`

- [ ] struct `suprnova::Auth` · framework/src/auth/guard.rs:52 (also `suprnova::auth::Auth`, `suprnova::auth::guard::Auth`, `suprnova::prelude::Auth`)
  - [ ] fn `suprnova::Auth::id` · framework/src/auth/guard.rs:58
  - [ ] fn `suprnova::Auth::check` · framework/src/auth/guard.rs:64
  - [ ] fn `suprnova::Auth::guest` · framework/src/auth/guard.rs:70
  - [ ] fn `suprnova::Auth::login_id` · framework/src/auth/guard.rs:97
  - [ ] fn `suprnova::Auth::login_remember` · framework/src/auth/guard.rs:168
  - [ ] fn `suprnova::Auth::issue_remember_cookie` · framework/src/auth/guard.rs:212
  - [ ] fn `suprnova::Auth::revoke_remember_tokens` · framework/src/auth/guard.rs:373
  - [ ] fn `suprnova::Auth::revoke_remember_tokens_for_user` · framework/src/auth/guard.rs:422
  - [ ] fn `suprnova::Auth::logout` · framework/src/auth/guard.rs:608
  - [ ] fn `suprnova::Auth::logout_and_invalidate` · framework/src/auth/guard.rs:627
  - [ ] fn `suprnova::Auth::user` · framework/src/auth/guard.rs:740
  - [ ] fn `suprnova::Auth::user_or_fail` · framework/src/auth/guard.rs:822
  - [ ] fn `suprnova::Auth::user_as` · framework/src/auth/guard.rs:871
  - [ ] fn `suprnova::Auth::user_as_arc` · framework/src/auth/guard.rs:890
  - [ ] fn `suprnova::Auth::default_guard_name` · framework/src/auth/guard.rs:921
  - [ ] fn `suprnova::Auth::register_provider` · framework/src/auth/guard.rs:942
  - [ ] fn `suprnova::Auth::guard` · framework/src/auth/guard.rs:958
  - [ ] fn `suprnova::Auth::stateful_guard` · framework/src/auth/guard.rs:977
  - [ ] fn `suprnova::Auth::attempt` · framework/src/auth/guard.rs:1012
  - [ ] fn `suprnova::Auth::once` · framework/src/auth/guard.rs:1023
  - [ ] fn `suprnova::Auth::login` · framework/src/auth/guard.rs:1032
  - [ ] fn `suprnova::Auth::login_using_id` · framework/src/auth/guard.rs:1045
  - [ ] fn `suprnova::Auth::once_using_id` · framework/src/auth/guard.rs:1058
  - [ ] fn `suprnova::Auth::validate` · framework/src/auth/guard.rs:1066
  - [ ] fn `suprnova::Auth::via_remember` · framework/src/auth/guard.rs:1077
  - [ ] fn `suprnova::Auth::set_user` · framework/src/auth/guard.rs:1089
  - [ ] fn `suprnova::Auth::has_user` · framework/src/auth/guard.rs:1099
  - [ ] fn `suprnova::Auth::factor` · framework/src/auth/guard.rs:1114
  - [ ] fn `suprnova::Auth::password` · framework/src/auth/guard.rs:1137
  - [ ] fn `suprnova::Auth::oauth` · framework/src/auth/guard.rs:1161
  - [ ] fn `suprnova::Auth::passkey` · framework/src/auth/guard.rs:1175
  - [ ] fn `suprnova::Auth::magic_link` · framework/src/auth/guard.rs:1189

### `suprnova::auth::manager`

- [ ] struct `suprnova::AuthManager` · framework/src/auth/manager.rs:31 (also `suprnova::auth::AuthManager`, `suprnova::auth::manager::AuthManager`)
  - [ ] fn `suprnova::AuthManager::new` · framework/src/auth/manager.rs:39
  - [ ] fn `suprnova::AuthManager::config` · framework/src/auth/manager.rs:47
  - [ ] fn `suprnova::AuthManager::default_guard_name` · framework/src/auth/manager.rs:52
  - [ ] fn `suprnova::AuthManager::register_provider` · framework/src/auth/manager.rs:61
  - [ ] fn `suprnova::AuthManager::guard` · framework/src/auth/manager.rs:83
  - [ ] fn `suprnova::AuthManager::stateful_guard` · framework/src/auth/manager.rs:97
  - [ ] fn `suprnova::AuthManager::default_guard` · framework/src/auth/manager.rs:112
  - [ ] fn `suprnova::AuthManager::default_provider` · framework/src/auth/manager.rs:125
  - [ ] fn `suprnova::AuthManager::default_stateful_guard` · framework/src/auth/manager.rs:131

### `suprnova::auth::middleware`

- [ ] struct `suprnova::AuthMiddleware` · framework/src/auth/middleware.rs:31 (also `suprnova::auth::AuthMiddleware`, `suprnova::auth::middleware::AuthMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::AuthMiddleware::new` · framework/src/auth/middleware.rs:45
  - [ ] fn `suprnova::AuthMiddleware::optional` · framework/src/auth/middleware.rs:61
  - [ ] fn `suprnova::AuthMiddleware::redirect_to` · framework/src/auth/middleware.rs:79
  - [ ] fn `suprnova::AuthMiddleware::for_guard` · framework/src/auth/middleware.rs:99
- [ ] struct `suprnova::BasicAuthMiddleware` · framework/src/auth/middleware.rs:304 (also `suprnova::auth::BasicAuthMiddleware`, `suprnova::auth::middleware::BasicAuthMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::BasicAuthMiddleware::new` · framework/src/auth/middleware.rs:333
  - [ ] fn `suprnova::BasicAuthMiddleware::once` · framework/src/auth/middleware.rs:339
  - [ ] fn `suprnova::BasicAuthMiddleware::field` · framework/src/auth/middleware.rs:345
  - [ ] fn `suprnova::BasicAuthMiddleware::realm` · framework/src/auth/middleware.rs:352
  - [ ] fn `suprnova::BasicAuthMiddleware::for_guard` · framework/src/auth/middleware.rs:358
- [ ] struct `suprnova::GuestMiddleware` · framework/src/auth/middleware.rs:216 (also `suprnova::auth::GuestMiddleware`, `suprnova::auth::middleware::GuestMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::GuestMiddleware::redirect_to` · framework/src/auth/middleware.rs:230
  - [ ] fn `suprnova::GuestMiddleware::new` · framework/src/auth/middleware.rs:238
  - [ ] fn `suprnova::GuestMiddleware::for_guard` · framework/src/auth/middleware.rs:244

### `suprnova::auth::must_verify_email`

- [ ] struct `suprnova::AuthFlowUser` · framework/src/auth/must_verify_email.rs:53 (also `suprnova::auth::AuthFlowUser`, `suprnova::auth::must_verify_email::AuthFlowUser`)
  - Public fields: `id`, `email`, `name`
- [ ] trait `suprnova::CanResetPassword` · framework/src/auth/must_verify_email.rs:35 (also `suprnova::auth::CanResetPassword`, `suprnova::auth::must_verify_email::CanResetPassword`)
  - [ ] fn `suprnova::CanResetPassword::email_for_reset` · framework/src/auth/must_verify_email.rs:39 (required)
  - [ ] fn `suprnova::CanResetPassword::set_password_hash` · framework/src/auth/must_verify_email.rs:46 (required)
- [ ] trait `suprnova::MustVerifyEmail` · framework/src/auth/must_verify_email.rs:13 (also `suprnova::auth::MustVerifyEmail`, `suprnova::auth::must_verify_email::MustVerifyEmail`)
  - [ ] fn `suprnova::MustVerifyEmail::email` · framework/src/auth/must_verify_email.rs:15 (required)
  - [ ] fn `suprnova::MustVerifyEmail::email_verified_at` · framework/src/auth/must_verify_email.rs:17 (required)
  - [ ] fn `suprnova::MustVerifyEmail::set_email_verified_at` · framework/src/auth/must_verify_email.rs:20 (required)
  - [ ] fn `suprnova::MustVerifyEmail::is_email_verified` · framework/src/auth/must_verify_email.rs:22 (provided)
  - [ ] fn `suprnova::MustVerifyEmail::name` · framework/src/auth/must_verify_email.rs:26 (provided)

### `suprnova::auth::provider`

- [ ] trait `suprnova::UserProvider` · framework/src/auth/provider.rs:71 (also `suprnova::auth::UserProvider`, `suprnova::auth::provider::UserProvider`)
  - Implemented here by: `DatabaseUserProvider`, `EloquentUserProvider`
  - [ ] fn `suprnova::UserProvider::retrieve_by_id` · framework/src/auth/provider.rs:77 (required)
  - [ ] fn `suprnova::UserProvider::retrieve_by_credentials` · framework/src/auth/provider.rs:86 (provided)
  - [ ] fn `suprnova::UserProvider::validate_credentials` · framework/src/auth/provider.rs:97 (provided)
  - [ ] fn `suprnova::UserProvider::dummy_verify` · framework/src/auth/provider.rs:125 (provided)
  - [ ] fn `suprnova::UserProvider::retrieve_by_email` · framework/src/auth/provider.rs:139 (provided)
  - [ ] fn `suprnova::UserProvider::supports_password_reset` · framework/src/auth/provider.rs:151 (provided)
  - [ ] fn `suprnova::UserProvider::retrieve_verified_user_for_password_reset` · framework/src/auth/provider.rs:161 (provided)
  - [ ] fn `suprnova::UserProvider::flow_user_by_id` · framework/src/auth/provider.rs:171 (provided)
  - [ ] fn `suprnova::UserProvider::mark_email_verified` · framework/src/auth/provider.rs:179 (provided)
  - [ ] fn `suprnova::UserProvider::set_password` · framework/src/auth/provider.rs:186 (provided)
  - [ ] fn `suprnova::UserProvider::is_email_verified` · framework/src/auth/provider.rs:193 (provided)

### `suprnova::auth::remember::entity`

- [ ] struct `suprnova::auth::remember::entity::ActiveModel` · framework/src/auth/remember.rs:393 (also `suprnova::auth_flows::remember_me::entity::ActiveModel`)
  - Public fields: `id`, `user_id`, `selector`, `token_hash`, `expires_at`, `created_at`, `last_used_at`
- [ ] struct `suprnova::auth::remember::entity::ColumnIter` · framework/src/auth/remember.rs:393 (also `suprnova::auth_flows::remember_me::entity::ColumnIter`)
- [ ] struct `suprnova::auth::remember::entity::Entity` · framework/src/auth/remember.rs:393 (also `suprnova::auth_flows::remember_me::entity::Entity`)
- [ ] struct `suprnova::auth::remember::entity::Model` · framework/src/auth/remember.rs:395 (also `suprnova::auth_flows::remember_me::entity::Model`)
  - Public fields: `id`, `user_id`, `selector`, `token_hash`, `expires_at`, `created_at`, `last_used_at`
  - [ ] fn `suprnova::auth::remember::entity::Model::into_ex` · framework/src/auth/remember.rs:393
- [ ] struct `suprnova::auth::remember::entity::PrimaryKeyIter` · framework/src/auth/remember.rs:393 (also `suprnova::auth_flows::remember_me::entity::PrimaryKeyIter`)
- [ ] struct `suprnova::auth::remember::entity::RelationIter` · framework/src/auth/remember.rs:416 (also `suprnova::auth_flows::remember_me::entity::RelationIter`)
- [ ] enum `suprnova::auth::remember::entity::Column` · framework/src/auth/remember.rs:393 (also `suprnova::auth_flows::remember_me::entity::Column`)
  - Variants: `Id`, `UserId`, `Selector`, `TokenHash`, `ExpiresAt`, `CreatedAt`, `LastUsedAt`
- [ ] enum `suprnova::auth::remember::entity::PrimaryKey` · framework/src/auth/remember.rs:393 (also `suprnova::auth_flows::remember_me::entity::PrimaryKey`)
  - Variants: `Id`
- [ ] enum `suprnova::auth::remember::entity::Relation` · framework/src/auth/remember.rs:417 (also `suprnova::auth_flows::remember_me::entity::Relation`)

### `suprnova::auth::remember`

- [ ] fn `suprnova::auth::remember::issue` · framework/src/auth/remember.rs:155 (also `suprnova::auth_flows::remember_me::issue`)
- [ ] fn `suprnova::auth::remember::prune_expired` · framework/src/auth/remember.rs:365 (also `suprnova::auth_flows::remember_me::prune_expired`)
- [ ] fn `suprnova::auth::remember::revoke_all_for_user` · framework/src/auth/remember.rs:268 (also `suprnova::auth_flows::remember_me::revoke_all_for_user`)
- [ ] fn `suprnova::auth::remember::revoke_by_id` · framework/src/auth/remember.rs:351 (also `suprnova::auth_flows::remember_me::revoke_by_id`)
- [ ] fn `suprnova::auth::remember::verify_and_rotate` · framework/src/auth/remember.rs:198 (also `suprnova::auth_flows::remember_me::verify_and_rotate`)

### `suprnova::auth::session_guard`

- [ ] struct `suprnova::SessionGuard` · framework/src/auth/session_guard.rs:45 (also `suprnova::auth::SessionGuard`, `suprnova::auth::session_guard::SessionGuard`)
  - Implements: `suprnova::Guard`, `suprnova::StatefulGuard`
  - [ ] fn `suprnova::SessionGuard::new` · framework/src/auth/session_guard.rs:58
  - [ ] fn `suprnova::SessionGuard::named` · framework/src/auth/session_guard.rs:67
  - [ ] fn `suprnova::SessionGuard::with_remember_ttl` · framework/src/auth/session_guard.rs:83

### `suprnova::auth::token_guard`

- [ ] struct `suprnova::TokenGuard` · framework/src/auth/token_guard.rs:38 (also `suprnova::auth::TokenGuard`, `suprnova::auth::token_guard::TokenGuard`)
  - Implements: `suprnova::Guard`
  - [ ] fn `suprnova::TokenGuard::new` · framework/src/auth/token_guard.rs:47

### `suprnova::auth::types::lockout` (private module; items are public through re-exports)

- [ ] struct `suprnova::LockoutStatus` · framework/src/auth/types/lockout.rs:12 (also `suprnova::auth::LockoutStatus`, `suprnova::magnetar_integration::LockoutStatus`)
  - Public fields: `email`, `failed_attempts`, `is_locked`, `locked_until`
  - [ ] fn `suprnova::LockoutStatus::retry_after_seconds` · framework/src/auth/types/lockout.rs:29

### `suprnova::auth::types::session` (private module; items are public through re-exports)

- [ ] struct `suprnova::Session` · framework/src/auth/types/session.rs:12 (also `suprnova::auth::Session`, `suprnova::magnetar_integration::Session`)
  - Public fields: `token`, `token_hash`, `user_id`, `user_agent`, `ip_address`, `created_at`, `updated_at`, `expires_at`
  - [ ] fn `suprnova::Session::builder` · framework/src/auth/types/session.rs:39
  - [ ] fn `suprnova::Session::is_expired` · framework/src/auth/types/session.rs:45
- [ ] struct `suprnova::SessionBuilder` · framework/src/auth/types/session.rs:52 (also `suprnova::auth::SessionBuilder`)
  - [ ] fn `suprnova::SessionBuilder::token` · framework/src/auth/types/session.rs:68
  - [ ] fn `suprnova::SessionBuilder::token_hash` · framework/src/auth/types/session.rs:75
  - [ ] fn `suprnova::SessionBuilder::user_id` · framework/src/auth/types/session.rs:82
  - [ ] fn `suprnova::SessionBuilder::user_agent` · framework/src/auth/types/session.rs:89
  - [ ] fn `suprnova::SessionBuilder::ip_address` · framework/src/auth/types/session.rs:96
  - [ ] fn `suprnova::SessionBuilder::created_at` · framework/src/auth/types/session.rs:103
  - [ ] fn `suprnova::SessionBuilder::updated_at` · framework/src/auth/types/session.rs:110
  - [ ] fn `suprnova::SessionBuilder::expires_at` · framework/src/auth/types/session.rs:117
  - [ ] fn `suprnova::SessionBuilder::build` · framework/src/auth/types/session.rs:127

### `suprnova::auth::types::token` (private module; items are public through re-exports)

- [ ] struct `suprnova::SessionToken` · framework/src/auth/types/token.rs:18 (also `suprnova::auth::SessionToken`, `suprnova::magnetar_integration::SessionToken`)
  - [ ] fn `suprnova::SessionToken::new` · framework/src/auth/types/token.rs:23
  - [ ] fn `suprnova::SessionToken::new_random` · framework/src/auth/types/token.rs:29
  - [ ] fn `suprnova::SessionToken::expose_secret` · framework/src/auth/types/token.rs:37
  - [ ] fn `suprnova::SessionToken::into_secret` · framework/src/auth/types/token.rs:43
  - [ ] fn `suprnova::SessionToken::token_hash` · framework/src/auth/types/token.rs:49
  - [ ] fn `suprnova::SessionToken::verify_hash` · framework/src/auth/types/token.rs:57

### `suprnova::auth::types::user` (private module; items are public through re-exports)

- [ ] struct `suprnova::User` · framework/src/auth/types/user.rs:108 (also `suprnova::auth::User`, `suprnova::magnetar_integration::User`)
  - Public fields: `id`, `name`, `email`, `email_verified_at`, `locked_at`, `created_at`, `updated_at`
  - [ ] fn `suprnova::User::builder` · framework/src/auth/types/user.rs:128
  - [ ] fn `suprnova::User::is_email_verified` · framework/src/auth/types/user.rs:134
  - [ ] fn `suprnova::User::is_locked` · framework/src/auth/types/user.rs:143
- [ ] struct `suprnova::UserBuilder` · framework/src/auth/types/user.rs:150 (also `suprnova::auth::UserBuilder`)
  - [ ] fn `suprnova::UserBuilder::id` · framework/src/auth/types/user.rs:163
  - [ ] fn `suprnova::UserBuilder::name` · framework/src/auth/types/user.rs:170
  - [ ] fn `suprnova::UserBuilder::email` · framework/src/auth/types/user.rs:177
  - [ ] fn `suprnova::UserBuilder::email_verified_at` · framework/src/auth/types/user.rs:184
  - [ ] fn `suprnova::UserBuilder::locked_at` · framework/src/auth/types/user.rs:191
  - [ ] fn `suprnova::UserBuilder::created_at` · framework/src/auth/types/user.rs:198
  - [ ] fn `suprnova::UserBuilder::updated_at` · framework/src/auth/types/user.rs:205
  - [ ] fn `suprnova::UserBuilder::build` · framework/src/auth/types/user.rs:215
- [ ] struct `suprnova::UserId` · framework/src/auth/types/user.rs:21 (also `suprnova::auth::UserId`, `suprnova::magnetar_integration::UserId`)
  - [ ] fn `suprnova::UserId::new` · framework/src/auth/types/user.rs:28
  - [ ] fn `suprnova::UserId::new_random` · framework/src/auth/types/user.rs:34
  - [ ] fn `suprnova::UserId::into_inner` · framework/src/auth/types/user.rs:43
  - [ ] fn `suprnova::UserId::as_str` · framework/src/auth/types/user.rs:49
  - [ ] fn `suprnova::UserId::is_valid` · framework/src/auth/types/user.rs:55

### `suprnova::magnetar_integration::abuse_limiter`

- [ ] struct `suprnova::FrameworkAbuseLimiter` · framework/src/magnetar_integration/abuse_limiter.rs:128 (feature: `magnetar-oauth`; also `suprnova::magnetar_integration::abuse_limiter::FrameworkAbuseLimiter`)
  - [ ] fn `suprnova::FrameworkAbuseLimiter::new` · framework/src/magnetar_integration/abuse_limiter.rs:134

### `suprnova::magnetar_integration::ceremony::entity`

- [ ] struct `suprnova::magnetar_integration::ceremony::entity::ActiveModel` · framework/src/magnetar_integration/ceremony.rs:146
  - Public fields: `id`, `selector`, `kind`, `payload`, `expires_at`, `created_at`
- [ ] struct `suprnova::magnetar_integration::ceremony::entity::ColumnIter` · framework/src/magnetar_integration/ceremony.rs:146
- [ ] struct `suprnova::magnetar_integration::ceremony::entity::Entity` · framework/src/magnetar_integration/ceremony.rs:146
- [ ] struct `suprnova::magnetar_integration::ceremony::entity::Model` · framework/src/magnetar_integration/ceremony.rs:148
  - Public fields: `id`, `selector`, `kind`, `payload`, `expires_at`, `created_at`
  - [ ] fn `suprnova::magnetar_integration::ceremony::entity::Model::into_ex` · framework/src/magnetar_integration/ceremony.rs:146
- [ ] struct `suprnova::magnetar_integration::ceremony::entity::PrimaryKeyIter` · framework/src/magnetar_integration/ceremony.rs:146
- [ ] struct `suprnova::magnetar_integration::ceremony::entity::RelationIter` · framework/src/magnetar_integration/ceremony.rs:168
- [ ] enum `suprnova::magnetar_integration::ceremony::entity::Column` · framework/src/magnetar_integration/ceremony.rs:146
  - Variants: `Id`, `Selector`, `Kind`, `Payload`, `ExpiresAt`, `CreatedAt`
- [ ] enum `suprnova::magnetar_integration::ceremony::entity::PrimaryKey` · framework/src/magnetar_integration/ceremony.rs:146
  - Variants: `Id`
- [ ] enum `suprnova::magnetar_integration::ceremony::entity::Relation` · framework/src/magnetar_integration/ceremony.rs:169

### `suprnova::magnetar_integration::ceremony::kind`

- [ ] const `suprnova::magnetar_integration::ceremony::kind::OAUTH` · framework/src/magnetar_integration/ceremony.rs:134
- [ ] const `suprnova::magnetar_integration::ceremony::kind::PASSKEY_AUTHENTICATE` · framework/src/magnetar_integration/ceremony.rs:138
- [ ] const `suprnova::magnetar_integration::ceremony::kind::PASSKEY_REGISTER` · framework/src/magnetar_integration/ceremony.rs:136

### `suprnova::magnetar_integration::ceremony`

- [ ] fn `suprnova::magnetar_integration::ceremony::consume` · framework/src/magnetar_integration/ceremony.rs:72
- [ ] fn `suprnova::magnetar_integration::ceremony::issue` · framework/src/magnetar_integration/ceremony.rs:38
- [ ] fn `suprnova::magnetar_integration::ceremony::prune_expired` · framework/src/magnetar_integration/ceremony.rs:117

### `suprnova::magnetar_integration::default_engine`

- [ ] fn `suprnova::init_magnetar` · framework/src/magnetar_integration/default_engine.rs:477 (feature: `database-sqlite` or `database-postgres` or `database-mysql`; also `suprnova::magnetar_integration::default_engine::init_magnetar`, `suprnova::magnetar_integration::init_magnetar`)
- [ ] fn `suprnova::init_magnetar_oauth_only` · framework/src/magnetar_integration/default_engine.rs:496 (feature: `magnetar-oauth`; also `suprnova::magnetar_integration::default_engine::init_magnetar_oauth_only`, `suprnova::magnetar_integration::init_magnetar_oauth_only`)
- [ ] struct `suprnova::MagnetarConfig` · framework/src/magnetar_integration/default_engine.rs:37 (feature: `database-sqlite` or `database-postgres` or `database-mysql`; also `suprnova::magnetar_integration::MagnetarConfig`, `suprnova::magnetar_integration::default_engine::MagnetarConfig`)
  - [ ] fn `suprnova::MagnetarConfig::from_sea_orm` · framework/src/magnetar_integration/default_engine.rs:51
  - [ ] fn `suprnova::MagnetarConfig::apply_migrations` · framework/src/magnetar_integration/default_engine.rs:66
  - [ ] fn `suprnova::MagnetarConfig::passkey_config` · framework/src/magnetar_integration/default_engine.rs:73
  - [ ] fn `suprnova::MagnetarConfig::oauth` · framework/src/magnetar_integration/default_engine.rs:81
  - [ ] fn `suprnova::MagnetarConfig::session_config` · framework/src/magnetar_integration/default_engine.rs:88
  - [ ] fn `suprnova::MagnetarConfig::lockout_config` · framework/src/magnetar_integration/default_engine.rs:95
  - [ ] fn `suprnova::MagnetarConfig::two_factor_config` · framework/src/magnetar_integration/default_engine.rs:102
- [ ] struct `suprnova::MagnetarOAuthOnlyConfig` · framework/src/magnetar_integration/default_engine.rs:111 (feature: `magnetar-oauth`; also `suprnova::magnetar_integration::MagnetarOAuthOnlyConfig`, `suprnova::magnetar_integration::default_engine::MagnetarOAuthOnlyConfig`)
  - [ ] fn `suprnova::MagnetarOAuthOnlyConfig::from_sea_orm` · framework/src/magnetar_integration/default_engine.rs:121
  - [ ] fn `suprnova::MagnetarOAuthOnlyConfig::apply_migrations` · framework/src/magnetar_integration/default_engine.rs:131

### `suprnova::magnetar_integration::engine`

- [ ] struct `suprnova::magnetar_integration::engine::HostPasswordResetIssued` · framework/src/magnetar_integration/engine.rs:362
  - Public fields: `user_id`, `email`, `token`
- [ ] struct `suprnova::magnetar_integration::engine::LockoutAdmission` · framework/src/magnetar_integration/engine.rs:212
  - Public fields: `admitted`, `status`, `locked_event`
  - [ ] fn `suprnova::magnetar_integration::engine::LockoutAdmission::new` · framework/src/magnetar_integration/engine.rs:225
  - [ ] fn `suprnova::magnetar_integration::engine::LockoutAdmission::with_event` · framework/src/magnetar_integration/engine.rs:240
  - [ ] fn `suprnova::magnetar_integration::engine::LockoutAdmission::reservation` · framework/src/magnetar_integration/engine.rs:256
- [ ] struct `suprnova::magnetar_integration::engine::LockoutFinalization` · framework/src/magnetar_integration/engine.rs:263
  - Public fields: `status`, `locked_event`
- [ ] struct `suprnova::magnetar_integration::engine::MagnetarBinding` · framework/src/magnetar_integration/engine.rs:89
  - Implements: `suprnova::magnetar_integration::engine::MagnetarAuthStore`
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarBinding::new` · framework/src/magnetar_integration/engine.rs:98
- [ ] struct `suprnova::magnetar_integration::engine::MagnetarHostEngine` · framework/src/magnetar_integration/engine.rs:690
  - Implements: `suprnova::MagnetarFactorAuthEngine`, `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine`
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::new` · framework/src/magnetar_integration/engine.rs:731
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::binding` · framework/src/magnetar_integration/engine.rs:798
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::session_store` · framework/src/magnetar_integration/engine.rs:804
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::session_provider` · framework/src/magnetar_integration/engine.rs:810
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::factor_gate` · framework/src/magnetar_integration/engine.rs:816
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::users` · framework/src/magnetar_integration/engine.rs:822
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::passkey_service` · framework/src/magnetar_integration/engine.rs:834
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::issue_remember` · framework/src/magnetar_integration/engine.rs:856
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::remember_sign_in` · framework/src/magnetar_integration/engine.rs:867
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::remember_sign_in_attempt` · framework/src/magnetar_integration/engine.rs:889
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::resolve_web_binding` · framework/src/magnetar_integration/engine.rs:957
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::revoke_remember` · framework/src/magnetar_integration/engine.rs:967
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::revoke_remember_selector` · framework/src/magnetar_integration/engine.rs:980
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::register_password` · framework/src/magnetar_integration/engine.rs:985
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::authenticate_password` · framework/src/magnetar_integration/engine.rs:991
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::complete_sign_in` · framework/src/magnetar_integration/engine.rs:1026
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::password_sign_in` · framework/src/magnetar_integration/engine.rs:1049
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::password_register` · framework/src/magnetar_integration/engine.rs:1063
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::magic_link_send` · framework/src/magnetar_integration/engine.rs:1076
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::magic_link_consume` · framework/src/magnetar_integration/engine.rs:1089
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::complete_challenge` · framework/src/magnetar_integration/engine.rs:1105
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::forward_lifecycle` · framework/src/magnetar_integration/engine.rs:1117
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostEngine::oauth_service` · framework/src/magnetar_integration/engine.rs:1840
- [ ] struct `suprnova::magnetar_integration::engine::MagnetarHostEngineParts` · framework/src/magnetar_integration/engine.rs:642
  - Public fields: `binding`, `session_store`, `remember_store`, `ceremonies`, `factors`, `password`, `first_email_proof`, `password_verifier`, `password_lockout`, `encryptor`, `session_config`, `users`, `lifecycle_deliveries`, `lifecycle_lease_duration`
- [ ] struct `suprnova::magnetar_integration::engine::MagnetarHostOAuthEngine` · framework/src/magnetar_integration/engine.rs:1804 (feature: `magnetar-oauth`)
  - Implements: `suprnova::magnetar_integration::engine::MagnetarOAuthAuthEngine`
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostOAuthEngine::supports_provider` · framework/src/magnetar_integration/engine.rs:1887
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostOAuthEngine::begin` · framework/src/magnetar_integration/engine.rs:1892
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostOAuthEngine::verify_identity` · framework/src/magnetar_integration/engine.rs:1938
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarHostOAuthEngine::complete` · framework/src/magnetar_integration/engine.rs:1949
- [ ] struct `suprnova::magnetar_integration::engine::MagnetarHostPasskeyService` · framework/src/magnetar_integration/engine.rs:378
  - Implements: `suprnova::magnetar_integration::engine::MagnetarPasskeyAuthEngine`
- [ ] struct `suprnova::magnetar_integration::engine::MagnetarIssuedSession` · framework/src/magnetar_integration/engine.rs:276
  - Public fields: `session_id`, `web_binding`, `session`
- [ ] struct `suprnova::magnetar_integration::engine::MagnetarLifecycleEvent` · framework/src/magnetar_integration/engine.rs:492
  - Public fields: `mutation_id`, `kind`, `user_id`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::magnetar_integration::engine::MagnetarLifecycleForwarder` · framework/src/magnetar_integration/engine.rs:559
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarLifecycleForwarder::new` · framework/src/magnetar_integration/engine.rs:566
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarLifecycleForwarder::forward` · framework/src/magnetar_integration/engine.rs:580
- [ ] struct `suprnova::magnetar_integration::engine::MagnetarOAuthBegin` · framework/src/magnetar_integration/engine.rs:1679 (feature: `magnetar-oauth`)
  - Public fields: `provider`, `intent`, `actor`, `binding`, `limiter_identity`
- [ ] struct `suprnova::magnetar_integration::engine::MagnetarOAuthCallback` · framework/src/magnetar_integration/engine.rs:1694 (feature: `magnetar-oauth`)
  - Public fields: `provider`, `state`, `code`, `host_session_digest`, `form_post_user`, `metadata`
- [ ] struct `suprnova::MagnetarOAuthHostConfig` · framework/src/magnetar_integration/engine.rs:1609 (feature: `magnetar-oauth`; also `suprnova::magnetar_integration::engine::MagnetarOAuthHostConfig`)
  - [ ] fn `suprnova::MagnetarOAuthHostConfig::new` · framework/src/magnetar_integration/engine.rs:1630
- [ ] struct `suprnova::magnetar_integration::engine::MagnetarOAuthKickoff` · framework/src/magnetar_integration/engine.rs:1711 (feature: `magnetar-oauth`)
  - Public fields: `authorization_url`, `state`
- [ ] struct `suprnova::MagnetarOAuthProviderConfig` · framework/src/magnetar_integration/engine.rs:1595 (feature: `magnetar-oauth`; also `suprnova::magnetar_integration::engine::MagnetarOAuthProviderConfig`)
  - Public fields: `provider`, `redirect_uri`, `scopes`
- [ ] struct `suprnova::magnetar_integration::engine::MagnetarRememberSignIn` · framework/src/magnetar_integration/engine.rs:287
  - Public fields: `session`, `replacement`
- [ ] enum `suprnova::magnetar_integration::engine::HostOAuthError` · framework/src/magnetar_integration/engine.rs:1766 (feature: `magnetar-oauth`)
  - Variants: `Protocol`, `Auth`
- [ ] enum `suprnova::magnetar_integration::engine::HostSignInDecision` · framework/src/magnetar_integration/engine.rs:350
  - Variants: `SessionAllowed`, `FactorRequired`
- [ ] enum `suprnova::magnetar_integration::engine::LifecycleDeliveryClaim` · framework/src/magnetar_integration/engine.rs:519
  - Variants: `Deliver`, `AlreadyDelivered`, `InFlight`
- [ ] enum `suprnova::magnetar_integration::engine::LifecycleForwardResult` · framework/src/magnetar_integration/engine.rs:632
  - Variants: `Delivered`, `AlreadyDelivered`, `InFlight`
- [ ] enum `suprnova::magnetar_integration::engine::MagnetarOAuthCompletion` · framework/src/magnetar_integration/engine.rs:1720 (feature: `magnetar-oauth`)
  - Variants: `SessionAllowed`, `FactorRequired`, `AccountCreated`, `AccountLinked`, `ExplicitLinkRequired`, `EmailCompletionRequired`
- [ ] enum `suprnova::magnetar_integration::engine::MagnetarRememberSignInAttempt` · framework/src/magnetar_integration/engine.rs:297
  - Variants: `Authenticated`, `RotationCommitted`, `RotationOutcomeUnknown`
- [ ] trait `suprnova::magnetar_integration::engine::HostLifecycleDeduplication` · framework/src/magnetar_integration/engine.rs:535
  - [ ] fn `suprnova::magnetar_integration::engine::HostLifecycleDeduplication::claim` · framework/src/magnetar_integration/engine.rs:537 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::HostLifecycleDeduplication::mark_delivered` · framework/src/magnetar_integration/engine.rs:546 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::HostLifecycleDeduplication::release` · framework/src/magnetar_integration/engine.rs:549 (required)
- [ ] trait `suprnova::magnetar_integration::engine::HostPasswordLockout` · framework/src/magnetar_integration/engine.rs:142
  - [ ] fn `suprnova::magnetar_integration::engine::HostPasswordLockout::status` · framework/src/magnetar_integration/engine.rs:144 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::HostPasswordLockout::record_failure` · framework/src/magnetar_integration/engine.rs:147 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::HostPasswordLockout::admit_attempt` · framework/src/magnetar_integration/engine.rs:154 (provided)
  - [ ] fn `suprnova::magnetar_integration::engine::HostPasswordLockout::cancel_attempt` · framework/src/magnetar_integration/engine.rs:167 (provided)
  - [ ] fn `suprnova::magnetar_integration::engine::HostPasswordLockout::finalize_failed_attempt` · framework/src/magnetar_integration/engine.rs:176 (provided)
  - [ ] fn `suprnova::magnetar_integration::engine::HostPasswordLockout::reset_admitted_attempts` · framework/src/magnetar_integration/engine.rs:191 (provided)
  - [ ] fn `suprnova::magnetar_integration::engine::HostPasswordLockout::reset_after_success` · framework/src/magnetar_integration/engine.rs:204 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::HostPasswordLockout::unlock` · framework/src/magnetar_integration/engine.rs:207 (required)
- [ ] trait `suprnova::magnetar_integration::engine::HostUserAdapter` · framework/src/magnetar_integration/engine.rs:126
  - [ ] type `suprnova::magnetar_integration::engine::HostUserAdapter::User` · framework/src/magnetar_integration/engine.rs:128
  - [ ] fn `suprnova::magnetar_integration::engine::HostUserAdapter::user_for_id` · framework/src/magnetar_integration/engine.rs:132 (required)
- [ ] trait `suprnova::magnetar_integration::engine::MagnetarAuthStore` · framework/src/magnetar_integration/engine.rs:73
  - Implemented here by: `magnetar_integration::engine::MagnetarBinding`
  - [ ] type `suprnova::magnetar_integration::engine::MagnetarAuthStore::Schema` · framework/src/magnetar_integration/engine.rs:75
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarAuthStore::database` · framework/src/magnetar_integration/engine.rs:78 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarAuthStore::storage` · framework/src/magnetar_integration/engine.rs:81 (required)
- [ ] trait `suprnova::MagnetarFactorAuthEngine` · framework/src/magnetar_integration/engine.rs:1288 (feature: `database-sqlite` or `database-postgres` or `database-mysql`; also `suprnova::magnetar_integration::MagnetarFactorAuthEngine`, `suprnova::magnetar_integration::engine::MagnetarFactorAuthEngine`)
  - Implemented here by: `magnetar_integration::engine::MagnetarHostEngine`
  - [ ] fn `suprnova::MagnetarFactorAuthEngine::complete_challenge` · framework/src/magnetar_integration/engine.rs:1290 (required)
  - [ ] fn `suprnova::MagnetarFactorAuthEngine::user_by_id` · framework/src/magnetar_integration/engine.rs:1294 (required)
  - [ ] fn `suprnova::MagnetarFactorAuthEngine::resolve_web_binding` · framework/src/magnetar_integration/engine.rs:1297 (required)
  - [ ] fn `suprnova::MagnetarFactorAuthEngine::bearer_user_id` · framework/src/magnetar_integration/engine.rs:1300 (required)
  - [ ] fn `suprnova::MagnetarFactorAuthEngine::revoke_session` · framework/src/magnetar_integration/engine.rs:1303 (required)
  - [ ] fn `suprnova::MagnetarFactorAuthEngine::revoke_all_sessions` · framework/src/magnetar_integration/engine.rs:1306 (required)
  - [ ] fn `suprnova::MagnetarFactorAuthEngine::list_sessions` · framework/src/magnetar_integration/engine.rs:1309 (required)
- [ ] trait `suprnova::magnetar_integration::engine::MagnetarOAuthAuthEngine` · framework/src/magnetar_integration/engine.rs:1782
  - Implemented here by: `magnetar_integration::engine::MagnetarHostOAuthEngine`
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarOAuthAuthEngine::oauth_supports_provider` · framework/src/magnetar_integration/engine.rs:1784 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarOAuthAuthEngine::oauth_begin` · framework/src/magnetar_integration/engine.rs:1786 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarOAuthAuthEngine::oauth_complete` · framework/src/magnetar_integration/engine.rs:1791 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarOAuthAuthEngine::oauth_verify_identity` · framework/src/magnetar_integration/engine.rs:1796 (required)
- [ ] trait `suprnova::magnetar_integration::engine::MagnetarPasskeyAuthEngine` · framework/src/magnetar_integration/engine.rs:408
  - Implemented here by: `magnetar_integration::engine::MagnetarHostPasskeyService`
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasskeyAuthEngine::passkey_begin_registration` · framework/src/magnetar_integration/engine.rs:410 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasskeyAuthEngine::passkey_finish_registration` · framework/src/magnetar_integration/engine.rs:415 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasskeyAuthEngine::passkey_begin_authentication` · framework/src/magnetar_integration/engine.rs:422 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasskeyAuthEngine::passkey_finish_authentication` · framework/src/magnetar_integration/engine.rs:424 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasskeyAuthEngine::passkey_user_by_id` · framework/src/magnetar_integration/engine.rs:432 (required)
- [ ] trait `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine` · framework/src/magnetar_integration/engine.rs:1126
  - Implemented here by: `magnetar_integration::engine::MagnetarHostEngine`
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::password_sign_in` · framework/src/magnetar_integration/engine.rs:1128 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::complete_challenge` · framework/src/magnetar_integration/engine.rs:1132 (provided)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::issue_password_reset` · framework/src/magnetar_integration/engine.rs:1144 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::check_password_reset` · framework/src/magnetar_integration/engine.rs:1146 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::complete_password_reset` · framework/src/magnetar_integration/engine.rs:1148 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::password_register` · framework/src/magnetar_integration/engine.rs:1154 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::bearer_user_id` · framework/src/magnetar_integration/engine.rs:1156 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::issue_remember` · framework/src/magnetar_integration/engine.rs:1158 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::remember_sign_in` · framework/src/magnetar_integration/engine.rs:1164 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::remember_sign_in_attempt` · framework/src/magnetar_integration/engine.rs:1176 (provided)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::resolve_web_binding` · framework/src/magnetar_integration/engine.rs:1191 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::revoke_remember` · framework/src/magnetar_integration/engine.rs:1193 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::revoke_remember_selector` · framework/src/magnetar_integration/engine.rs:1205 (provided)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::user_by_id` · framework/src/magnetar_integration/engine.rs:1213 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::revoke_session` · framework/src/magnetar_integration/engine.rs:1215 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::revoke_all_sessions` · framework/src/magnetar_integration/engine.rs:1217 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::list_sessions` · framework/src/magnetar_integration/engine.rs:1219 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::record_failed_attempt` · framework/src/magnetar_integration/engine.rs:1221 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::admit_attempt` · framework/src/magnetar_integration/engine.rs:1230 (provided)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::cancel_attempt` · framework/src/magnetar_integration/engine.rs:1238 (provided)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::finalize_failed_attempt` · framework/src/magnetar_integration/engine.rs:1246 (provided)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::reset_admitted_attempts` · framework/src/magnetar_integration/engine.rs:1258 (provided)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::lockout_status` · framework/src/magnetar_integration/engine.rs:1270 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::reset_attempts` · framework/src/magnetar_integration/engine.rs:1272 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::unlock_account` · framework/src/magnetar_integration/engine.rs:1274 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::magic_link_send` · framework/src/magnetar_integration/engine.rs:1276 (required)
  - [ ] fn `suprnova::magnetar_integration::engine::MagnetarPasswordAuthEngine::magic_link_consume` · framework/src/magnetar_integration/engine.rs:1278 (required)

### `suprnova::magnetar_integration::middleware` (feature: `database-sqlite` or `database-postgres` or `database-mysql`)

- [ ] struct `suprnova::BearerTokenMiddleware` · framework/src/magnetar_integration/middleware.rs:11 (also `suprnova::magnetar_integration::middleware::BearerTokenMiddleware`)
  - Implements: `suprnova::Middleware`

### `suprnova::magnetar_integration::password`

- [ ] struct `suprnova::magnetar_integration::password::PasswordAuth` · framework/src/magnetar_integration/password.rs:7
  - [ ] fn `suprnova::magnetar_integration::password::PasswordAuth::register` · framework/src/magnetar_integration/password.rs:15
  - [ ] fn `suprnova::magnetar_integration::password::PasswordAuth::authenticate` · framework/src/magnetar_integration/password.rs:37
  - [ ] fn `suprnova::magnetar_integration::password::PasswordAuth::authenticate_outcome` · framework/src/magnetar_integration/password.rs:58

### `suprnova::magnetar_integration::sign_in` (private module; items are public through re-exports)

- [ ] enum `suprnova::SignInOutcome` · framework/src/magnetar_integration/sign_in.rs:16 (also `suprnova::magnetar_integration::SignInOutcome`)
  - Variants: `Authenticated`, `FactorRequired`

### `suprnova::magnetar_integration`

- [ ] fn `suprnova::magnetar_integration::find_user_by_id` · framework/src/magnetar_integration/mod.rs:1020
- [ ] fn `suprnova::magnetar_integration::install_magnetar_engines` · framework/src/magnetar_integration/mod.rs:537
- [ ] fn `suprnova::magnetar_integration::install_magnetar_engines_with_factor` · framework/src/magnetar_integration/mod.rs:557
- [ ] fn `suprnova::install_magnetar_oauth_engine` · framework/src/magnetar_integration/mod.rs:829 (feature: `magnetar-oauth`; also `suprnova::magnetar_integration::install_magnetar_oauth_engine`)
- [ ] fn `suprnova::install_magnetar_oauth_engine_with_factor` · framework/src/magnetar_integration/mod.rs:853 (feature: `magnetar-oauth`; also `suprnova::magnetar_integration::install_magnetar_oauth_engine_with_factor`)
- [ ] fn `suprnova::magnetar_integration::list_sessions` · framework/src/magnetar_integration/mod.rs:879
- [ ] fn `suprnova::magnetar_integration::revoke_all_sessions` · framework/src/magnetar_integration/mod.rs:870
- [ ] fn `suprnova::magnetar_integration::revoke_session` · framework/src/magnetar_integration/mod.rs:861
- [ ] struct `suprnova::FactorAuth` · framework/src/magnetar_integration/mod.rs:40 (feature: `database-sqlite` or `database-postgres` or `database-mysql`; also `suprnova::magnetar_integration::FactorAuth`)
  - [ ] fn `suprnova::FactorAuth::complete_challenge` · framework/src/magnetar_integration/mod.rs:54
