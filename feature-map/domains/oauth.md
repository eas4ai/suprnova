# Feature map: `manual/oauth.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 185 checked.

## Rust API: suprnova

### Re-exported from other crates

- [ ] struct `suprnova::magnetar_integration::passkey::CreationChallengeResponse` re-exports `webauthn_rs_proto::attest::CreationChallengeResponse`
- [ ] struct `suprnova::magnetar_integration::passkey::PasskeyAuthentication` re-exports `webauthn_rs::interface::PasskeyAuthentication`
- [ ] struct `suprnova::magnetar_integration::passkey::PasskeyAuthenticationResult` re-exports `webauthn_rs_core::interface::AuthenticationResult`
- [ ] struct `suprnova::magnetar_integration::passkey::PasskeyRegistration` re-exports `webauthn_rs::interface::PasskeyRegistration`
- [ ] struct `suprnova::magnetar_integration::passkey::PublicKeyCredential` re-exports `webauthn_rs_proto::auth::PublicKeyCredential`
- [ ] struct `suprnova::magnetar_integration::passkey::RegisterPublicKeyCredential` re-exports `webauthn_rs_proto::attest::RegisterPublicKeyCredential`
- [ ] struct `suprnova::magnetar_integration::passkey::RequestChallengeResponse` re-exports `webauthn_rs_proto::auth::RequestChallengeResponse`

### `suprnova::magnetar_integration::magic_link`

- [ ] struct `suprnova::magnetar_integration::magic_link::MagicLinkAuth` · framework/src/magnetar_integration/magic_link.rs:7
  - [ ] fn `suprnova::magnetar_integration::magic_link::MagicLinkAuth::send` · framework/src/magnetar_integration/magic_link.rs:19
  - [ ] fn `suprnova::magnetar_integration::magic_link::MagicLinkAuth::consume` · framework/src/magnetar_integration/magic_link.rs:38
  - [ ] fn `suprnova::magnetar_integration::magic_link::MagicLinkAuth::consume_outcome` · framework/src/magnetar_integration/magic_link.rs:53

### `suprnova::magnetar_integration::oauth_transport` (feature: `magnetar-oauth`)

- [ ] struct `suprnova::ReqwestOAuthTransport` · framework/src/magnetar_integration/oauth_transport.rs:24 (also `suprnova::magnetar_integration::oauth_transport::ReqwestOAuthTransport`)
  - [ ] fn `suprnova::ReqwestOAuthTransport::try_default` · framework/src/magnetar_integration/oauth_transport.rs:31
  - [ ] fn `suprnova::ReqwestOAuthTransport::new` · framework/src/magnetar_integration/oauth_transport.rs:45
  - [ ] fn `suprnova::ReqwestOAuthTransport::with_max_response_bytes` · framework/src/magnetar_integration/oauth_transport.rs:53

### `suprnova::magnetar_integration::oauth` (feature: `magnetar-oauth`)

- [ ] struct `suprnova::magnetar_integration::oauth::AppleIdentity` · framework/src/magnetar_integration/oauth.rs:34
  - Public fields: `provider`, `subject`, `email`, `email_verified`, `is_private_email`
- [ ] struct `suprnova::magnetar_integration::oauth::OAuthAuth` · framework/src/magnetar_integration/oauth.rs:48
  - [ ] fn `suprnova::magnetar_integration::oauth::OAuthAuth::begin` · framework/src/magnetar_integration/oauth.rs:63
  - [ ] fn `suprnova::magnetar_integration::oauth::OAuthAuth::verify_oauth_identity` · framework/src/magnetar_integration/oauth.rs:92
  - [ ] fn `suprnova::magnetar_integration::oauth::OAuthAuth::verify_apple_identity` · framework/src/magnetar_integration/oauth.rs:111
  - [ ] fn `suprnova::magnetar_integration::oauth::OAuthAuth::complete_with_apple_form_post` · framework/src/magnetar_integration/oauth.rs:144
  - [ ] fn `suprnova::magnetar_integration::oauth::OAuthAuth::complete_with_apple_form_post_outcome` · framework/src/magnetar_integration/oauth.rs:164
  - [ ] fn `suprnova::magnetar_integration::oauth::OAuthAuth::complete` · framework/src/magnetar_integration/oauth.rs:180
  - [ ] fn `suprnova::magnetar_integration::oauth::OAuthAuth::complete_outcome` · framework/src/magnetar_integration/oauth.rs:199
- [ ] struct `suprnova::magnetar_integration::oauth::OAuthIdentity` · framework/src/magnetar_integration/oauth.rs:21
  - Public fields: `provider`, `subject`, `email`, `name`
- [ ] struct `suprnova::magnetar_integration::oauth::OAuthKickoff` · framework/src/magnetar_integration/oauth.rs:12
  - Public fields: `authorization_url`, `state`

### `suprnova::magnetar_integration::passkey`

- [ ] struct `suprnova::magnetar_integration::passkey::PasskeyAuth` · framework/src/magnetar_integration/passkey.rs:99
  - [ ] fn `suprnova::magnetar_integration::passkey::PasskeyAuth::begin_registration` · framework/src/magnetar_integration/passkey.rs:108
  - [ ] fn `suprnova::magnetar_integration::passkey::PasskeyAuth::finish_registration` · framework/src/magnetar_integration/passkey.rs:147
  - [ ] fn `suprnova::magnetar_integration::passkey::PasskeyAuth::begin_authentication` · framework/src/magnetar_integration/passkey.rs:169
  - [ ] fn `suprnova::magnetar_integration::passkey::PasskeyAuth::finish_authentication` · framework/src/magnetar_integration/passkey.rs:194
  - [ ] fn `suprnova::magnetar_integration::passkey::PasskeyAuth::finish_authentication_outcome` · framework/src/magnetar_integration/passkey.rs:213
- [ ] struct `suprnova::magnetar_integration::passkey::PasskeyAuthenticationChallenge` · framework/src/magnetar_integration/passkey.rs:34
  - Public fields: `challenge`, `user_email`, `raw_options`
- [ ] struct `suprnova::magnetar_integration::passkey::PasskeyRegistrationChallenge` · framework/src/magnetar_integration/passkey.rs:21
  - Public fields: `challenge`, `user_email`, `rp_id`, `raw_options`

## Rust API: suprnova-magnetar

### `magnetar::oauth::authorization` (feature: `oauth`, off by default)

- [ ] struct `magnetar::oauth::OAuthAuthorizationConfig` · crates/suprnova-magnetar/src/oauth/authorization.rs:138 (also `magnetar::oauth::authorization::OAuthAuthorizationConfig`) (re-exported as `suprnova::OAuthAuthorizationConfig`)
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
- [ ] enum `magnetar::oauth::OAuthProtocolError` · crates/suprnova-magnetar/src/oauth/errors.rs:56 (also `magnetar::oauth::errors::OAuthProtocolError`) (re-exported as `suprnova::OAuthProtocolError`)
  - Variants: `InvalidRequestShape`, `MalformedTokenResponse`, `MalformedProviderResponse`, `ProviderReportedError`, `IdentityVerificationFailed`, `UpstreamUnavailable`, `ProviderConfiguration`
  - [ ] fn `magnetar::oauth::OAuthProtocolError::class` · crates/suprnova-magnetar/src/oauth/errors.rs:113
  - [ ] fn `magnetar::oauth::OAuthProtocolError::provider` · crates/suprnova-magnetar/src/oauth/errors.rs:126
  - [ ] fn `magnetar::oauth::OAuthProtocolError::trace_context` · crates/suprnova-magnetar/src/oauth/errors.rs:138
- [ ] type `magnetar::oauth::OAuthResult` · crates/suprnova-magnetar/src/oauth/errors.rs:10 (also `magnetar::oauth::errors::OAuthResult`) (re-exported as `suprnova::OAuthResult`)

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
- [ ] enum `magnetar::oauth::AutoLinkPolicy` · crates/suprnova-magnetar/src/oauth/identity.rs:76 (also `magnetar::oauth::identity::AutoLinkPolicy`) (re-exported as `suprnova::AutoLinkPolicy`)
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

- [ ] struct `magnetar::oauth::ClientAuthenticationMaterial` · crates/suprnova-magnetar/src/oauth/provider.rs:268 (also `magnetar::oauth::provider::ClientAuthenticationMaterial`) (re-exported as `suprnova::ClientAuthenticationMaterial`)
  - Public fields: `params`, `headers`
- [ ] struct `magnetar::oauth::EndpointOverrides` · crates/suprnova-magnetar/src/oauth/provider.rs:64 (also `magnetar::oauth::provider::EndpointOverrides`) (re-exported as `suprnova::EndpointOverrides`)
  - Public fields: `authorization_endpoint`, `token_endpoint`, `userinfo_endpoint`, `revocation_endpoint`, `device_authorization_endpoint`, `device_token_endpoint`
- [ ] struct `magnetar::oauth::OAuthProviderRegistry` · crates/suprnova-magnetar/src/oauth/provider.rs:370 (also `magnetar::oauth::provider::OAuthProviderRegistry`)
  - [ ] fn `magnetar::oauth::OAuthProviderRegistry::new` · crates/suprnova-magnetar/src/oauth/provider.rs:377
  - [ ] fn `magnetar::oauth::OAuthProviderRegistry::register` · crates/suprnova-magnetar/src/oauth/provider.rs:390
  - [ ] fn `magnetar::oauth::OAuthProviderRegistry::get` · crates/suprnova-magnetar/src/oauth/provider.rs:404
  - [ ] fn `magnetar::oauth::OAuthProviderRegistry::names` · crates/suprnova-magnetar/src/oauth/provider.rs:410
- [ ] struct `magnetar::oauth::RefreshPolicy` · crates/suprnova-magnetar/src/oauth/provider.rs:194 (also `magnetar::oauth::provider::RefreshPolicy`) (re-exported as `suprnova::RefreshPolicy`)
  - Public fields: `supported`, `token_client_authentication`, `extra_authorization_params`, `required_scopes`, `requires_reconsent_for_reissue`, `invalid_grant_meaning`
- [ ] struct `magnetar::oauth::RevocationRequest` · crates/suprnova-magnetar/src/oauth/provider.rs:231 (also `magnetar::oauth::provider::RevocationRequest`) (re-exported as `suprnova::RevocationRequest`)
  - Public fields: `method`, `endpoint`, `placement`, `params`, `headers`
- [ ] enum `magnetar::oauth::ClientAuthentication` · crates/suprnova-magnetar/src/oauth/provider.rs:149 (also `magnetar::oauth::provider::ClientAuthentication`) (re-exported as `suprnova::ClientAuthentication`)
  - Variants: `RequestBody`, `HttpBasic`, `SignedJwt`
- [ ] enum `magnetar::oauth::InvalidGrantMeaning` · crates/suprnova-magnetar/src/oauth/provider.rs:179 (also `magnetar::oauth::provider::InvalidGrantMeaning`) (re-exported as `suprnova::InvalidGrantMeaning`)
  - Variants: `ReuseOrExternalRevocation`, `OrdinaryRevocation`
- [ ] enum `magnetar::oauth::ParamPlacement` · crates/suprnova-magnetar/src/oauth/provider.rs:217 (also `magnetar::oauth::provider::ParamPlacement`) (re-exported as `suprnova::ParamPlacement`)
  - Variants: `Body`, `Query`
- [ ] enum `magnetar::oauth::ProviderResponse` · crates/suprnova-magnetar/src/oauth/provider.rs:117 (also `magnetar::oauth::provider::ProviderResponse`) (re-exported as `suprnova::ProviderResponse`)
  - Variants: `UserInfo`, `AppleIdToken`
- [ ] enum `magnetar::oauth::TokenHint` · crates/suprnova-magnetar/src/oauth/provider.rs:87 (also `magnetar::oauth::provider::TokenHint`) (re-exported as `suprnova::TokenHint`)
  - Variants: `Access`, `Refresh`
  - [ ] fn `magnetar::oauth::TokenHint::wire_value` · crates/suprnova-magnetar/src/oauth/provider.rs:97
- [ ] trait `magnetar::oauth::OAuthProvider` · crates/suprnova-magnetar/src/oauth/provider.rs:293 (also `magnetar::oauth::provider::OAuthProvider`) (re-exported as `suprnova::OAuthProvider`)
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
- [ ] trait `magnetar::oauth::RevocationTransport` · crates/suprnova-magnetar/src/oauth/provider.rs:281 (also `magnetar::oauth::provider::RevocationTransport`) (re-exported as `suprnova::RevocationTransport`)
  - [ ] fn `magnetar::oauth::RevocationTransport::send` · crates/suprnova-magnetar/src/oauth/provider.rs:285 (required)
- [ ] type `magnetar::oauth::ProviderIdentity` · crates/suprnova-magnetar/src/oauth/provider.rs:41 (also `magnetar::oauth::provider::ProviderIdentity`) (re-exported as `suprnova::ProviderIdentity`)

### `magnetar::oauth::request_shape` (feature: `oauth`, off by default)

- [ ] fn `magnetar::oauth::render_authorization_request` · crates/suprnova-magnetar/src/oauth/request_shape.rs:175 (also `magnetar::oauth::request_shape::render_authorization_request`)
- [ ] fn `magnetar::oauth::render_token_request` · crates/suprnova-magnetar/src/oauth/request_shape.rs:247 (also `magnetar::oauth::request_shape::render_token_request`)
- [ ] struct `magnetar::oauth::AuthorizationRequestParams` · crates/suprnova-magnetar/src/oauth/request_shape.rs:119 (also `magnetar::oauth::request_shape::AuthorizationRequestParams`)
  - Public fields: `client_id`, `redirect_uri`, `scopes`, `state`, `code_challenge`, `nonce`
- [ ] struct `magnetar::oauth::AuthorizationRequestShape` · crates/suprnova-magnetar/src/oauth/request_shape.rs:39 (also `magnetar::oauth::request_shape::AuthorizationRequestShape`) (re-exported as `suprnova::AuthorizationRequestShape`)
  - Public fields: `client_id_param`, `scope_delimiter`, `always_send_scope`, `pkce`, `response_mode`, `requires_nonce`
- [ ] struct `magnetar::oauth::TokenRequestParams` · crates/suprnova-magnetar/src/oauth/request_shape.rs:143 (also `magnetar::oauth::request_shape::TokenRequestParams`)
  - Public fields: `client_id`, `code`, `redirect_uri`, `code_verifier`, `scopes`
- [ ] struct `magnetar::oauth::TokenRequestShape` · crates/suprnova-magnetar/src/oauth/request_shape.rs:81 (also `magnetar::oauth::request_shape::TokenRequestShape`) (re-exported as `suprnova::TokenRequestShape`)
  - Public fields: `client_id_param`, `scope_delimiter`, `always_send_scope`, `accept_http_success_error_body`
- [ ] enum `magnetar::oauth::PkcePosture` · crates/suprnova-magnetar/src/oauth/request_shape.rs:26 (also `magnetar::oauth::request_shape::PkcePosture`) (re-exported as `suprnova::PkcePosture`)
  - Variants: `Required`, `Disabled`

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
- [ ] struct `magnetar::passkey::PasskeyConfig` · crates/suprnova-magnetar/src/passkey/mod.rs:48 (re-exported as `suprnova::PasskeyConfig`, `suprnova::magnetar_integration::PasskeyConfig`)
  - Public fields: `rp_id`, `rp_origin`
- [ ] struct `magnetar::passkey::PasskeySummary` · crates/suprnova-magnetar/src/passkey/mod.rs:101
  - Public fields: `passkey_id`, `credential_id`, `name`, `created_at`, `last_used_at`
- [ ] struct `magnetar::passkey::RegistrationIntent` · crates/suprnova-magnetar/src/passkey/mod.rs:69
  - Public fields: `email`, `actor`, `reauthenticated_at`

### `magnetar::plugins::oauth_apple` (feature: `oauth-apple`, off by default)

- [ ] struct `magnetar::plugins::oauth_apple::AppleClaims` · crates/suprnova-magnetar/src/plugins/oauth_apple.rs:131
  - Public fields: `subject`, `email`, `email_verified`, `is_private_email`
- [ ] struct `magnetar::plugins::oauth_apple::AppleOAuthProvider` · crates/suprnova-magnetar/src/plugins/oauth_apple.rs:280 (re-exported as `suprnova::AppleOAuthProvider`)
  - [ ] fn `magnetar::plugins::oauth_apple::AppleOAuthProvider::new` · crates/suprnova-magnetar/src/plugins/oauth_apple.rs:310
- [ ] struct `magnetar::plugins::oauth_apple::AppleProviderConfig` · crates/suprnova-magnetar/src/plugins/oauth_apple.rs:109 (re-exported as `suprnova::AppleProviderConfig`)
  - Public fields: `client_id`, `team_id`, `key_id`, `private_key_pem`, `redirect_uri`, `scopes`, `endpoints`
- [ ] struct `magnetar::plugins::oauth_apple::LiveApplePublicKeySource` · crates/suprnova-magnetar/src/plugins/oauth_apple.rs:163
  - [ ] fn `magnetar::plugins::oauth_apple::LiveApplePublicKeySource::new` · crates/suprnova-magnetar/src/plugins/oauth_apple.rs:175
- [ ] trait `magnetar::plugins::oauth_apple::ApplePublicKeySource` · crates/suprnova-magnetar/src/plugins/oauth_apple.rs:148 (re-exported as `suprnova::ApplePublicKeySource`)
  - Implemented here by: `plugins::oauth_apple::LiveApplePublicKeySource`
  - [ ] fn `magnetar::plugins::oauth_apple::ApplePublicKeySource::verify` · crates/suprnova-magnetar/src/plugins/oauth_apple.rs:151 (required)

### `magnetar::plugins::oauth_facebook` (feature: `oauth-facebook`, off by default)

- [ ] struct `magnetar::plugins::oauth_facebook::FacebookOAuthProvider` · crates/suprnova-magnetar/src/plugins/oauth_facebook.rs:125 (re-exported as `suprnova::FacebookOAuthProvider`)
  - [ ] fn `magnetar::plugins::oauth_facebook::FacebookOAuthProvider::new` · crates/suprnova-magnetar/src/plugins/oauth_facebook.rs:138
- [ ] struct `magnetar::plugins::oauth_facebook::FacebookProviderConfig` · crates/suprnova-magnetar/src/plugins/oauth_facebook.rs:82 (re-exported as `suprnova::FacebookProviderConfig`)
  - Public fields: `client_id`, `client_secret`, `redirect_uri`, `scopes`, `graph_api_version`, `endpoints`
- [ ] const `magnetar::plugins::oauth_facebook::DEFAULT_GRAPH_API_VERSION` · crates/suprnova-magnetar/src/plugins/oauth_facebook.rs:78

### `magnetar::plugins::oauth_google` (feature: `oauth-google`, off by default)

- [ ] struct `magnetar::plugins::oauth_google::GoogleOAuthProvider` · crates/suprnova-magnetar/src/plugins/oauth_google.rs:81 (re-exported as `suprnova::GoogleOAuthProvider`)
  - [ ] fn `magnetar::plugins::oauth_google::GoogleOAuthProvider::new` · crates/suprnova-magnetar/src/plugins/oauth_google.rs:94
- [ ] struct `magnetar::plugins::oauth_google::GoogleProviderConfig` · crates/suprnova-magnetar/src/plugins/oauth_google.rs:59 (re-exported as `suprnova::GoogleProviderConfig`)
  - Public fields: `client_id`, `client_secret`, `redirect_uri`, `scopes`, `endpoints`

### `magnetar::plugins::oauth_tiktok` (feature: `oauth-tiktok`, off by default)

- [ ] struct `magnetar::plugins::oauth_tiktok::TikTokOAuthProvider` · crates/suprnova-magnetar/src/plugins/oauth_tiktok.rs:123 (re-exported as `suprnova::TikTokOAuthProvider`)
  - [ ] fn `magnetar::plugins::oauth_tiktok::TikTokOAuthProvider::new` · crates/suprnova-magnetar/src/plugins/oauth_tiktok.rs:132
- [ ] struct `magnetar::plugins::oauth_tiktok::TikTokProviderConfig` · crates/suprnova-magnetar/src/plugins/oauth_tiktok.rs:82 (re-exported as `suprnova::TikTokProviderConfig`)
  - Public fields: `client_id`, `client_secret`, `redirect_uri`, `scopes`, `endpoints`

### `magnetar::plugins::oauth_x` (feature: `oauth-x`, off by default)

- [ ] struct `magnetar::plugins::oauth_x::XOAuthProvider` · crates/suprnova-magnetar/src/plugins/oauth_x.rs:85 (re-exported as `suprnova::XOAuthProvider`)
  - [ ] fn `magnetar::plugins::oauth_x::XOAuthProvider::new` · crates/suprnova-magnetar/src/plugins/oauth_x.rs:94
- [ ] struct `magnetar::plugins::oauth_x::XProviderConfig` · crates/suprnova-magnetar/src/plugins/oauth_x.rs:59 (re-exported as `suprnova::XProviderConfig`)
  - Public fields: `client_id`, `client_secret`, `redirect_uri`, `scopes`, `endpoints`
