# Suprnova feature map: `suprnova` Rust API

Source: `framework/` at d03b4f1, rustdoc JSON (all features), cross-checked against a default-features build.

A checked box means the documentation for that item has been remediated
against the source. Items are listed under their shortest public path;
`also` names the other paths the same item is reachable by.

## Counts

- Top-level items (including re-exports): 2237
- Members (methods, associated consts and types): 3756
- By kind: constant 84, enum 190, function 493, macro 29, reexport 306, static 5, struct 954, trait 122, type_alias 52, unnameable 2

## Re-exported from other crates

Items from sibling Suprnova crates are mapped in those crates' files.

- [ ] trait `suprnova::AbuseLimiter` re-exports `magnetar::abuse::AbuseLimiter` (feature: `magnetar-oauth`)
- [ ] struct `suprnova::AbusePolicy` re-exports `magnetar::abuse::AbusePolicy` (feature: `magnetar-oauth`)
- [ ] trait `suprnova::ActiveModelBehavior` re-exports `sea_orm::entity::active_model::ActiveModelBehavior`
- [ ] trait `suprnova::ActiveModelTrait` re-exports `sea_orm::entity::active_model::ActiveModelTrait`
- [ ] enum `suprnova::ActiveValue` re-exports `sea_orm::entity::active_value::ActiveValue`
- [ ] struct `suprnova::AppleOAuthProvider` re-exports `magnetar::plugins::oauth_apple::AppleOAuthProvider` (feature: `magnetar-oauth`)
- [ ] struct `suprnova::AppleProviderConfig` re-exports `magnetar::plugins::oauth_apple::AppleProviderConfig` (feature: `magnetar-oauth`)
- [ ] trait `suprnova::ApplePublicKeySource` re-exports `magnetar::plugins::oauth_apple::ApplePublicKeySource` (feature: `magnetar-oauth`)
- [ ] struct `suprnova::AuthorizationRequestShape` re-exports `magnetar::oauth::request_shape::AuthorizationRequestShape` (feature: `magnetar-oauth`)
- [ ] enum `suprnova::AutoLinkPolicy` re-exports `magnetar::oauth::identity::AutoLinkPolicy` (feature: `magnetar-oauth`)
- [ ] enum `suprnova::ClientAuthentication` re-exports `magnetar::oauth::provider::ClientAuthentication` (feature: `magnetar-oauth`)
- [ ] struct `suprnova::ClientAuthenticationMaterial` re-exports `magnetar::oauth::provider::ClientAuthenticationMaterial` (feature: `magnetar-oauth`)
- [ ] trait `suprnova::ColumnTrait` re-exports `sea_orm::entity::column::ColumnTrait`
- [ ] proc derive `suprnova::Command` re-exports `suprnova_macros::Command`
- [ ] trait `suprnova::ConnectionTrait` re-exports `sea_orm::database::connection::ConnectionTrait`
- [ ] enum `suprnova::ContentEncoding` re-exports `suprnova_web_push::payload::ContentEncoding` (feature: `web-push`)
- [ ] enum `suprnova::Currency` re-exports `iso_currency::Currency`
- [ ] proc derive `suprnova::Data` re-exports `suprnova_macros::Data`
- [ ] struct `suprnova::DatabaseConnection` re-exports `sea_orm::database::db_connection::DatabaseConnection`
- [ ] struct `suprnova::DatabaseTransaction` re-exports `sea_orm::database::transaction::DatabaseTransaction`
- [ ] enum `suprnova::DbErr` re-exports `sea_orm::error::DbErr`
- [ ] proc derive `suprnova::DeriveActiveEnum` re-exports `sea_orm_macros::DeriveActiveEnum`
- [ ] proc derive `suprnova::Dummy` re-exports `dummy::Dummy`
- [ ] trait `suprnova::Dummy` re-exports `fake::Dummy`
- [ ] struct `suprnova::EndpointOverrides` re-exports `magnetar::oauth::provider::EndpointOverrides` (feature: `magnetar-oauth`)
- [ ] enum `suprnova::EndpointPolicy` re-exports `suprnova_web_push::client::EndpointPolicy` (feature: `web-push`)
- [ ] trait `suprnova::EntityName` re-exports `sea_orm::entity::base_entity::EntityName`
- [ ] trait `suprnova::EntityTrait` re-exports `sea_orm::entity::base_entity::EntityTrait`
- [ ] trait `suprnova::Evaluator` re-exports `featureflag::evaluator::Evaluator`
- [ ] struct `suprnova::EvaluatorRef` re-exports `featureflag::evaluator::EvaluatorRef`
- [ ] struct `suprnova::FacebookOAuthProvider` re-exports `magnetar::plugins::oauth_facebook::FacebookOAuthProvider` (feature: `magnetar-oauth`)
- [ ] struct `suprnova::FacebookProviderConfig` re-exports `magnetar::plugins::oauth_facebook::FacebookProviderConfig` (feature: `magnetar-oauth`)
- [ ] proc derive `suprnova::Factory` re-exports `suprnova_macros::Factory`
- [ ] trait `suprnova::Fake` re-exports `fake::Fake`
- [ ] struct `suprnova::Faker` re-exports `fake::Faker`
- [ ] struct `suprnova::Feature` re-exports `featureflag::feature::Feature`
- [ ] proc derive `suprnova::FormRequestDerive` re-exports `suprnova_macros::FormRequest`
- [ ] struct `suprnova::GoogleOAuthProvider` re-exports `magnetar::plugins::oauth_google::GoogleOAuthProvider` (feature: `magnetar-oauth`)
- [ ] struct `suprnova::GoogleProviderConfig` re-exports `magnetar::plugins::oauth_google::GoogleProviderConfig` (feature: `magnetar-oauth`)
- [ ] struct `suprnova::HeaderMap` re-exports `http::header::map::HeaderMap`
- [ ] trait `suprnova::Iden` re-exports `sea_query::types::iden::core::Iden`
- [ ] proc derive `suprnova::Iden` re-exports `sea_query_derive::Iden`
- [ ] proc derive `suprnova::InertiaProps` re-exports `suprnova_macros::InertiaProps`
- [ ] trait `suprnova::IntoActiveModel` re-exports `sea_orm::entity::active_model::IntoActiveModel`
- [ ] enum `suprnova::InvalidGrantMeaning` re-exports `magnetar::oauth::provider::InvalidGrantMeaning` (feature: `magnetar-oauth`)
- [ ] trait `suprnova::Iterable` re-exports `strum::IntoEnumIterator`
- [ ] proc derive `suprnova::LiveComponent` re-exports `suprnova_macros::LiveComponent`
- [ ] enum `suprnova::MagnetarError` re-exports `magnetar::error::Error` (feature: `magnetar-oauth`)
- [ ] type alias `suprnova::MagnetarResult` re-exports `magnetar::error::Result` (feature: `magnetar-oauth`)
- [ ] struct `suprnova::Method` re-exports `http::method::Method`
- [ ] trait `suprnova::ModelTrait` re-exports `sea_orm::entity::model::ModelTrait`
- [ ] proc derive `suprnova::MultipartRequest` re-exports `suprnova_macros::MultipartRequest`
- [ ] variant `suprnova::NotSet` re-exports `sea_orm::entity::active_value::ActiveValue::NotSet`
- [ ] proc derive `suprnova::NotificationMailable` re-exports `suprnova_macros::NotificationMailable`
- [ ] struct `suprnova::OAuthAuthorizationConfig` re-exports `magnetar::oauth::authorization::OAuthAuthorizationConfig` (feature: `magnetar-oauth`)
- [ ] struct `suprnova::OAuthHttpRequest` re-exports `magnetar::plugin::context::HttpRequest` (feature: `magnetar-oauth`)
- [ ] struct `suprnova::OAuthHttpResponse` re-exports `magnetar::plugin::context::HttpResponse` (feature: `magnetar-oauth`)
- [ ] trait `suprnova::OAuthHttpTransport` re-exports `magnetar::plugin::context::HttpTransport` (feature: `magnetar-oauth`)
- [ ] enum `suprnova::OAuthProtocolError` re-exports `magnetar::oauth::errors::OAuthProtocolError` (feature: `magnetar-oauth`)
- [ ] trait `suprnova::OAuthProvider` re-exports `magnetar::oauth::provider::OAuthProvider` (feature: `magnetar-oauth`)
- [ ] type alias `suprnova::OAuthResult` re-exports `magnetar::oauth::errors::OAuthResult` (feature: `magnetar-oauth`)
- [ ] enum `suprnova::ParamPlacement` re-exports `magnetar::oauth::provider::ParamPlacement` (feature: `magnetar-oauth`)
- [ ] struct `suprnova::PasskeyConfig` re-exports `magnetar::passkey::PasskeyConfig` (feature: `database-sqlite` or `database-postgres` or `database-mysql`)
- [ ] enum `suprnova::Permit` re-exports `magnetar::abuse::Permit` (feature: `magnetar-oauth`)
- [ ] enum `suprnova::PkcePosture` re-exports `magnetar::oauth::request_shape::PkcePosture` (feature: `magnetar-oauth`)
- [ ] trait `suprnova::PrimaryKeyToColumn` re-exports `sea_orm::entity::primary_key::PrimaryKeyToColumn`
- [ ] trait `suprnova::PrimaryKeyTrait` re-exports `sea_orm::entity::primary_key::PrimaryKeyTrait`
- [ ] type alias `suprnova::ProviderIdentity` re-exports `magnetar::oauth::provider::ProviderIdentity` (feature: `magnetar-oauth`)
- [ ] enum `suprnova::ProviderResponse` re-exports `magnetar::oauth::provider::ProviderResponse` (feature: `magnetar-oauth`)
- [ ] struct `suprnova::PushResponse` re-exports `suprnova_web_push::client::PushResponse` (feature: `web-push`)
- [ ] trait `suprnova::QueryFilter` re-exports `sea_orm::query::helper::QueryFilter`
- [ ] trait `suprnova::QueryOrder` re-exports `sea_orm::query::helper::QueryOrder`
- [ ] trait `suprnova::QuerySelect` re-exports `sea_orm::query::helper::QuerySelect`
- [ ] struct `suprnova::RefreshPolicy` re-exports `magnetar::oauth::provider::RefreshPolicy` (feature: `magnetar-oauth`)
- [ ] struct `suprnova::RelationDef` re-exports `sea_orm::entity::relation::RelationDef`
- [ ] trait `suprnova::RelationTrait` re-exports `sea_orm::entity::relation::RelationTrait`
- [ ] struct `suprnova::RequestBodyStream` re-exports `hyper::body::incoming::Incoming`
- [ ] struct `suprnova::RevocationRequest` re-exports `magnetar::oauth::provider::RevocationRequest` (feature: `magnetar-oauth`)
- [ ] trait `suprnova::RevocationTransport` re-exports `magnetar::oauth::provider::RevocationTransport` (feature: `magnetar-oauth`)
- [ ] struct `suprnova::Schema` re-exports `sea_orm::schema::Schema`
- [ ] type alias `suprnova::SecretString` re-exports `secrecy::SecretString` (feature: `magnetar-oauth`)
- [ ] struct `suprnova::Select` re-exports `sea_orm::query::select::Select`
- [ ] variant `suprnova::Set` re-exports `sea_orm::entity::active_value::ActiveValue::Set`
- [ ] enum `suprnova::SqlErr` re-exports `sea_orm::error::SqlErr`
- [ ] struct `suprnova::StatusCode` re-exports `http::status::StatusCode`
- [ ] struct `suprnova::SubscriptionInfo` re-exports `suprnova_web_push::client::SubscriptionInfo` (feature: `web-push`)
- [ ] struct `suprnova::TikTokOAuthProvider` re-exports `magnetar::plugins::oauth_tiktok::TikTokOAuthProvider` (feature: `magnetar-oauth`)
- [ ] struct `suprnova::TikTokProviderConfig` re-exports `magnetar::plugins::oauth_tiktok::TikTokProviderConfig` (feature: `magnetar-oauth`)
- [ ] enum `suprnova::TokenHint` re-exports `magnetar::oauth::provider::TokenHint` (feature: `magnetar-oauth`)
- [ ] struct `suprnova::TokenRequestShape` re-exports `magnetar::oauth::request_shape::TokenRequestShape` (feature: `magnetar-oauth`)
- [ ] trait `suprnova::TransactionTrait` re-exports `sea_orm::database::connection::TransactionTrait`
- [ ] trait `suprnova::TryGetable` re-exports `sea_orm::executor::query::TryGetable`
- [ ] enum `suprnova::Tz` re-exports `chrono_tz::timezones::Tz`
- [ ] struct `suprnova::Uri` re-exports `http::uri::Uri`
- [ ] trait `suprnova::Validate` re-exports `validator::traits::Validate`
- [ ] proc derive `suprnova::Validate` re-exports `validator_derive::Validate`
- [ ] struct `suprnova::VapidClaims` re-exports `suprnova_web_push::vapid::VapidClaims` (feature: `web-push`)
- [ ] struct `suprnova::VapidKey` re-exports `suprnova_web_push::vapid::VapidKey` (feature: `web-push`)
- [ ] struct `suprnova::VapidSigner` re-exports `suprnova_web_push::vapid::VapidSigner` (feature: `web-push`)
- [ ] struct `suprnova::WebPushClient` re-exports `suprnova_web_push::client::WebPushClient` (feature: `web-push`)
- [ ] enum `suprnova::WebPushError` re-exports `suprnova_web_push::error::WebPushError` (feature: `web-push`)
- [ ] struct `suprnova::XOAuthProvider` re-exports `magnetar::plugins::oauth_x::XOAuthProvider` (feature: `magnetar-oauth`)
- [ ] struct `suprnova::XProviderConfig` re-exports `magnetar::plugins::oauth_x::XProviderConfig` (feature: `magnetar-oauth`)
- [ ] proc attribute `suprnova::accessor` re-exports `suprnova_macros::accessor`
- [ ] proc attribute `suprnova::async_trait` re-exports `async_trait::async_trait`
- [ ] module `suprnova::chrono` re-exports `chrono`
- [ ] module `suprnova::chrono_tz` re-exports `chrono_tz`
- [ ] proc attribute `suprnova::command` re-exports `suprnova_macros::command`
- [ ] module `suprnova::database::sea_orm` re-exports `sea_orm`
- [ ] macro `suprnova::describe` re-exports `suprnova_macros::describe`
- [ ] proc attribute `suprnova::domain_error` re-exports `suprnova_macros::domain_error`
- [ ] macro `suprnova::feature` re-exports `featureflag::feature`
- [ ] module `suprnova::feature` re-exports `featureflag::feature`
- [ ] struct `suprnova::features::Context` re-exports `featureflag::context::Context`
- [ ] trait `suprnova::features::Evaluator` re-exports `featureflag::evaluator::Evaluator`
- [ ] struct `suprnova::features::EvaluatorRef` re-exports `featureflag::evaluator::EvaluatorRef`
- [ ] struct `suprnova::features::Feature` re-exports `featureflag::feature::Feature`
- [ ] function `suprnova::features::set_global_default` re-exports `featureflag::evaluator::global::set_global_default`
- [ ] function `suprnova::features::try_set_global_default` re-exports `featureflag::evaluator::global::try_set_global_default`
- [ ] proc attribute `suprnova::handler` re-exports `suprnova_macros::handler`
- [ ] module `suprnova::hyper` re-exports `hyper`
- [ ] module `suprnova::indexmap` re-exports `indexmap`
- [ ] macro `suprnova::inertia_response` re-exports `suprnova_macros::inertia_response`
- [ ] proc attribute `suprnova::injectable` re-exports `suprnova_macros::injectable`
- [ ] macro `suprnova::is_enabled` re-exports `featureflag::is_enabled`
- [ ] proc attribute `suprnova::live` re-exports `suprnova_macros::live`
- [ ] enum `suprnova::live::AcceptedOutcomeKind` re-exports `suprnova_live::ledger::contract::AcceptedOutcomeKind`
- [ ] enum `suprnova::live::ActionOutcome` re-exports `suprnova_live::action::outcome::ActionOutcome`
- [ ] struct `suprnova::live::ActionResult` re-exports `suprnova_live::action::outcome::ActionResult`
- [ ] struct `suprnova::live::AuthorizedAction` re-exports `suprnova_live::action::dispatch::AuthorizedAction`
- [ ] struct `suprnova::live::BoundedHeaders` re-exports `suprnova_live::upload::direct_provider::BoundedHeaders`
- [ ] enum `suprnova::live::CanonicalValue` re-exports `suprnova_live::canonical::value::CanonicalValue`
- [ ] enum `suprnova::live::ChunkDisposition` re-exports `suprnova_live::upload::provider::ChunkDisposition`
- [ ] struct `suprnova::live::ChunkReceipt` re-exports `suprnova_live::upload::provider::ChunkReceipt`
- [ ] struct `suprnova::live::DirectPartReference` re-exports `suprnova_live::upload::direct_provider::DirectPartReference`
- [ ] struct `suprnova::live::DirectTransferInstruction` re-exports `suprnova_live::upload::direct_provider::DirectTransferInstruction`
- [ ] trait `suprnova::live::DirectUploadProvider` re-exports `suprnova_live::upload::provider::DirectUploadProvider`
- [ ] struct `suprnova::live::DurableUpload` re-exports `suprnova_live::upload::finalize::DurableUpload`
- [ ] struct `suprnova::live::DurableUploadId` re-exports `suprnova_live::upload::finalize::DurableUploadId`
- [ ] trait `suprnova::live::EffectPayloadMetadata` re-exports `suprnova_live::metadata::browser::EffectPayloadMetadata`
- [ ] struct `suprnova::live::ErrorBag` re-exports `suprnova_live::validation::error_bag::ErrorBag`
- [ ] enum `suprnova::live::ErrorCategory` re-exports `suprnova_live::error::ErrorCategory`
- [ ] trait `suprnova::live::EventPayloadMetadata` re-exports `suprnova_live::metadata::browser::EventPayloadMetadata`
- [ ] struct `suprnova::live::FailedFinalize` re-exports `suprnova_live::upload::finalize::FailedFinalize`
- [ ] struct `suprnova::live::FinalizeRequest` re-exports `suprnova_live::upload::finalize::FinalizeRequest`
- [ ] struct `suprnova::live::FinalizeToken` re-exports `suprnova_live::upload::finalize::FinalizeToken`
- [ ] struct `suprnova::live::IntegrityEvidence` re-exports `suprnova_live::upload::provider::IntegrityEvidence`
- [ ] proc derive `suprnova::live::LiveComponent` re-exports `suprnova_macros::LiveComponent`
- [ ] struct `suprnova::live::LiveError` re-exports `suprnova_live::error::LiveError`
- [ ] struct `suprnova::live::MountFlags` re-exports `suprnova_live::mount::output::MountFlags`
- [ ] struct `suprnova::live::PrepareTransfer` re-exports `suprnova_live::upload::provider::PrepareTransfer`
- [ ] struct `suprnova::live::PreparedFinalize` re-exports `suprnova_live::upload::finalize::PreparedFinalize`
- [ ] type alias `suprnova::live::QuarantineBytes` re-exports `suprnova_live::upload::quarantine::QuarantineBytes`
- [ ] struct `suprnova::live::ReadUpload` re-exports `suprnova_live::upload::provider::ReadUpload`
- [ ] enum `suprnova::live::RecoveryInstruction` re-exports `suprnova_live::error::RecoveryInstruction`
- [ ] struct `suprnova::live::ReportDirectPart` re-exports `suprnova_live::upload::direct_provider::ReportDirectPart`
- [ ] enum `suprnova::live::SafeDiagnosticCode` re-exports `suprnova_live::error::SafeDiagnosticCode`
- [ ] enum `suprnova::live::ScanDisposition` re-exports `suprnova_live::upload::validation::ScanDisposition`
- [ ] struct `suprnova::live::ScanInput` re-exports `suprnova_live::upload::validation::ScanInput`
- [ ] enum `suprnova::live::TransferDisposition` re-exports `suprnova_live::upload::provider::TransferDisposition`
- [ ] enum `suprnova::live::TransferInstruction` re-exports `suprnova_live::upload::direct_provider::TransferInstruction`
- [ ] enum `suprnova::live::TransferMethod` re-exports `suprnova_live::upload::direct_provider::TransferMethod`
- [ ] struct `suprnova::live::TransferPlan` re-exports `suprnova_live::upload::provider::TransferPlan`
- [ ] struct `suprnova::live::TrustedProviderOrigin` re-exports `suprnova_live::upload::direct_provider::TrustedProviderOrigin`
- [ ] struct `suprnova::live::TrustedProviderUrl` re-exports `suprnova_live::upload::direct_provider::TrustedProviderUrl`
- [ ] struct `suprnova::live::UnixMillis` re-exports `suprnova_live::identity::UnixMillis`
- [ ] trait `suprnova::live::UploadApplicationValidator` re-exports `suprnova_live::upload::validation::UploadApplicationValidator`
- [ ] struct `suprnova::live::UploadError` re-exports `suprnova_live::upload::identity::UploadError`
- [ ] enum `suprnova::live::UploadErrorKind` re-exports `suprnova_live::upload::identity::UploadErrorKind`
- [ ] trait `suprnova::live::UploadFinalizer` re-exports `suprnova_live::upload::finalize::UploadFinalizer`
- [ ] type alias `suprnova::live::UploadFuture` re-exports `suprnova_live::upload::ledger::UploadFuture`
- [ ] struct `suprnova::live::UploadHandle` re-exports `suprnova_live::upload::identity::UploadHandle`
- [ ] struct `suprnova::live::UploadLimitConfig` re-exports `suprnova_live::limits::UploadLimitConfig`
- [ ] struct `suprnova::live::UploadLimits` re-exports `suprnova_live::limits::UploadLimits`
- [ ] struct `suprnova::live::UploadPart` re-exports `suprnova_live::upload::direct_provider::UploadPart`
- [ ] trait `suprnova::live::UploadProvider` re-exports `suprnova_live::upload::provider::UploadProvider`
- [ ] trait `suprnova::live::UploadScanner` re-exports `suprnova_live::upload::validation::UploadScanner`
- [ ] struct `suprnova::live::ValidationIssue` re-exports `suprnova_live::validation::error_bag::ValidationIssue`
- [ ] struct `suprnova::live::ValidationMessageId` re-exports `suprnova_live::validation::error_bag::ValidationMessageId`
- [ ] enum `suprnova::live::ValidationStatus` re-exports `suprnova_live::validation::error_bag::ValidationStatus`
- [ ] struct `suprnova::live::VerifyTransfer` re-exports `suprnova_live::upload::provider::VerifyTransfer`
- [ ] enum `suprnova::live::action::ActionOutcome` re-exports `suprnova_live::action::outcome::ActionOutcome`
- [ ] struct `suprnova::live::action::ActionResult` re-exports `suprnova_live::action::outcome::ActionResult`
- [ ] struct `suprnova::live::action::AuthorizedAction` re-exports `suprnova_live::action::dispatch::AuthorizedAction`
- [ ] struct `suprnova::live::action::FlashIntent` re-exports `suprnova_live::action::outcome::FlashIntent`
- [ ] struct `suprnova::live::action::OutcomeError` re-exports `suprnova_live::action::outcome::OutcomeError`
- [ ] enum `suprnova::live::action::OutcomeErrorKind` re-exports `suprnova_live::action::outcome::OutcomeErrorKind`
- [ ] struct `suprnova::live::action::OutcomeMetadata` re-exports `suprnova_live::action::outcome::OutcomeMetadata`
- [ ] struct `suprnova::live::action::RouteIntent` re-exports `suprnova_live::action::outcome::RouteIntent`
- [ ] struct `suprnova::live::action::UrlIntent` re-exports `suprnova_live::action::outcome::UrlIntent`
- [ ] struct `suprnova::live::assets::ArtifactError` re-exports `suprnova_live::artifacts::ArtifactError`
- [ ] enum `suprnova::live::assets::ArtifactErrorKind` re-exports `suprnova_live::artifacts::ArtifactErrorKind`
- [ ] enum `suprnova::live::assets::ArtifactRole` re-exports `suprnova_live::artifacts::ArtifactRole`
- [ ] enum `suprnova::live::assets::PreloadRelation` re-exports `suprnova_live::artifacts::PreloadRelation`
- [ ] struct `suprnova::live::assets::RuntimeArtifact` re-exports `suprnova_live::artifacts::RuntimeArtifact`
- [ ] enum `suprnova::live::assets::ScriptKind` re-exports `suprnova_live::artifacts::ScriptKind`
- [ ] struct `suprnova::live::charts::ChartError` re-exports `suprnova_live::view::charts::ChartError`
- [ ] enum `suprnova::live::charts::ChartErrorKind` re-exports `suprnova_live::view::charts::ChartErrorKind`
- [ ] enum `suprnova::live::charts::ChartKind` re-exports `suprnova_live::view::charts::ChartKind`
- [ ] struct `suprnova::live::charts::ChartSeries` re-exports `suprnova_live::view::charts::ChartSeries`
- [ ] function `suprnova::live::charts::render_chart` re-exports `suprnova_live::view::charts::render_chart`
- [ ] enum `suprnova::live::error::ErrorCategory` re-exports `suprnova_live::error::ErrorCategory`
- [ ] struct `suprnova::live::error::LiveError` re-exports `suprnova_live::error::LiveError`
- [ ] enum `suprnova::live::error::RecoveryInstruction` re-exports `suprnova_live::error::RecoveryInstruction`
- [ ] enum `suprnova::live::error::SafeDiagnosticCode` re-exports `suprnova_live::error::SafeDiagnosticCode`
- [ ] proc attribute `suprnova::live::live` re-exports `suprnova_macros::live`
- [ ] trait `suprnova::live::metadata::EffectPayloadMetadata` re-exports `suprnova_live::metadata::browser::EffectPayloadMetadata`
- [ ] trait `suprnova::live::metadata::EventPayloadMetadata` re-exports `suprnova_live::metadata::browser::EventPayloadMetadata`
- [ ] enum `suprnova::live::validation::BagPolicy` re-exports `suprnova_live::validation::error_bag::BagPolicy`
- [ ] struct `suprnova::live::validation::ErrorBag` re-exports `suprnova_live::validation::error_bag::ErrorBag`
- [ ] struct `suprnova::live::validation::ValidationIssue` re-exports `suprnova_live::validation::error_bag::ValidationIssue`
- [ ] struct `suprnova::live::validation::ValidationMessageId` re-exports `suprnova_live::validation::error_bag::ValidationMessageId`
- [ ] enum `suprnova::live::validation::ValidationStatus` re-exports `suprnova_live::validation::error_bag::ValidationStatus`
- [ ] struct `suprnova::magnetar_integration::PasskeyConfig` re-exports `magnetar::passkey::PasskeyConfig`
- [ ] struct `suprnova::magnetar_integration::passkey::CreationChallengeResponse` re-exports `webauthn_rs_proto::attest::CreationChallengeResponse`
- [ ] struct `suprnova::magnetar_integration::passkey::PasskeyAuthentication` re-exports `webauthn_rs::interface::PasskeyAuthentication`
- [ ] struct `suprnova::magnetar_integration::passkey::PasskeyAuthenticationResult` re-exports `webauthn_rs_core::interface::AuthenticationResult`
- [ ] struct `suprnova::magnetar_integration::passkey::PasskeyRegistration` re-exports `webauthn_rs::interface::PasskeyRegistration`
- [ ] struct `suprnova::magnetar_integration::passkey::PublicKeyCredential` re-exports `webauthn_rs_proto::auth::PublicKeyCredential`
- [ ] struct `suprnova::magnetar_integration::passkey::RegisterPublicKeyCredential` re-exports `webauthn_rs_proto::attest::RegisterPublicKeyCredential`
- [ ] struct `suprnova::magnetar_integration::passkey::RequestChallengeResponse` re-exports `webauthn_rs_proto::auth::RequestChallengeResponse`
- [ ] proc attribute `suprnova::main` re-exports `suprnova_macros::main`
- [ ] proc attribute `suprnova::model` re-exports `suprnova_macros::model`
- [ ] proc attribute `suprnova::mutator` re-exports `suprnova_macros::mutator`
- [ ] proc attribute `suprnova::observer` re-exports `suprnova_macros::observer`
- [ ] module `suprnova::opendal` re-exports `opendal` (feature: `filesystem`)
- [ ] enum `suprnova::payments::Currency` re-exports `iso_currency::Currency`
- [ ] enum `suprnova::payments::money::Currency` re-exports `iso_currency::Currency`
- [ ] proc attribute `suprnova::policy` re-exports `suprnova_macros::policy`
- [ ] proc attribute `suprnova::prelude::async_trait` re-exports `async_trait::async_trait`
- [ ] proc attribute `suprnova::prunable` re-exports `suprnova_macros::prunable`
- [ ] macro `suprnova::redirect` re-exports `suprnova_macros::redirect`
- [ ] enum `suprnova::render_cache::CoherenceMode` re-exports `suprnova_live::render_cache::policy::CoherenceMode`
- [ ] enum `suprnova::render_cache::DeclineReason` re-exports `suprnova_live::render_cache::policy::DeclineReason`
- [ ] enum `suprnova::render_cache::DependencyIdentity` re-exports `suprnova_live::render_cache::generation::DependencyIdentity`
- [ ] enum `suprnova::render_cache::Eligibility` re-exports `suprnova_live::render_cache::policy::Eligibility`
- [ ] struct `suprnova::render_cache::EntryInspection` re-exports `suprnova_live::render_cache::entry::EntryInspection`
- [ ] enum `suprnova::render_cache::EntryKind` re-exports `suprnova_live::render_cache::entry::EntryKind`
- [ ] enum `suprnova::render_cache::FailurePolicy` re-exports `suprnova_live::render_cache::policy::FailurePolicy`
- [ ] struct `suprnova::render_cache::FreshnessPolicy` re-exports `suprnova_live::render_cache::policy::FreshnessPolicy`
- [ ] struct `suprnova::render_cache::NegotiatedPolicy` re-exports `suprnova_live::render_cache::policy::NegotiatedPolicy`
- [ ] struct `suprnova::render_cache::PolicyPatch` re-exports `suprnova_live::render_cache::policy::PolicyPatch`
- [ ] struct `suprnova::render_cache::QueryPolicy` re-exports `suprnova_live::render_cache::policy::QueryPolicy`
- [ ] struct `suprnova::render_cache::RenderCachePolicy` re-exports `suprnova_live::render_cache::policy::RenderCachePolicy`
- [ ] struct `suprnova::render_cache::RenderCachePolicyBuilder` re-exports `suprnova_live::render_cache::policy::RenderCachePolicyBuilder`
- [ ] enum `suprnova::render_cache::RepresentationClass` re-exports `suprnova_live::render_cache::policy::RepresentationClass`
- [ ] enum `suprnova::render_cache::SharedCachePolicy` re-exports `suprnova_live::render_cache::policy::SharedCachePolicy`
- [ ] struct `suprnova::render_cache::StorageLayers` re-exports `suprnova_live::render_cache::policy::StorageLayers`
- [ ] enum `suprnova::render_cache::VarianceDimension` re-exports `suprnova_live::render_cache::variance::VarianceDimension`
- [ ] enum `suprnova::render_cache::config::FailurePolicy` re-exports `suprnova_live::render_cache::policy::FailurePolicy`
- [ ] proc attribute `suprnova::request` re-exports `suprnova_macros::request`
- [ ] proc attribute `suprnova::scopes` re-exports `suprnova_macros::scopes`
- [ ] module `suprnova::sea_orm` re-exports `sea_orm`
- [ ] module `suprnova::sea_query` re-exports `sea_query`
- [ ] module `suprnova::serde` re-exports `serde`
- [ ] proc attribute `suprnova::service` re-exports `suprnova_macros::service`
- [ ] proc attribute `suprnova::suprnova_test` re-exports `suprnova_macros::suprnova_test`
- [ ] struct `suprnova::telemetry::propagation::HeaderExtractor` re-exports `opentelemetry_http::HeaderExtractor` (feature: `otel`, off by default)
- [ ] struct `suprnova::telemetry::propagation::HeaderInjector` re-exports `opentelemetry_http::HeaderInjector` (feature: `otel`, off by default)
- [ ] macro `suprnova::test` re-exports `suprnova_macros::test`
- [ ] module `suprnova::tokio` re-exports `tokio`
- [ ] module `suprnova::validator` re-exports `validator`
- [ ] proc attribute `suprnova::view` re-exports `suprnova_macros::view`
- [ ] struct `suprnova::view::AssetSet` re-exports `suprnova_live::view::contract::AssetSet`
- [ ] struct `suprnova::view::CanonicalDocumentConformance` re-exports `suprnova_live::view::document::CanonicalDocumentConformance`
- [ ] struct `suprnova::view::CanonicalDocumentIntent` re-exports `suprnova_live::view::document::CanonicalDocumentIntent`
- [ ] struct `suprnova::view::CanonicalDocumentRequest` re-exports `suprnova_live::view::document::CanonicalDocumentRequest`
- [ ] struct `suprnova::view::ChildMount` re-exports `suprnova_live::view::contract::ChildMount`
- [ ] enum `suprnova::view::DocumentCachePolicy` re-exports `suprnova_live::view::document::DocumentCachePolicy`
- [ ] enum `suprnova::view::DocumentMediaType` re-exports `suprnova_live::view::document::DocumentMediaType`
- [ ] struct `suprnova::view::DocumentRender` re-exports `suprnova_live::view::document::DocumentRender`
- [ ] struct `suprnova::view::DocumentResponseIntent` re-exports `suprnova_live::view::document::DocumentResponseIntent`
- [ ] struct `suprnova::view::DocumentValidator` re-exports `suprnova_live::view::document::DocumentValidator`
- [ ] trait `suprnova::view::FilterValues` re-exports `askama::values::Values`
- [ ] struct `suprnova::view::HeaderName` re-exports `http::header::name::HeaderName`
- [ ] struct `suprnova::view::HeaderValue` re-exports `http::header::value::HeaderValue`
- [ ] struct `suprnova::view::IslandRender` re-exports `suprnova_live::view::island::IslandRender`
- [ ] struct `suprnova::view::MountMetadata` re-exports `suprnova_live::view::contract::MountMetadata`
- [ ] enum `suprnova::view::MountSnapshotKind` re-exports `suprnova_live::view::contract::MountSnapshotKind`
- [ ] struct `suprnova::view::RegisteredSanitizer` re-exports `suprnova_live::view::trusted_html::RegisteredSanitizer`
- [ ] struct `suprnova::view::RenderLimits` re-exports `suprnova_live::view::contract::RenderLimits`
- [ ] struct `suprnova::view::SanitizerFailure` re-exports `suprnova_live::view::trusted_html::SanitizerFailure`
- [ ] struct `suprnova::view::SanitizerId` re-exports `suprnova_live::view::trusted_html::SanitizerId`
- [ ] struct `suprnova::view::StatusCode` re-exports `http::status::StatusCode`
- [ ] struct `suprnova::view::TrustedHtml` re-exports `suprnova_live::view::trusted_html::TrustedHtml`
- [ ] struct `suprnova::view::TrustedMarkupError` re-exports `suprnova_live::view::trusted_html::TrustedMarkupError`
- [ ] enum `suprnova::view::TrustedMarkupErrorKind` re-exports `suprnova_live::view::trusted_html::TrustedMarkupErrorKind`
- [ ] struct `suprnova::view::TrustedMarkupReason` re-exports `suprnova_live::view::trusted_html::TrustedMarkupReason`
- [ ] struct `suprnova::view::ViewError` re-exports `suprnova_live::view::error::ViewError`
- [ ] enum `suprnova::view::ViewErrorKind` re-exports `suprnova_live::view::error::ViewErrorKind`
- [ ] struct `suprnova::view::ViewName` re-exports `suprnova_live::identity::ViewName`
- [ ] struct `suprnova::view::filters::live_key` re-exports `suprnova_live::view::live_key::filters::live_key`
- [ ] struct `suprnova::view::filters::live_key_digest` re-exports `suprnova_live::view::live_key::filters::live_key_digest`
- [ ] struct `suprnova::view::filters::trusted_html` re-exports `suprnova_live::view::trusted_html::filters::trusted_html`
- [ ] proc attribute `suprnova::view_filter` re-exports `suprnova_macros::view_filter`
- [ ] enum `suprnova::web_push::ContentEncoding` re-exports `suprnova_web_push::payload::ContentEncoding` (feature: `web-push`)
- [ ] enum `suprnova::web_push::EndpointPolicy` re-exports `suprnova_web_push::client::EndpointPolicy` (feature: `web-push`)
- [ ] struct `suprnova::web_push::PushResponse` re-exports `suprnova_web_push::client::PushResponse` (feature: `web-push`)
- [ ] struct `suprnova::web_push::SubscriptionInfo` re-exports `suprnova_web_push::client::SubscriptionInfo` (feature: `web-push`)
- [ ] struct `suprnova::web_push::VapidClaims` re-exports `suprnova_web_push::vapid::VapidClaims` (feature: `web-push`)
- [ ] struct `suprnova::web_push::VapidKey` re-exports `suprnova_web_push::vapid::VapidKey` (feature: `web-push`)
- [ ] struct `suprnova::web_push::VapidSigner` re-exports `suprnova_web_push::vapid::VapidSigner` (feature: `web-push`)
- [ ] struct `suprnova::web_push::WebPushClient` re-exports `suprnova_web_push::client::WebPushClient` (feature: `web-push`)
- [ ] enum `suprnova::web_push::WebPushError` re-exports `suprnova_web_push::error::WebPushError` (feature: `web-push`)
- [ ] proc attribute `suprnova::workflow` re-exports `suprnova_macros::workflow`
- [ ] proc attribute `suprnova::workflow_step` re-exports `suprnova_macros::workflow_step`

## (crate root)

### `suprnova`

- [ ] macro `suprnova::__` · framework/src/localization/mod.rs:500
- [ ] macro `suprnova::any` · framework/src/routing/macros.rs:560
- [ ] macro `suprnova::attrs` · framework/src/eloquent/attrs.rs:119
- [ ] macro `suprnova::bind` · framework/src/container/mod.rs:1075
- [ ] macro `suprnova::bind_factory` · framework/src/container/mod.rs:1095
- [ ] macro `suprnova::casts` · framework/src/eloquent/casts/mod.rs:117
- [ ] macro `suprnova::delete` · framework/src/routing/macros.rs:394
- [ ] macro `suprnova::expect` · framework/src/lib.rs:751
- [ ] macro `suprnova::factory` · framework/src/container/mod.rs:1135
- [ ] macro `suprnova::fallback` · framework/src/routing/macros.rs:897
- [ ] macro `suprnova::get` · framework/src/routing/macros.rs:295
- [ ] macro `suprnova::global_middleware` · framework/src/lib.rs:718
- [ ] macro `suprnova::group` · framework/src/routing/macros.rs:1365
- [ ] macro `suprnova::head` · framework/src/routing/macros.rs:470
- [ ] macro `suprnova::json_response` · framework/src/lib.rs:668
- [ ] macro `suprnova::options` · framework/src/routing/macros.rs:509
- [ ] macro `suprnova::patch` · framework/src/routing/macros.rs:431
- [ ] macro `suprnova::post` · framework/src/routing/macros.rs:328
- [ ] macro `suprnova::put` · framework/src/routing/macros.rs:361
- [ ] macro `suprnova::route_binding` · framework/src/database/route_binding.rs:292
- [ ] macro `suprnova::routes` · framework/src/routing/macros.rs:1420
- [ ] macro `suprnova::schedule_task` · framework/src/schedule/mod.rs:591
- [ ] macro `suprnova::singleton` · framework/src/container/mod.rs:1117
- [ ] macro `suprnova::start_workflow` · framework/src/workflow/mod.rs:539
- [ ] macro `suprnova::test_database` · framework/src/database/testing.rs:244
- [ ] macro `suprnova::text_response` · framework/src/lib.rs:679
- [ ] macro `suprnova::validate` · framework/src/validation/rule.rs:2226
- [ ] macro `suprnova::when_loaded` · framework/src/data/when_loaded.rs:75
- [ ] macro `suprnova::ws` · framework/src/routing/macros.rs:672
- [ ] const `suprnova::VERSION` · framework/src/lib.rs:123

## app

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

### `suprnova::app::maintenance`

- [ ] fn `suprnova::maintenance_mode` · framework/src/app/maintenance.rs:312 (also `suprnova::app::maintenance::maintenance_mode`)
- [ ] struct `suprnova::CacheMaintenanceMode` · framework/src/app/maintenance.rs:263 (also `suprnova::app::maintenance::CacheMaintenanceMode`)
  - Implements: `suprnova::MaintenanceMode`
  - [ ] fn `suprnova::CacheMaintenanceMode::new` · framework/src/app/maintenance.rs:269
  - [ ] fn `suprnova::CacheMaintenanceMode::with_key` · framework/src/app/maintenance.rs:276
- [ ] struct `suprnova::FileMaintenanceMode` · framework/src/app/maintenance.rs:177 (also `suprnova::app::maintenance::FileMaintenanceMode`)
  - Implements: `suprnova::MaintenanceMode`
  - [ ] fn `suprnova::FileMaintenanceMode::new` · framework/src/app/maintenance.rs:183
  - [ ] fn `suprnova::FileMaintenanceMode::with_path` · framework/src/app/maintenance.rs:190
- [ ] struct `suprnova::MaintenanceMiddleware` · framework/src/app/maintenance.rs:395 (also `suprnova::app::maintenance::MaintenanceMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::MaintenanceMiddleware::new` · framework/src/app/maintenance.rs:402
  - [ ] fn `suprnova::MaintenanceMiddleware::with_driver` · framework/src/app/maintenance.rs:410
  - [ ] fn `suprnova::MaintenanceMiddleware::except` · framework/src/app/maintenance.rs:419
- [ ] struct `suprnova::MaintenancePayload` · framework/src/app/maintenance.rs:72 (also `suprnova::app::maintenance::MaintenancePayload`)
  - Public fields: `except`, `redirect`, `retry`, `refresh`, `secret`, `status`, `template`
  - [ ] fn `suprnova::MaintenancePayload::new` · framework/src/app/maintenance.rs:132
- [ ] trait `suprnova::MaintenanceMode` · framework/src/app/maintenance.rs:164 (also `suprnova::app::maintenance::MaintenanceMode`)
  - Implemented here by: `CacheMaintenanceMode`, `FileMaintenanceMode`
  - [ ] fn `suprnova::MaintenanceMode::activate` · framework/src/app/maintenance.rs:166 (required)
  - [ ] fn `suprnova::MaintenanceMode::deactivate` · framework/src/app/maintenance.rs:168 (required)
  - [ ] fn `suprnova::MaintenanceMode::active` · framework/src/app/maintenance.rs:170 (required)
  - [ ] fn `suprnova::MaintenanceMode::data` · framework/src/app/maintenance.rs:172 (required)

### `suprnova::app::paths`

- [ ] fn `suprnova::base_path` · framework/src/app/paths.rs:91 (also `suprnova::app::paths::base_path`)
- [ ] fn `suprnova::config_path` · framework/src/app/paths.rs:96 (also `suprnova::app::paths::config_path`)
- [ ] fn `suprnova::database_path` · framework/src/app/paths.rs:102 (also `suprnova::app::paths::database_path`)
- [ ] fn `suprnova::lang_path` · framework/src/app/paths.rs:128 (also `suprnova::app::paths::lang_path`)
- [ ] fn `suprnova::public_path` · framework/src/app/paths.rs:108 (also `suprnova::app::paths::public_path`)
- [ ] fn `suprnova::resource_path` · framework/src/app/paths.rs:120 (also `suprnova::app::paths::resource_path`)
- [ ] fn `suprnova::set_base_path` · framework/src/app/paths.rs:135 (also `suprnova::app::paths::set_base_path`)
- [ ] fn `suprnova::storage_path` · framework/src/app/paths.rs:114 (also `suprnova::app::paths::storage_path`)
- [ ] fn `suprnova::use_config_path` · framework/src/app/paths.rs:140 (also `suprnova::app::paths::use_config_path`)
- [ ] fn `suprnova::use_database_path` · framework/src/app/paths.rs:145 (also `suprnova::app::paths::use_database_path`)
- [ ] fn `suprnova::use_lang_path` · framework/src/app/paths.rs:165 (also `suprnova::app::paths::use_lang_path`)
- [ ] fn `suprnova::use_public_path` · framework/src/app/paths.rs:150 (also `suprnova::app::paths::use_public_path`)
- [ ] fn `suprnova::use_resource_path` · framework/src/app/paths.rs:160 (also `suprnova::app::paths::use_resource_path`)
- [ ] fn `suprnova::use_storage_path` · framework/src/app/paths.rs:155 (also `suprnova::app::paths::use_storage_path`)

## auth

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

### `suprnova::auth::remember`

- [ ] fn `suprnova::auth::remember::issue` · framework/src/auth/remember.rs:155 (also `suprnova::auth_flows::remember_me::issue`)
- [ ] fn `suprnova::auth::remember::prune_expired` · framework/src/auth/remember.rs:365 (also `suprnova::auth_flows::remember_me::prune_expired`)
- [ ] fn `suprnova::auth::remember::revoke_all_for_user` · framework/src/auth/remember.rs:268 (also `suprnova::auth_flows::remember_me::revoke_all_for_user`)
- [ ] fn `suprnova::auth::remember::revoke_by_id` · framework/src/auth/remember.rs:351 (also `suprnova::auth_flows::remember_me::revoke_by_id`)
- [ ] fn `suprnova::auth::remember::verify_and_rotate` · framework/src/auth/remember.rs:198 (also `suprnova::auth_flows::remember_me::verify_and_rotate`)

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

## auth_flows

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

### `suprnova::auth_flows::two_factor::migration`

- [ ] struct `suprnova::auth_flows::two_factor::migration::Migration` · framework/src/auth_flows/two_factor/migration.rs:11

### `suprnova::auth_flows::two_factor::migration_replay`

- [ ] struct `suprnova::auth_flows::two_factor::migration_replay::Migration` · framework/src/auth_flows/two_factor/migration_replay.rs:15

### `suprnova::auth_flows::two_factor::recovery`

- [ ] fn `suprnova::auth_flows::two_factor::recovery::consume` · framework/src/auth_flows/two_factor/recovery.rs:71
- [ ] fn `suprnova::auth_flows::two_factor::recovery::generate` · framework/src/auth_flows/two_factor/recovery.rs:55

### `suprnova::auth_flows::two_factor_challenge_middleware`

- [ ] struct `suprnova::TwoFactorChallengeMiddleware` · framework/src/auth_flows/two_factor_challenge_middleware.rs:75 (also `suprnova::auth_flows::TwoFactorChallengeMiddleware`, `suprnova::auth_flows::two_factor_challenge_middleware::TwoFactorChallengeMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::TwoFactorChallengeMiddleware::new` · framework/src/auth_flows/two_factor_challenge_middleware.rs:87
  - [ ] fn `suprnova::TwoFactorChallengeMiddleware::redirect_to` · framework/src/auth_flows/two_factor_challenge_middleware.rs:96

## authorization

### `suprnova::authorization`

- [ ] fn `suprnova::authorization::init_policies` · framework/src/authorization/mod.rs:111
- [ ] trait `suprnova::Authorizable` · framework/src/authorization/mod.rs:26 (also `suprnova::authorization::Authorizable`)
  - [ ] fn `suprnova::Authorizable::can` · framework/src/authorization/mod.rs:29 (provided)
  - [ ] fn `suprnova::Authorizable::cannot` · framework/src/authorization/mod.rs:33 (provided)
  - [ ] fn `suprnova::Authorizable::authorize` · framework/src/authorization/mod.rs:43 (provided)
  - [ ] fn `suprnova::Authorizable::can_async` · framework/src/authorization/mod.rs:51 (provided)
  - [ ] fn `suprnova::Authorizable::cannot_async` · framework/src/authorization/mod.rs:63 (provided)
  - [ ] fn `suprnova::Authorizable::authorize_async` · framework/src/authorization/mod.rs:78 (provided)

### `suprnova::authorization::gate` (private module; items are public through re-exports)

- [ ] struct `suprnova::Gate` · framework/src/authorization/gate.rs:30 (also `suprnova::authorization::Gate`)
  - [ ] fn `suprnova::Gate::define` · framework/src/authorization/gate.rs:36
  - [ ] fn `suprnova::Gate::define_with` · framework/src/authorization/gate.rs:62
  - [ ] fn `suprnova::Gate::allows` · framework/src/authorization/gate.rs:76
  - [ ] fn `suprnova::Gate::denies` · framework/src/authorization/gate.rs:81
  - [ ] fn `suprnova::Gate::authorize` · framework/src/authorization/gate.rs:92
  - [ ] fn `suprnova::Gate::define_async` · framework/src/authorization/gate.rs:116
  - [ ] fn `suprnova::Gate::define_async_with` · framework/src/authorization/gate.rs:128
  - [ ] fn `suprnova::Gate::allows_async` · framework/src/authorization/gate.rs:139
  - [ ] fn `suprnova::Gate::denies_async` · framework/src/authorization/gate.rs:148
  - [ ] fn `suprnova::Gate::authorize_async` · framework/src/authorization/gate.rs:157
  - [ ] fn `suprnova::Gate::inspect` · framework/src/authorization/gate.rs:180
  - [ ] fn `suprnova::Gate::inspect_async` · framework/src/authorization/gate.rs:190
  - [ ] fn `suprnova::Gate::raw` · framework/src/authorization/gate.rs:211
  - [ ] fn `suprnova::Gate::raw_async` · framework/src/authorization/gate.rs:219
  - [ ] fn `suprnova::Gate::before` · framework/src/authorization/gate.rs:249
  - [ ] fn `suprnova::Gate::after` · framework/src/authorization/gate.rs:270
  - [ ] fn `suprnova::Gate::default_denial_response` · framework/src/authorization/gate.rs:310
  - [ ] fn `suprnova::Gate::has` · framework/src/authorization/gate.rs:334
  - [ ] fn `suprnova::Gate::abilities` · framework/src/authorization/gate.rs:342
  - [ ] fn `suprnova::Gate::any` · framework/src/authorization/gate.rs:355
  - [ ] fn `suprnova::Gate::none` · framework/src/authorization/gate.rs:363
  - [ ] fn `suprnova::Gate::check` · framework/src/authorization/gate.rs:374
  - [ ] fn `suprnova::Gate::any_async` · framework/src/authorization/gate.rs:385
  - [ ] fn `suprnova::Gate::none_async` · framework/src/authorization/gate.rs:399
  - [ ] fn `suprnova::Gate::check_async` · framework/src/authorization/gate.rs:408

### `suprnova::authorization::response` (private module; items are public through re-exports)

- [ ] struct `suprnova::GateResponse` · framework/src/authorization/response.rs:50 (also `suprnova::authorization::Response`)
  - [ ] fn `suprnova::GateResponse::allow` · framework/src/authorization/response.rs:64
  - [ ] fn `suprnova::GateResponse::deny` · framework/src/authorization/response.rs:76
  - [ ] fn `suprnova::GateResponse::deny_with` · framework/src/authorization/response.rs:87
  - [ ] fn `suprnova::GateResponse::deny_with_status` · framework/src/authorization/response.rs:98
  - [ ] fn `suprnova::GateResponse::deny_as_not_found` · framework/src/authorization/response.rs:110
  - [ ] fn `suprnova::GateResponse::with_message` · framework/src/authorization/response.rs:122
  - [ ] fn `suprnova::GateResponse::with_code` · framework/src/authorization/response.rs:132
  - [ ] fn `suprnova::GateResponse::with_status` · framework/src/authorization/response.rs:138
  - [ ] fn `suprnova::GateResponse::as_not_found` · framework/src/authorization/response.rs:144
  - [ ] fn `suprnova::GateResponse::allowed` · framework/src/authorization/response.rs:152
  - [ ] fn `suprnova::GateResponse::denied` · framework/src/authorization/response.rs:157
  - [ ] fn `suprnova::GateResponse::message` · framework/src/authorization/response.rs:162
  - [ ] fn `suprnova::GateResponse::code` · framework/src/authorization/response.rs:170
  - [ ] fn `suprnova::GateResponse::status` · framework/src/authorization/response.rs:175
  - [ ] fn `suprnova::GateResponse::authorize` · framework/src/authorization/response.rs:191

## boot

### `suprnova::boot`

- [ ] fn `suprnova::boot::default_build_id` · framework/src/boot.rs:50
- [ ] fn `suprnova::boot::env_loaded_pre_runtime` · framework/src/boot.rs:118
- [ ] fn `suprnova::boot::initialize_crypt_or_exit` · framework/src/boot.rs:107
- [ ] fn `suprnova::boot::load_env` · framework/src/boot.rs:67
- [ ] fn `suprnova::boot::load_env_or_exit` · framework/src/boot.rs:88
- [ ] fn `suprnova::boot::set_default_build_id` · framework/src/boot.rs:40

## broadcasting

### `suprnova::broadcasting::broadcastable` (private module; items are public through re-exports)

- [ ] struct `suprnova::BroadcastListener` · framework/src/broadcasting/broadcastable.rs:93 (also `suprnova::broadcasting::BroadcastListener`)
  - Implements: `suprnova::Listener`
  - [ ] fn `suprnova::BroadcastListener::new` · framework/src/broadcasting/broadcastable.rs:100
- [ ] trait `suprnova::Broadcastable` · framework/src/broadcasting/broadcastable.rs:48 (also `suprnova::broadcasting::Broadcastable`)
  - [ ] fn `suprnova::Broadcastable::broadcast_on` · framework/src/broadcasting/broadcastable.rs:51 (required)
  - [ ] fn `suprnova::Broadcastable::broadcast_event_name` · framework/src/broadcasting/broadcastable.rs:55 (provided)
  - [ ] fn `suprnova::Broadcastable::broadcast_with` · framework/src/broadcasting/broadcastable.rs:64 (provided)
  - [ ] fn `suprnova::Broadcastable::broadcast_when` · framework/src/broadcasting/broadcastable.rs:72 (provided)
  - [ ] fn `suprnova::Broadcastable::broadcast_to_others` · framework/src/broadcasting/broadcastable.rs:86 (provided)

### `suprnova::broadcasting::channel` (private module; items are public through re-exports)

- [ ] struct `suprnova::broadcasting::ChannelParams` · framework/src/broadcasting/channel.rs:26
  - [ ] fn `suprnova::broadcasting::ChannelParams::get` · framework/src/broadcasting/channel.rs:33
  - [ ] fn `suprnova::broadcasting::ChannelParams::is_empty` · framework/src/broadcasting/channel.rs:41
  - [ ] fn `suprnova::broadcasting::ChannelParams::len` · framework/src/broadcasting/channel.rs:46
  - [ ] fn `suprnova::broadcasting::ChannelParams::iter` · framework/src/broadcasting/channel.rs:51
- [ ] struct `suprnova::broadcasting::ChannelRegistry` · framework/src/broadcasting/channel.rs:307
  - [ ] fn `suprnova::broadcasting::ChannelRegistry::new` · framework/src/broadcasting/channel.rs:313
  - [ ] fn `suprnova::broadcasting::ChannelRegistry::register` · framework/src/broadcasting/channel.rs:330
  - [ ] fn `suprnova::broadcasting::ChannelRegistry::resolve` · framework/src/broadcasting/channel.rs:350
  - [ ] fn `suprnova::broadcasting::ChannelRegistry::len` · framework/src/broadcasting/channel.rs:376
  - [ ] fn `suprnova::broadcasting::ChannelRegistry::is_empty` · framework/src/broadcasting/channel.rs:381
- [ ] trait `suprnova::broadcasting::Channel` · framework/src/broadcasting/channel.rs:127
  - [ ] fn `suprnova::broadcasting::Channel::name` · framework/src/broadcasting/channel.rs:137 (required)
  - [ ] fn `suprnova::broadcasting::Channel::authorize` · framework/src/broadcasting/channel.rs:147 (provided)
  - [ ] fn `suprnova::broadcasting::Channel::authorize_publish` · framework/src/broadcasting/channel.rs:168 (provided)
  - [ ] fn `suprnova::broadcasting::Channel::presence_info` · framework/src/broadcasting/channel.rs:207 (provided)
- [ ] trait `suprnova::broadcasting::PresenceChannel` · framework/src/broadcasting/channel.rs:265
  - [ ] fn `suprnova::broadcasting::PresenceChannel::member_info` · framework/src/broadcasting/channel.rs:272 (required)
- [ ] trait `suprnova::broadcasting::PrivateChannel` · framework/src/broadcasting/channel.rs:216
- [ ] type `suprnova::broadcasting::BoxedChannel` · framework/src/broadcasting/channel.rs:280

### `suprnova::broadcasting::fanout::sea_streamer` (private module; items are public through re-exports)

- [ ] struct `suprnova::broadcasting::fanout::SeaStreamerBroadcastHub` · framework/src/broadcasting/fanout/sea_streamer.rs:255 (feature: `broadcasting-fanout`, off by default)
  - Implements: `suprnova::BroadcastHub`
  - [ ] fn `suprnova::broadcasting::fanout::SeaStreamerBroadcastHub::new` · framework/src/broadcasting/fanout/sea_streamer.rs:294
  - [ ] fn `suprnova::broadcasting::fanout::SeaStreamerBroadcastHub::new_with_presence_ttl` · framework/src/broadcasting/fanout/sea_streamer.rs:316
  - [ ] fn `suprnova::broadcasting::fanout::SeaStreamerBroadcastHub::new_loopback` · framework/src/broadcasting/fanout/sea_streamer.rs:332
  - [ ] fn `suprnova::broadcasting::fanout::SeaStreamerBroadcastHub::new_loopback_with_presence_ttl` · framework/src/broadcasting/fanout/sea_streamer.rs:343

### `suprnova::broadcasting::handler` (private module; items are public through re-exports)

- [ ] struct `suprnova::BroadcastingWsHandler` · framework/src/broadcasting/handler.rs:106 (also `suprnova::broadcasting::BroadcastingWsHandler`)
  - Implements: `suprnova::WebSocketHandler`
  - [ ] fn `suprnova::BroadcastingWsHandler::new` · framework/src/broadcasting/handler.rs:122
  - [ ] fn `suprnova::BroadcastingWsHandler::with_max_subscriptions` · framework/src/broadcasting/handler.rs:142
- [ ] const `suprnova::broadcasting::DEFAULT_MAX_SUBSCRIPTIONS_PER_CONNECTION` · framework/src/broadcasting/handler.rs:99

### `suprnova::broadcasting::hub` (private module; items are public through re-exports)

- [ ] struct `suprnova::BroadcastEnvelope` · framework/src/broadcasting/hub.rs:60 (also `suprnova::broadcasting::BroadcastEnvelope`)
  - Public fields: `channel`, `event`, `data`, `except`
  - [ ] fn `suprnova::BroadcastEnvelope::new` · framework/src/broadcasting/hub.rs:79
  - [ ] fn `suprnova::BroadcastEnvelope::with_except` · framework/src/broadcasting/hub.rs:91
- [ ] struct `suprnova::InMemoryBroadcastHub` · framework/src/broadcasting/hub.rs:166 (also `suprnova::broadcasting::InMemoryBroadcastHub`)
  - Implements: `suprnova::BroadcastHub`
  - [ ] fn `suprnova::InMemoryBroadcastHub::new` · framework/src/broadcasting/hub.rs:174
- [ ] trait `suprnova::BroadcastHub` · framework/src/broadcasting/hub.rs:102 (also `suprnova::broadcasting::BroadcastHub`)
  - Implemented here by: `InMemoryBroadcastHub`, `broadcasting::RecordingBroadcastHub`, `broadcasting::fanout::SeaStreamerBroadcastHub`
  - [ ] fn `suprnova::BroadcastHub::subscribe` · framework/src/broadcasting/hub.rs:107 (required)
  - [ ] fn `suprnova::BroadcastHub::publish` · framework/src/broadcasting/hub.rs:120 (required)
  - [ ] fn `suprnova::BroadcastHub::subscriber_count` · framework/src/broadcasting/hub.rs:124 (provided)
  - [ ] fn `suprnova::BroadcastHub::track_member` · framework/src/broadcasting/hub.rs:140 (provided)
  - [ ] fn `suprnova::BroadcastHub::untrack_member` · framework/src/broadcasting/hub.rs:153 (provided)
  - [ ] fn `suprnova::BroadcastHub::list_members` · framework/src/broadcasting/hub.rs:160 (provided)

### `suprnova::broadcasting::protocol` (private module; items are public through re-exports)

- [ ] enum `suprnova::broadcasting::ClientFrame` · framework/src/broadcasting/protocol.rs:26
  - Variants: `Subscribe`, `Unsubscribe`, `Publish`
- [ ] enum `suprnova::broadcasting::ServerFrame` · framework/src/broadcasting/protocol.rs:60
  - Variants: `Connected`, `Subscribed`, `Unsubscribed`, `Event`, `Lagged`, `Error`

### `suprnova::broadcasting::testing` (private module; items are public through re-exports)

- [ ] struct `suprnova::broadcasting::RecordingBroadcastHub` · framework/src/broadcasting/testing.rs:28
  - Implements: `suprnova::BroadcastHub`
  - [ ] fn `suprnova::broadcasting::RecordingBroadcastHub::new` · framework/src/broadcasting/testing.rs:35
  - [ ] fn `suprnova::broadcasting::RecordingBroadcastHub::broadcasts` · framework/src/broadcasting/testing.rs:44
  - [ ] fn `suprnova::broadcasting::RecordingBroadcastHub::count` · framework/src/broadcasting/testing.rs:49
  - [ ] fn `suprnova::broadcasting::RecordingBroadcastHub::assert_broadcast` · framework/src/broadcasting/testing.rs:54
  - [ ] fn `suprnova::broadcasting::RecordingBroadcastHub::assert_nothing_broadcast` · framework/src/broadcasting/testing.rs:71

## bus

### `suprnova::bus`

- [ ] struct `suprnova::Bus` · framework/src/bus/mod.rs:83 (also `suprnova::bus::Bus`, `suprnova::prelude::Bus`)
  - [ ] fn `suprnova::Bus::register` · framework/src/bus/mod.rs:96
  - [ ] fn `suprnova::Bus::dispatch` · framework/src/bus/mod.rs:147
  - [ ] fn `suprnova::Bus::chain` · framework/src/bus/mod.rs:180
  - [ ] fn `suprnova::Bus::batch` · framework/src/bus/mod.rs:201
- [ ] enum `suprnova::Dispatched` · framework/src/bus/mod.rs:23 (also `suprnova::bus::Dispatched`)
  - Variants: `Executed`, `Captured`
  - [ ] fn `suprnova::Dispatched::unwrap_executed` · framework/src/bus/mod.rs:32
  - [ ] fn `suprnova::Dispatched::is_executed` · framework/src/bus/mod.rs:42
  - [ ] fn `suprnova::Dispatched::is_captured` · framework/src/bus/mod.rs:47
  - [ ] fn `suprnova::Dispatched::executed` · framework/src/bus/mod.rs:52

### `suprnova::bus::command`

- [ ] trait `suprnova::bus::command::Command` · framework/src/bus/command.rs:14
  - [ ] type `suprnova::bus::command::Command::Output` · framework/src/bus/command.rs:16
  - [ ] fn `suprnova::bus::command::Command::command_name` · framework/src/bus/command.rs:19 (required)
- [ ] trait `suprnova::bus::command::Handler` · framework/src/bus/command.rs:27
  - [ ] fn `suprnova::bus::command::Handler::handle` · framework/src/bus/command.rs:30 (required)

### `suprnova::bus::testing`

- [ ] fn `suprnova::bus::testing::assert_dispatched` · framework/src/bus/testing.rs:101
- [ ] fn `suprnova::bus::testing::assert_dispatched_times` · framework/src/bus/testing.rs:127
- [ ] fn `suprnova::bus::testing::assert_not_dispatched` · framework/src/bus/testing.rs:113
- [ ] fn `suprnova::bus::testing::assert_nothing_dispatched` · framework/src/bus/testing.rs:140
- [ ] fn `suprnova::bus::testing::install_fake` · framework/src/bus/testing.rs:59
- [ ] struct `suprnova::bus::testing::BusFakeGuard` · framework/src/bus/testing.rs:67

## cache

### `suprnova::cache`

- [ ] struct `suprnova::Cache` · framework/src/cache/mod.rs:85 (also `suprnova::cache::Cache`, `suprnova::prelude::Cache`)
  - [ ] fn `suprnova::Cache::store` · framework/src/cache/mod.rs:130
  - [ ] fn `suprnova::Cache::is_initialized` · framework/src/cache/mod.rs:135
  - [ ] fn `suprnova::Cache::get` · framework/src/cache/mod.rs:155
  - [ ] fn `suprnova::Cache::put` · framework/src/cache/mod.rs:185
  - [ ] fn `suprnova::Cache::forever` · framework/src/cache/mod.rs:212
  - [ ] fn `suprnova::Cache::has` · framework/src/cache/mod.rs:234
  - [ ] fn `suprnova::Cache::missing` · framework/src/cache/mod.rs:253
  - [ ] fn `suprnova::Cache::pull` · framework/src/cache/mod.rs:275
  - [ ] fn `suprnova::Cache::add` · framework/src/cache/mod.rs:314
  - [ ] fn `suprnova::Cache::sear` · framework/src/cache/mod.rs:343
  - [ ] fn `suprnova::Cache::forget` · framework/src/cache/mod.rs:364
  - [ ] fn `suprnova::Cache::flush` · framework/src/cache/mod.rs:379
  - [ ] fn `suprnova::Cache::increment` · framework/src/cache/mod.rs:397
  - [ ] fn `suprnova::Cache::decrement` · framework/src/cache/mod.rs:415
  - [ ] fn `suprnova::Cache::remember` · framework/src/cache/mod.rs:469
  - [ ] fn `suprnova::Cache::remember_forever` · framework/src/cache/mod.rs:498
  - [ ] fn `suprnova::Cache::tags_put` · framework/src/cache/mod.rs:523
  - [ ] fn `suprnova::Cache::flush_tags` · framework/src/cache/mod.rs:545
  - [ ] fn `suprnova::Cache::lock` · framework/src/cache/mod.rs:572
  - [ ] fn `suprnova::Cache::touch` · framework/src/cache/mod.rs:598
- [ ] struct `suprnova::LockGuard` · framework/src/cache/mod.rs:608 (also `suprnova::cache::LockGuard`)
  - [ ] fn `suprnova::LockGuard::token` · framework/src/cache/mod.rs:616
  - [ ] fn `suprnova::LockGuard::owner` · framework/src/cache/mod.rs:623
  - [ ] fn `suprnova::LockGuard::release` · framework/src/cache/mod.rs:629
  - [ ] fn `suprnova::LockGuard::refresh` · framework/src/cache/mod.rs:635

### `suprnova::cache::config`

- [ ] struct `suprnova::CacheConfig` · framework/src/cache/config.rs:69 (also `suprnova::cache::CacheConfig`, `suprnova::cache::config::CacheConfig`)
  - Public fields: `driver`, `url`, `prefix`, `default_ttl`
  - [ ] fn `suprnova::CacheConfig::from_env` · framework/src/cache/config.rs:89
  - [ ] fn `suprnova::CacheConfig::builder` · framework/src/cache/config.rs:103
- [ ] struct `suprnova::cache::CacheConfigBuilder` · framework/src/cache/config.rs:125 (also `suprnova::cache::config::CacheConfigBuilder`)
  - [ ] fn `suprnova::cache::CacheConfigBuilder::driver` · framework/src/cache/config.rs:134
  - [ ] fn `suprnova::cache::CacheConfigBuilder::url` · framework/src/cache/config.rs:140
  - [ ] fn `suprnova::cache::CacheConfigBuilder::prefix` · framework/src/cache/config.rs:146
  - [ ] fn `suprnova::cache::CacheConfigBuilder::default_ttl` · framework/src/cache/config.rs:152
  - [ ] fn `suprnova::cache::CacheConfigBuilder::build` · framework/src/cache/config.rs:160
- [ ] enum `suprnova::cache::CacheDriver` · framework/src/cache/config.rs:16 (also `suprnova::cache::config::CacheDriver`)
  - Variants: `Memory`, `Redis`
  - [ ] fn `suprnova::cache::CacheDriver::parse` · framework/src/cache/config.rs:29

### `suprnova::cache::memory`

- [ ] struct `suprnova::InMemoryCache` · framework/src/cache/memory.rs:103 (also `suprnova::cache::InMemoryCache`, `suprnova::cache::memory::InMemoryCache`)
  - Implements: `suprnova::CacheStore`
  - [ ] fn `suprnova::InMemoryCache::new` · framework/src/cache/memory.rs:119
  - [ ] fn `suprnova::InMemoryCache::with_prefix` · framework/src/cache/memory.rs:129
  - [ ] fn `suprnova::InMemoryCache::with_config` · framework/src/cache/memory.rs:141
  - [ ] fn `suprnova::InMemoryCache::purge_expired` · framework/src/cache/memory.rs:228

### `suprnova::cache::redis`

- [ ] struct `suprnova::RedisCache` · framework/src/cache/redis.rs:118 (also `suprnova::cache::RedisCache`, `suprnova::cache::redis::RedisCache`)
  - Implements: `suprnova::CacheStore`
  - [ ] fn `suprnova::RedisCache::connect` · framework/src/cache/redis.rs:126

### `suprnova::cache::store`

- [ ] trait `suprnova::CacheStore` · framework/src/cache/store.rs:24 (also `suprnova::cache::CacheStore`, `suprnova::cache::store::CacheStore`)
  - Implemented here by: `InMemoryCache`, `RedisCache`
  - [ ] fn `suprnova::CacheStore::get_raw` · framework/src/cache/store.rs:26 (required)
  - [ ] fn `suprnova::CacheStore::put_raw` · framework/src/cache/store.rs:32 (required)
  - [ ] fn `suprnova::CacheStore::add_raw` · framework/src/cache/store.rs:51 (provided)
  - [ ] fn `suprnova::CacheStore::default_ttl` · framework/src/cache/store.rs:68 (provided)
  - [ ] fn `suprnova::CacheStore::has` · framework/src/cache/store.rs:73 (required)
  - [ ] fn `suprnova::CacheStore::forget` · framework/src/cache/store.rs:76 (required)
  - [ ] fn `suprnova::CacheStore::flush` · framework/src/cache/store.rs:79 (required)
  - [ ] fn `suprnova::CacheStore::increment` · framework/src/cache/store.rs:84 (required)
  - [ ] fn `suprnova::CacheStore::decrement` · framework/src/cache/store.rs:89 (required)
  - [ ] fn `suprnova::CacheStore::tagged_put_raw` · framework/src/cache/store.rs:96 (required)
  - [ ] fn `suprnova::CacheStore::flush_tags` · framework/src/cache/store.rs:118 (required)
  - [ ] fn `suprnova::CacheStore::acquire_lock` · framework/src/cache/store.rs:123 (required)
  - [ ] fn `suprnova::CacheStore::release_lock` · framework/src/cache/store.rs:132 (required)
  - [ ] fn `suprnova::CacheStore::refresh_lock` · framework/src/cache/store.rs:136 (required)
  - [ ] fn `suprnova::CacheStore::touch` · framework/src/cache/store.rs:146 (required)

## config

### `suprnova::config`

- [ ] struct `suprnova::Config` · framework/src/config/mod.rs:42 (also `suprnova::config::Config`)
  - [ ] fn `suprnova::Config::init` · framework/src/config/mod.rs:75
  - [ ] fn `suprnova::Config::get` · framework/src/config/mod.rs:120
  - [ ] fn `suprnova::Config::register` · framework/src/config/mod.rs:145
  - [ ] fn `suprnova::Config::has` · framework/src/config/mod.rs:150
  - [ ] fn `suprnova::Config::environment` · framework/src/config/mod.rs:158
  - [ ] fn `suprnova::Config::is_production` · framework/src/config/mod.rs:165
  - [ ] fn `suprnova::Config::is_development` · framework/src/config/mod.rs:170
  - [ ] fn `suprnova::Config::is_debug` · framework/src/config/mod.rs:185
  - [ ] fn `suprnova::Config::resolve` · framework/src/config/mod.rs:211
  - [ ] fn `suprnova::Config::resolve_prefixed` · framework/src/config/mod.rs:219

### `suprnova::config::env`

- [ ] fn `suprnova::env` · framework/src/config/env.rs:297 (also `suprnova::config::env`, `suprnova::config::env::env`, `suprnova::env::env`)
- [ ] fn `suprnova::env_optional` · framework/src/config/env.rs:396 (also `suprnova::config::env::env_optional`, `suprnova::config::env_optional`, `suprnova::env::env_optional`)
- [ ] fn `suprnova::env_required` · framework/src/config/env.rs:332 (also `suprnova::config::env::env_required`, `suprnova::config::env_required`, `suprnova::env::env_required`)
- [ ] fn `suprnova::config::load_dotenv` · framework/src/config/env.rs:160 (also `suprnova::config::env::load_dotenv`, `suprnova::env::load_dotenv`)
- [ ] fn `suprnova::try_env_required` · framework/src/config/env.rs:369 (also `suprnova::config::env::try_env_required`, `suprnova::config::try_env_required`, `suprnova::env::try_env_required`)
- [ ] enum `suprnova::Environment` · framework/src/config/env.rs:32 (also `suprnova::config::Environment`, `suprnova::config::env::Environment`, `suprnova::env::Environment`)
  - Variants: `Local`, `Development`, `Staging`, `Production`, `Testing`, `Custom`
  - [ ] fn `suprnova::Environment::detect` · framework/src/config/env.rs:71
  - [ ] fn `suprnova::Environment::env_file_suffix` · framework/src/config/env.rs:87
  - [ ] fn `suprnova::Environment::is_production` · framework/src/config/env.rs:99
  - [ ] fn `suprnova::Environment::is_development` · framework/src/config/env.rs:104

### `suprnova::config::providers::app` (private module; items are public through re-exports)

- [ ] struct `suprnova::AppConfig` · framework/src/config/providers/app.rs:8 (also `suprnova::config::AppConfig`, `suprnova::config::providers::AppConfig`)
  - Public fields: `name`, `environment`, `debug`, `url`, `trusted_proxies`
  - [ ] fn `suprnova::AppConfig::from_env` · framework/src/config/providers/app.rs:49
  - [ ] fn `suprnova::AppConfig::try_from_env` · framework/src/config/providers/app.rs:80
  - [ ] fn `suprnova::AppConfig::builder` · framework/src/config/providers/app.rs:99
  - [ ] fn `suprnova::AppConfig::is_debug` · framework/src/config/providers/app.rs:104
  - [ ] fn `suprnova::AppConfig::is_production` · framework/src/config/providers/app.rs:109
  - [ ] fn `suprnova::AppConfig::is_development` · framework/src/config/providers/app.rs:114
- [ ] struct `suprnova::AppConfigBuilder` · framework/src/config/providers/app.rs:187 (also `suprnova::config::AppConfigBuilder`, `suprnova::config::providers::AppConfigBuilder`)
  - [ ] fn `suprnova::AppConfigBuilder::name` · framework/src/config/providers/app.rs:197
  - [ ] fn `suprnova::AppConfigBuilder::environment` · framework/src/config/providers/app.rs:203
  - [ ] fn `suprnova::AppConfigBuilder::debug` · framework/src/config/providers/app.rs:209
  - [ ] fn `suprnova::AppConfigBuilder::url` · framework/src/config/providers/app.rs:215
  - [ ] fn `suprnova::AppConfigBuilder::trusted_proxies` · framework/src/config/providers/app.rs:222
  - [ ] fn `suprnova::AppConfigBuilder::build` · framework/src/config/providers/app.rs:228

### `suprnova::config::providers::server` (private module; items are public through re-exports)

- [ ] struct `suprnova::ServerConfig` · framework/src/config/providers/server.rs:49 (also `suprnova::config::ServerConfig`, `suprnova::config::providers::ServerConfig`)
  - Public fields: `host`, `port`, `max_body_size`, `max_connections`, `header_read_timeout`, `health_readiness_token`
  - [ ] fn `suprnova::ServerConfig::from_env` · framework/src/config/providers/server.rs:129
  - [ ] fn `suprnova::ServerConfig::try_from_env` · framework/src/config/providers/server.rs:152
  - [ ] fn `suprnova::ServerConfig::builder` · framework/src/config/providers/server.rs:194
- [ ] struct `suprnova::ServerConfigBuilder` · framework/src/config/providers/server.rs:301 (also `suprnova::config::ServerConfigBuilder`, `suprnova::config::providers::ServerConfigBuilder`)
  - [ ] fn `suprnova::ServerConfigBuilder::host` · framework/src/config/providers/server.rs:312
  - [ ] fn `suprnova::ServerConfigBuilder::port` · framework/src/config/providers/server.rs:318
  - [ ] fn `suprnova::ServerConfigBuilder::max_body_size` · framework/src/config/providers/server.rs:324
  - [ ] fn `suprnova::ServerConfigBuilder::max_connections` · framework/src/config/providers/server.rs:334
  - [ ] fn `suprnova::ServerConfigBuilder::header_read_timeout` · framework/src/config/providers/server.rs:342
  - [ ] fn `suprnova::ServerConfigBuilder::health_readiness_token` · framework/src/config/providers/server.rs:353
  - [ ] fn `suprnova::ServerConfigBuilder::build` · framework/src/config/providers/server.rs:359
- [ ] const `suprnova::config::providers::DEFAULT_HEADER_READ_TIMEOUT_SECS` · framework/src/config/providers/server.rs:27
- [ ] const `suprnova::config::providers::DEFAULT_MAX_CONNECTIONS_ON_MISCONFIGURATION` · framework/src/config/providers/server.rs:39

### `suprnova::config::repository`

- [ ] fn `suprnova::config::repository::get` · framework/src/config/repository.rs:79
- [ ] fn `suprnova::config::repository::has` · framework/src/config/repository.rs:92
- [ ] fn `suprnova::config::repository::init_repository` · framework/src/config/repository.rs:52
- [ ] fn `suprnova::config::repository::register` · framework/src/config/repository.rs:65
- [ ] struct `suprnova::config::repository::ConfigRepository` · framework/src/config/repository.rs:14
  - [ ] fn `suprnova::config::repository::ConfigRepository::new` · framework/src/config/repository.rs:20
  - [ ] fn `suprnova::config::repository::ConfigRepository::register` · framework/src/config/repository.rs:27
  - [ ] fn `suprnova::config::repository::ConfigRepository::get` · framework/src/config/repository.rs:32
  - [ ] fn `suprnova::config::repository::ConfigRepository::has` · framework/src/config/repository.rs:40

### `suprnova::config::typed`

- [ ] fn `suprnova::config::typed::resolve` · framework/src/config/typed.rs:40
- [ ] fn `suprnova::config::typed::resolve_prefixed` · framework/src/config/typed.rs:52

## console

### `suprnova::console`

- [ ] fn `suprnova::dispatch_argv` · framework/src/console/mod.rs:123 (also `suprnova::console::dispatch_argv`)
- [ ] fn `suprnova::console::dispatch_argv_with_init` · framework/src/console/mod.rs:142
- [ ] fn `suprnova::console::find` · framework/src/console/mod.rs:85
- [ ] fn `suprnova::console::list` · framework/src/console/mod.rs:92
- [ ] fn `suprnova::console::set_version` · framework/src/console/mod.rs:80
- [ ] struct `suprnova::CommandEntry` · framework/src/console/mod.rs:51 (also `suprnova::console::CommandEntry`)
  - Public fields: `name`, `description`, `clap_builder`, `handler`
- [ ] type `suprnova::CommandHandler` · framework/src/console/mod.rs:44 (also `suprnova::console::CommandHandler`)

### `suprnova::console::output`

- [ ] fn `suprnova::two_column_detail` · framework/src/console/output.rs:37 (also `suprnova::console::output::two_column_detail`, `suprnova::console::two_column_detail`)
- [ ] const `suprnova::console::DETAIL_WIDTH` · framework/src/console/output.rs:18 (also `suprnova::console::output::DETAIL_WIDTH`)

### `suprnova::console::typed` (private module; items are public through re-exports)

- [ ] trait `suprnova::TypedCommand` · framework/src/console/typed.rs:48 (also `suprnova::console::TypedCommand`)
  - Implemented here by: `eloquent::console::prune::PruneArgs`
  - [ ] fn `suprnova::TypedCommand::run` · framework/src/console/typed.rs:51 (required)

## container

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

### `suprnova::container::provider`

- [ ] fn `suprnova::container::provider::bootstrap` · framework/src/container/provider.rs:190
- [ ] fn `suprnova::container::provider::register_service_bindings` · framework/src/container/provider.rs:106
- [ ] fn `suprnova::container::provider::register_singletons` · framework/src/container/provider.rs:127
- [ ] struct `suprnova::container::provider::ServiceBindingEntry` · framework/src/container/provider.rs:69
  - Public fields: `register`, `name`
- [ ] struct `suprnova::container::provider::SingletonEntry` · framework/src/container/provider.rs:85
  - Public fields: `register`, `name`

### `suprnova::container::testing`

- [ ] struct `suprnova::testing::TestContainer` · framework/src/container/testing.rs:88 (also `suprnova::container::testing::TestContainer`)
  - [ ] fn `suprnova::testing::TestContainer::fake` · framework/src/container/testing.rs:104
  - [ ] fn `suprnova::testing::TestContainer::scope` · framework/src/container/testing.rs:157
  - [ ] fn `suprnova::testing::TestContainer::spawn` · framework/src/container/testing.rs:200
  - [ ] fn `suprnova::testing::TestContainer::singleton` · framework/src/container/testing.rs:228
  - [ ] fn `suprnova::testing::TestContainer::factory` · framework/src/container/testing.rs:258
  - [ ] fn `suprnova::testing::TestContainer::bind` · framework/src/container/testing.rs:292
  - [ ] fn `suprnova::testing::TestContainer::bind_factory` · framework/src/container/testing.rs:322
- [ ] struct `suprnova::testing::TestContainerGuard` · framework/src/container/testing.rs:345 (also `suprnova::container::testing::TestContainerGuard`)

## content

### `suprnova::content::docs` (private module; items are public through re-exports)

- [ ] fn `suprnova::content::build_docs` · framework/src/content/docs.rs:82
- [ ] struct `suprnova::content::DocsBuildConfig` · framework/src/content/docs.rs:12
  - Public fields: `source_dir`, `output_dir`, `toc_file`
- [ ] struct `suprnova::content::DocsCatalog` · framework/src/content/docs.rs:42
  - Public fields: `chapters`, `search`
- [ ] struct `suprnova::content::DocsCatalogEntry` · framework/src/content/docs.rs:51
  - Public fields: `slug`, `title`, `excerpt`, `headings`, `previous`, `next`
- [ ] struct `suprnova::content::DocsChapter` · framework/src/content/docs.rs:23
  - Public fields: `slug`, `title`, `html`, `excerpt`, `headings`, `previous`, `next`
- [ ] struct `suprnova::content::DocsSearchEntry` · framework/src/content/docs.rs:68
  - Public fields: `slug`, `title`, `excerpt`, `headings`, `plain_text`

### `suprnova::content::headings` (private module; items are public through re-exports)

- [ ] fn `suprnova::content::slugify_heading` · framework/src/content/headings.rs:19
- [ ] struct `suprnova::content::Heading` · framework/src/content/headings.rs:9
  - Public fields: `level`, `id`, `title`

### `suprnova::content::markdown` (private module; items are public through re-exports)

- [ ] struct `suprnova::content::MarkdownOptions` · framework/src/content/markdown.rs:33
  - Public fields: `unsafe_html`, `heading_anchor_prefix`, `render_math`
- [ ] struct `suprnova::content::MarkdownRenderer` · framework/src/content/markdown.rs:67
  - [ ] fn `suprnova::content::MarkdownRenderer::new` · framework/src/content/markdown.rs:73
  - [ ] fn `suprnova::content::MarkdownRenderer::options` · framework/src/content/markdown.rs:78
  - [ ] fn `suprnova::content::MarkdownRenderer::render` · framework/src/content/markdown.rs:83
- [ ] struct `suprnova::content::RenderedMarkdown` · framework/src/content/markdown.rs:54
  - Public fields: `html`, `plain_text`, `excerpt`, `headings`
- [ ] enum `suprnova::content::ContentError` · framework/src/content/markdown.rs:22
  - Variants: `Io`, `Json`
- [ ] type `suprnova::content::ContentResult` · framework/src/content/markdown.rs:18

## context

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

## cors

### `suprnova::cors`

- [ ] struct `suprnova::CorsConfig` · framework/src/cors/mod.rs:146 (also `suprnova::cors::CorsConfig`)
  - [ ] fn `suprnova::CorsConfig::allow_origins` · framework/src/cors/mod.rs:244
  - [ ] fn `suprnova::CorsConfig::any_origin` · framework/src/cors/mod.rs:266
  - [ ] fn `suprnova::CorsConfig::paths` · framework/src/cors/mod.rs:303
  - [ ] fn `suprnova::CorsConfig::allow_origin_patterns` · framework/src/cors/mod.rs:335
  - [ ] fn `suprnova::CorsConfig::skip_when` · framework/src/cors/mod.rs:368
  - [ ] fn `suprnova::CorsConfig::methods` · framework/src/cors/mod.rs:377
  - [ ] fn `suprnova::CorsConfig::allow_headers` · framework/src/cors/mod.rs:388
  - [ ] fn `suprnova::CorsConfig::allow_any_headers` · framework/src/cors/mod.rs:398
  - [ ] fn `suprnova::CorsConfig::expose_headers` · framework/src/cors/mod.rs:405
  - [ ] fn `suprnova::CorsConfig::allow_credentials` · framework/src/cors/mod.rs:427
  - [ ] fn `suprnova::CorsConfig::supports_credentials` · framework/src/cors/mod.rs:447
  - [ ] fn `suprnova::CorsConfig::allowed_methods` · framework/src/cors/mod.rs:453
  - [ ] fn `suprnova::CorsConfig::allowed_headers` · framework/src/cors/mod.rs:463
  - [ ] fn `suprnova::CorsConfig::exposed_headers` · framework/src/cors/mod.rs:473
  - [ ] fn `suprnova::CorsConfig::allowed_origins_patterns` · framework/src/cors/mod.rs:483
  - [ ] fn `suprnova::CorsConfig::max_age` · framework/src/cors/mod.rs:493
  - [ ] fn `suprnova::CorsConfig::max_age_secs` · framework/src/cors/mod.rs:501
- [ ] struct `suprnova::CorsMiddleware` · framework/src/cors/mod.rs:578 (also `suprnova::cors::CorsMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::CorsMiddleware::new` · framework/src/cors/mod.rs:584
- [ ] enum `suprnova::AllowedHeaders` · framework/src/cors/mod.rs:131 (also `suprnova::cors::AllowedHeaders`)
  - Variants: `Any`, `List`
- [ ] enum `suprnova::AllowedOrigins` · framework/src/cors/mod.rs:119 (also `suprnova::cors::AllowedOrigins`)
  - Variants: `Any`, `List`
- [ ] type `suprnova::cors::SkipPredicate` · framework/src/cors/mod.rs:115

## crypto

### `suprnova::crypto`

- [ ] fn `suprnova::crypto::resolve_boot_key` · framework/src/crypto/mod.rs:716
- [ ] fn `suprnova::crypto::resolve_boot_keyring` · framework/src/crypto/mod.rs:776
- [ ] struct `suprnova::crypto::BootKeyRing` · framework/src/crypto/mod.rs:945
  - Public fields: `current`, `previous`
  - [ ] fn `suprnova::crypto::BootKeyRing::is_current_generated` · framework/src/crypto/mod.rs:957
  - [ ] fn `suprnova::crypto::BootKeyRing::into_keys` · framework/src/crypto/mod.rs:963
- [ ] struct `suprnova::Crypt` · framework/src/crypto/mod.rs:297 (also `suprnova::crypto::Crypt`)
  - [ ] fn `suprnova::Crypt::init` · framework/src/crypto/mod.rs:305
  - [ ] fn `suprnova::Crypt::init_with_keyring` · framework/src/crypto/mod.rs:316
  - [ ] fn `suprnova::Crypt::is_initialized` · framework/src/crypto/mod.rs:323
  - [ ] fn `suprnova::Crypt::encrypt_string` · framework/src/crypto/mod.rs:345
  - [ ] fn `suprnova::Crypt::encrypt_string_for` · framework/src/crypto/mod.rs:357
  - [ ] fn `suprnova::Crypt::decrypt_string` · framework/src/crypto/mod.rs:396
  - [ ] fn `suprnova::Crypt::decrypt_string_for` · framework/src/crypto/mod.rs:413
  - [ ] fn `suprnova::Crypt::encrypt` · framework/src/crypto/mod.rs:443
  - [ ] fn `suprnova::Crypt::decrypt` · framework/src/crypto/mod.rs:459
  - [ ] fn `suprnova::Crypt::appears_encrypted` · framework/src/crypto/mod.rs:484
  - [ ] fn `suprnova::Crypt::previous_key_count` · framework/src/crypto/mod.rs:501
  - [ ] fn `suprnova::Crypt::has_previous_keys` · framework/src/crypto/mod.rs:508
- [ ] struct `suprnova::crypto::DecryptOrigin` · framework/src/crypto/mod.rs:265
  - Public fields: `key`, `aad`
- [ ] enum `suprnova::crypto::AadVersion` · framework/src/crypto/mod.rs:239
  - Variants: `Current`, `Legacy`
- [ ] enum `suprnova::crypto::BootKey` · framework/src/crypto/mod.rs:915
  - Variants: `Configured`, `GeneratedTransient`
  - [ ] fn `suprnova::crypto::BootKey::into_key` · framework/src/crypto/mod.rs:926
  - [ ] fn `suprnova::crypto::BootKey::is_generated` · framework/src/crypto/mod.rs:935
- [ ] enum `suprnova::CryptPurpose` · framework/src/crypto/mod.rs:120 (also `suprnova::crypto::CryptPurpose`)
  - Variants: `Cookie`, `Cursor`, `TwoFactorSecret`, `TwoFactorRecovery`, `MagnetarCeremonyState`, `MagnetarProviderToken`, `MagnetarRefreshToken`, `MagnetarSessionGrant`, `Cast`
- [ ] enum `suprnova::crypto::KeyOrigin` · framework/src/crypto/mod.rs:228
  - Variants: `Current`, `Previous`
- [ ] const `suprnova::crypto::MAX_PREVIOUS_KEYS` · framework/src/crypto/mod.rs:860

### `suprnova::crypto::key`

- [ ] struct `suprnova::EncryptionKey` · framework/src/crypto/key.rs:18 (also `suprnova::crypto::EncryptionKey`, `suprnova::crypto::key::EncryptionKey`)
  - [ ] fn `suprnova::EncryptionKey::from_env` · framework/src/crypto/key.rs:26
  - [ ] fn `suprnova::EncryptionKey::generate` · framework/src/crypto/key.rs:36
  - [ ] fn `suprnova::EncryptionKey::from_base64` · framework/src/crypto/key.rs:44
  - [ ] fn `suprnova::EncryptionKey::to_base64` · framework/src/crypto/key.rs:60
  - [ ] fn `suprnova::EncryptionKey::as_bytes` · framework/src/crypto/key.rs:65

## csrf

### `suprnova::csrf`

- [ ] fn `suprnova::csrf_field` · framework/src/csrf/mod.rs:105 (also `suprnova::csrf::csrf_field`)
- [ ] fn `suprnova::csrf_meta_tag` · framework/src/csrf/mod.rs:89 (also `suprnova::csrf::csrf_meta_tag`)
- [ ] fn `suprnova::csrf_token` · framework/src/csrf/mod.rs:75 (also `suprnova::csrf::csrf_token`)

### `suprnova::csrf::middleware`

- [ ] struct `suprnova::CsrfMiddleware` · framework/src/csrf/middleware.rs:190 (also `suprnova::csrf::CsrfMiddleware`, `suprnova::csrf::middleware::CsrfMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::CsrfMiddleware::new` · framework/src/csrf/middleware.rs:233
  - [ ] fn `suprnova::CsrfMiddleware::except` · framework/src/csrf/middleware.rs:277
  - [ ] fn `suprnova::CsrfMiddleware::except_method` · framework/src/csrf/middleware.rs:304
  - [ ] fn `suprnova::CsrfMiddleware::allow_same_site` · framework/src/csrf/middleware.rs:319
  - [ ] fn `suprnova::CsrfMiddleware::origin_only` · framework/src/csrf/middleware.rs:337
  - [ ] fn `suprnova::CsrfMiddleware::with_origin_policy` · framework/src/csrf/middleware.rs:345
  - [ ] fn `suprnova::CsrfMiddleware::without_xsrf_cookie` · framework/src/csrf/middleware.rs:355
  - [ ] fn `suprnova::CsrfMiddleware::xsrf_cookie_name` · framework/src/csrf/middleware.rs:361
  - [ ] fn `suprnova::CsrfMiddleware::xsrf_cookie_path` · framework/src/csrf/middleware.rs:367
  - [ ] fn `suprnova::CsrfMiddleware::xsrf_cookie_domain` · framework/src/csrf/middleware.rs:373
  - [ ] fn `suprnova::CsrfMiddleware::xsrf_cookie_secure` · framework/src/csrf/middleware.rs:381
  - [ ] fn `suprnova::CsrfMiddleware::xsrf_cookie_same_site` · framework/src/csrf/middleware.rs:387
  - [ ] fn `suprnova::CsrfMiddleware::xsrf_cookie_lifetime` · framework/src/csrf/middleware.rs:395
  - [ ] fn `suprnova::CsrfMiddleware::with_session_config` · framework/src/csrf/middleware.rs:434
- [ ] enum `suprnova::OriginPolicy` · framework/src/csrf/middleware.rs:31 (also `suprnova::csrf::OriginPolicy`, `suprnova::csrf::middleware::OriginPolicy`)
  - Variants: `Disabled`, `SameOriginOnly`, `AllowSameSite`, `OriginOnly`

## data

### `suprnova::data::error` (private module; items are public through re-exports)

- [ ] enum `suprnova::IncludeError` · framework/src/data/error.rs:8 (also `suprnova::data::IncludeError`)
  - Variants: `UnknownInclude`
  - [ ] fn `suprnova::IncludeError::into_framework_error` · framework/src/data/error.rs:22

### `suprnova::data::field` (private module; items are public through re-exports)

- [ ] enum `suprnova::Field` · framework/src/data/field.rs:26 (also `suprnova::data::Field`)
  - Variants: `Absent`, `Null`, `Value`
  - [ ] fn `suprnova::Field::is_absent` · framework/src/data/field.rs:38
  - [ ] fn `suprnova::Field::is_null` · framework/src/data/field.rs:43
  - [ ] fn `suprnova::Field::is_value` · framework/src/data/field.rs:48
  - [ ] fn `suprnova::Field::as_value` · framework/src/data/field.rs:53
  - [ ] fn `suprnova::Field::into_value` · framework/src/data/field.rs:61
  - [ ] fn `suprnova::Field::into_option_or_null` · framework/src/data/field.rs:73

### `suprnova::data::include_set` (private module; items are public through re-exports)

- [ ] fn `suprnova::current_include_set` · framework/src/data/include_set.rs:305 (also `suprnova::data::current_include_set`)
- [ ] fn `suprnova::scope_include_set` · framework/src/data/include_set.rs:323 (also `suprnova::data::scope_include_set`)
- [ ] fn `suprnova::with_include_overrides` · framework/src/data/include_set.rs:357 (also `suprnova::data::with_include_overrides`)
- [ ] struct `suprnova::RequestIncludeSet` · framework/src/data/include_set.rs:23 (also `suprnova::data::RequestIncludeSet`)
  - Public fields: `include`, `exclude`, `only`, `except`
  - [ ] fn `suprnova::RequestIncludeSet::from_query` · framework/src/data/include_set.rs:56
  - [ ] fn `suprnova::RequestIncludeSet::is_empty` · framework/src/data/include_set.rs:82
  - [ ] fn `suprnova::RequestIncludeSet::includes` · framework/src/data/include_set.rs:95
  - [ ] fn `suprnova::RequestIncludeSet::is_excluded` · framework/src/data/include_set.rs:103
  - [ ] fn `suprnova::RequestIncludeSet::is_excepted` · framework/src/data/include_set.rs:112
  - [ ] fn `suprnova::RequestIncludeSet::is_only_listed` · framework/src/data/include_set.rs:122
  - [ ] fn `suprnova::RequestIncludeSet::is_visible` · framework/src/data/include_set.rs:142
  - [ ] fn `suprnova::RequestIncludeSet::include` · framework/src/data/include_set.rs:158
  - [ ] fn `suprnova::RequestIncludeSet::exclude` · framework/src/data/include_set.rs:170
  - [ ] fn `suprnova::RequestIncludeSet::only` · framework/src/data/include_set.rs:185
  - [ ] fn `suprnova::RequestIncludeSet::except` · framework/src/data/include_set.rs:198
  - [ ] fn `suprnova::RequestIncludeSet::include_when` · framework/src/data/include_set.rs:213
  - [ ] fn `suprnova::RequestIncludeSet::exclude_when` · framework/src/data/include_set.rs:228
  - [ ] fn `suprnova::RequestIncludeSet::only_when` · framework/src/data/include_set.rs:243
  - [ ] fn `suprnova::RequestIncludeSet::except_when` · framework/src/data/include_set.rs:254
  - [ ] fn `suprnova::RequestIncludeSet::merge` · framework/src/data/include_set.rs:267
- [ ] static `suprnova::data::REQUEST_INCLUDE_SET` · framework/src/data/include_set.rs:297

### `suprnova::data::middleware` (private module; items are public through re-exports)

- [ ] struct `suprnova::IncludeMiddleware` · framework/src/data/middleware.rs:49 (also `suprnova::data::IncludeMiddleware`)
  - Implements: `suprnova::Middleware`

### `suprnova::data::registry`

- [ ] fn `suprnova::data::registry::allowed_for` · framework/src/data/registry.rs:107
- [ ] fn `suprnova::data::registry::is_allowed` · framework/src/data/registry.rs:93
- [ ] fn `suprnova::data::registry::register` · framework/src/data/registry.rs:84
- [ ] struct `suprnova::data::registry::AllowedIncludes` · framework/src/data/registry.rs:45
  - Public fields: `struct_name`, `fields`

### `suprnova::data::route_params`

- [ ] fn `suprnova::data::route_params::parse_bool` · framework/src/data/route_params.rs:80
- [ ] fn `suprnova::data::route_params::parse_f32` · framework/src/data/route_params.rs:72
- [ ] fn `suprnova::data::route_params::parse_f64` · framework/src/data/route_params.rs:64
- [ ] fn `suprnova::data::route_params::parse_i128` · framework/src/data/route_params.rs:50
- [ ] fn `suprnova::data::route_params::parse_i32` · framework/src/data/route_params.rs:33
- [ ] fn `suprnova::data::route_params::parse_i64` · framework/src/data/route_params.rs:19
- [ ] fn `suprnova::data::route_params::parse_u128` · framework/src/data/route_params.rs:58
- [ ] fn `suprnova::data::route_params::parse_u32` · framework/src/data/route_params.rs:40
- [ ] fn `suprnova::data::route_params::parse_u64` · framework/src/data/route_params.rs:26
- [ ] fn `suprnova::data::route_params::pass_string` · framework/src/data/route_params.rs:89

### `suprnova::data::when_loaded` (private module; items are public through re-exports)

- [ ] trait `suprnova::IsRelationLoaded` · framework/src/data/when_loaded.rs:41 (also `suprnova::data::IsRelationLoaded`)
  - [ ] fn `suprnova::IsRelationLoaded::is_relation_loaded` · framework/src/data/when_loaded.rs:45 (required)

## database

### `suprnova::database`

- [ ] struct `suprnova::DB` · framework/src/database/mod.rs:181 (also `suprnova::database::DB`)
  - [ ] fn `suprnova::DB::table` · framework/src/database/db_facade.rs:738
  - [ ] fn `suprnova::DB::select` · framework/src/database/db_facade.rs:763
  - [ ] fn `suprnova::DB::select_one` · framework/src/database/db_facade.rs:787
  - [ ] fn `suprnova::DB::scalar` · framework/src/database/db_facade.rs:848
  - [ ] fn `suprnova::DB::insert` · framework/src/database/db_facade.rs:904
  - [ ] fn `suprnova::DB::update` · framework/src/database/db_facade.rs:914
  - [ ] fn `suprnova::DB::delete` · framework/src/database/db_facade.rs:923
  - [ ] fn `suprnova::DB::statement` · framework/src/database/db_facade.rs:946
  - [ ] fn `suprnova::DB::unprepared` · framework/src/database/db_facade.rs:974
  - [ ] fn `suprnova::DB::affecting_statement` · framework/src/database/db_facade.rs:1031
  - [ ] fn `suprnova::DB::table_on` · framework/src/database/db_facade.rs:1085
  - [ ] fn `suprnova::DB::select_on` · framework/src/database/db_facade.rs:1095
  - [ ] fn `suprnova::DB::statement_on` · framework/src/database/db_facade.rs:1125
  - [ ] fn `suprnova::DB::affecting_statement_on` · framework/src/database/db_facade.rs:1154
  - [ ] fn `suprnova::DB::listen` · framework/src/database/db_facade.rs:1205
  - [ ] fn `suprnova::DB::flush_listeners` · framework/src/database/db_facade.rs:1219
  - [ ] fn `suprnova::DB::enable_query_log` · framework/src/database/db_facade.rs:1233
  - [ ] fn `suprnova::DB::disable_query_log` · framework/src/database/db_facade.rs:1244
  - [ ] fn `suprnova::DB::logging` · framework/src/database/db_facade.rs:1254
  - [ ] fn `suprnova::DB::get_query_log` · framework/src/database/db_facade.rs:1265
  - [ ] fn `suprnova::DB::flush_query_log` · framework/src/database/db_facade.rs:1275
  - [ ] fn `suprnova::DB::database_name` · framework/src/database/db_facade.rs:1293
  - [ ] fn `suprnova::DB::driver_name` · framework/src/database/db_facade.rs:1305
  - [ ] fn `suprnova::DB::driver_title` · framework/src/database/db_facade.rs:1322
  - [ ] fn `suprnova::DB::server_version` · framework/src/database/db_facade.rs:1343
  - [ ] fn `suprnova::DB::transaction` · framework/src/database/transaction.rs:1233
  - [ ] fn `suprnova::DB::begin_transaction` · framework/src/database/transaction.rs:1437
  - [ ] fn `suprnova::DB::transaction_with_attempts` · framework/src/database/transaction.rs:1487
  - [ ] fn `suprnova::DB::init` · framework/src/database/mod.rs:204
  - [ ] fn `suprnova::DB::init_with` · framework/src/database/mod.rs:237
  - [ ] fn `suprnova::DB::connection` · framework/src/database/mod.rs:281
  - [ ] fn `suprnova::DB::is_connected` · framework/src/database/mod.rs:298
  - [ ] fn `suprnova::DB::get` · framework/src/database/mod.rs:325
  - [ ] fn `suprnova::DB::register_named` · framework/src/database/mod.rs:346
  - [ ] fn `suprnova::DB::named` · framework/src/database/mod.rs:353
- [ ] type `suprnova::Database` · framework/src/database/mod.rs:135 (also `suprnova::database::Database`)

### `suprnova::database::config`

- [ ] struct `suprnova::DatabaseConfig` · framework/src/database/config.rs:70 (also `suprnova::database::DatabaseConfig`, `suprnova::database::config::DatabaseConfig`)
  - Public fields: `url`, `max_connections`, `min_connections`, `connect_timeout`, `logging`, `idle_timeout`, `max_lifetime`, `acquire_timeout`, `test_before_acquire`, `ping_after_idle`, `url_source`
  - [ ] const `suprnova::DatabaseConfig::DEFAULT_SQLITE_URL` · framework/src/database/config.rs:129
  - [ ] fn `suprnova::DatabaseConfig::from_env` · framework/src/database/config.rs:138
  - [ ] fn `suprnova::DatabaseConfig::builder` · framework/src/database/config.rs:159
  - [ ] fn `suprnova::DatabaseConfig::database_type` · framework/src/database/config.rs:164
  - [ ] fn `suprnova::DatabaseConfig::is_configured` · framework/src/database/config.rs:183
  - [ ] fn `suprnova::DatabaseConfig::validate_for_environment` · framework/src/database/config.rs:201
  - [ ] fn `suprnova::DatabaseConfig::validate_pool` · framework/src/database/config.rs:236
- [ ] struct `suprnova::database::DatabaseConfigBuilder` · framework/src/database/config.rs:271 (also `suprnova::database::config::DatabaseConfigBuilder`)
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::url` · framework/src/database/config.rs:286
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::max_connections` · framework/src/database/config.rs:292
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::min_connections` · framework/src/database/config.rs:298
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::connect_timeout` · framework/src/database/config.rs:304
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::logging` · framework/src/database/config.rs:310
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::idle_timeout` · framework/src/database/config.rs:317
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::max_lifetime` · framework/src/database/config.rs:324
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::acquire_timeout` · framework/src/database/config.rs:331
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::test_before_acquire` · framework/src/database/config.rs:337
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::ping_after_idle` · framework/src/database/config.rs:344
  - [ ] fn `suprnova::database::DatabaseConfigBuilder::build` · framework/src/database/config.rs:358
- [ ] enum `suprnova::DatabaseType` · framework/src/database/config.rs:8 (also `suprnova::database::DatabaseType`, `suprnova::database::config::DatabaseType`)
  - Variants: `Postgres`, `Mysql`, `Sqlite`, `Unknown`
- [ ] enum `suprnova::UrlSource` · framework/src/database/config.rs:28 (also `suprnova::database::UrlSource`, `suprnova::database::config::UrlSource`)
  - Variants: `Env`, `Default`, `Explicit`

### `suprnova::database::connection`

- [ ] struct `suprnova::DbConnection` · framework/src/database/connection.rs:24 (also `suprnova::database::DbConnection`, `suprnova::database::connection::DbConnection`)
  - [ ] fn `suprnova::DbConnection::connect` · framework/src/database/connection.rs:34
  - [ ] fn `suprnova::DbConnection::from_raw` · framework/src/database/connection.rs:157
  - [ ] fn `suprnova::DbConnection::inner` · framework/src/database/connection.rs:176
  - [ ] fn `suprnova::DbConnection::conn` · framework/src/database/connection.rs:190

### `suprnova::database::connection_registry`

- [ ] struct `suprnova::ConnectionRegistry` · framework/src/database/connection_registry.rs:74 (also `suprnova::database::ConnectionRegistry`, `suprnova::database::connection_registry::ConnectionRegistry`)
  - [ ] fn `suprnova::ConnectionRegistry::register` · framework/src/database/connection_registry.rs:116
  - [ ] fn `suprnova::ConnectionRegistry::register_existing` · framework/src/database/connection_registry.rs:134
  - [ ] fn `suprnova::ConnectionRegistry::get` · framework/src/database/connection_registry.rs:146
  - [ ] fn `suprnova::ConnectionRegistry::has` · framework/src/database/connection_registry.rs:168
- [ ] const `suprnova::PRIMARY_CONNECTION_NAME` · framework/src/database/connection_registry.rs:81 (also `suprnova::database::PRIMARY_CONNECTION_NAME`, `suprnova::database::connection_registry::PRIMARY_CONNECTION_NAME`)
- [ ] const `suprnova::READ_REPLICA_CONNECTION_NAME` · framework/src/database/connection_registry.rs:86 (also `suprnova::database::READ_REPLICA_CONNECTION_NAME`, `suprnova::database::connection_registry::READ_REPLICA_CONNECTION_NAME`)

### `suprnova::database::db_facade`

- [ ] struct `suprnova::DbTableBuilder` · framework/src/database/db_facade.rs:110 (also `suprnova::database::DbTableBuilder`, `suprnova::database::db_facade::DbTableBuilder`)
  - [ ] fn `suprnova::DbTableBuilder::new` · framework/src/database/db_facade.rs:129
  - [ ] fn `suprnova::DbTableBuilder::on` · framework/src/database/db_facade.rs:145
  - [ ] fn `suprnova::DbTableBuilder::select` · framework/src/database/db_facade.rs:158
  - [ ] fn `suprnova::DbTableBuilder::filter` · framework/src/database/db_facade.rs:168
  - [ ] fn `suprnova::DbTableBuilder::filter_op` · framework/src/database/db_facade.rs:181
  - [ ] fn `suprnova::DbTableBuilder::where_binary` · framework/src/database/db_facade.rs:204
  - [ ] fn `suprnova::DbTableBuilder::where_not_binary` · framework/src/database/db_facade.rs:216
  - [ ] fn `suprnova::DbTableBuilder::order_by_desc` · framework/src/database/db_facade.rs:228
  - [ ] fn `suprnova::DbTableBuilder::order_by_asc` · framework/src/database/db_facade.rs:234
  - [ ] fn `suprnova::DbTableBuilder::limit` · framework/src/database/db_facade.rs:240
  - [ ] fn `suprnova::DbTableBuilder::offset` · framework/src/database/db_facade.rs:246
  - [ ] fn `suprnova::DbTableBuilder::get` · framework/src/database/db_facade.rs:282
  - [ ] fn `suprnova::DbTableBuilder::first` · framework/src/database/db_facade.rs:310
  - [ ] fn `suprnova::DbTableBuilder::count` · framework/src/database/db_facade.rs:325
  - [ ] fn `suprnova::DbTableBuilder::insert` · framework/src/database/db_facade.rs:384
  - [ ] fn `suprnova::DbTableBuilder::update` · framework/src/database/db_facade.rs:506
  - [ ] fn `suprnova::DbTableBuilder::update_all` · framework/src/database/db_facade.rs:551
  - [ ] fn `suprnova::DbTableBuilder::delete` · framework/src/database/db_facade.rs:566
  - [ ] fn `suprnova::DbTableBuilder::delete_all` · framework/src/database/db_facade.rs:600

### `suprnova::database::dynamic_row`

- [ ] struct `suprnova::DynamicRow` · framework/src/database/dynamic_row.rs:59 (also `suprnova::database::DynamicRow`, `suprnova::database::dynamic_row::DynamicRow`)
  - Public tuple fields: 1
  - [ ] fn `suprnova::DynamicRow::from_map` · framework/src/database/dynamic_row.rs:65
  - [ ] fn `suprnova::DynamicRow::into_map` · framework/src/database/dynamic_row.rs:71
  - [ ] fn `suprnova::DynamicRow::get_int` · framework/src/database/dynamic_row.rs:77
  - [ ] fn `suprnova::DynamicRow::get_string` · framework/src/database/dynamic_row.rs:88
  - [ ] fn `suprnova::DynamicRow::get_bool` · framework/src/database/dynamic_row.rs:100
  - [ ] fn `suprnova::DynamicRow::get_float` · framework/src/database/dynamic_row.rs:112
  - [ ] fn `suprnova::DynamicRow::get_value` · framework/src/database/dynamic_row.rs:124
  - [ ] fn `suprnova::DynamicRow::get_as` · framework/src/database/dynamic_row.rs:138
  - [ ] fn `suprnova::DynamicRow::get_optional_string` · framework/src/database/dynamic_row.rs:148
  - [ ] fn `suprnova::DynamicRow::get_optional_int` · framework/src/database/dynamic_row.rs:164

### `suprnova::database::events`

- [ ] struct `suprnova::ConnectionEstablished` · framework/src/database/events.rs:54 (also `suprnova::database::ConnectionEstablished`, `suprnova::database::events::ConnectionEstablished`)
  - Public fields: `connection_name`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::DatabaseBusy` · framework/src/database/events.rs:208 (also `suprnova::database::DatabaseBusy`, `suprnova::database::events::DatabaseBusy`)
  - Public fields: `connection_name`, `connections`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::QueryExecuted` · framework/src/database/events.rs:73 (also `suprnova::database::QueryExecuted`, `suprnova::database::events::QueryExecuted`)
  - Public fields: `sql`, `bindings`, `time`, `connection_name`, `read_write_type`, `result`
  - Implements: `suprnova::Event`
  - [ ] fn `suprnova::QueryExecuted::to_raw_sql` · framework/src/database/events.rs:118
- [ ] struct `suprnova::TransactionBeginning` · framework/src/database/events.rs:162 (also `suprnova::database::TransactionBeginning`, `suprnova::database::events::TransactionBeginning`)
  - Public fields: `connection_name`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::TransactionCommitted` · framework/src/database/events.rs:175 (also `suprnova::database::TransactionCommitted`, `suprnova::database::events::TransactionCommitted`)
  - Public fields: `connection_name`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::TransactionRolledBack` · framework/src/database/events.rs:193 (also `suprnova::database::TransactionRolledBack`, `suprnova::database::events::TransactionRolledBack`)
  - Public fields: `connection_name`
  - Implements: `suprnova::Event`
- [ ] enum `suprnova::ReadWriteType` · framework/src/database/events.rs:100 (also `suprnova::database::ReadWriteType`, `suprnova::database::events::ReadWriteType`)
  - Variants: `Read`, `Write`
- [ ] type `suprnova::QueryListener` · framework/src/database/events.rs:270 (also `suprnova::database::QueryListener`, `suprnova::database::events::QueryListener`)

### `suprnova::database::identifier`

- [ ] fn `suprnova::database::validate_identifier` · framework/src/database/identifier.rs:61 (also `suprnova::database::identifier::validate_identifier`)
- [ ] fn `suprnova::database::identifier::validate_savepoint_name` · framework/src/database/identifier.rs:126
- [ ] fn `suprnova::database::validate_sql_operator` · framework/src/database/identifier.rs:200 (also `suprnova::database::identifier::validate_sql_operator`)

### `suprnova::database::model`

- [ ] trait `suprnova::EntityExt` · framework/src/database/model.rs:111 (also `suprnova::database::EntityExt`, `suprnova::database::model::EntityExt`)
  - [ ] fn `suprnova::EntityExt::all` · framework/src/database/model.rs:140 (provided)
  - [ ] fn `suprnova::EntityExt::find_by_pk` · framework/src/database/model.rs:175 (provided)
  - [ ] fn `suprnova::EntityExt::find_or_fail` · framework/src/database/model.rs:214 (provided)
  - [ ] fn `suprnova::EntityExt::count_all` · framework/src/database/model.rs:252 (provided)
  - [ ] fn `suprnova::EntityExt::exists_any` · framework/src/database/model.rs:289 (provided)
  - [ ] fn `suprnova::EntityExt::first` · framework/src/database/model.rs:318 (provided)
- [ ] trait `suprnova::EntityExtMut` · framework/src/database/model.rs:373 (also `suprnova::database::EntityExtMut`, `suprnova::database::model::EntityExtMut`)
  - [ ] fn `suprnova::EntityExtMut::insert_one` · framework/src/database/model.rs:411 (provided)
  - [ ] fn `suprnova::EntityExtMut::update_one` · framework/src/database/model.rs:456 (provided)
  - [ ] fn `suprnova::EntityExtMut::delete_by_pk` · framework/src/database/model.rs:497 (provided)
  - [ ] fn `suprnova::EntityExtMut::save_one` · framework/src/database/model.rs:546 (provided)

### `suprnova::database::query_builder`

- [ ] struct `suprnova::database::QueryBuilder` · framework/src/database/query_builder.rs:84 (also `suprnova::database::query_builder::QueryBuilder`)
  - [ ] fn `suprnova::database::QueryBuilder::new` · framework/src/database/query_builder.rs:97
  - [ ] fn `suprnova::database::QueryBuilder::filter` · framework/src/database/query_builder.rs:126
  - [ ] fn `suprnova::database::QueryBuilder::order_by_asc` · framework/src/database/query_builder.rs:156
  - [ ] fn `suprnova::database::QueryBuilder::order_by_desc` · framework/src/database/query_builder.rs:186
  - [ ] fn `suprnova::database::QueryBuilder::order_by` · framework/src/database/query_builder.rs:217
  - [ ] fn `suprnova::database::QueryBuilder::limit` · framework/src/database/query_builder.rs:243
  - [ ] fn `suprnova::database::QueryBuilder::offset` · framework/src/database/query_builder.rs:268
  - [ ] fn `suprnova::database::QueryBuilder::all` · framework/src/database/query_builder.rs:288
  - [ ] fn `suprnova::database::QueryBuilder::first` · framework/src/database/query_builder.rs:322
  - [ ] fn `suprnova::database::QueryBuilder::first_or_fail` · framework/src/database/query_builder.rs:356
  - [ ] fn `suprnova::database::QueryBuilder::count` · framework/src/database/query_builder.rs:385
  - [ ] fn `suprnova::database::QueryBuilder::exists` · framework/src/database/query_builder.rs:416
  - [ ] fn `suprnova::database::QueryBuilder::into_select` · framework/src/database/query_builder.rs:453

### `suprnova::database::route_binding`

- [ ] struct `suprnova::RouteParam` · framework/src/database/route_binding.rs:115 (also `suprnova::database::RouteParam`, `suprnova::database::route_binding::RouteParam`)
  - Public tuple fields: 1
  - Implements: `suprnova::AutoRouteBinding`
  - [ ] fn `suprnova::RouteParam::into_inner` · framework/src/database/route_binding.rs:119
- [ ] trait `suprnova::AutoRouteBinding` · framework/src/database/route_binding.rs:206 (also `suprnova::database::AutoRouteBinding`, `suprnova::database::route_binding::AutoRouteBinding`)
  - Implemented here by: `RouteParam`
  - [ ] fn `suprnova::AutoRouteBinding::from_route_param` · framework/src/database/route_binding.rs:217 (required)
- [ ] trait `suprnova::RouteBinding` · framework/src/database/route_binding.rs:168 (also `suprnova::database::RouteBinding`, `suprnova::database::route_binding::RouteBinding`)
  - [ ] fn `suprnova::RouteBinding::param_name` · framework/src/database/route_binding.rs:173 (required)
  - [ ] fn `suprnova::RouteBinding::from_route_param` · framework/src/database/route_binding.rs:185 (required)

### `suprnova::database::testing`

- [ ] struct `suprnova::database::TestDatabase` · framework/src/database/testing.rs:60 (also `suprnova::database::testing::TestDatabase`, `suprnova::testing::TestDatabase`)
  - [ ] fn `suprnova::database::TestDatabase::fresh` · framework/src/database/testing.rs:94
  - [ ] fn `suprnova::database::TestDatabase::conn` · framework/src/database/testing.rs:134
  - [ ] fn `suprnova::database::TestDatabase::db` · framework/src/database/testing.rs:141
  - [ ] fn `suprnova::database::TestDatabase::sqlite_memory` · framework/src/database/testing.rs:153
  - [ ] fn `suprnova::database::TestDatabase::execute_unprepared` · framework/src/database/testing.rs:172
  - [ ] fn `suprnova::database::TestDatabase::fetch_one` · framework/src/database/testing.rs:184
  - [ ] fn `suprnova::database::TestDatabase::fetch_all` · framework/src/database/testing.rs:201

### `suprnova::database::transaction`

- [ ] struct `suprnova::Transaction` · framework/src/database/transaction.rs:301 (also `suprnova::database::Transaction`, `suprnova::database::transaction::Transaction`)
  - [ ] fn `suprnova::Transaction::backend` · framework/src/database/transaction.rs:1014
  - [ ] fn `suprnova::Transaction::query_all` · framework/src/database/transaction.rs:1022
  - [ ] fn `suprnova::Transaction::handle` · framework/src/database/transaction.rs:1033
  - [ ] fn `suprnova::Transaction::savepoint` · framework/src/database/transaction.rs:1061
  - [ ] fn `suprnova::Transaction::rollback_to` · framework/src/database/transaction.rs:1107
  - [ ] fn `suprnova::Transaction::commit` · framework/src/database/transaction.rs:1148
  - [ ] fn `suprnova::Transaction::rollback` · framework/src/database/transaction.rs:1172
- [ ] struct `suprnova::TxHandle` · framework/src/database/transaction.rs:330 (also `suprnova::database::TxHandle`, `suprnova::database::transaction::TxHandle`)

## eloquent

### `suprnova::eloquent`

- [ ] trait `suprnova::EloquentModel` · framework/src/eloquent/mod.rs:68 (also `suprnova::eloquent::EloquentModel`)
  - Implemented here by: `features::entity::Feature`, `payments::entities::customer::Customer`, `payments::entities::payment_method::PaymentMethod`, `payments::entities::subscription::Subscription`, `payments::entities::subscription_item::SubscriptionItem`, `payments::entities::transaction::Transaction`, `payments::entities::webhook_event::WebhookEvent`, `rbac::entity::ModelPermission`, `rbac::entity::ModelRole`, `rbac::entity::Permission`, `rbac::entity::Role`, `rbac::entity::RolePermission`
  - [ ] type `suprnova::EloquentModel::Entity` · framework/src/eloquent/mod.rs:70
  - [ ] type `suprnova::EloquentModel::Column` · framework/src/eloquent/mod.rs:72
  - [ ] type `suprnova::EloquentModel::Key` · framework/src/eloquent/mod.rs:88
  - [ ] const `suprnova::EloquentModel::TABLE` · framework/src/eloquent/mod.rs:90
  - [ ] const `suprnova::EloquentModel::PRIMARY_KEY` · framework/src/eloquent/mod.rs:97
  - [ ] const `suprnova::EloquentModel::SOFT_DELETES_COLUMN` · framework/src/eloquent/mod.rs:104
  - [ ] const `suprnova::EloquentModel::TOUCHES` · framework/src/eloquent/mod.rs:113
  - [ ] const `suprnova::EloquentModel::HAS_TIMESTAMPS` · framework/src/eloquent/mod.rs:125
  - [ ] const `suprnova::EloquentModel::UPDATED_AT_COLUMN` · framework/src/eloquent/mod.rs:130
  - [ ] fn `suprnova::EloquentModel::default_connection_name` · framework/src/eloquent/mod.rs:151 (provided)

### `suprnova::eloquent::attrs`

- [ ] struct `suprnova::Attrs` · framework/src/eloquent/attrs.rs:28 (also `suprnova::eloquent::Attrs`, `suprnova::eloquent::attrs::Attrs`)
  - Public tuple fields: 1
  - [ ] fn `suprnova::Attrs::new` · framework/src/eloquent/attrs.rs:32
  - [ ] fn `suprnova::Attrs::insert` · framework/src/eloquent/attrs.rs:40
  - [ ] fn `suprnova::Attrs::get` · framework/src/eloquent/attrs.rs:46
  - [ ] fn `suprnova::Attrs::keys` · framework/src/eloquent/attrs.rs:51
  - [ ] fn `suprnova::Attrs::iter` · framework/src/eloquent/attrs.rs:56
  - [ ] fn `suprnova::Attrs::len` · framework/src/eloquent/attrs.rs:61
  - [ ] fn `suprnova::Attrs::is_empty` · framework/src/eloquent/attrs.rs:66
  - [ ] fn `suprnova::Attrs::contains_key` · framework/src/eloquent/attrs.rs:71
  - [ ] fn `suprnova::Attrs::merge` · framework/src/eloquent/attrs.rs:78

### `suprnova::eloquent::builder`

- [ ] struct `suprnova::Builder` · framework/src/eloquent/builder.rs:400 (also `suprnova::eloquent::Builder`, `suprnova::eloquent::builder::Builder`)
  - [ ] fn `suprnova::Builder::new` · framework/src/eloquent/builder.rs:901
  - [ ] fn `suprnova::Builder::on` · framework/src/eloquent/builder.rs:951
  - [ ] fn `suprnova::Builder::on_write_connection` · framework/src/eloquent/builder.rs:976
  - [ ] fn `suprnova::Builder::with_tx` · framework/src/eloquent/builder.rs:998
  - [ ] fn `suprnova::Builder::with` · framework/src/eloquent/builder.rs:1020
  - [ ] fn `suprnova::Builder::with_count` · framework/src/eloquent/builder.rs:1046
  - [ ] fn `suprnova::Builder::with_sum` · framework/src/eloquent/builder.rs:1071
  - [ ] fn `suprnova::Builder::with_avg` · framework/src/eloquent/builder.rs:1080
  - [ ] fn `suprnova::Builder::with_min` · framework/src/eloquent/builder.rs:1089
  - [ ] fn `suprnova::Builder::with_max` · framework/src/eloquent/builder.rs:1097
  - [ ] fn `suprnova::Builder::with_where` · framework/src/eloquent/builder.rs:1124
  - [ ] fn `suprnova::Builder::filter` · framework/src/eloquent/builder.rs:1184
  - [ ] fn `suprnova::Builder::db_where` · framework/src/eloquent/builder.rs:1192
  - [ ] fn `suprnova::Builder::filter_op` · framework/src/eloquent/builder.rs:1204
  - [ ] fn `suprnova::Builder::db_where_op` · framework/src/eloquent/builder.rs:1215
  - [ ] fn `suprnova::Builder::or_filter` · framework/src/eloquent/builder.rs:1223
  - [ ] fn `suprnova::Builder::or_where` · framework/src/eloquent/builder.rs:1245
  - [ ] fn `suprnova::Builder::filter_not` · framework/src/eloquent/builder.rs:1251
  - [ ] fn `suprnova::Builder::where_not` · framework/src/eloquent/builder.rs:1261
  - [ ] fn `suprnova::Builder::filter_in` · framework/src/eloquent/builder.rs:1270
  - [ ] fn `suprnova::Builder::where_in` · framework/src/eloquent/builder.rs:1282
  - [ ] fn `suprnova::Builder::filter_not_in` · framework/src/eloquent/builder.rs:1293
  - [ ] fn `suprnova::Builder::where_not_in` · framework/src/eloquent/builder.rs:1305
  - [ ] fn `suprnova::Builder::filter_between` · framework/src/eloquent/builder.rs:1318
  - [ ] fn `suprnova::Builder::where_between` · framework/src/eloquent/builder.rs:1334
  - [ ] fn `suprnova::Builder::filter_not_between` · framework/src/eloquent/builder.rs:1344
  - [ ] fn `suprnova::Builder::where_not_between` · framework/src/eloquent/builder.rs:1360
  - [ ] fn `suprnova::Builder::filter_null` · framework/src/eloquent/builder.rs:1372
  - [ ] fn `suprnova::Builder::where_null` · framework/src/eloquent/builder.rs:1379
  - [ ] fn `suprnova::Builder::filter_not_null` · framework/src/eloquent/builder.rs:1385
  - [ ] fn `suprnova::Builder::where_not_null` · framework/src/eloquent/builder.rs:1392
  - [ ] fn `suprnova::Builder::filter_like` · framework/src/eloquent/builder.rs:1401
  - [ ] fn `suprnova::Builder::where_like` · framework/src/eloquent/builder.rs:1409
  - [ ] fn `suprnova::Builder::filter_not_like` · framework/src/eloquent/builder.rs:1415
  - [ ] fn `suprnova::Builder::where_not_like` · framework/src/eloquent/builder.rs:1423
  - [ ] fn `suprnova::Builder::filter_binary` · framework/src/eloquent/builder.rs:1439
  - [ ] fn `suprnova::Builder::where_binary` · framework/src/eloquent/builder.rs:1447
  - [ ] fn `suprnova::Builder::or_filter_binary` · framework/src/eloquent/builder.rs:1455
  - [ ] fn `suprnova::Builder::or_where_binary` · framework/src/eloquent/builder.rs:1462
  - [ ] fn `suprnova::Builder::filter_not_binary` · framework/src/eloquent/builder.rs:1469
  - [ ] fn `suprnova::Builder::where_not_binary` · framework/src/eloquent/builder.rs:1477
  - [ ] fn `suprnova::Builder::or_filter_not_binary` · framework/src/eloquent/builder.rs:1483
  - [ ] fn `suprnova::Builder::or_where_not_binary` · framework/src/eloquent/builder.rs:1490
  - [ ] fn `suprnova::Builder::filter_date` · framework/src/eloquent/builder.rs:1499
  - [ ] fn `suprnova::Builder::where_date` · framework/src/eloquent/builder.rs:1510
  - [ ] fn `suprnova::Builder::filter_day` · framework/src/eloquent/builder.rs:1516
  - [ ] fn `suprnova::Builder::where_day` · framework/src/eloquent/builder.rs:1527
  - [ ] fn `suprnova::Builder::filter_month` · framework/src/eloquent/builder.rs:1533
  - [ ] fn `suprnova::Builder::where_month` · framework/src/eloquent/builder.rs:1544
  - [ ] fn `suprnova::Builder::filter_year` · framework/src/eloquent/builder.rs:1550
  - [ ] fn `suprnova::Builder::where_year` · framework/src/eloquent/builder.rs:1561
  - [ ] fn `suprnova::Builder::filter_time` · framework/src/eloquent/builder.rs:1567
  - [ ] fn `suprnova::Builder::where_time` · framework/src/eloquent/builder.rs:1578
  - [ ] fn `suprnova::Builder::filter_json_contains` · framework/src/eloquent/builder.rs:1588
  - [ ] fn `suprnova::Builder::where_json_contains` · framework/src/eloquent/builder.rs:1596
  - [ ] fn `suprnova::Builder::filter_json_length` · framework/src/eloquent/builder.rs:1603
  - [ ] fn `suprnova::Builder::where_json_length` · framework/src/eloquent/builder.rs:1611
  - [ ] fn `suprnova::Builder::filter_column` · framework/src/eloquent/builder.rs:1619
  - [ ] fn `suprnova::Builder::where_column` · framework/src/eloquent/builder.rs:1627
  - [ ] fn `suprnova::Builder::filter_raw` · framework/src/eloquent/builder.rs:1646
  - [ ] fn `suprnova::Builder::where_raw` · framework/src/eloquent/builder.rs:1653
  - [ ] fn `suprnova::Builder::order_by` · framework/src/eloquent/builder.rs:1660
  - [ ] fn `suprnova::Builder::order_by_desc` · framework/src/eloquent/builder.rs:1666
  - [ ] fn `suprnova::Builder::order_by_asc` · framework/src/eloquent/builder.rs:1671
  - [ ] fn `suprnova::Builder::order_by_raw` · framework/src/eloquent/builder.rs:1685
  - [ ] fn `suprnova::Builder::in_random_order` · framework/src/eloquent/builder.rs:1693
  - [ ] fn `suprnova::Builder::in_order_of` · framework/src/eloquent/builder.rs:1722
  - [ ] fn `suprnova::Builder::group_by` · framework/src/eloquent/builder.rs:1737
  - [ ] fn `suprnova::Builder::having` · framework/src/eloquent/builder.rs:1743
  - [ ] fn `suprnova::Builder::having_op` · framework/src/eloquent/builder.rs:1751
  - [ ] fn `suprnova::Builder::limit` · framework/src/eloquent/builder.rs:1761
  - [ ] fn `suprnova::Builder::offset` · framework/src/eloquent/builder.rs:1767
  - [ ] fn `suprnova::Builder::take` · framework/src/eloquent/builder.rs:1773
  - [ ] fn `suprnova::Builder::skip` · framework/src/eloquent/builder.rs:1778
  - [ ] fn `suprnova::Builder::distinct` · framework/src/eloquent/builder.rs:1783
  - [ ] fn `suprnova::Builder::select` · framework/src/eloquent/builder.rs:1790
  - [ ] fn `suprnova::Builder::add_select` · framework/src/eloquent/builder.rs:1801
  - [ ] fn `suprnova::Builder::select_raw` · framework/src/eloquent/builder.rs:1817
  - [ ] fn `suprnova::Builder::union` · framework/src/eloquent/builder.rs:1824
  - [ ] fn `suprnova::Builder::union_all` · framework/src/eloquent/builder.rs:1830
  - [ ] fn `suprnova::Builder::lock_for_update` · framework/src/eloquent/builder.rs:1863
  - [ ] fn `suprnova::Builder::shared_lock` · framework/src/eloquent/builder.rs:1881
  - [ ] fn `suprnova::Builder::with_casts` · framework/src/eloquent/builder.rs:1898
  - [ ] fn `suprnova::Builder::has` · framework/src/eloquent/builder.rs:3010
  - [ ] fn `suprnova::Builder::has_count` · framework/src/eloquent/builder.rs:3024
  - [ ] fn `suprnova::Builder::or_has` · framework/src/eloquent/builder.rs:3040
  - [ ] fn `suprnova::Builder::doesnt_have` · framework/src/eloquent/builder.rs:3050
  - [ ] fn `suprnova::Builder::or_doesnt_have` · framework/src/eloquent/builder.rs:3058
  - [ ] fn `suprnova::Builder::where_has` · framework/src/eloquent/builder.rs:3076
  - [ ] fn `suprnova::Builder::or_where_has` · framework/src/eloquent/builder.rs:3097
  - [ ] fn `suprnova::Builder::where_doesnt_have` · framework/src/eloquent/builder.rs:3119
  - [ ] fn `suprnova::Builder::or_where_doesnt_have` · framework/src/eloquent/builder.rs:3140
  - [ ] fn `suprnova::Builder::where_relation` · framework/src/eloquent/builder.rs:3165
  - [ ] fn `suprnova::Builder::where_relation_op` · framework/src/eloquent/builder.rs:3187
  - [ ] fn `suprnova::Builder::or_where_relation` · framework/src/eloquent/builder.rs:3209
  - [ ] fn `suprnova::Builder::where_belongs_to` · framework/src/eloquent/builder.rs:3238
  - [ ] fn `suprnova::Builder::where_key` · framework/src/eloquent/builder.rs:3255
  - [ ] fn `suprnova::Builder::filter_key` · framework/src/eloquent/builder.rs:3261
  - [ ] fn `suprnova::Builder::where_key_not` · framework/src/eloquent/builder.rs:3267
  - [ ] fn `suprnova::Builder::filter_key_not` · framework/src/eloquent/builder.rs:3273
  - [ ] fn `suprnova::Builder::or_where_key` · framework/src/eloquent/builder.rs:3281
  - [ ] fn `suprnova::Builder::or_filter_key` · framework/src/eloquent/builder.rs:3287
  - [ ] fn `suprnova::Builder::or_where_key_not` · framework/src/eloquent/builder.rs:3296
  - [ ] fn `suprnova::Builder::or_filter_key_not` · framework/src/eloquent/builder.rs:3303
  - [ ] fn `suprnova::Builder::latest` · framework/src/eloquent/builder.rs:3309
  - [ ] fn `suprnova::Builder::latest_by` · framework/src/eloquent/builder.rs:3314
  - [ ] fn `suprnova::Builder::oldest` · framework/src/eloquent/builder.rs:3320
  - [ ] fn `suprnova::Builder::oldest_by` · framework/src/eloquent/builder.rs:3325
  - [ ] fn `suprnova::Builder::without` · framework/src/eloquent/builder.rs:3333
  - [ ] fn `suprnova::Builder::with_only` · framework/src/eloquent/builder.rs:3354
  - [ ] fn `suprnova::Builder::qualify_column` · framework/src/eloquent/builder.rs:3371
  - [ ] fn `suprnova::Builder::qualify_columns` · framework/src/eloquent/builder.rs:3377
  - [ ] fn `suprnova::Builder::to_sql` · framework/src/eloquent/builder.rs:3391
  - [ ] fn `suprnova::Builder::to_sql_with_bindings` · framework/src/eloquent/builder.rs:3407
  - [ ] fn `suprnova::Builder::to_sql_for` · framework/src/eloquent/builder.rs:3419
  - [ ] fn `suprnova::Builder::to_sql_with_bindings_for` · framework/src/eloquent/builder.rs:3434
  - [ ] fn `suprnova::Builder::try_to_sql_with_bindings_for` · framework/src/eloquent/builder.rs:3450
  - [ ] fn `suprnova::Builder::dump` · framework/src/eloquent/builder.rs:3478
  - [ ] fn `suprnova::Builder::dd` · framework/src/eloquent/builder.rs:3522
  - [ ] fn `suprnova::Builder::to_delete_sql_with_bindings_for` · framework/src/eloquent/builder.rs:3553
  - [ ] fn `suprnova::Builder::get` · framework/src/eloquent/builder.rs:3682
  - [ ] fn `suprnova::Builder::first` · framework/src/eloquent/builder.rs:3809
  - [ ] fn `suprnova::Builder::first_or_fail` · framework/src/eloquent/builder.rs:3819
  - [ ] fn `suprnova::Builder::sole` · framework/src/eloquent/builder.rs:3832
  - [ ] fn `suprnova::Builder::sole_value` · framework/src/eloquent/builder.rs:3849
  - [ ] fn `suprnova::Builder::value_or_fail` · framework/src/eloquent/builder.rs:3880
  - [ ] fn `suprnova::Builder::exists` · framework/src/eloquent/builder.rs:3894
  - [ ] fn `suprnova::Builder::doesnt_exist` · framework/src/eloquent/builder.rs:3899
  - [ ] fn `suprnova::Builder::count` · framework/src/eloquent/builder.rs:3904
  - [ ] fn `suprnova::Builder::paginate` · framework/src/eloquent/builder.rs:3938
  - [ ] fn `suprnova::Builder::paginate_using` · framework/src/eloquent/builder.rs:3957
  - [ ] fn `suprnova::Builder::simple_paginate` · framework/src/eloquent/builder.rs:4021
  - [ ] fn `suprnova::Builder::cursor_paginate` · framework/src/eloquent/builder.rs:4065
  - [ ] fn `suprnova::Builder::chunk` · framework/src/eloquent/builder.rs:4181
  - [ ] fn `suprnova::Builder::chunk_by_id` · framework/src/eloquent/builder.rs:4250
  - [ ] fn `suprnova::Builder::chunk_map` · framework/src/eloquent/builder.rs:4319
  - [ ] fn `suprnova::Builder::each` · framework/src/eloquent/builder.rs:4373
  - [ ] fn `suprnova::Builder::lazy` · framework/src/eloquent/builder.rs:4432
  - [ ] fn `suprnova::Builder::lazy_by_id` · framework/src/eloquent/builder.rs:4444
  - [ ] fn `suprnova::Builder::cursor` · framework/src/eloquent/builder.rs:4493
  - [ ] fn `suprnova::Builder::sum` · framework/src/eloquent/builder.rs:4508
  - [ ] fn `suprnova::Builder::avg` · framework/src/eloquent/builder.rs:4520
  - [ ] fn `suprnova::Builder::min` · framework/src/eloquent/builder.rs:4531
  - [ ] fn `suprnova::Builder::max` · framework/src/eloquent/builder.rs:4542
  - [ ] fn `suprnova::Builder::value` · framework/src/eloquent/builder.rs:4553
  - [ ] fn `suprnova::Builder::pluck` · framework/src/eloquent/builder.rs:4576
  - [ ] fn `suprnova::Builder::pluck_keyed` · framework/src/eloquent/builder.rs:4600
  - [ ] fn `suprnova::Builder::model_keys` · framework/src/eloquent/builder.rs:4645
  - [ ] fn `suprnova::Builder::update_all` · framework/src/eloquent/builder.rs:4739
  - [ ] fn `suprnova::Builder::delete_all` · framework/src/eloquent/builder.rs:4801
  - [ ] fn `suprnova::Builder::increment_each` · framework/src/eloquent/builder.rs:4831
  - [ ] fn `suprnova::Builder::decrement_each` · framework/src/eloquent/builder.rs:4892
  - [ ] fn `suprnova::Builder::upsert` · framework/src/eloquent/builder.rs:4915
  - [ ] fn `suprnova::Builder::with_trashed` · framework/src/eloquent/soft_deletes.rs:109
  - [ ] fn `suprnova::Builder::only_trashed` · framework/src/eloquent/soft_deletes.rs:120
- [ ] enum `suprnova::Direction` · framework/src/eloquent/builder.rs:137 (also `suprnova::eloquent::Direction`, `suprnova::eloquent::builder::Direction`)
  - Variants: `Asc`, `Desc`
- [ ] trait `suprnova::IntoColumn` · framework/src/eloquent/builder.rs:92 (also `suprnova::eloquent::IntoColumn`, `suprnova::eloquent::builder::IntoColumn`)
  - Implemented here by: `String`, `features::entity::Column`, `payments::entities::customer::Column`, `payments::entities::payment_method::Column`, `payments::entities::subscription::Column`, `payments::entities::subscription_item::Column`, `payments::entities::transaction::Column`, `payments::entities::webhook_event::Column`, `rbac::entity::ModelPermissionColumn`, `rbac::entity::ModelRoleColumn`, `rbac::entity::PermissionColumn`, `rbac::entity::RoleColumn`, `rbac::entity::RolePermissionColumn`
  - [ ] fn `suprnova::IntoColumn::col_name` · framework/src/eloquent/builder.rs:96 (required)
- [ ] trait `suprnova::IntoVal` · framework/src/eloquent/builder.rs:121 (also `suprnova::eloquent::IntoVal`, `suprnova::eloquent::builder::IntoVal`)
  - [ ] fn `suprnova::IntoVal::into_val` · framework/src/eloquent/builder.rs:123 (required)

### `suprnova::eloquent::casts`

- [ ] trait `suprnova::Cast` · framework/src/eloquent/casts/mod.rs:37 (also `suprnova::eloquent::Cast`, `suprnova::eloquent::casts::Cast`)
  - Implemented here by: `AsArray`, `AsArrayObject`, `AsBool`, `AsCollection`, `AsDate`, `AsDateTime`, `AsDecimal`, `AsEncrypted`, `AsEncryptedArray`, `AsEncryptedCollection`, `AsEncryptedObject`, `AsEnum`, `AsFloat`, `AsHashed`, `AsImmutableDate`, `AsImmutableDateTime`, `AsInt`, `AsJson`, `AsObject`, `AsOptionalDateTime`, `AsString`, `AsTimestamp`
  - [ ] type `suprnova::Cast::Runtime` · framework/src/eloquent/casts/mod.rs:39
  - [ ] type `suprnova::Cast::Storage` · framework/src/eloquent/casts/mod.rs:41
  - [ ] fn `suprnova::Cast::to_storage` · framework/src/eloquent/casts/mod.rs:44 (required)
  - [ ] fn `suprnova::Cast::from_storage` · framework/src/eloquent/casts/mod.rs:46 (required)
- [ ] trait `suprnova::DynCast` · framework/src/eloquent/casts/mod.rs:64 (also `suprnova::eloquent::DynCast`, `suprnova::eloquent::casts::DynCast`)
  - [ ] fn `suprnova::DynCast::from_storage_json` · framework/src/eloquent/casts/mod.rs:67 (required)
  - [ ] fn `suprnova::DynCast::to_storage_json` · framework/src/eloquent/casts/mod.rs:73 (required)
- [ ] trait `suprnova::IntoDynCast` · framework/src/eloquent/casts/mod.rs:79 (also `suprnova::eloquent::IntoDynCast`, `suprnova::eloquent::casts::IntoDynCast`)
  - Implemented here by: `AsArray`, `AsArrayObject`, `AsBool`, `AsCollection`, `AsDate`, `AsDateTime`, `AsDecimal`, `AsEncrypted`, `AsEncryptedArray`, `AsEncryptedCollection`, `AsEncryptedObject`, `AsEnum`, `AsFloat`, `AsHashed`, `AsImmutableDate`, `AsImmutableDateTime`, `AsInt`, `AsJson`, `AsObject`, `AsOptionalDateTime`, `AsString`, `AsTimestamp`
  - [ ] fn `suprnova::IntoDynCast::into_dyn` · framework/src/eloquent/casts/mod.rs:81 (required)

### `suprnova::eloquent::casts::encrypted`

- [ ] struct `suprnova::AsEncrypted` · framework/src/eloquent/casts/encrypted.rs:41 (also `suprnova::eloquent::AsEncrypted`, `suprnova::eloquent::casts::AsEncrypted`, `suprnova::eloquent::casts::encrypted::AsEncrypted`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsEncryptedArray` · framework/src/eloquent/casts/encrypted.rs:107 (also `suprnova::eloquent::AsEncryptedArray`, `suprnova::eloquent::casts::AsEncryptedArray`, `suprnova::eloquent::casts::encrypted::AsEncryptedArray`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsEncryptedCollection` · framework/src/eloquent/casts/encrypted.rs:261 (also `suprnova::eloquent::AsEncryptedCollection`, `suprnova::eloquent::casts::AsEncryptedCollection`, `suprnova::eloquent::casts::encrypted::AsEncryptedCollection`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsEncryptedObject` · framework/src/eloquent/casts/encrypted.rs:183 (also `suprnova::eloquent::AsEncryptedObject`, `suprnova::eloquent::casts::AsEncryptedObject`, `suprnova::eloquent::casts::encrypted::AsEncryptedObject`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsHashed` · framework/src/eloquent/casts/encrypted.rs:348 (also `suprnova::eloquent::AsHashed`, `suprnova::eloquent::casts::AsHashed`, `suprnova::eloquent::casts::encrypted::AsHashed`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`

### `suprnova::eloquent::casts::enum_cast`

- [ ] struct `suprnova::AsEnum` · framework/src/eloquent/casts/enum_cast.rs:31 (also `suprnova::eloquent::AsEnum`, `suprnova::eloquent::casts::AsEnum`, `suprnova::eloquent::casts::enum_cast::AsEnum`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`

### `suprnova::eloquent::casts::primitive`

- [ ] struct `suprnova::AsBool` · framework/src/eloquent/casts/primitive.rs:33 (also `suprnova::eloquent::AsBool`, `suprnova::eloquent::casts::AsBool`, `suprnova::eloquent::casts::primitive::AsBool`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsDecimal` · framework/src/eloquent/casts/primitive.rs:237 (also `suprnova::eloquent::AsDecimal`, `suprnova::eloquent::casts::AsDecimal`, `suprnova::eloquent::casts::primitive::AsDecimal`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsFloat` · framework/src/eloquent/casts/primitive.rs:150 (also `suprnova::eloquent::AsFloat`, `suprnova::eloquent::casts::AsFloat`, `suprnova::eloquent::casts::primitive::AsFloat`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsInt` · framework/src/eloquent/casts/primitive.rs:97 (also `suprnova::eloquent::AsInt`, `suprnova::eloquent::casts::AsInt`, `suprnova::eloquent::casts::primitive::AsInt`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsString` · framework/src/eloquent/casts/primitive.rs:195 (also `suprnova::eloquent::AsString`, `suprnova::eloquent::casts::AsString`, `suprnova::eloquent::casts::primitive::AsString`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`

### `suprnova::eloquent::casts::structured`

- [ ] struct `suprnova::AsArray` · framework/src/eloquent/casts/structured.rs:39 (also `suprnova::eloquent::AsArray`, `suprnova::eloquent::casts::AsArray`, `suprnova::eloquent::casts::structured::AsArray`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsArrayObject` · framework/src/eloquent/casts/structured.rs:306 (also `suprnova::eloquent::AsArrayObject`, `suprnova::eloquent::casts::AsArrayObject`, `suprnova::eloquent::casts::structured::AsArrayObject`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsCollection` · framework/src/eloquent/casts/structured.rs:173 (also `suprnova::eloquent::AsCollection`, `suprnova::eloquent::casts::AsCollection`, `suprnova::eloquent::casts::structured::AsCollection`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsJson` · framework/src/eloquent/casts/structured.rs:239 (also `suprnova::eloquent::AsJson`, `suprnova::eloquent::casts::AsJson`, `suprnova::eloquent::casts::structured::AsJson`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsObject` · framework/src/eloquent/casts/structured.rs:108 (also `suprnova::eloquent::AsObject`, `suprnova::eloquent::casts::AsObject`, `suprnova::eloquent::casts::structured::AsObject`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`

### `suprnova::eloquent::casts::temporal`

- [ ] struct `suprnova::AsDate` · framework/src/eloquent/casts/temporal.rs:33 (also `suprnova::eloquent::AsDate`, `suprnova::eloquent::casts::AsDate`, `suprnova::eloquent::casts::temporal::AsDate`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsDateTime` · framework/src/eloquent/casts/temporal.rs:88 (also `suprnova::eloquent::AsDateTime`, `suprnova::eloquent::casts::AsDateTime`, `suprnova::eloquent::casts::temporal::AsDateTime`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsImmutableDate` · framework/src/eloquent/casts/temporal.rs:154 (also `suprnova::eloquent::AsImmutableDate`, `suprnova::eloquent::casts::AsImmutableDate`, `suprnova::eloquent::casts::temporal::AsImmutableDate`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsImmutableDateTime` · framework/src/eloquent/casts/temporal.rs:181 (also `suprnova::eloquent::AsImmutableDateTime`, `suprnova::eloquent::casts::AsImmutableDateTime`, `suprnova::eloquent::casts::temporal::AsImmutableDateTime`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsOptionalDateTime` · framework/src/eloquent/casts/temporal.rs:215 (also `suprnova::eloquent::AsOptionalDateTime`, `suprnova::eloquent::casts::AsOptionalDateTime`, `suprnova::eloquent::casts::temporal::AsOptionalDateTime`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`
- [ ] struct `suprnova::AsTimestamp` · framework/src/eloquent/casts/temporal.rs:275 (also `suprnova::eloquent::AsTimestamp`, `suprnova::eloquent::casts::AsTimestamp`, `suprnova::eloquent::casts::temporal::AsTimestamp`)
  - Implements: `suprnova::Cast`, `suprnova::IntoDynCast`

### `suprnova::eloquent::collection`

- [ ] struct `suprnova::Collection` · framework/src/eloquent/collection.rs:42 (also `suprnova::eloquent::Collection`, `suprnova::eloquent::collection::Collection`)
  - Public tuple fields: 1
  - [ ] fn `suprnova::Collection::new` · framework/src/eloquent/collection.rs:46
  - [ ] fn `suprnova::Collection::into_vec` · framework/src/eloquent/collection.rs:51
  - [ ] fn `suprnova::Collection::as_slice` · framework/src/eloquent/collection.rs:58
  - [ ] fn `suprnova::Collection::from_vec` · framework/src/eloquent/collection.rs:72
  - [ ] fn `suprnova::Collection::len` · framework/src/eloquent/collection.rs:78
  - [ ] fn `suprnova::Collection::is_empty` · framework/src/eloquent/collection.rs:83
  - [ ] fn `suprnova::Collection::is_not_empty` · framework/src/eloquent/collection.rs:89
  - [ ] fn `suprnova::Collection::first` · framework/src/eloquent/collection.rs:94
  - [ ] fn `suprnova::Collection::last` · framework/src/eloquent/collection.rs:99
  - [ ] fn `suprnova::Collection::first_where` · framework/src/eloquent/collection.rs:105
  - [ ] fn `suprnova::Collection::last_where` · framework/src/eloquent/collection.rs:114
  - [ ] fn `suprnova::Collection::each` · framework/src/eloquent/collection.rs:129
  - [ ] fn `suprnova::Collection::map` · framework/src/eloquent/collection.rs:140
  - [ ] fn `suprnova::Collection::map_to_map` · framework/src/eloquent/collection.rs:149
  - [ ] fn `suprnova::Collection::filter` · framework/src/eloquent/collection.rs:158
  - [ ] fn `suprnova::Collection::reject` · framework/src/eloquent/collection.rs:167
  - [ ] fn `suprnova::Collection::reduce` · framework/src/eloquent/collection.rs:175
  - [ ] fn `suprnova::Collection::group_by_with` · framework/src/eloquent/collection.rs:185
  - [ ] fn `suprnova::Collection::key_by_with` · framework/src/eloquent/collection.rs:202
  - [ ] fn `suprnova::Collection::pluck_by` · framework/src/eloquent/collection.rs:220
  - [ ] fn `suprnova::Collection::sort_with` · framework/src/eloquent/collection.rs:229
  - [ ] fn `suprnova::Collection::unique` · framework/src/eloquent/collection.rs:240
  - [ ] fn `suprnova::Collection::unique_by` · framework/src/eloquent/collection.rs:255
  - [ ] fn `suprnova::Collection::contains_where` · framework/src/eloquent/collection.rs:267
  - [ ] fn `suprnova::Collection::chunk` · framework/src/eloquent/collection.rs:278
  - [ ] fn `suprnova::Collection::take` · framework/src/eloquent/collection.rs:293
  - [ ] fn `suprnova::Collection::skip` · framework/src/eloquent/collection.rs:299
  - [ ] fn `suprnova::Collection::slice` · framework/src/eloquent/collection.rs:305
  - [ ] fn `suprnova::Collection::reverse` · framework/src/eloquent/collection.rs:310
  - [ ] fn `suprnova::Collection::shuffle` · framework/src/eloquent/collection.rs:316
  - [ ] fn `suprnova::Collection::random` · framework/src/eloquent/collection.rs:324
  - [ ] fn `suprnova::Collection::random_n` · framework/src/eloquent/collection.rs:333
  - [ ] fn `suprnova::Collection::concat` · framework/src/eloquent/collection.rs:345
  - [ ] fn `suprnova::Collection::merge` · framework/src/eloquent/collection.rs:351
  - [ ] fn `suprnova::Collection::diff` · framework/src/eloquent/collection.rs:357
  - [ ] fn `suprnova::Collection::intersect` · framework/src/eloquent/collection.rs:371
  - [ ] fn `suprnova::Collection::load` · framework/src/eloquent/collection.rs:482
  - [ ] fn `suprnova::Collection::load_missing` · framework/src/eloquent/collection.rs:522
  - [ ] fn `suprnova::Collection::pluck` · framework/src/eloquent/collection.rs:606
  - [ ] fn `suprnova::Collection::model_keys` · framework/src/eloquent/collection.rs:635
  - [ ] fn `suprnova::Collection::pluck_keyed` · framework/src/eloquent/collection.rs:653
  - [ ] fn `suprnova::Collection::group_by` · framework/src/eloquent/collection.rs:681
  - [ ] fn `suprnova::Collection::key_by` · framework/src/eloquent/collection.rs:699
  - [ ] fn `suprnova::Collection::sort_by` · framework/src/eloquent/collection.rs:724
  - [ ] fn `suprnova::Collection::sort_by_desc` · framework/src/eloquent/collection.rs:735
  - [ ] fn `suprnova::Collection::where_eq` · framework/src/eloquent/collection.rs:746
  - [ ] fn `suprnova::Collection::where_in` · framework/src/eloquent/collection.rs:757
  - [ ] fn `suprnova::Collection::where_not_in` · framework/src/eloquent/collection.rs:773
  - [ ] fn `suprnova::Collection::sum` · framework/src/eloquent/collection.rs:790
  - [ ] fn `suprnova::Collection::avg` · framework/src/eloquent/collection.rs:808
  - [ ] fn `suprnova::Collection::min` · framework/src/eloquent/collection.rs:829
  - [ ] fn `suprnova::Collection::max` · framework/src/eloquent/collection.rs:852
  - [ ] fn `suprnova::Collection::to_array` · framework/src/eloquent/collection.rs:883
  - [ ] fn `suprnova::Collection::to_json` · framework/src/eloquent/collection.rs:900
  - [ ] fn `suprnova::Collection::try_to_json` · framework/src/eloquent/collection.rs:908

### `suprnova::eloquent::console::prune`

- [ ] struct `suprnova::eloquent::console::prune::PruneArgs` · framework/src/eloquent/console/prune.rs:28
  - Public fields: `model`, `pretend`
  - Implements: `suprnova::TypedCommand`

### `suprnova::eloquent::events`

- [ ] fn `suprnova::dispatch_after` · framework/src/eloquent/events.rs:175 (also `suprnova::eloquent::events::dispatch_after`)
- [ ] fn `suprnova::dispatch_cancellable` · framework/src/eloquent/events.rs:145 (also `suprnova::eloquent::events::dispatch_cancellable`)
- [ ] fn `suprnova::listen_cancellable` · framework/src/eloquent/events.rs:214 (also `suprnova::eloquent::events::listen_cancellable`)
- [ ] enum `suprnova::EventResult` · framework/src/eloquent/events.rs:45 (also `suprnova::eloquent::events::EventResult`)
  - Variants: `Ok`, `Cancel`
  - [ ] fn `suprnova::EventResult::ok` · framework/src/eloquent/events.rs:55
  - [ ] fn `suprnova::EventResult::cancel` · framework/src/eloquent/events.rs:60
  - [ ] fn `suprnova::EventResult::is_cancelled` · framework/src/eloquent/events.rs:65
- [ ] trait `suprnova::CancellableListener` · framework/src/eloquent/events.rs:76 (also `suprnova::eloquent::events::CancellableListener`)
  - [ ] fn `suprnova::CancellableListener::handle` · framework/src/eloquent/events.rs:79 (required)
- [ ] trait `suprnova::ModelEventHooks` · framework/src/eloquent/events.rs:286 (also `suprnova::eloquent::events::ModelEventHooks`)
  - Implemented here by: `features::entity::Feature`, `payments::entities::customer::Customer`, `payments::entities::payment_method::PaymentMethod`, `payments::entities::subscription::Subscription`, `payments::entities::subscription_item::SubscriptionItem`, `payments::entities::transaction::Transaction`, `payments::entities::webhook_event::WebhookEvent`, `rbac::entity::ModelPermission`, `rbac::entity::ModelRole`, `rbac::entity::Permission`, `rbac::entity::Role`, `rbac::entity::RolePermission`
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_creating` · framework/src/eloquent/events.rs:288 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_saving` · framework/src/eloquent/events.rs:292 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_created` · framework/src/eloquent/events.rs:297 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_saved` · framework/src/eloquent/events.rs:299 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_updating` · framework/src/eloquent/events.rs:301 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_updated` · framework/src/eloquent/events.rs:306 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_deleting` · framework/src/eloquent/events.rs:308 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_deleted` · framework/src/eloquent/events.rs:310 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_trashed` · framework/src/eloquent/events.rs:312 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_restoring` · framework/src/eloquent/events.rs:314 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_restored` · framework/src/eloquent/events.rs:316 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_force_deleting` · framework/src/eloquent/events.rs:318 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_force_deleted` · framework/src/eloquent/events.rs:320 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_replicating` · framework/src/eloquent/events.rs:322 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_retrieving` · framework/src/eloquent/events.rs:327 (required)
  - [ ] fn `suprnova::ModelEventHooks::__dispatch_retrieved` · framework/src/eloquent/events.rs:329 (required)

### `suprnova::eloquent::fillable`

- [ ] fn `suprnova::prevent_silently_discarding_attributes` · framework/src/eloquent/fillable.rs:66 (also `suprnova::eloquent::fillable::prevent_silently_discarding_attributes`, `suprnova::eloquent::prevent_silently_discarding_attributes`)
- [ ] fn `suprnova::preventing_silently_discarding_attributes` · framework/src/eloquent/fillable.rs:72 (also `suprnova::eloquent::fillable::preventing_silently_discarding_attributes`, `suprnova::eloquent::preventing_silently_discarding_attributes`)
- [ ] fn `suprnova::unguarded` · framework/src/eloquent/fillable.rs:262 (also `suprnova::eloquent::fillable::unguarded`, `suprnova::eloquent::unguarded`)
- [ ] struct `suprnova::Fillable` · framework/src/eloquent/fillable.rs:79 (also `suprnova::eloquent::Fillable`, `suprnova::eloquent::fillable::Fillable`)
  - [ ] fn `suprnova::Fillable::allow_all` · framework/src/eloquent/fillable.rs:98
  - [ ] fn `suprnova::Fillable::guarded_default` · framework/src/eloquent/fillable.rs:109
  - [ ] fn `suprnova::Fillable::fillable` · framework/src/eloquent/fillable.rs:120
  - [ ] fn `suprnova::Fillable::guarded` · framework/src/eloquent/fillable.rs:127
  - [ ] fn `suprnova::Fillable::apply` · framework/src/eloquent/fillable.rs:141
  - [ ] fn `suprnova::Fillable::apply_checked` · framework/src/eloquent/fillable.rs:159

### `suprnova::eloquent::lazy`

- [ ] struct `suprnova::LazyCollection` · framework/src/eloquent/lazy.rs:57 (also `suprnova::eloquent::LazyCollection`, `suprnova::eloquent::lazy::LazyCollection`)
  - [ ] fn `suprnova::LazyCollection::boxed` · framework/src/eloquent/lazy.rs:67
  - [ ] fn `suprnova::LazyCollection::next` · framework/src/eloquent/lazy.rs:76

### `suprnova::eloquent::model`

- [ ] fn `suprnova::eloquent::model::json_value_to_sea_value` · framework/src/eloquent/model.rs:1469
- [ ] fn `suprnova::eloquent::model::sea_value_to_json_loose` · framework/src/eloquent/model.rs:1501
- [ ] trait `suprnova::FirstOrCreate` · framework/src/eloquent/model.rs:1567 (also `suprnova::eloquent::FirstOrCreate`, `suprnova::eloquent::model::FirstOrCreate`)
  - Implemented here by: `features::entity::Feature`, `payments::entities::customer::Customer`, `payments::entities::payment_method::PaymentMethod`, `payments::entities::subscription::Subscription`, `payments::entities::subscription_item::SubscriptionItem`, `payments::entities::transaction::Transaction`, `payments::entities::webhook_event::WebhookEvent`, `rbac::entity::ModelPermission`, `rbac::entity::ModelRole`, `rbac::entity::Permission`, `rbac::entity::Role`, `rbac::entity::RolePermission`
  - [ ] fn `suprnova::FirstOrCreate::first_or_create` · framework/src/eloquent/model.rs:1582 (provided)
  - [ ] fn `suprnova::FirstOrCreate::update_or_create` · framework/src/eloquent/model.rs:1593 (provided)
  - [ ] fn `suprnova::FirstOrCreate::first_or_new` · framework/src/eloquent/model.rs:1603 (provided)
  - [ ] fn `suprnova::FirstOrCreate::first_or` · framework/src/eloquent/model.rs:1612 (provided)
  - [ ] fn `suprnova::FirstOrCreate::find_or` · framework/src/eloquent/model.rs:1625 (provided)
  - [ ] fn `suprnova::FirstOrCreate::find_or_new` · framework/src/eloquent/model.rs:1641 (provided)
  - [ ] fn `suprnova::FirstOrCreate::create_or_first` · framework/src/eloquent/model.rs:1661 (provided)
  - [ ] fn `suprnova::FirstOrCreate::from_attrs_unsaved` · framework/src/eloquent/model.rs:1677 (required)
- [ ] trait `suprnova::Model` · framework/src/eloquent/model.rs:68 (also `suprnova::eloquent::Model`, `suprnova::eloquent::model::Model`)
  - Implemented here by: `features::entity::Feature`, `payments::entities::customer::Customer`, `payments::entities::payment_method::PaymentMethod`, `payments::entities::subscription::Subscription`, `payments::entities::subscription_item::SubscriptionItem`, `payments::entities::transaction::Transaction`, `payments::entities::webhook_event::WebhookEvent`, `rbac::entity::ModelPermission`, `rbac::entity::ModelRole`, `rbac::entity::Permission`, `rbac::entity::Role`, `rbac::entity::RolePermission`
  - [ ] fn `suprnova::Model::primary_key_name` · framework/src/eloquent/model.rs:91 (provided)
  - [ ] fn `suprnova::Model::qualified_key_name` · framework/src/eloquent/model.rs:103 (provided)
  - [ ] fn `suprnova::Model::fillable_filter` · framework/src/eloquent/model.rs:110 (required)
  - [ ] fn `suprnova::Model::try_from_storage` · framework/src/eloquent/model.rs:132 (provided)
  - [ ] fn `suprnova::Model::try_into_storage` · framework/src/eloquent/model.rs:147 (provided)
  - [ ] fn `suprnova::Model::field_value` · framework/src/eloquent/model.rs:167 (provided)
  - [ ] fn `suprnova::Model::to_array` · framework/src/eloquent/model.rs:188 (provided)
  - [ ] fn `suprnova::Model::to_json` · framework/src/eloquent/model.rs:200 (provided)
  - [ ] fn `suprnova::Model::find` · framework/src/eloquent/model.rs:227 (provided)
  - [ ] fn `suprnova::Model::find_or_fail` · framework/src/eloquent/model.rs:285 (provided)
  - [ ] fn `suprnova::Model::find_many` · framework/src/eloquent/model.rs:309 (provided)
  - [ ] fn `suprnova::Model::all` · framework/src/eloquent/model.rs:391 (provided)
  - [ ] fn `suprnova::Model::query` · framework/src/eloquent/model.rs:427 (provided)
  - [ ] fn `suprnova::Model::create` · framework/src/eloquent/model.rs:449 (provided)
  - [ ] fn `suprnova::Model::save` · framework/src/eloquent/model.rs:507 (provided)
  - [ ] fn `suprnova::Model::update` · framework/src/eloquent/model.rs:561 (provided)
  - [ ] fn `suprnova::Model::delete` · framework/src/eloquent/model.rs:611 (provided)
  - [ ] fn `suprnova::Model::force_delete` · framework/src/eloquent/model.rs:643 (provided)
  - [ ] fn `suprnova::Model::touch_owners` · framework/src/eloquent/model.rs:683 (provided)
  - [ ] fn `suprnova::Model::touch_owners_with_tx` · framework/src/eloquent/model.rs:709 (provided)
  - [ ] fn `suprnova::Model::save_with_tx` · framework/src/eloquent/model.rs:852 (provided)
  - [ ] fn `suprnova::Model::update_with_tx` · framework/src/eloquent/model.rs:887 (provided)
  - [ ] fn `suprnova::Model::delete_with_tx` · framework/src/eloquent/model.rs:923 (provided)
  - [ ] fn `suprnova::Model::create_with_tx` · framework/src/eloquent/model.rs:948 (provided)
  - [ ] fn `suprnova::Model::force_delete_with_tx` · framework/src/eloquent/model.rs:976 (provided)
  - [ ] fn `suprnova::Model::refresh` · framework/src/eloquent/model.rs:1000 (provided)
  - [ ] fn `suprnova::Model::refresh_for_update` · framework/src/eloquent/model.rs:1042 (provided)
  - [ ] fn `suprnova::Model::fresh` · framework/src/eloquent/model.rs:1059 (provided)
  - [ ] fn `suprnova::Model::replicate` · framework/src/eloquent/model.rs:1086 (provided)
  - [ ] fn `suprnova::Model::replicate_except` · framework/src/eloquent/model.rs:1104 (provided)
  - [ ] fn `suprnova::Model::replicate_into` · framework/src/eloquent/model.rs:1148 (provided)
  - [ ] fn `suprnova::Model::increment` · framework/src/eloquent/model.rs:1186 (provided)
  - [ ] fn `suprnova::Model::decrement` · framework/src/eloquent/model.rs:1223 (provided)
  - [ ] fn `suprnova::Model::destroy` · framework/src/eloquent/model.rs:1237 (provided)
  - [ ] fn `suprnova::Model::force_destroy` · framework/src/eloquent/model.rs:1256 (provided)
  - [ ] fn `suprnova::Model::is` · framework/src/eloquent/model.rs:1275 (provided)
  - [ ] fn `suprnova::Model::is_not` · framework/src/eloquent/model.rs:1280 (provided)
  - [ ] fn `suprnova::Model::to_array_except` · framework/src/eloquent/model.rs:1293 (provided)
  - [ ] fn `suprnova::Model::to_array_only` · framework/src/eloquent/model.rs:1307 (provided)
  - [ ] fn `suprnova::Model::save_quietly` · framework/src/eloquent/model.rs:1323 (provided)
  - [ ] fn `suprnova::Model::update_quietly` · framework/src/eloquent/model.rs:1329 (provided)
  - [ ] fn `suprnova::Model::delete_quietly` · framework/src/eloquent/model.rs:1334 (provided)
  - [ ] fn `suprnova::Model::force_delete_quietly` · framework/src/eloquent/model.rs:1339 (provided)
  - [ ] fn `suprnova::Model::update_or_fail` · framework/src/eloquent/model.rs:1361 (provided)
  - [ ] fn `suprnova::Model::delete_or_fail` · framework/src/eloquent/model.rs:1412 (provided)
  - [ ] fn `suprnova::Model::primary_key_value` · framework/src/eloquent/model.rs:1431 (required)
  - [ ] fn `suprnova::Model::primary_key_value_json` · framework/src/eloquent/model.rs:1438 (required)
  - [ ] fn `suprnova::Model::reset_primary_key` · framework/src/eloquent/model.rs:1442 (required)
  - [ ] fn `suprnova::Model::active_model_from_attrs` · framework/src/eloquent/model.rs:1446 (required)
  - [ ] fn `suprnova::Model::apply_attrs_to_active_model` · framework/src/eloquent/model.rs:1453 (required)
  - [ ] fn `suprnova::Model::into_active_model_for_update` · framework/src/eloquent/model.rs:1461 (required)
- [ ] trait `suprnova::ReplicateExt` · framework/src/eloquent/model.rs:1550 (also `suprnova::eloquent::ReplicateExt`, `suprnova::eloquent::model::ReplicateExt`)
  - Implemented here by: `features::entity::Feature`, `payments::entities::customer::Customer`, `payments::entities::payment_method::PaymentMethod`, `payments::entities::subscription::Subscription`, `payments::entities::subscription_item::SubscriptionItem`, `payments::entities::transaction::Transaction`, `payments::entities::webhook_event::WebhookEvent`, `rbac::entity::ModelPermission`, `rbac::entity::ModelRole`, `rbac::entity::Permission`, `rbac::entity::Role`, `rbac::entity::RolePermission`
  - [ ] fn `suprnova::ReplicateExt::replicate_with` · framework/src/eloquent/model.rs:1553 (required)

### `suprnova::eloquent::observers`

- [ ] fn `suprnova::bootstrap_observers` · framework/src/eloquent/observers.rs:311 (also `suprnova::eloquent::observers::bootstrap_observers`)
- [ ] struct `suprnova::ObserverEntry` · framework/src/eloquent/observers.rs:277 (also `suprnova::eloquent::observers::ObserverEntry`)
  - Public fields: `name`, `install`
- [ ] trait `suprnova::Observer` · framework/src/eloquent/observers.rs:130 (also `suprnova::eloquent::observers::Observer`)
  - [ ] fn `suprnova::Observer::retrieving` · framework/src/eloquent/observers.rs:147 (provided)
  - [ ] fn `suprnova::Observer::retrieved` · framework/src/eloquent/observers.rs:152 (provided)
  - [ ] fn `suprnova::Observer::saving` · framework/src/eloquent/observers.rs:161 (provided)
  - [ ] fn `suprnova::Observer::creating` · framework/src/eloquent/observers.rs:168 (provided)
  - [ ] fn `suprnova::Observer::updating` · framework/src/eloquent/observers.rs:175 (provided)
  - [ ] fn `suprnova::Observer::deleting` · framework/src/eloquent/observers.rs:182 (provided)
  - [ ] fn `suprnova::Observer::restoring` · framework/src/eloquent/observers.rs:188 (provided)
  - [ ] fn `suprnova::Observer::created` · framework/src/eloquent/observers.rs:195 (provided)
  - [ ] fn `suprnova::Observer::updated` · framework/src/eloquent/observers.rs:201 (provided)
  - [ ] fn `suprnova::Observer::saved` · framework/src/eloquent/observers.rs:206 (provided)
  - [ ] fn `suprnova::Observer::deleted` · framework/src/eloquent/observers.rs:213 (provided)
  - [ ] fn `suprnova::Observer::trashed` · framework/src/eloquent/observers.rs:220 (provided)
  - [ ] fn `suprnova::Observer::restored` · framework/src/eloquent/observers.rs:225 (provided)
  - [ ] fn `suprnova::Observer::replicating` · framework/src/eloquent/observers.rs:233 (provided)
  - [ ] fn `suprnova::Observer::force_deleting` · framework/src/eloquent/observers.rs:242 (provided)
  - [ ] fn `suprnova::Observer::force_deleted` · framework/src/eloquent/observers.rs:247 (provided)
- [ ] type `suprnova::ObserverInstallFuture` · framework/src/eloquent/observers.rs:258 (also `suprnova::eloquent::observers::ObserverInstallFuture`)

### `suprnova::eloquent::prunable`

- [ ] fn `suprnova::prune_all` · framework/src/eloquent/prunable.rs:154 (also `suprnova::eloquent::prunable::prune_all`, `suprnova::eloquent::prune_all`)
- [ ] fn `suprnova::prune_all_dry` · framework/src/eloquent/prunable.rs:164 (also `suprnova::eloquent::prunable::prune_all_dry`, `suprnova::eloquent::prune_all_dry`)
- [ ] fn `suprnova::prune_one` · framework/src/eloquent/prunable.rs:185 (also `suprnova::eloquent::prunable::prune_one`, `suprnova::eloquent::prune_one`)
- [ ] fn `suprnova::eloquent::pruners` · framework/src/eloquent/prunable.rs:147 (also `suprnova::eloquent::prunable::pruners`)
- [ ] struct `suprnova::PrunerEntry` · framework/src/eloquent/prunable.rs:132 (also `suprnova::eloquent::PrunerEntry`, `suprnova::eloquent::prunable::PrunerEntry`)
  - Public fields: `type_name`, `run`
- [ ] trait `suprnova::MassPrunable` · framework/src/eloquent/prunable.rs:102 (also `suprnova::eloquent::MassPrunable`, `suprnova::eloquent::prunable::MassPrunable`)
  - [ ] fn `suprnova::MassPrunable::prunable` · framework/src/eloquent/prunable.rs:119 (required)
- [ ] trait `suprnova::Prunable` · framework/src/eloquent/prunable.rs:69 (also `suprnova::eloquent::Prunable`, `suprnova::eloquent::prunable::Prunable`)
  - [ ] fn `suprnova::Prunable::prunable` · framework/src/eloquent/prunable.rs:83 (required)
  - [ ] fn `suprnova::Prunable::pruning` · framework/src/eloquent/prunable.rs:89 (provided)
- [ ] type `suprnova::eloquent::PrunerFn` · framework/src/eloquent/prunable.rs:126 (also `suprnova::eloquent::prunable::PrunerFn`)

### `suprnova::eloquent::registry`

- [ ] fn `suprnova::find_model_by_table` · framework/src/eloquent/registry.rs:46 (also `suprnova::eloquent::find_model_by_table`, `suprnova::eloquent::registry::find_model_by_table`)
- [ ] fn `suprnova::models` · framework/src/eloquent/registry.rs:29 (also `suprnova::eloquent::models`, `suprnova::eloquent::registry::models`)
- [ ] struct `suprnova::ModelEntry` · framework/src/eloquent/registry.rs:14 (also `suprnova::eloquent::ModelEntry`, `suprnova::eloquent::registry::ModelEntry`)
  - Public fields: `type_name`, `table`, `module_path`, `primary_key`

### `suprnova::eloquent::relations`

- [ ] fn `suprnova::eloquent::aggregate_cache_key` · framework/src/eloquent/relations/mod.rs:167 (also `suprnova::eloquent::relations::aggregate_cache_key`, `suprnova::relations::aggregate_cache_key`)
- [ ] fn `suprnova::find_relation` · framework/src/eloquent/relations/mod.rs:339 (also `suprnova::eloquent::find_relation`, `suprnova::eloquent::relations::find_relation`, `suprnova::relations::find_relation`)
- [ ] fn `suprnova::relations` · framework/src/eloquent/relations/mod.rs:327 (also `suprnova::eloquent::relations`, `suprnova::eloquent::relations::relations`, `suprnova::relations::relations`)
- [ ] fn `suprnova::relations_of` · framework/src/eloquent/relations/mod.rs:332 (also `suprnova::eloquent::relations::relations_of`, `suprnova::eloquent::relations_of`, `suprnova::relations::relations_of`)
- [ ] fn `suprnova::eloquent::touch_column` · framework/src/eloquent/relations/mod.rs:189 (also `suprnova::eloquent::relations::touch_column`, `suprnova::relations::touch_column`)
- [ ] struct `suprnova::RelationEntry` · framework/src/eloquent/relations/mod.rs:261 (also `suprnova::eloquent::RelationEntry`, `suprnova::eloquent::relations::RelationEntry`, `suprnova::relations::RelationEntry`)
  - Public fields: `parent_type`, `target_type`, `name`, `kind`, `parent_type_name`, `target_type_name`, `target_table`, `foreign_key`, `parent_key`, `pivot_table`, `pivot_parent_key`, `pivot_related_key`, `morph_type_column`, `morph_type_value`, `target_primary_key`, `related_soft_deletes_column`, `related_updated_at_column`
- [ ] enum `suprnova::AggregateKind` · framework/src/eloquent/relations/mod.rs:126 (also `suprnova::eloquent::AggregateKind`, `suprnova::eloquent::relations::AggregateKind`, `suprnova::relations::AggregateKind`)
  - Variants: `Sum`, `Avg`, `Min`, `Max`
  - [ ] fn `suprnova::AggregateKind::as_key_str` · framework/src/eloquent/relations/mod.rs:142
- [ ] enum `suprnova::RelationKind` · framework/src/eloquent/relations/mod.rs:96 (also `suprnova::eloquent::RelationKind`, `suprnova::eloquent::relations::RelationKind`, `suprnova::relations::RelationKind`)
  - Variants: `HasOne`, `BelongsTo`, `HasMany`, `BelongsToMany`, `HasOneThrough`, `HasManyThrough`, `MorphTo`, `MorphOne`, `MorphMany`, `MorphToMany`, `MorphedByMany`
- [ ] trait `suprnova::EagerLoadDispatch` · framework/src/eloquent/relations/mod.rs:459 (also `suprnova::eloquent::EagerLoadDispatch`, `suprnova::eloquent::relations::EagerLoadDispatch`, `suprnova::relations::EagerLoadDispatch`)
  - Implemented here by: `features::entity::Feature`, `payments::entities::customer::Customer`, `payments::entities::payment_method::PaymentMethod`, `payments::entities::subscription::Subscription`, `payments::entities::subscription_item::SubscriptionItem`, `payments::entities::transaction::Transaction`, `payments::entities::webhook_event::WebhookEvent`, `rbac::entity::ModelPermission`, `rbac::entity::ModelRole`, `rbac::entity::Permission`, `rbac::entity::Role`, `rbac::entity::RolePermission`
  - [ ] fn `suprnova::EagerLoadDispatch::eager_load` · framework/src/eloquent/relations/mod.rs:461 (required)
  - [ ] fn `suprnova::EagerLoadDispatch::count_relation` · framework/src/eloquent/relations/mod.rs:469 (required)
  - [ ] fn `suprnova::EagerLoadDispatch::aggregate_relation` · framework/src/eloquent/relations/mod.rs:476 (required)
  - [ ] fn `suprnova::EagerLoadDispatch::recurse_eager_load` · framework/src/eloquent/relations/mod.rs:497 (required)
  - [ ] fn `suprnova::EagerLoadDispatch::recurse_eager_load_batched` · framework/src/eloquent/relations/mod.rs:516 (required)
  - [ ] fn `suprnova::EagerLoadDispatch::set_pivot_arc` · framework/src/eloquent/relations/mod.rs:534 (required)
  - [ ] fn `suprnova::EagerLoadDispatch::has_eager` · framework/src/eloquent/relations/mod.rs:544 (required)
- [ ] trait `suprnova::Relation` · framework/src/eloquent/relations/mod.rs:207 (also `suprnova::eloquent::Relation`, `suprnova::eloquent::relations::Relation`, `suprnova::relations::Relation`)
  - Implemented here by: `BelongsTo`, `BelongsToMany`, `HasMany`, `HasManyThrough`, `HasOne`, `HasOneThrough`, `MorphMany`, `MorphOne`, `MorphTo`, `MorphToMany`, `MorphedByMany`
  - [ ] type `suprnova::Relation::Parent` · framework/src/eloquent/relations/mod.rs:209
  - [ ] type `suprnova::Relation::Target` · framework/src/eloquent/relations/mod.rs:211
  - [ ] const `suprnova::Relation::KIND` · framework/src/eloquent/relations/mod.rs:213
  - [ ] fn `suprnova::Relation::parent_key` · framework/src/eloquent/relations/mod.rs:217 (required)
  - [ ] fn `suprnova::Relation::foreign_key` · framework/src/eloquent/relations/mod.rs:223 (required)

### `suprnova::eloquent::relations::belongs_to`

- [ ] struct `suprnova::BelongsTo` · framework/src/eloquent/relations/belongs_to.rs:43 (also `suprnova::eloquent::BelongsTo`, `suprnova::eloquent::relations::BelongsTo`, `suprnova::eloquent::relations::belongs_to::BelongsTo`, `suprnova::relations::BelongsTo` (+1 more))
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::BelongsTo::foreign_key` · framework/src/eloquent/relations/belongs_to.rs:143
  - [ ] fn `suprnova::BelongsTo::owner_key` · framework/src/eloquent/relations/belongs_to.rs:149
  - [ ] fn `suprnova::BelongsTo::with_default` · framework/src/eloquent/relations/belongs_to.rs:162
  - [ ] fn `suprnova::BelongsTo::first` · framework/src/eloquent/relations/belongs_to.rs:182
  - [ ] fn `suprnova::BelongsTo::with_trashed` · framework/src/eloquent/relations/belongs_to.rs:238
  - [ ] fn `suprnova::BelongsTo::only_trashed` · framework/src/eloquent/relations/belongs_to.rs:244

### `suprnova::eloquent::relations::belongs_to_many`

- [ ] struct `suprnova::BelongsToMany` · framework/src/eloquent/relations/belongs_to_many.rs:87 (also `suprnova::eloquent::BelongsToMany`, `suprnova::eloquent::relations::BelongsToMany`, `suprnova::eloquent::relations::belongs_to_many::BelongsToMany`, `suprnova::relations::BelongsToMany` (+1 more))
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::BelongsToMany::with_pivot` · framework/src/eloquent/relations/belongs_to_many.rs:238
  - [ ] fn `suprnova::BelongsToMany::with_timestamps` · framework/src/eloquent/relations/belongs_to_many.rs:252
  - [ ] fn `suprnova::BelongsToMany::foreign_key` · framework/src/eloquent/relations/belongs_to_many.rs:258
  - [ ] fn `suprnova::BelongsToMany::related_key` · framework/src/eloquent/relations/belongs_to_many.rs:264
  - [ ] fn `suprnova::BelongsToMany::local_key` · framework/src/eloquent/relations/belongs_to_many.rs:271
  - [ ] fn `suprnova::BelongsToMany::related_pk` · framework/src/eloquent/relations/belongs_to_many.rs:287
  - [ ] fn `suprnova::BelongsToMany::where_pivot` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::or_where_pivot` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::where_pivot_op` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::or_where_pivot_op` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::where_pivot_in` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::or_where_pivot_in` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::where_pivot_not_in` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::or_where_pivot_not_in` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::where_pivot_null` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::or_where_pivot_null` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::where_pivot_not_null` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::or_where_pivot_not_null` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::where_pivot_between` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::or_where_pivot_between` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::where_pivot_not_between` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::or_where_pivot_not_between` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::where_pivot_group` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::or_where_pivot_group` · framework/src/eloquent/relations/belongs_to_many.rs:292
  - [ ] fn `suprnova::BelongsToMany::attach` · framework/src/eloquent/relations/belongs_to_many.rs:314
  - [ ] fn `suprnova::BelongsToMany::attach_with` · framework/src/eloquent/relations/belongs_to_many.rs:338
  - [ ] fn `suprnova::BelongsToMany::detach` · framework/src/eloquent/relations/belongs_to_many.rs:410
  - [ ] fn `suprnova::BelongsToMany::sync` · framework/src/eloquent/relations/belongs_to_many.rs:474
  - [ ] fn `suprnova::BelongsToMany::get` · framework/src/eloquent/relations/belongs_to_many.rs:673
  - [ ] fn `suprnova::BelongsToMany::first` · framework/src/eloquent/relations/belongs_to_many.rs:782
  - [ ] fn `suprnova::BelongsToMany::count` · framework/src/eloquent/relations/belongs_to_many.rs:789
  - [ ] fn `suprnova::BelongsToMany::with_trashed` · framework/src/eloquent/relations/belongs_to_many.rs:865
  - [ ] fn `suprnova::BelongsToMany::only_trashed` · framework/src/eloquent/relations/belongs_to_many.rs:871

### `suprnova::eloquent::relations::eager_cache`

- [ ] struct `suprnova::EagerLoadCache` · framework/src/eloquent/relations/eager_cache.rs:35 (also `suprnova::eloquent::EagerLoadCache`, `suprnova::eloquent::relations::EagerLoadCache`, `suprnova::eloquent::relations::eager_cache::EagerLoadCache`, `suprnova::relations::EagerLoadCache` (+1 more))
  - [ ] fn `suprnova::EagerLoadCache::new` · framework/src/eloquent/relations/eager_cache.rs:59
  - [ ] fn `suprnova::EagerLoadCache::has` · framework/src/eloquent/relations/eager_cache.rs:66
  - [ ] fn `suprnova::EagerLoadCache::set_many` · framework/src/eloquent/relations/eager_cache.rs:71
  - [ ] fn `suprnova::EagerLoadCache::get_many` · framework/src/eloquent/relations/eager_cache.rs:85
  - [ ] fn `suprnova::EagerLoadCache::get_many_mut` · framework/src/eloquent/relations/eager_cache.rs:118
  - [ ] fn `suprnova::EagerLoadCache::get_one_mut` · framework/src/eloquent/relations/eager_cache.rs:133
  - [ ] fn `suprnova::EagerLoadCache::set_one` · framework/src/eloquent/relations/eager_cache.rs:143
  - [ ] fn `suprnova::EagerLoadCache::take_many` · framework/src/eloquent/relations/eager_cache.rs:155
  - [ ] fn `suprnova::EagerLoadCache::take_one` · framework/src/eloquent/relations/eager_cache.rs:166
  - [ ] fn `suprnova::EagerLoadCache::get_one` · framework/src/eloquent/relations/eager_cache.rs:182
  - [ ] fn `suprnova::EagerLoadCache::set_count` · framework/src/eloquent/relations/eager_cache.rs:196
  - [ ] fn `suprnova::EagerLoadCache::get_count` · framework/src/eloquent/relations/eager_cache.rs:204
  - [ ] fn `suprnova::EagerLoadCache::set_aggregate` · framework/src/eloquent/relations/eager_cache.rs:220
  - [ ] fn `suprnova::EagerLoadCache::get_aggregate` · framework/src/eloquent/relations/eager_cache.rs:230

### `suprnova::eloquent::relations::has_many`

- [ ] struct `suprnova::HasMany` · framework/src/eloquent/relations/has_many.rs:50 (also `suprnova::eloquent::HasMany`, `suprnova::eloquent::relations::HasMany`, `suprnova::eloquent::relations::has_many::HasMany`, `suprnova::relations::HasMany` (+1 more))
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::HasMany::foreign_key` · framework/src/eloquent/relations/has_many.rs:148
  - [ ] fn `suprnova::HasMany::local_key` · framework/src/eloquent/relations/has_many.rs:159
  - [ ] fn `suprnova::HasMany::filter` · framework/src/eloquent/relations/has_many.rs:167
  - [ ] fn `suprnova::HasMany::db_where` · framework/src/eloquent/relations/has_many.rs:173
  - [ ] fn `suprnova::HasMany::order_by` · framework/src/eloquent/relations/has_many.rs:178
  - [ ] fn `suprnova::HasMany::latest` · framework/src/eloquent/relations/has_many.rs:189
  - [ ] fn `suprnova::HasMany::oldest` · framework/src/eloquent/relations/has_many.rs:195
  - [ ] fn `suprnova::HasMany::limit` · framework/src/eloquent/relations/has_many.rs:200
  - [ ] fn `suprnova::HasMany::take` · framework/src/eloquent/relations/has_many.rs:206
  - [ ] fn `suprnova::HasMany::first` · framework/src/eloquent/relations/has_many.rs:215
  - [ ] fn `suprnova::HasMany::get` · framework/src/eloquent/relations/has_many.rs:227
  - [ ] fn `suprnova::HasMany::count` · framework/src/eloquent/relations/has_many.rs:235
  - [ ] fn `suprnova::HasMany::with_trashed` · framework/src/eloquent/relations/has_many.rs:264
  - [ ] fn `suprnova::HasMany::only_trashed` · framework/src/eloquent/relations/has_many.rs:270

### `suprnova::eloquent::relations::has_one`

- [ ] struct `suprnova::HasOne` · framework/src/eloquent/relations/has_one.rs:42 (also `suprnova::eloquent::HasOne`, `suprnova::eloquent::relations::HasOne`, `suprnova::eloquent::relations::has_one::HasOne`, `suprnova::relations::HasOne` (+1 more))
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::HasOne::foreign_key` · framework/src/eloquent/relations/has_one.rs:138
  - [ ] fn `suprnova::HasOne::local_key` · framework/src/eloquent/relations/has_one.rs:149
  - [ ] fn `suprnova::HasOne::filter` · framework/src/eloquent/relations/has_one.rs:157
  - [ ] fn `suprnova::HasOne::db_where` · framework/src/eloquent/relations/has_one.rs:163
  - [ ] fn `suprnova::HasOne::first` · framework/src/eloquent/relations/has_one.rs:170
  - [ ] fn `suprnova::HasOne::get` · framework/src/eloquent/relations/has_one.rs:182
  - [ ] fn `suprnova::HasOne::with_trashed` · framework/src/eloquent/relations/has_one.rs:212
  - [ ] fn `suprnova::HasOne::only_trashed` · framework/src/eloquent/relations/has_one.rs:219

### `suprnova::eloquent::relations::morph`

- [ ] struct `suprnova::MorphMany` · framework/src/eloquent/relations/morph.rs:52 (also `suprnova::eloquent::MorphMany`, `suprnova::eloquent::relations::MorphMany`, `suprnova::eloquent::relations::morph::MorphMany`, `suprnova::relations::MorphMany` (+1 more))
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::MorphMany::filter` · framework/src/eloquent/relations/morph.rs:158
  - [ ] fn `suprnova::MorphMany::db_where` · framework/src/eloquent/relations/morph.rs:164
  - [ ] fn `suprnova::MorphMany::order_by` · framework/src/eloquent/relations/morph.rs:169
  - [ ] fn `suprnova::MorphMany::latest` · framework/src/eloquent/relations/morph.rs:176
  - [ ] fn `suprnova::MorphMany::oldest` · framework/src/eloquent/relations/morph.rs:181
  - [ ] fn `suprnova::MorphMany::limit` · framework/src/eloquent/relations/morph.rs:186
  - [ ] fn `suprnova::MorphMany::take` · framework/src/eloquent/relations/morph.rs:192
  - [ ] fn `suprnova::MorphMany::first` · framework/src/eloquent/relations/morph.rs:197
  - [ ] fn `suprnova::MorphMany::get` · framework/src/eloquent/relations/morph.rs:206
  - [ ] fn `suprnova::MorphMany::count` · framework/src/eloquent/relations/morph.rs:213
  - [ ] fn `suprnova::MorphMany::with_trashed` · framework/src/eloquent/relations/morph.rs:241
  - [ ] fn `suprnova::MorphMany::only_trashed` · framework/src/eloquent/relations/morph.rs:247
- [ ] struct `suprnova::MorphOne` · framework/src/eloquent/relations/morph.rs:306 (also `suprnova::eloquent::MorphOne`, `suprnova::eloquent::relations::MorphOne`, `suprnova::eloquent::relations::morph::MorphOne`, `suprnova::relations::MorphOne` (+1 more))
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::MorphOne::filter` · framework/src/eloquent/relations/morph.rs:359
  - [ ] fn `suprnova::MorphOne::db_where` · framework/src/eloquent/relations/morph.rs:365
  - [ ] fn `suprnova::MorphOne::order_by` · framework/src/eloquent/relations/morph.rs:370
  - [ ] fn `suprnova::MorphOne::first` · framework/src/eloquent/relations/morph.rs:376
  - [ ] fn `suprnova::MorphOne::with_trashed` · framework/src/eloquent/relations/morph.rs:403
  - [ ] fn `suprnova::MorphOne::only_trashed` · framework/src/eloquent/relations/morph.rs:409
- [ ] struct `suprnova::MorphTo` · framework/src/eloquent/relations/morph.rs:485 (also `suprnova::eloquent::MorphTo`, `suprnova::eloquent::relations::MorphTo`, `suprnova::eloquent::relations::morph::MorphTo`, `suprnova::relations::MorphTo` (+1 more))
  - Public fields: `morph_id`, `morph_type`
  - Implements: `suprnova::Relation`

### `suprnova::eloquent::relations::morph_registry`

- [ ] fn `suprnova::find_morph_type` · framework/src/eloquent/relations/morph_registry.rs:76 (also `suprnova::eloquent::find_morph_type`, `suprnova::eloquent::relations::find_morph_type`, `suprnova::eloquent::relations::morph_registry::find_morph_type`, `suprnova::relations::find_morph_type` (+1 more))
- [ ] fn `suprnova::find_morph_type_by_id` · framework/src/eloquent/relations/morph_registry.rs:83 (also `suprnova::eloquent::find_morph_type_by_id`, `suprnova::eloquent::relations::find_morph_type_by_id`, `suprnova::eloquent::relations::morph_registry::find_morph_type_by_id`, `suprnova::relations::find_morph_type_by_id` (+1 more))
- [ ] fn `suprnova::morph_types` · framework/src/eloquent/relations/morph_registry.rs:48 (also `suprnova::eloquent::morph_types`, `suprnova::eloquent::relations::morph_registry::morph_types`, `suprnova::eloquent::relations::morph_types`, `suprnova::relations::morph_registry::morph_types` (+1 more))
- [ ] struct `suprnova::MorphTypeEntry` · framework/src/eloquent/relations/morph_registry.rs:29 (also `suprnova::eloquent::MorphTypeEntry`, `suprnova::eloquent::relations::MorphTypeEntry`, `suprnova::eloquent::relations::morph_registry::MorphTypeEntry`, `suprnova::relations::MorphTypeEntry` (+1 more))
  - Public fields: `morph_type`, `type_name`, `table`, `type_id`

### `suprnova::eloquent::relations::morph_to_many`

- [ ] struct `suprnova::MorphedByMany` · framework/src/eloquent/relations/morph_to_many.rs:902 (also `suprnova::eloquent::MorphedByMany`, `suprnova::eloquent::relations::MorphedByMany`, `suprnova::eloquent::relations::morph_to_many::MorphedByMany`, `suprnova::relations::MorphedByMany` (+1 more))
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::MorphedByMany::related_pk` · framework/src/eloquent/relations/morph_to_many.rs:1029
  - [ ] fn `suprnova::MorphedByMany::local_key` · framework/src/eloquent/relations/morph_to_many.rs:1035
  - [ ] fn `suprnova::MorphedByMany::where_pivot` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::or_where_pivot` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::where_pivot_op` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::or_where_pivot_op` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::where_pivot_in` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::or_where_pivot_in` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::where_pivot_not_in` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::or_where_pivot_not_in` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::where_pivot_null` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::or_where_pivot_null` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::where_pivot_not_null` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::or_where_pivot_not_null` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::where_pivot_between` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::or_where_pivot_between` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::where_pivot_not_between` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::or_where_pivot_not_between` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::where_pivot_group` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::or_where_pivot_group` · framework/src/eloquent/relations/morph_to_many.rs:1040
  - [ ] fn `suprnova::MorphedByMany::get` · framework/src/eloquent/relations/morph_to_many.rs:1063
  - [ ] fn `suprnova::MorphedByMany::first` · framework/src/eloquent/relations/morph_to_many.rs:1140
  - [ ] fn `suprnova::MorphedByMany::count` · framework/src/eloquent/relations/morph_to_many.rs:1145
  - [ ] fn `suprnova::MorphedByMany::with_trashed` · framework/src/eloquent/relations/morph_to_many.rs:1226
  - [ ] fn `suprnova::MorphedByMany::only_trashed` · framework/src/eloquent/relations/morph_to_many.rs:1232
- [ ] struct `suprnova::MorphToMany` · framework/src/eloquent/relations/morph_to_many.rs:111 (also `suprnova::eloquent::MorphToMany`, `suprnova::eloquent::relations::MorphToMany`, `suprnova::eloquent::relations::morph_to_many::MorphToMany`, `suprnova::relations::MorphToMany` (+1 more))
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::MorphToMany::with_pivot` · framework/src/eloquent/relations/morph_to_many.rs:260
  - [ ] fn `suprnova::MorphToMany::with_timestamps` · framework/src/eloquent/relations/morph_to_many.rs:273
  - [ ] fn `suprnova::MorphToMany::local_key` · framework/src/eloquent/relations/morph_to_many.rs:280
  - [ ] fn `suprnova::MorphToMany::related_pk` · framework/src/eloquent/relations/morph_to_many.rs:288
  - [ ] fn `suprnova::MorphToMany::where_pivot` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::or_where_pivot` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::where_pivot_op` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::or_where_pivot_op` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::where_pivot_in` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::or_where_pivot_in` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::where_pivot_not_in` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::or_where_pivot_not_in` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::where_pivot_null` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::or_where_pivot_null` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::where_pivot_not_null` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::or_where_pivot_not_null` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::where_pivot_between` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::or_where_pivot_between` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::where_pivot_not_between` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::or_where_pivot_not_between` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::where_pivot_group` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::or_where_pivot_group` · framework/src/eloquent/relations/morph_to_many.rs:293
  - [ ] fn `suprnova::MorphToMany::attach` · framework/src/eloquent/relations/morph_to_many.rs:311
  - [ ] fn `suprnova::MorphToMany::attach_with` · framework/src/eloquent/relations/morph_to_many.rs:328
  - [ ] fn `suprnova::MorphToMany::detach` · framework/src/eloquent/relations/morph_to_many.rs:394
  - [ ] fn `suprnova::MorphToMany::sync` · framework/src/eloquent/relations/morph_to_many.rs:444
  - [ ] fn `suprnova::MorphToMany::get` · framework/src/eloquent/relations/morph_to_many.rs:622
  - [ ] fn `suprnova::MorphToMany::first` · framework/src/eloquent/relations/morph_to_many.rs:740
  - [ ] fn `suprnova::MorphToMany::count` · framework/src/eloquent/relations/morph_to_many.rs:746
  - [ ] fn `suprnova::MorphToMany::with_trashed` · framework/src/eloquent/relations/morph_to_many.rs:828
  - [ ] fn `suprnova::MorphToMany::only_trashed` · framework/src/eloquent/relations/morph_to_many.rs:834

### `suprnova::eloquent::relations::through`

- [ ] struct `suprnova::HasManyThrough` · framework/src/eloquent/relations/through.rs:84 (also `suprnova::eloquent::HasManyThrough`, `suprnova::eloquent::relations::HasManyThrough`, `suprnova::eloquent::relations::through::HasManyThrough`, `suprnova::relations::HasManyThrough` (+1 more))
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::HasManyThrough::first_key` · framework/src/eloquent/relations/through.rs:173
  - [ ] fn `suprnova::HasManyThrough::second_key` · framework/src/eloquent/relations/through.rs:179
  - [ ] fn `suprnova::HasManyThrough::local_key` · framework/src/eloquent/relations/through.rs:188
  - [ ] fn `suprnova::HasManyThrough::second_local_key` · framework/src/eloquent/relations/through.rs:196
  - [ ] fn `suprnova::HasManyThrough::get` · framework/src/eloquent/relations/through.rs:232
  - [ ] fn `suprnova::HasManyThrough::first` · framework/src/eloquent/relations/through.rs:259
  - [ ] fn `suprnova::HasManyThrough::count` · framework/src/eloquent/relations/through.rs:269
- [ ] struct `suprnova::HasOneThrough` · framework/src/eloquent/relations/through.rs:399 (also `suprnova::eloquent::HasOneThrough`, `suprnova::eloquent::relations::HasOneThrough`, `suprnova::eloquent::relations::through::HasOneThrough`, `suprnova::relations::HasOneThrough` (+1 more))
  - Implements: `suprnova::Relation`
  - [ ] fn `suprnova::HasOneThrough::first_key` · framework/src/eloquent/relations/through.rs:454
  - [ ] fn `suprnova::HasOneThrough::second_key` · framework/src/eloquent/relations/through.rs:460
  - [ ] fn `suprnova::HasOneThrough::local_key` · framework/src/eloquent/relations/through.rs:466
  - [ ] fn `suprnova::HasOneThrough::second_local_key` · framework/src/eloquent/relations/through.rs:472
  - [ ] fn `suprnova::HasOneThrough::first` · framework/src/eloquent/relations/through.rs:480
  - [ ] fn `suprnova::HasOneThrough::get` · framework/src/eloquent/relations/through.rs:486

### `suprnova::eloquent::scopes`

- [ ] struct `suprnova::ScopeRegistry` · framework/src/eloquent/scopes.rs:196 (also `suprnova::eloquent::ScopeRegistry`, `suprnova::eloquent::scopes::ScopeRegistry`)
  - [ ] fn `suprnova::ScopeRegistry::register` · framework/src/eloquent/scopes.rs:210
  - [ ] fn `suprnova::ScopeRegistry::apply_to` · framework/src/eloquent/scopes.rs:294
- [ ] enum `suprnova::eloquent::scopes::ScopeDependency` · framework/src/eloquent/scopes.rs:125
  - Variants: `Constant`, `PerRequest`
- [ ] trait `suprnova::GlobalScope` · framework/src/eloquent/scopes.rs:88 (also `suprnova::eloquent::GlobalScope`, `suprnova::eloquent::scopes::GlobalScope`)
  - [ ] fn `suprnova::GlobalScope::apply` · framework/src/eloquent/scopes.rs:104 (required)
  - [ ] fn `suprnova::GlobalScope::dependency` · framework/src/eloquent/scopes.rs:112 (provided)

### `suprnova::eloquent::soft_deletes`

- [ ] trait `suprnova::SoftDeletes` · framework/src/eloquent/soft_deletes.rs:42 (also `suprnova::eloquent::SoftDeletes`, `suprnova::eloquent::soft_deletes::SoftDeletes`)
  - [ ] fn `suprnova::SoftDeletes::deleted_at_column` · framework/src/eloquent/soft_deletes.rs:57 (required)
  - [ ] fn `suprnova::SoftDeletes::is_trashed` · framework/src/eloquent/soft_deletes.rs:62 (required)

### `suprnova::eloquent::timestamps`

- [ ] fn `suprnova::eloquent::touches_disabled` · framework/src/eloquent/timestamps.rs:49 (also `suprnova::eloquent::timestamps::touches_disabled`)
- [ ] fn `suprnova::eloquent::touches_ignored_for` · framework/src/eloquent/timestamps.rs:130 (also `suprnova::eloquent::timestamps::touches_ignored_for`)
- [ ] fn `suprnova::eloquent::without_touching` · framework/src/eloquent/timestamps.rs:78 (also `suprnova::eloquent::timestamps::without_touching`)
- [ ] fn `suprnova::eloquent::without_touching_on` · framework/src/eloquent/timestamps.rs:107 (also `suprnova::eloquent::timestamps::without_touching_on`)
- [ ] trait `suprnova::Touchable` · framework/src/eloquent/timestamps.rs:162 (also `suprnova::eloquent::Touchable`, `suprnova::eloquent::timestamps::Touchable`)
  - Implemented here by: `features::entity::Feature`, `payments::entities::customer::Customer`, `payments::entities::payment_method::PaymentMethod`, `payments::entities::subscription::Subscription`, `payments::entities::subscription_item::SubscriptionItem`, `payments::entities::transaction::Transaction`, `rbac::entity::Permission`, `rbac::entity::Role`
  - [ ] fn `suprnova::Touchable::touch` · framework/src/eloquent/timestamps.rs:167 (required)

### `suprnova::eloquent::unique_id`

- [ ] enum `suprnova::eloquent::UniqueIdKind` · framework/src/eloquent/unique_id.rs:32 (also `suprnova::eloquent::unique_id::UniqueIdKind`)
  - Variants: `UuidV7`, `UuidV4`, `Ulid`
  - [ ] fn `suprnova::eloquent::UniqueIdKind::parse` · framework/src/eloquent/unique_id.rs:51
  - [ ] fn `suprnova::eloquent::UniqueIdKind::is_valid` · framework/src/eloquent/unique_id.rs:77
  - [ ] fn `suprnova::eloquent::UniqueIdKind::generate` · framework/src/eloquent/unique_id.rs:85
- [ ] trait `suprnova::eloquent::HasUniqueId` · framework/src/eloquent/unique_id.rs:98 (also `suprnova::eloquent::unique_id::HasUniqueId`)
  - [ ] const `suprnova::eloquent::HasUniqueId::UNIQUE_ID_KIND` · framework/src/eloquent/unique_id.rs:100
  - [ ] fn `suprnova::eloquent::HasUniqueId::new_unique_id` · framework/src/eloquent/unique_id.rs:106 (provided)

## error

### `suprnova::error`

- [ ] fn `suprnova::render_error_chain` · framework/src/error.rs:1419 (also `suprnova::error::render_error_chain`)
- [ ] struct `suprnova::AppError` · framework/src/error.rs:67 (also `suprnova::error::AppError`, `suprnova::prelude::AppError`)
  - Implements: `suprnova::HttpError`
  - [ ] fn `suprnova::AppError::new` · framework/src/error.rs:74
  - [ ] fn `suprnova::AppError::status` · framework/src/error.rs:82
  - [ ] fn `suprnova::AppError::not_found` · framework/src/error.rs:88
  - [ ] fn `suprnova::AppError::bad_request` · framework/src/error.rs:93
  - [ ] fn `suprnova::AppError::unauthorized` · framework/src/error.rs:98
  - [ ] fn `suprnova::AppError::forbidden` · framework/src/error.rs:103
  - [ ] fn `suprnova::AppError::unprocessable` · framework/src/error.rs:108
  - [ ] fn `suprnova::AppError::conflict` · framework/src/error.rs:113
- [ ] struct `suprnova::ValidationErrors` · framework/src/error.rs:172 (also `suprnova::error::ValidationErrors`)
  - Public fields: `errors`
  - [ ] fn `suprnova::ValidationErrors::new` · framework/src/error.rs:179
  - [ ] fn `suprnova::ValidationErrors::add` · framework/src/error.rs:191
  - [ ] fn `suprnova::ValidationErrors::add_to_bag` · framework/src/error.rs:205
  - [ ] fn `suprnova::ValidationErrors::is_empty` · framework/src/error.rs:216
  - [ ] fn `suprnova::ValidationErrors::into_result` · framework/src/error.rs:237
  - [ ] fn `suprnova::ValidationErrors::from_validator` · framework/src/error.rs:294
  - [ ] fn `suprnova::ValidationErrors::messages_for` · framework/src/error.rs:395
  - [ ] fn `suprnova::ValidationErrors::to_json` · framework/src/error.rs:407
  - [ ] fn `suprnova::ValidationErrors::retain_fields` · framework/src/error.rs:432
- [ ] enum `suprnova::FrameworkError` · framework/src/error.rs:790 (also `suprnova::error::FrameworkError`, `suprnova::prelude::FrameworkError`)
  - Variants: `ServiceNotFound`, `ParamError`, `ValidationError`, `Database`, `Internal`, `Domain`, `Validation`, `Unauthorized`, `ModelNotFound`, `ParamParse`, `UnsupportedMediaType`, `PrecognitionSuccess`, `PrecognitionFailure`, `AlreadyReported`, `RateLimited`, `External`
  - [ ] fn `suprnova::FrameworkError::service_not_found` · framework/src/error.rs:955
  - [ ] fn `suprnova::FrameworkError::param` · framework/src/error.rs:962
  - [ ] fn `suprnova::FrameworkError::validation` · framework/src/error.rs:969
  - [ ] fn `suprnova::FrameworkError::database` · framework/src/error.rs:977
  - [ ] fn `suprnova::FrameworkError::internal` · framework/src/error.rs:982
  - [ ] fn `suprnova::FrameworkError::silent` · framework/src/error.rs:998
  - [ ] fn `suprnova::FrameworkError::is_silent` · framework/src/error.rs:1007
  - [ ] fn `suprnova::FrameworkError::domain` · framework/src/error.rs:1012
  - [ ] fn `suprnova::FrameworkError::from_http_error` · framework/src/error.rs:1055
  - [ ] fn `suprnova::FrameworkError::from_external` · framework/src/error.rs:1070
  - [ ] fn `suprnova::FrameworkError::from_external_with` · framework/src/error.rs:1088
  - [ ] fn `suprnova::FrameworkError::external_source` · framework/src/error.rs:1105
  - [ ] fn `suprnova::FrameworkError::bad_request` · framework/src/error.rs:1113
  - [ ] fn `suprnova::FrameworkError::status_code` · framework/src/error.rs:1121
  - [ ] fn `suprnova::FrameworkError::rate_limited` · framework/src/error.rs:1147
  - [ ] fn `suprnova::FrameworkError::retry_after` · framework/src/error.rs:1160
  - [ ] fn `suprnova::FrameworkError::validation_errors` · framework/src/error.rs:1168
  - [ ] fn `suprnova::FrameworkError::from_unique_violation` · framework/src/error.rs:1216
  - [ ] fn `suprnova::FrameworkError::model_not_found` · framework/src/error.rs:1235
  - [ ] fn `suprnova::FrameworkError::param_parse` · framework/src/error.rs:1242
  - [ ] fn `suprnova::FrameworkError::not_found` · framework/src/error.rs:1250
  - [ ] fn `suprnova::FrameworkError::message` · framework/src/error.rs:1263
  - [ ] fn `suprnova::FrameworkError::field` · framework/src/error.rs:1287
  - [ ] fn `suprnova::FrameworkError::context` · framework/src/error.rs:1326
  - [ ] fn `suprnova::FrameworkError::into_json_api_response` · framework/src/resources/errors.rs:30
- [ ] trait `suprnova::HttpError` · framework/src/error.rs:36 (also `suprnova::error::HttpError`, `suprnova::prelude::HttpError`)
  - Implemented here by: `AppError`
  - [ ] fn `suprnova::HttpError::status_code` · framework/src/error.rs:38 (provided)
  - [ ] fn `suprnova::HttpError::error_message` · framework/src/error.rs:43 (provided)

## events

### `suprnova::events`

- [ ] trait `suprnova::Event` · framework/src/events/mod.rs:56 (also `suprnova::events::Event`)
  - Implemented here by: `ConnectionEstablished`, `DatabaseBusy`, `ErrorOccurred`, `MessageSending`, `MessageSent`, `NotificationFailed`, `NotificationSending`, `NotificationSent`, `PasswordResetLinkSent`, `QueryExecuted`, `TransactionBeginning`, `TransactionCommitted`, `TransactionRolledBack`, `TwoFactorChallengeFailed`, `TwoFactorChallenged`, `auth::events::Attempting`, `auth::events::Authenticated`, `auth::events::Failed`, `auth::events::Login`, `auth::events::Logout`, `auth_flows::AccountLocked`, `auth_flows::AccountUnlocked`, `auth_flows::EmailVerified`, `auth_flows::PasswordResetCompleted`, `auth_flows::TwoFactorDisabled`, `auth_flows::TwoFactorEnrolled`, `features::FeatureDeleted`, `features::FeatureUpdated`, `features::entity::feature::events::Created`, `features::entity::feature::events::Creating`, `features::entity::feature::events::Deleted`, `features::entity::feature::events::Deleting`, `features::entity::feature::events::ForceDeleted`, `features::entity::feature::events::ForceDeleting`, `features::entity::feature::events::Replicating`, `features::entity::feature::events::Restored`, `features::entity::feature::events::Restoring`, `features::entity::feature::events::Retrieved`, `features::entity::feature::events::Retrieving`, `features::entity::feature::events::Saved`, `features::entity::feature::events::Saving`, `features::entity::feature::events::Trashed`, `features::entity::feature::events::Updated`, `features::entity::feature::events::Updating`, `live::LiveOutcomeAccepted`, `magnetar_integration::engine::MagnetarLifecycleEvent`, `payments::entities::customer::customer::events::Created`, `payments::entities::customer::customer::events::Creating`, `payments::entities::customer::customer::events::Deleted`, `payments::entities::customer::customer::events::Deleting`, `payments::entities::customer::customer::events::ForceDeleted`, `payments::entities::customer::customer::events::ForceDeleting`, `payments::entities::customer::customer::events::Replicating`, `payments::entities::customer::customer::events::Restored`, `payments::entities::customer::customer::events::Restoring`, `payments::entities::customer::customer::events::Retrieved`, `payments::entities::customer::customer::events::Retrieving`, `payments::entities::customer::customer::events::Saved`, `payments::entities::customer::customer::events::Saving`, `payments::entities::customer::customer::events::Trashed`, `payments::entities::customer::customer::events::Updated`, `payments::entities::customer::customer::events::Updating`, `payments::entities::payment_method::payment_method::events::Created`, `payments::entities::payment_method::payment_method::events::Creating`, `payments::entities::payment_method::payment_method::events::Deleted`, `payments::entities::payment_method::payment_method::events::Deleting`, `payments::entities::payment_method::payment_method::events::ForceDeleted`, `payments::entities::payment_method::payment_method::events::ForceDeleting`, `payments::entities::payment_method::payment_method::events::Replicating`, `payments::entities::payment_method::payment_method::events::Restored`, `payments::entities::payment_method::payment_method::events::Restoring`, `payments::entities::payment_method::payment_method::events::Retrieved`, `payments::entities::payment_method::payment_method::events::Retrieving`, `payments::entities::payment_method::payment_method::events::Saved`, `payments::entities::payment_method::payment_method::events::Saving`, `payments::entities::payment_method::payment_method::events::Trashed`, `payments::entities::payment_method::payment_method::events::Updated`, `payments::entities::payment_method::payment_method::events::Updating`, `payments::entities::subscription::subscription::events::Created`, `payments::entities::subscription::subscription::events::Creating`, `payments::entities::subscription::subscription::events::Deleted`, `payments::entities::subscription::subscription::events::Deleting`, `payments::entities::subscription::subscription::events::ForceDeleted`, `payments::entities::subscription::subscription::events::ForceDeleting`, `payments::entities::subscription::subscription::events::Replicating`, `payments::entities::subscription::subscription::events::Restored`, `payments::entities::subscription::subscription::events::Restoring`, `payments::entities::subscription::subscription::events::Retrieved`, `payments::entities::subscription::subscription::events::Retrieving`, `payments::entities::subscription::subscription::events::Saved`, `payments::entities::subscription::subscription::events::Saving`, `payments::entities::subscription::subscription::events::Trashed`, `payments::entities::subscription::subscription::events::Updated`, `payments::entities::subscription::subscription::events::Updating`, `payments::entities::subscription_item::subscription_item::events::Created`, `payments::entities::subscription_item::subscription_item::events::Creating`, `payments::entities::subscription_item::subscription_item::events::Deleted`, `payments::entities::subscription_item::subscription_item::events::Deleting`, `payments::entities::subscription_item::subscription_item::events::ForceDeleted`, `payments::entities::subscription_item::subscription_item::events::ForceDeleting`, `payments::entities::subscription_item::subscription_item::events::Replicating`, `payments::entities::subscription_item::subscription_item::events::Restored`, `payments::entities::subscription_item::subscription_item::events::Restoring`, `payments::entities::subscription_item::subscription_item::events::Retrieved`, `payments::entities::subscription_item::subscription_item::events::Retrieving`, `payments::entities::subscription_item::subscription_item::events::Saved`, `payments::entities::subscription_item::subscription_item::events::Saving`, `payments::entities::subscription_item::subscription_item::events::Trashed`, `payments::entities::subscription_item::subscription_item::events::Updated`, `payments::entities::subscription_item::subscription_item::events::Updating`, `payments::entities::transaction::transaction::events::Created`, `payments::entities::transaction::transaction::events::Creating`, `payments::entities::transaction::transaction::events::Deleted`, `payments::entities::transaction::transaction::events::Deleting`, `payments::entities::transaction::transaction::events::ForceDeleted`, `payments::entities::transaction::transaction::events::ForceDeleting`, `payments::entities::transaction::transaction::events::Replicating`, `payments::entities::transaction::transaction::events::Restored`, `payments::entities::transaction::transaction::events::Restoring`, `payments::entities::transaction::transaction::events::Retrieved`, `payments::entities::transaction::transaction::events::Retrieving`, `payments::entities::transaction::transaction::events::Saved`, `payments::entities::transaction::transaction::events::Saving`, `payments::entities::transaction::transaction::events::Trashed`, `payments::entities::transaction::transaction::events::Updated`, `payments::entities::transaction::transaction::events::Updating`, `payments::entities::webhook_event::webhook_event::events::Created`, `payments::entities::webhook_event::webhook_event::events::Creating`, `payments::entities::webhook_event::webhook_event::events::Deleted`, `payments::entities::webhook_event::webhook_event::events::Deleting`, `payments::entities::webhook_event::webhook_event::events::ForceDeleted`, `payments::entities::webhook_event::webhook_event::events::ForceDeleting`, `payments::entities::webhook_event::webhook_event::events::Replicating`, `payments::entities::webhook_event::webhook_event::events::Restored`, `payments::entities::webhook_event::webhook_event::events::Restoring`, `payments::entities::webhook_event::webhook_event::events::Retrieved`, `payments::entities::webhook_event::webhook_event::events::Retrieving`, `payments::entities::webhook_event::webhook_event::events::Saved`, `payments::entities::webhook_event::webhook_event::events::Saving`, `payments::entities::webhook_event::webhook_event::events::Trashed`, `payments::entities::webhook_event::webhook_event::events::Updated`, `payments::entities::webhook_event::webhook_event::events::Updating`, `queue::events::JobAttempted`, `queue::events::JobDebounced`, `queue::events::JobExceptionOccurred`, `queue::events::JobFailed`, `queue::events::JobProcessed`, `queue::events::JobProcessing`, `queue::events::JobQueued`, `queue::events::JobQueueing`, `queue::events::JobReleased`, `queue::events::JobReleasedAfterException`, `queue::events::JobTimedOut`, `queue::events::Looping`, `queue::events::QueueFailedOver`, `queue::events::QueuePaused`, `queue::events::QueueResumed`, `queue::events::QueuesPaused`, `queue::events::QueuesResumed`, `queue::events::UniqueJobSkipped`, `queue::events::WorkerInterrupted`, `queue::events::WorkerQueuePaused`, `queue::events::WorkerQueueResumed`, `queue::events::WorkerStarting`, `queue::events::WorkerStopping`, `rbac::entity::model_permission::events::Created`, `rbac::entity::model_permission::events::Creating`, `rbac::entity::model_permission::events::Deleted`, `rbac::entity::model_permission::events::Deleting`, `rbac::entity::model_permission::events::ForceDeleted`, `rbac::entity::model_permission::events::ForceDeleting`, `rbac::entity::model_permission::events::Replicating`, `rbac::entity::model_permission::events::Restored`, `rbac::entity::model_permission::events::Restoring`, `rbac::entity::model_permission::events::Retrieved`, `rbac::entity::model_permission::events::Retrieving`, `rbac::entity::model_permission::events::Saved`, `rbac::entity::model_permission::events::Saving`, `rbac::entity::model_permission::events::Trashed`, `rbac::entity::model_permission::events::Updated`, `rbac::entity::model_permission::events::Updating`, `rbac::entity::model_role::events::Created`, `rbac::entity::model_role::events::Creating`, `rbac::entity::model_role::events::Deleted`, `rbac::entity::model_role::events::Deleting`, `rbac::entity::model_role::events::ForceDeleted`, `rbac::entity::model_role::events::ForceDeleting`, `rbac::entity::model_role::events::Replicating`, `rbac::entity::model_role::events::Restored`, `rbac::entity::model_role::events::Restoring`, `rbac::entity::model_role::events::Retrieved`, `rbac::entity::model_role::events::Retrieving`, `rbac::entity::model_role::events::Saved`, `rbac::entity::model_role::events::Saving`, `rbac::entity::model_role::events::Trashed`, `rbac::entity::model_role::events::Updated`, `rbac::entity::model_role::events::Updating`, `rbac::entity::permission::events::Created`, `rbac::entity::permission::events::Creating`, `rbac::entity::permission::events::Deleted`, `rbac::entity::permission::events::Deleting`, `rbac::entity::permission::events::ForceDeleted`, `rbac::entity::permission::events::ForceDeleting`, `rbac::entity::permission::events::Replicating`, `rbac::entity::permission::events::Restored`, `rbac::entity::permission::events::Restoring`, `rbac::entity::permission::events::Retrieved`, `rbac::entity::permission::events::Retrieving`, `rbac::entity::permission::events::Saved`, `rbac::entity::permission::events::Saving`, `rbac::entity::permission::events::Trashed`, `rbac::entity::permission::events::Updated`, `rbac::entity::permission::events::Updating`, `rbac::entity::role::events::Created`, `rbac::entity::role::events::Creating`, `rbac::entity::role::events::Deleted`, `rbac::entity::role::events::Deleting`, `rbac::entity::role::events::ForceDeleted`, `rbac::entity::role::events::ForceDeleting`, `rbac::entity::role::events::Replicating`, `rbac::entity::role::events::Restored`, `rbac::entity::role::events::Restoring`, `rbac::entity::role::events::Retrieved`, `rbac::entity::role::events::Retrieving`, `rbac::entity::role::events::Saved`, `rbac::entity::role::events::Saving`, `rbac::entity::role::events::Trashed`, `rbac::entity::role::events::Updated`, `rbac::entity::role::events::Updating`, `rbac::entity::role_permission::events::Created`, `rbac::entity::role_permission::events::Creating`, `rbac::entity::role_permission::events::Deleted`, `rbac::entity::role_permission::events::Deleting`, `rbac::entity::role_permission::events::ForceDeleted`, `rbac::entity::role_permission::events::ForceDeleting`, `rbac::entity::role_permission::events::Replicating`, `rbac::entity::role_permission::events::Restored`, `rbac::entity::role_permission::events::Restoring`, `rbac::entity::role_permission::events::Retrieved`, `rbac::entity::role_permission::events::Retrieving`, `rbac::entity::role_permission::events::Saved`, `rbac::entity::role_permission::events::Saving`, `rbac::entity::role_permission::events::Trashed`, `rbac::entity::role_permission::events::Updated`, `rbac::entity::role_permission::events::Updating`
  - [ ] fn `suprnova::Event::event_name` · framework/src/events/mod.rs:58 (required)
  - [ ] fn `suprnova::Event::queued` · framework/src/events/mod.rs:64 (provided)
- [ ] trait `suprnova::Listener` · framework/src/events/mod.rs:74 (also `suprnova::events::Listener`)
  - Implemented here by: `BroadcastListener`, `DebouncedListener`, `QueuedListener`
  - [ ] fn `suprnova::Listener::handle` · framework/src/events/mod.rs:77 (required)
- [ ] trait `suprnova::Subscriber` · framework/src/events/mod.rs:121 (also `suprnova::events::Subscriber`)
  - [ ] fn `suprnova::Subscriber::subscribe` · framework/src/events/mod.rs:125 (required)

### `suprnova::events::builtins` (private module; items are public through re-exports)

- [ ] struct `suprnova::ErrorOccurred` · framework/src/events/builtins.rs:17 (also `suprnova::events::ErrorOccurred`)
  - Public fields: `error_message`, `status_code`, `request_id`
  - Implements: `suprnova::Event`

### `suprnova::events::dispatcher` (private module; items are public through re-exports)

- [ ] struct `suprnova::EventDispatcher` · framework/src/events/dispatcher.rs:96 (also `suprnova::events::EventDispatcher`)
  - [ ] fn `suprnova::EventDispatcher::new` · framework/src/events/dispatcher.rs:111
  - [ ] fn `suprnova::EventDispatcher::with_concurrency` · framework/src/events/dispatcher.rs:122
  - [ ] fn `suprnova::EventDispatcher::listen` · framework/src/events/dispatcher.rs:145
  - [ ] fn `suprnova::EventDispatcher::dispatch` · framework/src/events/dispatcher.rs:203
  - [ ] fn `suprnova::EventDispatcher::dispatch_best_effort` · framework/src/events/dispatcher.rs:318
  - [ ] fn `suprnova::EventDispatcher::drain_queued` · framework/src/events/dispatcher.rs:480
  - [ ] fn `suprnova::EventDispatcher::has_listeners` · framework/src/events/dispatcher.rs:537
  - [ ] fn `suprnova::EventDispatcher::forget` · framework/src/events/dispatcher.rs:561
  - [ ] fn `suprnova::EventDispatcher::push` · framework/src/events/dispatcher.rs:584
  - [ ] fn `suprnova::EventDispatcher::flush` · framework/src/events/dispatcher.rs:603
  - [ ] fn `suprnova::EventDispatcher::forget_pushed` · framework/src/events/dispatcher.rs:625
  - [ ] fn `suprnova::EventDispatcher::defer` · framework/src/events/dispatcher.rs:643
  - [ ] fn `suprnova::EventDispatcher::subscribe` · framework/src/events/dispatcher.rs:681
- [ ] struct `suprnova::EventFacade` · framework/src/events/dispatcher.rs:713 (also `suprnova::events::EventFacade`)
  - [ ] fn `suprnova::EventFacade::dispatch` · framework/src/events/dispatcher.rs:719
  - [ ] fn `suprnova::EventFacade::dispatch_best_effort` · framework/src/events/dispatcher.rs:729
  - [ ] fn `suprnova::EventFacade::listen` · framework/src/events/dispatcher.rs:738
  - [ ] fn `suprnova::EventFacade::broadcast` · framework/src/events/dispatcher.rs:750
  - [ ] fn `suprnova::EventFacade::drain_queued` · framework/src/events/dispatcher.rs:762
  - [ ] fn `suprnova::EventFacade::has_listeners` · framework/src/events/dispatcher.rs:768
  - [ ] fn `suprnova::EventFacade::forget` · framework/src/events/dispatcher.rs:775
  - [ ] fn `suprnova::EventFacade::push` · framework/src/events/dispatcher.rs:785
  - [ ] fn `suprnova::EventFacade::flush` · framework/src/events/dispatcher.rs:796
  - [ ] fn `suprnova::EventFacade::forget_pushed` · framework/src/events/dispatcher.rs:805
  - [ ] fn `suprnova::EventFacade::defer` · framework/src/events/dispatcher.rs:814
  - [ ] fn `suprnova::EventFacade::subscribe` · framework/src/events/dispatcher.rs:827
  - [ ] fn `suprnova::EventFacade::muted` · framework/src/events/dispatcher.rs:836
  - [ ] fn `suprnova::EventFacade::fake` · framework/src/events/dispatcher.rs:846
  - [ ] fn `suprnova::EventFacade::fake_only` · framework/src/events/dispatcher.rs:854
  - [ ] fn `suprnova::EventFacade::fake_except` · framework/src/events/dispatcher.rs:861

### `suprnova::events::queued_listener` (private module; items are public through re-exports)

- [ ] struct `suprnova::DebouncedListener` · framework/src/events/queued_listener.rs:131 (also `suprnova::events::DebouncedListener`)
  - Implements: `suprnova::Listener`
  - [ ] fn `suprnova::DebouncedListener::new` · framework/src/events/queued_listener.rs:146
  - [ ] fn `suprnova::DebouncedListener::max_wait` · framework/src/events/queued_listener.rs:160
  - [ ] fn `suprnova::DebouncedListener::keyed_by` · framework/src/events/queued_listener.rs:168
- [ ] struct `suprnova::QueuedListener` · framework/src/events/queued_listener.rs:58 (also `suprnova::events::QueuedListener`)
  - Implements: `suprnova::Listener`
  - [ ] fn `suprnova::QueuedListener::new` · framework/src/events/queued_listener.rs:69

### `suprnova::events::testing`

- [ ] fn `suprnova::events::assert_dispatched` · framework/src/events/testing.rs:266 (also `suprnova::events::testing::assert_dispatched`)
- [ ] fn `suprnova::events::assert_dispatched_once` · framework/src/events/testing.rs:308 (also `suprnova::events::testing::assert_dispatched_once`)
- [ ] fn `suprnova::events::assert_dispatched_times` · framework/src/events/testing.rs:314 (also `suprnova::events::testing::assert_dispatched_times`)
- [ ] fn `suprnova::events::assert_listening` · framework/src/events/testing.rs:392 (also `suprnova::events::testing::assert_listening`)
- [ ] fn `suprnova::events::assert_not_dispatched` · framework/src/events/testing.rs:276 (also `suprnova::events::testing::assert_not_dispatched`)
- [ ] fn `suprnova::events::assert_nothing_dispatched` · framework/src/events/testing.rs:328 (also `suprnova::events::testing::assert_nothing_dispatched`)
- [ ] fn `suprnova::events::dispatched` · framework/src/events/testing.rs:354 (also `suprnova::events::testing::dispatched`)
- [ ] fn `suprnova::events::dispatched_count` · framework/src/events/testing.rs:288 (also `suprnova::events::testing::dispatched_count`)
- [ ] fn `suprnova::events::dispatched_events` · framework/src/events/testing.rs:377 (also `suprnova::events::testing::dispatched_events`)
- [ ] fn `suprnova::events::has_dispatched` · framework/src/events/testing.rs:338 (also `suprnova::events::testing::has_dispatched`)
- [ ] fn `suprnova::events::testing::install_fake` · framework/src/events/testing.rs:198
- [ ] fn `suprnova::events::testing::install_fake_except` · framework/src/events/testing.rs:224
- [ ] fn `suprnova::events::testing::install_fake_only` · framework/src/events/testing.rs:210
- [ ] fn `suprnova::events::testing::muted` · framework/src/events/testing.rs:244
- [ ] struct `suprnova::EventFakeGuard` · framework/src/events/testing.rs:254 (also `suprnova::events::EventFakeGuard`, `suprnova::events::testing::EventFakeGuard`)

## factory

### `suprnova::factory`

- [ ] struct `suprnova::FactoryBuilder` · framework/src/factory/mod.rs:107 (also `suprnova::factory::FactoryBuilder`)
  - [ ] fn `suprnova::FactoryBuilder::count` · framework/src/factory/mod.rs:117
  - [ ] fn `suprnova::FactoryBuilder::with` · framework/src/factory/mod.rs:126
  - [ ] fn `suprnova::FactoryBuilder::prepend` · framework/src/factory/mod.rs:141
  - [ ] fn `suprnova::FactoryBuilder::when` · framework/src/factory/mod.rs:161
  - [ ] fn `suprnova::FactoryBuilder::make` · framework/src/factory/mod.rs:172
  - [ ] fn `suprnova::FactoryBuilder::make_one` · framework/src/factory/mod.rs:186
  - [ ] fn `suprnova::FactoryBuilder::make_many` · framework/src/factory/mod.rs:193
  - [ ] fn `suprnova::FactoryBuilder::create` · framework/src/factory/mod.rs:221
  - [ ] fn `suprnova::FactoryBuilder::create_one` · framework/src/factory/mod.rs:230
  - [ ] fn `suprnova::FactoryBuilder::create_many` · framework/src/factory/mod.rs:238
- [ ] trait `suprnova::Factory` · framework/src/factory/mod.rs:59 (also `suprnova::factory::Factory`)
  - [ ] type `suprnova::Factory::Model` · framework/src/factory/mod.rs:61
  - [ ] fn `suprnova::Factory::definition` · framework/src/factory/mod.rs:67 (required)
  - [ ] fn `suprnova::Factory::new` · framework/src/factory/mod.rs:73 (provided)
  - [ ] fn `suprnova::Factory::times` · framework/src/factory/mod.rs:87 (provided)

### `suprnova::factory::persist` (private module; items are public through re-exports)

- [ ] fn `suprnova::persist_via_seaorm` · framework/src/factory/persist.rs:80 (also `suprnova::factory::persist_via_seaorm`)
- [ ] trait `suprnova::Persistable` · framework/src/factory/persist.rs:49 (also `suprnova::factory::Persistable`)
  - Implemented here by: `features::entity::Feature`, `payments::entities::customer::Customer`, `payments::entities::payment_method::PaymentMethod`, `payments::entities::subscription::Subscription`, `payments::entities::subscription_item::SubscriptionItem`, `payments::entities::transaction::Transaction`, `payments::entities::webhook_event::WebhookEvent`, `rbac::entity::ModelPermission`, `rbac::entity::ModelRole`, `rbac::entity::Permission`, `rbac::entity::Role`, `rbac::entity::RolePermission`
  - [ ] fn `suprnova::Persistable::persist` · framework/src/factory/persist.rs:53 (required)

### `suprnova::factory::sequence` (private module; items are public through re-exports)

- [ ] struct `suprnova::Sequence` · framework/src/factory/sequence.rs:36 (also `suprnova::factory::Sequence`)
  - [ ] fn `suprnova::Sequence::new` · framework/src/factory/sequence.rs:43
  - [ ] fn `suprnova::Sequence::next` · framework/src/factory/sequence.rs:51
  - [ ] fn `suprnova::Sequence::reset` · framework/src/factory/sequence.rs:56

## features

### `suprnova::features::admin`

- [ ] fn `suprnova::features::admin::delete` · framework/src/features/admin.rs:183
- [ ] fn `suprnova::features::admin::get` · framework/src/features/admin.rs:90
- [ ] fn `suprnova::features::admin::list` · framework/src/features/admin.rs:77
- [ ] fn `suprnova::features::admin::upsert` · framework/src/features/admin.rs:113
- [ ] struct `suprnova::features::admin::FeatureRow` · framework/src/features/admin.rs:29
  - Public fields: `id`, `name`, `scope_key`, `enabled`, `description`, `updated_by`, `created_at`, `updated_at`

### `suprnova::features::bootstrap`

- [ ] fn `suprnova::features::bootstrap_database_cached` · framework/src/features/bootstrap.rs:165 (also `suprnova::features::bootstrap::bootstrap_database_cached`)
- [ ] fn `suprnova::features::install_evaluator` · framework/src/features/bootstrap.rs:95 (also `suprnova::features::bootstrap::install_evaluator`)
- [ ] fn `suprnova::features::is_installed` · framework/src/features/bootstrap.rs:77 (also `suprnova::features::bootstrap::is_installed`)
- [ ] fn `suprnova::features::mark_installed` · framework/src/features/bootstrap.rs:70 (also `suprnova::features::bootstrap::mark_installed`)
- [ ] struct `suprnova::features::BootstrappedFeatures` · framework/src/features/bootstrap.rs:120 (also `suprnova::features::bootstrap::BootstrappedFeatures`)
  - Public fields: `database`, `cached`

### `suprnova::features::entity`

- [ ] struct `suprnova::features::entity::Feature` · framework/src/features/entity.rs:29
  - Public fields: `id`, `name`, `scope_key`, `enabled`, `description`, `updated_by`, `created_at`, `updated_at`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`, `suprnova::Touchable`
  - [ ] fn `suprnova::features::entity::Feature::fill` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::without_global_scope` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::without_global_scopes` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::on` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::on_write_connection` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::count` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::sum` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::avg` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::min` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::max` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::pluck` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::pluck_keyed` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::filter` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::db_where` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::where_in` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::where_like` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::latest` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::oldest` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::pivot` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::with` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::with_count` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::with_sum` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::with_avg` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::with_min` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::with_max` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Feature::observe` · framework/src/features/entity.rs:28

### `suprnova::features::entity::feature`

- [ ] struct `suprnova::features::entity::ActiveModel` · framework/src/features/entity.rs:28 (also `suprnova::features::entity::feature::ActiveModel`)
  - Public fields: `id`, `name`, `scope_key`, `enabled`, `description`, `updated_by`, `created_at`, `updated_at`
- [ ] struct `suprnova::features::entity::feature::ColumnIter` · framework/src/features/entity.rs:28
- [ ] struct `suprnova::features::entity::Entity` · framework/src/features/entity.rs:28 (also `suprnova::features::entity::feature::Entity`)
- [ ] struct `suprnova::features::entity::Model` · framework/src/features/entity.rs:28 (also `suprnova::features::entity::feature::Model`)
  - Public fields: `id`, `name`, `scope_key`, `enabled`, `description`, `updated_by`, `created_at`, `updated_at`
  - [ ] fn `suprnova::features::entity::Model::into_ex` · framework/src/features/entity.rs:28
- [ ] struct `suprnova::features::entity::feature::PrimaryKeyIter` · framework/src/features/entity.rs:28
- [ ] struct `suprnova::features::entity::feature::RelationIter` · framework/src/features/entity.rs:28
- [ ] enum `suprnova::features::entity::Column` · framework/src/features/entity.rs:28 (also `suprnova::features::entity::feature::Column`)
  - Variants: `Id`, `Name`, `ScopeKey`, `Enabled`, `Description`, `UpdatedBy`, `CreatedAt`, `UpdatedAt`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::features::entity::Column::as_str` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Column::from_name` · framework/src/features/entity.rs:28
  - [ ] fn `suprnova::features::entity::Column::iter` · framework/src/features/entity.rs:28
- [ ] enum `suprnova::features::entity::feature::PrimaryKey` · framework/src/features/entity.rs:28
  - Variants: `Id`
- [ ] enum `suprnova::features::entity::feature::Relation` · framework/src/features/entity.rs:28
- [ ] type `suprnova::features::entity::feature::__Suprnova_Cast_Storage_created_at` · framework/src/features/entity.rs:28
- [ ] type `suprnova::features::entity::feature::__Suprnova_Cast_Storage_updated_at` · framework/src/features/entity.rs:28

### `suprnova::features::entity::feature::events`

- [ ] struct `suprnova::features::entity::feature::events::Created` · framework/src/features/entity.rs:28
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Creating` · framework/src/features/entity.rs:28
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Deleted` · framework/src/features/entity.rs:28
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Deleting` · framework/src/features/entity.rs:28
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::ForceDeleted` · framework/src/features/entity.rs:28
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::ForceDeleting` · framework/src/features/entity.rs:28
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Replicating` · framework/src/features/entity.rs:28
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Restored` · framework/src/features/entity.rs:28
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Restoring` · framework/src/features/entity.rs:28
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Retrieved` · framework/src/features/entity.rs:28
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Retrieving` · framework/src/features/entity.rs:28
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Saved` · framework/src/features/entity.rs:28
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Saving` · framework/src/features/entity.rs:28
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Trashed` · framework/src/features/entity.rs:28
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Updated` · framework/src/features/entity.rs:28
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::entity::feature::events::Updating` · framework/src/features/entity.rs:28
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::features::evaluators::cached`

- [ ] struct `suprnova::features::CachedEvaluator` · framework/src/features/evaluators/cached.rs:75 (also `suprnova::features::evaluators::cached::CachedEvaluator`)
  - Implements: `suprnova::features::FeatureSync`
  - [ ] fn `suprnova::features::CachedEvaluator::new` · framework/src/features/evaluators/cached.rs:110
  - [ ] fn `suprnova::features::CachedEvaluator::inner` · framework/src/features/evaluators/cached.rs:121
  - [ ] fn `suprnova::features::CachedEvaluator::invalidate` · framework/src/features/evaluators/cached.rs:129
  - [ ] fn `suprnova::features::CachedEvaluator::invalidate_all` · framework/src/features/evaluators/cached.rs:136
  - [ ] fn `suprnova::features::CachedEvaluator::len` · framework/src/features/evaluators/cached.rs:142
  - [ ] fn `suprnova::features::CachedEvaluator::is_empty` · framework/src/features/evaluators/cached.rs:147

### `suprnova::features::evaluators::database`

- [ ] struct `suprnova::features::DatabaseEvaluator` · framework/src/features/evaluators/database.rs:85 (also `suprnova::features::evaluators::database::DatabaseEvaluator`)
  - Implements: `suprnova::features::FeatureSync`
  - [ ] fn `suprnova::features::DatabaseEvaluator::new` · framework/src/features/evaluators/database.rs:211
  - [ ] fn `suprnova::features::DatabaseEvaluator::new_in_memory` · framework/src/features/evaluators/database.rs:233
  - [ ] fn `suprnova::features::DatabaseEvaluator::reload` · framework/src/features/evaluators/database.rs:265
  - [ ] fn `suprnova::features::DatabaseEvaluator::set_flag` · framework/src/features/evaluators/database.rs:338

### `suprnova::features::events`

- [ ] struct `suprnova::features::FeatureDeleted` · framework/src/features/events.rs:43 (also `suprnova::features::events::FeatureDeleted`)
  - Public fields: `name`, `scope_key`, `actor_id`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::features::FeatureUpdated` · framework/src/features/events.rs:19 (also `suprnova::features::events::FeatureUpdated`)
  - Public fields: `name`, `scope_key`, `enabled`, `actor_id`
  - Implements: `suprnova::Event`

### `suprnova::features::fields`

- [ ] struct `suprnova::features::TeamField` · framework/src/features/fields.rs:109 (also `suprnova::features::fields::TeamField`)
  - Public tuple fields: 1
  - [ ] fn `suprnova::features::TeamField::new` · framework/src/features/fields.rs:113
  - [ ] fn `suprnova::features::TeamField::as_str` · framework/src/features/fields.rs:118
- [ ] struct `suprnova::features::UserIdField` · framework/src/features/fields.rs:71 (also `suprnova::features::fields::UserIdField`)
  - Public tuple fields: 1
  - [ ] fn `suprnova::features::UserIdField::new` · framework/src/features/fields.rs:76
  - [ ] fn `suprnova::features::UserIdField::from_i64` · framework/src/features/fields.rs:82
  - [ ] fn `suprnova::features::UserIdField::as_str` · framework/src/features/fields.rs:87
  - [ ] fn `suprnova::features::UserIdField::as_i64` · framework/src/features/fields.rs:94

### `suprnova::features::middleware`

- [ ] struct `suprnova::features::FeatureMiddleware` · framework/src/features/middleware.rs:87 (also `suprnova::features::middleware::FeatureMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::features::FeatureMiddleware::new` · framework/src/features/middleware.rs:95
  - [ ] fn `suprnova::features::FeatureMiddleware::with_user_id_extractor` · framework/src/features/middleware.rs:105
  - [ ] fn `suprnova::features::FeatureMiddleware::with_team_extractor` · framework/src/features/middleware.rs:116
  - [ ] fn `suprnova::features::FeatureMiddleware::with_team_from_header` · framework/src/features/middleware.rs:130

### `suprnova::features::migrations::m_create_features_table`

- [ ] struct `suprnova::features::migrations::CreateFeaturesTable` · framework/src/features/migrations/m_create_features_table.rs:30 (also `suprnova::features::migrations::m_create_features_table::Migration`)

### `suprnova::features::sync`

- [ ] fn `suprnova::features::sync::notify` · framework/src/features/sync.rs:186
- [ ] fn `suprnova::features::sync::notify_reloaded` · framework/src/features/sync.rs:196
- [ ] struct `suprnova::features::CompositeFeatureSync` · framework/src/features/sync.rs:133 (also `suprnova::features::sync::CompositeFeatureSync`)
  - Implements: `suprnova::features::FeatureSync`
  - [ ] fn `suprnova::features::CompositeFeatureSync::new` · framework/src/features/sync.rs:146
- [ ] trait `suprnova::features::FeatureSync` · framework/src/features/sync.rs:108 (also `suprnova::features::sync::FeatureSync`)
  - Implemented here by: `features::CachedEvaluator`, `features::CompositeFeatureSync`, `features::DatabaseEvaluator`
  - [ ] fn `suprnova::features::FeatureSync::on_flag_changed` · framework/src/features/sync.rs:114 (required)
  - [ ] fn `suprnova::features::FeatureSync::on_snapshot_reloaded` · framework/src/features/sync.rs:124 (provided)

## filesystem

### `suprnova::filesystem` (feature: `filesystem`)

- [ ] struct `suprnova::AzBlobConfig` · framework/src/filesystem/mod.rs:177 (feature: `filesystem-azure`, off by default; also `suprnova::filesystem::AzBlobConfig`)
  - Public fields: `container`, `account_name`, `account_key`, `endpoint`, `root`
- [ ] struct `suprnova::GcsConfig` · framework/src/filesystem/mod.rs:223 (feature: `filesystem-gcs`, off by default; also `suprnova::filesystem::GcsConfig`)
  - Public fields: `bucket`, `credential`, `credential_path`, `endpoint`, `root`
- [ ] struct `suprnova::ReadThroughConfig` · framework/src/filesystem/mod.rs:320 (also `suprnova::filesystem::ReadThroughConfig`)
  - Public fields: `primary`, `fallback`, `copy`, `throw_on_promotion_failure`
- [ ] struct `suprnova::S3Config` · framework/src/filesystem/mod.rs:137 (also `suprnova::filesystem::S3Config`)
  - Public fields: `bucket`, `region`, `endpoint`, `access_key_id`, `secret_access_key`, `root`
- [ ] struct `suprnova::Storage` · framework/src/filesystem/mod.rs:124 (also `suprnova::filesystem::Storage`)
  - [ ] fn `suprnova::Storage::disk` · framework/src/filesystem/mod.rs:387
  - [ ] fn `suprnova::Storage::register_fs` · framework/src/filesystem/mod.rs:427
  - [ ] fn `suprnova::Storage::register_fs_with` · framework/src/filesystem/mod.rs:479
  - [ ] fn `suprnova::Storage::register_memory` · framework/src/filesystem/mod.rs:514
  - [ ] fn `suprnova::Storage::register_memory_with` · framework/src/filesystem/mod.rs:537
  - [ ] fn `suprnova::Storage::register_s3` · framework/src/filesystem/mod.rs:560
  - [ ] fn `suprnova::Storage::register_s3_with` · framework/src/filesystem/mod.rs:599
  - [ ] fn `suprnova::Storage::register_azblob` · framework/src/filesystem/mod.rs:648
  - [ ] fn `suprnova::Storage::register_azblob_with` · framework/src/filesystem/mod.rs:664
  - [ ] fn `suprnova::Storage::register_gcs` · framework/src/filesystem/mod.rs:716
  - [ ] fn `suprnova::Storage::register_gcs_with` · framework/src/filesystem/mod.rs:729
  - [ ] fn `suprnova::Storage::register_read_through` · framework/src/filesystem/mod.rs:803
  - [ ] fn `suprnova::Storage::register_read_through_with` · framework/src/filesystem/mod.rs:821
  - [ ] fn `suprnova::Storage::forget` · framework/src/filesystem/mod.rs:874
  - [ ] fn `suprnova::Storage::purge` · framework/src/filesystem/mod.rs:884
  - [ ] fn `suprnova::Storage::disks` · framework/src/filesystem/mod.rs:892
  - [ ] fn `suprnova::Storage::fake` · framework/src/filesystem/mod.rs:908
- [ ] const `suprnova::ATOMIC_STAGING_DIR` · framework/src/filesystem/mod.rs:72 (also `suprnova::filesystem::ATOMIC_STAGING_DIR`)

### `suprnova::filesystem::disk` (private module; items are public through re-exports)

- [ ] enum `suprnova::ChecksumAlgorithm` · framework/src/filesystem/disk.rs:46 (feature: `filesystem`; also `suprnova::filesystem::ChecksumAlgorithm`)
  - Variants: `Md5`, `Sha1`, `Sha256`
- [ ] trait `suprnova::DiskExt` · framework/src/filesystem/disk.rs:69 (feature: `filesystem`; also `suprnova::filesystem::DiskExt`)
  - Implemented here by: `opendal::Operator`
  - [ ] fn `suprnova::DiskExt::missing` · framework/src/filesystem/disk.rs:75 (required)
  - [ ] fn `suprnova::DiskExt::file_exists` · framework/src/filesystem/disk.rs:82 (required)
  - [ ] fn `suprnova::DiskExt::file_missing` · framework/src/filesystem/disk.rs:85 (required)
  - [ ] fn `suprnova::DiskExt::directory_exists` · framework/src/filesystem/disk.rs:94 (required)
  - [ ] fn `suprnova::DiskExt::directory_missing` · framework/src/filesystem/disk.rs:100 (required)
  - [ ] fn `suprnova::DiskExt::get` · framework/src/filesystem/disk.rs:109 (required)
  - [ ] fn `suprnova::DiskExt::put` · framework/src/filesystem/disk.rs:112 (required)
  - [ ] fn `suprnova::DiskExt::json` · framework/src/filesystem/disk.rs:122 (required)
  - [ ] fn `suprnova::DiskExt::put_json` · framework/src/filesystem/disk.rs:131 (required)
  - [ ] fn `suprnova::DiskExt::prepend` · framework/src/filesystem/disk.rs:140 (required)
  - [ ] fn `suprnova::DiskExt::prepend_with_separator` · framework/src/filesystem/disk.rs:148 (required)
  - [ ] fn `suprnova::DiskExt::append` · framework/src/filesystem/disk.rs:156 (required)
  - [ ] fn `suprnova::DiskExt::append_with_separator` · framework/src/filesystem/disk.rs:164 (required)
  - [ ] fn `suprnova::DiskExt::size` · framework/src/filesystem/disk.rs:175 (required)
  - [ ] fn `suprnova::DiskExt::last_modified` · framework/src/filesystem/disk.rs:179 (required)
  - [ ] fn `suprnova::DiskExt::mime_type` · framework/src/filesystem/disk.rs:192 (required)
  - [ ] fn `suprnova::DiskExt::checksum` · framework/src/filesystem/disk.rs:200 (required)
  - [ ] fn `suprnova::DiskExt::files` · framework/src/filesystem/disk.rs:211 (required)
  - [ ] fn `suprnova::DiskExt::all_files` · framework/src/filesystem/disk.rs:218 (required)
  - [ ] fn `suprnova::DiskExt::directories` · framework/src/filesystem/disk.rs:225 (required)
  - [ ] fn `suprnova::DiskExt::all_directories` · framework/src/filesystem/disk.rs:232 (required)
  - [ ] fn `suprnova::DiskExt::make_directory` · framework/src/filesystem/disk.rs:238 (required)
  - [ ] fn `suprnova::DiskExt::delete_directory` · framework/src/filesystem/disk.rs:243 (required)
  - [ ] fn `suprnova::DiskExt::move_to` · framework/src/filesystem/disk.rs:253 (required)
  - [ ] fn `suprnova::DiskExt::temporary_url` · framework/src/filesystem/disk.rs:266 (required)
  - [ ] fn `suprnova::DiskExt::temporary_upload_url` · framework/src/filesystem/disk.rs:277 (required)

### `suprnova::filesystem::streaming` (feature: `filesystem`)

- [ ] fn `suprnova::copy_between_disks` · framework/src/filesystem/streaming.rs:57 (also `suprnova::filesystem::copy_between_disks`, `suprnova::filesystem::streaming::copy_between_disks`)

### `suprnova::filesystem::testing` (feature: `filesystem`)

- [ ] struct `suprnova::filesystem::testing::StorageFakeGuard` · framework/src/filesystem/testing.rs:34
- [ ] trait `suprnova::filesystem::testing::DiskAssertExt` · framework/src/filesystem/testing.rs:65
  - Implemented here by: `opendal::Operator`
  - [ ] fn `suprnova::filesystem::testing::DiskAssertExt::assert_exists` · framework/src/filesystem/testing.rs:68 (required)
  - [ ] fn `suprnova::filesystem::testing::DiskAssertExt::assert_contents` · framework/src/filesystem/testing.rs:72 (required)
  - [ ] fn `suprnova::filesystem::testing::DiskAssertExt::assert_missing` · framework/src/filesystem/testing.rs:79 (required)
  - [ ] fn `suprnova::filesystem::testing::DiskAssertExt::assert_count` · framework/src/filesystem/testing.rs:84 (required)
  - [ ] fn `suprnova::filesystem::testing::DiskAssertExt::assert_directory_empty` · framework/src/filesystem/testing.rs:93 (required)

## hashing

### `suprnova::hashing`

- [ ] fn `suprnova::hashing::default_driver` · framework/src/hashing/mod.rs:133
- [ ] fn `suprnova::hash` · framework/src/hashing/mod.rs:171 (also `suprnova::hashing::hash`)
- [ ] fn `suprnova::hashing::hash_async` · framework/src/hashing/mod.rs:325
- [ ] fn `suprnova::hash_info` · framework/src/hashing/mod.rs:389 (also `suprnova::hashing::info`)
- [ ] fn `suprnova::hashing::hash_with` · framework/src/hashing/mod.rs:178
- [ ] fn `suprnova::hashing::hash_with_cost` · framework/src/hashing/mod.rs:195
- [ ] fn `suprnova::hashing::hash_with_cost_async` · framework/src/hashing/mod.rs:333
- [ ] fn `suprnova::needs_rehash` · framework/src/hashing/mod.rs:369 (also `suprnova::hashing::needs_rehash`)
- [ ] fn `suprnova::hashing::needs_rehash_with` · framework/src/hashing/mod.rs:378
- [ ] fn `suprnova::hashing::set_default_driver` · framework/src/hashing/mod.rs:154
- [ ] fn `suprnova::verify` · framework/src/hashing/mod.rs:227 (also `suprnova::hashing::verify`)
- [ ] fn `suprnova::hashing::verify_async` · framework/src/hashing/mod.rs:341
- [ ] fn `suprnova::hashing::verify_with` · framework/src/hashing/mod.rs:237
- [ ] const `suprnova::HASH_DEFAULT_COST` · framework/src/hashing/mod.rs:93 (also `suprnova::hashing::DEFAULT_COST`)
- [ ] const `suprnova::HASH_DEFAULT_ROUNDS` · framework/src/hashing/mod.rs:96 (also `suprnova::hashing::DEFAULT_ROUNDS`)
- [ ] const `suprnova::hashing::MAX_BCRYPT_COST` · framework/src/hashing/mod.rs:108
- [ ] const `suprnova::MAX_BCRYPT_PASSWORD_BYTES` · framework/src/hashing/mod.rs:119 (also `suprnova::hashing::MAX_BCRYPT_PASSWORD_BYTES`)
- [ ] const `suprnova::hashing::MAX_PASSWORD_BYTES` · framework/src/hashing/mod.rs:123
- [ ] const `suprnova::hashing::MIN_BCRYPT_COST` · framework/src/hashing/mod.rs:100

### `suprnova::hashing::config` (private module; items are public through re-exports)

- [ ] struct `suprnova::HashConfig` · framework/src/hashing/config.rs:55 (also `suprnova::hashing::HashConfig`)
  - Public fields: `driver`, `rounds`, `memory`, `time`, `threads`, `verify_algorithm`
  - [ ] fn `suprnova::HashConfig::from_env` · framework/src/hashing/config.rs:96
- [ ] enum `suprnova::HashAlgorithm` · framework/src/hashing/config.rs:15 (also `suprnova::hashing::Algorithm`)
  - Variants: `Bcrypt`, `Argon2i`, `Argon2id`
  - [ ] fn `suprnova::HashAlgorithm::as_str` · framework/src/hashing/config.rs:31
  - [ ] fn `suprnova::HashAlgorithm::parse` · framework/src/hashing/config.rs:42

### `suprnova::hashing::driver` (private module; items are public through re-exports)

- [ ] struct `suprnova::Argon2idHasher` · framework/src/hashing/driver.rs:287 (also `suprnova::hashing::Argon2idHasher`)
  - Implements: `suprnova::Hasher`
  - [ ] fn `suprnova::Argon2idHasher::new` · framework/src/hashing/driver.rs:296
  - [ ] fn `suprnova::Argon2idHasher::with_verify_algorithm` · framework/src/hashing/driver.rs:307
- [ ] struct `suprnova::Argon2iHasher` · framework/src/hashing/driver.rs:226 (also `suprnova::hashing::Argon2iHasher`)
  - Implements: `suprnova::Hasher`
  - [ ] fn `suprnova::Argon2iHasher::new` · framework/src/hashing/driver.rs:235
  - [ ] fn `suprnova::Argon2iHasher::with_verify_algorithm` · framework/src/hashing/driver.rs:247
- [ ] struct `suprnova::Argon2Options` · framework/src/hashing/driver.rs:205 (also `suprnova::hashing::Argon2Options`)
  - Public fields: `memory`, `time`, `threads`
- [ ] struct `suprnova::BcryptHasher` · framework/src/hashing/driver.rs:98 (also `suprnova::hashing::BcryptHasher`)
  - Implements: `suprnova::Hasher`
  - [ ] fn `suprnova::BcryptHasher::new` · framework/src/hashing/driver.rs:106
  - [ ] fn `suprnova::BcryptHasher::with_verify_algorithm` · framework/src/hashing/driver.rs:114
- [ ] struct `suprnova::BcryptOptions` · framework/src/hashing/driver.rs:80 (also `suprnova::hashing::BcryptOptions`)
  - Public fields: `rounds`
- [ ] trait `suprnova::Hasher` · framework/src/hashing/driver.rs:18 (also `suprnova::hashing::Hasher`)
  - Implemented here by: `Argon2iHasher`, `Argon2idHasher`, `BcryptHasher`
  - [ ] fn `suprnova::Hasher::algorithm` · framework/src/hashing/driver.rs:22 (required)
  - [ ] fn `suprnova::Hasher::hash` · framework/src/hashing/driver.rs:29 (required)
  - [ ] fn `suprnova::Hasher::verify` · framework/src/hashing/driver.rs:43 (required)
  - [ ] fn `suprnova::Hasher::needs_rehash` · framework/src/hashing/driver.rs:51 (required)
  - [ ] fn `suprnova::Hasher::verify_algorithm` · framework/src/hashing/driver.rs:56 (provided)

### `suprnova::hashing::info` (private module; items are public through re-exports)

- [ ] fn `suprnova::is_hashed` · framework/src/hashing/info.rs:106 (also `suprnova::hashing::is_hashed`)
- [ ] fn `suprnova::hashing::parse` · framework/src/hashing/info.rs:84
- [ ] struct `suprnova::HashInfo` · framework/src/hashing/info.rs:20 (also `suprnova::hashing::HashInfo`)
  - Public fields: `algo`, `rounds`, `memory`, `time`, `threads`, `bcrypt_variant`
- [ ] enum `suprnova::hashing::AlgoName` · framework/src/hashing/info.rs:41
  - Variants: `Bcrypt`, `Argon2i`, `Argon2id`, `Argon2d`, `Unknown`
  - [ ] fn `suprnova::hashing::AlgoName::supported` · framework/src/hashing/info.rs:59
  - [ ] fn `suprnova::hashing::AlgoName::as_str` · framework/src/hashing/info.rs:69

## http

### `suprnova::http`

- [ ] fn `suprnova::json` · framework/src/http/mod.rs:68 (also `suprnova::http::json`)
- [ ] fn `suprnova::text` · framework/src/http/mod.rs:63 (also `suprnova::http::text`)
- [ ] struct `suprnova::http::ParamError` · framework/src/http/mod.rs:33
  - Public fields: `param_name`

### `suprnova::http::abort`

- [ ] fn `suprnova::abort_if` · framework/src/http/abort.rs:49 (also `suprnova::http::abort::abort_if`, `suprnova::http::abort_if`)
- [ ] fn `suprnova::abort_unless` · framework/src/http/abort.rs:63 (also `suprnova::http::abort::abort_unless`, `suprnova::http::abort_unless`)
- [ ] fn `suprnova::abort_with` · framework/src/http/abort.rs:37 (also `suprnova::http::abort::abort`, `suprnova::http::abort_with`)

### `suprnova::http::body`

- [ ] fn `suprnova::http::collect_body` · framework/src/http/body.rs:147 (also `suprnova::http::body::collect_body`)
- [ ] fn `suprnova::collect_body_with_cap` · framework/src/http/body.rs:82 (also `suprnova::http::body::collect_body_with_cap`)
- [ ] fn `suprnova::global_max_request_body_bytes` · framework/src/http/body.rs:54 (also `suprnova::http::body::global_max_request_body_bytes`)
- [ ] fn `suprnova::http::parse_form` · framework/src/http/body.rs:164 (also `suprnova::http::body::parse_form`)
- [ ] fn `suprnova::http::parse_json` · framework/src/http/body.rs:155 (also `suprnova::http::body::parse_json`)
- [ ] fn `suprnova::set_global_max_request_body_bytes` · framework/src/http/body.rs:46 (also `suprnova::http::body::set_global_max_request_body_bytes`)
- [ ] const `suprnova::DEFAULT_MAX_REQUEST_BODY_BYTES` · framework/src/http/body.rs:30 (also `suprnova::http::body::DEFAULT_MAX_REQUEST_BODY_BYTES`)

### `suprnova::http::cookie`

- [ ] fn `suprnova::http::parse_cookies` · framework/src/http/cookie.rs:570 (also `suprnova::http::cookie::parse_cookies`)
- [ ] struct `suprnova::Cookie` · framework/src/http/cookie.rs:239 (also `suprnova::http::Cookie`, `suprnova::http::cookie::Cookie`)
  - [ ] fn `suprnova::Cookie::new` · framework/src/http/cookie.rs:253
  - [ ] fn `suprnova::Cookie::name` · framework/src/http/cookie.rs:262
  - [ ] fn `suprnova::Cookie::value` · framework/src/http/cookie.rs:267
  - [ ] fn `suprnova::Cookie::http_only` · framework/src/http/cookie.rs:274
  - [ ] fn `suprnova::Cookie::secure` · framework/src/http/cookie.rs:282
  - [ ] fn `suprnova::Cookie::same_site` · framework/src/http/cookie.rs:290
  - [ ] fn `suprnova::Cookie::max_age` · framework/src/http/cookie.rs:298
  - [ ] fn `suprnova::Cookie::path` · framework/src/http/cookie.rs:304
  - [ ] fn `suprnova::Cookie::domain` · framework/src/http/cookie.rs:310
  - [ ] fn `suprnova::Cookie::prefixed` · framework/src/http/cookie.rs:320
  - [ ] fn `suprnova::Cookie::partitioned` · framework/src/http/cookie.rs:329
  - [ ] fn `suprnova::Cookie::to_header_value` · framework/src/http/cookie.rs:335
  - [ ] fn `suprnova::Cookie::forget` · framework/src/http/cookie.rs:421
  - [ ] fn `suprnova::Cookie::forget_with` · framework/src/http/cookie.rs:438
  - [ ] fn `suprnova::Cookie::forever` · framework/src/http/cookie.rs:453
  - [ ] fn `suprnova::Cookie::encrypted` · framework/src/http/cookie.rs:473
  - [ ] fn `suprnova::Cookie::read_encrypted_for` · framework/src/http/cookie.rs:492
  - [ ] fn `suprnova::Cookie::read_encrypted` · framework/src/http/cookie.rs:504 (deprecated)
  - [ ] fn `suprnova::Cookie::queue` · framework/src/http/cookie.rs:534
  - [ ] fn `suprnova::Cookie::queued` · framework/src/http/cookie.rs:541
  - [ ] fn `suprnova::Cookie::unqueue` · framework/src/http/cookie.rs:547
  - [ ] fn `suprnova::Cookie::expire` · framework/src/http/cookie.rs:556
- [ ] struct `suprnova::CookieOptions` · framework/src/http/cookie.rs:192 (also `suprnova::http::CookieOptions`, `suprnova::http::cookie::CookieOptions`)
  - Public fields: `http_only`, `secure`, `same_site`, `path`, `domain`, `max_age`, `partitioned`
- [ ] enum `suprnova::CookiePrefix` · framework/src/http/cookie.rs:83 (also `suprnova::http::CookiePrefix`, `suprnova::http::cookie::CookiePrefix`)
  - Variants: `None`, `Secure`, `Host`
  - [ ] fn `suprnova::CookiePrefix::parse` · framework/src/http/cookie.rs:101
  - [ ] fn `suprnova::CookiePrefix::as_str` · framework/src/http/cookie.rs:111
  - [ ] fn `suprnova::CookiePrefix::apply` · framework/src/http/cookie.rs:132
  - [ ] fn `suprnova::CookiePrefix::strip` · framework/src/http/cookie.rs:151
  - [ ] fn `suprnova::CookiePrefix::validate` · framework/src/http/cookie.rs:164
- [ ] enum `suprnova::SameSite` · framework/src/http/cookie.rs:53 (also `suprnova::http::SameSite`, `suprnova::http::cookie::SameSite`)
  - Variants: `Strict`, `Lax`, `None`

### `suprnova::http::extract` (private module; items are public through re-exports)

- [ ] trait `suprnova::FromParam` · framework/src/http/extract.rs:73 (also `suprnova::http::FromParam`)
  - Implemented here by: `String`
  - [ ] fn `suprnova::FromParam::from_param` · framework/src/http/extract.rs:78 (required)
- [ ] trait `suprnova::FromRequest` · framework/src/http/extract.rs:40 (also `suprnova::http::FromRequest`)
  - Implemented here by: `Request`
  - [ ] fn `suprnova::FromRequest::from_request` · framework/src/http/extract.rs:45 (required)

### `suprnova::http::form_request` (private module; items are public through re-exports)

- [ ] trait `suprnova::FormRequest` · framework/src/http/form_request.rs:63 (also `suprnova::http::FormRequest`)
  - [ ] fn `suprnova::FormRequest::authorize` · framework/src/http/form_request.rs:70 (provided)
  - [ ] fn `suprnova::FormRequest::after_validation` · framework/src/http/form_request.rs:107 (provided)
  - [ ] fn `suprnova::FormRequest::after_validation_async` · framework/src/http/form_request.rs:157 (provided)
  - [ ] fn `suprnova::FormRequest::max_body_bytes` · framework/src/http/form_request.rs:186 (provided)
  - [ ] fn `suprnova::FormRequest::extract` · framework/src/http/form_request.rs:199 (provided)

### `suprnova::http::request` (private module; items are public through re-exports)

- [ ] struct `suprnova::Request` · framework/src/http/request.rs:35 (also `suprnova::http::Request`, `suprnova::prelude::Request`)
  - Implements: `suprnova::FromRequest`, `suprnova::InertiaRequestExt`
  - [ ] fn `suprnova::Request::new` · framework/src/http/request.rs:95
  - [ ] fn `suprnova::Request::with_params` · framework/src/http/request.rs:115
  - [ ] fn `suprnova::Request::with_route_pattern` · framework/src/http/request.rs:125
  - [ ] fn `suprnova::Request::with_peer_addr` · framework/src/http/request.rs:134
  - [ ] fn `suprnova::Request::with_trusted_proxies` · framework/src/http/request.rs:148
  - [ ] fn `suprnova::Request::with_auth_user_id` · framework/src/http/request.rs:159
  - [ ] fn `suprnova::Request::for_test` · framework/src/http/request.rs:175
  - [ ] fn `suprnova::Request::for_test_with_headers` · framework/src/http/request.rs:205
  - [ ] fn `suprnova::Request::live_tenant` · framework/src/http/request.rs:373
  - [ ] fn `suprnova::Request::auth_user_id` · framework/src/http/request.rs:391
  - [ ] fn `suprnova::Request::trusted_proxies` · framework/src/http/request.rs:399
  - [ ] fn `suprnova::Request::peer_is_trusted_proxy` · framework/src/http/request.rs:407
  - [ ] fn `suprnova::Request::method` · framework/src/http/request.rs:412
  - [ ] fn `suprnova::Request::path` · framework/src/http/request.rs:417
  - [ ] fn `suprnova::Request::query` · framework/src/http/request.rs:423
  - [ ] fn `suprnova::Request::uri` · framework/src/http/request.rs:428
  - [ ] fn `suprnova::Request::headers` · framework/src/http/request.rs:433
  - [ ] fn `suprnova::Request::param` · framework/src/http/request.rs:439
  - [ ] fn `suprnova::Request::params` · framework/src/http/request.rs:449
  - [ ] fn `suprnova::Request::all_route_params` · framework/src/http/request.rs:457
  - [ ] fn `suprnova::Request::header` · framework/src/http/request.rs:462
  - [ ] fn `suprnova::Request::content_type` · framework/src/http/request.rs:467
  - [ ] fn `suprnova::Request::is_inertia` · framework/src/http/request.rs:472
  - [ ] fn `suprnova::Request::cookies` · framework/src/http/request.rs:493
  - [ ] fn `suprnova::Request::cookie` · framework/src/http/request.rs:516
  - [ ] fn `suprnova::Request::has_header` · framework/src/http/request.rs:523
  - [ ] fn `suprnova::Request::bearer_token` · framework/src/http/request.rs:532
  - [ ] fn `suprnova::Request::is_method` · framework/src/http/request.rs:552
  - [ ] fn `suprnova::Request::ajax` · framework/src/http/request.rs:560
  - [ ] fn `suprnova::Request::pjax` · framework/src/http/request.rs:568
  - [ ] fn `suprnova::Request::prefetch` · framework/src/http/request.rs:582
  - [ ] fn `suprnova::Request::secure` · framework/src/http/request.rs:616
  - [ ] fn `suprnova::Request::scheme` · framework/src/http/request.rs:641
  - [ ] fn `suprnova::Request::ip` · framework/src/http/request.rs:670
  - [ ] fn `suprnova::Request::ips` · framework/src/http/request.rs:703
  - [ ] fn `suprnova::Request::user_agent` · framework/src/http/request.rs:737
  - [ ] fn `suprnova::Request::host` · framework/src/http/request.rs:746
  - [ ] fn `suprnova::Request::http_host` · framework/src/http/request.rs:763
  - [ ] fn `suprnova::Request::scheme_and_http_host` · framework/src/http/request.rs:782
  - [ ] fn `suprnova::Request::port` · framework/src/http/request.rs:796
  - [ ] fn `suprnova::Request::decoded_path` · framework/src/http/request.rs:823
  - [ ] fn `suprnova::Request::segments` · framework/src/http/request.rs:832
  - [ ] fn `suprnova::Request::segment` · framework/src/http/request.rs:842
  - [ ] fn `suprnova::Request::url` · framework/src/http/request.rs:855
  - [ ] fn `suprnova::Request::full_url` · framework/src/http/request.rs:868
  - [ ] fn `suprnova::Request::full_url_with_query` · framework/src/http/request.rs:877
  - [ ] fn `suprnova::Request::full_url_without_query` · framework/src/http/request.rs:887
  - [ ] fn `suprnova::Request::query_params` · framework/src/http/request.rs:912
  - [ ] fn `suprnova::Request::query_param` · framework/src/http/request.rs:937
  - [ ] fn `suprnova::Request::has_query` · framework/src/http/request.rs:949
  - [ ] fn `suprnova::Request::query_into` · framework/src/http/request.rs:957
  - [ ] fn `suprnova::Request::route_pattern` · framework/src/http/request.rs:966
  - [ ] fn `suprnova::Request::route_name` · framework/src/http/request.rs:974
  - [ ] fn `suprnova::Request::route_is` · framework/src/http/request.rs:982
  - [ ] fn `suprnova::Request::is` · framework/src/http/request.rs:993
  - [ ] fn `suprnova::Request::full_url_is` · framework/src/http/request.rs:1004
  - [ ] fn `suprnova::Request::is_json` · framework/src/http/request.rs:1012
  - [ ] fn `suprnova::Request::expects_json` · framework/src/http/request.rs:1023
  - [ ] fn `suprnova::Request::wants_json` · framework/src/http/request.rs:1029
  - [ ] fn `suprnova::Request::acceptable_content_types` · framework/src/http/request.rs:1043
  - [ ] fn `suprnova::Request::accepts` · framework/src/http/request.rs:1053
  - [ ] fn `suprnova::Request::prefers` · framework/src/http/request.rs:1079
  - [ ] fn `suprnova::Request::accepts_any_content_type` · framework/src/http/request.rs:1102
  - [ ] fn `suprnova::Request::accepts_json` · framework/src/http/request.rs:1113
  - [ ] fn `suprnova::Request::accepts_html` · framework/src/http/request.rs:1119
  - [ ] fn `suprnova::Request::inertia_version` · framework/src/http/request.rs:1149
  - [ ] fn `suprnova::Request::inertia_partial_component` · framework/src/http/request.rs:1154
  - [ ] fn `suprnova::Request::inertia_partial_data` · framework/src/http/request.rs:1159
  - [ ] fn `suprnova::Request::body_bytes` · framework/src/http/request.rs:1175
  - [ ] fn `suprnova::Request::body_bytes_with_cap` · framework/src/http/request.rs:1190
  - [ ] fn `suprnova::Request::buffer_body` · framework/src/http/request.rs:1258
  - [ ] fn `suprnova::Request::cached_body` · framework/src/http/request.rs:1282
  - [ ] fn `suprnova::Request::cached_form_field` · framework/src/http/request.rs:1305
  - [ ] fn `suprnova::Request::json` · framework/src/http/request.rs:1338
  - [ ] fn `suprnova::Request::form` · framework/src/http/request.rs:1361
  - [ ] fn `suprnova::Request::input` · framework/src/http/request.rs:1373
  - [ ] fn `suprnova::Request::into_parts` · framework/src/http/request.rs:1388
- [ ] struct `suprnova::http::RequestParts` · framework/src/http/request.rs:1412
  - Public fields: `params`, `content_type`
- [ ] enum `suprnova::http::BodyState` · framework/src/http/request.rs:21
  - Variants: `Streaming`, `Buffered`, `Consumed`

### `suprnova::http::response` (private module; items are public through re-exports)

- [ ] struct `suprnova::HttpResponse` · framework/src/http/response.rs:41 (also `suprnova::http::HttpResponse`, `suprnova::prelude::HttpResponse`)
  - [ ] fn `suprnova::HttpResponse::new` · framework/src/http/response.rs:70
  - [ ] fn `suprnova::HttpResponse::text` · framework/src/http/response.rs:79
  - [ ] fn `suprnova::HttpResponse::json` · framework/src/http/response.rs:88
  - [ ] fn `suprnova::HttpResponse::bytes` · framework/src/http/response.rs:106
  - [ ] fn `suprnova::HttpResponse::from_engine_response` · framework/src/http/response.rs:134
  - [ ] fn `suprnova::HttpResponse::html` · framework/src/http/response.rs:164
  - [ ] fn `suprnova::HttpResponse::sse` · framework/src/http/response.rs:189
  - [ ] fn `suprnova::HttpResponse::event_stream` · framework/src/http/response.rs:211
  - [ ] fn `suprnova::HttpResponse::stream_bytes` · framework/src/http/response.rs:251
  - [ ] fn `suprnova::HttpResponse::stream_json` · framework/src/http/response.rs:273
  - [ ] fn `suprnova::HttpResponse::status` · framework/src/http/response.rs:310
  - [ ] fn `suprnova::HttpResponse::status_code` · framework/src/http/response.rs:316
  - [ ] fn `suprnova::HttpResponse::bytes_body` · framework/src/http/response.rs:324
  - [ ] fn `suprnova::HttpResponse::body` · framework/src/http/response.rs:334
  - [ ] fn `suprnova::HttpResponse::is_streaming` · framework/src/http/response.rs:349
  - [ ] fn `suprnova::HttpResponse::header` · framework/src/http/response.rs:354
  - [ ] fn `suprnova::HttpResponse::with_headers` · framework/src/http/response.rs:366
  - [ ] fn `suprnova::HttpResponse::without_header` · framework/src/http/response.rs:380
  - [ ] fn `suprnova::HttpResponse::header_value` · framework/src/http/response.rs:388
  - [ ] fn `suprnova::HttpResponse::header_values` · framework/src/http/response.rs:411
  - [ ] fn `suprnova::HttpResponse::headers` · framework/src/http/response.rs:433
  - [ ] fn `suprnova::HttpResponse::replace_header` · framework/src/http/response.rs:439
  - [ ] fn `suprnova::HttpResponse::cookie` · framework/src/http/response.rs:457
  - [ ] fn `suprnova::HttpResponse::with_cookies` · framework/src/http/response.rs:465
  - [ ] fn `suprnova::HttpResponse::without_cookie` · framework/src/http/response.rs:478
  - [ ] fn `suprnova::HttpResponse::without_cookies` · framework/src/http/response.rs:489
  - [ ] fn `suprnova::HttpResponse::ok` · framework/src/http/response.rs:501
  - [ ] fn `suprnova::HttpResponse::into_hyper` · framework/src/http/response.rs:528
- [ ] struct `suprnova::Redirect` · framework/src/http/response.rs:684 (also `suprnova::http::Redirect`, `suprnova::prelude::Redirect`)
  - [ ] fn `suprnova::Redirect::to` · framework/src/http/response.rs:719
  - [ ] fn `suprnova::Redirect::route` · framework/src/http/response.rs:734
  - [ ] fn `suprnova::Redirect::back` · framework/src/http/response.rs:763
  - [ ] fn `suprnova::Redirect::away` · framework/src/http/response.rs:777
  - [ ] fn `suprnova::Redirect::refresh` · framework/src/http/response.rs:791
  - [ ] fn `suprnova::Redirect::refresh_for` · framework/src/http/response.rs:804
  - [ ] fn `suprnova::Redirect::guest` · framework/src/http/response.rs:818
  - [ ] fn `suprnova::Redirect::intended` · framework/src/http/response.rs:833
  - [ ] fn `suprnova::Redirect::set_intended_url` · framework/src/http/response.rs:844
  - [ ] fn `suprnova::Redirect::signed_route` · framework/src/http/response.rs:861
  - [ ] fn `suprnova::Redirect::temporary_signed_route` · framework/src/http/response.rs:868
  - [ ] fn `suprnova::Redirect::query` · framework/src/http/response.rs:879
  - [ ] fn `suprnova::Redirect::permanent` · framework/src/http/response.rs:885
  - [ ] fn `suprnova::Redirect::status` · framework/src/http/response.rs:893
  - [ ] fn `suprnova::Redirect::with` · framework/src/http/response.rs:903
  - [ ] fn `suprnova::Redirect::with_input` · framework/src/http/response.rs:912
  - [ ] fn `suprnova::Redirect::with_errors` · framework/src/http/response.rs:942
  - [ ] fn `suprnova::Redirect::with_errors_bag` · framework/src/http/response.rs:953
  - [ ] fn `suprnova::Redirect::cookie` · framework/src/http/response.rs:980
  - [ ] fn `suprnova::Redirect::with_cookies` · framework/src/http/response.rs:987
  - [ ] fn `suprnova::Redirect::without_cookie` · framework/src/http/response.rs:1002
  - [ ] fn `suprnova::Redirect::without_cookies` · framework/src/http/response.rs:1009
  - [ ] fn `suprnova::Redirect::header` · framework/src/http/response.rs:1022
  - [ ] fn `suprnova::Redirect::with_headers` · framework/src/http/response.rs:1029
  - [ ] fn `suprnova::Redirect::with_fragment` · framework/src/http/response.rs:1044
  - [ ] fn `suprnova::Redirect::without_fragment` · framework/src/http/response.rs:1054
  - [ ] fn `suprnova::Redirect::preserve_fragment` · framework/src/http/response.rs:1071
- [ ] struct `suprnova::RedirectRouteBuilder` · framework/src/http/response.rs:1219 (also `suprnova::http::RedirectRouteBuilder`)
  - [ ] fn `suprnova::RedirectRouteBuilder::with` · framework/src/http/response.rs:1240
  - [ ] fn `suprnova::RedirectRouteBuilder::query` · framework/src/http/response.rs:1246
  - [ ] fn `suprnova::RedirectRouteBuilder::permanent` · framework/src/http/response.rs:1252
  - [ ] fn `suprnova::RedirectRouteBuilder::status` · framework/src/http/response.rs:1259
  - [ ] fn `suprnova::RedirectRouteBuilder::flash` · framework/src/http/response.rs:1268
  - [ ] fn `suprnova::RedirectRouteBuilder::with_input` · framework/src/http/response.rs:1275
  - [ ] fn `suprnova::RedirectRouteBuilder::with_errors` · framework/src/http/response.rs:1298
  - [ ] fn `suprnova::RedirectRouteBuilder::with_errors_bag` · framework/src/http/response.rs:1308
  - [ ] fn `suprnova::RedirectRouteBuilder::cookie` · framework/src/http/response.rs:1330
  - [ ] fn `suprnova::RedirectRouteBuilder::with_cookies` · framework/src/http/response.rs:1336
  - [ ] fn `suprnova::RedirectRouteBuilder::without_cookie` · framework/src/http/response.rs:1346
  - [ ] fn `suprnova::RedirectRouteBuilder::without_cookies` · framework/src/http/response.rs:1353
  - [ ] fn `suprnova::RedirectRouteBuilder::header` · framework/src/http/response.rs:1365
  - [ ] fn `suprnova::RedirectRouteBuilder::with_headers` · framework/src/http/response.rs:1371
  - [ ] fn `suprnova::RedirectRouteBuilder::with_fragment` · framework/src/http/response.rs:1384
  - [ ] fn `suprnova::RedirectRouteBuilder::without_fragment` · framework/src/http/response.rs:1393
  - [ ] fn `suprnova::RedirectRouteBuilder::preserve_fragment` · framework/src/http/response.rs:1401
- [ ] trait `suprnova::ResponseExt` · framework/src/http/response.rs:604 (also `suprnova::http::ResponseExt`)
  - Implemented here by: `Response`
  - [ ] fn `suprnova::ResponseExt::status` · framework/src/http/response.rs:606 (required)
  - [ ] fn `suprnova::ResponseExt::header` · framework/src/http/response.rs:608 (required)
  - [ ] fn `suprnova::ResponseExt::with_headers` · framework/src/http/response.rs:611 (required)
  - [ ] fn `suprnova::ResponseExt::without_header` · framework/src/http/response.rs:618 (required)
  - [ ] fn `suprnova::ResponseExt::cookie` · framework/src/http/response.rs:620 (required)
  - [ ] fn `suprnova::ResponseExt::with_cookies` · framework/src/http/response.rs:623 (required)
  - [ ] fn `suprnova::ResponseExt::without_cookie` · framework/src/http/response.rs:628 (required)
  - [ ] fn `suprnova::ResponseExt::without_cookies` · framework/src/http/response.rs:631 (required)
- [ ] type `suprnova::Response` · framework/src/http/response.rs:56 (also `suprnova::http::Response`, `suprnova::prelude::Response`)

### `suprnova::http::trusted_proxies` (private module; items are public through re-exports)

- [ ] struct `suprnova::http::TrustedProxiesConfig` · framework/src/http/trusted_proxies.rs:69
  - [ ] fn `suprnova::http::TrustedProxiesConfig::empty` · framework/src/http/trusted_proxies.rs:76
  - [ ] fn `suprnova::http::TrustedProxiesConfig::with_ips` · framework/src/http/trusted_proxies.rs:84
  - [ ] fn `suprnova::http::TrustedProxiesConfig::is_empty` · framework/src/http/trusted_proxies.rs:98
  - [ ] fn `suprnova::http::TrustedProxiesConfig::trusts` · framework/src/http/trusted_proxies.rs:108
  - [ ] fn `suprnova::http::TrustedProxiesConfig::proxies` · framework/src/http/trusted_proxies.rs:115

### `suprnova::http::upload`

- [ ] fn `suprnova::global_max_multipart_body_bytes` · framework/src/http/upload/mod.rs:89 (also `suprnova::http::upload::global_max_multipart_body_bytes`)
- [ ] fn `suprnova::global_max_multipart_parts` · framework/src/http/upload/mod.rs:159 (also `suprnova::http::upload::global_max_multipart_parts`)
- [ ] fn `suprnova::global_upload_spill_threshold` · framework/src/http/upload/mod.rs:116 (also `suprnova::http::upload::global_upload_spill_threshold`)
- [ ] fn `suprnova::parse_multipart_streaming` · framework/src/http/upload/mod.rs:930 (also `suprnova::http::upload::parse_multipart_streaming`)
- [ ] fn `suprnova::parse_multipart_streaming_with_cap` · framework/src/http/upload/mod.rs:900 (also `suprnova::http::upload::parse_multipart_streaming_with_cap`)
- [ ] fn `suprnova::parse_multipart_streaming_with_limits` · framework/src/http/upload/mod.rs:666 (also `suprnova::http::upload::parse_multipart_streaming_with_limits`)
- [ ] fn `suprnova::set_global_max_multipart_body_bytes` · framework/src/http/upload/mod.rs:82 (also `suprnova::http::upload::set_global_max_multipart_body_bytes`)
- [ ] fn `suprnova::set_global_max_multipart_parts` · framework/src/http/upload/mod.rs:152 (also `suprnova::http::upload::set_global_max_multipart_parts`)
- [ ] fn `suprnova::set_global_upload_spill_threshold` · framework/src/http/upload/mod.rs:109 (also `suprnova::http::upload::set_global_upload_spill_threshold`)
- [ ] fn `suprnova::upload_tempfiles_spilled_total` · framework/src/http/upload/mod.rs:175 (also `suprnova::http::upload::upload_tempfiles_spilled_total`)
- [ ] struct `suprnova::MultipartLimits` · framework/src/http/upload/mod.rs:187 (also `suprnova::http::upload::MultipartLimits`)
  - Public fields: `max_body_bytes`, `max_parts`, `spill_threshold`, `per_field_max_counts`
- [ ] struct `suprnova::MultipartPayload` · framework/src/http/upload/mod.rs:419 (also `suprnova::http::upload::MultipartPayload`)
  - Public fields: `fields`
- [ ] struct `suprnova::UploadedFile` · framework/src/http/upload/mod.rs:234 (also `suprnova::http::upload::UploadedFile`)
  - Public fields: `size`, `file_name`, `content_type`
  - [ ] fn `suprnova::UploadedFile::bytes` · framework/src/http/upload/mod.rs:311
  - [ ] fn `suprnova::UploadedFile::store_as` · framework/src/http/upload/mod.rs:342
  - [ ] fn `suprnova::UploadedFile::extension_from_magic` · framework/src/http/upload/mod.rs:411
- [ ] enum `suprnova::MultipartValue` · framework/src/http/upload/mod.rs:426 (also `suprnova::http::upload::MultipartValue`)
  - Variants: `File`, `Text`
- [ ] trait `suprnova::MultipartRequestHooks` · framework/src/http/upload/mod.rs:957 (also `suprnova::http::upload::MultipartRequestHooks`)
  - [ ] fn `suprnova::MultipartRequestHooks::authorize` · framework/src/http/upload/mod.rs:960 (provided)
  - [ ] fn `suprnova::MultipartRequestHooks::after_validation` · framework/src/http/upload/mod.rs:967 (provided)
- [ ] const `suprnova::DEFAULT_MAX_MULTIPART_BODY_BYTES` · framework/src/http/upload/mod.rs:46 (also `suprnova::http::upload::DEFAULT_MAX_MULTIPART_BODY_BYTES`)
- [ ] const `suprnova::DEFAULT_MAX_MULTIPART_PARTS` · framework/src/http/upload/mod.rs:134 (also `suprnova::http::upload::DEFAULT_MAX_MULTIPART_PARTS`)
- [ ] const `suprnova::DEFAULT_UPLOAD_SPILL_THRESHOLD` · framework/src/http/upload/mod.rs:52 (also `suprnova::http::upload::DEFAULT_UPLOAD_SPILL_THRESHOLD`)

### `suprnova::http::upload::validators`

- [ ] struct `suprnova::ImageFile` · framework/src/http/upload/validators.rs:115 (also `suprnova::http::upload::validators::ImageFile`)
  - Implements: `suprnova::http::upload::validators::UploadValidator`
- [ ] struct `suprnova::MaxSize` · framework/src/http/upload/validators.rs:83 (also `suprnova::http::upload::validators::MaxSize`)
  - Implements: `suprnova::http::upload::validators::UploadValidator`
- [ ] struct `suprnova::MimeType` · framework/src/http/upload/validators.rs:228 (also `suprnova::http::upload::validators::MimeType`)
  - Implements: `suprnova::http::upload::validators::UploadValidator`
- [ ] trait `suprnova::MimeAllowlist` · framework/src/http/upload/validators.rs:141 (also `suprnova::http::upload::validators::MimeAllowlist`)
  - [ ] fn `suprnova::MimeAllowlist::allowed` · framework/src/http/upload/validators.rs:143 (required)
- [ ] trait `suprnova::http::upload::validators::UploadValidator` · framework/src/http/upload/validators.rs:42
  - Implemented here by: `ImageFile`, `MaxSize`, `MimeType`
  - [ ] fn `suprnova::http::upload::validators::UploadValidator::validate_chunk` · framework/src/http/upload/validators.rs:55 (provided)
  - [ ] fn `suprnova::http::upload::validators::UploadValidator::validate_final` · framework/src/http/upload/validators.rs:67 (provided)

## http_client

### `suprnova::http_client`

- [ ] struct `suprnova::ClientResponse` · framework/src/http_client/mod.rs:923 (also `suprnova::http_client::ClientResponse`)
  - [ ] fn `suprnova::ClientResponse::status` · framework/src/http_client/mod.rs:964
  - [ ] fn `suprnova::ClientResponse::header` · framework/src/http_client/mod.rs:972
  - [ ] fn `suprnova::ClientResponse::json` · framework/src/http_client/mod.rs:988
  - [ ] fn `suprnova::ClientResponse::text` · framework/src/http_client/mod.rs:999
  - [ ] fn `suprnova::ClientResponse::bytes` · framework/src/http_client/mod.rs:1010
  - [ ] fn `suprnova::ClientResponse::into_inner` · framework/src/http_client/mod.rs:1028
- [ ] struct `suprnova::FailOnRealCallsGuard` · framework/src/http_client/mod.rs:349 (also `suprnova::http_client::FailOnRealCallsGuard`)
  - [ ] fn `suprnova::FailOnRealCallsGuard::install` · framework/src/http_client/mod.rs:355
- [ ] struct `suprnova::Http` · framework/src/http_client/mod.rs:112 (also `suprnova::http_client::Http`, `suprnova::prelude::Http`)
  - [ ] fn `suprnova::Http::get` · framework/src/http_client/mod.rs:116
  - [ ] fn `suprnova::Http::post` · framework/src/http_client/mod.rs:121
  - [ ] fn `suprnova::Http::put` · framework/src/http_client/mod.rs:126
  - [ ] fn `suprnova::Http::patch` · framework/src/http_client/mod.rs:131
  - [ ] fn `suprnova::Http::delete` · framework/src/http_client/mod.rs:136
  - [ ] fn `suprnova::Http::fake` · framework/src/http_client/mod.rs:177
  - [ ] fn `suprnova::Http::fake_response_text` · framework/src/http_client/mod.rs:217
  - [ ] fn `suprnova::Http::fail_on_real_calls` · framework/src/http_client/mod.rs:236
  - [ ] fn `suprnova::Http::allow_real_calls` · framework/src/http_client/mod.rs:244
  - [ ] fn `suprnova::Http::is_guarded` · framework/src/http_client/mod.rs:249
  - [ ] fn `suprnova::Http::set_max_response_bytes` · framework/src/http_client/mod.rs:258
  - [ ] fn `suprnova::Http::max_response_bytes` · framework/src/http_client/mod.rs:265
  - [ ] fn `suprnova::Http::spawn_with_fake_inheritance` · framework/src/http_client/mod.rs:309
- [ ] struct `suprnova::RequestBuilder` · framework/src/http_client/mod.rs:473 (also `suprnova::http_client::RequestBuilder`)
  - [ ] fn `suprnova::RequestBuilder::no_redirects` · framework/src/http_client/mod.rs:526
  - [ ] fn `suprnova::RequestBuilder::header` · framework/src/http_client/mod.rs:533
  - [ ] fn `suprnova::RequestBuilder::json` · framework/src/http_client/mod.rs:540
  - [ ] fn `suprnova::RequestBuilder::form` · framework/src/http_client/mod.rs:552
  - [ ] fn `suprnova::RequestBuilder::body` · framework/src/http_client/mod.rs:562
  - [ ] fn `suprnova::RequestBuilder::timeout` · framework/src/http_client/mod.rs:569
  - [ ] fn `suprnova::RequestBuilder::max_response_bytes` · framework/src/http_client/mod.rs:577
  - [ ] fn `suprnova::RequestBuilder::bearer_token` · framework/src/http_client/mod.rs:583
  - [ ] fn `suprnova::RequestBuilder::basic_auth` · framework/src/http_client/mod.rs:589
  - [ ] fn `suprnova::RequestBuilder::retry` · framework/src/http_client/mod.rs:618
  - [ ] fn `suprnova::RequestBuilder::retry_non_idempotent` · framework/src/http_client/mod.rs:639
  - [ ] fn `suprnova::RequestBuilder::retry_when` · framework/src/http_client/mod.rs:673
  - [ ] fn `suprnova::RequestBuilder::send` · framework/src/http_client/mod.rs:686
- [ ] struct `suprnova::RetryContext` · framework/src/http_client/mod.rs:455 (also `suprnova::http_client::RetryContext`)
  - Public fields: `attempt`, `method`, `url`, `outcome`
- [ ] enum `suprnova::RetryOutcome` · framework/src/http_client/mod.rs:442 (also `suprnova::http_client::RetryOutcome`)
  - Variants: `TransportError`, `Status`

### `suprnova::http_client::fake` (private module; items are public through re-exports)

- [ ] fn `suprnova::assert_not_sent` · framework/src/http_client/fake.rs:153 (also `suprnova::http_client::assert_not_sent`)
- [ ] fn `suprnova::assert_sent` · framework/src/http_client/fake.rs:137 (also `suprnova::http_client::assert_sent`)
- [ ] fn `suprnova::fake_response` · framework/src/http_client/fake.rs:92 (also `suprnova::http_client::fake_response`)
- [ ] struct `suprnova::RecordedRequest` · framework/src/http_client/fake.rs:56 (also `suprnova::http_client::RecordedRequest`)
  - Public fields: `method`, `url`, `headers`, `body`

## idempotency

### `suprnova::idempotency`

- [ ] struct `suprnova::Idempotency` · framework/src/idempotency/mod.rs:111 (also `suprnova::idempotency::Idempotency`)
  - [ ] fn `suprnova::Idempotency::once` · framework/src/idempotency/mod.rs:136
  - [ ] fn `suprnova::Idempotency::commit_on_success` · framework/src/idempotency/mod.rs:177
  - [ ] fn `suprnova::Idempotency::commit_on_success_owned` · framework/src/idempotency/mod.rs:213
  - [ ] fn `suprnova::Idempotency::release_owned` · framework/src/idempotency/mod.rs:259
  - [ ] fn `suprnova::Idempotency::remember` · framework/src/idempotency/mod.rs:326
- [ ] enum `suprnova::Idempotent` · framework/src/idempotency/mod.rs:66 (also `suprnova::idempotency::Idempotent`)
  - Variants: `Fresh`, `FreshUnfenced`, `Duplicate`
- [ ] enum `suprnova::Replay` · framework/src/idempotency/mod.rs:91 (also `suprnova::idempotency::Replay`)
  - Variants: `Fresh`, `FreshUnfenced`, `Replayed`, `InProgress`

## inertia

### `suprnova::inertia::config` (private module; items are public through re-exports)

- [ ] struct `suprnova::InertiaConfig` · framework/src/inertia/config.rs:212 (also `suprnova::inertia::InertiaConfig`)
  - Public fields: `vite_dev_server`, `entry_point`, `version`, `development`, `frontend`, `default_title`, `encrypt_history_default`, `ssr`, `manifest_path`, `assets_base_url`, `with_all_errors`, `max_concurrent_resolvers`, `error_page`
  - [ ] fn `suprnova::InertiaConfig::new` · framework/src/inertia/config.rs:555
  - [ ] fn `suprnova::InertiaConfig::vite_dev_server` · framework/src/inertia/config.rs:560
  - [ ] fn `suprnova::InertiaConfig::entry_point` · framework/src/inertia/config.rs:567
  - [ ] fn `suprnova::InertiaConfig::version` · framework/src/inertia/config.rs:574
  - [ ] fn `suprnova::InertiaConfig::version_with` · framework/src/inertia/config.rs:612
  - [ ] fn `suprnova::InertiaConfig::production` · framework/src/inertia/config.rs:621
  - [ ] fn `suprnova::InertiaConfig::development` · framework/src/inertia/config.rs:631
  - [ ] fn `suprnova::InertiaConfig::frontend` · framework/src/inertia/config.rs:638
  - [ ] fn `suprnova::InertiaConfig::default_title` · framework/src/inertia/config.rs:647
  - [ ] fn `suprnova::InertiaConfig::encrypt_history` · framework/src/inertia/config.rs:654
  - [ ] fn `suprnova::InertiaConfig::ssr` · framework/src/inertia/config.rs:660
  - [ ] fn `suprnova::InertiaConfig::ssr_disabled` · framework/src/inertia/config.rs:667
  - [ ] fn `suprnova::InertiaConfig::ssr_timeout` · framework/src/inertia/config.rs:673
  - [ ] fn `suprnova::InertiaConfig::ssr_throw_on_error` · framework/src/inertia/config.rs:679
  - [ ] fn `suprnova::InertiaConfig::ssr_exclude` · framework/src/inertia/config.rs:685
  - [ ] fn `suprnova::InertiaConfig::ssr_max_response_bytes` · framework/src/inertia/config.rs:697
  - [ ] fn `suprnova::InertiaConfig::ssr_bundle_path` · framework/src/inertia/config.rs:708
  - [ ] fn `suprnova::InertiaConfig::ssr_ensure_bundle_exists` · framework/src/inertia/config.rs:718
  - [ ] fn `suprnova::InertiaConfig::on_ssr_error` · framework/src/inertia/config.rs:725
  - [ ] fn `suprnova::InertiaConfig::manifest_path` · framework/src/inertia/config.rs:739
  - [ ] fn `suprnova::InertiaConfig::assets_base_url` · framework/src/inertia/config.rs:758
  - [ ] fn `suprnova::InertiaConfig::max_concurrent_resolvers` · framework/src/inertia/config.rs:766
  - [ ] fn `suprnova::InertiaConfig::with_all_errors` · framework/src/inertia/config.rs:780
  - [ ] fn `suprnova::InertiaConfig::error_page` · framework/src/inertia/config.rs:817
  - [ ] fn `suprnova::InertiaConfig::url_resolver` · framework/src/inertia/config.rs:841
  - [ ] fn `suprnova::InertiaConfig::vite_manifest` · framework/src/inertia/config.rs:858
- [ ] struct `suprnova::SsrConfig` · framework/src/inertia/config.rs:342 (also `suprnova::inertia::SsrConfig`)
  - Public fields: `enabled`, `url`, `timeout`, `throw_on_error`, `excluded_paths`, `on_error`, `max_response_bytes`, `bundle_path`, `ensure_bundle_exists`
  - [ ] fn `suprnova::SsrConfig::is_path_excluded` · framework/src/inertia/config.rs:428
- [ ] enum `suprnova::Frontend` · framework/src/inertia/config.rs:150 (also `suprnova::inertia::Frontend`)
  - Variants: `Svelte`, `React`, `Vue`
  - [ ] fn `suprnova::Frontend::detect_from_env` · framework/src/inertia/config.rs:164
  - [ ] fn `suprnova::Frontend::default_entry_point` · framework/src/inertia/config.rs:174
  - [ ] fn `suprnova::Frontend::page_extensions` · framework/src/inertia/config.rs:186
  - [ ] fn `suprnova::Frontend::as_str` · framework/src/inertia/config.rs:195
- [ ] enum `suprnova::VersionResolver` · framework/src/inertia/config.rs:28 (also `suprnova::inertia::VersionResolver`)
  - Variants: `Static`, `Dynamic`, `Manifest`
  - [ ] fn `suprnova::VersionResolver::new` · framework/src/inertia/config.rs:43
  - [ ] fn `suprnova::VersionResolver::with` · framework/src/inertia/config.rs:49
  - [ ] fn `suprnova::VersionResolver::from_manifest` · framework/src/inertia/config.rs:75
  - [ ] fn `suprnova::VersionResolver::resolve` · framework/src/inertia/config.rs:80
- [ ] const `suprnova::MANIFEST_VERSION_FALLBACK` · framework/src/inertia/config.rs:115 (also `suprnova::inertia::MANIFEST_VERSION_FALLBACK`)

### `suprnova::inertia::conversion_middleware` (private module; items are public through re-exports)

- [ ] struct `suprnova::Inertia303Middleware` · framework/src/inertia/conversion_middleware.rs:22 (also `suprnova::inertia::Inertia303Middleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::Inertia303Middleware::new` · framework/src/inertia/conversion_middleware.rs:26

### `suprnova::inertia::encrypt_middleware` (private module; items are public through re-exports)

- [ ] struct `suprnova::EncryptHistoryMiddleware` · framework/src/inertia/encrypt_middleware.rs:28 (also `suprnova::inertia::EncryptHistoryMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::EncryptHistoryMiddleware::new` · framework/src/inertia/encrypt_middleware.rs:32

### `suprnova::inertia::error_page_middleware` (private module; items are public through re-exports)

- [ ] struct `suprnova::InertiaErrorPageMiddleware` · framework/src/inertia/error_page_middleware.rs:103 (also `suprnova::inertia::InertiaErrorPageMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::InertiaErrorPageMiddleware::new` · framework/src/inertia/error_page_middleware.rs:115

### `suprnova::inertia::facade` (private module; items are public through re-exports)

- [ ] struct `suprnova::Inertia` · framework/src/inertia/facade.rs:16 (also `suprnova::inertia::Inertia`)
  - [ ] fn `suprnova::Inertia::paginate` · framework/src/inertia/facade.rs:30
  - [ ] fn `suprnova::Inertia::data` · framework/src/inertia/facade.rs:47
  - [ ] fn `suprnova::Inertia::try_data` · framework/src/inertia/facade.rs:63
  - [ ] fn `suprnova::Inertia::install` · framework/src/inertia/facade.rs:172

### `suprnova::inertia::headers_middleware` (private module; items are public through re-exports)

- [ ] struct `suprnova::InertiaHeadersMiddleware` · framework/src/inertia/headers_middleware.rs:36 (also `suprnova::inertia::InertiaHeadersMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::InertiaHeadersMiddleware::new` · framework/src/inertia/headers_middleware.rs:40

### `suprnova::inertia::manifest` (private module; items are public through re-exports)

- [ ] struct `suprnova::ManifestEntry` · framework/src/inertia/manifest.rs:48 (also `suprnova::inertia::ManifestEntry`)
  - Public fields: `file`, `css`, `imports`, `is_entry`
- [ ] struct `suprnova::ResolvedAssets` · framework/src/inertia/manifest.rs:76 (also `suprnova::inertia::ResolvedAssets`)
  - Public fields: `js`, `css`, `preload`
- [ ] struct `suprnova::ViteManifest` · framework/src/inertia/manifest.rs:69 (also `suprnova::inertia::ViteManifest`)
  - [ ] fn `suprnova::ViteManifest::load` · framework/src/inertia/manifest.rs:87
  - [ ] fn `suprnova::ViteManifest::resolve_entry` · framework/src/inertia/manifest.rs:116

### `suprnova::inertia::prop` (private module; items are public through re-exports)

- [ ] struct `suprnova::DeferOptions` · framework/src/inertia/prop.rs:115 (also `suprnova::inertia::DeferOptions`)
  - [ ] fn `suprnova::DeferOptions::new` · framework/src/inertia/prop.rs:131
  - [ ] fn `suprnova::DeferOptions::group` · framework/src/inertia/prop.rs:137
  - [ ] fn `suprnova::DeferOptions::rescue` · framework/src/inertia/prop.rs:145
- [ ] struct `suprnova::OnceOptions` · framework/src/inertia/prop.rs:297 (also `suprnova::inertia::OnceOptions`)
  - [ ] fn `suprnova::OnceOptions::new` · framework/src/inertia/prop.rs:305
  - [ ] fn `suprnova::OnceOptions::as_key` · framework/src/inertia/prop.rs:311
  - [ ] fn `suprnova::OnceOptions::until` · framework/src/inertia/prop.rs:319
  - [ ] fn `suprnova::OnceOptions::fresh` · framework/src/inertia/prop.rs:327
- [ ] struct `suprnova::PartialFilter` · framework/src/inertia/prop.rs:1078 (also `suprnova::inertia::PartialFilter`)
  - Public fields: `matched`, `only`, `except`
  - [ ] fn `suprnova::PartialFilter::build` · framework/src/inertia/prop.rs:1092
  - [ ] fn `suprnova::PartialFilter::should_include_eager` · framework/src/inertia/prop.rs:1146
  - [ ] fn `suprnova::PartialFilter::should_include_optional` · framework/src/inertia/prop.rs:1180
  - [ ] fn `suprnova::PartialFilter::should_include` · framework/src/inertia/prop.rs:1213
  - [ ] fn `suprnova::PartialFilter::narrow` · framework/src/inertia/prop.rs:1265
- [ ] struct `suprnova::Prop` · framework/src/inertia/prop.rs:422 (also `suprnova::inertia::Prop`)
  - [ ] fn `suprnova::Prop::eager` · framework/src/inertia/prop.rs:570
  - [ ] fn `suprnova::Prop::absent` · framework/src/inertia/prop.rs:580
  - [ ] fn `suprnova::Prop::from_resolver` · framework/src/inertia/prop.rs:590
  - [ ] fn `suprnova::Prop::lazy` · framework/src/inertia/prop.rs:598
  - [ ] fn `suprnova::Prop::always` · framework/src/inertia/prop.rs:616
  - [ ] fn `suprnova::Prop::optional` · framework/src/inertia/prop.rs:626
  - [ ] fn `suprnova::Prop::defer` · framework/src/inertia/prop.rs:637
  - [ ] fn `suprnova::Prop::group` · framework/src/inertia/prop.rs:649
  - [ ] fn `suprnova::Prop::rescue` · framework/src/inertia/prop.rs:661
  - [ ] fn `suprnova::Prop::merge` · framework/src/inertia/prop.rs:670
  - [ ] fn `suprnova::Prop::prepend` · framework/src/inertia/prop.rs:677
  - [ ] fn `suprnova::Prop::deep_merge` · framework/src/inertia/prop.rs:684
  - [ ] fn `suprnova::Prop::merge_with_path` · framework/src/inertia/prop.rs:722
  - [ ] fn `suprnova::Prop::match_on` · framework/src/inertia/prop.rs:738
  - [ ] fn `suprnova::Prop::merge_strategy` · framework/src/inertia/prop.rs:746
  - [ ] fn `suprnova::Prop::once` · framework/src/inertia/prop.rs:764
  - [ ] fn `suprnova::Prop::as_key` · framework/src/inertia/prop.rs:775
  - [ ] fn `suprnova::Prop::until` · framework/src/inertia/prop.rs:786
  - [ ] fn `suprnova::Prop::fresh` · framework/src/inertia/prop.rs:795
  - [ ] fn `suprnova::Prop::scroll` · framework/src/inertia/prop.rs:818
  - [ ] fn `suprnova::Prop::scroll_wrap` · framework/src/inertia/prop.rs:842
  - [ ] fn `suprnova::Prop::visibility` · framework/src/inertia/prop.rs:850
  - [ ] fn `suprnova::Prop::is_always` · framework/src/inertia/prop.rs:855
  - [ ] fn `suprnova::Prop::is_optional` · framework/src/inertia/prop.rs:860
  - [ ] fn `suprnova::Prop::is_defer` · framework/src/inertia/prop.rs:866
  - [ ] fn `suprnova::Prop::is_absent` · framework/src/inertia/prop.rs:871
  - [ ] fn `suprnova::Prop::is_lazy` · framework/src/inertia/prop.rs:880
  - [ ] fn `suprnova::Prop::has_resolver` · framework/src/inertia/prop.rs:890
  - [ ] fn `suprnova::Prop::as_value` · framework/src/inertia/prop.rs:895
  - [ ] fn `suprnova::Prop::defer_group` · framework/src/inertia/prop.rs:903
  - [ ] fn `suprnova::Prop::rescues` · framework/src/inertia/prop.rs:909
  - [ ] fn `suprnova::Prop::merge_mode` · framework/src/inertia/prop.rs:914
  - [ ] fn `suprnova::Prop::match_on_fields` · framework/src/inertia/prop.rs:919
  - [ ] fn `suprnova::Prop::merge_paths` · framework/src/inertia/prop.rs:924
  - [ ] fn `suprnova::Prop::is_once` · framework/src/inertia/prop.rs:929
  - [ ] fn `suprnova::Prop::once_cache_key` · framework/src/inertia/prop.rs:935
  - [ ] fn `suprnova::Prop::once_expires_at` · framework/src/inertia/prop.rs:942
  - [ ] fn `suprnova::Prop::is_fresh` · framework/src/inertia/prop.rs:947
  - [ ] fn `suprnova::Prop::scroll_metadata` · framework/src/inertia/prop.rs:952
  - [ ] fn `suprnova::Prop::scroll_wrap_key` · framework/src/inertia/prop.rs:958
  - [ ] fn `suprnova::Prop::resolve` · framework/src/inertia/prop.rs:977
  - [ ] fn `suprnova::Prop::resolve_with_owner` · framework/src/inertia/prop.rs:1008
- [ ] struct `suprnova::ScrollMetadata` · framework/src/inertia/prop.rs:195 (also `suprnova::inertia::ScrollMetadata`)
  - Public fields: `page_name`, `previous_page`, `next_page`, `current_page`
  - [ ] fn `suprnova::ScrollMetadata::new` · framework/src/inertia/prop.rs:212
  - [ ] fn `suprnova::ScrollMetadata::current` · framework/src/inertia/prop.rs:222
  - [ ] fn `suprnova::ScrollMetadata::previous` · framework/src/inertia/prop.rs:228
  - [ ] fn `suprnova::ScrollMetadata::next` · framework/src/inertia/prop.rs:234
- [ ] enum `suprnova::MergeMode` · framework/src/inertia/prop.rs:358 (also `suprnova::inertia::MergeMode`)
  - Variants: `Append`, `Prepend`, `Deep`
- [ ] enum `suprnova::MergeStrategy` · framework/src/inertia/prop.rs:167 (also `suprnova::inertia::MergeStrategy`)
  - Variants: `Append`, `Prepend`, `Deep`
- [ ] enum `suprnova::Visibility` · framework/src/inertia/prop.rs:378 (also `suprnova::inertia::Visibility`)
  - Variants: `Standard`, `Always`, `Optional`, `Deferred`
- [ ] trait `suprnova::InertiaRequestExt` · framework/src/inertia/prop.rs:15 (also `suprnova::inertia::InertiaRequestExt`)
  - Implemented here by: `Request`
  - [ ] fn `suprnova::InertiaRequestExt::path` · framework/src/inertia/prop.rs:17 (required)
  - [ ] fn `suprnova::InertiaRequestExt::path_and_query` · framework/src/inertia/prop.rs:33 (provided)
  - [ ] fn `suprnova::InertiaRequestExt::header` · framework/src/inertia/prop.rs:37 (required)
  - [ ] fn `suprnova::InertiaRequestExt::is_inertia` · framework/src/inertia/prop.rs:39 (provided)
  - [ ] fn `suprnova::InertiaRequestExt::is_prefetch` · framework/src/inertia/prop.rs:49 (provided)
- [ ] trait `suprnova::MatchOnFields` · framework/src/inertia/prop.rs:515 (also `suprnova::inertia::MatchOnFields`)
  - Implemented here by: `String`, `Vec`
  - [ ] fn `suprnova::MatchOnFields::into_match_on_fields` · framework/src/inertia/prop.rs:517 (required)
- [ ] trait `suprnova::ProvidesScrollMetadata` · framework/src/inertia/prop.rs:267 (also `suprnova::inertia::ProvidesScrollMetadata`)
  - Implemented here by: `CursorPaginator`, `LengthAwarePaginator`, `Paginator`
  - [ ] fn `suprnova::ProvidesScrollMetadata::page_name` · framework/src/inertia/prop.rs:270 (required)
  - [ ] fn `suprnova::ProvidesScrollMetadata::previous_page` · framework/src/inertia/prop.rs:274 (required)
  - [ ] fn `suprnova::ProvidesScrollMetadata::next_page` · framework/src/inertia/prop.rs:278 (required)
  - [ ] fn `suprnova::ProvidesScrollMetadata::current_page` · framework/src/inertia/prop.rs:281 (required)
  - [ ] fn `suprnova::ProvidesScrollMetadata::scroll_metadata` · framework/src/inertia/prop.rs:284 (provided)
- [ ] type `suprnova::PropFuture` · framework/src/inertia/prop.rs:104 (also `suprnova::inertia::PropFuture`)
- [ ] type `suprnova::PropResolver` · framework/src/inertia/prop.rs:110 (also `suprnova::inertia::PropResolver`)

### `suprnova::inertia::response` (private module; items are public through re-exports)

- [ ] struct `suprnova::InertiaResponse` · framework/src/inertia/response.rs:103 (also `suprnova::inertia::InertiaResponse`)
  - [ ] fn `suprnova::InertiaResponse::new` · framework/src/inertia/response.rs:274
  - [ ] fn `suprnova::InertiaResponse::with_config` · framework/src/inertia/response.rs:307
  - [ ] fn `suprnova::InertiaResponse::title` · framework/src/inertia/response.rs:326
  - [ ] fn `suprnova::InertiaResponse::with` · framework/src/inertia/response.rs:335
  - [ ] fn `suprnova::InertiaResponse::always` · framework/src/inertia/response.rs:344
  - [ ] fn `suprnova::InertiaResponse::always_with` · framework/src/inertia/response.rs:358
  - [ ] fn `suprnova::InertiaResponse::lazy` · framework/src/inertia/response.rs:384
  - [ ] fn `suprnova::InertiaResponse::prop_lazy_with_owner` · framework/src/inertia/response.rs:414
  - [ ] fn `suprnova::InertiaResponse::prop` · framework/src/inertia/response.rs:448
  - [ ] fn `suprnova::InertiaResponse::from_data_props` · framework/src/inertia/response.rs:460
  - [ ] fn `suprnova::InertiaResponse::optional` · framework/src/inertia/response.rs:481
  - [ ] fn `suprnova::InertiaResponse::defer` · framework/src/inertia/response.rs:498
  - [ ] fn `suprnova::InertiaResponse::defer_with` · framework/src/inertia/response.rs:511
  - [ ] fn `suprnova::InertiaResponse::merge` · framework/src/inertia/response.rs:535
  - [ ] fn `suprnova::InertiaResponse::merge_prepend` · framework/src/inertia/response.rs:541
  - [ ] fn `suprnova::InertiaResponse::deep_merge` · framework/src/inertia/response.rs:547
  - [ ] fn `suprnova::InertiaResponse::merge_with` · framework/src/inertia/response.rs:553
  - [ ] fn `suprnova::InertiaResponse::merge_lazy` · framework/src/inertia/response.rs:577
  - [ ] fn `suprnova::InertiaResponse::merge_lazy_with` · framework/src/inertia/response.rs:589
  - [ ] fn `suprnova::InertiaResponse::once` · framework/src/inertia/response.rs:612
  - [ ] fn `suprnova::InertiaResponse::once_with` · framework/src/inertia/response.rs:625
  - [ ] fn `suprnova::InertiaResponse::scroll` · framework/src/inertia/response.rs:678
  - [ ] fn `suprnova::InertiaResponse::scroll_with` · framework/src/inertia/response.rs:691
  - [ ] fn `suprnova::InertiaResponse::scroll_wrapped` · framework/src/inertia/response.rs:716
  - [ ] fn `suprnova::InertiaResponse::scroll_with_wrapped` · framework/src/inertia/response.rs:728
  - [ ] fn `suprnova::InertiaResponse::paginate` · framework/src/inertia/response.rs:770
  - [ ] fn `suprnova::InertiaResponse::flash` · framework/src/inertia/response.rs:785
  - [ ] fn `suprnova::InertiaResponse::try_with` · framework/src/inertia/response.rs:803
  - [ ] fn `suprnova::InertiaResponse::try_always` · framework/src/inertia/response.rs:815
  - [ ] fn `suprnova::InertiaResponse::try_merge_with` · framework/src/inertia/response.rs:830
  - [ ] fn `suprnova::InertiaResponse::try_scroll` · framework/src/inertia/response.rs:845
  - [ ] fn `suprnova::InertiaResponse::try_scroll_wrapped` · framework/src/inertia/response.rs:860
  - [ ] fn `suprnova::InertiaResponse::try_flash` · framework/src/inertia/response.rs:873
  - [ ] fn `suprnova::InertiaResponse::encrypt_history` · framework/src/inertia/response.rs:888
  - [ ] fn `suprnova::InertiaResponse::clear_history` · framework/src/inertia/response.rs:904
  - [ ] fn `suprnova::InertiaResponse::preserve_fragment` · framework/src/inertia/response.rs:919
  - [ ] fn `suprnova::InertiaResponse::location` · framework/src/inertia/response.rs:943
  - [ ] fn `suprnova::InertiaResponse::location_for` · framework/src/inertia/response.rs:961
  - [ ] fn `suprnova::InertiaResponse::redirect` · framework/src/inertia/response.rs:985
  - [ ] fn `suprnova::InertiaResponse::resolve` · framework/src/inertia/response.rs:1013
  - [ ] fn `suprnova::InertiaResponse::version_conflict` · framework/src/inertia/response.rs:1297
- [ ] enum `suprnova::PropEntry` · framework/src/inertia/response.rs:37 (also `suprnova::inertia::PropEntry`)
  - Variants: `Eager`, `LazyOwned`, `DeferredOwned`, `ClosureOwned`
- [ ] trait `suprnova::IntoInertiaData` · framework/src/inertia/response.rs:73 (also `suprnova::inertia::IntoInertiaData`)
  - [ ] fn `suprnova::IntoInertiaData::__into_inertia_props` · framework/src/inertia/response.rs:76 (required)
  - [ ] fn `suprnova::IntoInertiaData::__try_into_inertia_props` · framework/src/inertia/response.rs:87 (provided)

### `suprnova::inertia::shared` (private module; items are public through re-exports)

- [ ] struct `suprnova::InertiaRegistry` · framework/src/inertia/shared.rs:84 (also `suprnova::inertia::InertiaRegistry`)
  - [ ] fn `suprnova::InertiaRegistry::new` · framework/src/inertia/shared.rs:102
  - [ ] fn `suprnova::InertiaRegistry::share_value` · framework/src/inertia/shared.rs:128
  - [ ] fn `suprnova::InertiaRegistry::share_lazy` · framework/src/inertia/shared.rs:136
  - [ ] fn `suprnova::InertiaRegistry::share_once` · framework/src/inertia/shared.rs:150
  - [ ] fn `suprnova::InertiaRegistry::register_trait` · framework/src/inertia/shared.rs:193
- [ ] trait `suprnova::InertiaSharedData` · framework/src/inertia/shared.rs:53 (also `suprnova::inertia::InertiaSharedData`)
  - Implemented here by: `LocaleShare`
  - [ ] fn `suprnova::InertiaSharedData::share` · framework/src/inertia/shared.rs:64 (required)

### `suprnova::inertia::ssr` (private module; items are public through re-exports)

- [ ] struct `suprnova::SsrResponse` · framework/src/inertia/ssr.rs:27 (also `suprnova::inertia::SsrResponse`)
  - Public fields: `head`, `body`

### `suprnova::inertia::validation_redirect_middleware` (private module; items are public through re-exports)

- [ ] struct `suprnova::InertiaValidationRedirectMiddleware` · framework/src/inertia/validation_redirect_middleware.rs:24 (also `suprnova::inertia::InertiaValidationRedirectMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::InertiaValidationRedirectMiddleware::new` · framework/src/inertia/validation_redirect_middleware.rs:29

### `suprnova::inertia::version_middleware` (private module; items are public through re-exports)

- [ ] struct `suprnova::InertiaVersionMiddleware` · framework/src/inertia/version_middleware.rs:54 (also `suprnova::inertia::InertiaVersionMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::InertiaVersionMiddleware::new` · framework/src/inertia/version_middleware.rs:61
  - [ ] fn `suprnova::InertiaVersionMiddleware::with_resolver` · framework/src/inertia/version_middleware.rs:70

## live

### `suprnova::live::action`

- [ ] fn `suprnova::live::action::action_result` · framework/src/live/action.rs:66
- [ ] fn `suprnova::live::action::flash_intent` · framework/src/live/action.rs:58
- [ ] fn `suprnova::live::action::route_intent` · framework/src/live/action.rs:122
- [ ] fn `suprnova::live::action::url_intent` · framework/src/live/action.rs:52
- [ ] struct `suprnova::live::action::ResponseIntentError` · framework/src/live/action.rs:90
  - [ ] fn `suprnova::live::action::ResponseIntentError::kind` · framework/src/live/action.rs:101
- [ ] struct `suprnova::live::action::RouteIntentError` · framework/src/live/action.rs:143
  - [ ] fn `suprnova::live::action::RouteIntentError::kind` · framework/src/live/action.rs:168
- [ ] enum `suprnova::live::action::ResponseIntentErrorKind` · framework/src/live/action.rs:79
  - Variants: `InvalidKey`, `InvalidPayload`, `InvalidComponent`
- [ ] enum `suprnova::live::action::RouteIntentErrorKind` · framework/src/live/action.rs:134
  - Variants: `RouteUnavailable`, `InvalidParameters`

### `suprnova::live::assets`

- [ ] fn `suprnova::live::assets::live_asset_catalog` · framework/src/live/assets.rs:340
- [ ] struct `suprnova::live::assets::BootScript` · framework/src/live/assets.rs:174
  - [ ] fn `suprnova::live::assets::BootScript::strategy` · framework/src/live/assets.rs:196
  - [ ] fn `suprnova::live::assets::BootScript::file` · framework/src/live/assets.rs:202
  - [ ] fn `suprnova::live::assets::BootScript::bytes` · framework/src/live/assets.rs:208
  - [ ] fn `suprnova::live::assets::BootScript::sha256_hex` · framework/src/live/assets.rs:214
  - [ ] fn `suprnova::live::assets::BootScript::sri` · framework/src/live/assets.rs:220
- [ ] struct `suprnova::live::assets::LiveAssetCatalog` · framework/src/live/assets.rs:226
  - [ ] fn `suprnova::live::assets::LiveAssetCatalog::identity` · framework/src/live/assets.rs:234
  - [ ] fn `suprnova::live::assets::LiveAssetCatalog::manifest_bytes` · framework/src/live/assets.rs:240
  - [ ] fn `suprnova::live::assets::LiveAssetCatalog::artifacts` · framework/src/live/assets.rs:246
  - [ ] fn `suprnova::live::assets::LiveAssetCatalog::artifact` · framework/src/live/assets.rs:252
  - [ ] fn `suprnova::live::assets::LiveAssetCatalog::boot_scripts` · framework/src/live/assets.rs:258
  - [ ] fn `suprnova::live::assets::LiveAssetCatalog::boot_script` · framework/src/live/assets.rs:265
  - [ ] fn `suprnova::live::assets::LiveAssetCatalog::boot_script_for` · framework/src/live/assets.rs:273
  - [ ] fn `suprnova::live::assets::LiveAssetCatalog::url` · framework/src/live/assets.rs:287
- [ ] struct `suprnova::live::LiveBootstrap` · framework/src/live/assets.rs:434 (also `suprnova::live::assets::LiveBootstrap`)
  - [ ] fn `suprnova::live::LiveBootstrap::html` · framework/src/live/assets.rs:443
  - [ ] fn `suprnova::live::LiveBootstrap::roles` · framework/src/live/assets.rs:449
  - [ ] fn `suprnova::live::LiveBootstrap::strategy` · framework/src/live/assets.rs:455
- [ ] struct `suprnova::live::LiveBootstrapOptions` · framework/src/live/assets.rs:102 (also `suprnova::live::assets::LiveBootstrapOptions`)
  - [ ] fn `suprnova::live::LiveBootstrapOptions::esm` · framework/src/live/assets.rs:112
  - [ ] fn `suprnova::live::LiveBootstrapOptions::classic` · framework/src/live/assets.rs:123
  - [ ] fn `suprnova::live::LiveBootstrapOptions::with_stimulus` · framework/src/live/assets.rs:134
  - [ ] fn `suprnova::live::LiveBootstrapOptions::with_suprnova_ui` · framework/src/live/assets.rs:144
  - [ ] fn `suprnova::live::LiveBootstrapOptions::with_nonce` · framework/src/live/assets.rs:151
  - [ ] fn `suprnova::live::LiveBootstrapOptions::strategy` · framework/src/live/assets.rs:167
- [ ] enum `suprnova::live::LiveBootstrapStrategy` · framework/src/live/assets.rs:63 (also `suprnova::live::assets::LiveBootstrapStrategy`)
  - Variants: `Esm`, `Classic`
- [ ] const `suprnova::live::assets::LIVE_ASSET_PATH_PREFIX` · framework/src/live/assets.rs:31

### `suprnova::live::config` (private module; items are public through re-exports)

- [ ] struct `suprnova::live::LiveConfig` · framework/src/live/config.rs:121
  - [ ] fn `suprnova::live::LiveConfig::builder` · framework/src/live/config.rs:130
  - [ ] fn `suprnova::live::LiveConfig::standard` · framework/src/live/config.rs:136
  - [ ] fn `suprnova::live::LiveConfig::max_request_bytes` · framework/src/live/config.rs:146
  - [ ] fn `suprnova::live::LiveConfig::max_response_bytes` · framework/src/live/config.rs:152
  - [ ] fn `suprnova::live::LiveConfig::max_context_lifetime_ms` · framework/src/live/config.rs:158
- [ ] struct `suprnova::live::LiveConfigBuilder` · framework/src/live/config.rs:171
  - [ ] fn `suprnova::live::LiveConfigBuilder::new` · framework/src/live/config.rs:180
  - [ ] fn `suprnova::live::LiveConfigBuilder::max_request_bytes` · framework/src/live/config.rs:190
  - [ ] fn `suprnova::live::LiveConfigBuilder::max_response_bytes` · framework/src/live/config.rs:197
  - [ ] fn `suprnova::live::LiveConfigBuilder::max_context_lifetime_ms` · framework/src/live/config.rs:204
  - [ ] fn `suprnova::live::LiveConfigBuilder::build` · framework/src/live/config.rs:210
- [ ] struct `suprnova::live::LiveConfigError` · framework/src/live/config.rs:259
  - [ ] fn `suprnova::live::LiveConfigError::kind` · framework/src/live/config.rs:270
- [ ] enum `suprnova::live::LedgerDriver` · framework/src/live/config.rs:27
  - Variants: `Memory`, `Database`, `Redis`
  - [ ] fn `suprnova::live::LedgerDriver::from_env` · framework/src/live/config.rs:84
- [ ] enum `suprnova::live::LiveConfigErrorKind` · framework/src/live/config.rs:239
  - Variants: `InvalidByteLimits`, `InvalidContextLifetime`
  - [ ] fn `suprnova::live::LiveConfigErrorKind::as_str` · framework/src/live/config.rs:249

### `suprnova::live::document` (private module; items are public through re-exports)

- [ ] struct `suprnova::live::LiveDocument` · framework/src/live/document.rs:669
  - [ ] fn `suprnova::live::LiveDocument::from_request` · framework/src/live/document.rs:679
  - [ ] fn `suprnova::live::LiveDocument::mount` · framework/src/live/document.rs:697
  - [ ] fn `suprnova::live::LiveDocument::bootstrap` · framework/src/live/document.rs:825
  - [ ] fn `suprnova::live::LiveDocument::render` · framework/src/live/document.rs:879
- [ ] struct `suprnova::live::LiveDocumentError` · framework/src/live/document.rs:981
  - [ ] fn `suprnova::live::LiveDocumentError::kind` · framework/src/live/document.rs:992
- [ ] struct `suprnova::live::LiveMount` · framework/src/live/document.rs:110
  - [ ] fn `suprnova::live::LiveMount::public_seed` · framework/src/live/document.rs:146
  - [ ] fn `suprnova::live::LiveMount::identity_bound` · framework/src/live/document.rs:155
  - [ ] fn `suprnova::live::LiveMount::kind` · framework/src/live/document.rs:230
  - [ ] fn `suprnova::live::LiveMount::on_stitch_failure` · framework/src/live/document.rs:247
- [ ] struct `suprnova::live::LiveNestedSegment` · framework/src/live/document.rs:385
  - [ ] fn `suprnova::live::LiveNestedSegment::identity_free` · framework/src/live/document.rs:396
  - [ ] fn `suprnova::live::LiveNestedSegment::identity_bound` · framework/src/live/document.rs:438
  - [ ] fn `suprnova::live::LiveNestedSegment::on_failure` · framework/src/live/document.rs:468
  - [ ] fn `suprnova::live::LiveNestedSegment::is_identity_bound` · framework/src/live/document.rs:479
  - [ ] fn `suprnova::live::LiveNestedSegment::route_pattern` · framework/src/live/document.rs:485
  - [ ] fn `suprnova::live::LiveNestedSegment::into_segment` · framework/src/live/document.rs:509
- [ ] struct `suprnova::live::MountedIsland` · framework/src/live/document.rs:650
  - [ ] fn `suprnova::live::MountedIsland::html` · framework/src/live/document.rs:657
- [ ] struct `suprnova::live::StitchSlotDescriptor` · framework/src/live/document.rs:86
  - Public fields: `route`, `slot`, `document_key`, `component`, `contract_digest`, `protocol`, `build`, `parameters`, `flags`, `on_failure`
- [ ] enum `suprnova::live::LiveDocumentErrorKind` · framework/src/live/document.rs:946
  - Variants: `InvalidDeclaration`, `UnpreparedRequest`, `RuntimeUnavailable`, `RouteMismatch`, `ContextRejected`, `InvalidMount`, `RenderRejected`, `AssetsUnavailable`, `InvalidBootstrap`, `BootstrapRepeated`, `MountAfterBootstrap`, `StitchFallbackTooLarge`, `DynamicIncluderPath`, `NestedSegmentLengthMismatch`
- [ ] enum `suprnova::live::LiveMountKind` · framework/src/live/document.rs:40
  - Variants: `PublicSeed`, `IdentityBound`
- [ ] enum `suprnova::live::StitchFailurePolicy` · framework/src/live/document.rs:54
  - Variants: `FailDocument`, `Omit`, `Fallback`

### `suprnova::live::events` (private module; items are public through re-exports)

- [ ] struct `suprnova::live::LiveOutcomeAccepted` · framework/src/live/events.rs:10
  - Implements: `suprnova::Event`
  - [ ] fn `suprnova::live::LiveOutcomeAccepted::revision` · framework/src/live/events.rs:22
  - [ ] fn `suprnova::live::LiveOutcomeAccepted::outcome` · framework/src/live/events.rs:28

### `suprnova::live::registry` (private module; items are public through re-exports)

- [ ] struct `suprnova::live::LiveRegistry` · framework/src/live/registry.rs:59
  - [ ] fn `suprnova::live::LiveRegistry::builder` · framework/src/live/registry.rs:71
  - [ ] fn `suprnova::live::LiveRegistry::len` · framework/src/live/registry.rs:77
  - [ ] fn `suprnova::live::LiveRegistry::is_empty` · framework/src/live/registry.rs:83
- [ ] struct `suprnova::live::LiveRegistryBuilder` · framework/src/live/registry.rs:120
  - [ ] fn `suprnova::live::LiveRegistryBuilder::new` · framework/src/live/registry.rs:128
  - [ ] fn `suprnova::live::LiveRegistryBuilder::register` · framework/src/live/registry.rs:136
  - [ ] fn `suprnova::live::LiveRegistryBuilder::build` · framework/src/live/registry.rs:209
- [ ] struct `suprnova::live::RegistryError` · framework/src/live/registry.rs:270
  - [ ] fn `suprnova::live::RegistryError::kind` · framework/src/live/registry.rs:281
- [ ] enum `suprnova::live::RegistryErrorKind` · framework/src/live/registry.rs:234
  - Variants: `InvalidComponent`, `DuplicateComponent`, `DuplicateView`, `CapacityExceeded`, `RequiredTransactionUnsupported`, `ReservedName`
  - [ ] fn `suprnova::live::RegistryErrorKind::as_str` · framework/src/live/registry.rs:256
- [ ] trait `suprnova::live::ComponentContract` · framework/src/live/registry.rs:27

### `suprnova::live::routes` (private module; items are public through re-exports)

- [ ] struct `suprnova::live::LiveRouteGuard` · framework/src/live/routes.rs:42
  - [ ] fn `suprnova::live::LiveRouteGuard::middleware` · framework/src/live/routes.rs:49

### `suprnova::live::runtime` (private module; items are public through re-exports)

- [ ] fn `suprnova::live::verify_ledger_backend` · framework/src/live/runtime.rs:1930
- [ ] struct `suprnova::live::LiveRuntime` · framework/src/live/runtime.rs:482
  - [ ] fn `suprnova::live::LiveRuntime::config` · framework/src/live/runtime.rs:516
  - [ ] fn `suprnova::live::LiveRuntime::registry_len` · framework/src/live/runtime.rs:522

### `suprnova::live::streams` (private module; items are public through re-exports)

- [ ] struct `suprnova::live::LiveStreamError` · framework/src/live/streams.rs:42
  - [ ] fn `suprnova::live::LiveStreamError::kind` · framework/src/live/streams.rs:53
- [ ] struct `suprnova::live::LiveStreams` · framework/src/live/streams.rs:79
  - [ ] fn `suprnova::live::LiveStreams::resolve` · framework/src/live/streams.rs:85
  - [ ] fn `suprnova::live::LiveStreams::from_runtime` · framework/src/live/streams.rs:93
  - [ ] fn `suprnova::live::LiveStreams::refresh` · framework/src/live/streams.rs:100
  - [ ] fn `suprnova::live::LiveStreams::event` · framework/src/live/streams.rs:108
- [ ] enum `suprnova::live::LiveEventTarget` · framework/src/live/streams.rs:14
  - Variants: `Island`, `Parent`, `Child`, `Document`, `NamedIsland`, `Browser`
- [ ] enum `suprnova::live::LiveStreamErrorKind` · framework/src/live/streams.rs:31
  - Variants: `RuntimeUnavailable`, `InvalidTopic`, `InvalidPayload`

### `suprnova::live::tenant` (private module; items are public through re-exports)

- [ ] fn `suprnova::live::current_tenant` · framework/src/live/tenant.rs:51
- [ ] struct `suprnova::live::LiveTenantMiddleware` · framework/src/live/tenant.rs:63
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::live::LiveTenantMiddleware::new` · framework/src/live/tenant.rs:70
- [ ] trait `suprnova::live::LiveTenantResolver` · framework/src/live/tenant.rs:25
  - [ ] fn `suprnova::live::LiveTenantResolver::resolve` · framework/src/live/tenant.rs:27 (required)

### `suprnova::live::testing`

- [ ] fn `suprnova::live::testing::await_async_transport_retirement_for_test` · framework/src/live/testing.rs:807
- [ ] fn `suprnova::live::testing::complete_live_route_policy_for_test` · framework/src/live/testing.rs:897
- [ ] fn `suprnova::live::testing::component_registration_has_runtime_hooks` · framework/src/live/testing.rs:812
- [ ] fn `suprnova::live::testing::inspect_async_transports_for_test` · framework/src/live/testing.rs:787
- [ ] fn `suprnova::live::testing::inspect_configured_upload_residue_for_test` · framework/src/live/testing.rs:1795
- [ ] fn `suprnova::live::testing::inspect_deterministic_upload_handle_for_test` · framework/src/live/testing.rs:1758
- [ ] fn `suprnova::live::testing::inspect_request_attestation` · framework/src/live/testing.rs:476
- [ ] fn `suprnova::live::testing::inspect_runtime` · framework/src/live/testing.rs:1574
- [ ] fn `suprnova::live::testing::inspect_upload_mount_authority_for_test` · framework/src/live/testing.rs:1723
- [ ] fn `suprnova::live::testing::prepare_child_parameter_delivery_for_test` · framework/src/live/testing.rs:75
- [ ] fn `suprnova::live::testing::prepare_live_request_for_test` · framework/src/live/testing.rs:516
- [ ] fn `suprnova::live::testing::prepare_live_request_until_for_test` · framework/src/live/testing.rs:878
- [ ] fn `suprnova::live::testing::prepare_live_request_with_fixed_clock_for_test` · framework/src/live/testing.rs:1042
- [ ] fn `suprnova::live::testing::prepare_live_router_for_test` · framework/src/live/testing.rs:679
- [ ] fn `suprnova::live::testing::prepare_live_router_with_clock_for_test` · framework/src/live/testing.rs:728
- [ ] fn `suprnova::live::testing::project_live_response_for_test` · framework/src/live/testing.rs:2331
- [ ] fn `suprnova::live::testing::record_live_security_not_required_for_test` · framework/src/live/testing.rs:829
- [ ] fn `suprnova::live::testing::record_live_security_pass_for_test` · framework/src/live/testing.rs:819
- [ ] fn `suprnova::live::testing::register_live_mount_for_test` · framework/src/live/testing.rs:608
- [ ] fn `suprnova::live::testing::register_live_route_for_test` · framework/src/live/testing.rs:586
- [ ] fn `suprnova::live::testing::remove_live_security_check_for_test` · framework/src/live/testing.rs:872 (feature: `testing`)
- [ ] fn `suprnova::live::testing::report_live_outcome_for_test` · framework/src/live/testing.rs:2356
- [ ] fn `suprnova::live::testing::request_cancellation_for_test` · framework/src/live/testing.rs:1013
- [ ] fn `suprnova::live::testing::resolve_upload_mount_authority_for_test` · framework/src/live/testing.rs:1739
- [ ] fn `suprnova::live::testing::run_upload_cleanup_for_test` · framework/src/live/testing.rs:1997
- [ ] fn `suprnova::live::testing::run_upload_provider_conformance_for_test` · framework/src/live/testing.rs:2043
- [ ] fn `suprnova::live::testing::same_request_identity` · framework/src/live/testing.rs:1007
- [ ] fn `suprnova::live::testing::same_runtime_instance` · framework/src/live/testing.rs:1021
- [ ] fn `suprnova::live::testing::select_upload_mount_for_test` · framework/src/live/testing.rs:1595
- [ ] fn `suprnova::live::testing::validate_runtime_provider_omission_for_test` · framework/src/live/testing.rs:1026
- [ ] struct `suprnova::live::testing::ActionAssertion` · framework/src/live/testing.rs:2367
  - [ ] fn `suprnova::live::testing::ActionAssertion::new` · framework/src/live/testing.rs:2374
  - [ ] fn `suprnova::live::testing::ActionAssertion::assert_rendered` · framework/src/live/testing.rs:2379
  - [ ] fn `suprnova::live::testing::ActionAssertion::assert_not_rendered` · framework/src/live/testing.rs:2388
  - [ ] fn `suprnova::live::testing::ActionAssertion::assert_redirected` · framework/src/live/testing.rs:2397
- [ ] struct `suprnova::live::testing::AdjustableTestClock` · framework/src/live/testing.rs:695
  - Implements: `suprnova_live::clock::Clock`
  - [ ] fn `suprnova::live::testing::AdjustableTestClock::new` · framework/src/live/testing.rs:702
  - [ ] fn `suprnova::live::testing::AdjustableTestClock::now_ms` · framework/src/live/testing.rs:710
  - [ ] fn `suprnova::live::testing::AdjustableTestClock::advance_ms` · framework/src/live/testing.rs:715
- [ ] struct `suprnova::live::testing::AsyncTransportReport` · framework/src/live/testing.rs:746
  - Public fields: `memberships`, `retained_events`, `retained_bytes`, `degraded`, `reader_active`, `coalesced`, `degraded_lanes`
  - [ ] fn `suprnova::live::testing::AsyncTransportReport::kind` · framework/src/live/testing.rs:768
  - [ ] fn `suprnova::live::testing::AsyncTransportReport::credential_matches` · framework/src/live/testing.rs:774
- [ ] struct `suprnova::live::testing::LiveChildParameterDeliveryFixture` · framework/src/live/testing.rs:15
  - [ ] fn `suprnova::live::testing::LiveChildParameterDeliveryFixture::child_snapshot` · framework/src/live/testing.rs:29
  - [ ] fn `suprnova::live::testing::LiveChildParameterDeliveryFixture::admission_carrier` · framework/src/live/testing.rs:35
  - [ ] fn `suprnova::live::testing::LiveChildParameterDeliveryFixture::historical_v1_envelope` · framework/src/live/testing.rs:44
  - [ ] fn `suprnova::live::testing::LiveChildParameterDeliveryFixture::current_child_revision` · framework/src/live/testing.rs:49
  - [ ] fn `suprnova::live::testing::LiveChildParameterDeliveryFixture::current_parent_revision` · framework/src/live/testing.rs:56
  - [ ] fn `suprnova::live::testing::LiveChildParameterDeliveryFixture::advance_parent_revision` · framework/src/live/testing.rs:63
- [ ] struct `suprnova::live::testing::LiveContextHarness` · framework/src/live/testing.rs:903
  - [ ] fn `suprnova::live::testing::LiveContextHarness::anonymous` · framework/src/live/testing.rs:910
  - [ ] fn `suprnova::live::testing::LiveContextHarness::validate` · framework/src/live/testing.rs:998
- [ ] struct `suprnova::live::testing::LiveRuntimeReport` · framework/src/live/testing.rs:1447
  - [ ] fn `suprnova::live::testing::LiveRuntimeReport::is_complete` · framework/src/live/testing.rs:1467
  - [ ] fn `suprnova::live::testing::LiveRuntimeReport::has_subscription_ports` · framework/src/live/testing.rs:1486
  - [ ] fn `suprnova::live::testing::LiveRuntimeReport::has_async_state` · framework/src/live/testing.rs:1492
  - [ ] fn `suprnova::live::testing::LiveRuntimeReport::has_clock` · framework/src/live/testing.rs:1498
  - [ ] fn `suprnova::live::testing::LiveRuntimeReport::has_random_source` · framework/src/live/testing.rs:1504
  - [ ] fn `suprnova::live::testing::LiveRuntimeReport::has_key_ring` · framework/src/live/testing.rs:1510
  - [ ] fn `suprnova::live::testing::LiveRuntimeReport::has_instance_ledger` · framework/src/live/testing.rs:1516
  - [ ] fn `suprnova::live::testing::LiveRuntimeReport::has_seed_promotion_service` · framework/src/live/testing.rs:1522
  - [ ] fn `suprnova::live::testing::LiveRuntimeReport::has_execution_kernel` · framework/src/live/testing.rs:1528
  - [ ] fn `suprnova::live::testing::LiveRuntimeReport::has_context_validator` · framework/src/live/testing.rs:1534
  - [ ] fn `suprnova::live::testing::LiveRuntimeReport::has_host_ports` · framework/src/live/testing.rs:1540
  - [ ] fn `suprnova::live::testing::LiveRuntimeReport::has_upload_ports` · framework/src/live/testing.rs:1549
  - [ ] fn `suprnova::live::testing::LiveRuntimeReport::has_upload_services` · framework/src/live/testing.rs:1555
  - [ ] fn `suprnova::live::testing::LiveRuntimeReport::has_mount_catalog` · framework/src/live/testing.rs:1561
- [ ] struct `suprnova::live::testing::LiveSecurityReport` · framework/src/live/testing.rs:419
  - [ ] fn `suprnova::live::testing::LiveSecurityReport::is_complete` · framework/src/live/testing.rs:430
  - [ ] fn `suprnova::live::testing::LiveSecurityReport::missing_checks` · framework/src/live/testing.rs:436
  - [ ] fn `suprnova::live::testing::LiveSecurityReport::present_count` · framework/src/live/testing.rs:442
  - [ ] fn `suprnova::live::testing::LiveSecurityReport::disposition` · framework/src/live/testing.rs:448
  - [ ] fn `suprnova::live::testing::LiveSecurityReport::policy_reason` · framework/src/live/testing.rs:454
  - [ ] fn `suprnova::live::testing::LiveSecurityReport::order_is_valid` · framework/src/live/testing.rs:463
- [ ] struct `suprnova::live::testing::LiveTestRoutePolicy` · framework/src/live/testing.rs:536
  - Public fields: `trusted_internal_origin`, `stateless_csrf`, `stateless_session`, `anonymous_principal`, `tenantless`, `direct_peer`, `upstream_rate_limit`, `no_additional_middleware`
  - [ ] fn `suprnova::live::testing::LiveTestRoutePolicy::strict` · framework/src/live/testing.rs:571
- [ ] struct `suprnova::live::testing::LiveValidationHarness` · framework/src/live/testing.rs:1119
  - [ ] fn `suprnova::live::testing::LiveValidationHarness::new` · framework/src/live/testing.rs:1126
  - [ ] fn `suprnova::live::testing::LiveValidationHarness::from_registry` · framework/src/live/testing.rs:1156
  - [ ] fn `suprnova::live::testing::LiveValidationHarness::validate` · framework/src/live/testing.rs:1164
  - [ ] fn `suprnova::live::testing::LiveValidationHarness::validate_target` · framework/src/live/testing.rs:1214
  - [ ] fn `suprnova::live::testing::LiveValidationHarness::validate_target_with_issue_limit` · framework/src/live/testing.rs:1259
  - [ ] fn `suprnova::live::testing::LiveValidationHarness::validate_selected_target` · framework/src/live/testing.rs:1309
  - [ ] fn `suprnova::live::testing::LiveValidationHarness::validate_string_action_target` · framework/src/live/testing.rs:1367
- [ ] struct `suprnova::live::testing::UploadCleanupConformanceReport` · framework/src/live/testing.rs:1964
  - [ ] fn `suprnova::live::testing::UploadCleanupConformanceReport::claimed` · framework/src/live/testing.rs:1973
  - [ ] fn `suprnova::live::testing::UploadCleanupConformanceReport::reclaimed` · framework/src/live/testing.rs:1979
  - [ ] fn `suprnova::live::testing::UploadCleanupConformanceReport::residue_is_empty` · framework/src/live/testing.rs:1985
- [ ] struct `suprnova::live::testing::UploadHandleDerivationReport` · framework/src/live/testing.rs:1610
  - [ ] fn `suprnova::live::testing::UploadHandleDerivationReport::same_current` · framework/src/live/testing.rs:1639
  - [ ] fn `suprnova::live::testing::UploadHandleDerivationReport::accepts` · framework/src/live/testing.rs:1645
  - [ ] fn `suprnova::live::testing::UploadHandleDerivationReport::is_uuid_v4` · framework/src/live/testing.rs:1651
- [ ] struct `suprnova::live::testing::UploadMountAuthorityReport` · framework/src/live/testing.rs:1605
  - [ ] fn `suprnova::live::testing::UploadMountAuthorityReport::same_scope` · framework/src/live/testing.rs:1666
  - [ ] fn `suprnova::live::testing::UploadMountAuthorityReport::matches_build` · framework/src/live/testing.rs:1672
  - [ ] fn `suprnova::live::testing::UploadMountAuthorityReport::matches_contract` · framework/src/live/testing.rs:1683
- [ ] struct `suprnova::live::testing::UploadMountResolutionReport` · framework/src/live/testing.rs:1700
  - [ ] fn `suprnova::live::testing::UploadMountResolutionReport::matches` · framework/src/live/testing.rs:1707
- [ ] struct `suprnova::live::testing::UploadProviderConformanceReport` · framework/src/live/testing.rs:1854
  - [ ] fn `suprnova::live::testing::UploadProviderConformanceReport::received_bytes` · framework/src/live/testing.rs:1874
  - [ ] fn `suprnova::live::testing::UploadProviderConformanceReport::next_chunk_index` · framework/src/live/testing.rs:1880
  - [ ] fn `suprnova::live::testing::UploadProviderConformanceReport::cancel_removed_quarantine` · framework/src/live/testing.rs:1886
  - [ ] fn `suprnova::live::testing::UploadProviderConformanceReport::direct_provider_fails_closed` · framework/src/live/testing.rs:1892
  - [ ] fn `suprnova::live::testing::UploadProviderConformanceReport::storage_provider_fails_closed` · framework/src/live/testing.rs:1898
  - [ ] fn `suprnova::live::testing::UploadProviderConformanceReport::quarantine_permissions_are_private` · framework/src/live/testing.rs:1904
  - [ ] fn `suprnova::live::testing::UploadProviderConformanceReport::memo_exact_replay` · framework/src/live/testing.rs:1910
  - [ ] fn `suprnova::live::testing::UploadProviderConformanceReport::memo_mismatch_fails_closed` · framework/src/live/testing.rs:1916
  - [ ] fn `suprnova::live::testing::UploadProviderConformanceReport::memo_missing_fails_closed` · framework/src/live/testing.rs:1922
  - [ ] fn `suprnova::live::testing::UploadProviderConformanceReport::memo_exhaustion_fails_closed` · framework/src/live/testing.rs:1928
  - [ ] fn `suprnova::live::testing::UploadProviderConformanceReport::memo_scope_isolation` · framework/src/live/testing.rs:1934
  - [ ] fn `suprnova::live::testing::UploadProviderConformanceReport::memo_lifecycle_deletion` · framework/src/live/testing.rs:1940
  - [ ] fn `suprnova::live::testing::UploadProviderConformanceReport::memo_partial_order_recovered` · framework/src/live/testing.rs:1946
  - [ ] fn `suprnova::live::testing::UploadProviderConformanceReport::memo_redacted` · framework/src/live/testing.rs:1952
- [ ] struct `suprnova::live::testing::UploadResidueReport` · framework/src/live/testing.rs:1616
  - [ ] fn `suprnova::live::testing::UploadResidueReport::is_empty` · framework/src/live/testing.rs:1625
- [ ] enum `suprnova::live::testing::LiveSecurityCheck` · framework/src/live/testing.rs:143
  - Variants: `Origin`, `Csrf`, `Session`, `Principal`, `Tenant`, `Proxy`, `RateLimit`, `Middleware`
- [ ] enum `suprnova::live::testing::LiveSecurityDisposition` · framework/src/live/testing.rs:179
  - Variants: `Passed`, `NotRequired`
- [ ] enum `suprnova::live::testing::LiveSecurityPolicyReason` · framework/src/live/testing.rs:188
  - Variants: `TrustedInternalOrigin`, `StatelessCsrfPolicy`, `StatelessRequest`, `AnonymousPrincipal`, `TenantlessRoute`, `DirectPeer`, `UpstreamRateLimited`, `NoAdditionalMiddleware`
- [ ] enum `suprnova::live::testing::LiveTestOperation` · framework/src/live/testing.rs:236
  - Variants: `Action`, `Upload`, `SseControl`, `WebSocketHandshake`
- [ ] enum `suprnova::live::testing::LiveTestRuntimeProvider` · framework/src/live/testing.rs:249
  - Variants: `Clock`, `Random`, `KeyRing`, `Ledger`, `Authorization`, `Transaction`, `Validation`, `EventReporter`, `Telemetry`, `Cancellation`, `ResponseIntent`, `UploadLedger`, `UploadCleanupLedger`, `UploadQuarantine`, `UploadProvider`, `UploadReverseProxy`, `UploadReverseProxyProgress`, `UploadDirect`, `UploadAuthorizationAdapter`, `UploadAuthorization`, `UploadScanner`, `UploadApplicationValidation`, `UploadEvidence`, `UploadFinalizer`, `SubscriptionAuthorization`, `SubscriptionCredentials`
  - [ ] const `suprnova::live::testing::LiveTestRuntimeProvider::ALL` · framework/src/live/testing.rs:306
  - [ ] fn `suprnova::live::testing::LiveTestRuntimeProvider::name` · framework/src/live/testing.rs:337
- [ ] const `suprnova::live::testing::PUBLIC_SEED_MAX_AGE_MS` · framework/src/live/testing.rs:675

### `suprnova::live::tooling`

- [ ] fn `suprnova::live::tooling::execute` · framework/src/live/tooling.rs:233
- [ ] struct `suprnova::live::tooling::ToolingError` · framework/src/live/tooling.rs:105
  - [ ] fn `suprnova::live::tooling::ToolingError::kind` · framework/src/live/tooling.rs:116
- [ ] struct `suprnova::live::tooling::ToolRequest` · framework/src/live/tooling.rs:143
  - [ ] fn `suprnova::live::tooling::ToolRequest::new` · framework/src/live/tooling.rs:152
  - [ ] fn `suprnova::live::tooling::ToolRequest::parse` · framework/src/live/tooling.rs:161
  - [ ] fn `suprnova::live::tooling::ToolRequest::protocol` · framework/src/live/tooling.rs:173
  - [ ] fn `suprnova::live::tooling::ToolRequest::operation` · framework/src/live/tooling.rs:179
  - [ ] fn `suprnova::live::tooling::ToolRequest::template_roots` · framework/src/live/tooling.rs:185
- [ ] enum `suprnova::live::tooling::ToolingErrorKind` · framework/src/live/tooling.rs:58
  - Variants: `UnsupportedProtocol`, `UnknownOperation`, `RegistryUnavailable`, `TemplateRootRejected`, `TemplateRejected`, `TemplateLimitExceeded`, `ComponentLimitExceeded`, `DiagnosticLimitExceeded`, `AssetsUnavailable`, `OutputLimitExceeded`, `OutputFailed`
  - [ ] fn `suprnova::live::tooling::ToolingErrorKind::as_str` · framework/src/live/tooling.rs:86

### `suprnova::live::tooling_protocol`

- [ ] struct `suprnova::live::tooling_protocol::AssetReport` · framework/src/live/tooling_protocol.rs:327
  - Public fields: `kind`, `file`, `bytes`, `sha256`, `sri`, `content_type`, `content`
- [ ] struct `suprnova::live::tooling_protocol::CheckSummary` · framework/src/live/tooling_protocol.rs:309
  - Public fields: `registry_bound`, `components`, `proved`, `errors`, `unproved`, `template_files`
- [ ] struct `suprnova::live::tooling_protocol::ComponentReport` · framework/src/live/tooling_protocol.rs:185
  - Public fields: `name`, `view`, `component_version`, `state_schema_version`, `action_schema_version`, `checker_contract_version`, `minimum_protocol`, `fields`, `upload_fields`, `actions`, `events`, `effects`, `subscriptions`, `refresh_on_promote`, `contract_digest`
- [ ] struct `suprnova::live::tooling_protocol::ConfigReport` · framework/src/live/tooling_protocol.rs:221
  - Public fields: `max_request_bytes`, `max_response_bytes`, `max_context_lifetime_ms`
- [ ] struct `suprnova::live::tooling_protocol::DiagnosticReport` · framework/src/live/tooling_protocol.rs:167
  - Public fields: `component`, `view`, `code`, `severity`, `line`, `column`
- [ ] struct `suprnova::live::tooling_protocol::EndReport` · framework/src/live/tooling_protocol.rs:347
  - Public fields: `status`, `error`
- [ ] struct `suprnova::live::tooling_protocol::Envelope` · framework/src/live/tooling_protocol.rs:124
  - Public fields: `protocol`, `sequence`, `operation`, `framework`, `assets`, `body`
- [ ] struct `suprnova::live::tooling_protocol::ReadinessReport` · framework/src/live/tooling_protocol.rs:249
  - Public fields: `clock`, `random`, `key_ring`, `ledger`, `promotion`, `execution`, `context_validator`, `host_ports`, `upload_ports`, `upload_services`, `mount_catalog`, `response_and_cancellation`, `subscription_ports`, `async_state`
- [ ] struct `suprnova::live::tooling_protocol::RuntimeReport` · framework/src/live/tooling_protocol.rs:283
  - Public fields: `registry_bound`, `components`, `config`, `upload_host`, `runtime_bound`, `readiness`, `asset_identity`, `browser_runtime_version`, `runtime_contract_version`, `protocol_versions`
- [ ] struct `suprnova::live::tooling_protocol::UploadHostReport` · framework/src/live/tooling_protocol.rs:233
  - Public fields: `installed`, `finalizer`, `direct_provider`, `scanner`, `application_validator`
- [ ] enum `suprnova::live::tooling_protocol::AssetKind` · framework/src/live/tooling_protocol.rs:112
  - Variants: `Manifest`, `Artifact`, `Boot`
- [ ] enum `suprnova::live::tooling_protocol::Body` · framework/src/live/tooling_protocol.rs:147
  - Variants: `Begin`, `Diagnostic`, `Component`, `Runtime`, `Summary`, `Asset`, `End`
- [ ] enum `suprnova::live::tooling_protocol::Operation` · framework/src/live/tooling_protocol.rs:57
  - Variants: `Check`, `Inspect`, `Assets`
  - [ ] fn `suprnova::live::tooling_protocol::Operation::as_str` · framework/src/live/tooling_protocol.rs:69
  - [ ] fn `suprnova::live::tooling_protocol::Operation::parse` · framework/src/live/tooling_protocol.rs:79
- [ ] enum `suprnova::live::tooling_protocol::Outcome` · framework/src/live/tooling_protocol.rs:92
  - Variants: `Ok`, `Failed`
- [ ] enum `suprnova::live::tooling_protocol::Severity` · framework/src/live/tooling_protocol.rs:102
  - Variants: `Error`, `Unproved`
- [ ] const `suprnova::live::tooling_protocol::COMMAND_NAME` · framework/src/live/tooling_protocol.rs:26
- [ ] const `suprnova::live::tooling_protocol::MAX_ASSET_BYTES` · framework/src/live/tooling_protocol.rs:40
- [ ] const `suprnova::live::tooling_protocol::MAX_ASSETS` · framework/src/live/tooling_protocol.rs:38
- [ ] const `suprnova::live::tooling_protocol::MAX_COMPONENTS` · framework/src/live/tooling_protocol.rs:36
- [ ] const `suprnova::live::tooling_protocol::MAX_DIAGNOSTICS` · framework/src/live/tooling_protocol.rs:34
- [ ] const `suprnova::live::tooling_protocol::MAX_ENVELOPES` · framework/src/live/tooling_protocol.rs:32
- [ ] const `suprnova::live::tooling_protocol::MAX_LINE_BYTES` · framework/src/live/tooling_protocol.rs:28
- [ ] const `suprnova::live::tooling_protocol::MAX_TEMPLATE_DEPTH` · framework/src/live/tooling_protocol.rs:50
- [ ] const `suprnova::live::tooling_protocol::MAX_TEMPLATE_FILE_BYTES` · framework/src/live/tooling_protocol.rs:46
- [ ] const `suprnova::live::tooling_protocol::MAX_TEMPLATE_FILES` · framework/src/live/tooling_protocol.rs:44
- [ ] const `suprnova::live::tooling_protocol::MAX_TEMPLATE_ROOTS` · framework/src/live/tooling_protocol.rs:42
- [ ] const `suprnova::live::tooling_protocol::MAX_TEMPLATE_TOTAL_BYTES` · framework/src/live/tooling_protocol.rs:48
- [ ] const `suprnova::live::tooling_protocol::MAX_TEXT_BYTES` · framework/src/live/tooling_protocol.rs:52
- [ ] const `suprnova::live::tooling_protocol::MAX_TOTAL_BYTES` · framework/src/live/tooling_protocol.rs:30
- [ ] const `suprnova::live::tooling_protocol::PROTOCOL_VERSION` · framework/src/live/tooling_protocol.rs:24

### `suprnova::live::ui_assets` (private module; items are public through re-exports)

- [ ] const `suprnova::live::LIVE_UI_ASSET_PATH_PREFIX` · framework/src/live/ui_assets.rs:21
- [ ] const `suprnova::live::LIVE_UI_TEMPLATE_ROOT` · framework/src/live/ui_assets.rs:24

### `suprnova::live::upload_host` (private module; items are public through re-exports)

- [ ] struct `suprnova::live::LiveUploadHost` · framework/src/live/upload_host.rs:15
  - [ ] fn `suprnova::live::LiveUploadHost::new` · framework/src/live/upload_host.rs:25
  - [ ] fn `suprnova::live::LiveUploadHost::with_finalizer` · framework/src/live/upload_host.rs:36
  - [ ] fn `suprnova::live::LiveUploadHost::with_direct_provider` · framework/src/live/upload_host.rs:43
  - [ ] fn `suprnova::live::LiveUploadHost::with_scanner` · framework/src/live/upload_host.rs:50
  - [ ] fn `suprnova::live::LiveUploadHost::with_application_validator` · framework/src/live/upload_host.rs:57

### `suprnova::live::upload_policy` (private module; items are public through re-exports)

- [ ] struct `suprnova::live::UploadPolicy` · framework/src/live/upload_policy.rs:68
  - [ ] fn `suprnova::live::UploadPolicy::builder` · framework/src/live/upload_policy.rs:81
- [ ] struct `suprnova::live::UploadPolicyBuilder` · framework/src/live/upload_policy.rs:172
  - [ ] fn `suprnova::live::UploadPolicyBuilder::maximum_files` · framework/src/live/upload_policy.rs:179
  - [ ] fn `suprnova::live::UploadPolicyBuilder::maximum_file_bytes` · framework/src/live/upload_policy.rs:186
  - [ ] fn `suprnova::live::UploadPolicyBuilder::replacement` · framework/src/live/upload_policy.rs:193
  - [ ] fn `suprnova::live::UploadPolicyBuilder::accept` · framework/src/live/upload_policy.rs:200
  - [ ] fn `suprnova::live::UploadPolicyBuilder::accept_application` · framework/src/live/upload_policy.rs:207
  - [ ] fn `suprnova::live::UploadPolicyBuilder::dimensions` · framework/src/live/upload_policy.rs:220
  - [ ] fn `suprnova::live::UploadPolicyBuilder::scan` · framework/src/live/upload_policy.rs:236
  - [ ] fn `suprnova::live::UploadPolicyBuilder::finalize_action` · framework/src/live/upload_policy.rs:243
  - [ ] fn `suprnova::live::UploadPolicyBuilder::build` · framework/src/live/upload_policy.rs:250
- [ ] enum `suprnova::live::UploadReplacement` · framework/src/live/upload_policy.rs:20
  - Variants: `RetirePrevious`, `PreservePrevious`
- [ ] enum `suprnova::live::UploadScan` · framework/src/live/upload_policy.rs:38
  - Variants: `Disabled`, `Required`
- [ ] enum `suprnova::live::UploadScanFailure` · framework/src/live/upload_policy.rs:29
  - Variants: `Retry`, `Reject`
- [ ] enum `suprnova::live::UploadType` · framework/src/live/upload_policy.rs:7
  - Variants: `Gif`, `Jpeg`, `Png`, `Webp`

## localization

### `suprnova::localization` (feature: `localization`)

- [ ] fn `suprnova::scope_locale` · framework/src/localization/mod.rs:160 (also `suprnova::localization::scope_locale`)
- [ ] struct `suprnova::Lang` · framework/src/localization/mod.rs:204 (also `suprnova::localization::Lang`)
  - [ ] fn `suprnova::Lang::locale` · framework/src/localization/mod.rs:212
  - [ ] fn `suprnova::Lang::set_locale` · framework/src/localization/mod.rs:240
  - [ ] fn `suprnova::Lang::get` · framework/src/localization/mod.rs:256
  - [ ] fn `suprnova::Lang::get_with` · framework/src/localization/mod.rs:262
  - [ ] fn `suprnova::Lang::try_get` · framework/src/localization/mod.rs:284
  - [ ] fn `suprnova::Lang::try_get_with` · framework/src/localization/mod.rs:312
  - [ ] fn `suprnova::Lang::has` · framework/src/localization/mod.rs:337
  - [ ] fn `suprnova::Lang::available_locales` · framework/src/localization/mod.rs:352
  - [ ] fn `suprnova::Lang::number` · framework/src/localization/mod.rs:362
  - [ ] fn `suprnova::Lang::try_number` · framework/src/localization/mod.rs:371
  - [ ] fn `suprnova::Lang::currency` · framework/src/localization/mod.rs:379
  - [ ] fn `suprnova::Lang::try_currency` · framework/src/localization/mod.rs:388
  - [ ] fn `suprnova::Lang::date` · framework/src/localization/mod.rs:395
  - [ ] fn `suprnova::Lang::try_date` · framework/src/localization/mod.rs:404
  - [ ] fn `suprnova::Lang::time` · framework/src/localization/mod.rs:411
  - [ ] fn `suprnova::Lang::try_time` · framework/src/localization/mod.rs:420
  - [ ] fn `suprnova::Lang::datetime` · framework/src/localization/mod.rs:427
  - [ ] fn `suprnova::Lang::try_datetime` · framework/src/localization/mod.rs:436
  - [ ] fn `suprnova::Lang::list` · framework/src/localization/mod.rs:447
  - [ ] fn `suprnova::Lang::try_list` · framework/src/localization/mod.rs:456
  - [ ] fn `suprnova::Lang::relative` · framework/src/localization/mod.rs:464
  - [ ] fn `suprnova::Lang::try_relative` · framework/src/localization/mod.rs:473
- [ ] struct `suprnova::LocaleShare` · framework/src/localization/mod.rs:548 (also `suprnova::localization::LocaleShare`)
  - Implements: `suprnova::InertiaSharedData`
- [ ] struct `suprnova::Localization` · framework/src/localization/mod.rs:168 (also `suprnova::localization::Localization`)
- [ ] static `suprnova::localization::CURRENT_LOCALE` · framework/src/localization/mod.rs:36

### `suprnova::localization::config` (private module; items are public through re-exports)

- [ ] struct `suprnova::LocalizationConfig` · framework/src/localization/config.rs:21 (feature: `localization`; also `suprnova::localization::LocalizationConfig`)
  - Public fields: `default_locale`, `fallback_locale`, `use_isolating`, `detection`, `session_key`, `cookie_name`, `parents`
  - [ ] fn `suprnova::LocalizationConfig::from_env` · framework/src/localization/config.rs:50
  - [ ] fn `suprnova::LocalizationConfig::default_locale` · framework/src/localization/config.rs:68
  - [ ] fn `suprnova::LocalizationConfig::fallback_locale` · framework/src/localization/config.rs:74
  - [ ] fn `suprnova::LocalizationConfig::use_isolating` · framework/src/localization/config.rs:80
  - [ ] fn `suprnova::LocalizationConfig::detection` · framework/src/localization/config.rs:86
  - [ ] fn `suprnova::LocalizationConfig::session_key` · framework/src/localization/config.rs:92
  - [ ] fn `suprnova::LocalizationConfig::cookie_name` · framework/src/localization/config.rs:98
  - [ ] fn `suprnova::LocalizationConfig::parent` · framework/src/localization/config.rs:107
- [ ] enum `suprnova::Detect` · framework/src/localization/config.rs:10 (feature: `localization`; also `suprnova::localization::Detect`)
  - Variants: `Session`, `Cookie`, `Header`

### `suprnova::localization::fluent` (private module; items are public through re-exports)

- [ ] struct `suprnova::FluentTranslator` · framework/src/localization/fluent.rs:68 (feature: `localization`; also `suprnova::localization::FluentTranslator`)
  - Implements: `suprnova::Translator`
  - [ ] fn `suprnova::FluentTranslator::from_dir` · framework/src/localization/fluent.rs:106
  - [ ] fn `suprnova::FluentTranslator::reload_if_stale` · framework/src/localization/fluent.rs:129

### `suprnova::localization::format` (private module; items are public through re-exports)

- [ ] enum `suprnova::DateStyle` · framework/src/localization/format.rs:45 (feature: `localization`; also `suprnova::localization::DateStyle`)
  - Variants: `Full`, `Long`, `Medium`, `Short`
- [ ] enum `suprnova::ListStyle` · framework/src/localization/format.rs:71 (feature: `localization`; also `suprnova::localization::ListStyle`)
  - Variants: `And`, `Or`, `Unit`
- [ ] enum `suprnova::RelativeUnit` · framework/src/localization/format.rs:82 (feature: `localization`; also `suprnova::localization::RelativeUnit`)
  - Variants: `Second`, `Minute`, `Hour`, `Day`, `Week`, `Month`, `Year`
- [ ] enum `suprnova::TimeStyle` · framework/src/localization/format.rs:62 (feature: `localization`; also `suprnova::localization::TimeStyle`)
  - Variants: `Medium`, `Short`

### `suprnova::localization::locale` (private module; items are public through re-exports)

- [ ] fn `suprnova::localization::negotiate` · framework/src/localization/locale.rs:70 (feature: `localization`)
- [ ] struct `suprnova::Locale` · framework/src/localization/locale.rs:14 (feature: `localization`; also `suprnova::localization::Locale`)
  - [ ] fn `suprnova::Locale::parse` · framework/src/localization/locale.rs:18
  - [ ] fn `suprnova::Locale::as_str` · framework/src/localization/locale.rs:27
  - [ ] fn `suprnova::Locale::language` · framework/src/localization/locale.rs:32

### `suprnova::localization::middleware` (private module; items are public through re-exports)

- [ ] struct `suprnova::LocaleMiddleware` · framework/src/localization/middleware.rs:36 (feature: `localization`; also `suprnova::localization::LocaleMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::LocaleMiddleware::new` · framework/src/localization/middleware.rs:44
  - [ ] fn `suprnova::LocaleMiddleware::from_env` · framework/src/localization/middleware.rs:54

### `suprnova::localization::translator` (private module; items are public through re-exports)

- [ ] struct `suprnova::CatalogSource` · framework/src/localization/translator.rs:11 (feature: `localization`; also `suprnova::localization::CatalogSource`)
  - Public fields: `text`, `hash`
- [ ] trait `suprnova::Translator` · framework/src/localization/translator.rs:23 (feature: `localization`; also `suprnova::localization::Translator`)
  - Implemented here by: `FluentTranslator`
  - [ ] fn `suprnova::Translator::translate` · framework/src/localization/translator.rs:28 (required)
  - [ ] fn `suprnova::Translator::has` · framework/src/localization/translator.rs:36 (required)
  - [ ] fn `suprnova::Translator::available_locales` · framework/src/localization/translator.rs:39 (required)
  - [ ] fn `suprnova::Translator::catalog` · framework/src/localization/translator.rs:47 (required)
  - [ ] fn `suprnova::Translator::reload` · framework/src/localization/translator.rs:50 (required)
  - [ ] fn `suprnova::Translator::reload_if_stale` · framework/src/localization/translator.rs:59 (provided)

## logging

### `suprnova::logging::config`

- [ ] struct `suprnova::LogConfig` · framework/src/logging/config.rs:17 (also `suprnova::logging::LogConfig`, `suprnova::logging::config::LogConfig`)
  - Public fields: `level`, `format`
  - [ ] fn `suprnova::LogConfig::from_env` · framework/src/logging/config.rs:36
- [ ] enum `suprnova::LogFormat` · framework/src/logging/config.rs:8 (also `suprnova::logging::LogFormat`, `suprnova::logging::config::LogFormat`)
  - Variants: `Pretty`, `Json`

### `suprnova::logging::init`

- [ ] fn `suprnova::init_subscriber` · framework/src/logging/init.rs:57 (also `suprnova::logging::init::init_subscriber`, `suprnova::logging::init_subscriber`)

### `suprnova::logging::request_id`

- [ ] fn `suprnova::current_request_id` · framework/src/logging/request_id.rs:58 (also `suprnova::logging::current_request_id`, `suprnova::logging::request_id::current_request_id`)
- [ ] fn `suprnova::spawn_with_request_id` · framework/src/logging/request_id.rs:80 (also `suprnova::logging::request_id::spawn_with_request_id`, `suprnova::logging::spawn_with_request_id`)
- [ ] struct `suprnova::RequestId` · framework/src/logging/request_id.rs:16 (also `suprnova::logging::RequestId`, `suprnova::logging::request_id::RequestId`)
  - [ ] fn `suprnova::RequestId::new` · framework/src/logging/request_id.rs:20
  - [ ] fn `suprnova::RequestId::from_string` · framework/src/logging/request_id.rs:26
  - [ ] fn `suprnova::RequestId::as_str` · framework/src/logging/request_id.rs:31
- [ ] struct `suprnova::RequestIdMiddleware` · framework/src/logging/request_id.rs:104 (also `suprnova::logging::RequestIdMiddleware`, `suprnova::logging::request_id::RequestIdMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::RequestIdMiddleware::new` · framework/src/logging/request_id.rs:116
  - [ ] fn `suprnova::RequestIdMiddleware::with_id` · framework/src/logging/request_id.rs:127
- [ ] static `suprnova::logging::REQUEST_ID` · framework/src/logging/request_id.rs:48 (also `suprnova::logging::request_id::REQUEST_ID`)

## magnetar_integration

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

### `suprnova::magnetar_integration::abuse_limiter`

- [ ] struct `suprnova::FrameworkAbuseLimiter` · framework/src/magnetar_integration/abuse_limiter.rs:128 (feature: `magnetar-oauth`; also `suprnova::magnetar_integration::abuse_limiter::FrameworkAbuseLimiter`)
  - [ ] fn `suprnova::FrameworkAbuseLimiter::new` · framework/src/magnetar_integration/abuse_limiter.rs:134

### `suprnova::magnetar_integration::ceremony`

- [ ] fn `suprnova::magnetar_integration::ceremony::consume` · framework/src/magnetar_integration/ceremony.rs:72
- [ ] fn `suprnova::magnetar_integration::ceremony::issue` · framework/src/magnetar_integration/ceremony.rs:38
- [ ] fn `suprnova::magnetar_integration::ceremony::prune_expired` · framework/src/magnetar_integration/ceremony.rs:117

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

### `suprnova::magnetar_integration::magic_link`

- [ ] struct `suprnova::magnetar_integration::magic_link::MagicLinkAuth` · framework/src/magnetar_integration/magic_link.rs:7
  - [ ] fn `suprnova::magnetar_integration::magic_link::MagicLinkAuth::send` · framework/src/magnetar_integration/magic_link.rs:19
  - [ ] fn `suprnova::magnetar_integration::magic_link::MagicLinkAuth::consume` · framework/src/magnetar_integration/magic_link.rs:38
  - [ ] fn `suprnova::magnetar_integration::magic_link::MagicLinkAuth::consume_outcome` · framework/src/magnetar_integration/magic_link.rs:53

### `suprnova::magnetar_integration::middleware` (feature: `database-sqlite` or `database-postgres` or `database-mysql`)

- [ ] struct `suprnova::BearerTokenMiddleware` · framework/src/magnetar_integration/middleware.rs:11 (also `suprnova::magnetar_integration::middleware::BearerTokenMiddleware`)
  - Implements: `suprnova::Middleware`

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

### `suprnova::magnetar_integration::oauth_transport` (feature: `magnetar-oauth`)

- [ ] struct `suprnova::ReqwestOAuthTransport` · framework/src/magnetar_integration/oauth_transport.rs:24 (also `suprnova::magnetar_integration::oauth_transport::ReqwestOAuthTransport`)
  - [ ] fn `suprnova::ReqwestOAuthTransport::try_default` · framework/src/magnetar_integration/oauth_transport.rs:31
  - [ ] fn `suprnova::ReqwestOAuthTransport::new` · framework/src/magnetar_integration/oauth_transport.rs:45
  - [ ] fn `suprnova::ReqwestOAuthTransport::with_max_response_bytes` · framework/src/magnetar_integration/oauth_transport.rs:53

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

### `suprnova::magnetar_integration::password`

- [ ] struct `suprnova::magnetar_integration::password::PasswordAuth` · framework/src/magnetar_integration/password.rs:7
  - [ ] fn `suprnova::magnetar_integration::password::PasswordAuth::register` · framework/src/magnetar_integration/password.rs:15
  - [ ] fn `suprnova::magnetar_integration::password::PasswordAuth::authenticate` · framework/src/magnetar_integration/password.rs:37
  - [ ] fn `suprnova::magnetar_integration::password::PasswordAuth::authenticate_outcome` · framework/src/magnetar_integration/password.rs:58

### `suprnova::magnetar_integration::sign_in` (private module; items are public through re-exports)

- [ ] enum `suprnova::SignInOutcome` · framework/src/magnetar_integration/sign_in.rs:16 (also `suprnova::magnetar_integration::SignInOutcome`)
  - Variants: `Authenticated`, `FactorRequired`

## mail

### `suprnova::mail`

- [ ] struct `suprnova::Mail` · framework/src/mail/mod.rs:109 (also `suprnova::mail::Mail`, `suprnova::prelude::Mail`)
  - [ ] fn `suprnova::Mail::set_transport` · framework/src/mail/mod.rs:114
  - [ ] fn `suprnova::Mail::clear_transport` · framework/src/mail/mod.rs:120
  - [ ] fn `suprnova::Mail::to` · framework/src/mail/mod.rs:127
  - [ ] fn `suprnova::Mail::cc` · framework/src/mail/mod.rs:132
  - [ ] fn `suprnova::Mail::bcc` · framework/src/mail/mod.rs:137
  - [ ] fn `suprnova::Mail::raw` · framework/src/mail/mod.rs:152
  - [ ] fn `suprnova::Mail::html` · framework/src/mail/mod.rs:162
  - [ ] fn `suprnova::Mail::always_from` · framework/src/mail/mod.rs:173
  - [ ] fn `suprnova::Mail::always_reply_to` · framework/src/mail/mod.rs:181
  - [ ] fn `suprnova::Mail::always_to` · framework/src/mail/mod.rs:189
  - [ ] fn `suprnova::Mail::always_return_path` · framework/src/mail/mod.rs:196
  - [ ] fn `suprnova::Mail::forget_always` · framework/src/mail/mod.rs:204
  - [ ] fn `suprnova::Mail::fake` · framework/src/mail/mod.rs:226
- [ ] struct `suprnova::MailBuilder` · framework/src/mail/mod.rs:286 (also `suprnova::mail::MailBuilder`)
  - [ ] fn `suprnova::MailBuilder::to` · framework/src/mail/mod.rs:310
  - [ ] fn `suprnova::MailBuilder::cc` · framework/src/mail/mod.rs:315
  - [ ] fn `suprnova::MailBuilder::bcc` · framework/src/mail/mod.rs:320
  - [ ] fn `suprnova::MailBuilder::reply_to` · framework/src/mail/mod.rs:325
  - [ ] fn `suprnova::MailBuilder::from` · framework/src/mail/mod.rs:331
  - [ ] fn `suprnova::MailBuilder::return_path` · framework/src/mail/mod.rs:336
  - [ ] fn `suprnova::MailBuilder::on_queue` · framework/src/mail/mod.rs:346
  - [ ] fn `suprnova::MailBuilder::on_connection` · framework/src/mail/mod.rs:352
  - [ ] fn `suprnova::MailBuilder::tag` · framework/src/mail/mod.rs:358
  - [ ] fn `suprnova::MailBuilder::metadata` · framework/src/mail/mod.rs:363
  - [ ] fn `suprnova::MailBuilder::priority` · framework/src/mail/mod.rs:369
  - [ ] fn `suprnova::MailBuilder::header` · framework/src/mail/mod.rs:374
  - [ ] fn `suprnova::MailBuilder::subject` · framework/src/mail/mod.rs:380
  - [ ] fn `suprnova::MailBuilder::html` · framework/src/mail/mod.rs:385
  - [ ] fn `suprnova::MailBuilder::text` · framework/src/mail/mod.rs:390
  - [ ] fn `suprnova::MailBuilder::attach` · framework/src/mail/mod.rs:396
  - [ ] fn `suprnova::MailBuilder::reply_to_address` · framework/src/mail/mod.rs:405
  - [ ] fn `suprnova::MailBuilder::send` · framework/src/mail/mod.rs:410
  - [ ] fn `suprnova::MailBuilder::queue` · framework/src/mail/mod.rs:499
  - [ ] fn `suprnova::MailBuilder::later` · framework/src/mail/mod.rs:524
- [ ] struct `suprnova::MailFake` · framework/src/mail/mod.rs:642 (also `suprnova::mail::MailFake`, `suprnova::prelude::MailFake`)
  - [ ] fn `suprnova::MailFake::captured` · framework/src/mail/mod.rs:649
  - [ ] fn `suprnova::MailFake::count` · framework/src/mail/mod.rs:654
  - [ ] fn `suprnova::MailFake::queued` · framework/src/mail/mod.rs:661
  - [ ] fn `suprnova::MailFake::queued_count` · framework/src/mail/mod.rs:678
  - [ ] fn `suprnova::MailFake::outgoing_count` · framework/src/mail/mod.rs:684
  - [ ] fn `suprnova::MailFake::sent` · framework/src/mail/mod.rs:689
  - [ ] fn `suprnova::MailFake::sent_to` · framework/src/mail/mod.rs:702
  - [ ] fn `suprnova::MailFake::queued_named` · framework/src/mail/mod.rs:707
  - [ ] fn `suprnova::MailFake::queued_to` · framework/src/mail/mod.rs:715
  - [ ] fn `suprnova::MailFake::assert_sent` · framework/src/mail/mod.rs:724
  - [ ] fn `suprnova::MailFake::assert_sent_to` · framework/src/mail/mod.rs:742
  - [ ] fn `suprnova::MailFake::assert_not_sent` · framework/src/mail/mod.rs:754
  - [ ] fn `suprnova::MailFake::assert_not_sent_to` · framework/src/mail/mod.rs:768
  - [ ] fn `suprnova::MailFake::assert_sent_count` · framework/src/mail/mod.rs:778
  - [ ] fn `suprnova::MailFake::assert_nothing_sent` · framework/src/mail/mod.rs:787
  - [ ] fn `suprnova::MailFake::assert_queued` · framework/src/mail/mod.rs:799
  - [ ] fn `suprnova::MailFake::assert_queued_with` · framework/src/mail/mod.rs:815
  - [ ] fn `suprnova::MailFake::assert_not_queued` · framework/src/mail/mod.rs:831
  - [ ] fn `suprnova::MailFake::assert_queued_to` · framework/src/mail/mod.rs:844
  - [ ] fn `suprnova::MailFake::assert_nothing_queued` · framework/src/mail/mod.rs:856
  - [ ] fn `suprnova::MailFake::assert_queued_count` · framework/src/mail/mod.rs:867
  - [ ] fn `suprnova::MailFake::queued_on` · framework/src/mail/mod.rs:876
  - [ ] fn `suprnova::MailFake::assert_queued_on` · framework/src/mail/mod.rs:887
  - [ ] fn `suprnova::MailFake::queued_on_connection` · framework/src/mail/mod.rs:907
  - [ ] fn `suprnova::MailFake::assert_queued_on_connection` · framework/src/mail/mod.rs:921
  - [ ] fn `suprnova::MailFake::assert_not_outgoing` · framework/src/mail/mod.rs:938
  - [ ] fn `suprnova::MailFake::assert_nothing_outgoing` · framework/src/mail/mod.rs:947
  - [ ] fn `suprnova::MailFake::assert_outgoing_count` · framework/src/mail/mod.rs:953
- [ ] struct `suprnova::QueuedSnapshot` · framework/src/mail/mod.rs:981 (also `suprnova::mail::QueuedSnapshot`)
  - Public fields: `mailable_name`, `payload`, `to`, `cc`, `bcc`, `delay`, `queue`, `connection`
  - [ ] fn `suprnova::QueuedSnapshot::decode` · framework/src/mail/mod.rs:1008
  - [ ] fn `suprnova::QueuedSnapshot::has_to` · framework/src/mail/mod.rs:1014

### `suprnova::mail::address`

- [ ] struct `suprnova::Address` · framework/src/mail/address.rs:8 (also `suprnova::mail::Address`, `suprnova::mail::address::Address`, `suprnova::prelude::Address`)
  - Public fields: `email`, `name`
  - [ ] fn `suprnova::Address::new` · framework/src/mail/address.rs:17
  - [ ] fn `suprnova::Address::with_name` · framework/src/mail/address.rs:24
- [ ] struct `suprnova::Attachment` · framework/src/mail/address.rs:73 (also `suprnova::mail::Attachment`, `suprnova::mail::address::Attachment`, `suprnova::prelude::Attachment`)
  - Public fields: `filename`, `content`, `content_type`
  - [ ] fn `suprnova::Attachment::new` · framework/src/mail/address.rs:85

### `suprnova::mail::boot`

- [ ] fn `suprnova::mail::boot::bootstrap_from_env` · framework/src/mail/boot.rs:359
- [ ] fn `suprnova::mail::boot::captured_in_memory` · framework/src/mail/boot.rs:32

### `suprnova::mail::events`

- [ ] struct `suprnova::MessageSending` · framework/src/mail/events.rs:21 (also `suprnova::mail::MessageSending`, `suprnova::mail::events::MessageSending`)
  - Public fields: `from`, `to`, `cc`, `bcc`, `reply_to`, `subject`, `has_html`, `has_text`, `attachment_count`, `tags`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::MessageSent` · framework/src/mail/events.rs:55 (also `suprnova::mail::MessageSent`, `suprnova::mail::events::MessageSent`)
  - Public fields: `from`, `to`, `cc`, `bcc`, `reply_to`, `subject`, `has_html`, `has_text`, `attachment_count`, `tags`
  - Implements: `suprnova::Event`

### `suprnova::mail::file`

- [ ] struct `suprnova::mail::file::FileMailTransport` · framework/src/mail/file.rs:20
  - Implements: `suprnova::mail::MailTransport`
  - [ ] fn `suprnova::mail::file::FileMailTransport::new` · framework/src/mail/file.rs:30

### `suprnova::mail::log`

- [ ] struct `suprnova::mail::log::LogMailTransport` · framework/src/mail/log.rs:39
  - Implements: `suprnova::mail::MailTransport`
  - [ ] fn `suprnova::mail::log::LogMailTransport::new` · framework/src/mail/log.rs:43

### `suprnova::mail::mailable`

- [ ] fn `suprnova::mail::register_mailable_factory` · framework/src/mail/mailable.rs:184 (also `suprnova::mail::mailable::register_mailable_factory`)
- [ ] trait `suprnova::Mailable` · framework/src/mail/mailable.rs:33 (also `suprnova::mail::Mailable`, `suprnova::mail::mailable::Mailable`, `suprnova::prelude::Mailable`)
  - Implemented here by: `EmailVerificationMail`, `PasswordChangedMail`, `PasswordResetMail`
  - [ ] fn `suprnova::Mailable::mailable_name` · framework/src/mail/mailable.rs:35 (required)
  - [ ] fn `suprnova::Mailable::subject` · framework/src/mail/mailable.rs:44 (required)
  - [ ] fn `suprnova::Mailable::subject_template_source` · framework/src/mail/mailable.rs:57 (provided)
  - [ ] fn `suprnova::Mailable::html_template_source` · framework/src/mail/mailable.rs:62 (provided)
  - [ ] fn `suprnova::Mailable::text_template_source` · framework/src/mail/mailable.rs:67 (provided)
  - [ ] fn `suprnova::Mailable::from` · framework/src/mail/mailable.rs:72 (provided)
  - [ ] fn `suprnova::Mailable::attachments` · framework/src/mail/mailable.rs:77 (provided)
  - [ ] fn `suprnova::Mailable::tags` · framework/src/mail/mailable.rs:84 (provided)
  - [ ] fn `suprnova::Mailable::metadata` · framework/src/mail/mailable.rs:91 (provided)
  - [ ] fn `suprnova::Mailable::priority` · framework/src/mail/mailable.rs:97 (provided)
  - [ ] fn `suprnova::Mailable::headers` · framework/src/mail/mailable.rs:104 (provided)
  - [ ] fn `suprnova::Mailable::return_path` · framework/src/mail/mailable.rs:110 (provided)
  - [ ] fn `suprnova::Mailable::queue` · framework/src/mail/mailable.rs:124 (provided)
  - [ ] fn `suprnova::Mailable::render_subject` · framework/src/mail/mailable.rs:135 (provided)
  - [ ] fn `suprnova::Mailable::render_html` · framework/src/mail/mailable.rs:145 (provided)
  - [ ] fn `suprnova::Mailable::render_text` · framework/src/mail/mailable.rs:153 (provided)

### `suprnova::mail::mailable_registry`

- [ ] fn `suprnova::mail::mailable_registry::build` · framework/src/mail/mailable_registry.rs:116
- [ ] fn `suprnova::mail::mailable_registry::register` · framework/src/mail/mailable_registry.rs:100
- [ ] fn `suprnova::mail::mailable_registry::render_outgoing` · framework/src/mail/mailable_registry.rs:140
- [ ] trait `suprnova::mail::mailable_registry::AnyMailable` · framework/src/mail/mailable_registry.rs:39
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::render_subject` · framework/src/mail/mailable_registry.rs:41 (required)
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::render_html` · framework/src/mail/mailable_registry.rs:43 (required)
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::render_text` · framework/src/mail/mailable_registry.rs:45 (required)
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::from` · framework/src/mail/mailable_registry.rs:47 (required)
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::attachments` · framework/src/mail/mailable_registry.rs:49 (required)
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::tags` · framework/src/mail/mailable_registry.rs:51 (required)
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::metadata` · framework/src/mail/mailable_registry.rs:53 (required)
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::priority` · framework/src/mail/mailable_registry.rs:55 (required)
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::headers` · framework/src/mail/mailable_registry.rs:57 (required)
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::return_path` · framework/src/mail/mailable_registry.rs:59 (required)

### `suprnova::mail::mailgun`

- [ ] struct `suprnova::mail::mailgun::MailgunMailTransport` · framework/src/mail/mailgun.rs:20
  - Implements: `suprnova::mail::MailTransport`
  - [ ] fn `suprnova::mail::mailgun::MailgunMailTransport::new` · framework/src/mail/mailgun.rs:30
  - [ ] fn `suprnova::mail::mailgun::MailgunMailTransport::with_endpoint` · framework/src/mail/mailgun.rs:40

### `suprnova::mail::memory`

- [ ] struct `suprnova::mail::memory::InMemoryMailTransport` · framework/src/mail/memory.rs:11
  - Implements: `suprnova::mail::MailTransport`
  - [ ] fn `suprnova::mail::memory::InMemoryMailTransport::new` · framework/src/mail/memory.rs:17
  - [ ] fn `suprnova::mail::memory::InMemoryMailTransport::captured` · framework/src/mail/memory.rs:22
  - [ ] fn `suprnova::mail::memory::InMemoryMailTransport::clear` · framework/src/mail/memory.rs:27

### `suprnova::mail::postmark`

- [ ] struct `suprnova::mail::postmark::PostmarkMailTransport` · framework/src/mail/postmark.rs:15
  - Implements: `suprnova::mail::MailTransport`
  - [ ] fn `suprnova::mail::postmark::PostmarkMailTransport::new` · framework/src/mail/postmark.rs:22
  - [ ] fn `suprnova::mail::postmark::PostmarkMailTransport::with_endpoint` · framework/src/mail/postmark.rs:32

### `suprnova::mail::resend`

- [ ] struct `suprnova::mail::resend::ResendMailTransport` · framework/src/mail/resend.rs:16
  - Implements: `suprnova::mail::MailTransport`
  - [ ] fn `suprnova::mail::resend::ResendMailTransport::new` · framework/src/mail/resend.rs:23
  - [ ] fn `suprnova::mail::resend::ResendMailTransport::with_endpoint` · framework/src/mail/resend.rs:33

### `suprnova::mail::send_job`

- [ ] struct `suprnova::SendMailJob` · framework/src/mail/send_job.rs:26 (also `suprnova::mail::SendMailJob`, `suprnova::mail::send_job::SendMailJob`)
  - Public fields: `to`, `cc`, `bcc`, `reply_to`, `from_override`, `mailable_name`, `mailable_payload`, `tags`, `metadata`, `priority`, `headers`, `return_path`, `subject_override`, `attachments`
  - Implements: `suprnova::Job`

### `suprnova::mail::sendgrid`

- [ ] struct `suprnova::mail::sendgrid::SendGridMailTransport` · framework/src/mail/sendgrid.rs:15
  - Implements: `suprnova::mail::MailTransport`
  - [ ] fn `suprnova::mail::sendgrid::SendGridMailTransport::new` · framework/src/mail/sendgrid.rs:22
  - [ ] fn `suprnova::mail::sendgrid::SendGridMailTransport::with_endpoint` · framework/src/mail/sendgrid.rs:32

### `suprnova::mail::ses`

- [ ] struct `suprnova::mail::ses::SesMailTransport` · framework/src/mail/ses.rs:52
  - Implements: `suprnova::mail::MailTransport`
  - [ ] fn `suprnova::mail::ses::SesMailTransport::new` · framework/src/mail/ses.rs:65
  - [ ] fn `suprnova::mail::ses::SesMailTransport::with_endpoint` · framework/src/mail/ses.rs:86
  - [ ] fn `suprnova::mail::ses::SesMailTransport::tenant_name` · framework/src/mail/ses.rs:116
  - [ ] fn `suprnova::mail::ses::SesMailTransport::configuration_set_name` · framework/src/mail/ses.rs:129
  - [ ] fn `suprnova::mail::ses::SesMailTransport::list_management` · framework/src/mail/ses.rs:141

### `suprnova::mail::smtp`

- [ ] struct `suprnova::mail::smtp::SmtpMailTransport` · framework/src/mail/smtp.rs:11
  - Implements: `suprnova::mail::MailTransport`
  - [ ] fn `suprnova::mail::smtp::SmtpMailTransport::starttls` · framework/src/mail/smtp.rs:18
  - [ ] fn `suprnova::mail::smtp::SmtpMailTransport::tls` · framework/src/mail/smtp.rs:35
  - [ ] fn `suprnova::mail::smtp::SmtpMailTransport::unencrypted` · framework/src/mail/smtp.rs:46

### `suprnova::mail::transport`

- [ ] fn `suprnova::mail::dispatch_with_telemetry` · framework/src/mail/transport.rs:195 (also `suprnova::mail::transport::dispatch_with_telemetry`)
- [ ] struct `suprnova::OutgoingMessage` · framework/src/mail/transport.rs:35 (also `suprnova::mail::OutgoingMessage`, `suprnova::mail::transport::OutgoingMessage`)
  - Public fields: `from`, `to`, `cc`, `bcc`, `reply_to`, `subject`, `html`, `text`, `attachments`, `tags`, `metadata`, `priority`, `headers`, `return_path`
  - [ ] fn `suprnova::OutgoingMessage::new` · framework/src/mail/transport.rs:78
  - [ ] fn `suprnova::OutgoingMessage::has_to` · framework/src/mail/transport.rs:100
  - [ ] fn `suprnova::OutgoingMessage::has_cc` · framework/src/mail/transport.rs:105
  - [ ] fn `suprnova::OutgoingMessage::has_bcc` · framework/src/mail/transport.rs:110
  - [ ] fn `suprnova::OutgoingMessage::has_reply_to` · framework/src/mail/transport.rs:115
  - [ ] fn `suprnova::OutgoingMessage::has_from` · framework/src/mail/transport.rs:120
  - [ ] fn `suprnova::OutgoingMessage::has_subject` · framework/src/mail/transport.rs:125
  - [ ] fn `suprnova::OutgoingMessage::has_attachment` · framework/src/mail/transport.rs:131
  - [ ] fn `suprnova::OutgoingMessage::has_tag` · framework/src/mail/transport.rs:136
  - [ ] fn `suprnova::OutgoingMessage::has_metadata` · framework/src/mail/transport.rs:141
  - [ ] fn `suprnova::OutgoingMessage::metadata_equals` · framework/src/mail/transport.rs:146
  - [ ] fn `suprnova::OutgoingMessage::has_header` · framework/src/mail/transport.rs:151
- [ ] trait `suprnova::mail::MailTransport` · framework/src/mail/transport.rs:165 (also `suprnova::mail::transport::MailTransport`)
  - Implemented here by: `mail::file::FileMailTransport`, `mail::log::LogMailTransport`, `mail::mailgun::MailgunMailTransport`, `mail::memory::InMemoryMailTransport`, `mail::postmark::PostmarkMailTransport`, `mail::resend::ResendMailTransport`, `mail::sendgrid::SendGridMailTransport`, `mail::ses::SesMailTransport`, `mail::smtp::SmtpMailTransport`
  - [ ] fn `suprnova::mail::MailTransport::send` · framework/src/mail/transport.rs:169 (required)
  - [ ] fn `suprnova::mail::MailTransport::name` · framework/src/mail/transport.rs:173 (provided)
- [ ] const `suprnova::mail::PRIORITY_HIGH` · framework/src/mail/transport.rs:25 (also `suprnova::mail::transport::PRIORITY_HIGH`)
- [ ] const `suprnova::mail::PRIORITY_HIGHEST` · framework/src/mail/transport.rs:23 (also `suprnova::mail::transport::PRIORITY_HIGHEST`)
- [ ] const `suprnova::mail::PRIORITY_LOW` · framework/src/mail/transport.rs:29 (also `suprnova::mail::transport::PRIORITY_LOW`)
- [ ] const `suprnova::mail::PRIORITY_LOWEST` · framework/src/mail/transport.rs:31 (also `suprnova::mail::transport::PRIORITY_LOWEST`)
- [ ] const `suprnova::mail::PRIORITY_NORMAL` · framework/src/mail/transport.rs:27 (also `suprnova::mail::transport::PRIORITY_NORMAL`)

## media

### `suprnova::media` (feature: `media`)

- [ ] fn `suprnova::media::config` · framework/src/media/mod.rs:237
- [ ] fn `suprnova::media::default_driver` · framework/src/media/mod.rs:263
- [ ] fn `suprnova::media::set_default_driver` · framework/src/media/mod.rs:289
- [ ] struct `suprnova::Image` · framework/src/media/mod.rs:323 (also `suprnova::media::Image`)
  - [ ] fn `suprnova::Image::from_bytes` · framework/src/media/mod.rs:344
  - [ ] fn `suprnova::Image::from_path` · framework/src/media/mod.rs:349
  - [ ] fn `suprnova::Image::from_disk` · framework/src/media/mod.rs:356
  - [ ] fn `suprnova::Image::from_upload` · framework/src/media/mod.rs:368
  - [ ] fn `suprnova::Image::from_stream` · framework/src/media/mod.rs:383
  - [ ] fn `suprnova::Image::resize` · framework/src/media/mod.rs:411
  - [ ] fn `suprnova::Image::resize_width` · framework/src/media/mod.rs:416
  - [ ] fn `suprnova::Image::resize_height` · framework/src/media/mod.rs:421
  - [ ] fn `suprnova::Image::scale` · framework/src/media/mod.rs:426
  - [ ] fn `suprnova::Image::scale_width` · framework/src/media/mod.rs:431
  - [ ] fn `suprnova::Image::scale_height` · framework/src/media/mod.rs:436
  - [ ] fn `suprnova::Image::crop` · framework/src/media/mod.rs:441
  - [ ] fn `suprnova::Image::cover` · framework/src/media/mod.rs:451
  - [ ] fn `suprnova::Image::contain` · framework/src/media/mod.rs:456
  - [ ] fn `suprnova::Image::rotate` · framework/src/media/mod.rs:461
  - [ ] fn `suprnova::Image::flip_vertically` · framework/src/media/mod.rs:466
  - [ ] fn `suprnova::Image::flip_horizontally` · framework/src/media/mod.rs:471
  - [ ] fn `suprnova::Image::blur` · framework/src/media/mod.rs:476
  - [ ] fn `suprnova::Image::sharpen` · framework/src/media/mod.rs:481
  - [ ] fn `suprnova::Image::grayscale` · framework/src/media/mod.rs:486
  - [ ] fn `suprnova::Image::to_format` · framework/src/media/mod.rs:491
  - [ ] fn `suprnova::Image::quality` · framework/src/media/mod.rs:499
  - [ ] fn `suprnova::Image::to_bytes` · framework/src/media/mod.rs:526
  - [ ] fn `suprnova::Image::to_response` · framework/src/media/mod.rs:533
  - [ ] fn `suprnova::Image::save` · framework/src/media/mod.rs:544
  - [ ] fn `suprnova::Image::store` · framework/src/media/mod.rs:553
  - [ ] fn `suprnova::Image::dimensions` · framework/src/media/mod.rs:560
  - [ ] fn `suprnova::Image::mime_type` · framework/src/media/mod.rs:572
  - [ ] fn `suprnova::Image::dominant_color` · framework/src/media/mod.rs:582
- [ ] struct `suprnova::ImageConfig` · framework/src/media/mod.rs:135 (also `suprnova::media::ImageConfig`)
  - Public fields: `max_dimension`, `max_alloc_bytes`, `magick_timeout_secs`
  - [ ] fn `suprnova::ImageConfig::from_env` · framework/src/media/mod.rs:205
- [ ] enum `suprnova::ImageDriverKind` · framework/src/media/mod.rs:89 (also `suprnova::media::ImageDriverKind`)
  - Variants: `OxideAv`, `Magick`
  - [ ] fn `suprnova::ImageDriverKind::parse` · framework/src/media/mod.rs:103
  - [ ] fn `suprnova::ImageDriverKind::from_env` · framework/src/media/mod.rs:114
- [ ] const `suprnova::DEFAULT_IMAGE_MAGICK_TIMEOUT_SECS` · framework/src/media/mod.rs:85 (also `suprnova::media::DEFAULT_IMAGE_MAGICK_TIMEOUT_SECS`)
- [ ] const `suprnova::DEFAULT_IMAGE_MAX_ALLOC_BYTES` · framework/src/media/mod.rs:78 (also `suprnova::media::DEFAULT_IMAGE_MAX_ALLOC_BYTES`)
- [ ] const `suprnova::DEFAULT_IMAGE_MAX_DIMENSION` · framework/src/media/mod.rs:75 (also `suprnova::media::DEFAULT_IMAGE_MAX_DIMENSION`)

### `suprnova::media::driver` (private module; items are public through re-exports)

- [ ] struct `suprnova::ImagePipeline` · framework/src/media/driver.rs:141 (feature: `media`; also `suprnova::media::ImagePipeline`)
  - Public fields: `transformations`, `format`, `quality`
- [ ] enum `suprnova::OutputFormat` · framework/src/media/driver.rs:30 (feature: `media`; also `suprnova::media::OutputFormat`)
  - Variants: `Jpeg`, `Png`, `WebP`, `Gif`, `Bmp`
  - [ ] fn `suprnova::OutputFormat::mime_type` · framework/src/media/driver.rs:49
  - [ ] fn `suprnova::OutputFormat::extension` · framework/src/media/driver.rs:60
- [ ] enum `suprnova::Transformation` · framework/src/media/driver.rs:77 (feature: `media`; also `suprnova::media::Transformation`)
  - Variants: `Resize`, `ResizeWidth`, `ResizeHeight`, `Scale`, `ScaleWidth`, `ScaleHeight`, `Crop`, `Cover`, `Contain`, `Rotate`, `FlipVertically`, `FlipHorizontally`, `Blur`, `Sharpen`, `Grayscale`
- [ ] trait `suprnova::ImageDriver` · framework/src/media/driver.rs:184 (feature: `media`; also `suprnova::media::ImageDriver`)
  - Implemented here by: `MagickCliDriver`, `OxideAvImageDriver`
  - [ ] fn `suprnova::ImageDriver::process` · framework/src/media/driver.rs:188 (required)
  - [ ] fn `suprnova::ImageDriver::dimensions` · framework/src/media/driver.rs:195 (required)
  - [ ] fn `suprnova::ImageDriver::dominant_color` · framework/src/media/driver.rs:200 (required)
  - [ ] fn `suprnova::ImageDriver::name` · framework/src/media/driver.rs:203 (required)
- [ ] const `suprnova::DEFAULT_IMAGE_QUALITY` · framework/src/media/driver.rs:21 (feature: `media`; also `suprnova::media::DEFAULT_IMAGE_QUALITY`)

### `suprnova::media::magick` (private module; items are public through re-exports)

- [ ] struct `suprnova::MagickCliDriver` · framework/src/media/magick.rs:80 (feature: `media`; also `suprnova::media::MagickCliDriver`)
  - Implements: `suprnova::ImageDriver`
  - [ ] fn `suprnova::MagickCliDriver::new` · framework/src/media/magick.rs:86
  - [ ] fn `suprnova::MagickCliDriver::from_env` · framework/src/media/magick.rs:93
  - [ ] fn `suprnova::MagickCliDriver::binary` · framework/src/media/magick.rs:102

### `suprnova::media::oxideav` (private module; items are public through re-exports)

- [ ] struct `suprnova::OxideAvImageDriver` · framework/src/media/oxideav.rs:172 (feature: `media`; also `suprnova::media::OxideAvImageDriver`)
  - Implements: `suprnova::ImageDriver`
  - [ ] fn `suprnova::OxideAvImageDriver::new` · framework/src/media/oxideav.rs:182

## middleware

### `suprnova::middleware`

- [ ] fn `suprnova::middleware::into_boxed` · framework/src/middleware/mod.rs:110
- [ ] trait `suprnova::Middleware` · framework/src/middleware/mod.rs:100 (also `suprnova::middleware::Middleware`)
  - Implemented here by: `AuthMiddleware`, `BasicAuthMiddleware`, `BearerTokenMiddleware`, `CorsMiddleware`, `CsrfMiddleware`, `EncryptHistoryMiddleware`, `EnsureEmailVerifiedMiddleware`, `GuestMiddleware`, `IncludeMiddleware`, `Inertia303Middleware`, `InertiaErrorPageMiddleware`, `InertiaHeadersMiddleware`, `InertiaValidationRedirectMiddleware`, `InertiaVersionMiddleware`, `LocaleMiddleware`, `LoginThrottleMiddleware`, `MaintenanceMiddleware`, `PermissionMiddleware`, `RateLimitMiddleware`, `RequestIdMiddleware`, `RoleMiddleware`, `SessionMiddleware`, `ThrottleRequestsMiddleware`, `TimeoutMiddleware`, `TwoFactorChallengeMiddleware`, `features::FeatureMiddleware`, `live::LiveTenantMiddleware`, `render_cache::RenderCacheMiddleware`
  - [ ] fn `suprnova::Middleware::handle` · framework/src/middleware/mod.rs:106 (required)
- [ ] type `suprnova::middleware::BoxedMiddleware` · framework/src/middleware/mod.rs:75
- [ ] type `suprnova::MiddlewareFuture` · framework/src/middleware/mod.rs:67 (also `suprnova::middleware::MiddlewareFuture`)
- [ ] type `suprnova::Next` · framework/src/middleware/mod.rs:72 (also `suprnova::middleware::Next`)

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

## notifications

### `suprnova::notifications`

- [ ] fn `suprnova::notifications::register_notification_factory` · framework/src/notifications/mod.rs:430
- [ ] fn `suprnova::notifications::set_dispatcher` · framework/src/notifications/mod.rs:405
- [ ] struct `suprnova::NotificationDispatcher` · framework/src/notifications/mod.rs:218 (also `suprnova::notifications::NotificationDispatcher`)
  - [ ] fn `suprnova::NotificationDispatcher::new` · framework/src/notifications/mod.rs:224
  - [ ] fn `suprnova::NotificationDispatcher::register_channel` · framework/src/notifications/mod.rs:234
  - [ ] fn `suprnova::NotificationDispatcher::notify` · framework/src/notifications/mod.rs:266
  - [ ] fn `suprnova::NotificationDispatcher::channel` · framework/src/notifications/mod.rs:380
- [ ] struct `suprnova::Notify` · framework/src/notifications/mod.rs:464 (also `suprnova::notifications::Notify`, `suprnova::prelude::Notify`)
  - [ ] fn `suprnova::Notify::queue` · framework/src/notifications/mod.rs:505
  - [ ] fn `suprnova::Notify::send` · framework/src/notifications/mod.rs:583
  - [ ] fn `suprnova::Notify::fake` · framework/src/notifications/mod.rs:613
  - [ ] fn `suprnova::Notify::route` · framework/src/notifications/mod.rs:638
  - [ ] fn `suprnova::Notify::routes` · framework/src/notifications/mod.rs:648
- [ ] trait `suprnova::Channel` · framework/src/notifications/mod.rs:194 (also `suprnova::notifications::Channel`)
  - Implemented here by: `BroadcastChannel`, `DatabaseChannel`, `MailChannel`, `WebPushChannel`
  - [ ] fn `suprnova::Channel::name` · framework/src/notifications/mod.rs:198 (required)
  - [ ] fn `suprnova::Channel::deliver` · framework/src/notifications/mod.rs:202 (required)
- [ ] trait `suprnova::DynNotification` · framework/src/notifications/mod.rs:159 (also `suprnova::notifications::DynNotification`)
  - [ ] fn `suprnova::DynNotification::name` · framework/src/notifications/mod.rs:161 (required)
  - [ ] fn `suprnova::DynNotification::data` · framework/src/notifications/mod.rs:163 (required)
  - [ ] fn `suprnova::DynNotification::should_send` · framework/src/notifications/mod.rs:167 (required)
  - [ ] fn `suprnova::DynNotification::after_sending` · framework/src/notifications/mod.rs:170 (required)
- [ ] trait `suprnova::Notifiable` · framework/src/notifications/mod.rs:53 (also `suprnova::notifications::Notifiable`, `suprnova::prelude::Notifiable`)
  - Implemented here by: `AnonymousNotifiable`
  - [ ] fn `suprnova::Notifiable::route_for` · framework/src/notifications/mod.rs:55 (required)
- [ ] trait `suprnova::Notification` · framework/src/notifications/mod.rs:64 (also `suprnova::notifications::Notification`, `suprnova::prelude::Notification`)
  - [ ] fn `suprnova::Notification::notification_name` · framework/src/notifications/mod.rs:67 (required)
  - [ ] fn `suprnova::Notification::channels` · framework/src/notifications/mod.rs:72 (required)
  - [ ] fn `suprnova::Notification::data` · framework/src/notifications/mod.rs:75 (required)
  - [ ] fn `suprnova::Notification::should_send` · framework/src/notifications/mod.rs:84 (provided)
  - [ ] fn `suprnova::Notification::after_sending` · framework/src/notifications/mod.rs:94 (provided)
  - [ ] fn `suprnova::Notification::queue` · framework/src/notifications/mod.rs:111 (provided)
  - [ ] fn `suprnova::Notification::timeout` · framework/src/notifications/mod.rs:119 (provided)
  - [ ] fn `suprnova::Notification::fail_on_timeout` · framework/src/notifications/mod.rs:130 (provided)
  - [ ] fn `suprnova::Notification::max_tries` · framework/src/notifications/mod.rs:138 (provided)
  - [ ] fn `suprnova::Notification::backoff` · framework/src/notifications/mod.rs:147 (provided)
- [ ] type `suprnova::NotificationFactory` · framework/src/notifications/mod.rs:393 (also `suprnova::notifications::NotificationFactory`)

### `suprnova::notifications::anonymous`

- [ ] struct `suprnova::AnonymousNotifiable` · framework/src/notifications/anonymous.rs:30 (also `suprnova::notifications::AnonymousNotifiable`, `suprnova::notifications::anonymous::AnonymousNotifiable`)
  - Implements: `suprnova::Notifiable`
  - [ ] fn `suprnova::AnonymousNotifiable::new` · framework/src/notifications/anonymous.rs:37
  - [ ] fn `suprnova::AnonymousNotifiable::route` · framework/src/notifications/anonymous.rs:47
  - [ ] fn `suprnova::AnonymousNotifiable::routes` · framework/src/notifications/anonymous.rs:66
  - [ ] fn `suprnova::AnonymousNotifiable::raw_routes` · framework/src/notifications/anonymous.rs:80

### `suprnova::notifications::channels::broadcast`

- [ ] struct `suprnova::BroadcastChannel` · framework/src/notifications/channels/broadcast.rs:36 (also `suprnova::notifications::channels::broadcast::BroadcastChannel`)
  - Implements: `suprnova::Channel`
  - [ ] fn `suprnova::BroadcastChannel::new` · framework/src/notifications/channels/broadcast.rs:40

### `suprnova::notifications::channels::database`

- [ ] struct `suprnova::DatabaseChannel` · framework/src/notifications/channels/database.rs:26 (also `suprnova::notifications::channels::database::DatabaseChannel`)
  - Implements: `suprnova::Channel`
  - [ ] fn `suprnova::DatabaseChannel::new` · framework/src/notifications/channels/database.rs:34

### `suprnova::notifications::channels::mail`

- [ ] fn `suprnova::register_mail_renderer` · framework/src/notifications/channels/mail.rs:119 (also `suprnova::notifications::channels::mail::register_mail_renderer`, `suprnova::prelude::register_mail_renderer`)
- [ ] struct `suprnova::MailChannel` · framework/src/notifications/channels/mail.rs:154 (also `suprnova::notifications::channels::mail::MailChannel`)
  - Implements: `suprnova::Channel`
  - [ ] fn `suprnova::MailChannel::new` · framework/src/notifications/channels/mail.rs:158
- [ ] struct `suprnova::MailRendering` · framework/src/notifications/channels/mail.rs:64 (also `suprnova::notifications::channels::mail::MailRendering`, `suprnova::prelude::MailRendering`)
  - Public fields: `subject`, `html`, `text`, `from`, `cc`, `bcc`, `reply_to`, `attachments`
- [ ] trait `suprnova::NotificationMailable` · framework/src/notifications/channels/mail.rs:96 (also `suprnova::notifications::channels::mail::NotificationMailable`, `suprnova::prelude::NotificationMailable`)
  - [ ] fn `suprnova::NotificationMailable::to_mail` · framework/src/notifications/channels/mail.rs:99 (required)

### `suprnova::notifications::channels::webpush` (feature: `web-push`)

- [ ] struct `suprnova::WebPushChannel` · framework/src/notifications/channels/webpush.rs:37 (also `suprnova::notifications::channels::webpush::WebPushChannel`)
  - Implements: `suprnova::Channel`
  - [ ] fn `suprnova::WebPushChannel::new` · framework/src/notifications/channels/webpush.rs:45

### `suprnova::notifications::database_read`

- [ ] fn `suprnova::notifications::all_for` · framework/src/notifications/database_read.rs:103 (also `suprnova::notifications::database_read::all_for`)
- [ ] fn `suprnova::notifications::delete_for` · framework/src/notifications/database_read.rs:227 (also `suprnova::notifications::database_read::delete_for`)
- [ ] fn `suprnova::notifications::mark_all_as_read` · framework/src/notifications/database_read.rs:195 (also `suprnova::notifications::database_read::mark_all_as_read`)
- [ ] fn `suprnova::notifications::mark_as_read` · framework/src/notifications/database_read.rs:151 (also `suprnova::notifications::database_read::mark_as_read`)
- [ ] fn `suprnova::notifications::mark_as_unread` · framework/src/notifications/database_read.rs:173 (also `suprnova::notifications::database_read::mark_as_unread`)
- [ ] fn `suprnova::notifications::read_for` · framework/src/notifications/database_read.rs:135 (also `suprnova::notifications::database_read::read_for`)
- [ ] fn `suprnova::notifications::unread_for` · framework/src/notifications/database_read.rs:119 (also `suprnova::notifications::database_read::unread_for`)
- [ ] struct `suprnova::StoredNotification` · framework/src/notifications/database_read.rs:27 (also `suprnova::notifications::StoredNotification`, `suprnova::notifications::database_read::StoredNotification`)
  - Public fields: `id`, `type_name`, `notifiable_type`, `notifiable_id`, `data`, `read_at`, `created_at`, `updated_at`

### `suprnova::notifications::events`

- [ ] struct `suprnova::NotificationFailed` · framework/src/notifications/events.rs:73 (also `suprnova::notifications::NotificationFailed`, `suprnova::notifications::events::NotificationFailed`)
  - Public fields: `notification`, `channel`, `route`, `data`, `error`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::NotificationSending` · framework/src/notifications/events.rs:32 (also `suprnova::notifications::NotificationSending`, `suprnova::notifications::events::NotificationSending`)
  - Public fields: `notification`, `channel`, `route`, `data`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::NotificationSent` · framework/src/notifications/events.rs:51 (also `suprnova::notifications::NotificationSent`, `suprnova::notifications::events::NotificationSent`)
  - Public fields: `notification`, `channel`, `route`, `data`
  - Implements: `suprnova::Event`

### `suprnova::notifications::notify_job`

- [ ] struct `suprnova::SendNotificationJob` · framework/src/notifications/notify_job.rs:21 (also `suprnova::notifications::SendNotificationJob`, `suprnova::notifications::notify_job::SendNotificationJob`)
  - Public fields: `notifiable_route_per_channel`, `notification_name`, `notification_payload`, `channels`
  - Implements: `suprnova::Job`

### `suprnova::notifications::testing`

- [ ] fn `suprnova::notifications::assert_count` · framework/src/notifications/testing.rs:145 (also `suprnova::notifications::testing::assert_count`)
- [ ] fn `suprnova::notifications::assert_nothing_sent` · framework/src/notifications/testing.rs:132 (also `suprnova::notifications::testing::assert_nothing_sent`)
- [ ] fn `suprnova::notifications::assert_nothing_sent_to` · framework/src/notifications/testing.rs:158 (also `suprnova::notifications::testing::assert_nothing_sent_to`)
- [ ] fn `suprnova::notifications::assert_sent` · framework/src/notifications/testing.rs:87 (also `suprnova::notifications::testing::assert_sent`)
- [ ] fn `suprnova::notifications::assert_sent_named` · framework/src/notifications/testing.rs:112 (also `suprnova::notifications::testing::assert_sent_named`)
- [ ] fn `suprnova::notifications::assert_sent_times` · framework/src/notifications/testing.rs:117 (also `suprnova::notifications::testing::assert_sent_times`)
- [ ] fn `suprnova::notifications::assert_sent_to` · framework/src/notifications/testing.rs:98 (also `suprnova::notifications::testing::assert_sent_to`)
- [ ] fn `suprnova::notifications::assert_sent_to_on` · framework/src/notifications/testing.rs:103 (also `suprnova::notifications::testing::assert_sent_to_on`)
- [ ] fn `suprnova::notifications::testing::install_fake` · framework/src/notifications/testing.rs:59
- [ ] fn `suprnova::notifications::recorded_notifications` · framework/src/notifications/testing.rs:79 (also `suprnova::notifications::testing::recorded`)
- [ ] struct `suprnova::notifications::FakeRecord` · framework/src/notifications/testing.rs:21 (also `suprnova::notifications::testing::FakeRecord`)
  - Public fields: `notification`, `channel`, `route`, `data`
- [ ] struct `suprnova::NotifyFakeGuard` · framework/src/notifications/testing.rs:66 (also `suprnova::notifications::NotifyFakeGuard`, `suprnova::notifications::testing::NotifyFakeGuard`)

## pagination

### `suprnova::pagination`

- [ ] struct `suprnova::Pagination` · framework/src/pagination/mod.rs:25 (also `suprnova::pagination::Pagination`)
  - [ ] fn `suprnova::Pagination::length_aware` · framework/src/pagination/mod.rs:44
  - [ ] fn `suprnova::Pagination::length_aware_on` · framework/src/pagination/mod.rs:65
  - [ ] fn `suprnova::Pagination::cursor` · framework/src/pagination/mod.rs:141
  - [ ] fn `suprnova::Pagination::cursor_on` · framework/src/pagination/mod.rs:165
- [ ] trait `suprnova::Paginated` · framework/src/pagination/mod.rs:278 (also `suprnova::pagination::Paginated`)
  - Implemented here by: `CursorPaginator`, `LengthAwarePaginator`
  - [ ] fn `suprnova::Paginated::items` · framework/src/pagination/mod.rs:280 (required)
  - [ ] fn `suprnova::Paginated::meta_value` · framework/src/pagination/mod.rs:284 (required)
  - [ ] fn `suprnova::Paginated::links_iter` · framework/src/pagination/mod.rs:288 (required)

### `suprnova::pagination::cursor`

- [ ] struct `suprnova::CursorPaginator` · framework/src/pagination/cursor.rs:87 (also `suprnova::pagination::CursorPaginator`, `suprnova::pagination::cursor::CursorPaginator`)
  - Public fields: `data`, `per_page`, `next_cursor`, `prev_cursor`, `path`, `cursor_name`
  - Implements: `suprnova::IntoInertiaScroll`, `suprnova::Paginated`, `suprnova::ProvidesScrollMetadata`
  - [ ] fn `suprnova::CursorPaginator::new` · framework/src/pagination/cursor.rs:119
  - [ ] fn `suprnova::CursorPaginator::with_path` · framework/src/pagination/cursor.rs:137
  - [ ] fn `suprnova::CursorPaginator::with_cursor_name` · framework/src/pagination/cursor.rs:145
  - [ ] fn `suprnova::CursorPaginator::on_first_page` · framework/src/pagination/cursor.rs:153
  - [ ] fn `suprnova::CursorPaginator::on_last_page` · framework/src/pagination/cursor.rs:160
  - [ ] fn `suprnova::CursorPaginator::has_more_pages` · framework/src/pagination/cursor.rs:171
  - [ ] fn `suprnova::CursorPaginator::has_pages` · framework/src/pagination/cursor.rs:179
  - [ ] fn `suprnova::CursorPaginator::is_empty` · framework/src/pagination/cursor.rs:185
  - [ ] fn `suprnova::CursorPaginator::is_not_empty` · framework/src/pagination/cursor.rs:191
  - [ ] fn `suprnova::CursorPaginator::count` · framework/src/pagination/cursor.rs:197
  - [ ] fn `suprnova::CursorPaginator::encode_value` · framework/src/pagination/cursor.rs:236
  - [ ] fn `suprnova::CursorPaginator::decode_value` · framework/src/pagination/cursor.rs:288
  - [ ] fn `suprnova::CursorPaginator::encode_cursor` · framework/src/pagination/cursor.rs:323
  - [ ] fn `suprnova::CursorPaginator::try_encode_cursor` · framework/src/pagination/cursor.rs:335
  - [ ] fn `suprnova::CursorPaginator::decode_cursor` · framework/src/pagination/cursor.rs:352
- [ ] enum `suprnova::CursorDirection` · framework/src/pagination/cursor.rs:15 (also `suprnova::pagination::CursorDirection`, `suprnova::pagination::cursor::CursorDirection`)
  - Variants: `Next`, `Prev`

### `suprnova::pagination::inertia`

- [ ] trait `suprnova::IntoInertiaScroll` · framework/src/pagination/inertia.rs:13 (also `suprnova::pagination::IntoInertiaScroll`, `suprnova::pagination::inertia::IntoInertiaScroll`)
  - Implemented here by: `CursorPaginator`, `LengthAwarePaginator`, `Paginator`
  - [ ] fn `suprnova::IntoInertiaScroll::into_inertia_scroll` · framework/src/pagination/inertia.rs:16 (required)

### `suprnova::pagination::length_aware`

- [ ] struct `suprnova::LengthAwarePaginator` · framework/src/pagination/length_aware.rs:45 (also `suprnova::pagination::LengthAwarePaginator`, `suprnova::pagination::length_aware::LengthAwarePaginator`)
  - Public fields: `data`, `current_page`, `last_page`, `per_page`, `total`, `from`, `to`, `path`, `page_name`
  - Implements: `suprnova::IntoInertiaScroll`, `suprnova::Paginated`, `suprnova::ProvidesScrollMetadata`
  - [ ] fn `suprnova::LengthAwarePaginator::new` · framework/src/pagination/length_aware.rs:90
  - [ ] fn `suprnova::LengthAwarePaginator::with_page_name` · framework/src/pagination/length_aware.rs:151
  - [ ] fn `suprnova::LengthAwarePaginator::with_path` · framework/src/pagination/length_aware.rs:158
  - [ ] fn `suprnova::LengthAwarePaginator::with_base_url` · framework/src/pagination/length_aware.rs:168
  - [ ] fn `suprnova::LengthAwarePaginator::url_for_page` · framework/src/pagination/length_aware.rs:182
  - [ ] fn `suprnova::LengthAwarePaginator::has_more_pages` · framework/src/pagination/length_aware.rs:188
  - [ ] fn `suprnova::LengthAwarePaginator::on_first_page` · framework/src/pagination/length_aware.rs:195
  - [ ] fn `suprnova::LengthAwarePaginator::on_last_page` · framework/src/pagination/length_aware.rs:207
  - [ ] fn `suprnova::LengthAwarePaginator::has_pages` · framework/src/pagination/length_aware.rs:215
  - [ ] fn `suprnova::LengthAwarePaginator::is_empty` · framework/src/pagination/length_aware.rs:221
  - [ ] fn `suprnova::LengthAwarePaginator::is_not_empty` · framework/src/pagination/length_aware.rs:227
  - [ ] fn `suprnova::LengthAwarePaginator::count` · framework/src/pagination/length_aware.rs:235

### `suprnova::pagination::simple`

- [ ] struct `suprnova::Paginator` · framework/src/pagination/simple.rs:42 (also `suprnova::pagination::Paginator`, `suprnova::pagination::simple::Paginator`)
  - Public fields: `data`, `current_page`, `per_page`, `has_more`, `path`
  - Implements: `suprnova::IntoInertiaScroll`, `suprnova::ProvidesScrollMetadata`
  - [ ] fn `suprnova::Paginator::new` · framework/src/pagination/simple.rs:61
  - [ ] fn `suprnova::Paginator::with_path` · framework/src/pagination/simple.rs:73
  - [ ] fn `suprnova::Paginator::on_first_page` · framework/src/pagination/simple.rs:80
  - [ ] fn `suprnova::Paginator::on_last_page` · framework/src/pagination/simple.rs:86
  - [ ] fn `suprnova::Paginator::has_more_pages` · framework/src/pagination/simple.rs:93
  - [ ] fn `suprnova::Paginator::has_pages` · framework/src/pagination/simple.rs:100
  - [ ] fn `suprnova::Paginator::is_empty` · framework/src/pagination/simple.rs:106
  - [ ] fn `suprnova::Paginator::is_not_empty` · framework/src/pagination/simple.rs:112
  - [ ] fn `suprnova::Paginator::count` · framework/src/pagination/simple.rs:118

## payments

### `suprnova::payments::dto::country`

- [ ] struct `suprnova::payments::CountryCode` · framework/src/payments/dto/country.rs:16 (also `suprnova::payments::country::CountryCode`, `suprnova::payments::dto::CountryCode`, `suprnova::payments::dto::country::CountryCode`)
  - [ ] fn `suprnova::payments::CountryCode::new` · framework/src/payments/dto/country.rs:27
  - [ ] fn `suprnova::payments::CountryCode::as_str` · framework/src/payments/dto/country.rs:38

### `suprnova::payments::dto::customer`

- [ ] struct `suprnova::payments::CreateCustomerRequest` · framework/src/payments/dto/customer.rs:35 (also `suprnova::payments::customer::CreateCustomerRequest`, `suprnova::payments::dto::CreateCustomerRequest`, `suprnova::payments::dto::customer::CreateCustomerRequest`)
  - Public fields: `user_id`, `email`, `name`, `metadata`
- [ ] struct `suprnova::payments::CustomerRef` · framework/src/payments/dto/customer.rs:20 (also `suprnova::payments::customer::CustomerRef`, `suprnova::payments::dto::CustomerRef`, `suprnova::payments::dto::customer::CustomerRef`)
  - Public fields: `provider_customer_id`, `user_id`, `email`, `provider_metadata`
- [ ] struct `suprnova::payments::UpdateCustomerRequest` · framework/src/payments/dto/customer.rs:49 (also `suprnova::payments::customer::UpdateCustomerRequest`, `suprnova::payments::dto::UpdateCustomerRequest`, `suprnova::payments::dto::customer::UpdateCustomerRequest`)
  - Public fields: `provider_customer_id`, `email`, `name`, `metadata`

### `suprnova::payments::dto::payment`

- [ ] struct `suprnova::payments::ChargeRequest` · framework/src/payments/dto/payment.rs:40 (also `suprnova::payments::dto::ChargeRequest`, `suprnova::payments::dto::payment::ChargeRequest`, `suprnova::payments::payment::ChargeRequest`)
  - Public fields: `customer_ref`, `payment_method_ref`, `amount`, `description`, `idempotency_key`, `metadata`
- [ ] struct `suprnova::payments::RefundRequest` · framework/src/payments/dto/payment.rs:105 (also `suprnova::payments::dto::RefundRequest`, `suprnova::payments::dto::payment::RefundRequest`, `suprnova::payments::payment::RefundRequest`)
  - Public fields: `provider_transaction_id`, `amount`, `reason`, `idempotency_key`
- [ ] struct `suprnova::payments::RefundResult` · framework/src/payments/dto/payment.rs:120 (also `suprnova::payments::dto::RefundResult`, `suprnova::payments::dto::payment::RefundResult`, `suprnova::payments::payment::RefundResult`)
  - Public fields: `provider_refund_id`, `provider_transaction_id`, `amount`, `provider_metadata`
- [ ] enum `suprnova::payments::ChargeResult` · framework/src/payments/dto/payment.rs:62 (also `suprnova::payments::dto::ChargeResult`, `suprnova::payments::dto::payment::ChargeResult`, `suprnova::payments::payment::ChargeResult`)
  - Variants: `Completed`, `RedirectRequired`, `RequiresClientAction`
- [ ] enum `suprnova::payments::PaymentStatus` · framework/src/payments/dto/payment.rs:11 (also `suprnova::payments::dto::PaymentStatus`, `suprnova::payments::dto::payment::PaymentStatus`, `suprnova::payments::payment::PaymentStatus`)
  - Variants: `Created`, `RequiresAction`, `Pending`, `Processing`, `Authorized`, `Expired`, `Succeeded`, `Failed`, `Canceled`, `Refunded`, `PartiallyRefunded`, `Disputed`

### `suprnova::payments::dto::payment_method`

- [ ] enum `suprnova::payments::MobileMoneyOperator` · framework/src/payments/dto/payment_method.rs:80 (also `suprnova::payments::dto::MobileMoneyOperator`, `suprnova::payments::dto::payment_method::MobileMoneyOperator`, `suprnova::payments::payment_method::MobileMoneyOperator`)
  - Variants: `MtnMomo`, `Mpesa`, `AirtelMoney`, `OrangeMoney`, `Lipila`, `Custom`
- [ ] enum `suprnova::payments::PaymentMethod` · framework/src/payments/dto/payment_method.rs:10 (also `suprnova::payments::dto::PaymentMethod`, `suprnova::payments::dto::payment_method::PaymentMethod`, `suprnova::payments::payment_method::PaymentMethod`)
  - Variants: `Card`, `BankTransfer`, `EWallet`, `MobileMoney`, `Stablecoin`, `Crypto`, `Custom`
- [ ] enum `suprnova::payments::StablecoinAsset` · framework/src/payments/dto/payment_method.rs:103 (also `suprnova::payments::dto::StablecoinAsset`, `suprnova::payments::dto::payment_method::StablecoinAsset`, `suprnova::payments::payment_method::StablecoinAsset`)
  - Variants: `Usdc`, `Usdt`, `Dai`, `Custom`

### `suprnova::payments::dto::phone`

- [ ] struct `suprnova::payments::PhoneNumber` · framework/src/payments/dto/phone.rs:23 (also `suprnova::payments::dto::PhoneNumber`, `suprnova::payments::dto::phone::PhoneNumber`, `suprnova::payments::phone::PhoneNumber`)
  - [ ] fn `suprnova::payments::PhoneNumber::new` · framework/src/payments/dto/phone.rs:33
  - [ ] fn `suprnova::payments::PhoneNumber::as_e164` · framework/src/payments/dto/phone.rs:45
  - [ ] fn `suprnova::payments::PhoneNumber::digits` · framework/src/payments/dto/phone.rs:52

### `suprnova::payments::dto::session`

- [ ] struct `suprnova::payments::StartSessionRequest` · framework/src/payments/dto/session.rs:21 (also `suprnova::payments::dto::StartSessionRequest`, `suprnova::payments::dto::session::StartSessionRequest`, `suprnova::payments::session::StartSessionRequest`)
  - Public fields: `mode`, `customer_ref`, `price_refs`, `success_return_url`, `cancel_return_url`, `amount_hint`, `idempotency_key`, `metadata`
- [ ] enum `suprnova::payments::CheckoutSessionState` · framework/src/payments/dto/session.rs:48 (also `suprnova::payments::dto::CheckoutSessionState`, `suprnova::payments::dto::session::CheckoutSessionState`, `suprnova::payments::session::CheckoutSessionState`)
  - Variants: `Open`, `Complete`, `Expired`
- [ ] enum `suprnova::payments::SessionMode` · framework/src/payments/dto/session.rs:12 (also `suprnova::payments::dto::SessionMode`, `suprnova::payments::dto::session::SessionMode`, `suprnova::payments::session::SessionMode`)
  - Variants: `OneOff`, `Subscription`
- [ ] enum `suprnova::payments::SessionPayload` · framework/src/payments/dto/session.rs:73 (also `suprnova::payments::dto::SessionPayload`, `suprnova::payments::dto::session::SessionPayload`, `suprnova::payments::session::SessionPayload`)
  - Variants: `StripeElements`, `StripeCheckoutRedirect`, `PaddleInline`, `MobileMoneyPrompt`, `Redirect`

### `suprnova::payments::dto::subscription`

- [ ] struct `suprnova::payments::SubscribeRequest` · framework/src/payments/dto/subscription.rs:29 (also `suprnova::payments::dto::SubscribeRequest`, `suprnova::payments::dto::subscription::SubscribeRequest`, `suprnova::payments::subscription::SubscribeRequest`)
  - Public fields: `customer_ref`, `price_refs`, `trial_days`, `idempotency_key`, `metadata`
- [ ] struct `suprnova::payments::SubscriptionItemSnapshot` · framework/src/payments/dto/subscription.rs:82 (also `suprnova::payments::dto::SubscriptionItemSnapshot`, `suprnova::payments::dto::subscription::SubscriptionItemSnapshot`, `suprnova::payments::subscription::SubscriptionItemSnapshot`)
  - Public fields: `provider_item_id`, `provider_price_id`, `quantity`, `unit_amount`
- [ ] struct `suprnova::payments::SubscriptionResult` · framework/src/payments/dto/subscription.rs:60 (also `suprnova::payments::dto::SubscriptionResult`, `suprnova::payments::dto::subscription::SubscriptionResult`, `suprnova::payments::subscription::SubscriptionResult`)
  - Public fields: `provider_subscription_id`, `provider_customer_id`, `status`, `items`, `current_period_start`, `current_period_end`, `cancel_at_period_end`, `provider_metadata`
- [ ] struct `suprnova::payments::UpdateSubscriptionRequest` · framework/src/payments/dto/subscription.rs:45 (also `suprnova::payments::dto::UpdateSubscriptionRequest`, `suprnova::payments::dto::subscription::UpdateSubscriptionRequest`, `suprnova::payments::subscription::UpdateSubscriptionRequest`)
  - Public fields: `provider_subscription_id`, `new_price_refs`, `cancel_at_period_end`, `idempotency_key`
- [ ] enum `suprnova::payments::SubscriptionStatus` · framework/src/payments/dto/subscription.rs:12 (also `suprnova::payments::dto::SubscriptionStatus`, `suprnova::payments::dto::subscription::SubscriptionStatus`, `suprnova::payments::subscription::SubscriptionStatus`)
  - Variants: `Trialing`, `Active`, `PastDue`, `Canceled`, `Incomplete`, `Paused`

### `suprnova::payments::dto::webhook`

- [ ] struct `suprnova::payments::WebhookContext` · framework/src/payments/dto/webhook.rs:59 (also `suprnova::payments::dto::WebhookContext`, `suprnova::payments::dto::webhook::WebhookContext`, `suprnova::payments::webhook::WebhookContext`)
  - Public fields: `body`, `headers`, `remote_addr`
- [ ] struct `suprnova::payments::WebhookEvent` · framework/src/payments/dto/webhook.rs:39 (also `suprnova::payments::dto::WebhookEvent`, `suprnova::payments::dto::webhook::WebhookEvent`, `suprnova::payments::webhook::WebhookEvent`)
  - Public fields: `provider`, `provider_event_id`, `provider_event_type`, `neutral`, `raw_payload`
- [ ] enum `suprnova::payments::NeutralEventKind` · framework/src/payments/dto/webhook.rs:11 (also `suprnova::payments::dto::NeutralEventKind`, `suprnova::payments::dto::webhook::NeutralEventKind`, `suprnova::payments::webhook::NeutralEventKind`)
  - Variants: `PaymentSucceeded`, `PaymentFailed`, `PaymentRefunded`, `PaymentDisputed`, `SubscriptionCreated`, `SubscriptionUpdated`, `SubscriptionCanceled`, `InvoicePaid`, `InvoiceFailed`, `CustomerCreated`, `CustomerUpdated`

### `suprnova::payments::entities::customer`

- [ ] struct `suprnova::payments::entities::customer::Customer` · framework/src/payments/entities/customer.rs:15
  - Public fields: `id`, `provider`, `provider_customer_id`, `user_id`, `email`, `provider_metadata`, `created_at`, `updated_at`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`, `suprnova::Touchable`
  - [ ] fn `suprnova::payments::entities::customer::Customer::fill` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::without_global_scope` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::without_global_scopes` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::on` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::on_write_connection` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::count` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::sum` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::avg` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::min` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::max` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::pluck` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::pluck_keyed` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::filter` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::db_where` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::where_in` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::where_like` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::latest` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::oldest` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::pivot` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::with` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::with_count` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::with_sum` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::with_avg` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::with_min` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::with_max` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Customer::observe` · framework/src/payments/entities/customer.rs:14

### `suprnova::payments::entities::customer::customer`

- [ ] struct `suprnova::payments::entities::customer::ActiveModel` · framework/src/payments/entities/customer.rs:14 (also `suprnova::payments::entities::customer::customer::ActiveModel`)
  - Public fields: `id`, `provider`, `provider_customer_id`, `user_id`, `email`, `provider_metadata`, `created_at`, `updated_at`
- [ ] struct `suprnova::payments::entities::customer::customer::ColumnIter` · framework/src/payments/entities/customer.rs:14
- [ ] struct `suprnova::payments::entities::customer::Entity` · framework/src/payments/entities/customer.rs:14 (also `suprnova::payments::entities::customer::customer::Entity`)
- [ ] struct `suprnova::payments::entities::customer::Model` · framework/src/payments/entities/customer.rs:14 (also `suprnova::payments::entities::customer::customer::Model`)
  - Public fields: `id`, `provider`, `provider_customer_id`, `user_id`, `email`, `provider_metadata`, `created_at`, `updated_at`
  - [ ] fn `suprnova::payments::entities::customer::Model::into_ex` · framework/src/payments/entities/customer.rs:14
- [ ] struct `suprnova::payments::entities::customer::customer::PrimaryKeyIter` · framework/src/payments/entities/customer.rs:14
- [ ] struct `suprnova::payments::entities::customer::customer::RelationIter` · framework/src/payments/entities/customer.rs:14
- [ ] enum `suprnova::payments::entities::customer::Column` · framework/src/payments/entities/customer.rs:14 (also `suprnova::payments::entities::customer::customer::Column`)
  - Variants: `Id`, `Provider`, `ProviderCustomerId`, `UserId`, `Email`, `ProviderMetadata`, `CreatedAt`, `UpdatedAt`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::payments::entities::customer::Column::as_str` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Column::from_name` · framework/src/payments/entities/customer.rs:14
  - [ ] fn `suprnova::payments::entities::customer::Column::iter` · framework/src/payments/entities/customer.rs:14
- [ ] enum `suprnova::payments::entities::customer::customer::PrimaryKey` · framework/src/payments/entities/customer.rs:14
  - Variants: `Id`
- [ ] enum `suprnova::payments::entities::customer::customer::Relation` · framework/src/payments/entities/customer.rs:14
- [ ] type `suprnova::payments::entities::customer::customer::__Suprnova_Cast_Storage_created_at` · framework/src/payments/entities/customer.rs:14
- [ ] type `suprnova::payments::entities::customer::customer::__Suprnova_Cast_Storage_updated_at` · framework/src/payments/entities/customer.rs:14

### `suprnova::payments::entities::customer::customer::events`

- [ ] struct `suprnova::payments::entities::customer::customer::events::Created` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Creating` · framework/src/payments/entities/customer.rs:14
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Deleted` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Deleting` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::ForceDeleted` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::ForceDeleting` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Replicating` · framework/src/payments/entities/customer.rs:14
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Restored` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Restoring` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Retrieved` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Retrieving` · framework/src/payments/entities/customer.rs:14
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Saved` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Saving` · framework/src/payments/entities/customer.rs:14
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Trashed` · framework/src/payments/entities/customer.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Updated` · framework/src/payments/entities/customer.rs:14
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::customer::customer::events::Updating` · framework/src/payments/entities/customer.rs:14
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::payments::entities::payment_method`

- [ ] struct `suprnova::payments::entities::payment_method::PaymentMethod` · framework/src/payments/entities/payment_method.rs:16
  - Public fields: `id`, `provider`, `provider_payment_method_id`, `provider_customer_id`, `method_type`, `method_details`, `is_default`, `provider_metadata`, `created_at`, `updated_at`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`, `suprnova::Touchable`
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::fill` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::without_global_scope` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::without_global_scopes` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::on` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::on_write_connection` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::count` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::sum` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::avg` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::min` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::max` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::pluck` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::pluck_keyed` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::filter` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::db_where` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::where_in` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::where_like` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::latest` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::oldest` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::pivot` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::with` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::with_count` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::with_sum` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::with_avg` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::with_min` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::with_max` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::PaymentMethod::observe` · framework/src/payments/entities/payment_method.rs:15

### `suprnova::payments::entities::payment_method::payment_method`

- [ ] struct `suprnova::payments::entities::payment_method::ActiveModel` · framework/src/payments/entities/payment_method.rs:15 (also `suprnova::payments::entities::payment_method::payment_method::ActiveModel`)
  - Public fields: `id`, `provider`, `provider_payment_method_id`, `provider_customer_id`, `method_type`, `method_details`, `is_default`, `provider_metadata`, `created_at`, `updated_at`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::ColumnIter` · framework/src/payments/entities/payment_method.rs:15
- [ ] struct `suprnova::payments::entities::payment_method::Entity` · framework/src/payments/entities/payment_method.rs:15 (also `suprnova::payments::entities::payment_method::payment_method::Entity`)
- [ ] struct `suprnova::payments::entities::payment_method::Model` · framework/src/payments/entities/payment_method.rs:15 (also `suprnova::payments::entities::payment_method::payment_method::Model`)
  - Public fields: `id`, `provider`, `provider_payment_method_id`, `provider_customer_id`, `method_type`, `method_details`, `is_default`, `provider_metadata`, `created_at`, `updated_at`
  - [ ] fn `suprnova::payments::entities::payment_method::Model::into_ex` · framework/src/payments/entities/payment_method.rs:15
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::PrimaryKeyIter` · framework/src/payments/entities/payment_method.rs:15
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::RelationIter` · framework/src/payments/entities/payment_method.rs:15
- [ ] enum `suprnova::payments::entities::payment_method::Column` · framework/src/payments/entities/payment_method.rs:15 (also `suprnova::payments::entities::payment_method::payment_method::Column`)
  - Variants: `Id`, `Provider`, `ProviderPaymentMethodId`, `ProviderCustomerId`, `MethodType`, `MethodDetails`, `IsDefault`, `ProviderMetadata`, `CreatedAt`, `UpdatedAt`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::payments::entities::payment_method::Column::as_str` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::Column::from_name` · framework/src/payments/entities/payment_method.rs:15
  - [ ] fn `suprnova::payments::entities::payment_method::Column::iter` · framework/src/payments/entities/payment_method.rs:15
- [ ] enum `suprnova::payments::entities::payment_method::payment_method::PrimaryKey` · framework/src/payments/entities/payment_method.rs:15
  - Variants: `Id`
- [ ] enum `suprnova::payments::entities::payment_method::payment_method::Relation` · framework/src/payments/entities/payment_method.rs:15
- [ ] type `suprnova::payments::entities::payment_method::payment_method::__Suprnova_Cast_Storage_created_at` · framework/src/payments/entities/payment_method.rs:15
- [ ] type `suprnova::payments::entities::payment_method::payment_method::__Suprnova_Cast_Storage_updated_at` · framework/src/payments/entities/payment_method.rs:15

### `suprnova::payments::entities::payment_method::payment_method::events`

- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Created` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Creating` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Deleted` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Deleting` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::ForceDeleted` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::ForceDeleting` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Replicating` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Restored` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Restoring` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Retrieved` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Retrieving` · framework/src/payments/entities/payment_method.rs:15
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Saved` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Saving` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Trashed` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Updated` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::payment_method::payment_method::events::Updating` · framework/src/payments/entities/payment_method.rs:15
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::payments::entities::subscription`

- [ ] struct `suprnova::payments::entities::subscription::Subscription` · framework/src/payments/entities/subscription.rs:21
  - Public fields: `id`, `provider`, `provider_subscription_id`, `provider_customer_id`, `status`, `current_period_start`, `current_period_end`, `cancel_at_period_end`, `canceled_at`, `provider_metadata`, `created_at`, `updated_at`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`, `suprnova::Touchable`
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::fill` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::without_global_scope` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::without_global_scopes` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::on` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::on_write_connection` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::count` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::sum` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::avg` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::min` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::max` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::pluck` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::pluck_keyed` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::filter` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::db_where` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::where_in` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::where_like` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::latest` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::oldest` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::pivot` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::with` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::with_count` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::with_sum` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::with_avg` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::with_min` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::with_max` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::items` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::items_loaded` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::items_count` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::items_sum_of` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::items_avg_of` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::items_min_of` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::items_max_of` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::with_where_items` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Subscription::observe` · framework/src/payments/entities/subscription.rs:14

### `suprnova::payments::entities::subscription::subscription`

- [ ] struct `suprnova::payments::entities::subscription::ActiveModel` · framework/src/payments/entities/subscription.rs:14 (also `suprnova::payments::entities::subscription::subscription::ActiveModel`)
  - Public fields: `id`, `provider`, `provider_subscription_id`, `provider_customer_id`, `status`, `current_period_start`, `current_period_end`, `cancel_at_period_end`, `canceled_at`, `provider_metadata`, `created_at`, `updated_at`
- [ ] struct `suprnova::payments::entities::subscription::subscription::ColumnIter` · framework/src/payments/entities/subscription.rs:14
- [ ] struct `suprnova::payments::entities::subscription::Entity` · framework/src/payments/entities/subscription.rs:14 (also `suprnova::payments::entities::subscription::subscription::Entity`)
- [ ] struct `suprnova::payments::entities::subscription::Model` · framework/src/payments/entities/subscription.rs:14 (also `suprnova::payments::entities::subscription::subscription::Model`)
  - Public fields: `id`, `provider`, `provider_subscription_id`, `provider_customer_id`, `status`, `current_period_start`, `current_period_end`, `cancel_at_period_end`, `canceled_at`, `provider_metadata`, `created_at`, `updated_at`
  - [ ] fn `suprnova::payments::entities::subscription::Model::into_ex` · framework/src/payments/entities/subscription.rs:14
- [ ] struct `suprnova::payments::entities::subscription::subscription::PrimaryKeyIter` · framework/src/payments/entities/subscription.rs:14
- [ ] struct `suprnova::payments::entities::subscription::subscription::RelationIter` · framework/src/payments/entities/subscription.rs:14
- [ ] enum `suprnova::payments::entities::subscription::Column` · framework/src/payments/entities/subscription.rs:14 (also `suprnova::payments::entities::subscription::subscription::Column`)
  - Variants: `Id`, `Provider`, `ProviderSubscriptionId`, `ProviderCustomerId`, `Status`, `CurrentPeriodStart`, `CurrentPeriodEnd`, `CancelAtPeriodEnd`, `CanceledAt`, `ProviderMetadata`, `CreatedAt`, `UpdatedAt`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::payments::entities::subscription::Column::as_str` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Column::from_name` · framework/src/payments/entities/subscription.rs:14
  - [ ] fn `suprnova::payments::entities::subscription::Column::iter` · framework/src/payments/entities/subscription.rs:14
- [ ] enum `suprnova::payments::entities::subscription::subscription::PrimaryKey` · framework/src/payments/entities/subscription.rs:14
  - Variants: `Id`
- [ ] enum `suprnova::payments::entities::subscription::subscription::Relation` · framework/src/payments/entities/subscription.rs:14
- [ ] type `suprnova::payments::entities::subscription::subscription::__Suprnova_Cast_Storage_canceled_at` · framework/src/payments/entities/subscription.rs:14
- [ ] type `suprnova::payments::entities::subscription::subscription::__Suprnova_Cast_Storage_created_at` · framework/src/payments/entities/subscription.rs:14
- [ ] type `suprnova::payments::entities::subscription::subscription::__Suprnova_Cast_Storage_current_period_end` · framework/src/payments/entities/subscription.rs:14
- [ ] type `suprnova::payments::entities::subscription::subscription::__Suprnova_Cast_Storage_current_period_start` · framework/src/payments/entities/subscription.rs:14
- [ ] type `suprnova::payments::entities::subscription::subscription::__Suprnova_Cast_Storage_updated_at` · framework/src/payments/entities/subscription.rs:14

### `suprnova::payments::entities::subscription::subscription::events`

- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Created` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Creating` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Deleted` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Deleting` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::ForceDeleted` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::ForceDeleting` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Replicating` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Restored` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Restoring` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Retrieved` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Retrieving` · framework/src/payments/entities/subscription.rs:14
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Saved` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Saving` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Trashed` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Updated` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription::subscription::events::Updating` · framework/src/payments/entities/subscription.rs:14
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::payments::entities::subscription_item`

- [ ] struct `suprnova::payments::entities::subscription_item::SubscriptionItem` · framework/src/payments/entities/subscription_item.rs:19
  - Public fields: `id`, `subscription_id`, `provider_item_id`, `provider_price_id`, `quantity`, `unit_amount_minor`, `unit_currency`, `provider_metadata`, `created_at`, `updated_at`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`, `suprnova::Touchable`
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::fill` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::without_global_scope` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::without_global_scopes` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::on` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::on_write_connection` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::count` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::sum` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::avg` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::min` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::max` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::pluck` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::pluck_keyed` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::filter` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::db_where` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::where_in` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::where_like` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::latest` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::oldest` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::pivot` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::with` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::with_count` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::with_sum` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::with_avg` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::with_min` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::with_max` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::subscription` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::subscription_loaded` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::subscription_count` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::subscription_sum_of` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::subscription_avg_of` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::subscription_min_of` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::subscription_max_of` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::with_where_subscription` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::SubscriptionItem::observe` · framework/src/payments/entities/subscription_item.rs:12

### `suprnova::payments::entities::subscription_item::subscription_item`

- [ ] struct `suprnova::payments::entities::subscription_item::ActiveModel` · framework/src/payments/entities/subscription_item.rs:12 (also `suprnova::payments::entities::subscription_item::subscription_item::ActiveModel`)
  - Public fields: `id`, `subscription_id`, `provider_item_id`, `provider_price_id`, `quantity`, `unit_amount_minor`, `unit_currency`, `provider_metadata`, `created_at`, `updated_at`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::ColumnIter` · framework/src/payments/entities/subscription_item.rs:12
- [ ] struct `suprnova::payments::entities::subscription_item::Entity` · framework/src/payments/entities/subscription_item.rs:12 (also `suprnova::payments::entities::subscription_item::subscription_item::Entity`)
- [ ] struct `suprnova::payments::entities::subscription_item::Model` · framework/src/payments/entities/subscription_item.rs:12 (also `suprnova::payments::entities::subscription_item::subscription_item::Model`)
  - Public fields: `id`, `subscription_id`, `provider_item_id`, `provider_price_id`, `quantity`, `unit_amount_minor`, `unit_currency`, `provider_metadata`, `created_at`, `updated_at`
  - [ ] fn `suprnova::payments::entities::subscription_item::Model::into_ex` · framework/src/payments/entities/subscription_item.rs:12
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::PrimaryKeyIter` · framework/src/payments/entities/subscription_item.rs:12
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::RelationIter` · framework/src/payments/entities/subscription_item.rs:12
- [ ] enum `suprnova::payments::entities::subscription_item::Column` · framework/src/payments/entities/subscription_item.rs:12 (also `suprnova::payments::entities::subscription_item::subscription_item::Column`)
  - Variants: `Id`, `SubscriptionId`, `ProviderItemId`, `ProviderPriceId`, `Quantity`, `UnitAmountMinor`, `UnitCurrency`, `ProviderMetadata`, `CreatedAt`, `UpdatedAt`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::payments::entities::subscription_item::Column::as_str` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::Column::from_name` · framework/src/payments/entities/subscription_item.rs:12
  - [ ] fn `suprnova::payments::entities::subscription_item::Column::iter` · framework/src/payments/entities/subscription_item.rs:12
- [ ] enum `suprnova::payments::entities::subscription_item::subscription_item::PrimaryKey` · framework/src/payments/entities/subscription_item.rs:12
  - Variants: `Id`
- [ ] enum `suprnova::payments::entities::subscription_item::subscription_item::Relation` · framework/src/payments/entities/subscription_item.rs:12
- [ ] type `suprnova::payments::entities::subscription_item::subscription_item::__Suprnova_Cast_Storage_created_at` · framework/src/payments/entities/subscription_item.rs:12
- [ ] type `suprnova::payments::entities::subscription_item::subscription_item::__Suprnova_Cast_Storage_updated_at` · framework/src/payments/entities/subscription_item.rs:12

### `suprnova::payments::entities::subscription_item::subscription_item::events`

- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Created` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Creating` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Deleted` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Deleting` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::ForceDeleted` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::ForceDeleting` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Replicating` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Restored` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Restoring` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Retrieved` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Retrieving` · framework/src/payments/entities/subscription_item.rs:12
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Saved` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Saving` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Trashed` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Updated` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::subscription_item::subscription_item::events::Updating` · framework/src/payments/entities/subscription_item.rs:12
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::payments::entities::transaction`

- [ ] struct `suprnova::payments::entities::transaction::Transaction` · framework/src/payments/entities/transaction.rs:17
  - Public fields: `id`, `provider`, `provider_transaction_id`, `provider_customer_id`, `provider_subscription_id`, `amount_total_minor`, `amount_tax_minor`, `currency`, `status`, `provider_metadata`, `paid_at`, `created_at`, `updated_at`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`, `suprnova::Touchable`
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::fill` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::without_global_scope` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::without_global_scopes` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::on` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::on_write_connection` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::count` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::sum` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::avg` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::min` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::max` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::pluck` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::pluck_keyed` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::filter` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::db_where` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::where_in` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::where_like` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::latest` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::oldest` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::pivot` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::with` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::with_count` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::with_sum` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::with_avg` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::with_min` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::with_max` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Transaction::observe` · framework/src/payments/entities/transaction.rs:16

### `suprnova::payments::entities::transaction::transaction`

- [ ] struct `suprnova::payments::entities::transaction::ActiveModel` · framework/src/payments/entities/transaction.rs:16 (also `suprnova::payments::entities::transaction::transaction::ActiveModel`)
  - Public fields: `id`, `provider`, `provider_transaction_id`, `provider_customer_id`, `provider_subscription_id`, `amount_total_minor`, `amount_tax_minor`, `currency`, `status`, `provider_metadata`, `paid_at`, `created_at`, `updated_at`
- [ ] struct `suprnova::payments::entities::transaction::transaction::ColumnIter` · framework/src/payments/entities/transaction.rs:16
- [ ] struct `suprnova::payments::entities::transaction::Entity` · framework/src/payments/entities/transaction.rs:16 (also `suprnova::payments::entities::transaction::transaction::Entity`)
- [ ] struct `suprnova::payments::entities::transaction::Model` · framework/src/payments/entities/transaction.rs:16 (also `suprnova::payments::entities::transaction::transaction::Model`)
  - Public fields: `id`, `provider`, `provider_transaction_id`, `provider_customer_id`, `provider_subscription_id`, `amount_total_minor`, `amount_tax_minor`, `currency`, `status`, `provider_metadata`, `paid_at`, `created_at`, `updated_at`
  - [ ] fn `suprnova::payments::entities::transaction::Model::into_ex` · framework/src/payments/entities/transaction.rs:16
- [ ] struct `suprnova::payments::entities::transaction::transaction::PrimaryKeyIter` · framework/src/payments/entities/transaction.rs:16
- [ ] struct `suprnova::payments::entities::transaction::transaction::RelationIter` · framework/src/payments/entities/transaction.rs:16
- [ ] enum `suprnova::payments::entities::transaction::Column` · framework/src/payments/entities/transaction.rs:16 (also `suprnova::payments::entities::transaction::transaction::Column`)
  - Variants: `Id`, `Provider`, `ProviderTransactionId`, `ProviderCustomerId`, `ProviderSubscriptionId`, `AmountTotalMinor`, `AmountTaxMinor`, `Currency`, `Status`, `ProviderMetadata`, `PaidAt`, `CreatedAt`, `UpdatedAt`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::payments::entities::transaction::Column::as_str` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Column::from_name` · framework/src/payments/entities/transaction.rs:16
  - [ ] fn `suprnova::payments::entities::transaction::Column::iter` · framework/src/payments/entities/transaction.rs:16
- [ ] enum `suprnova::payments::entities::transaction::transaction::PrimaryKey` · framework/src/payments/entities/transaction.rs:16
  - Variants: `Id`
- [ ] enum `suprnova::payments::entities::transaction::transaction::Relation` · framework/src/payments/entities/transaction.rs:16
- [ ] type `suprnova::payments::entities::transaction::transaction::__Suprnova_Cast_Storage_created_at` · framework/src/payments/entities/transaction.rs:16
- [ ] type `suprnova::payments::entities::transaction::transaction::__Suprnova_Cast_Storage_paid_at` · framework/src/payments/entities/transaction.rs:16
- [ ] type `suprnova::payments::entities::transaction::transaction::__Suprnova_Cast_Storage_updated_at` · framework/src/payments/entities/transaction.rs:16

### `suprnova::payments::entities::transaction::transaction::events`

- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Created` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Creating` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Deleted` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Deleting` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::ForceDeleted` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::ForceDeleting` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Replicating` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Restored` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Restoring` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Retrieved` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Retrieving` · framework/src/payments/entities/transaction.rs:16
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Saved` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Saving` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Trashed` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Updated` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::transaction::transaction::events::Updating` · framework/src/payments/entities/transaction.rs:16
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::payments::entities::webhook_event`

- [ ] struct `suprnova::payments::entities::webhook_event::WebhookEvent` · framework/src/payments/entities/webhook_event.rs:22
  - Public fields: `id`, `provider`, `provider_event_id`, `provider_event_type`, `neutral_event_kind`, `payload`, `received_at`, `processed_at`, `process_error`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::fill` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::without_global_scope` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::without_global_scopes` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::on` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::on_write_connection` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::count` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::sum` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::avg` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::min` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::max` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::pluck` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::pluck_keyed` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::filter` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::db_where` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::where_in` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::where_like` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::latest` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::oldest` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::pivot` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::with` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::with_count` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::with_sum` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::with_avg` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::with_min` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::with_max` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::WebhookEvent::observe` · framework/src/payments/entities/webhook_event.rs:21

### `suprnova::payments::entities::webhook_event::webhook_event`

- [ ] struct `suprnova::payments::entities::webhook_event::ActiveModel` · framework/src/payments/entities/webhook_event.rs:21 (also `suprnova::payments::entities::webhook_event::webhook_event::ActiveModel`)
  - Public fields: `id`, `provider`, `provider_event_id`, `provider_event_type`, `neutral_event_kind`, `payload`, `received_at`, `processed_at`, `process_error`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::ColumnIter` · framework/src/payments/entities/webhook_event.rs:21
- [ ] struct `suprnova::payments::entities::webhook_event::Entity` · framework/src/payments/entities/webhook_event.rs:21 (also `suprnova::payments::entities::webhook_event::webhook_event::Entity`)
- [ ] struct `suprnova::payments::entities::webhook_event::Model` · framework/src/payments/entities/webhook_event.rs:21 (also `suprnova::payments::entities::webhook_event::webhook_event::Model`)
  - Public fields: `id`, `provider`, `provider_event_id`, `provider_event_type`, `neutral_event_kind`, `payload`, `received_at`, `processed_at`, `process_error`
  - [ ] fn `suprnova::payments::entities::webhook_event::Model::into_ex` · framework/src/payments/entities/webhook_event.rs:21
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::PrimaryKeyIter` · framework/src/payments/entities/webhook_event.rs:21
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::RelationIter` · framework/src/payments/entities/webhook_event.rs:21
- [ ] enum `suprnova::payments::entities::webhook_event::Column` · framework/src/payments/entities/webhook_event.rs:21 (also `suprnova::payments::entities::webhook_event::webhook_event::Column`)
  - Variants: `Id`, `Provider`, `ProviderEventId`, `ProviderEventType`, `NeutralEventKind`, `Payload`, `ReceivedAt`, `ProcessedAt`, `ProcessError`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::payments::entities::webhook_event::Column::as_str` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::Column::from_name` · framework/src/payments/entities/webhook_event.rs:21
  - [ ] fn `suprnova::payments::entities::webhook_event::Column::iter` · framework/src/payments/entities/webhook_event.rs:21
- [ ] enum `suprnova::payments::entities::webhook_event::webhook_event::PrimaryKey` · framework/src/payments/entities/webhook_event.rs:21
  - Variants: `Id`
- [ ] enum `suprnova::payments::entities::webhook_event::webhook_event::Relation` · framework/src/payments/entities/webhook_event.rs:21
- [ ] type `suprnova::payments::entities::webhook_event::webhook_event::__Suprnova_Cast_Storage_processed_at` · framework/src/payments/entities/webhook_event.rs:21
- [ ] type `suprnova::payments::entities::webhook_event::webhook_event::__Suprnova_Cast_Storage_received_at` · framework/src/payments/entities/webhook_event.rs:21

### `suprnova::payments::entities::webhook_event::webhook_event::events`

- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Created` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Creating` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Deleted` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Deleting` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::ForceDeleted` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::ForceDeleting` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Replicating` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Restored` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Restoring` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Retrieved` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Retrieving` · framework/src/payments/entities/webhook_event.rs:21
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Saved` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Saving` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Trashed` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Updated` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::payments::entities::webhook_event::webhook_event::events::Updating` · framework/src/payments/entities/webhook_event.rs:21
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::payments::error`

- [ ] enum `suprnova::payments::PaymentError` · framework/src/payments/error.rs:11 (also `suprnova::payments::error::PaymentError`)
  - Variants: `Provider`, `Validation`, `NotSupported`, `Declined`, `Authentication`, `NotFound`, `WebhookSignature`, `InvalidPhoneNumber`, `InvalidCountryCode`, `Internal`
- [ ] type `suprnova::payments::PaymentResult` · framework/src/payments/error.rs:68 (also `suprnova::payments::error::PaymentResult`)

### `suprnova::payments::migrations`

- [ ] fn `suprnova::payments::migrations::migrations` · framework/src/payments/migrations/mod.rs:29

### `suprnova::payments::migrations::m_2026_05_22_000001_create_payments_tables`

- [ ] struct `suprnova::payments::migrations::CreatePaymentsTables` · framework/src/payments/migrations/m_2026_05_22_000001_create_payments_tables.rs:23 (also `suprnova::payments::migrations::m_2026_05_22_000001_create_payments_tables::Migration`)

### `suprnova::payments::mock`

- [ ] struct `suprnova::MockPaymentProvider` · framework/src/payments/mock.rs:169 (also `suprnova::payments::MockPaymentProvider`, `suprnova::payments::mock::MockPaymentProvider`)
  - Implements: `suprnova::payments::Checkout`, `suprnova::payments::CustomerStore`, `suprnova::payments::PaymentProvider`, `suprnova::payments::Promotions`, `suprnova::payments::Subscription`, `suprnova::payments::WebhookHandler`
  - [ ] fn `suprnova::MockPaymentProvider::new` · framework/src/payments/mock.rs:184
  - [ ] fn `suprnova::MockPaymentProvider::script_session_status` · framework/src/payments/mock.rs:198
  - [ ] fn `suprnova::MockPaymentProvider::recorded_sessions` · framework/src/payments/mock.rs:211
  - [ ] fn `suprnova::MockPaymentProvider::recorded_promotion_requests` · framework/src/payments/mock.rs:216

### `suprnova::payments::money`

- [ ] struct `suprnova::Money` · framework/src/payments/money.rs:29 (also `suprnova::payments::Money`, `suprnova::payments::money::Money`)
  - [ ] fn `suprnova::Money::from_minor_units` · framework/src/payments/money.rs:36
  - [ ] fn `suprnova::Money::from_decimal` · framework/src/payments/money.rs:58
  - [ ] fn `suprnova::Money::minor_units` · framework/src/payments/money.rs:72
  - [ ] fn `suprnova::Money::currency` · framework/src/payments/money.rs:77
  - [ ] fn `suprnova::Money::as_decimal` · framework/src/payments/money.rs:82
  - [ ] fn `suprnova::Money::is_zero` · framework/src/payments/money.rs:89

### `suprnova::payments::registry`

- [ ] struct `suprnova::PaymentProviderEntry` · framework/src/payments/registry.rs:46 (also `suprnova::payments::PaymentProviderEntry`, `suprnova::payments::registry::PaymentProviderEntry`)
  - Public fields: `name`, `factory`
- [ ] struct `suprnova::PaymentProviderRegistry` · framework/src/payments/registry.rs:75 (also `suprnova::payments::PaymentProviderRegistry`, `suprnova::payments::registry::PaymentProviderRegistry`)
  - [ ] fn `suprnova::PaymentProviderRegistry::get` · framework/src/payments/registry.rs:85
  - [ ] fn `suprnova::PaymentProviderRegistry::names` · framework/src/payments/registry.rs:101
  - [ ] fn `suprnova::PaymentProviderRegistry::bind` · framework/src/payments/registry.rs:119

### `suprnova::payments::traits`

- [ ] trait `suprnova::payments::PaymentProvider` · framework/src/payments/traits/mod.rs:31 (also `suprnova::payments::traits::PaymentProvider`)
  - Implemented here by: `MockPaymentProvider`
  - [ ] fn `suprnova::payments::PaymentProvider::name` · framework/src/payments/traits/mod.rs:33 (required)
  - [ ] fn `suprnova::payments::PaymentProvider::as_payment` · framework/src/payments/traits/mod.rs:37 (provided)
  - [ ] fn `suprnova::payments::PaymentProvider::as_promotions` · framework/src/payments/traits/mod.rs:44 (provided)

### `suprnova::payments::traits::checkout`

- [ ] trait `suprnova::payments::Checkout` · framework/src/payments/traits/checkout.rs:16 (also `suprnova::payments::traits::Checkout`, `suprnova::payments::traits::checkout::Checkout`)
  - Implemented here by: `MockPaymentProvider`
  - [ ] fn `suprnova::payments::Checkout::start_session` · framework/src/payments/traits/checkout.rs:21 (required)
  - [ ] fn `suprnova::payments::Checkout::session_status` · framework/src/payments/traits/checkout.rs:34 (provided)

### `suprnova::payments::traits::customer`

- [ ] trait `suprnova::payments::CustomerStore` · framework/src/payments/traits/customer.rs:13 (also `suprnova::payments::traits::CustomerStore`, `suprnova::payments::traits::customer::CustomerStore`)
  - Implemented here by: `MockPaymentProvider`
  - [ ] fn `suprnova::payments::CustomerStore::create_customer` · framework/src/payments/traits/customer.rs:15 (required)
  - [ ] fn `suprnova::payments::CustomerStore::update_customer` · framework/src/payments/traits/customer.rs:18 (required)
  - [ ] fn `suprnova::payments::CustomerStore::get_customer` · framework/src/payments/traits/customer.rs:20 (required)
  - [ ] fn `suprnova::payments::CustomerStore::delete_customer` · framework/src/payments/traits/customer.rs:22 (required)

### `suprnova::payments::traits::payment`

- [ ] trait `suprnova::payments::Payment` · framework/src/payments/traits/payment.rs:11 (also `suprnova::payments::traits::Payment`, `suprnova::payments::traits::payment::Payment`)
  - [ ] fn `suprnova::payments::Payment::charge` · framework/src/payments/traits/payment.rs:15 (required)
  - [ ] fn `suprnova::payments::Payment::capture` · framework/src/payments/traits/payment.rs:17 (required)
  - [ ] fn `suprnova::payments::Payment::refund` · framework/src/payments/traits/payment.rs:19 (required)
  - [ ] fn `suprnova::payments::Payment::void` · framework/src/payments/traits/payment.rs:22 (required)
  - [ ] fn `suprnova::payments::Payment::status` · framework/src/payments/traits/payment.rs:24 (required)

### `suprnova::payments::traits::promotions`

- [ ] struct `suprnova::payments::CreatePromotionCodeRequest` · framework/src/payments/traits/promotions.rs:16 (also `suprnova::payments::traits::CreatePromotionCodeRequest`, `suprnova::payments::traits::promotions::CreatePromotionCodeRequest`)
  - Public fields: `coupon_ref`, `customer_ref`, `expires_at`, `max_redemptions`
- [ ] struct `suprnova::payments::PromotionCode` · framework/src/payments/traits/promotions.rs:32 (also `suprnova::payments::traits::PromotionCode`, `suprnova::payments::traits::promotions::PromotionCode`)
  - Public fields: `code`, `provider_promotion_id`
- [ ] trait `suprnova::payments::Promotions` · framework/src/payments/traits/promotions.rs:44 (also `suprnova::payments::traits::Promotions`, `suprnova::payments::traits::promotions::Promotions`)
  - Implemented here by: `MockPaymentProvider`
  - [ ] fn `suprnova::payments::Promotions::create_promotion_code` · framework/src/payments/traits/promotions.rs:47 (required)

### `suprnova::payments::traits::subscription`

- [ ] trait `suprnova::payments::Subscription` · framework/src/payments/traits/subscription.rs:15 (also `suprnova::payments::traits::Subscription`, `suprnova::payments::traits::subscription::Subscription`)
  - Implemented here by: `MockPaymentProvider`
  - [ ] fn `suprnova::payments::Subscription::subscribe` · framework/src/payments/traits/subscription.rs:17 (required)
  - [ ] fn `suprnova::payments::Subscription::update` · framework/src/payments/traits/subscription.rs:21 (required)
  - [ ] fn `suprnova::payments::Subscription::cancel` · framework/src/payments/traits/subscription.rs:25 (required)
  - [ ] fn `suprnova::payments::Subscription::get` · framework/src/payments/traits/subscription.rs:32 (required)

### `suprnova::payments::traits::webhook`

- [ ] fn `suprnova::payments::constant_time_eq` · framework/src/payments/traits/webhook.rs:35 (also `suprnova::payments::traits::constant_time_eq`, `suprnova::payments::traits::webhook::constant_time_eq`)
- [ ] struct `suprnova::payments::CustomerSnapshot` · framework/src/payments/traits/webhook.rs:178 (also `suprnova::payments::traits::CustomerSnapshot`, `suprnova::payments::traits::webhook::CustomerSnapshot`)
  - Public fields: `provider_customer_id`, `email`, `provider_metadata`
- [ ] struct `suprnova::payments::PayloadIds` · framework/src/payments/traits/webhook.rs:131 (also `suprnova::payments::traits::PayloadIds`, `suprnova::payments::traits::webhook::PayloadIds`)
  - Public fields: `subscription_id`, `customer_id`, `transaction_id`
- [ ] struct `suprnova::payments::PaymentSnapshot` · framework/src/payments/traits/webhook.rs:146 (also `suprnova::payments::traits::PaymentSnapshot`, `suprnova::payments::traits::webhook::PaymentSnapshot`)
  - Public fields: `provider_transaction_id`, `provider_customer_id`, `provider_subscription_id`, `amount_total_minor`, `amount_tax_minor`, `currency`, `status`, `paid_at`, `provider_metadata`
- [ ] trait `suprnova::payments::WebhookHandler` · framework/src/payments/traits/webhook.rs:49 (also `suprnova::payments::traits::WebhookHandler`, `suprnova::payments::traits::webhook::WebhookHandler`)
  - Implemented here by: `MockPaymentProvider`
  - [ ] fn `suprnova::payments::WebhookHandler::mirrors_payment_transactions` · framework/src/payments/traits/webhook.rs:57 (provided)
  - [ ] fn `suprnova::payments::WebhookHandler::verify` · framework/src/payments/traits/webhook.rs:73 (required)
  - [ ] fn `suprnova::payments::WebhookHandler::parse_event` · framework/src/payments/traits/webhook.rs:78 (required)
  - [ ] fn `suprnova::payments::WebhookHandler::extract_payload_ids` · framework/src/payments/traits/webhook.rs:86 (provided)
  - [ ] fn `suprnova::payments::WebhookHandler::extract_payment_snapshot` · framework/src/payments/traits/webhook.rs:95 (provided)
  - [ ] fn `suprnova::payments::WebhookHandler::try_extract_payment_snapshot` · framework/src/payments/traits/webhook.rs:108 (provided)
  - [ ] fn `suprnova::payments::WebhookHandler::extract_customer_snapshot` · framework/src/payments/traits/webhook.rs:122 (provided)

### `suprnova::payments::webhook_route`

- [ ] fn `suprnova::payments::webhook_routes` · framework/src/payments/webhook_route.rs:999 (also `suprnova::payments::webhook_route::webhook_routes`)

## queue

### `suprnova::queue`

- [ ] fn `suprnova::queue::bootstrap_default` · framework/src/queue/mod.rs:1269
- [ ] fn `suprnova::queue::bootstrap_from_env` · framework/src/queue/mod.rs:1292
- [ ] struct `suprnova::EnvelopeOverrides` · framework/src/queue/mod.rs:92 (also `suprnova::queue::EnvelopeOverrides`)
  - Public fields: `queue`, `connection`, `timeout`, `fail_on_timeout`, `max_tries`, `backoff`, `after_commit`
- [ ] struct `suprnova::Queue` · framework/src/queue/mod.rs:124 (also `suprnova::prelude::Queue`, `suprnova::queue::Queue`)
  - [ ] fn `suprnova::Queue::route` · framework/src/queue/mod.rs:163
  - [ ] fn `suprnova::Queue::try_route` · framework/src/queue/mod.rs:176
  - [ ] fn `suprnova::Queue::route_for` · framework/src/queue/mod.rs:184
  - [ ] fn `suprnova::Queue::forward` · framework/src/queue/mod.rs:234
  - [ ] fn `suprnova::Queue::forward_on` · framework/src/queue/mod.rs:261
  - [ ] fn `suprnova::Queue::try_forward` · framework/src/queue/mod.rs:269
  - [ ] fn `suprnova::Queue::forward_for` · framework/src/queue/mod.rs:281
  - [ ] fn `suprnova::Queue::push` · framework/src/queue/mod.rs:309
  - [ ] fn `suprnova::Queue::push_after_commit` · framework/src/queue/mod.rs:328
  - [ ] fn `suprnova::Queue::push_later` · framework/src/queue/mod.rs:349
  - [ ] fn `suprnova::Queue::later` · framework/src/queue/mod.rs:393
  - [ ] fn `suprnova::Queue::push_with` · framework/src/queue/mod.rs:406
  - [ ] fn `suprnova::Queue::later_with` · framework/src/queue/mod.rs:415
  - [ ] fn `suprnova::Queue::push_debounced` · framework/src/queue/mod.rs:614
  - [ ] fn `suprnova::Queue::push_unique` · framework/src/queue/mod.rs:664
  - [ ] fn `suprnova::Queue::push_unique_later` · framework/src/queue/mod.rs:672
  - [ ] fn `suprnova::Queue::later_unique` · framework/src/queue/mod.rs:681
  - [ ] fn `suprnova::Queue::bulk` · framework/src/queue/mod.rs:868
  - [ ] fn `suprnova::Queue::batch` · framework/src/queue/mod.rs:904
  - [ ] fn `suprnova::Queue::chain` · framework/src/queue/mod.rs:909
  - [ ] fn `suprnova::Queue::size` · framework/src/queue/mod.rs:915
  - [ ] fn `suprnova::Queue::pending_size` · framework/src/queue/mod.rs:920
  - [ ] fn `suprnova::Queue::delayed_size` · framework/src/queue/mod.rs:925
  - [ ] fn `suprnova::Queue::reserved_size` · framework/src/queue/mod.rs:930
  - [ ] fn `suprnova::Queue::pending_jobs` · framework/src/queue/mod.rs:940
  - [ ] fn `suprnova::Queue::delayed_jobs` · framework/src/queue/mod.rs:947
  - [ ] fn `suprnova::Queue::reserved_jobs` · framework/src/queue/mod.rs:954
  - [ ] fn `suprnova::Queue::clear` · framework/src/queue/mod.rs:960
  - [ ] fn `suprnova::Queue::restart` · framework/src/queue/mod.rs:973
  - [ ] fn `suprnova::Queue::restart_signal` · framework/src/queue/mod.rs:981
  - [ ] fn `suprnova::Queue::pause` · framework/src/queue/mod.rs:997
  - [ ] fn `suprnova::Queue::resume` · framework/src/queue/mod.rs:1011
  - [ ] fn `suprnova::Queue::pause_all` · framework/src/queue/mod.rs:1026
  - [ ] fn `suprnova::Queue::resume_all` · framework/src/queue/mod.rs:1042
  - [ ] fn `suprnova::Queue::is_paused` · framework/src/queue/mod.rs:1051
  - [ ] fn `suprnova::Queue::paused_queues` · framework/src/queue/mod.rs:1066
  - [ ] fn `suprnova::Queue::set_failed_store` · framework/src/queue/mod.rs:1087
  - [ ] fn `suprnova::Queue::failed_store` · framework/src/queue/mod.rs:1094
  - [ ] fn `suprnova::Queue::retry_failed` · framework/src/queue/mod.rs:1106
  - [ ] fn `suprnova::Queue::retry_all_failed` · framework/src/queue/mod.rs:1132
  - [ ] fn `suprnova::Queue::set_batch_repository` · framework/src/queue/mod.rs:1166
  - [ ] fn `suprnova::Queue::batch_repository` · framework/src/queue/mod.rs:1171
  - [ ] fn `suprnova::Queue::set_connection_name` · framework/src/queue/mod.rs:1177
  - [ ] fn `suprnova::Queue::connection_name` · framework/src/queue/mod.rs:1185
  - [ ] fn `suprnova::Queue::set_driver` · framework/src/queue/mod.rs:1198
  - [ ] fn `suprnova::Queue::driver_name` · framework/src/queue/mod.rs:1217
  - [ ] fn `suprnova::Queue::driver` · framework/src/queue/mod.rs:1228

### `suprnova::queue::batch`

- [ ] fn `suprnova::queue::batch::current_repository` · framework/src/queue/batch.rs:1172
- [ ] fn `suprnova::queue::batch::install_repository` · framework/src/queue/batch.rs:1163
- [ ] fn `suprnova::queue::batch::register_callback` · framework/src/queue/batch.rs:1144
- [ ] struct `suprnova::Batch` · framework/src/queue/batch.rs:32 (also `suprnova::queue::Batch`, `suprnova::queue::batch::Batch`)
  - Public fields: `id`, `name`, `total_jobs`, `pending_jobs`, `failed_jobs`, `failed_job_ids`, `options`, `created_at`, `cancelled_at`, `finished_at`
  - [ ] fn `suprnova::Batch::finished` · framework/src/queue/batch.rs:57
  - [ ] fn `suprnova::Batch::cancelled` · framework/src/queue/batch.rs:62
  - [ ] fn `suprnova::Batch::processed_jobs` · framework/src/queue/batch.rs:68
  - [ ] fn `suprnova::Batch::progress` · framework/src/queue/batch.rs:73
- [ ] struct `suprnova::BatchOptions` · framework/src/queue/batch.rs:84 (also `suprnova::queue::BatchOptions`, `suprnova::queue::batch::BatchOptions`)
  - Public fields: `then_callbacks`, `catch_callbacks`, `finally_callbacks`, `allow_failures`
- [ ] struct `suprnova::DatabaseBatchRepository` · framework/src/queue/batch.rs:456 (also `suprnova::queue::DatabaseBatchRepository`, `suprnova::queue::batch::DatabaseBatchRepository`)
  - Implements: `suprnova::BatchRepository`
  - [ ] fn `suprnova::DatabaseBatchRepository::new` · framework/src/queue/batch.rs:474
  - [ ] fn `suprnova::DatabaseBatchRepository::with_tables` · framework/src/queue/batch.rs:493
- [ ] struct `suprnova::MemoryBatchRepository` · framework/src/queue/batch.rs:234 (also `suprnova::queue::MemoryBatchRepository`, `suprnova::queue::batch::MemoryBatchRepository`)
  - Implements: `suprnova::BatchRepository`
  - [ ] fn `suprnova::MemoryBatchRepository::new` · framework/src/queue/batch.rs:240
- [ ] struct `suprnova::PendingBatch` · framework/src/queue/batch.rs:1211 (also `suprnova::queue::PendingBatch`, `suprnova::queue::batch::PendingBatch`)
  - Public fields: `name`, `options`
  - [ ] fn `suprnova::PendingBatch::new` · framework/src/queue/batch.rs:1235
  - [ ] fn `suprnova::PendingBatch::name` · framework/src/queue/batch.rs:1246
  - [ ] fn `suprnova::PendingBatch::add` · framework/src/queue/batch.rs:1254
  - [ ] fn `suprnova::PendingBatch::then` · framework/src/queue/batch.rs:1278
  - [ ] fn `suprnova::PendingBatch::catch` · framework/src/queue/batch.rs:1284
  - [ ] fn `suprnova::PendingBatch::finally` · framework/src/queue/batch.rs:1291
  - [ ] fn `suprnova::PendingBatch::allow_failures` · framework/src/queue/batch.rs:1298
  - [ ] fn `suprnova::PendingBatch::len` · framework/src/queue/batch.rs:1304
  - [ ] fn `suprnova::PendingBatch::is_empty` · framework/src/queue/batch.rs:1309
  - [ ] fn `suprnova::PendingBatch::dispatch` · framework/src/queue/batch.rs:1345
- [ ] struct `suprnova::TerminalCallbackClaim` · framework/src/queue/batch.rs:114 (also `suprnova::queue::TerminalCallbackClaim`, `suprnova::queue::batch::TerminalCallbackClaim`)
  - Public fields: `finished_at`, `cancelled_at`
- [ ] struct `suprnova::UpdatedBatchJobCounts` · framework/src/queue/batch.rs:101 (also `suprnova::queue::UpdatedBatchJobCounts`, `suprnova::queue::batch::UpdatedBatchJobCounts`)
  - Public fields: `pending_jobs`, `failed_jobs`
- [ ] trait `suprnova::BatchCallback` · framework/src/queue/batch.rs:1119 (also `suprnova::queue::BatchCallback`, `suprnova::queue::batch::BatchCallback`)
  - [ ] fn `suprnova::BatchCallback::name` · framework/src/queue/batch.rs:1121 (required)
  - [ ] fn `suprnova::BatchCallback::handle` · framework/src/queue/batch.rs:1133 (required)
- [ ] trait `suprnova::BatchRepository` · framework/src/queue/batch.rs:125 (also `suprnova::queue::BatchRepository`, `suprnova::queue::batch::BatchRepository`)
  - Implemented here by: `DatabaseBatchRepository`, `MemoryBatchRepository`
  - [ ] fn `suprnova::BatchRepository::store` · framework/src/queue/batch.rs:132 (required)
  - [ ] fn `suprnova::BatchRepository::find` · framework/src/queue/batch.rs:134 (required)
  - [ ] fn `suprnova::BatchRepository::increment_total_jobs` · framework/src/queue/batch.rs:140 (required)
  - [ ] fn `suprnova::BatchRepository::record_successful_job` · framework/src/queue/batch.rs:155 (required)
  - [ ] fn `suprnova::BatchRepository::record_failed_job` · framework/src/queue/batch.rs:168 (required)
  - [ ] fn `suprnova::BatchRepository::cancel` · framework/src/queue/batch.rs:175 (required)
  - [ ] fn `suprnova::BatchRepository::is_cancelled` · framework/src/queue/batch.rs:177 (required)
  - [ ] fn `suprnova::BatchRepository::mark_finished` · framework/src/queue/batch.rs:184 (required)
  - [ ] fn `suprnova::BatchRepository::claim_terminal_callbacks` · framework/src/queue/batch.rs:199 (provided)
  - [ ] fn `suprnova::BatchRepository::delete` · framework/src/queue/batch.rs:209 (required)
- [ ] const `suprnova::DEFAULT_BATCH_SETTLEMENTS_TABLE` · framework/src/queue/batch.rs:402 (also `suprnova::queue::DEFAULT_BATCH_SETTLEMENTS_TABLE`, `suprnova::queue::batch::DEFAULT_BATCH_SETTLEMENTS_TABLE`)
- [ ] const `suprnova::DEFAULT_BATCHES_TABLE` · framework/src/queue/batch.rs:400 (also `suprnova::queue::DEFAULT_BATCHES_TABLE`, `suprnova::queue::batch::DEFAULT_BATCHES_TABLE`)

### `suprnova::queue::chain`

- [ ] fn `suprnova::queue::chain::next_link_id` · framework/src/queue/chain.rs:166
- [ ] struct `suprnova::ChainLink` · framework/src/queue/chain.rs:22 (also `suprnova::queue::ChainLink`, `suprnova::queue::chain::ChainLink`)
  - Public fields: `job_name`, `payload`, `max_tries`, `timeout_secs`, `fail_on_timeout`, `backoff`, `queue`
  - [ ] fn `suprnova::ChainLink::from_job` · framework/src/queue/chain.rs:52
  - [ ] fn `suprnova::ChainLink::to_envelope` · framework/src/queue/chain.rs:70
  - [ ] fn `suprnova::ChainLink::to_envelope_after` · framework/src/queue/chain.rs:108
- [ ] struct `suprnova::PendingChain` · framework/src/queue/chain.rs:173 (also `suprnova::queue::PendingChain`, `suprnova::queue::chain::PendingChain`)
  - [ ] fn `suprnova::PendingChain::new` · framework/src/queue/chain.rs:185
  - [ ] fn `suprnova::PendingChain::add` · framework/src/queue/chain.rs:191
  - [ ] fn `suprnova::PendingChain::len` · framework/src/queue/chain.rs:207
  - [ ] fn `suprnova::PendingChain::is_empty` · framework/src/queue/chain.rs:211
  - [ ] fn `suprnova::PendingChain::dispatch` · framework/src/queue/chain.rs:217

### `suprnova::queue::database`

- [ ] struct `suprnova::DatabaseQueueDriver` · framework/src/queue/database.rs:27 (also `suprnova::queue::DatabaseQueueDriver`, `suprnova::queue::database::DatabaseQueueDriver`)
  - Implements: `suprnova::QueueDriver`
  - [ ] fn `suprnova::DatabaseQueueDriver::new` · framework/src/queue/database.rs:46

### `suprnova::queue::debounce`

- [ ] struct `suprnova::Debounced` · framework/src/queue/debounce.rs:29 (also `suprnova::queue::Debounced`, `suprnova::queue::debounce::Debounced`)
  - Public fields: `owner`, `max_wait_exceeded`
- [ ] struct `suprnova::DebounceOptions` · framework/src/queue/debounce.rs:53 (also `suprnova::queue::DebounceOptions`, `suprnova::queue::debounce::DebounceOptions`)
  - Public fields: `window`, `max_wait`, `id`
  - [ ] fn `suprnova::DebounceOptions::new` · framework/src/queue/debounce.rs:65
  - [ ] fn `suprnova::DebounceOptions::max_wait` · framework/src/queue/debounce.rs:74
  - [ ] fn `suprnova::DebounceOptions::id` · framework/src/queue/debounce.rs:81

### `suprnova::queue::driver`

- [ ] struct `suprnova::Reservation` · framework/src/queue/driver.rs:42 (also `suprnova::queue::Reservation`, `suprnova::queue::driver::Reservation`)
  - Public fields: `envelope`, `token`
- [ ] struct `suprnova::ReservationToken` · framework/src/queue/driver.rs:14 (also `suprnova::queue::ReservationToken`, `suprnova::queue::driver::ReservationToken`)
  - Public tuple fields: 1
- [ ] enum `suprnova::QueueFilterCapability` · framework/src/queue/driver.rs:59 (also `suprnova::queue::QueueFilterCapability`, `suprnova::queue::driver::QueueFilterCapability`)
  - Variants: `Supported`, `Unsupported`, `Unknown`
- [ ] enum `suprnova::Settled` · framework/src/queue/driver.rs:21 (also `suprnova::queue::Settled`, `suprnova::queue::driver::Settled`)
  - Variants: `Atomically`, `Stale`, `Unsupported`
- [ ] trait `suprnova::QueueDriver` · framework/src/queue/driver.rs:72 (also `suprnova::queue::QueueDriver`, `suprnova::queue::driver::QueueDriver`)
  - Implemented here by: `DatabaseQueueDriver`, `FailoverQueueDriver`, `MemoryQueueDriver`, `NullQueueDriver`, `RedisQueueDriver`, `SyncQueueDriver`
  - [ ] fn `suprnova::QueueDriver::push` · framework/src/queue/driver.rs:74 (required)
  - [ ] fn `suprnova::QueueDriver::pop` · framework/src/queue/driver.rs:79 (required)
  - [ ] fn `suprnova::QueueDriver::queue_filter_capability` · framework/src/queue/driver.rs:89 (provided)
  - [ ] fn `suprnova::QueueDriver::reservation_deadline` · framework/src/queue/driver.rs:107 (provided)
  - [ ] fn `suprnova::QueueDriver::pop_from` · framework/src/queue/driver.rs:131 (provided)
  - [ ] fn `suprnova::QueueDriver::ack` · framework/src/queue/driver.rs:150 (required)
  - [ ] fn `suprnova::QueueDriver::nack` · framework/src/queue/driver.rs:162 (required)
  - [ ] fn `suprnova::QueueDriver::release` · framework/src/queue/driver.rs:197 (provided)
  - [ ] fn `suprnova::QueueDriver::settle` · framework/src/queue/driver.rs:251 (provided)
  - [ ] fn `suprnova::QueueDriver::size` · framework/src/queue/driver.rs:266 (provided)
  - [ ] fn `suprnova::QueueDriver::pending_size` · framework/src/queue/driver.rs:276 (provided)
  - [ ] fn `suprnova::QueueDriver::delayed_size` · framework/src/queue/driver.rs:285 (provided)
  - [ ] fn `suprnova::QueueDriver::reserved_size` · framework/src/queue/driver.rs:291 (provided)
  - [ ] fn `suprnova::QueueDriver::pending_jobs` · framework/src/queue/driver.rs:311 (provided)
  - [ ] fn `suprnova::QueueDriver::delayed_jobs` · framework/src/queue/driver.rs:323 (provided)
  - [ ] fn `suprnova::QueueDriver::reserved_jobs` · framework/src/queue/driver.rs:336 (provided)
  - [ ] fn `suprnova::QueueDriver::clear` · framework/src/queue/driver.rs:349 (provided)
  - [ ] fn `suprnova::QueueDriver::bulk_push` · framework/src/queue/driver.rs:359 (provided)
  - [ ] fn `suprnova::QueueDriver::name` · framework/src/queue/driver.rs:367 (provided)

### `suprnova::queue::envelope`

- [ ] fn `suprnova::queue::envelope::queue_matches` · framework/src/queue/envelope.rs:28
- [ ] struct `suprnova::Envelope` · framework/src/queue/envelope.rs:55 (also `suprnova::queue::Envelope`, `suprnova::queue::envelope::Envelope`)
  - Public fields: `schema_version`, `id`, `job_name`, `queue`, `payload`, `dispatched_at`, `available_at`, `attempts`, `max_tries`, `backoff`, `timeout_secs`, `fail_on_timeout`, `idempotency_key`, `unique_lock_owner`, `debounce_id`, `debounce_owner`, `batch_id`, `chain_remaining`
  - [ ] fn `suprnova::Envelope::from_json` · framework/src/queue/envelope.rs:179
  - [ ] fn `suprnova::Envelope::to_json` · framework/src/queue/envelope.rs:188
- [ ] enum `suprnova::EnvelopeError` · framework/src/queue/envelope.rs:166 (also `suprnova::queue::EnvelopeError`, `suprnova::queue::envelope::EnvelopeError`)
  - Variants: `UnsupportedSchemaVersion`, `Decode`
- [ ] const `suprnova::queue::CURRENT_SCHEMA_VERSION` · framework/src/queue/envelope.rs:13 (also `suprnova::queue::envelope::CURRENT_SCHEMA_VERSION`)
- [ ] const `suprnova::queue::envelope::DEFAULT_QUEUE` · framework/src/queue/envelope.rs:20

### `suprnova::queue::errors`

- [ ] struct `suprnova::ManuallyFailed` · framework/src/queue/errors.rs:42 (also `suprnova::queue::ManuallyFailed`, `suprnova::queue::errors::ManuallyFailed`)
  - Public fields: `job_name`, `reason`
- [ ] struct `suprnova::MaxAttemptsExceeded` · framework/src/queue/errors.rs:17 (also `suprnova::queue::MaxAttemptsExceeded`, `suprnova::queue::errors::MaxAttemptsExceeded`)
  - Public fields: `job_name`, `attempts`, `reason`
- [ ] struct `suprnova::TimeoutExceeded` · framework/src/queue/errors.rs:30 (also `suprnova::queue::TimeoutExceeded`, `suprnova::queue::errors::TimeoutExceeded`)
  - Public fields: `job_name`, `timeout`

### `suprnova::queue::events`

- [ ] struct `suprnova::queue::events::JobAttempted` · framework/src/queue/events.rs:118
  - Public fields: `job`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobDebounced` · framework/src/queue/events.rs:448
  - Public fields: `job`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobExceptionOccurred` · framework/src/queue/events.rs:132
  - Public fields: `job`, `exception`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobFailed` · framework/src/queue/events.rs:148
  - Public fields: `job`, `exception`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobIdentity` · framework/src/queue/events.rs:25
  - Public fields: `id`, `job_name`, `attempts`, `max_tries`, `connection`
- [ ] struct `suprnova::queue::events::JobProcessed` · framework/src/queue/events.rs:101
  - Public fields: `job`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobProcessing` · framework/src/queue/events.rs:87
  - Public fields: `job`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobQueued` · framework/src/queue/events.rs:69
  - Public fields: `id`, `job_name`, `connection`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobQueueing` · framework/src/queue/events.rs:53
  - Public fields: `job_name`, `connection`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobReleased` · framework/src/queue/events.rs:186
  - Public fields: `job`, `delay_secs`, `reason`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobReleasedAfterException` · framework/src/queue/events.rs:165
  - Public fields: `job`, `exception`, `delay_secs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::JobTimedOut` · framework/src/queue/events.rs:204
  - Public fields: `job`, `timeout`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::Looping` · framework/src/queue/events.rs:220
  - Public fields: `connection`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::QueueFailedOver` · framework/src/queue/events.rs:375
  - Public fields: `connection`, `job_name`, `exception`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::QueuePaused` · framework/src/queue/events.rs:333
  - Public fields: `connection`, `queue`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::QueueResumed` · framework/src/queue/events.rs:349
  - Public fields: `connection`, `queue`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::QueuesPaused` · framework/src/queue/events.rs:309
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::QueuesResumed` · framework/src/queue/events.rs:321
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::UniqueJobSkipped` · framework/src/queue/events.rs:290
  - Public fields: `job_name`, `unique_id`, `connection`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::WorkerInterrupted` · framework/src/queue/events.rs:262
  - Public fields: `connection`, `processed`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::WorkerQueuePaused` · framework/src/queue/events.rs:400
  - Public fields: `connection`, `queue`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::WorkerQueueResumed` · framework/src/queue/events.rs:428
  - Public fields: `connection`, `queue`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::WorkerStarting` · framework/src/queue/events.rs:233
  - Public fields: `connection`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::queue::events::WorkerStopping` · framework/src/queue/events.rs:246
  - Public fields: `connection`, `processed`
  - Implements: `suprnova::Event`

### `suprnova::queue::failed`

- [ ] struct `suprnova::DatabaseFailedJobStore` · framework/src/queue/failed.rs:255 (also `suprnova::queue::DatabaseFailedJobStore`, `suprnova::queue::failed::DatabaseFailedJobStore`)
  - Implements: `suprnova::FailedJobStore`
  - [ ] fn `suprnova::DatabaseFailedJobStore::new` · framework/src/queue/failed.rs:264
- [ ] struct `suprnova::FailedJob` · framework/src/queue/failed.rs:34 (also `suprnova::queue::FailedJob`, `suprnova::queue::failed::FailedJob`)
  - Public fields: `id`, `connection`, `queue`, `job_name`, `envelope_json`, `exception`, `failed_at`
- [ ] struct `suprnova::MemoryFailedJobStore` · framework/src/queue/failed.rs:91 (also `suprnova::queue::MemoryFailedJobStore`, `suprnova::queue::failed::MemoryFailedJobStore`)
  - Implements: `suprnova::FailedJobStore`
  - [ ] fn `suprnova::MemoryFailedJobStore::new` · framework/src/queue/failed.rs:97
- [ ] struct `suprnova::NullFailedJobStore` · framework/src/queue/failed.rs:194 (also `suprnova::queue::NullFailedJobStore`, `suprnova::queue::failed::NullFailedJobStore`)
  - Implements: `suprnova::FailedJobStore`
  - [ ] fn `suprnova::NullFailedJobStore::new` · framework/src/queue/failed.rs:198
- [ ] trait `suprnova::FailedJobStore` · framework/src/queue/failed.rs:54 (also `suprnova::queue::FailedJobStore`, `suprnova::queue::failed::FailedJobStore`)
  - Implemented here by: `DatabaseFailedJobStore`, `MemoryFailedJobStore`, `NullFailedJobStore`
  - [ ] fn `suprnova::FailedJobStore::log` · framework/src/queue/failed.rs:56 (required)
  - [ ] fn `suprnova::FailedJobStore::all` · framework/src/queue/failed.rs:65 (required)
  - [ ] fn `suprnova::FailedJobStore::ids` · framework/src/queue/failed.rs:68 (required)
  - [ ] fn `suprnova::FailedJobStore::find` · framework/src/queue/failed.rs:71 (required)
  - [ ] fn `suprnova::FailedJobStore::forget` · framework/src/queue/failed.rs:74 (required)
  - [ ] fn `suprnova::FailedJobStore::flush` · framework/src/queue/failed.rs:78 (required)
  - [ ] fn `suprnova::FailedJobStore::count` · framework/src/queue/failed.rs:81 (required)

### `suprnova::queue::failover`

- [ ] struct `suprnova::FailoverQueueDriver` · framework/src/queue/failover.rs:111 (also `suprnova::queue::FailoverQueueDriver`, `suprnova::queue::failover::FailoverQueueDriver`)
  - Implements: `suprnova::QueueDriver`
  - [ ] fn `suprnova::FailoverQueueDriver::new` · framework/src/queue/failover.rs:154

### `suprnova::queue::inspect`

- [ ] struct `suprnova::InspectedJob` · framework/src/queue/inspect.rs:28 (also `suprnova::queue::InspectedJob`, `suprnova::queue::inspect::InspectedJob`)
  - Public fields: `id`, `queue`, `name`, `attempts`, `payload`, `created_at`
  - [ ] fn `suprnova::InspectedJob::from_envelope` · framework/src/queue/inspect.rs:50

### `suprnova::queue::job`

- [ ] enum `suprnova::BackoffSchedule` · framework/src/queue/job.rs:11 (also `suprnova::queue::BackoffSchedule`, `suprnova::queue::job::BackoffSchedule`)
  - Variants: `Fixed`, `Exponential`, `Sequence`
- [ ] trait `suprnova::Job` · framework/src/queue/job.rs:59 (also `suprnova::prelude::Job`, `suprnova::queue::Job`, `suprnova::queue::job::Job`)
  - Implemented here by: `SendMailJob`, `SendNotificationJob`
  - [ ] fn `suprnova::Job::job_name` · framework/src/queue/job.rs:62 (required)
  - [ ] fn `suprnova::Job::handle` · framework/src/queue/job.rs:67 (required)
  - [ ] fn `suprnova::Job::queue` · framework/src/queue/job.rs:83 (provided)
  - [ ] fn `suprnova::Job::connection` · framework/src/queue/job.rs:100 (provided)
  - [ ] fn `suprnova::Job::delay` · framework/src/queue/job.rs:117 (provided)
  - [ ] fn `suprnova::Job::max_tries` · framework/src/queue/job.rs:125 (provided)
  - [ ] fn `suprnova::Job::backoff` · framework/src/queue/job.rs:133 (provided)
  - [ ] fn `suprnova::Job::timeout` · framework/src/queue/job.rs:141 (provided)
  - [ ] fn `suprnova::Job::fail_on_timeout` · framework/src/queue/job.rs:150 (provided)
  - [ ] fn `suprnova::Job::after_commit` · framework/src/queue/job.rs:175 (provided)
  - [ ] fn `suprnova::Job::unique_id` · framework/src/queue/job.rs:191 (provided)
  - [ ] fn `suprnova::Job::unique_for` · framework/src/queue/job.rs:204 (provided)
  - [ ] fn `suprnova::Job::unique_until_processing` · framework/src/queue/job.rs:225 (provided)
  - [ ] fn `suprnova::Job::debounce_for` · framework/src/queue/job.rs:251 (provided)
  - [ ] fn `suprnova::Job::max_debounce_wait` · framework/src/queue/job.rs:266 (provided)
  - [ ] fn `suprnova::Job::debounce_id` · framework/src/queue/job.rs:284 (provided)
  - [ ] fn `suprnova::Job::middleware` · framework/src/queue/job.rs:295 (provided)

### `suprnova::queue::memory`

- [ ] struct `suprnova::MemoryQueueDriver` · framework/src/queue/memory.rs:94 (also `suprnova::queue::MemoryQueueDriver`, `suprnova::queue::memory::MemoryQueueDriver`)
  - Implements: `suprnova::QueueDriver`
  - [ ] fn `suprnova::MemoryQueueDriver::new` · framework/src/queue/memory.rs:182

### `suprnova::queue::middleware`

- [ ] struct `suprnova::FailOnException` · framework/src/queue/middleware.rs:375 (also `suprnova::queue::FailOnException`, `suprnova::queue::middleware::FailOnException`)
  - Implements: `suprnova::JobMiddleware`
  - [ ] fn `suprnova::FailOnException::new` · framework/src/queue/middleware.rs:382
  - [ ] fn `suprnova::FailOnException::on_substring` · framework/src/queue/middleware.rs:396
- [ ] struct `suprnova::RateLimited` · framework/src/queue/middleware.rs:191 (also `suprnova::queue::RateLimited`, `suprnova::queue::middleware::RateLimited`)
  - Public fields: `max_attempts`, `decay`, `key`, `release_after`
  - Implements: `suprnova::JobMiddleware`
  - [ ] fn `suprnova::RateLimited::new` · framework/src/queue/middleware.rs:204
  - [ ] fn `suprnova::RateLimited::by` · framework/src/queue/middleware.rs:214
  - [ ] fn `suprnova::RateLimited::release_after` · framework/src/queue/middleware.rs:220
- [ ] struct `suprnova::Skip` · framework/src/queue/middleware.rs:338 (also `suprnova::queue::Skip`, `suprnova::queue::middleware::Skip`)
  - Implements: `suprnova::JobMiddleware`
  - [ ] fn `suprnova::Skip::when` · framework/src/queue/middleware.rs:344
  - [ ] fn `suprnova::Skip::unless` · framework/src/queue/middleware.rs:349
- [ ] struct `suprnova::SkipIfBatchCancelled` · framework/src/queue/middleware.rs:429 (also `suprnova::queue::SkipIfBatchCancelled`, `suprnova::queue::middleware::SkipIfBatchCancelled`)
  - Implements: `suprnova::JobMiddleware`
- [ ] struct `suprnova::ThrottlesExceptions` · framework/src/queue/middleware.rs:260 (also `suprnova::queue::ThrottlesExceptions`, `suprnova::queue::middleware::ThrottlesExceptions`)
  - Public fields: `max_attempts`, `decay`, `backoff`, `key`
  - Implements: `suprnova::JobMiddleware`
  - [ ] fn `suprnova::ThrottlesExceptions::new` · framework/src/queue/middleware.rs:274
  - [ ] fn `suprnova::ThrottlesExceptions::backoff` · framework/src/queue/middleware.rs:285
  - [ ] fn `suprnova::ThrottlesExceptions::by` · framework/src/queue/middleware.rs:291
- [ ] struct `suprnova::WithoutOverlapping` · framework/src/queue/middleware.rs:64 (also `suprnova::queue::WithoutOverlapping`, `suprnova::queue::middleware::WithoutOverlapping`)
  - Public fields: `key`, `release_after`, `expires_after`, `prefix`, `share_key`
  - Implements: `suprnova::JobMiddleware`
  - [ ] fn `suprnova::WithoutOverlapping::new` · framework/src/queue/middleware.rs:81
  - [ ] fn `suprnova::WithoutOverlapping::release_after` · framework/src/queue/middleware.rs:93
  - [ ] fn `suprnova::WithoutOverlapping::dont_release` · framework/src/queue/middleware.rs:100
  - [ ] fn `suprnova::WithoutOverlapping::expire_after` · framework/src/queue/middleware.rs:106
  - [ ] fn `suprnova::WithoutOverlapping::with_prefix` · framework/src/queue/middleware.rs:112
  - [ ] fn `suprnova::WithoutOverlapping::shared` · framework/src/queue/middleware.rs:119
- [ ] trait `suprnova::JobMiddleware` · framework/src/queue/middleware.rs:47 (also `suprnova::queue::JobMiddleware`, `suprnova::queue::middleware::JobMiddleware`)
  - Implemented here by: `FailOnException`, `RateLimited`, `Skip`, `SkipIfBatchCancelled`, `ThrottlesExceptions`, `WithoutOverlapping`
  - [ ] fn `suprnova::JobMiddleware::handle` · framework/src/queue/middleware.rs:51 (required)
- [ ] type `suprnova::JobMiddlewareNext` · framework/src/queue/middleware.rs:39 (also `suprnova::queue::JobMiddlewareNext`, `suprnova::queue::middleware::Next`)

### `suprnova::queue::null`

- [ ] struct `suprnova::NullQueueDriver` · framework/src/queue/null.rs:19 (also `suprnova::queue::NullQueueDriver`, `suprnova::queue::null::NullQueueDriver`)
  - Implements: `suprnova::QueueDriver`
  - [ ] fn `suprnova::NullQueueDriver::new` · framework/src/queue/null.rs:23

### `suprnova::queue::outcome`

- [ ] enum `suprnova::JobOutcome` · framework/src/queue/outcome.rs:20 (also `suprnova::queue::JobOutcome`, `suprnova::queue::outcome::JobOutcome`)
  - Variants: `Completed`, `Released`, `Failed`, `Deleted`
  - [ ] fn `suprnova::JobOutcome::is_release` · framework/src/queue/outcome.rs:52
  - [ ] fn `suprnova::JobOutcome::is_terminal` · framework/src/queue/outcome.rs:57

### `suprnova::queue::redis`

- [ ] struct `suprnova::RedisQueueDriver` · framework/src/queue/redis.rs:1485 (also `suprnova::queue::RedisQueueDriver`, `suprnova::queue::redis::RedisQueueDriver`)
  - Implements: `suprnova::QueueDriver`
  - [ ] fn `suprnova::RedisQueueDriver::connect` · framework/src/queue/redis.rs:1647

### `suprnova::queue::retry`

- [ ] fn `suprnova::queue::retry::next_delay` · framework/src/queue/retry.rs:16

### `suprnova::queue::routing`

- [ ] struct `suprnova::QueueRoute` · framework/src/queue/routing.rs:66 (also `suprnova::queue::QueueRoute`, `suprnova::queue::routing::QueueRoute`)
  - Public fields: `connection`, `queue`

### `suprnova::queue::sync`

- [ ] struct `suprnova::SyncQueueDriver` · framework/src/queue/sync.rs:27 (also `suprnova::queue::SyncQueueDriver`, `suprnova::queue::sync::SyncQueueDriver`)
  - Implements: `suprnova::QueueDriver`
  - [ ] fn `suprnova::SyncQueueDriver::new` · framework/src/queue/sync.rs:31

### `suprnova::queue::testing`

- [ ] fn `suprnova::queue::testing::assert_pushed` · framework/src/queue/testing.rs:156
- [ ] fn `suprnova::queue::testing::assert_pushed_later` · framework/src/queue/testing.rs:194
- [ ] fn `suprnova::queue::testing::assert_pushed_on_connection` · framework/src/queue/testing.rs:324
- [ ] fn `suprnova::queue::testing::assert_pushed_on_queue` · framework/src/queue/testing.rs:301
- [ ] fn `suprnova::queue::testing::delayed_jobs` · framework/src/queue/testing.rs:369
- [ ] fn `suprnova::queue::testing::install_fake` · framework/src/queue/testing.rs:134
- [ ] fn `suprnova::queue::testing::pending_jobs` · framework/src/queue/testing.rs:352
- [ ] fn `suprnova::queue::testing::pushed` · framework/src/queue/testing.rs:219
- [ ] fn `suprnova::queue::testing::pushed_with_available_at` · framework/src/queue/testing.rs:174
- [ ] fn `suprnova::queue::testing::pushed_with_id` · framework/src/queue/testing.rs:241
- [ ] fn `suprnova::queue::testing::pushed_with_overrides` · framework/src/queue/testing.rs:271
- [ ] struct `suprnova::queue::testing::QueueFakeGuard` · framework/src/queue/testing.rs:142

### `suprnova::queue::worker`

- [ ] fn `suprnova::queue::worker::dispatch_by_name` · framework/src/queue/worker.rs:131
- [ ] fn `suprnova::queue::worker::register_job` · framework/src/queue/worker.rs:89
- [ ] fn `suprnova::queue::worker::registered_job_names` · framework/src/queue/worker.rs:333
- [ ] fn `suprnova::queue::worker::run_through_middleware` · framework/src/queue/worker.rs:278
- [ ] fn `suprnova::queue::worker::run_worker` · framework/src/queue/worker.rs:533
- [ ] struct `suprnova::queue::worker::WorkerConfig` · framework/src/queue/worker.rs:352
  - Public fields: `visibility_timeout`, `poll_interval`, `max_jobs`, `queues`

## rate_limit

### `suprnova::rate_limit`

- [ ] fn `suprnova::rate_limit::bootstrap_default` · framework/src/rate_limit/mod.rs:110
- [ ] fn `suprnova::rate_limit::bootstrap_from_env` · framework/src/rate_limit/mod.rs:250
- [ ] fn `suprnova::identity_key` · framework/src/rate_limit/mod.rs:357 (also `suprnova::rate_limit::identity_key`)
- [ ] fn `suprnova::names_identity` · framework/src/rate_limit/mod.rs:397 (also `suprnova::rate_limit::names_identity`)
- [ ] struct `suprnova::RateLimitMiddleware` · framework/src/rate_limit/mod.rs:480 (also `suprnova::rate_limit::RateLimitMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::RateLimitMiddleware::new` · framework/src/rate_limit/mod.rs:507
  - [ ] fn `suprnova::RateLimitMiddleware::only_when` · framework/src/rate_limit/mod.rs:539
  - [ ] fn `suprnova::RateLimitMiddleware::key_reads_body` · framework/src/rate_limit/mod.rs:569
  - [ ] fn `suprnova::RateLimitMiddleware::on_backend_error` · framework/src/rate_limit/mod.rs:580
- [ ] struct `suprnova::SlidingWindowConfig` · framework/src/rate_limit/mod.rs:51 (also `suprnova::rate_limit::SlidingWindowConfig`)
  - Public fields: `max_requests`, `window`
- [ ] enum `suprnova::BackendErrorPolicy` · framework/src/rate_limit/mod.rs:433 (also `suprnova::rate_limit::BackendErrorPolicy`)
  - Variants: `FailOpen`, `FailClosed`
- [ ] trait `suprnova::RateLimiterDriver` · framework/src/rate_limit/mod.rs:66 (also `suprnova::rate_limit::RateLimiterDriver`)
  - Implemented here by: `rate_limit::memory::InMemoryRateLimiter`, `rate_limit::redis::RedisRateLimiter`
  - [ ] fn `suprnova::RateLimiterDriver::try_acquire` · framework/src/rate_limit/mod.rs:69 (required)
  - [ ] fn `suprnova::RateLimiterDriver::retry_after` · framework/src/rate_limit/mod.rs:77 (required)

### `suprnova::rate_limit::algorithm`

- [ ] struct `suprnova::rate_limit::algorithm::Bucket` · framework/src/rate_limit/algorithm.rs:9
  - [ ] fn `suprnova::rate_limit::algorithm::Bucket::new` · framework/src/rate_limit/algorithm.rs:16
  - [ ] fn `suprnova::rate_limit::algorithm::Bucket::evict_old` · framework/src/rate_limit/algorithm.rs:24
  - [ ] fn `suprnova::rate_limit::algorithm::Bucket::try_record` · framework/src/rate_limit/algorithm.rs:37
  - [ ] fn `suprnova::rate_limit::algorithm::Bucket::retry_after` · framework/src/rate_limit/algorithm.rs:53
  - [ ] fn `suprnova::rate_limit::algorithm::Bucket::is_inactive` · framework/src/rate_limit/algorithm.rs:75

### `suprnova::rate_limit::laravel`

- [ ] fn `suprnova::rate_limit::laravel::registry` · framework/src/rate_limit/laravel.rs:133
- [ ] struct `suprnova::rate_limit::NamedLimiterRegistry` · framework/src/rate_limit/laravel.rs:78 (also `suprnova::rate_limit::laravel::NamedLimiterRegistry`)
  - [ ] fn `suprnova::rate_limit::NamedLimiterRegistry::new` · framework/src/rate_limit/laravel.rs:92
  - [ ] fn `suprnova::rate_limit::NamedLimiterRegistry::insert` · framework/src/rate_limit/laravel.rs:100
  - [ ] fn `suprnova::rate_limit::NamedLimiterRegistry::get` · framework/src/rate_limit/laravel.rs:116
  - [ ] fn `suprnova::rate_limit::NamedLimiterRegistry::has` · framework/src/rate_limit/laravel.rs:122
- [ ] struct `suprnova::RateLimiter` · framework/src/rate_limit/laravel.rs:176 (also `suprnova::rate_limit::RateLimiter`, `suprnova::rate_limit::laravel::RateLimiter`)
  - [ ] fn `suprnova::RateLimiter::define` · framework/src/rate_limit/laravel.rs:206
  - [ ] fn `suprnova::RateLimiter::for` · framework/src/rate_limit/laravel.rs:216
  - [ ] fn `suprnova::RateLimiter::limiter` · framework/src/rate_limit/laravel.rs:226
  - [ ] fn `suprnova::RateLimiter::has_limiter` · framework/src/rate_limit/laravel.rs:231
  - [ ] fn `suprnova::RateLimiter::available_in` · framework/src/rate_limit/laravel.rs:242
  - [ ] fn `suprnova::RateLimiter::attempts` · framework/src/rate_limit/laravel.rs:256
  - [ ] fn `suprnova::RateLimiter::reset_attempts` · framework/src/rate_limit/laravel.rs:265
  - [ ] fn `suprnova::RateLimiter::too_many_attempts` · framework/src/rate_limit/laravel.rs:280
  - [ ] fn `suprnova::RateLimiter::hit` · framework/src/rate_limit/laravel.rs:296
  - [ ] fn `suprnova::RateLimiter::hit_and_check` · framework/src/rate_limit/laravel.rs:322
  - [ ] fn `suprnova::RateLimiter::increment` · framework/src/rate_limit/laravel.rs:341
  - [ ] fn `suprnova::RateLimiter::decrement` · framework/src/rate_limit/laravel.rs:364
  - [ ] fn `suprnova::RateLimiter::remaining` · framework/src/rate_limit/laravel.rs:380
  - [ ] fn `suprnova::RateLimiter::retries_left` · framework/src/rate_limit/laravel.rs:388
  - [ ] fn `suprnova::RateLimiter::clear` · framework/src/rate_limit/laravel.rs:395
  - [ ] fn `suprnova::RateLimiter::clean_rate_limiter_key` · framework/src/rate_limit/laravel.rs:413
  - [ ] fn `suprnova::RateLimiter::attempt` · framework/src/rate_limit/laravel.rs:456
  - [ ] fn `suprnova::RateLimiter::is_cache_initialized` · framework/src/rate_limit/laravel.rs:478

### `suprnova::rate_limit::limit`

- [ ] struct `suprnova::GlobalLimit` · framework/src/rate_limit/limit.rs:231 (also `suprnova::rate_limit::GlobalLimit`, `suprnova::rate_limit::limit::GlobalLimit`)
  - Public tuple fields: 1
  - [ ] fn `suprnova::GlobalLimit::new` · framework/src/rate_limit/limit.rs:235
  - [ ] fn `suprnova::GlobalLimit::per_minute` · framework/src/rate_limit/limit.rs:240
  - [ ] fn `suprnova::GlobalLimit::per_hour` · framework/src/rate_limit/limit.rs:245
- [ ] struct `suprnova::Limit` · framework/src/rate_limit/limit.rs:84 (also `suprnova::rate_limit::Limit`, `suprnova::rate_limit::limit::Limit`)
  - Public fields: `key`, `max_attempts`, `decay`, `after_callback`, `response_callback`
  - [ ] fn `suprnova::Limit::new` · framework/src/rate_limit/limit.rs:108
  - [ ] fn `suprnova::Limit::per_second` · framework/src/rate_limit/limit.rs:120
  - [ ] fn `suprnova::Limit::per_minute` · framework/src/rate_limit/limit.rs:125
  - [ ] fn `suprnova::Limit::per_minutes` · framework/src/rate_limit/limit.rs:132
  - [ ] fn `suprnova::Limit::per_hour` · framework/src/rate_limit/limit.rs:137
  - [ ] fn `suprnova::Limit::per_hours` · framework/src/rate_limit/limit.rs:142
  - [ ] fn `suprnova::Limit::per_day` · framework/src/rate_limit/limit.rs:147
  - [ ] fn `suprnova::Limit::per_days` · framework/src/rate_limit/limit.rs:152
  - [ ] fn `suprnova::Limit::none` · framework/src/rate_limit/limit.rs:159
  - [ ] fn `suprnova::Limit::by` · framework/src/rate_limit/limit.rs:172
  - [ ] fn `suprnova::Limit::after` · framework/src/rate_limit/limit.rs:180
  - [ ] fn `suprnova::Limit::response` · framework/src/rate_limit/limit.rs:190
  - [ ] fn `suprnova::Limit::decay_seconds` · framework/src/rate_limit/limit.rs:201
  - [ ] fn `suprnova::Limit::fallback_key` · framework/src/rate_limit/limit.rs:209
- [ ] struct `suprnova::Unlimited` · framework/src/rate_limit/limit.rs:271 (also `suprnova::rate_limit::Unlimited`, `suprnova::rate_limit::limit::Unlimited`)
  - Public tuple fields: 1
  - [ ] fn `suprnova::Unlimited::new` · framework/src/rate_limit/limit.rs:276
- [ ] enum `suprnova::LimitResult` · framework/src/rate_limit/limit.rs:47 (also `suprnova::rate_limit::LimitResult`, `suprnova::rate_limit::limit::LimitResult`)
  - Variants: `Single`, `Many`, `Response`
- [ ] type `suprnova::rate_limit::limit::AfterCallback` · framework/src/rate_limit/limit.rs:33
- [ ] type `suprnova::rate_limit::limit::ResponseCallback` · framework/src/rate_limit/limit.rs:37

### `suprnova::rate_limit::memory`

- [ ] struct `suprnova::rate_limit::memory::InMemoryRateLimiter` · framework/src/rate_limit/memory.rs:41
  - Implements: `suprnova::RateLimiterDriver`
  - [ ] fn `suprnova::rate_limit::memory::InMemoryRateLimiter::new` · framework/src/rate_limit/memory.rs:51
  - [ ] fn `suprnova::rate_limit::memory::InMemoryRateLimiter::with_periodic_sweep` · framework/src/rate_limit/memory.rs:74
  - [ ] fn `suprnova::rate_limit::memory::InMemoryRateLimiter::purge_inactive` · framework/src/rate_limit/memory.rs:115
  - [ ] fn `suprnova::rate_limit::memory::InMemoryRateLimiter::bucket_count` · framework/src/rate_limit/memory.rs:127

### `suprnova::rate_limit::redis`

- [ ] struct `suprnova::rate_limit::redis::RedisRateLimiter` · framework/src/rate_limit/redis.rs:14
  - Implements: `suprnova::RateLimiterDriver`
  - [ ] fn `suprnova::rate_limit::redis::RedisRateLimiter::connect` · framework/src/rate_limit/redis.rs:22

### `suprnova::rate_limit::throttle`

- [ ] struct `suprnova::ThrottleRequestsMiddleware` · framework/src/rate_limit/throttle.rs:48 (also `suprnova::rate_limit::ThrottleRequestsMiddleware`, `suprnova::rate_limit::throttle::ThrottleRequestsMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::ThrottleRequestsMiddleware::by_name` · framework/src/rate_limit/throttle.rs:73
  - [ ] fn `suprnova::ThrottleRequestsMiddleware::with` · framework/src/rate_limit/throttle.rs:83
  - [ ] fn `suprnova::ThrottleRequestsMiddleware::with_limits` · framework/src/rate_limit/throttle.rs:97
  - [ ] fn `suprnova::ThrottleRequestsMiddleware::prefix` · framework/src/rate_limit/throttle.rs:107

## rbac

### `suprnova::rbac::entity`

- [ ] struct `suprnova::rbac::entity::ModelPermission` · framework/src/rbac/entity.rs:67
  - Public fields: `id`, `model_type`, `model_id`, `permission_id`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`
  - [ ] fn `suprnova::rbac::entity::ModelPermission::fill` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::without_global_scope` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::without_global_scopes` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::on` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::on_write_connection` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::count` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::sum` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::avg` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::min` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::max` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::pluck` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::pluck_keyed` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::filter` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::db_where` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::where_in` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::where_like` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::latest` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::oldest` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::pivot` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::with` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::with_count` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::with_sum` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::with_avg` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::with_min` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::with_max` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermission::observe` · framework/src/rbac/entity.rs:66
- [ ] struct `suprnova::rbac::entity::ModelRole` · framework/src/rbac/entity.rs:54
  - Public fields: `id`, `model_type`, `model_id`, `role_id`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`
  - [ ] fn `suprnova::rbac::entity::ModelRole::fill` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::without_global_scope` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::without_global_scopes` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::on` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::on_write_connection` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::count` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::sum` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::avg` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::min` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::max` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::pluck` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::pluck_keyed` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::filter` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::db_where` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::where_in` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::where_like` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::latest` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::oldest` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::pivot` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::with` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::with_count` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::with_sum` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::with_avg` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::with_min` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::with_max` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRole::observe` · framework/src/rbac/entity.rs:53
- [ ] struct `suprnova::rbac::entity::Permission` · framework/src/rbac/entity.rs:26
  - Public fields: `id`, `name`, `display_name`, `guard_name`, `created_at`, `updated_at`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`, `suprnova::Touchable`
  - [ ] fn `suprnova::rbac::entity::Permission::fill` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::without_global_scope` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::without_global_scopes` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::on` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::on_write_connection` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::count` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::sum` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::avg` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::min` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::max` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::pluck` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::pluck_keyed` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::filter` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::db_where` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::where_in` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::where_like` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::latest` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::oldest` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::pivot` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::with` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::with_count` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::with_sum` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::with_avg` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::with_min` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::with_max` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::Permission::observe` · framework/src/rbac/entity.rs:25
- [ ] struct `suprnova::rbac::entity::Role` · framework/src/rbac/entity.rs:8
  - Public fields: `id`, `name`, `display_name`, `guard_name`, `created_at`, `updated_at`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`, `suprnova::Touchable`
  - [ ] fn `suprnova::rbac::entity::Role::fill` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::without_global_scope` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::without_global_scopes` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::on` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::on_write_connection` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::count` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::sum` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::avg` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::min` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::max` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::pluck` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::pluck_keyed` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::filter` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::db_where` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::where_in` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::where_like` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::latest` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::oldest` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::pivot` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::with` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::with_count` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::with_sum` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::with_avg` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::with_min` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::with_max` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::Role::observe` · framework/src/rbac/entity.rs:7
- [ ] struct `suprnova::rbac::entity::RolePermission` · framework/src/rbac/entity.rs:43
  - Public fields: `id`, `role_id`, `permission_id`
  - Implements: `suprnova::EagerLoadDispatch`, `suprnova::EloquentModel`, `suprnova::FirstOrCreate`, `suprnova::Model`, `suprnova::ModelEventHooks`, `suprnova::Persistable`, `suprnova::ReplicateExt`
  - [ ] fn `suprnova::rbac::entity::RolePermission::fill` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::without_global_scope` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::without_global_scopes` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::on` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::on_write_connection` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::count` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::sum` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::avg` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::min` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::max` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::pluck` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::pluck_keyed` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::filter` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::db_where` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::where_in` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::where_like` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::latest` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::oldest` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::pivot` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::with` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::with_count` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::with_sum` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::with_avg` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::with_min` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::with_max` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermission::observe` · framework/src/rbac/entity.rs:42

### `suprnova::rbac::entity::model_permission`

- [ ] struct `suprnova::rbac::entity::model_permission::ColumnIter` · framework/src/rbac/entity.rs:66
- [ ] struct `suprnova::rbac::entity::ModelPermissionActiveModel` · framework/src/rbac/entity.rs:66 (also `suprnova::rbac::entity::model_permission::ActiveModel`)
  - Public fields: `id`, `model_type`, `model_id`, `permission_id`
- [ ] struct `suprnova::rbac::entity::ModelPermissionEntity` · framework/src/rbac/entity.rs:66 (also `suprnova::rbac::entity::model_permission::Entity`)
- [ ] struct `suprnova::rbac::entity::ModelPermissionModel` · framework/src/rbac/entity.rs:66 (also `suprnova::rbac::entity::model_permission::Model`)
  - Public fields: `id`, `model_type`, `model_id`, `permission_id`
  - [ ] fn `suprnova::rbac::entity::ModelPermissionModel::into_ex` · framework/src/rbac/entity.rs:66
- [ ] struct `suprnova::rbac::entity::model_permission::PrimaryKeyIter` · framework/src/rbac/entity.rs:66
- [ ] struct `suprnova::rbac::entity::model_permission::RelationIter` · framework/src/rbac/entity.rs:66
- [ ] enum `suprnova::rbac::entity::ModelPermissionColumn` · framework/src/rbac/entity.rs:66 (also `suprnova::rbac::entity::model_permission::Column`)
  - Variants: `Id`, `ModelType`, `ModelId`, `PermissionId`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::rbac::entity::ModelPermissionColumn::as_str` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermissionColumn::from_name` · framework/src/rbac/entity.rs:66
  - [ ] fn `suprnova::rbac::entity::ModelPermissionColumn::iter` · framework/src/rbac/entity.rs:66
- [ ] enum `suprnova::rbac::entity::model_permission::PrimaryKey` · framework/src/rbac/entity.rs:66
  - Variants: `Id`
- [ ] enum `suprnova::rbac::entity::model_permission::Relation` · framework/src/rbac/entity.rs:66

### `suprnova::rbac::entity::model_permission::events`

- [ ] struct `suprnova::rbac::entity::model_permission::events::Created` · framework/src/rbac/entity.rs:66
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Creating` · framework/src/rbac/entity.rs:66
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Deleted` · framework/src/rbac/entity.rs:66
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Deleting` · framework/src/rbac/entity.rs:66
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::ForceDeleted` · framework/src/rbac/entity.rs:66
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::ForceDeleting` · framework/src/rbac/entity.rs:66
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Replicating` · framework/src/rbac/entity.rs:66
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Restored` · framework/src/rbac/entity.rs:66
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Restoring` · framework/src/rbac/entity.rs:66
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Retrieved` · framework/src/rbac/entity.rs:66
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Retrieving` · framework/src/rbac/entity.rs:66
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Saved` · framework/src/rbac/entity.rs:66
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Saving` · framework/src/rbac/entity.rs:66
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Trashed` · framework/src/rbac/entity.rs:66
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Updated` · framework/src/rbac/entity.rs:66
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_permission::events::Updating` · framework/src/rbac/entity.rs:66
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::rbac::entity::model_role`

- [ ] struct `suprnova::rbac::entity::model_role::ColumnIter` · framework/src/rbac/entity.rs:53
- [ ] struct `suprnova::rbac::entity::ModelRoleActiveModel` · framework/src/rbac/entity.rs:53 (also `suprnova::rbac::entity::model_role::ActiveModel`)
  - Public fields: `id`, `model_type`, `model_id`, `role_id`
- [ ] struct `suprnova::rbac::entity::ModelRoleEntity` · framework/src/rbac/entity.rs:53 (also `suprnova::rbac::entity::model_role::Entity`)
- [ ] struct `suprnova::rbac::entity::ModelRoleModel` · framework/src/rbac/entity.rs:53 (also `suprnova::rbac::entity::model_role::Model`)
  - Public fields: `id`, `model_type`, `model_id`, `role_id`
  - [ ] fn `suprnova::rbac::entity::ModelRoleModel::into_ex` · framework/src/rbac/entity.rs:53
- [ ] struct `suprnova::rbac::entity::model_role::PrimaryKeyIter` · framework/src/rbac/entity.rs:53
- [ ] struct `suprnova::rbac::entity::model_role::RelationIter` · framework/src/rbac/entity.rs:53
- [ ] enum `suprnova::rbac::entity::ModelRoleColumn` · framework/src/rbac/entity.rs:53 (also `suprnova::rbac::entity::model_role::Column`)
  - Variants: `Id`, `ModelType`, `ModelId`, `RoleId`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::rbac::entity::ModelRoleColumn::as_str` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRoleColumn::from_name` · framework/src/rbac/entity.rs:53
  - [ ] fn `suprnova::rbac::entity::ModelRoleColumn::iter` · framework/src/rbac/entity.rs:53
- [ ] enum `suprnova::rbac::entity::model_role::PrimaryKey` · framework/src/rbac/entity.rs:53
  - Variants: `Id`
- [ ] enum `suprnova::rbac::entity::model_role::Relation` · framework/src/rbac/entity.rs:53

### `suprnova::rbac::entity::model_role::events`

- [ ] struct `suprnova::rbac::entity::model_role::events::Created` · framework/src/rbac/entity.rs:53
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Creating` · framework/src/rbac/entity.rs:53
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Deleted` · framework/src/rbac/entity.rs:53
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Deleting` · framework/src/rbac/entity.rs:53
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::ForceDeleted` · framework/src/rbac/entity.rs:53
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::ForceDeleting` · framework/src/rbac/entity.rs:53
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Replicating` · framework/src/rbac/entity.rs:53
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Restored` · framework/src/rbac/entity.rs:53
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Restoring` · framework/src/rbac/entity.rs:53
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Retrieved` · framework/src/rbac/entity.rs:53
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Retrieving` · framework/src/rbac/entity.rs:53
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Saved` · framework/src/rbac/entity.rs:53
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Saving` · framework/src/rbac/entity.rs:53
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Trashed` · framework/src/rbac/entity.rs:53
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Updated` · framework/src/rbac/entity.rs:53
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::model_role::events::Updating` · framework/src/rbac/entity.rs:53
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::rbac::entity::permission`

- [ ] struct `suprnova::rbac::entity::permission::ColumnIter` · framework/src/rbac/entity.rs:25
- [ ] struct `suprnova::rbac::entity::PermissionActiveModel` · framework/src/rbac/entity.rs:25 (also `suprnova::rbac::entity::permission::ActiveModel`)
  - Public fields: `id`, `name`, `display_name`, `guard_name`, `created_at`, `updated_at`
- [ ] struct `suprnova::rbac::entity::PermissionEntity` · framework/src/rbac/entity.rs:25 (also `suprnova::rbac::entity::permission::Entity`)
- [ ] struct `suprnova::rbac::entity::PermissionModel` · framework/src/rbac/entity.rs:25 (also `suprnova::rbac::entity::permission::Model`)
  - Public fields: `id`, `name`, `display_name`, `guard_name`, `created_at`, `updated_at`
  - [ ] fn `suprnova::rbac::entity::PermissionModel::into_ex` · framework/src/rbac/entity.rs:25
- [ ] struct `suprnova::rbac::entity::permission::PrimaryKeyIter` · framework/src/rbac/entity.rs:25
- [ ] struct `suprnova::rbac::entity::permission::RelationIter` · framework/src/rbac/entity.rs:25
- [ ] enum `suprnova::rbac::entity::PermissionColumn` · framework/src/rbac/entity.rs:25 (also `suprnova::rbac::entity::permission::Column`)
  - Variants: `Id`, `Name`, `DisplayName`, `GuardName`, `CreatedAt`, `UpdatedAt`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::rbac::entity::PermissionColumn::as_str` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::PermissionColumn::from_name` · framework/src/rbac/entity.rs:25
  - [ ] fn `suprnova::rbac::entity::PermissionColumn::iter` · framework/src/rbac/entity.rs:25
- [ ] enum `suprnova::rbac::entity::permission::PrimaryKey` · framework/src/rbac/entity.rs:25
  - Variants: `Id`
- [ ] enum `suprnova::rbac::entity::permission::Relation` · framework/src/rbac/entity.rs:25
- [ ] type `suprnova::rbac::entity::permission::__Suprnova_Cast_Storage_created_at` · framework/src/rbac/entity.rs:25
- [ ] type `suprnova::rbac::entity::permission::__Suprnova_Cast_Storage_updated_at` · framework/src/rbac/entity.rs:25

### `suprnova::rbac::entity::permission::events`

- [ ] struct `suprnova::rbac::entity::permission::events::Created` · framework/src/rbac/entity.rs:25
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Creating` · framework/src/rbac/entity.rs:25
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Deleted` · framework/src/rbac/entity.rs:25
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Deleting` · framework/src/rbac/entity.rs:25
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::ForceDeleted` · framework/src/rbac/entity.rs:25
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::ForceDeleting` · framework/src/rbac/entity.rs:25
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Replicating` · framework/src/rbac/entity.rs:25
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Restored` · framework/src/rbac/entity.rs:25
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Restoring` · framework/src/rbac/entity.rs:25
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Retrieved` · framework/src/rbac/entity.rs:25
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Retrieving` · framework/src/rbac/entity.rs:25
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Saved` · framework/src/rbac/entity.rs:25
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Saving` · framework/src/rbac/entity.rs:25
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Trashed` · framework/src/rbac/entity.rs:25
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Updated` · framework/src/rbac/entity.rs:25
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::permission::events::Updating` · framework/src/rbac/entity.rs:25
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::rbac::entity::role`

- [ ] struct `suprnova::rbac::entity::role::ColumnIter` · framework/src/rbac/entity.rs:7
- [ ] struct `suprnova::rbac::entity::role::PrimaryKeyIter` · framework/src/rbac/entity.rs:7
- [ ] struct `suprnova::rbac::entity::role::RelationIter` · framework/src/rbac/entity.rs:7
- [ ] struct `suprnova::rbac::entity::RoleActiveModel` · framework/src/rbac/entity.rs:7 (also `suprnova::rbac::entity::role::ActiveModel`)
  - Public fields: `id`, `name`, `display_name`, `guard_name`, `created_at`, `updated_at`
- [ ] struct `suprnova::rbac::entity::RoleEntity` · framework/src/rbac/entity.rs:7 (also `suprnova::rbac::entity::role::Entity`)
- [ ] struct `suprnova::rbac::entity::RoleModel` · framework/src/rbac/entity.rs:7 (also `suprnova::rbac::entity::role::Model`)
  - Public fields: `id`, `name`, `display_name`, `guard_name`, `created_at`, `updated_at`
  - [ ] fn `suprnova::rbac::entity::RoleModel::into_ex` · framework/src/rbac/entity.rs:7
- [ ] enum `suprnova::rbac::entity::role::PrimaryKey` · framework/src/rbac/entity.rs:7
  - Variants: `Id`
- [ ] enum `suprnova::rbac::entity::role::Relation` · framework/src/rbac/entity.rs:7
- [ ] enum `suprnova::rbac::entity::RoleColumn` · framework/src/rbac/entity.rs:7 (also `suprnova::rbac::entity::role::Column`)
  - Variants: `Id`, `Name`, `DisplayName`, `GuardName`, `CreatedAt`, `UpdatedAt`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::rbac::entity::RoleColumn::as_str` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::RoleColumn::from_name` · framework/src/rbac/entity.rs:7
  - [ ] fn `suprnova::rbac::entity::RoleColumn::iter` · framework/src/rbac/entity.rs:7
- [ ] type `suprnova::rbac::entity::role::__Suprnova_Cast_Storage_created_at` · framework/src/rbac/entity.rs:7
- [ ] type `suprnova::rbac::entity::role::__Suprnova_Cast_Storage_updated_at` · framework/src/rbac/entity.rs:7

### `suprnova::rbac::entity::role::events`

- [ ] struct `suprnova::rbac::entity::role::events::Created` · framework/src/rbac/entity.rs:7
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Creating` · framework/src/rbac/entity.rs:7
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Deleted` · framework/src/rbac/entity.rs:7
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Deleting` · framework/src/rbac/entity.rs:7
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::ForceDeleted` · framework/src/rbac/entity.rs:7
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::ForceDeleting` · framework/src/rbac/entity.rs:7
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Replicating` · framework/src/rbac/entity.rs:7
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Restored` · framework/src/rbac/entity.rs:7
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Restoring` · framework/src/rbac/entity.rs:7
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Retrieved` · framework/src/rbac/entity.rs:7
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Retrieving` · framework/src/rbac/entity.rs:7
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Saved` · framework/src/rbac/entity.rs:7
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Saving` · framework/src/rbac/entity.rs:7
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Trashed` · framework/src/rbac/entity.rs:7
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Updated` · framework/src/rbac/entity.rs:7
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role::events::Updating` · framework/src/rbac/entity.rs:7
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::rbac::entity::role_permission`

- [ ] struct `suprnova::rbac::entity::role_permission::ColumnIter` · framework/src/rbac/entity.rs:42
- [ ] struct `suprnova::rbac::entity::role_permission::PrimaryKeyIter` · framework/src/rbac/entity.rs:42
- [ ] struct `suprnova::rbac::entity::role_permission::RelationIter` · framework/src/rbac/entity.rs:42
- [ ] struct `suprnova::rbac::entity::RolePermissionActiveModel` · framework/src/rbac/entity.rs:42 (also `suprnova::rbac::entity::role_permission::ActiveModel`)
  - Public fields: `id`, `role_id`, `permission_id`
- [ ] struct `suprnova::rbac::entity::RolePermissionEntity` · framework/src/rbac/entity.rs:42 (also `suprnova::rbac::entity::role_permission::Entity`)
- [ ] struct `suprnova::rbac::entity::RolePermissionModel` · framework/src/rbac/entity.rs:42 (also `suprnova::rbac::entity::role_permission::Model`)
  - Public fields: `id`, `role_id`, `permission_id`
  - [ ] fn `suprnova::rbac::entity::RolePermissionModel::into_ex` · framework/src/rbac/entity.rs:42
- [ ] enum `suprnova::rbac::entity::role_permission::PrimaryKey` · framework/src/rbac/entity.rs:42
  - Variants: `Id`
- [ ] enum `suprnova::rbac::entity::role_permission::Relation` · framework/src/rbac/entity.rs:42
- [ ] enum `suprnova::rbac::entity::RolePermissionColumn` · framework/src/rbac/entity.rs:42 (also `suprnova::rbac::entity::role_permission::Column`)
  - Variants: `Id`, `RoleId`, `PermissionId`
  - Implements: `suprnova::IntoColumn`
  - [ ] fn `suprnova::rbac::entity::RolePermissionColumn::as_str` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermissionColumn::from_name` · framework/src/rbac/entity.rs:42
  - [ ] fn `suprnova::rbac::entity::RolePermissionColumn::iter` · framework/src/rbac/entity.rs:42

### `suprnova::rbac::entity::role_permission::events`

- [ ] struct `suprnova::rbac::entity::role_permission::events::Created` · framework/src/rbac/entity.rs:42
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Creating` · framework/src/rbac/entity.rs:42
  - Public fields: `attrs`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Deleted` · framework/src/rbac/entity.rs:42
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Deleting` · framework/src/rbac/entity.rs:42
  - Public fields: `model`, `is_force`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::ForceDeleted` · framework/src/rbac/entity.rs:42
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::ForceDeleting` · framework/src/rbac/entity.rs:42
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Replicating` · framework/src/rbac/entity.rs:42
  - Public fields: `source`, `replica`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Restored` · framework/src/rbac/entity.rs:42
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Restoring` · framework/src/rbac/entity.rs:42
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Retrieved` · framework/src/rbac/entity.rs:42
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Retrieving` · framework/src/rbac/entity.rs:42
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Saved` · framework/src/rbac/entity.rs:42
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Saving` · framework/src/rbac/entity.rs:42
  - Public fields: `attrs`, `is_creating`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Trashed` · framework/src/rbac/entity.rs:42
  - Public fields: `model`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Updated` · framework/src/rbac/entity.rs:42
  - Public fields: `previous`, `current`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::rbac::entity::role_permission::events::Updating` · framework/src/rbac/entity.rs:42
  - Public fields: `previous`, `attrs`
  - Implements: `suprnova::Event`

### `suprnova::rbac::has_roles` (private module; items are public through re-exports)

- [ ] fn `suprnova::rbac::assign_role_to_model` · framework/src/rbac/has_roles.rs:417
- [ ] fn `suprnova::rbac::assign_role_to_model_on_guard` · framework/src/rbac/has_roles.rs:429
- [ ] fn `suprnova::rbac::create_permission` · framework/src/rbac/has_roles.rs:304
- [ ] fn `suprnova::rbac::create_permission_on_guard` · framework/src/rbac/has_roles.rs:312
- [ ] fn `suprnova::rbac::create_role` · framework/src/rbac/has_roles.rs:278
- [ ] fn `suprnova::rbac::create_role_on_guard` · framework/src/rbac/has_roles.rs:286
- [ ] fn `suprnova::rbac::give_permission_to_model` · framework/src/rbac/has_roles.rs:510
- [ ] fn `suprnova::rbac::give_permission_to_model_on_guard` · framework/src/rbac/has_roles.rs:521
- [ ] fn `suprnova::rbac::give_permission_to_role` · framework/src/rbac/has_roles.rs:332
- [ ] fn `suprnova::rbac::give_permission_to_role_on_guard` · framework/src/rbac/has_roles.rs:342
- [ ] fn `suprnova::rbac::has_permission_for_model` · framework/src/rbac/has_roles.rs:624
- [ ] fn `suprnova::rbac::has_permission_for_model_on_guard` · framework/src/rbac/has_roles.rs:636
- [ ] fn `suprnova::rbac::has_role_for_model` · framework/src/rbac/has_roles.rs:593
- [ ] fn `suprnova::rbac::has_role_for_model_on_guard` · framework/src/rbac/has_roles.rs:602
- [ ] fn `suprnova::rbac::remove_permission_from_model` · framework/src/rbac/has_roles.rs:560
- [ ] fn `suprnova::rbac::remove_permission_from_model_on_guard` · framework/src/rbac/has_roles.rs:578
- [ ] fn `suprnova::rbac::remove_permission_from_role` · framework/src/rbac/has_roles.rs:380
- [ ] fn `suprnova::rbac::remove_permission_from_role_on_guard` · framework/src/rbac/has_roles.rs:398
- [ ] fn `suprnova::rbac::remove_role_from_model` · framework/src/rbac/has_roles.rs:466
- [ ] fn `suprnova::rbac::remove_role_from_model_on_guard` · framework/src/rbac/has_roles.rs:492
- [ ] trait `suprnova::HasRoles` · framework/src/rbac/has_roles.rs:690 (also `suprnova::rbac::HasRoles`)
  - [ ] fn `suprnova::HasRoles::rbac_model_type` · framework/src/rbac/has_roles.rs:693 (provided)
  - [ ] fn `suprnova::HasRoles::rbac_model_id` · framework/src/rbac/has_roles.rs:699 (provided)
  - [ ] fn `suprnova::HasRoles::assign_role` · framework/src/rbac/has_roles.rs:704 (provided)
  - [ ] fn `suprnova::HasRoles::give_permission_to` · framework/src/rbac/has_roles.rs:709 (provided)
  - [ ] fn `suprnova::HasRoles::remove_role` · framework/src/rbac/has_roles.rs:728 (provided)
  - [ ] fn `suprnova::HasRoles::remove_permission_to` · framework/src/rbac/has_roles.rs:744 (provided)
  - [ ] fn `suprnova::HasRoles::has_role` · framework/src/rbac/has_roles.rs:754 (provided)
  - [ ] fn `suprnova::HasRoles::has_permission_to` · framework/src/rbac/has_roles.rs:761 (provided)

### `suprnova::rbac::middleware` (private module; items are public through re-exports)

- [ ] struct `suprnova::PermissionMiddleware` · framework/src/rbac/middleware.rs:84 (also `suprnova::rbac::PermissionMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::PermissionMiddleware::new` · framework/src/rbac/middleware.rs:92
  - [ ] fn `suprnova::PermissionMiddleware::redirect_to` · framework/src/rbac/middleware.rs:104
- [ ] struct `suprnova::RoleMiddleware` · framework/src/rbac/middleware.rs:29 (also `suprnova::rbac::RoleMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::RoleMiddleware::new` · framework/src/rbac/middleware.rs:37
  - [ ] fn `suprnova::RoleMiddleware::redirect_to` · framework/src/rbac/middleware.rs:49

### `suprnova::rbac::migrations::m_create_rbac_tables`

- [ ] struct `suprnova::rbac::migrations::CreateRbacTables` · framework/src/rbac/migrations/m_create_rbac_tables.rs:6 (also `suprnova::rbac::migrations::m_create_rbac_tables::Migration`)

## render_cache

### `suprnova::render_cache`

- [ ] struct `suprnova::RenderCache` · framework/src/render_cache/mod.rs:331 (also `suprnova::render_cache::RenderCache`)
  - [ ] fn `suprnova::RenderCache::install` · framework/src/render_cache/mod.rs:490
  - [ ] fn `suprnova::RenderCache::bump_permission_version` · framework/src/render_cache/mod.rs:670
  - [ ] fn `suprnova::RenderCache::advance_epoch` · framework/src/render_cache/mod.rs:720
  - [ ] fn `suprnova::RenderCache::inspect` · framework/src/render_cache/mod.rs:738
  - [ ] fn `suprnova::RenderCache::store_inspection` · framework/src/render_cache/mod.rs:760
  - [ ] fn `suprnova::RenderCache::sweep` · framework/src/render_cache/mod.rs:793
- [ ] struct `suprnova::render_cache::StoreInspection` · framework/src/render_cache/mod.rs:174
  - Public fields: `entries`, `bytes`, `epoch`
- [ ] enum `suprnova::render_cache::L1Provider` · framework/src/render_cache/mod.rs:103
  - Variants: `File`, `Database`, `Redis`
  - Implements: `suprnova_live::render_cache::RenderStore`

### `suprnova::render_cache::collector`

- [ ] fn `suprnova::render_cache::collector::begin_authorization_decision` · framework/src/render_cache/collector.rs:769
- [ ] fn `suprnova::render_cache::collector::current_report` · framework/src/render_cache/collector.rs:986
- [ ] fn `suprnova::render_cache::collector::end_authorization_decision` · framework/src/render_cache/collector.rs:787
- [ ] fn `suprnova::render_cache::collector::is_active` · framework/src/render_cache/collector.rs:487
- [ ] fn `suprnova::render_cache::collector::observe` · framework/src/render_cache/collector.rs:530
- [ ] fn `suprnova::render_cache::collector::observe_feature_read` · framework/src/render_cache/collector.rs:683
- [ ] fn `suprnova::render_cache::collector::observe_foreign_connection_read` · framework/src/render_cache/collector.rs:748
- [ ] fn `suprnova::render_cache::collector::observe_live_document_bootstrap_nonce` · framework/src/render_cache/collector.rs:919
- [ ] fn `suprnova::render_cache::collector::observe_live_document_digest` · framework/src/render_cache/collector.rs:943
- [ ] fn `suprnova::render_cache::collector::observe_live_document_mount` · framework/src/render_cache/collector.rs:834
- [ ] fn `suprnova::render_cache::collector::observe_live_document_no_store` · framework/src/render_cache/collector.rs:862
- [ ] fn `suprnova::render_cache::collector::observe_live_document_shell_island` · framework/src/render_cache/collector.rs:897
- [ ] fn `suprnova::render_cache::collector::observe_live_document_stitch_invalid` · framework/src/render_cache/collector.rs:961
- [ ] fn `suprnova::render_cache::collector::observe_live_document_stitch_slot` · framework/src/render_cache/collector.rs:878
- [ ] fn `suprnova::render_cache::collector::observe_locale_value` · framework/src/render_cache/collector.rs:734
- [ ] fn `suprnova::render_cache::collector::observe_principal_read` · framework/src/render_cache/collector.rs:694
- [ ] fn `suprnova::render_cache::collector::observe_principal_value` · framework/src/render_cache/collector.rs:705
- [ ] fn `suprnova::render_cache::collector::observe_record_read` · framework/src/render_cache/collector.rs:632
- [ ] fn `suprnova::render_cache::collector::observe_record_read_json` · framework/src/render_cache/collector.rs:664
- [ ] fn `suprnova::render_cache::collector::observe_secret_context_read` · framework/src/render_cache/collector.rs:825
- [ ] fn `suprnova::render_cache::collector::observe_session_read` · framework/src/render_cache/collector.rs:741
- [ ] fn `suprnova::render_cache::collector::observe_table_read` · framework/src/render_cache/collector.rs:590
- [ ] fn `suprnova::render_cache::collector::observe_tenant_read` · framework/src/render_cache/collector.rs:713
- [ ] fn `suprnova::render_cache::collector::observe_tenant_value` · framework/src/render_cache/collector.rs:721
- [ ] fn `suprnova::render_cache::collector::observe_undeclared` · framework/src/render_cache/collector.rs:973
- [ ] fn `suprnova::render_cache::collector::observe_unkeyed_write` · framework/src/render_cache/collector.rs:608
- [ ] fn `suprnova::render_cache::collector::observe_unobservable_read` · framework/src/render_cache/collector.rs:577
- [ ] fn `suprnova::render_cache::collector::permission_version_identity` · framework/src/render_cache/collector.rs:119
- [ ] fn `suprnova::render_cache::collector::record_identity` · framework/src/render_cache/collector.rs:657
- [ ] fn `suprnova::render_cache::collector::resolvable_reads` · framework/src/render_cache/collector.rs:816
- [ ] fn `suprnova::render_cache::collector::strip_classification_reasons_for_test` · framework/src/render_cache/collector.rs:1019 (feature: `testing`)
- [ ] struct `suprnova::render_cache::collector::CollectedContext` · framework/src/render_cache/collector.rs:140
  - Public fields: `principal_read`, `principal_material`, `tenant_read`, `tenant_material`, `locale_material`, `session_read`, `principal_reads`, `tenant_reads`, `locale_reads`, `authorization`, `secret_context_read`, `foreign_connection_read`, `overflowed`
- [ ] struct `suprnova::render_cache::collector::Collector` · framework/src/render_cache/collector.rs:379
  - [ ] fn `suprnova::render_cache::collector::Collector::scope` · framework/src/render_cache/collector.rs:390
- [ ] struct `suprnova::render_cache::collector::CollectorReport` · framework/src/render_cache/collector.rs:251
  - Public fields: `observed`, `context`, `gate`, `slot_reads`, `handler_began`, `undeclared`, `live_document`, `strip_classification_reasons`
  - [ ] fn `suprnova::render_cache::collector::CollectorReport::storable` · framework/src/render_cache/collector.rs:304
  - [ ] fn `suprnova::render_cache::collector::CollectorReport::fold_gate_into_content` · framework/src/render_cache/collector.rs:321
- [ ] struct `suprnova::render_cache::collector::ConsultWindow` · framework/src/render_cache/collector.rs:755
- [ ] struct `suprnova::render_cache::collector::GateReport` · framework/src/render_cache/collector.rs:242
  - Public fields: `context`, `observed`
- [ ] const `suprnova::render_cache::collector::PERMISSION_VERSION_CONFIG_KEY` · framework/src/render_cache/collector.rs:94

### `suprnova::render_cache::config`

- [ ] struct `suprnova::render_cache::L0Limits` · framework/src/render_cache/config.rs:37 (also `suprnova::render_cache::config::L0Limits`)
  - Public fields: `max_entries`, `max_bytes`
- [ ] struct `suprnova::render_cache::RenderCacheConfig` · framework/src/render_cache/config.rs:264 (also `suprnova::render_cache::config::RenderCacheConfig`)
  - Public fields: `enabled`, `profile`, `l0`, `l1`, `coordinator`, `failure`, `hints`, `build_id`
  - [ ] fn `suprnova::render_cache::RenderCacheConfig::with_build_id` · framework/src/render_cache/config.rs:398
  - [ ] fn `suprnova::render_cache::RenderCacheConfig::from_env` · framework/src/render_cache/config.rs:458
  - [ ] fn `suprnova::render_cache::RenderCacheConfig::enabled_from_env` · framework/src/render_cache/config.rs:471
- [ ] enum `suprnova::render_cache::CoordinatorConfig` · framework/src/render_cache/config.rs:149 (also `suprnova::render_cache::config::CoordinatorConfig`)
  - Variants: `Local`, `Database`, `Redis`
- [ ] enum `suprnova::render_cache::HintsConfig` · framework/src/render_cache/config.rs:233 (also `suprnova::render_cache::config::HintsConfig`)
  - Variants: `Disabled`, `Redis`
- [ ] enum `suprnova::render_cache::L1Config` · framework/src/render_cache/config.rs:70 (also `suprnova::render_cache::config::L1Config`)
  - Variants: `Disabled`, `File`, `Database`, `Redis`
- [ ] enum `suprnova::render_cache::Profile` · framework/src/render_cache/config.rs:53 (also `suprnova::render_cache::config::Profile`)
  - Variants: `Embedded`, `Database`, `Redis`

### `suprnova::render_cache::file_store`

- [ ] struct `suprnova::render_cache::file_store::FileRenderStore` · framework/src/render_cache/file_store.rs:117
  - Implements: `suprnova_live::render_cache::RenderStore`
  - [ ] fn `suprnova::render_cache::file_store::FileRenderStore::open` · framework/src/render_cache/file_store.rs:145
  - [ ] fn `suprnova::render_cache::file_store::FileRenderStore::sweep` · framework/src/render_cache/file_store.rs:577
- [ ] struct `suprnova::render_cache::SweepOutcome` · framework/src/render_cache/file_store.rs:630 (also `suprnova::render_cache::file_store::SweepOutcome`)
  - Public fields: `removed`, `more_remain`

### `suprnova::render_cache::hints`

- [ ] const `suprnova::render_cache::hints::MAX_HINT_DIGESTS` · framework/src/render_cache/hints.rs:83
- [ ] const `suprnova::render_cache::hints::MAX_INBOUND_HINTS` · framework/src/render_cache/hints.rs:127

### `suprnova::render_cache::ledger`

- [ ] fn `suprnova::render_cache::ledger::advance_in_current_transaction` · framework/src/render_cache/ledger.rs:372
- [ ] fn `suprnova::render_cache::ledger::advance_via_handle` · framework/src/render_cache/ledger.rs:599
- [ ] fn `suprnova::render_cache::ledger::advance_via_tx` · framework/src/render_cache/ledger.rs:574
- [ ] fn `suprnova::render_cache::ledger::tier_migration_present` · framework/src/render_cache/ledger.rs:207
- [ ] struct `suprnova::render_cache::ledger::SqlGenerationLedger` · framework/src/render_cache/ledger.rs:615
  - Implements: `suprnova_live::render_cache::GenerationLedger`
  - [ ] fn `suprnova::render_cache::ledger::SqlGenerationLedger::new` · framework/src/render_cache/ledger.rs:620
  - [ ] fn `suprnova::render_cache::ledger::SqlGenerationLedger::advance_epoch` · framework/src/render_cache/ledger.rs:636

### `suprnova::render_cache::live`

- [ ] fn `suprnova::render_cache::live::document_declines` · framework/src/render_cache/live.rs:211
- [ ] fn `suprnova::render_cache::live::record_bootstrap_nonce` · framework/src/render_cache/live.rs:135
- [ ] fn `suprnova::render_cache::live::record_document_digest` · framework/src/render_cache/live.rs:143
- [ ] fn `suprnova::render_cache::live::record_document_intent` · framework/src/render_cache/live.rs:163
- [ ] fn `suprnova::render_cache::live::record_mount` · framework/src/render_cache/live.rs:107
- [ ] fn `suprnova::render_cache::live::record_shell_island` · framework/src/render_cache/live.rs:123
- [ ] fn `suprnova::render_cache::live::record_stitch_capture_invalid` · framework/src/render_cache/live.rs:151
- [ ] fn `suprnova::render_cache::live::record_stitch_slot` · framework/src/render_cache/live.rs:116
- [ ] fn `suprnova::render_cache::live::seed_remaining_ms` · framework/src/render_cache/live.rs:234
- [ ] struct `suprnova::render_cache::live::CapturedSlot` · framework/src/render_cache/live.rs:47
  - Public fields: `descriptor`, `html`
- [ ] struct `suprnova::render_cache::live::LiveDocumentFacts` · framework/src/render_cache/live.rs:22
  - Public fields: `public_seed_islands`, `identity_bound_islands`, `seed_deadline_ms`, `no_store`, `stitch`
- [ ] struct `suprnova::render_cache::live::StitchCapture` · framework/src/render_cache/live.rs:84
  - Public fields: `slots`, `shell_islands`, `nonce`, `document_digest`, `invalid`
- [ ] enum `suprnova::render_cache::live::LiveDocumentDecline` · framework/src/render_cache/live.rs:174
  - Variants: `IdentityBoundWithoutStitching`, `InvalidStitchCapture`, `NoStoreIntent`, `UnresolvableSeedDeadline`

### `suprnova::render_cache::middleware`

- [ ] struct `suprnova::render_cache::RenderCacheMiddleware` · framework/src/render_cache/middleware.rs:285 (also `suprnova::render_cache::middleware::RenderCacheMiddleware`)
  - Implements: `suprnova::Middleware`
- [ ] struct `suprnova::render_cache::RenderCacheRuntime` · framework/src/render_cache/middleware.rs:369 (also `suprnova::render_cache::middleware::RenderCacheRuntime`)

### `suprnova::render_cache::migration`

- [ ] struct `suprnova::render_cache::migration::Migration` · framework/src/render_cache/migration.rs:23
- [ ] struct `suprnova::render_cache::migration::TierMigration` · framework/src/render_cache/migration.rs:244

### `suprnova::render_cache::orm`

- [ ] fn `suprnova::render_cache::orm::after_bulk_write` · framework/src/render_cache/orm.rs:251
- [ ] fn `suprnova::render_cache::orm::after_bulk_write_with_handle` · framework/src/render_cache/orm.rs:265
- [ ] fn `suprnova::render_cache::orm::after_model_write` · framework/src/render_cache/orm.rs:173
- [ ] fn `suprnova::render_cache::orm::after_model_write_with_tx` · framework/src/render_cache/orm.rs:205
- [ ] fn `suprnova::render_cache::orm::after_row_write` · framework/src/render_cache/orm.rs:331
- [ ] fn `suprnova::render_cache::orm::after_row_write_with_handle` · framework/src/render_cache/orm.rs:341
- [ ] fn `suprnova::render_cache::orm::after_table_write` · framework/src/render_cache/orm.rs:277
- [ ] fn `suprnova::render_cache::orm::after_unknown_write` · framework/src/render_cache/orm.rs:297

### `suprnova::render_cache::providers`

- [ ] fn `suprnova::render_cache::providers::sql_now_ms` · framework/src/render_cache/providers/mod.rs:66
- [ ] fn `suprnova::render_cache::providers::store_now_ms` · framework/src/render_cache/providers/mod.rs:96

### `suprnova::render_cache::providers::redis`

- [ ] fn `suprnova::render_cache::providers::redis::ping` · framework/src/render_cache/providers/redis.rs:295
- [ ] struct `suprnova::render_cache::providers::RedisProviderConfig` · framework/src/render_cache/providers/redis.rs:115 (also `suprnova::render_cache::providers::redis::RedisProviderConfig`)
  - Public fields: `url`, `prefix`

### `suprnova::render_cache::providers::redis_instances`

- [ ] struct `suprnova::render_cache::providers::RedisInstanceRecordStore` · framework/src/render_cache/providers/redis_instances.rs:245 (also `suprnova::render_cache::providers::redis_instances::RedisInstanceRecordStore`)
  - Implements: `suprnova_live::ledger::InstanceRecordStore`
  - [ ] fn `suprnova::render_cache::providers::RedisInstanceRecordStore::connect` · framework/src/render_cache/providers/redis_instances.rs:261
  - [ ] fn `suprnova::render_cache::providers::RedisInstanceRecordStore::open` · framework/src/render_cache/providers/redis_instances.rs:280

### `suprnova::render_cache::providers::redis_lease`

- [ ] struct `suprnova::render_cache::providers::RedisLeaseStore` · framework/src/render_cache/providers/redis_lease.rs:152 (also `suprnova::render_cache::providers::redis_lease::RedisLeaseStore`)
  - Implements: `suprnova_live::render_cache::LeaseStore`
  - [ ] fn `suprnova::render_cache::providers::RedisLeaseStore::connect` · framework/src/render_cache/providers/redis_lease.rs:168

### `suprnova::render_cache::providers::redis_store`

- [ ] struct `suprnova::render_cache::RedisRenderStore` · framework/src/render_cache/providers/redis_store.rs:169 (also `suprnova::render_cache::providers::RedisRenderStore`, `suprnova::render_cache::providers::redis_store::RedisRenderStore`)
  - Implements: `suprnova_live::render_cache::RenderStore`
  - [ ] fn `suprnova::render_cache::RedisRenderStore::connect` · framework/src/render_cache/providers/redis_store.rs:186

### `suprnova::render_cache::providers::sql_instances`

- [ ] struct `suprnova::render_cache::providers::SqlInstanceRecordStore` · framework/src/render_cache/providers/sql_instances.rs:93 (also `suprnova::render_cache::providers::sql_instances::SqlInstanceRecordStore`)
  - Implements: `suprnova_live::ledger::InstanceRecordStore`
  - [ ] fn `suprnova::render_cache::providers::SqlInstanceRecordStore::new` · framework/src/render_cache/providers/sql_instances.rs:104

### `suprnova::render_cache::providers::sql_lease`

- [ ] struct `suprnova::render_cache::providers::SqlLeaseStore` · framework/src/render_cache/providers/sql_lease.rs:73 (also `suprnova::render_cache::providers::sql_lease::SqlLeaseStore`)
  - Implements: `suprnova_live::render_cache::LeaseStore`
  - [ ] fn `suprnova::render_cache::providers::SqlLeaseStore::new` · framework/src/render_cache/providers/sql_lease.rs:83

### `suprnova::render_cache::providers::sql_store`

- [ ] struct `suprnova::render_cache::SqlRenderStore` · framework/src/render_cache/providers/sql_store.rs:82 (also `suprnova::render_cache::providers::SqlRenderStore`, `suprnova::render_cache::providers::sql_store::SqlRenderStore`)
  - Implements: `suprnova_live::render_cache::RenderStore`
  - [ ] fn `suprnova::render_cache::SqlRenderStore::new` · framework/src/render_cache/providers/sql_store.rs:94
  - [ ] fn `suprnova::render_cache::SqlRenderStore::sweep` · framework/src/render_cache/providers/sql_store.rs:136

### `suprnova::render_cache::registry`

- [ ] struct `suprnova::render_cache::registry::RenderCachePolicyTable` · framework/src/render_cache/registry.rs:33
  - [ ] fn `suprnova::render_cache::registry::RenderCachePolicyTable::register_group` · framework/src/render_cache/registry.rs:40
  - [ ] fn `suprnova::render_cache::registry::RenderCachePolicyTable::register_route` · framework/src/render_cache/registry.rs:64
  - [ ] fn `suprnova::render_cache::registry::RenderCachePolicyTable::effective_policy` · framework/src/render_cache/registry.rs:110
- [ ] enum `suprnova::render_cache::registry::GroupPolicy` · framework/src/render_cache/registry.rs:12
  - Variants: `Policy`, `Patch`

### `suprnova::render_cache::telemetry`

- [ ] fn `suprnova::render_cache::telemetry::await_hint_for_test` · framework/src/render_cache/telemetry.rs:328 (feature: `testing`)
- [ ] fn `suprnova::render_cache::telemetry::decline_reason_labels_for_test` · framework/src/render_cache/telemetry.rs:212 (feature: `testing`)
- [ ] fn `suprnova::render_cache::telemetry::recorded_hints_for_test` · framework/src/render_cache/telemetry.rs:309 (feature: `testing`)
- [ ] fn `suprnova::render_cache::telemetry::recorded_lookups_for_test` · framework/src/render_cache/telemetry.rs:195 (feature: `testing`)
- [ ] fn `suprnova::render_cache::telemetry::reset_recorded_hints_for_test` · framework/src/render_cache/telemetry.rs:316 (feature: `testing`)
- [ ] fn `suprnova::render_cache::telemetry::reset_recorded_lookups_for_test` · framework/src/render_cache/telemetry.rs:202 (feature: `testing`)
- [ ] struct `suprnova::render_cache::telemetry::RecordedLookup` · framework/src/render_cache/telemetry.rs:137 (feature: `testing`)
  - Public fields: `outcome`, `reason`
- [ ] const `suprnova::render_cache::telemetry::CAUSE` · framework/src/render_cache/telemetry.rs:48
- [ ] const `suprnova::render_cache::telemetry::EPOCH_REWINDS` · framework/src/render_cache/telemetry.rs:88
- [ ] const `suprnova::render_cache::telemetry::HINTS` · framework/src/render_cache/telemetry.rs:73
- [ ] const `suprnova::render_cache::telemetry::HITS` · framework/src/render_cache/telemetry.rs:18
- [ ] const `suprnova::render_cache::telemetry::LOOKUPS` · framework/src/render_cache/telemetry.rs:4
- [ ] const `suprnova::render_cache::telemetry::OUTCOME` · framework/src/render_cache/telemetry.rs:95
- [ ] const `suprnova::render_cache::telemetry::PUBLICATIONS` · framework/src/render_cache/telemetry.rs:20
- [ ] const `suprnova::render_cache::telemetry::REASON` · framework/src/render_cache/telemetry.rs:126
- [ ] const `suprnova::render_cache::telemetry::REBUILDS` · framework/src/render_cache/telemetry.rs:22
- [ ] const `suprnova::render_cache::telemetry::STITCH_ASSEMBLIES` · framework/src/render_cache/telemetry.rs:28
- [ ] const `suprnova::render_cache::telemetry::STITCH_NESTED` · framework/src/render_cache/telemetry.rs:45
- [ ] const `suprnova::render_cache::telemetry::STITCH_SLOTS` · framework/src/render_cache/telemetry.rs:36

### `suprnova::render_cache::write_side`

- [ ] fn `suprnova::render_cache::write_side::decide` · framework/src/render_cache/write_side.rs:70
- [ ] fn `suprnova::render_cache::write_side::decision` · framework/src/render_cache/write_side.rs:177
- [ ] enum `suprnova::render_cache::write_side::WriteSideDecision` · framework/src/render_cache/write_side.rs:38
  - Variants: `Open`, `Closed`, `Undecided`

## resources

### `suprnova::resources`

- [ ] trait `suprnova::AsRelationshipValue` · framework/src/resources/mod.rs:76 (also `suprnova::resources::AsRelationshipValue`)
  - Implemented here by: `Option`, `Vec`
  - [ ] fn `suprnova::AsRelationshipValue::as_relationship_value` · framework/src/resources/mod.rs:80 (required)
- [ ] trait `suprnova::PushIncluded` · framework/src/resources/mod.rs:121 (also `suprnova::resources::PushIncluded`)
  - Implemented here by: `Option`, `Vec`
  - [ ] fn `suprnova::PushIncluded::push_included` · framework/src/resources/mod.rs:125 (required)

### `suprnova::resources::builder`

- [ ] fn `suprnova::resources::render_resource_object` · framework/src/resources/builder.rs:239 (also `suprnova::resources::builder::render_resource_object`)
- [ ] struct `suprnova::IncludedSink` · framework/src/resources/builder.rs:17 (also `suprnova::resources::IncludedSink`, `suprnova::resources::builder::IncludedSink`)
  - [ ] fn `suprnova::IncludedSink::new` · framework/src/resources/builder.rs:24
  - [ ] fn `suprnova::IncludedSink::push` · framework/src/resources/builder.rs:30
  - [ ] fn `suprnova::IncludedSink::into_items` · framework/src/resources/builder.rs:52
  - [ ] fn `suprnova::IncludedSink::items` · framework/src/resources/builder.rs:57
  - [ ] fn `suprnova::IncludedSink::is_empty` · framework/src/resources/builder.rs:63
- [ ] struct `suprnova::JsonApiBuilder` · framework/src/resources/builder.rs:71 (also `suprnova::resources::JsonApiBuilder`, `suprnova::resources::builder::JsonApiBuilder`)
  - [ ] fn `suprnova::JsonApiBuilder::with_meta` · framework/src/resources/builder.rs:115
  - [ ] fn `suprnova::JsonApiBuilder::with_meta_kv` · framework/src/resources/builder.rs:121
  - [ ] fn `suprnova::JsonApiBuilder::with_meta_map` · framework/src/resources/builder.rs:126
  - [ ] fn `suprnova::JsonApiBuilder::with_link` · framework/src/resources/builder.rs:134
  - [ ] fn `suprnova::JsonApiBuilder::with_link_value` · framework/src/resources/builder.rs:141
  - [ ] fn `suprnova::JsonApiBuilder::with_additional` · framework/src/resources/builder.rs:149
  - [ ] fn `suprnova::JsonApiBuilder::with_additional_map` · framework/src/resources/builder.rs:155
  - [ ] fn `suprnova::JsonApiBuilder::with_jsonapi` · framework/src/resources/builder.rs:163
  - [ ] fn `suprnova::JsonApiBuilder::build` · framework/src/resources/builder.rs:202

### `suprnova::resources::fieldset`

- [ ] fn `suprnova::current_fieldset` · framework/src/resources/fieldset.rs:68 (also `suprnova::resources::current_fieldset`, `suprnova::resources::fieldset::current_fieldset`)
- [ ] fn `suprnova::scope_fieldset` · framework/src/resources/fieldset.rs:74 (also `suprnova::resources::fieldset::scope_fieldset`, `suprnova::resources::scope_fieldset`)
- [ ] struct `suprnova::RequestFieldsetSet` · framework/src/resources/fieldset.rs:8 (also `suprnova::resources::RequestFieldsetSet`, `suprnova::resources::fieldset::RequestFieldsetSet`)
  - [ ] fn `suprnova::RequestFieldsetSet::from_query` · framework/src/resources/fieldset.rs:21
  - [ ] fn `suprnova::RequestFieldsetSet::fields_for` · framework/src/resources/fieldset.rs:47
  - [ ] fn `suprnova::RequestFieldsetSet::is_empty` · framework/src/resources/fieldset.rs:55
- [ ] static `suprnova::resources::REQUEST_FIELDSET` · framework/src/resources/fieldset.rs:60 (also `suprnova::resources::fieldset::REQUEST_FIELDSET`)

### `suprnova::resources::include_tree`

- [ ] fn `suprnova::current_max_relationship_depth` · framework/src/resources/include_tree.rs:76 (also `suprnova::resources::current_max_relationship_depth`, `suprnova::resources::include_tree::current_max_relationship_depth`)
- [ ] fn `suprnova::max_relationship_depth` · framework/src/resources/include_tree.rs:66 (also `suprnova::resources::include_tree::max_relationship_depth`, `suprnova::resources::max_relationship_depth`)
- [ ] struct `suprnova::IncludeTree` · framework/src/resources/include_tree.rs:30 (also `suprnova::resources::IncludeTree`, `suprnova::resources::include_tree::IncludeTree`)
  - Public fields: `children`
  - [ ] fn `suprnova::IncludeTree::from_include_set` · framework/src/resources/include_tree.rs:87
  - [ ] fn `suprnova::IncludeTree::is_empty` · framework/src/resources/include_tree.rs:105
  - [ ] fn `suprnova::IncludeTree::subtree` · framework/src/resources/include_tree.rs:111
  - [ ] fn `suprnova::IncludeTree::iter` · framework/src/resources/include_tree.rs:116
- [ ] const `suprnova::DEFAULT_MAX_RELATIONSHIP_DEPTH` · framework/src/resources/include_tree.rs:38 (also `suprnova::resources::DEFAULT_MAX_RELATIONSHIP_DEPTH`, `suprnova::resources::include_tree::DEFAULT_MAX_RELATIONSHIP_DEPTH`)

### `suprnova::resources::jsonapi_info`

- [ ] struct `suprnova::JsonApiInfo` · framework/src/resources/jsonapi_info.rs:10 (also `suprnova::resources::JsonApiInfo`, `suprnova::resources::jsonapi_info::JsonApiInfo`)
  - Public fields: `version`, `ext`, `profile`, `meta`
  - [ ] fn `suprnova::JsonApiInfo::new` · framework/src/resources/jsonapi_info.rs:23
  - [ ] fn `suprnova::JsonApiInfo::with_version` · framework/src/resources/jsonapi_info.rs:28
  - [ ] fn `suprnova::JsonApiInfo::with_ext` · framework/src/resources/jsonapi_info.rs:34
  - [ ] fn `suprnova::JsonApiInfo::with_profile` · framework/src/resources/jsonapi_info.rs:40
  - [ ] fn `suprnova::JsonApiInfo::with_meta` · framework/src/resources/jsonapi_info.rs:46
  - [ ] fn `suprnova::JsonApiInfo::is_empty` · framework/src/resources/jsonapi_info.rs:52
  - [ ] fn `suprnova::JsonApiInfo::to_value` · framework/src/resources/jsonapi_info.rs:62

### `suprnova::resources::maybe`

- [ ] fn `suprnova::insert_maybe` · framework/src/resources/maybe.rs:224 (also `suprnova::resources::insert_maybe`, `suprnova::resources::maybe::insert_maybe`)
- [ ] fn `suprnova::strip_missing_values` · framework/src/resources/maybe.rs:194 (also `suprnova::resources::maybe::strip_missing_values`, `suprnova::resources::strip_missing_values`)
- [ ] enum `suprnova::Maybe` · framework/src/resources/maybe.rs:51 (also `suprnova::resources::Maybe`, `suprnova::resources::maybe::Maybe`)
  - Variants: `Present`, `Missing`
  - [ ] fn `suprnova::Maybe::present` · framework/src/resources/maybe.rs:65
  - [ ] fn `suprnova::Maybe::missing` · framework/src/resources/maybe.rs:70
  - [ ] fn `suprnova::Maybe::when` · framework/src/resources/maybe.rs:76
  - [ ] fn `suprnova::Maybe::unless` · framework/src/resources/maybe.rs:85
  - [ ] fn `suprnova::Maybe::when_with` · framework/src/resources/maybe.rs:90
  - [ ] fn `suprnova::Maybe::is_missing` · framework/src/resources/maybe.rs:99
  - [ ] fn `suprnova::Maybe::is_present` · framework/src/resources/maybe.rs:104
  - [ ] fn `suprnova::Maybe::map` · framework/src/resources/maybe.rs:109
  - [ ] fn `suprnova::Maybe::into_option` · framework/src/resources/maybe.rs:117
  - [ ] fn `suprnova::Maybe::as_ref` · framework/src/resources/maybe.rs:125
- [ ] type `suprnova::MissingValue` · framework/src/resources/maybe.rs:61 (also `suprnova::resources::MissingValue`, `suprnova::resources::maybe::MissingValue`)

### `suprnova::resources::response`

- [ ] struct `suprnova::JsonApi` · framework/src/resources/response.rs:283 (also `suprnova::resources::JsonApi`, `suprnova::resources::response::JsonApi`)
  - [ ] fn `suprnova::JsonApi::single` · framework/src/resources/response.rs:288
  - [ ] fn `suprnova::JsonApi::collection` · framework/src/resources/response.rs:294
  - [ ] fn `suprnova::JsonApi::paginated` · framework/src/resources/response.rs:300
- [ ] struct `suprnova::JsonApiResponse` · framework/src/resources/response.rs:22 (also `suprnova::resources::JsonApiResponse`, `suprnova::resources::response::JsonApiResponse`)
  - [ ] fn `suprnova::JsonApiResponse::status` · framework/src/resources/response.rs:39
  - [ ] fn `suprnova::JsonApiResponse::created` · framework/src/resources/response.rs:47
  - [ ] fn `suprnova::JsonApiResponse::with_meta` · framework/src/resources/response.rs:55
  - [ ] fn `suprnova::JsonApiResponse::meta` · framework/src/resources/response.rs:63
  - [ ] fn `suprnova::JsonApiResponse::with_meta_map` · framework/src/resources/response.rs:69
  - [ ] fn `suprnova::JsonApiResponse::with_link` · framework/src/resources/response.rs:78
  - [ ] fn `suprnova::JsonApiResponse::link` · framework/src/resources/response.rs:86
  - [ ] fn `suprnova::JsonApiResponse::with_link_value` · framework/src/resources/response.rs:93
  - [ ] fn `suprnova::JsonApiResponse::additional` · framework/src/resources/response.rs:104
  - [ ] fn `suprnova::JsonApiResponse::with_additional` · framework/src/resources/response.rs:112
  - [ ] fn `suprnova::JsonApiResponse::with_jsonapi` · framework/src/resources/response.rs:121
  - [ ] fn `suprnova::JsonApiResponse::render` · framework/src/resources/response.rs:132
- [ ] struct `suprnova::Resource` · framework/src/resources/response.rs:157 (also `suprnova::resources::Resource`, `suprnova::resources::response::Resource`)
  - [ ] fn `suprnova::Resource::single` · framework/src/resources/response.rs:162
  - [ ] fn `suprnova::Resource::collection` · framework/src/resources/response.rs:182
  - [ ] fn `suprnova::Resource::paginated` · framework/src/resources/response.rs:223

### `suprnova::resources::trait_def`

- [ ] struct `suprnova::IncludeResolutionError` · framework/src/resources/trait_def.rs:122 (also `suprnova::resources::IncludeResolutionError`, `suprnova::resources::trait_def::IncludeResolutionError`)
  - Public fields: `path`, `on_type`
- [ ] struct `suprnova::ResourceIdentifier` · framework/src/resources/trait_def.rs:92 (also `suprnova::resources::ResourceIdentifier`, `suprnova::resources::trait_def::ResourceIdentifier`)
  - Public fields: `resource_type`, `id`
  - [ ] fn `suprnova::ResourceIdentifier::new` · framework/src/resources/trait_def.rs:101
  - [ ] fn `suprnova::ResourceIdentifier::to_value` · framework/src/resources/trait_def.rs:110
- [ ] enum `suprnova::RelationshipValue` · framework/src/resources/trait_def.rs:80 (also `suprnova::resources::RelationshipValue`, `suprnova::resources::trait_def::RelationshipValue`)
  - Variants: `Single`, `Many`, `Null`
- [ ] trait `suprnova::IntoJsonResource` · framework/src/resources/trait_def.rs:14 (also `suprnova::resources::IntoJsonResource`, `suprnova::resources::trait_def::IntoJsonResource`)
  - [ ] fn `suprnova::IntoJsonResource::resource_type` · framework/src/resources/trait_def.rs:16 (required)
  - [ ] fn `suprnova::IntoJsonResource::resource_id` · framework/src/resources/trait_def.rs:22 (required)
  - [ ] fn `suprnova::IntoJsonResource::resource_attributes` · framework/src/resources/trait_def.rs:26 (required)
  - [ ] fn `suprnova::IntoJsonResource::resource_relationships` · framework/src/resources/trait_def.rs:32 (required)
  - [ ] fn `suprnova::IntoJsonResource::resource_included` · framework/src/resources/trait_def.rs:43 (required)
  - [ ] fn `suprnova::IntoJsonResource::resource_links` · framework/src/resources/trait_def.rs:54 (provided)
  - [ ] fn `suprnova::IntoJsonResource::resource_meta` · framework/src/resources/trait_def.rs:63 (provided)
  - [ ] fn `suprnova::IntoJsonResource::resource_top_level_meta` · framework/src/resources/trait_def.rs:73 (provided)

## routing

### `suprnova::routing`

- [ ] fn `suprnova::redirect` · framework/src/routing/mod.rs:71 (also `suprnova::routing::redirect`)
- [ ] fn `suprnova::redirect_to` · framework/src/routing/mod.rs:79 (also `suprnova::routing::redirect_to`)

### `suprnova::routing::group` (private module; items are public through re-exports)

- [ ] struct `suprnova::GroupBuilder` · framework/src/routing/group.rs:37 (also `suprnova::routing::GroupBuilder`)
  - [ ] fn `suprnova::GroupBuilder::middleware` · framework/src/routing/group.rs:89
  - [ ] fn `suprnova::GroupBuilder::block_session` · framework/src/routing/group.rs:96
  - [ ] fn `suprnova::GroupBuilder::try_finalize` · framework/src/routing/group.rs:136
- [ ] struct `suprnova::GroupRouter` · framework/src/routing/group.rs:205 (also `suprnova::routing::GroupRouter`)
  - [ ] fn `suprnova::GroupRouter::get` · framework/src/routing/group.rs:215
  - [ ] fn `suprnova::GroupRouter::post` · framework/src/routing/group.rs:230
  - [ ] fn `suprnova::GroupRouter::put` · framework/src/routing/group.rs:245
  - [ ] fn `suprnova::GroupRouter::delete` · framework/src/routing/group.rs:260
  - [ ] fn `suprnova::GroupRouter::patch` · framework/src/routing/group.rs:275
  - [ ] fn `suprnova::GroupRouter::head` · framework/src/routing/group.rs:295
  - [ ] fn `suprnova::GroupRouter::options` · framework/src/routing/group.rs:314
  - [ ] fn `suprnova::GroupRouter::any` · framework/src/routing/group.rs:336
  - [ ] fn `suprnova::GroupRouter::methods` · framework/src/routing/group.rs:369
  - [ ] fn `suprnova::GroupRouter::try_methods` · framework/src/routing/group.rs:382

### `suprnova::routing::macros` (private module; items are public through re-exports)

- [ ] fn `suprnova::validate_route_path` · framework/src/routing/macros.rs:45 (also `suprnova::routing::validate_route_path`)
- [ ] struct `suprnova::routing::AnyRouteDefBuilder` · framework/src/routing/macros.rs:586
  - Implements: `suprnova::IntoGroupItem`
  - [ ] fn `suprnova::routing::AnyRouteDefBuilder::name` · framework/src/routing/macros.rs:611
  - [ ] fn `suprnova::routing::AnyRouteDefBuilder::middleware` · framework/src/routing/macros.rs:618
  - [ ] fn `suprnova::routing::AnyRouteDefBuilder::register` · framework/src/routing/macros.rs:626
  - [ ] fn `suprnova::routing::AnyRouteDefBuilder::into_group_any_route` · framework/src/routing/macros.rs:1293
- [ ] struct `suprnova::FallbackDefBuilder` · framework/src/routing/macros.rs:814 (also `suprnova::routing::FallbackDefBuilder`)
  - [ ] fn `suprnova::FallbackDefBuilder::new` · framework/src/routing/macros.rs:825
  - [ ] fn `suprnova::FallbackDefBuilder::middleware` · framework/src/routing/macros.rs:833
  - [ ] fn `suprnova::FallbackDefBuilder::register` · framework/src/routing/macros.rs:839
- [ ] struct `suprnova::routing::GroupAnyRoute` · framework/src/routing/macros.rs:930
- [ ] struct `suprnova::GroupDef` · framework/src/routing/macros.rs:983 (also `suprnova::routing::GroupDef`)
  - Implements: `suprnova::IntoGroupItem`
  - [ ] fn `suprnova::GroupDef::add` · framework/src/routing/macros.rs:1012
  - [ ] fn `suprnova::GroupDef::route` · framework/src/routing/macros.rs:1020
  - [ ] fn `suprnova::GroupDef::middleware` · framework/src/routing/macros.rs:1051
  - [ ] fn `suprnova::GroupDef::block_session` · framework/src/routing/macros.rs:1060
  - [ ] fn `suprnova::GroupDef::register` · framework/src/routing/macros.rs:1083
- [ ] struct `suprnova::GroupRoute` · framework/src/routing/macros.rs:916 (also `suprnova::routing::GroupRoute`)
- [ ] struct `suprnova::RouteDefBuilder` · framework/src/routing/macros.rs:198 (also `suprnova::routing::RouteDefBuilder`)
  - Implements: `suprnova::IntoGroupItem`
  - [ ] fn `suprnova::RouteDefBuilder::new` · framework/src/routing/macros.rs:213
  - [ ] fn `suprnova::RouteDefBuilder::name` · framework/src/routing/macros.rs:225
  - [ ] fn `suprnova::RouteDefBuilder::middleware` · framework/src/routing/macros.rs:231
  - [ ] fn `suprnova::RouteDefBuilder::block_session` · framework/src/routing/macros.rs:239
  - [ ] fn `suprnova::RouteDefBuilder::register` · framework/src/routing/macros.rs:245
  - [ ] fn `suprnova::RouteDefBuilder::into_group_route` · framework/src/routing/macros.rs:1251
- [ ] struct `suprnova::WsRouteDef` · framework/src/routing/macros.rs:725 (also `suprnova::routing::WsRouteDef`)
  - [ ] fn `suprnova::WsRouteDef::new` · framework/src/routing/macros.rs:736
  - [ ] fn `suprnova::WsRouteDef::middleware` · framework/src/routing/macros.rs:755
  - [ ] fn `suprnova::WsRouteDef::config` · framework/src/routing/macros.rs:789
  - [ ] fn `suprnova::WsRouteDef::register` · framework/src/routing/macros.rs:796
- [ ] enum `suprnova::GroupItem` · framework/src/routing/macros.rs:940 (also `suprnova::routing::GroupItem`)
  - Variants: `Route`, `AnyRoute`, `NestedGroup`
- [ ] enum `suprnova::routing::HttpMethod` · framework/src/routing/macros.rs:161
  - Variants: `Get`, `Post`, `Put`, `Patch`, `Delete`, `Head`, `Options`
- [ ] trait `suprnova::IntoGroupItem` · framework/src/routing/macros.rs:950 (also `suprnova::routing::IntoGroupItem`)
  - Implemented here by: `GroupDef`, `RouteDefBuilder`, `routing::AnyRouteDefBuilder`
  - [ ] fn `suprnova::IntoGroupItem::into_group_item` · framework/src/routing/macros.rs:952 (required)

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

### `suprnova::routing::router` (private module; items are public through re-exports)

- [ ] fn `suprnova::clear_route_names_for_test` · framework/src/routing/router.rs:80 (also `suprnova::routing::clear_route_names_for_test`)
- [ ] fn `suprnova::routing::register_route_name` · framework/src/routing/router.rs:148
- [ ] fn `suprnova::route` · framework/src/routing/router.rs:412 (also `suprnova::routing::route`)
- [ ] fn `suprnova::routing::route_name_for_pattern` · framework/src/routing/router.rs:537
- [ ] fn `suprnova::routing::route_with_params` · framework/src/routing/router.rs:426
- [ ] fn `suprnova::routing::try_register_route_name` · framework/src/routing/router.rs:159
- [ ] fn `suprnova::routing::try_route` · framework/src/routing/router.rs:487
- [ ] fn `suprnova::routing::try_route_with_params` · framework/src/routing/router.rs:506
- [ ] struct `suprnova::routing::MultiMethodRouteBuilder` · framework/src/routing/router.rs:2296
  - [ ] fn `suprnova::routing::MultiMethodRouteBuilder::methods` · framework/src/routing/router.rs:2306
  - [ ] fn `suprnova::routing::MultiMethodRouteBuilder::path` · framework/src/routing/router.rs:2312
  - [ ] fn `suprnova::routing::MultiMethodRouteBuilder::name` · framework/src/routing/router.rs:2324
  - [ ] fn `suprnova::routing::MultiMethodRouteBuilder::try_name` · framework/src/routing/router.rs:2329
  - [ ] fn `suprnova::routing::MultiMethodRouteBuilder::middleware` · framework/src/routing/router.rs:2338
  - [ ] fn `suprnova::routing::MultiMethodRouteBuilder::middleware_boxed` · framework/src/routing/router.rs:2350
  - [ ] fn `suprnova::routing::MultiMethodRouteBuilder::block_session` · framework/src/routing/router.rs:2361
- [ ] struct `suprnova::RouteBuilder` · framework/src/routing/router.rs:1975 (also `suprnova::routing::RouteBuilder`)
  - [ ] fn `suprnova::RouteBuilder::group` · framework/src/routing/group.rs:498
  - [ ] fn `suprnova::RouteBuilder::name` · framework/src/routing/router.rs:2011
  - [ ] fn `suprnova::RouteBuilder::try_name` · framework/src/routing/router.rs:2019
  - [ ] fn `suprnova::RouteBuilder::middleware` · framework/src/routing/router.rs:2046
  - [ ] fn `suprnova::RouteBuilder::middleware_boxed` · framework/src/routing/router.rs:2056
  - [ ] fn `suprnova::RouteBuilder::block_session` · framework/src/routing/router.rs:2069
  - [ ] fn `suprnova::RouteBuilder::get` · framework/src/routing/router.rs:2075
  - [ ] fn `suprnova::RouteBuilder::post` · framework/src/routing/router.rs:2084
  - [ ] fn `suprnova::RouteBuilder::put` · framework/src/routing/router.rs:2093
  - [ ] fn `suprnova::RouteBuilder::delete` · framework/src/routing/router.rs:2102
  - [ ] fn `suprnova::RouteBuilder::try_get` · framework/src/routing/router.rs:2111
  - [ ] fn `suprnova::RouteBuilder::try_post` · framework/src/routing/router.rs:2120
  - [ ] fn `suprnova::RouteBuilder::try_put` · framework/src/routing/router.rs:2129
  - [ ] fn `suprnova::RouteBuilder::try_delete` · framework/src/routing/router.rs:2139
  - [ ] fn `suprnova::RouteBuilder::patch` · framework/src/routing/router.rs:2148
  - [ ] fn `suprnova::RouteBuilder::try_patch` · framework/src/routing/router.rs:2157
  - [ ] fn `suprnova::RouteBuilder::head` · framework/src/routing/router.rs:2166
  - [ ] fn `suprnova::RouteBuilder::try_head` · framework/src/routing/router.rs:2175
  - [ ] fn `suprnova::RouteBuilder::options` · framework/src/routing/router.rs:2184
  - [ ] fn `suprnova::RouteBuilder::try_options` · framework/src/routing/router.rs:2194
  - [ ] fn `suprnova::RouteBuilder::any` · framework/src/routing/router.rs:2205
  - [ ] fn `suprnova::RouteBuilder::try_any` · framework/src/routing/router.rs:2214
  - [ ] fn `suprnova::RouteBuilder::methods` · framework/src/routing/router.rs:2228
  - [ ] fn `suprnova::RouteBuilder::try_methods` · framework/src/routing/router.rs:2243
- [ ] struct `suprnova::Router` · framework/src/routing/router.rs:564 (also `suprnova::routing::Router`)
  - [ ] fn `suprnova::Router::try_live_nested_segment` · framework/src/live/document.rs:575
  - [ ] fn `suprnova::Router::try_live_document` · framework/src/live/document.rs:606
  - [ ] fn `suprnova::Router::try_live_mount` · framework/src/live/document.rs:621
  - [ ] fn `suprnova::Router::try_live` · framework/src/live/routes.rs:83
  - [ ] fn `suprnova::Router::try_live_ui_assets` · framework/src/live/routes.rs:98
  - [ ] fn `suprnova::Router::try_live_ui_assets_from` · framework/src/live/routes.rs:107
  - [ ] fn `suprnova::Router::try_live_with` · framework/src/live/routes.rs:144
  - [ ] fn `suprnova::Router::try_live_upload_reacquisition` · framework/src/live/upload.rs:38
  - [ ] fn `suprnova::Router::group` · framework/src/routing/group.rs:472
  - [ ] fn `suprnova::Router::resource` · framework/src/routing/resource.rs:693
  - [ ] fn `suprnova::Router::api_resource` · framework/src/routing/resource.rs:705
  - [ ] fn `suprnova::Router::resources` · framework/src/routing/resource.rs:719
  - [ ] fn `suprnova::Router::api_resources` · framework/src/routing/resource.rs:741
  - [ ] fn `suprnova::Router::new` · framework/src/routing/router.rs:631
  - [ ] fn `suprnova::Router::get_route_middleware` · framework/src/routing/router.rs:661
  - [ ] fn `suprnova::Router::try_render_cache` · framework/src/routing/router.rs:804
  - [ ] fn `suprnova::Router::try_render_cache_group` · framework/src/routing/router.rs:822
  - [ ] fn `suprnova::Router::get_fallback` · framework/src/routing/router.rs:863
  - [ ] fn `suprnova::Router::get` · framework/src/routing/router.rs:1081
  - [ ] fn `suprnova::Router::try_get` · framework/src/routing/router.rs:1096
  - [ ] fn `suprnova::Router::post` · framework/src/routing/router.rs:1120
  - [ ] fn `suprnova::Router::try_post` · framework/src/routing/router.rs:1130
  - [ ] fn `suprnova::Router::put` · framework/src/routing/router.rs:1158
  - [ ] fn `suprnova::Router::try_put` · framework/src/routing/router.rs:1168
  - [ ] fn `suprnova::Router::delete` · framework/src/routing/router.rs:1192
  - [ ] fn `suprnova::Router::try_delete` · framework/src/routing/router.rs:1202
  - [ ] fn `suprnova::Router::patch` · framework/src/routing/router.rs:1230
  - [ ] fn `suprnova::Router::try_patch` · framework/src/routing/router.rs:1240
  - [ ] fn `suprnova::Router::head` · framework/src/routing/router.rs:1284
  - [ ] fn `suprnova::Router::try_head` · framework/src/routing/router.rs:1294
  - [ ] fn `suprnova::Router::options` · framework/src/routing/router.rs:1328
  - [ ] fn `suprnova::Router::try_options` · framework/src/routing/router.rs:1338
  - [ ] fn `suprnova::Router::methods` · framework/src/routing/router.rs:1381
  - [ ] fn `suprnova::Router::try_methods` · framework/src/routing/router.rs:1399
  - [ ] fn `suprnova::Router::any` · framework/src/routing/router.rs:1456
  - [ ] fn `suprnova::Router::try_any` · framework/src/routing/router.rs:1467
  - [ ] fn `suprnova::Router::ws` · framework/src/routing/router.rs:1490
  - [ ] fn `suprnova::Router::try_ws` · framework/src/routing/router.rs:1501
  - [ ] fn `suprnova::Router::ws_with_config` · framework/src/routing/router.rs:1536
  - [ ] fn `suprnova::Router::try_ws_with_config` · framework/src/routing/router.rs:1546
  - [ ] fn `suprnova::Router::ws_with_middleware` · framework/src/routing/router.rs:1566
  - [ ] fn `suprnova::Router::try_ws_with_middleware` · framework/src/routing/router.rs:1581
  - [ ] fn `suprnova::Router::ws_with_middleware_and_config` · framework/src/routing/router.rs:1596
  - [ ] fn `suprnova::Router::try_ws_with_middleware_and_config` · framework/src/routing/router.rs:1612
  - [ ] fn `suprnova::Router::match_ws` · framework/src/routing/router.rs:1741
  - [ ] fn `suprnova::Router::match_route` · framework/src/routing/router.rs:1766
  - [ ] fn `suprnova::Router::has_explicit_head` · framework/src/routing/router.rs:1810
  - [ ] fn `suprnova::Router::redirect` · framework/src/routing/router.rs:1831
  - [ ] fn `suprnova::Router::permanent_redirect` · framework/src/routing/router.rs:1853
  - [ ] fn `suprnova::Router::inertia` · framework/src/routing/router.rs:1890
  - [ ] fn `suprnova::Router::try_inertia` · framework/src/routing/router.rs:1908
  - [ ] fn `suprnova::Router::view` · framework/src/routing/router.rs:1963
- [ ] struct `suprnova::routing::WsMatch` · framework/src/routing/router.rs:2382
  - [ ] fn `suprnova::routing::WsMatch::handler` · framework/src/routing/router.rs:2393
  - [ ] fn `suprnova::routing::WsMatch::pattern` · framework/src/routing/router.rs:2399
  - [ ] fn `suprnova::routing::WsMatch::params` · framework/src/routing/router.rs:2408
  - [ ] fn `suprnova::routing::WsMatch::middleware` · framework/src/routing/router.rs:2415
  - [ ] fn `suprnova::routing::WsMatch::config` · framework/src/routing/router.rs:2426
- [ ] enum `suprnova::routing::RouteUrlError` · framework/src/routing/router.rs:445
  - Variants: `NameNotFound`, `MissingParams`
- [ ] type `suprnova::routing::BoxedHandler` · framework/src/routing/router.rs:552

### `suprnova::routing::signed` (private module; items are public through re-exports)

- [ ] fn `suprnova::sign_route` · framework/src/routing/signed.rs:416 (also `suprnova::routing::sign_route`)
- [ ] fn `suprnova::sign_url` · framework/src/routing/signed.rs:224 (also `suprnova::routing::sign_url`)
- [ ] fn `suprnova::verify_signature` · framework/src/routing/signed.rs:296 (also `suprnova::routing::verify_signature`)
- [ ] enum `suprnova::SignatureVerdict` · framework/src/routing/signed.rs:84 (also `suprnova::routing::SignatureVerdict`)
  - Variants: `Valid`, `Expired`, `Invalid`
  - [ ] fn `suprnova::SignatureVerdict::is_valid` · framework/src/routing/signed.rs:99
  - [ ] fn `suprnova::SignatureVerdict::is_expired` · framework/src/routing/signed.rs:105
- [ ] const `suprnova::routing::EXPIRES_KEY` · framework/src/routing/signed.rs:80
- [ ] const `suprnova::routing::SIGNATURE_KEY` · framework/src/routing/signed.rs:76

### `suprnova::routing::url`

- [ ] fn `suprnova::url::current` · framework/src/routing/url.rs:91 (also `suprnova::routing::url::current`)
- [ ] fn `suprnova::url::full` · framework/src/routing/url.rs:101 (also `suprnova::routing::url::full`)
- [ ] fn `suprnova::url::has_valid_signature` · framework/src/routing/url.rs:168 (also `suprnova::routing::url::has_valid_signature`)
- [ ] fn `suprnova::url::previous` · framework/src/routing/url.rs:111 (also `suprnova::routing::url::previous`)
- [ ] fn `suprnova::url::secure` · framework/src/routing/url.rs:75 (also `suprnova::routing::url::secure`)
- [ ] fn `suprnova::url::signature_has_not_expired` · framework/src/routing/url.rs:207 (deprecated; also `suprnova::routing::url::signature_has_not_expired`)
- [ ] fn `suprnova::url::signature_verdict` · framework/src/routing/url.rs:214 (also `suprnova::routing::url::signature_verdict`)
- [ ] fn `suprnova::url::signed_route` · framework/src/routing/url.rs:123 (also `suprnova::routing::url::signed_route`)
- [ ] fn `suprnova::url::signed_url` · framework/src/routing/url.rs:149 (also `suprnova::routing::url::signed_url`)
- [ ] fn `suprnova::url::temporary_signed_route` · framework/src/routing/url.rs:137 (also `suprnova::routing::url::temporary_signed_route`)
- [ ] fn `suprnova::url::to` · framework/src/routing/url.rs:65 (also `suprnova::routing::url::to`)

## schedule

### `suprnova::schedule`

- [ ] struct `suprnova::Schedule` · framework/src/schedule/mod.rs:138 (also `suprnova::schedule::Schedule`)
  - [ ] fn `suprnova::Schedule::new` · framework/src/schedule/mod.rs:181
  - [ ] fn `suprnova::Schedule::timezone` · framework/src/schedule/mod.rs:208
  - [ ] fn `suprnova::Schedule::validate_single_server_locking` · framework/src/schedule/mod.rs:238
  - [ ] fn `suprnova::Schedule::task` · framework/src/schedule/mod.rs:287
  - [ ] fn `suprnova::Schedule::call` · framework/src/schedule/mod.rs:315
  - [ ] fn `suprnova::Schedule::add` · framework/src/schedule/mod.rs:342
  - [ ] fn `suprnova::Schedule::try_add` · framework/src/schedule/mod.rs:360
  - [ ] fn `suprnova::Schedule::tasks` · framework/src/schedule/mod.rs:380
  - [ ] fn `suprnova::Schedule::len` · framework/src/schedule/mod.rs:385
  - [ ] fn `suprnova::Schedule::is_empty` · framework/src/schedule/mod.rs:390
  - [ ] fn `suprnova::Schedule::due_tasks` · framework/src/schedule/mod.rs:395
  - [ ] fn `suprnova::Schedule::run_due_tasks` · framework/src/schedule/mod.rs:409
  - [ ] fn `suprnova::Schedule::run_due_tasks_into` · framework/src/schedule/mod.rs:424
  - [ ] fn `suprnova::Schedule::run_all_tasks` · framework/src/schedule/mod.rs:434
  - [ ] fn `suprnova::Schedule::run_all_tasks_into` · framework/src/schedule/mod.rs:444
  - [ ] fn `suprnova::Schedule::find` · framework/src/schedule/mod.rs:452
  - [ ] fn `suprnova::Schedule::run_task` · framework/src/schedule/mod.rs:457
- [ ] type `suprnova::schedule::ScheduledTaskJoin` · framework/src/schedule/mod.rs:101

### `suprnova::schedule::builder`

- [ ] struct `suprnova::TaskBuilder` · framework/src/schedule/builder.rs:34 (also `suprnova::schedule::TaskBuilder`, `suprnova::schedule::builder::TaskBuilder`)
  - [ ] fn `suprnova::TaskBuilder::new` · framework/src/schedule/builder.rs:51
  - [ ] fn `suprnova::TaskBuilder::from_async` · framework/src/schedule/builder.rs:72
  - [ ] fn `suprnova::TaskBuilder::from_task` · framework/src/schedule/builder.rs:103
  - [ ] fn `suprnova::TaskBuilder::cron` · framework/src/schedule/builder.rs:138
  - [ ] fn `suprnova::TaskBuilder::try_cron` · framework/src/schedule/builder.rs:152
  - [ ] fn `suprnova::TaskBuilder::every_minute` · framework/src/schedule/builder.rs:158
  - [ ] fn `suprnova::TaskBuilder::every_two_minutes` · framework/src/schedule/builder.rs:164
  - [ ] fn `suprnova::TaskBuilder::every_five_minutes` · framework/src/schedule/builder.rs:170
  - [ ] fn `suprnova::TaskBuilder::every_ten_minutes` · framework/src/schedule/builder.rs:176
  - [ ] fn `suprnova::TaskBuilder::every_fifteen_minutes` · framework/src/schedule/builder.rs:182
  - [ ] fn `suprnova::TaskBuilder::every_thirty_minutes` · framework/src/schedule/builder.rs:188
  - [ ] fn `suprnova::TaskBuilder::hourly` · framework/src/schedule/builder.rs:194
  - [ ] fn `suprnova::TaskBuilder::hourly_at` · framework/src/schedule/builder.rs:214
  - [ ] fn `suprnova::TaskBuilder::try_hourly_at` · framework/src/schedule/builder.rs:226
  - [ ] fn `suprnova::TaskBuilder::every_two_hours` · framework/src/schedule/builder.rs:232
  - [ ] fn `suprnova::TaskBuilder::every_three_hours` · framework/src/schedule/builder.rs:238
  - [ ] fn `suprnova::TaskBuilder::every_four_hours` · framework/src/schedule/builder.rs:244
  - [ ] fn `suprnova::TaskBuilder::every_six_hours` · framework/src/schedule/builder.rs:250
  - [ ] fn `suprnova::TaskBuilder::daily` · framework/src/schedule/builder.rs:256
  - [ ] fn `suprnova::TaskBuilder::daily_at` · framework/src/schedule/builder.rs:279
  - [ ] fn `suprnova::TaskBuilder::try_daily_at` · framework/src/schedule/builder.rs:294
  - [ ] fn `suprnova::TaskBuilder::twice_daily` · framework/src/schedule/builder.rs:315
  - [ ] fn `suprnova::TaskBuilder::try_twice_daily` · framework/src/schedule/builder.rs:327
  - [ ] fn `suprnova::TaskBuilder::at` · framework/src/schedule/builder.rs:353
  - [ ] fn `suprnova::TaskBuilder::try_at` · framework/src/schedule/builder.rs:366
  - [ ] fn `suprnova::TaskBuilder::weekly` · framework/src/schedule/builder.rs:372
  - [ ] fn `suprnova::TaskBuilder::weekly_on` · framework/src/schedule/builder.rs:387
  - [ ] fn `suprnova::TaskBuilder::days` · framework/src/schedule/builder.rs:402
  - [ ] fn `suprnova::TaskBuilder::weekdays` · framework/src/schedule/builder.rs:408
  - [ ] fn `suprnova::TaskBuilder::weekends` · framework/src/schedule/builder.rs:414
  - [ ] fn `suprnova::TaskBuilder::sundays` · framework/src/schedule/builder.rs:420
  - [ ] fn `suprnova::TaskBuilder::mondays` · framework/src/schedule/builder.rs:426
  - [ ] fn `suprnova::TaskBuilder::tuesdays` · framework/src/schedule/builder.rs:432
  - [ ] fn `suprnova::TaskBuilder::wednesdays` · framework/src/schedule/builder.rs:438
  - [ ] fn `suprnova::TaskBuilder::thursdays` · framework/src/schedule/builder.rs:444
  - [ ] fn `suprnova::TaskBuilder::fridays` · framework/src/schedule/builder.rs:450
  - [ ] fn `suprnova::TaskBuilder::saturdays` · framework/src/schedule/builder.rs:456
  - [ ] fn `suprnova::TaskBuilder::monthly` · framework/src/schedule/builder.rs:462
  - [ ] fn `suprnova::TaskBuilder::monthly_on` · framework/src/schedule/builder.rs:483
  - [ ] fn `suprnova::TaskBuilder::try_monthly_on` · framework/src/schedule/builder.rs:495
  - [ ] fn `suprnova::TaskBuilder::quarterly` · framework/src/schedule/builder.rs:501
  - [ ] fn `suprnova::TaskBuilder::yearly` · framework/src/schedule/builder.rs:507
  - [ ] fn `suprnova::TaskBuilder::name` · framework/src/schedule/builder.rs:519
  - [ ] fn `suprnova::TaskBuilder::description` · framework/src/schedule/builder.rs:527
  - [ ] fn `suprnova::TaskBuilder::timezone` · framework/src/schedule/builder.rs:562
  - [ ] fn `suprnova::TaskBuilder::try_timezone` · framework/src/schedule/builder.rs:577
  - [ ] fn `suprnova::TaskBuilder::without_overlapping` · framework/src/schedule/builder.rs:602
  - [ ] fn `suprnova::TaskBuilder::without_overlapping_for` · framework/src/schedule/builder.rs:613
  - [ ] fn `suprnova::TaskBuilder::on_one_server` · framework/src/schedule/builder.rs:653
  - [ ] fn `suprnova::TaskBuilder::on_one_server_for` · framework/src/schedule/builder.rs:665
  - [ ] fn `suprnova::TaskBuilder::run_in_background` · framework/src/schedule/builder.rs:675

### `suprnova::schedule::expression`

- [ ] struct `suprnova::CronExpression` · framework/src/schedule/expression.rs:63 (also `suprnova::schedule::CronExpression`, `suprnova::schedule::expression::CronExpression`)
  - [ ] fn `suprnova::CronExpression::parse` · framework/src/schedule/expression.rs:287
  - [ ] fn `suprnova::CronExpression::is_due` · framework/src/schedule/expression.rs:313
  - [ ] fn `suprnova::CronExpression::is_due_at` · framework/src/schedule/expression.rs:332
  - [ ] fn `suprnova::CronExpression::next_run_after` · framework/src/schedule/expression.rs:407
  - [ ] fn `suprnova::CronExpression::expression` · framework/src/schedule/expression.rs:430
  - [ ] fn `suprnova::CronExpression::at` · framework/src/schedule/expression.rs:474
  - [ ] fn `suprnova::CronExpression::try_at` · framework/src/schedule/expression.rs:505
  - [ ] fn `suprnova::CronExpression::every_minute` · framework/src/schedule/expression.rs:521
  - [ ] fn `suprnova::CronExpression::every_n_minutes` · framework/src/schedule/expression.rs:533
  - [ ] fn `suprnova::CronExpression::try_every_n_minutes` · framework/src/schedule/expression.rs:549
  - [ ] fn `suprnova::CronExpression::hourly` · framework/src/schedule/expression.rs:559
  - [ ] fn `suprnova::CronExpression::hourly_at` · framework/src/schedule/expression.rs:569
  - [ ] fn `suprnova::CronExpression::try_hourly_at` · framework/src/schedule/expression.rs:580
  - [ ] fn `suprnova::CronExpression::daily` · framework/src/schedule/expression.rs:590
  - [ ] fn `suprnova::CronExpression::daily_at` · framework/src/schedule/expression.rs:605
  - [ ] fn `suprnova::CronExpression::try_daily_at` · framework/src/schedule/expression.rs:620
  - [ ] fn `suprnova::CronExpression::weekly` · framework/src/schedule/expression.rs:638
  - [ ] fn `suprnova::CronExpression::weekly_on` · framework/src/schedule/expression.rs:643
  - [ ] fn `suprnova::CronExpression::on_days` · framework/src/schedule/expression.rs:648
  - [ ] fn `suprnova::CronExpression::monthly` · framework/src/schedule/expression.rs:654
  - [ ] fn `suprnova::CronExpression::monthly_on` · framework/src/schedule/expression.rs:665
  - [ ] fn `suprnova::CronExpression::try_monthly_on` · framework/src/schedule/expression.rs:677
  - [ ] fn `suprnova::CronExpression::quarterly` · framework/src/schedule/expression.rs:685
  - [ ] fn `suprnova::CronExpression::yearly` · framework/src/schedule/expression.rs:690
  - [ ] fn `suprnova::CronExpression::weekdays` · framework/src/schedule/expression.rs:695
  - [ ] fn `suprnova::CronExpression::weekends` · framework/src/schedule/expression.rs:700
- [ ] enum `suprnova::DayOfWeek` · framework/src/schedule/expression.rs:10 (also `suprnova::schedule::DayOfWeek`, `suprnova::schedule::expression::DayOfWeek`)
  - Variants: `Sunday`, `Monday`, `Tuesday`, `Wednesday`, `Thursday`, `Friday`, `Saturday`
  - [ ] fn `suprnova::DayOfWeek::from_chrono` · framework/src/schedule/expression.rs:29

### `suprnova::schedule::task`

- [ ] struct `suprnova::TaskEntry` · framework/src/schedule/task.rs:154 (also `suprnova::schedule::TaskEntry`, `suprnova::schedule::task::TaskEntry`)
  - Public fields: `name`, `expression`, `task`, `description`, `without_overlapping`, `run_in_background`, `overlap_ttl`, `on_one_server`, `one_server_ttl`, `timezone`, `state`
  - [ ] fn `suprnova::TaskEntry::is_due` · framework/src/schedule/task.rs:196
  - [ ] fn `suprnova::TaskEntry::run` · framework/src/schedule/task.rs:225
  - [ ] fn `suprnova::TaskEntry::schedule_description` · framework/src/schedule/task.rs:239
- [ ] struct `suprnova::schedule::task::TaskState` · framework/src/schedule/task.rs:46
  - [ ] fn `suprnova::schedule::task::TaskState::new` · framework/src/schedule/task.rs:71
  - [ ] fn `suprnova::schedule::task::TaskState::skip_count` · framework/src/schedule/task.rs:77
- [ ] trait `suprnova::Task` · framework/src/schedule/task.rs:137 (also `suprnova::schedule::Task`, `suprnova::schedule::task::Task`)
  - [ ] fn `suprnova::Task::handle` · framework/src/schedule/task.rs:139 (required)
- [ ] trait `suprnova::schedule::TaskHandler` · framework/src/schedule/task.rs:95 (also `suprnova::schedule::task::TaskHandler`)
  - [ ] fn `suprnova::schedule::TaskHandler::handle` · framework/src/schedule/task.rs:97 (required)
- [ ] type `suprnova::schedule::BoxedFuture` · framework/src/schedule/task.rs:89 (also `suprnova::schedule::task::BoxedFuture`)
- [ ] type `suprnova::schedule::BoxedTask` · framework/src/schedule/task.rs:83 (also `suprnova::schedule::task::BoxedTask`)
- [ ] type `suprnova::TaskResult` · framework/src/schedule/task.rs:86 (also `suprnova::schedule::TaskResult`, `suprnova::schedule::task::TaskResult`)
- [ ] const `suprnova::schedule::task::DEFAULT_ON_ONE_SERVER_TTL` · framework/src/schedule/task.rs:32
- [ ] const `suprnova::schedule::task::DEFAULT_WITHOUT_OVERLAPPING_TTL` · framework/src/schedule/task.rs:20

## seed

### `suprnova::seed`

- [ ] fn `suprnova::seed::count` · framework/src/seed/mod.rs:200
- [ ] fn `suprnova::seed::is_registered` · framework/src/seed/mod.rs:215
- [ ] fn `suprnova::seed::register` · framework/src/seed/mod.rs:130
- [ ] fn `suprnova::seed::run_all` · framework/src/seed/mod.rs:150
- [ ] fn `suprnova::seed::run_one` · framework/src/seed/mod.rs:178
- [ ] fn `suprnova::seed::without_events` · framework/src/seed/mod.rs:286
- [ ] trait `suprnova::Seeder` · framework/src/seed/mod.rs:109 (also `suprnova::seed::Seeder`)
  - [ ] fn `suprnova::Seeder::name` · framework/src/seed/mod.rs:113 (required)
  - [ ] fn `suprnova::Seeder::run` · framework/src/seed/mod.rs:120 (required)

## server

### `suprnova::server`

- [ ] fn `suprnova::handle_request` · framework/src/server.rs:687 (also `suprnova::server::handle_request`)
- [ ] fn `suprnova::handle_request_with_peer` · framework/src/server.rs:701 (also `suprnova::server::handle_request_with_peer`)
- [ ] struct `suprnova::Server` · framework/src/server.rs:73 (also `suprnova::server::Server`)
  - [ ] fn `suprnova::Server::new` · framework/src/server.rs:104
  - [ ] fn `suprnova::Server::from_config` · framework/src/server.rs:145
  - [ ] fn `suprnova::Server::try_from_config_with_routes` · framework/src/server.rs:158
  - [ ] fn `suprnova::Server::try_from_config_with_routes_async` · framework/src/server.rs:188
  - [ ] fn `suprnova::Server::middleware` · framework/src/server.rs:301
  - [ ] fn `suprnova::Server::host` · framework/src/server.rs:308
  - [ ] fn `suprnova::Server::port` · framework/src/server.rs:314
  - [ ] fn `suprnova::Server::max_connections` · framework/src/server.rs:330
  - [ ] fn `suprnova::Server::header_read_timeout` · framework/src/server.rs:340
  - [ ] fn `suprnova::Server::run` · framework/src/server.rs:375

## session

### `suprnova::session`

- [ ] fn `suprnova::destroy_all_for_user` · framework/src/session/mod.rs:87 (also `suprnova::session::destroy_all_for_user`)

### `suprnova::session::blocking`

- [ ] struct `suprnova::SessionBlock` · framework/src/session/blocking.rs:33 (also `suprnova::session::SessionBlock`, `suprnova::session::blocking::SessionBlock`)
  - [ ] fn `suprnova::SessionBlock::new` · framework/src/session/blocking.rs:51
  - [ ] fn `suprnova::SessionBlock::lock_for` · framework/src/session/blocking.rs:59
  - [ ] fn `suprnova::SessionBlock::wait_for` · framework/src/session/blocking.rs:64

### `suprnova::session::config`

- [ ] struct `suprnova::SessionConfig` · framework/src/session/config.rs:32 (also `suprnova::session::SessionConfig`, `suprnova::session::config::SessionConfig`)
  - Public fields: `lifetime`, `touch_interval`, `gc_interval`, `cookie_name`, `cookie_path`, `cookie_domain`, `cookie_secure`, `cookie_http_only`, `cookie_same_site`, `cookie_partitioned`, `cookie_prefix`, `expire_on_close`, `table_name`, `connection`, `remember_lifetime`, `block`
  - [ ] fn `suprnova::SessionConfig::new` · framework/src/session/config.rs:121
  - [ ] fn `suprnova::SessionConfig::from_env` · framework/src/session/config.rs:155
  - [ ] fn `suprnova::SessionConfig::lifetime` · framework/src/session/config.rs:240
  - [ ] fn `suprnova::SessionConfig::touch_interval` · framework/src/session/config.rs:246
  - [ ] fn `suprnova::SessionConfig::gc_interval` · framework/src/session/config.rs:252
  - [ ] fn `suprnova::SessionConfig::cookie_name` · framework/src/session/config.rs:258
  - [ ] fn `suprnova::SessionConfig::secure` · framework/src/session/config.rs:264
  - [ ] fn `suprnova::SessionConfig::remember_lifetime` · framework/src/session/config.rs:270
  - [ ] fn `suprnova::SessionConfig::domain` · framework/src/session/config.rs:276
  - [ ] fn `suprnova::SessionConfig::partitioned` · framework/src/session/config.rs:282
  - [ ] fn `suprnova::SessionConfig::expire_on_close` · framework/src/session/config.rs:289
  - [ ] fn `suprnova::SessionConfig::connection` · framework/src/session/config.rs:295
  - [ ] fn `suprnova::SessionConfig::block` · framework/src/session/config.rs:302
- [ ] const `suprnova::session::MAX_SESSION_LIFETIME_MINUTES` · framework/src/session/config.rs:23 (also `suprnova::session::config::MAX_SESSION_LIFETIME_MINUTES`)
- [ ] const `suprnova::session::MAX_SESSION_LIFETIME_SECS` · framework/src/session/config.rs:18 (also `suprnova::session::config::MAX_SESSION_LIFETIME_SECS`)

### `suprnova::session::driver::database`

- [ ] struct `suprnova::DatabaseSessionDriver` · framework/src/session/driver/database.rs:25 (also `suprnova::session::DatabaseSessionDriver`, `suprnova::session::driver::DatabaseSessionDriver`, `suprnova::session::driver::database::DatabaseSessionDriver`)
  - Implements: `suprnova::SessionStore`
  - [ ] fn `suprnova::DatabaseSessionDriver::new` · framework/src/session/driver/database.rs:31

### `suprnova::session::driver::database::sessions`

- [ ] struct `suprnova::session::driver::database::sessions::ActiveModel` · framework/src/session/driver/database.rs:336
  - Public fields: `id`, `user_id`, `payload`, `csrf_token`, `last_activity`
- [ ] struct `suprnova::session::driver::database::sessions::ColumnIter` · framework/src/session/driver/database.rs:336
- [ ] struct `suprnova::session::driver::database::sessions::Entity` · framework/src/session/driver/database.rs:336
- [ ] struct `suprnova::session::driver::database::sessions::Model` · framework/src/session/driver/database.rs:338
  - Public fields: `id`, `user_id`, `payload`, `csrf_token`, `last_activity`
  - [ ] fn `suprnova::session::driver::database::sessions::Model::into_ex` · framework/src/session/driver/database.rs:336
- [ ] struct `suprnova::session::driver::database::sessions::PrimaryKeyIter` · framework/src/session/driver/database.rs:336
- [ ] struct `suprnova::session::driver::database::sessions::RelationIter` · framework/src/session/driver/database.rs:355
- [ ] enum `suprnova::session::driver::database::sessions::Column` · framework/src/session/driver/database.rs:336
  - Variants: `Id`, `UserId`, `Payload`, `CsrfToken`, `LastActivity`
- [ ] enum `suprnova::session::driver::database::sessions::PrimaryKey` · framework/src/session/driver/database.rs:336
  - Variants: `Id`
- [ ] enum `suprnova::session::driver::database::sessions::Relation` · framework/src/session/driver/database.rs:356

### `suprnova::session::middleware`

- [ ] fn `suprnova::auth_user_id` · framework/src/session/middleware.rs:2202 (also `suprnova::session::auth_user_id`, `suprnova::session::middleware::auth_user_id`)
- [ ] fn `suprnova::clear_auth_user` · framework/src/session/middleware.rs:2256 (also `suprnova::session::clear_auth_user`, `suprnova::session::middleware::clear_auth_user`)
- [ ] fn `suprnova::session::clear_two_factor_pending` · framework/src/session/middleware.rs:2300 (also `suprnova::session::middleware::clear_two_factor_pending`)
- [ ] fn `suprnova::session::clear_two_factor_pending_remember` · framework/src/session/middleware.rs:2361 (also `suprnova::session::middleware::clear_two_factor_pending_remember`)
- [ ] fn `suprnova::generate_csrf_token` · framework/src/session/middleware.rs:465 (also `suprnova::session::generate_csrf_token`, `suprnova::session::middleware::generate_csrf_token`)
- [ ] fn `suprnova::generate_session_id` · framework/src/session/middleware.rs:450 (also `suprnova::session::generate_session_id`, `suprnova::session::middleware::generate_session_id`)
- [ ] fn `suprnova::get_csrf_token` · framework/src/session/middleware.rs:2079 (also `suprnova::session::get_csrf_token`, `suprnova::session::middleware::get_csrf_token`)
- [ ] fn `suprnova::invalidate_session` · framework/src/session/middleware.rs:2070 (also `suprnova::session::invalidate_session`, `suprnova::session::middleware::invalidate_session`)
- [ ] fn `suprnova::is_authenticated` · framework/src/session/middleware.rs:2097 (also `suprnova::session::is_authenticated`, `suprnova::session::middleware::is_authenticated`)
- [ ] fn `suprnova::regenerate_csrf_token` · framework/src/session/middleware.rs:2087 (also `suprnova::session::middleware::regenerate_csrf_token`, `suprnova::session::regenerate_csrf_token`)
- [ ] fn `suprnova::regenerate_session_id` · framework/src/session/middleware.rs:2056 (also `suprnova::session::middleware::regenerate_session_id`, `suprnova::session::regenerate_session_id`)
- [ ] fn `suprnova::session` · framework/src/session/middleware.rs:410 (also `suprnova::session::middleware::session`, `suprnova::session::session`)
- [ ] fn `suprnova::session::session_gc_metrics` · framework/src/session/middleware.rs:543 (also `suprnova::session::middleware::session_gc_metrics`)
- [ ] fn `suprnova::session_mut` · framework/src/session/middleware.rs:429 (also `suprnova::session::middleware::session_mut`, `suprnova::session::session_mut`)
- [ ] fn `suprnova::set_auth_user` · framework/src/session/middleware.rs:2237 (also `suprnova::session::middleware::set_auth_user`, `suprnova::session::set_auth_user`)
- [ ] fn `suprnova::session::set_two_factor_pending` · framework/src/session/middleware.rs:2286 (also `suprnova::session::middleware::set_two_factor_pending`)
- [ ] fn `suprnova::session::set_two_factor_pending_remember` · framework/src/session/middleware.rs:2343 (also `suprnova::session::middleware::set_two_factor_pending_remember`)
- [ ] fn `suprnova::session::two_factor_pending_remember` · framework/src/session/middleware.rs:2326 (also `suprnova::session::middleware::two_factor_pending_remember`)
- [ ] fn `suprnova::session::two_factor_pending_user_id` · framework/src/session/middleware.rs:2274 (also `suprnova::session::middleware::two_factor_pending_user_id`)
- [ ] struct `suprnova::session::SessionGcMetrics` · framework/src/session/middleware.rs:527 (also `suprnova::session::middleware::SessionGcMetrics`)
  - Public fields: `runs`, `successes`, `failures`, `removed_rows`, `last_success_unix_seconds`, `last_failure_unix_seconds`
- [ ] struct `suprnova::SessionGcSupervisor` · framework/src/session/middleware.rs:975 (also `suprnova::session::SessionGcSupervisor`, `suprnova::session::middleware::SessionGcSupervisor`)
  - Public fields: `store`, `interval`
  - Implements: `suprnova::Supervisor`
- [ ] struct `suprnova::SessionMiddleware` · framework/src/session/middleware.rs:562 (also `suprnova::session::SessionMiddleware`, `suprnova::session::middleware::SessionMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::SessionMiddleware::new` · framework/src/session/middleware.rs:607
  - [ ] fn `suprnova::SessionMiddleware::with_store` · framework/src/session/middleware.rs:614
  - [ ] fn `suprnova::SessionMiddleware::install_with_gc` · framework/src/session/middleware.rs:636
  - [ ] fn `suprnova::SessionMiddleware::install` · framework/src/session/middleware.rs:649
  - [ ] fn `suprnova::SessionMiddleware::store` · framework/src/session/middleware.rs:656

### `suprnova::session::store`

- [ ] fn `suprnova::is_valid_session_id` · framework/src/session/store.rs:744 (also `suprnova::session::is_valid_session_id`, `suprnova::session::store::is_valid_session_id`)
- [ ] struct `suprnova::SessionData` · framework/src/session/store.rs:24 (also `suprnova::session::SessionData`, `suprnova::session::store::SessionData`)
  - Public fields: `id`, `data`, `user_id`, `csrf_token`, `dirty`, `loaded_from_store`
  - [ ] fn `suprnova::SessionData::new` · framework/src/session/store.rs:57
  - [ ] fn `suprnova::SessionData::rotate_id` · framework/src/session/store.rs:82
  - [ ] fn `suprnova::SessionData::get` · framework/src/session/store.rs:225
  - [ ] fn `suprnova::SessionData::put` · framework/src/session/store.rs:241
  - [ ] fn `suprnova::SessionData::forget` · framework/src/session/store.rs:251
  - [ ] fn `suprnova::SessionData::has` · framework/src/session/store.rs:263
  - [ ] fn `suprnova::SessionData::flash` · framework/src/session/store.rs:276
  - [ ] fn `suprnova::SessionData::get_flash` · framework/src/session/store.rs:281
  - [ ] fn `suprnova::SessionData::age_flash_data` · framework/src/session/store.rs:291
  - [ ] fn `suprnova::SessionData::flush` · framework/src/session/store.rs:325
  - [ ] fn `suprnova::SessionData::set_magnetar_web_binding` · framework/src/session/store.rs:332
  - [ ] fn `suprnova::SessionData::magnetar_web_binding` · framework/src/session/store.rs:337
  - [ ] fn `suprnova::SessionData::clear_magnetar_web_binding` · framework/src/session/store.rs:352
  - [ ] fn `suprnova::SessionData::is_dirty` · framework/src/session/store.rs:357
  - [ ] fn `suprnova::SessionData::mark_clean` · framework/src/session/store.rs:362
  - [ ] fn `suprnova::SessionData::pull` · framework/src/session/store.rs:375
  - [ ] fn `suprnova::SessionData::push` · framework/src/session/store.rs:387
  - [ ] fn `suprnova::SessionData::increment` · framework/src/session/store.rs:403
  - [ ] fn `suprnova::SessionData::decrement` · framework/src/session/store.rs:412
  - [ ] fn `suprnova::SessionData::remember` · framework/src/session/store.rs:419
  - [ ] fn `suprnova::SessionData::has_any` · framework/src/session/store.rs:434
  - [ ] fn `suprnova::SessionData::has_all` · framework/src/session/store.rs:440
  - [ ] fn `suprnova::SessionData::missing` · framework/src/session/store.rs:446
  - [ ] fn `suprnova::SessionData::all` · framework/src/session/store.rs:454
  - [ ] fn `suprnova::SessionData::only` · framework/src/session/store.rs:460
  - [ ] fn `suprnova::SessionData::except` · framework/src/session/store.rs:472
  - [ ] fn `suprnova::SessionData::replace` · framework/src/session/store.rs:489
  - [ ] fn `suprnova::SessionData::put_many` · framework/src/session/store.rs:503
  - [ ] fn `suprnova::SessionData::forget_many` · framework/src/session/store.rs:514
  - [ ] fn `suprnova::SessionData::now` · framework/src/session/store.rs:525
  - [ ] fn `suprnova::SessionData::reflash` · framework/src/session/store.rs:534
  - [ ] fn `suprnova::SessionData::keep` · framework/src/session/store.rs:555
  - [ ] fn `suprnova::SessionData::flash_input` · framework/src/session/store.rs:574
  - [ ] fn `suprnova::SessionData::old_input` · framework/src/session/store.rs:582
  - [ ] fn `suprnova::SessionData::get_old_input` · framework/src/session/store.rs:599
  - [ ] fn `suprnova::SessionData::has_old_input` · framework/src/session/store.rs:608
  - [ ] fn `suprnova::SessionData::pull_errors_flash` · framework/src/session/store.rs:631
  - [ ] fn `suprnova::SessionData::previous_url` · framework/src/session/store.rs:675
  - [ ] fn `suprnova::SessionData::set_previous_url` · framework/src/session/store.rs:682
  - [ ] fn `suprnova::SessionData::previous_route` · framework/src/session/store.rs:688
  - [ ] fn `suprnova::SessionData::set_previous_route` · framework/src/session/store.rs:694
  - [ ] fn `suprnova::SessionData::has_previous_uri` · framework/src/session/store.rs:701
  - [ ] fn `suprnova::SessionData::password_confirmed` · framework/src/session/store.rs:710
  - [ ] fn `suprnova::SessionData::password_confirmed_at` · framework/src/session/store.rs:717
- [ ] enum `suprnova::SessionMigrationError` · framework/src/session/store.rs:756 (also `suprnova::session::SessionMigrationError`, `suprnova::session::store::SessionMigrationError`)
  - Variants: `RolledBack`, `OutcomeUnknown`
- [ ] trait `suprnova::SessionStore` · framework/src/session/store.rs:782 (also `suprnova::session::SessionStore`, `suprnova::session::store::SessionStore`)
  - Implemented here by: `DatabaseSessionDriver`
  - [ ] fn `suprnova::SessionStore::read` · framework/src/session/store.rs:786 (required)
  - [ ] fn `suprnova::SessionStore::write` · framework/src/session/store.rs:791 (required)
  - [ ] fn `suprnova::SessionStore::migrate_two_factor_session` · framework/src/session/store.rs:809 (provided)
  - [ ] fn `suprnova::SessionStore::destroy` · framework/src/session/store.rs:820 (required)
  - [ ] fn `suprnova::SessionStore::destroy_for_user` · framework/src/session/store.rs:828 (required)
  - [ ] fn `suprnova::SessionStore::gc` · framework/src/session/store.rs:833 (required)

## sse

### `suprnova::sse`

- [ ] fn `suprnova::sse::last_event_id` · framework/src/sse/mod.rs:627
- [ ] fn `suprnova::sse::last_event_id_from_value` · framework/src/sse/mod.rs:602
- [ ] struct `suprnova::SseEvent` · framework/src/sse/mod.rs:134 (also `suprnova::sse::SseEvent`)
  - [ ] fn `suprnova::SseEvent::data` · framework/src/sse/mod.rs:157
  - [ ] fn `suprnova::SseEvent::json` · framework/src/sse/mod.rs:171
  - [ ] fn `suprnova::SseEvent::comment` · framework/src/sse/mod.rs:198
  - [ ] fn `suprnova::SseEvent::keep_alive` · framework/src/sse/mod.rs:211
  - [ ] fn `suprnova::SseEvent::error` · framework/src/sse/mod.rs:227
  - [ ] fn `suprnova::SseEvent::with_event` · framework/src/sse/mod.rs:247
  - [ ] fn `suprnova::SseEvent::with_id` · framework/src/sse/mod.rs:261
  - [ ] fn `suprnova::SseEvent::with_retry` · framework/src/sse/mod.rs:276
  - [ ] fn `suprnova::SseEvent::try_with_event` · framework/src/sse/mod.rs:293
  - [ ] fn `suprnova::SseEvent::try_with_id` · framework/src/sse/mod.rs:304
  - [ ] fn `suprnova::SseEvent::event` · framework/src/sse/mod.rs:317
  - [ ] fn `suprnova::SseEvent::id` · framework/src/sse/mod.rs:326
  - [ ] fn `suprnova::SseEvent::retry` · framework/src/sse/mod.rs:335
  - [ ] fn `suprnova::SseEvent::payload` · framework/src/sse/mod.rs:350
  - [ ] fn `suprnova::SseEvent::is_comment` · framework/src/sse/mod.rs:359
  - [ ] fn `suprnova::SseEvent::comment_text` · framework/src/sse/mod.rs:365
  - [ ] fn `suprnova::SseEvent::to_wire` · framework/src/sse/mod.rs:411
- [ ] struct `suprnova::StreamedEvent` · framework/src/sse/mod.rs:494 (also `suprnova::sse::StreamedEvent`)
  - Public fields: `event`, `data`
  - [ ] fn `suprnova::StreamedEvent::message` · framework/src/sse/mod.rs:505
  - [ ] fn `suprnova::StreamedEvent::named` · framework/src/sse/mod.rs:513
- [ ] enum `suprnova::EndSignal` · framework/src/sse/mod.rs:539 (also `suprnova::sse::EndSignal`)
  - Variants: `None`, `Message`, `Event`
  - [ ] fn `suprnova::EndSignal::text` · framework/src/sse/mod.rs:550

## static_files

### `suprnova::static_files`

- [ ] struct `suprnova::StaticFiles` · framework/src/static_files.rs:33 (also `suprnova::static_files::StaticFiles`)
  - [ ] fn `suprnova::StaticFiles::public` · framework/src/static_files.rs:40
  - [ ] fn `suprnova::StaticFiles::from_dir` · framework/src/static_files.rs:45
  - [ ] fn `suprnova::StaticFiles::cache_control` · framework/src/static_files.rs:53
  - [ ] fn `suprnova::StaticFiles::handler` · framework/src/static_files.rs:59

## supervisor

### `suprnova::supervisor`

- [ ] fn `suprnova::supervisor::run_with_restart_for_testing` · framework/src/supervisor/mod.rs:498
- [ ] fn `suprnova::supervisor::run_with_restart_for_testing_with_cancel` · framework/src/supervisor/mod.rs:507
- [ ] fn `suprnova::supervisor::supervisor_cancel_token` · framework/src/supervisor/mod.rs:155
- [ ] fn `suprnova::supervisor::supervisor_tasks` · framework/src/supervisor/mod.rs:148
- [ ] struct `suprnova::SupervisorRegistry` · framework/src/supervisor/mod.rs:224 (also `suprnova::supervisor::SupervisorRegistry`)
  - [ ] fn `suprnova::SupervisorRegistry::start_all` · framework/src/supervisor/mod.rs:239
  - [ ] fn `suprnova::SupervisorRegistry::spawn` · framework/src/supervisor/mod.rs:279
  - [ ] fn `suprnova::SupervisorRegistry::shutdown` · framework/src/supervisor/mod.rs:305
- [ ] enum `suprnova::RestartPolicy` · framework/src/supervisor/mod.rs:207 (also `suprnova::supervisor::RestartPolicy`)
  - Variants: `OnError`, `Always`, `Never`
- [ ] trait `suprnova::Supervisor` · framework/src/supervisor/mod.rs:174 (also `suprnova::supervisor::Supervisor`)
  - Implemented here by: `SessionGcSupervisor`
  - [ ] fn `suprnova::Supervisor::name` · framework/src/supervisor/mod.rs:176 (required)
  - [ ] fn `suprnova::Supervisor::run` · framework/src/supervisor/mod.rs:193 (required)
  - [ ] fn `suprnova::Supervisor::restart_policy` · framework/src/supervisor/mod.rs:198 (provided)

### `suprnova::supervisor::registry`

- [ ] struct `suprnova::SupervisorEntry` · framework/src/supervisor/registry.rs:30 (also `suprnova::supervisor::SupervisorEntry`, `suprnova::supervisor::registry::SupervisorEntry`)
  - Public fields: `factory`

## telemetry

### `suprnova::telemetry::init`

- [ ] fn `suprnova::init_telemetry` · framework/src/telemetry/init.rs:239 (also `suprnova::telemetry::init::init_telemetry`, `suprnova::telemetry::init_telemetry`)
- [ ] struct `suprnova::OtelConfig` · framework/src/telemetry/init.rs:38 (also `suprnova::telemetry::OtelConfig`, `suprnova::telemetry::init::OtelConfig`)
  - Public fields: `endpoint`, `service_name`, `service_version`, `disabled`
  - [ ] fn `suprnova::OtelConfig::from_env` · framework/src/telemetry/init.rs:53
  - [ ] fn `suprnova::OtelConfig::disabled` · framework/src/telemetry/init.rs:76
  - [ ] fn `suprnova::OtelConfig::is_enabled` · framework/src/telemetry/init.rs:87
- [ ] struct `suprnova::TelemetryGuard` · framework/src/telemetry/init.rs:119 (also `suprnova::telemetry::TelemetryGuard`, `suprnova::telemetry::init::TelemetryGuard`)
  - [ ] fn `suprnova::TelemetryGuard::shutdown` · framework/src/telemetry/init.rs:161

### `suprnova::telemetry::metrics::real` (private module; items are public through re-exports)

- [ ] struct `suprnova::CounterHandle` · framework/src/telemetry/metrics.rs:71 (also `suprnova::telemetry::CounterHandle`, `suprnova::telemetry::metrics::CounterHandle`)
  - [ ] fn `suprnova::CounterHandle::inc` · framework/src/telemetry/metrics.rs:75
  - [ ] fn `suprnova::CounterHandle::inc_by` · framework/src/telemetry/metrics.rs:79
  - [ ] fn `suprnova::CounterHandle::inc_with` · framework/src/telemetry/metrics.rs:83
- [ ] struct `suprnova::GaugeHandle` · framework/src/telemetry/metrics.rs:105 (also `suprnova::telemetry::GaugeHandle`, `suprnova::telemetry::metrics::GaugeHandle`)
  - [ ] fn `suprnova::GaugeHandle::set` · framework/src/telemetry/metrics.rs:110
  - [ ] fn `suprnova::GaugeHandle::set_with` · framework/src/telemetry/metrics.rs:114
- [ ] struct `suprnova::HistogramHandle` · framework/src/telemetry/metrics.rs:90 (also `suprnova::telemetry::HistogramHandle`, `suprnova::telemetry::metrics::HistogramHandle`)
  - [ ] fn `suprnova::HistogramHandle::record` · framework/src/telemetry/metrics.rs:94
  - [ ] fn `suprnova::HistogramHandle::record_with` · framework/src/telemetry/metrics.rs:98
- [ ] struct `suprnova::Metrics` · framework/src/telemetry/metrics.rs:47 (also `suprnova::telemetry::Metrics`, `suprnova::telemetry::metrics::Metrics`)
  - [ ] fn `suprnova::Metrics::counter` · framework/src/telemetry/metrics.rs:51
  - [ ] fn `suprnova::Metrics::histogram` · framework/src/telemetry/metrics.rs:57
  - [ ] fn `suprnova::Metrics::gauge` · framework/src/telemetry/metrics.rs:63

### `suprnova::telemetry::propagation`

- [ ] fn `suprnova::telemetry::propagation::extract_w3c_trace_context` · framework/src/telemetry/propagation.rs:57 (feature: `otel`, off by default)
- [ ] fn `suprnova::telemetry::propagation::install_trace_context_propagator` · framework/src/telemetry/propagation.rs:24 (feature: `otel`, off by default; a stub with the same name exists when the feature is off)
- [ ] fn `suprnova::telemetry::propagation::join_upstream_trace` · framework/src/telemetry/propagation.rs:81 (feature: `otel`, off by default; a stub with the same name exists when the feature is off)

## testing

### `suprnova::testing`

- [ ] fn `suprnova::testing::install_test_encryption_key` · framework/src/testing/mod.rs:49 (feature: `testing`)
- [ ] fn `suprnova::testing::install_test_encryption_keyring` · framework/src/testing/mod.rs:75 (feature: `testing`)

### `suprnova::testing::expect` (private module; items are public through re-exports)

- [ ] fn `suprnova::testing::set_current_test_name` · framework/src/testing/expect.rs:13
- [ ] struct `suprnova::testing::Expect` · framework/src/testing/expect.rs:34
  - [ ] fn `suprnova::testing::Expect::new` · framework/src/testing/expect.rs:41
  - [ ] fn `suprnova::testing::Expect::to_equal` · framework/src/testing/expect.rs:57
  - [ ] fn `suprnova::testing::Expect::to_not_equal` · framework/src/testing/expect.rs:77
  - [ ] fn `suprnova::testing::Expect::to_be_true` · framework/src/testing/expect.rs:99
  - [ ] fn `suprnova::testing::Expect::to_be_false` · framework/src/testing/expect.rs:116
  - [ ] fn `suprnova::testing::Expect::to_be_some` · framework/src/testing/expect.rs:136
  - [ ] fn `suprnova::testing::Expect::to_be_none` · framework/src/testing/expect.rs:153
  - [ ] fn `suprnova::testing::Expect::to_contain_value` · framework/src/testing/expect.rs:173
  - [ ] fn `suprnova::testing::Expect::to_be_ok` · framework/src/testing/expect.rs:205
  - [ ] fn `suprnova::testing::Expect::to_be_err` · framework/src/testing/expect.rs:223
  - [ ] fn `suprnova::testing::Expect::to_contain` · framework/src/testing/expect.rs:244
  - [ ] fn `suprnova::testing::Expect::to_start_with` · framework/src/testing/expect.rs:263
  - [ ] fn `suprnova::testing::Expect::to_end_with` · framework/src/testing/expect.rs:282
  - [ ] fn `suprnova::testing::Expect::to_have_length` · framework/src/testing/expect.rs:301
  - [ ] fn `suprnova::testing::Expect::to_be_empty` · framework/src/testing/expect.rs:323
  - [ ] fn `suprnova::testing::Expect::to_contain` · framework/src/testing/expect.rs:337
  - [ ] fn `suprnova::testing::Expect::to_start_with` · framework/src/testing/expect.rs:349
  - [ ] fn `suprnova::testing::Expect::to_end_with` · framework/src/testing/expect.rs:361
  - [ ] fn `suprnova::testing::Expect::to_have_length` · framework/src/testing/expect.rs:373
  - [ ] fn `suprnova::testing::Expect::to_be_empty` · framework/src/testing/expect.rs:388
  - [ ] fn `suprnova::testing::Expect::to_have_length` · framework/src/testing/expect.rs:409
  - [ ] fn `suprnova::testing::Expect::to_contain` · framework/src/testing/expect.rs:431
  - [ ] fn `suprnova::testing::Expect::to_be_empty` · framework/src/testing/expect.rs:450
  - [ ] fn `suprnova::testing::Expect::to_be_greater_than` · framework/src/testing/expect.rs:470
  - [ ] fn `suprnova::testing::Expect::to_be_less_than` · framework/src/testing/expect.rs:488
  - [ ] fn `suprnova::testing::Expect::to_be_greater_than_or_equal` · framework/src/testing/expect.rs:506
  - [ ] fn `suprnova::testing::Expect::to_be_less_than_or_equal` · framework/src/testing/expect.rs:524

### `suprnova::testing::inertia` (private module; items are public through re-exports)

- [ ] struct `suprnova::testing::AssertableInertia` · framework/src/testing/inertia.rs:71
  - [ ] fn `suprnova::testing::AssertableInertia::from_response` · framework/src/testing/inertia.rs:100
  - [ ] fn `suprnova::testing::AssertableInertia::with_reload` · framework/src/testing/inertia.rs:176
  - [ ] fn `suprnova::testing::AssertableInertia::component` · framework/src/testing/inertia.rs:186
  - [ ] fn `suprnova::testing::AssertableInertia::url` · framework/src/testing/inertia.rs:198
  - [ ] fn `suprnova::testing::AssertableInertia::version` · framework/src/testing/inertia.rs:214
  - [ ] fn `suprnova::testing::AssertableInertia::prop` · framework/src/testing/inertia.rs:229
  - [ ] fn `suprnova::testing::AssertableInertia::has` · framework/src/testing/inertia.rs:234
  - [ ] fn `suprnova::testing::AssertableInertia::missing` · framework/src/testing/inertia.rs:245
  - [ ] fn `suprnova::testing::AssertableInertia::where_` · framework/src/testing/inertia.rs:256
  - [ ] fn `suprnova::testing::AssertableInertia::count` · framework/src/testing/inertia.rs:272
  - [ ] fn `suprnova::testing::AssertableInertia::has_flash` · framework/src/testing/inertia.rs:293
  - [ ] fn `suprnova::testing::AssertableInertia::reload_only` · framework/src/testing/inertia.rs:322
  - [ ] fn `suprnova::testing::AssertableInertia::reload_except` · framework/src/testing/inertia.rs:344
  - [ ] fn `suprnova::testing::AssertableInertia::load_deferred_props` · framework/src/testing/inertia.rs:368
- [ ] struct `suprnova::testing::ReloadRequest` · framework/src/testing/inertia.rs:448
  - Public fields: `url`, `component`, `version`, `only`, `except`
  - [ ] fn `suprnova::testing::ReloadRequest::headers` · framework/src/testing/inertia.rs:472

### `suprnova::testing::response` (private module; items are public through re-exports)

- [ ] struct `suprnova::testing::TestResponse` · framework/src/testing/response.rs:19
  - [ ] fn `suprnova::testing::TestResponse::new` · framework/src/testing/response.rs:36
  - [ ] fn `suprnova::testing::TestResponse::with_session_store` · framework/src/testing/response.rs:59
  - [ ] fn `suprnova::testing::TestResponse::status` · framework/src/testing/response.rs:69
  - [ ] fn `suprnova::testing::TestResponse::header` · framework/src/testing/response.rs:74
  - [ ] fn `suprnova::testing::TestResponse::cookie` · framework/src/testing/response.rs:94
  - [ ] fn `suprnova::testing::TestResponse::body_text` · framework/src/testing/response.rs:102
  - [ ] fn `suprnova::testing::TestResponse::json` · framework/src/testing/response.rs:112
  - [ ] fn `suprnova::testing::TestResponse::assert_status` · framework/src/testing/response.rs:122
  - [ ] fn `suprnova::testing::TestResponse::assert_ok` · framework/src/testing/response.rs:134
  - [ ] fn `suprnova::testing::TestResponse::assert_redirect` · framework/src/testing/response.rs:148
  - [ ] fn `suprnova::testing::TestResponse::assert_json` · framework/src/testing/response.rs:174
  - [ ] fn `suprnova::testing::TestResponse::assert_json_path` · framework/src/testing/response.rs:188
  - [ ] fn `suprnova::testing::TestResponse::assert_json_count` · framework/src/testing/response.rs:206
  - [ ] fn `suprnova::testing::TestResponse::assert_see` · framework/src/testing/response.rs:229
  - [ ] fn `suprnova::testing::TestResponse::assert_header` · framework/src/testing/response.rs:238
  - [ ] fn `suprnova::testing::TestResponse::assert_cookie` · framework/src/testing/response.rs:250
  - [ ] fn `suprnova::testing::TestResponse::assert_session_has` · framework/src/testing/response.rs:278
  - [ ] fn `suprnova::testing::TestResponse::assert_inertia` · framework/src/testing/response.rs:342

## timeout

### `suprnova::timeout`

- [ ] struct `suprnova::TimeoutMiddleware` · framework/src/timeout/mod.rs:94 (also `suprnova::timeout::TimeoutMiddleware`)
  - Implements: `suprnova::Middleware`
  - [ ] fn `suprnova::TimeoutMiddleware::new` · framework/src/timeout/mod.rs:100
  - [ ] fn `suprnova::TimeoutMiddleware::seconds` · framework/src/timeout/mod.rs:108
  - [ ] fn `suprnova::TimeoutMiddleware::duration` · framework/src/timeout/mod.rs:115
- [ ] const `suprnova::timeout::DEFAULT_TIMEOUT` · framework/src/timeout/mod.rs:86

## validation

### `suprnova::validation::message`

- [ ] struct `suprnova::ValidationMessage` · framework/src/validation/message.rs:29 (also `suprnova::validation::message::ValidationMessage`)
  - Public fields: `key`, `args`, `fallback`, `prefix`
  - [ ] fn `suprnova::ValidationMessage::keyed` · framework/src/validation/message.rs:47
  - [ ] fn `suprnova::ValidationMessage::arg` · framework/src/validation/message.rs:57
  - [ ] fn `suprnova::ValidationMessage::fallback` · framework/src/validation/message.rs:63
  - [ ] fn `suprnova::ValidationMessage::prefix` · framework/src/validation/message.rs:74
  - [ ] fn `suprnova::ValidationMessage::is_keyed` · framework/src/validation/message.rs:84
- [ ] type `suprnova::TranslateArgs` · framework/src/validation/message.rs:19 (also `suprnova::validation::message::TranslateArgs`)

### `suprnova::validation::rule`

- [ ] trait `suprnova::AsyncRule` · framework/src/validation/rule.rs:1873 (also `suprnova::validation::rule::AsyncRule`)
  - Implemented here by: `Password`, `Unique`
  - [ ] fn `suprnova::AsyncRule::passes` · framework/src/validation/rule.rs:1881 (required)
  - [ ] fn `suprnova::AsyncRule::check_async` · framework/src/validation/rule.rs:1906 (provided)
- [ ] trait `suprnova::ContextualRule` · framework/src/validation/rule.rs:163 (also `suprnova::validation::rule::ContextualRule`)
  - Implemented here by: `Confirmed`, `Different`, `Gt`, `Gte`, `Lt`, `Lte`, `RequiredIf`, `RequiredUnless`, `RequiredWith`, `RequiredWithAll`, `Same`
  - [ ] fn `suprnova::ContextualRule::passes` · framework/src/validation/rule.rs:178 (required)
  - [ ] fn `suprnova::ContextualRule::check` · framework/src/validation/rule.rs:190 (provided)
  - [ ] fn `suprnova::ContextualRule::check_named` · framework/src/validation/rule.rs:205 (provided)
- [ ] trait `suprnova::Rule` · framework/src/validation/rule.rs:97 (also `suprnova::validation::rule::Rule`)
  - Implemented here by: `Alpha`, `AlphaDash`, `AlphaNum`, `Between`, `Boolean`, `Email`, `HttpUrl`, `In`, `InArray`, `Integer`, `Max`, `Min`, `NotIn`, `Numeric`, `Password`, `Required`, `Url`, `UrlProtocols`, `Uuid`
  - [ ] fn `suprnova::Rule::passes` · framework/src/validation/rule.rs:107 (required)
  - [ ] fn `suprnova::Rule::check` · framework/src/validation/rule.rs:116 (provided)
- [ ] trait `suprnova::ValueRule` · framework/src/validation/rule.rs:139 (also `suprnova::validation::rule::ValueRule`)
  - Implemented here by: `ArrayKeys`, `Contains`, `Distinct`, `DoesntContain`
  - [ ] fn `suprnova::ValueRule::passes` · framework/src/validation/rule.rs:142 (required)
  - [ ] fn `suprnova::ValueRule::check` · framework/src/validation/rule.rs:145 (provided)
- [ ] type `suprnova::FormContext` · framework/src/validation/rule.rs:154 (also `suprnova::validation::rule::FormContext`)

### `suprnova::validation::rule::async_rules`

- [ ] struct `suprnova::Unique` · framework/src/validation/rule.rs:1970 (also `suprnova::async_rules::Unique`, `suprnova::validation::rule::Unique`, `suprnova::validation::rule::async_rules::Unique`)
  - Implements: `suprnova::AsyncRule`
  - [ ] fn `suprnova::Unique::new` · framework/src/validation/rule.rs:1980
  - [ ] fn `suprnova::Unique::ignore` · framework/src/validation/rule.rs:1996
  - [ ] fn `suprnova::Unique::ignore_with_column` · framework/src/validation/rule.rs:2004
  - [ ] fn `suprnova::Unique::where_eq` · framework/src/validation/rule.rs:2014
  - [ ] fn `suprnova::Unique::case_insensitive` · framework/src/validation/rule.rs:2022

### `suprnova::validation::rule::rules`

- [ ] struct `suprnova::Alpha` · framework/src/validation/rule.rs:397 (also `suprnova::rules::Alpha`, `suprnova::validation::rule::rules::Alpha`)
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::AlphaDash` · framework/src/validation/rule.rs:428 (also `suprnova::rules::AlphaDash`, `suprnova::validation::rule::rules::AlphaDash`)
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::AlphaNum` · framework/src/validation/rule.rs:412 (also `suprnova::rules::AlphaNum`, `suprnova::validation::rule::rules::AlphaNum`)
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::ArrayKeys` · framework/src/validation/rule.rs:1465 (also `suprnova::rules::ArrayKeys`, `suprnova::validation::rule::rules::ArrayKeys`)
  - Public tuple fields: 1
  - Implements: `suprnova::ValueRule`
- [ ] struct `suprnova::Between` · framework/src/validation/rule.rs:297 (also `suprnova::rules::Between`, `suprnova::validation::rule::rules::Between`)
  - Public tuple fields: 2
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::Boolean` · framework/src/validation/rule.rs:375 (also `suprnova::rules::Boolean`, `suprnova::validation::rule::rules::Boolean`)
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::Confirmed` · framework/src/validation/rule.rs:1419 (also `suprnova::rules::Confirmed`, `suprnova::validation::rule::rules::Confirmed`)
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::Contains` · framework/src/validation/rule.rs:1609 (also `suprnova::rules::Contains`, `suprnova::validation::rule::rules::Contains`)
  - Public tuple fields: 1
  - Implements: `suprnova::ValueRule`
- [ ] struct `suprnova::Different` · framework/src/validation/rule.rs:1367 (also `suprnova::rules::Different`, `suprnova::validation::rule::rules::Different`)
  - Public fields: `other`
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::Distinct` · framework/src/validation/rule.rs:1514 (also `suprnova::rules::Distinct`, `suprnova::validation::rule::rules::Distinct`)
  - Public fields: `ignore_case`, `strict`
  - Implements: `suprnova::ValueRule`
- [ ] struct `suprnova::DoesntContain` · framework/src/validation/rule.rs:1650 (also `suprnova::rules::DoesntContain`, `suprnova::validation::rule::rules::DoesntContain`)
  - Public tuple fields: 1
  - Implements: `suprnova::ValueRule`
- [ ] struct `suprnova::Email` · framework/src/validation/rule.rs:251 (also `suprnova::rules::Email`, `suprnova::validation::rule::rules::Email`)
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::Gt` · framework/src/validation/rule.rs:1770 (also `suprnova::rules::Gt`, `suprnova::validation::rule::rules::Gt`)
  - Public tuple fields: 1
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::Gte` · framework/src/validation/rule.rs:1787 (also `suprnova::rules::Gte`, `suprnova::validation::rule::rules::Gte`)
  - Public tuple fields: 1
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::HibpVerifier` · framework/src/validation/rule.rs:1116 (also `suprnova::rules::HibpVerifier`, `suprnova::validation::rule::rules::HibpVerifier`)
  - Implements: `suprnova::UncompromisedVerifier`
- [ ] struct `suprnova::HttpUrl` · framework/src/validation/rule.rs:677 (also `suprnova::rules::HttpUrl`, `suprnova::validation::rule::rules::HttpUrl`)
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::In` · framework/src/validation/rule.rs:317 (also `suprnova::rules::In`, `suprnova::validation::rule::rules::In`)
  - Public tuple fields: 1
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::InArray` · framework/src/validation/rule.rs:1579 (also `suprnova::rules::InArray`, `suprnova::validation::rule::rules::InArray`)
  - Public tuple fields: 1
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::Integer` · framework/src/validation/rule.rs:346 (also `suprnova::rules::Integer`, `suprnova::validation::rule::rules::Integer`)
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::Lt` · framework/src/validation/rule.rs:1804 (also `suprnova::rules::Lt`, `suprnova::validation::rule::rules::Lt`)
  - Public tuple fields: 1
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::Lte` · framework/src/validation/rule.rs:1821 (also `suprnova::rules::Lte`, `suprnova::validation::rule::rules::Lte`)
  - Public tuple fields: 1
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::Max` · framework/src/validation/rule.rs:282 (also `suprnova::rules::Max`, `suprnova::validation::rule::rules::Max`)
  - Public tuple fields: 1
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::Min` · framework/src/validation/rule.rs:266 (also `suprnova::rules::Min`, `suprnova::validation::rule::rules::Min`)
  - Public tuple fields: 1
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::NotIn` · framework/src/validation/rule.rs:332 (also `suprnova::rules::NotIn`, `suprnova::validation::rule::rules::NotIn`)
  - Public tuple fields: 1
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::Numeric` · framework/src/validation/rule.rs:361 (also `suprnova::rules::Numeric`, `suprnova::validation::rule::rules::Numeric`)
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::Password` · framework/src/validation/rule.rs:788 (also `suprnova::rules::Password`, `suprnova::validation::rule::rules::Password`)
  - Implements: `suprnova::AsyncRule`, `suprnova::Rule`
  - [ ] fn `suprnova::Password::min` · framework/src/validation/rule.rs:805
  - [ ] fn `suprnova::Password::max` · framework/src/validation/rule.rs:820
  - [ ] fn `suprnova::Password::letters` · framework/src/validation/rule.rs:826
  - [ ] fn `suprnova::Password::mixed_case` · framework/src/validation/rule.rs:833
  - [ ] fn `suprnova::Password::numbers` · framework/src/validation/rule.rs:839
  - [ ] fn `suprnova::Password::symbols` · framework/src/validation/rule.rs:848
  - [ ] fn `suprnova::Password::uncompromised` · framework/src/validation/rule.rs:871
  - [ ] fn `suprnova::Password::uncompromised_with_threshold` · framework/src/validation/rule.rs:882
  - [ ] fn `suprnova::Password::verifier` · framework/src/validation/rule.rs:892
  - [ ] fn `suprnova::Password::defaults_with` · framework/src/validation/rule.rs:910
  - [ ] fn `suprnova::Password::defaults` · framework/src/validation/rule.rs:922
- [ ] struct `suprnova::Required` · framework/src/validation/rule.rs:238 (also `suprnova::rules::Required`, `suprnova::validation::rule::rules::Required`)
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::RequiredIf` · framework/src/validation/rule.rs:1234 (also `suprnova::rules::RequiredIf`, `suprnova::validation::rule::rules::RequiredIf`)
  - Public fields: `other`, `value`
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::RequiredUnless` · framework/src/validation/rule.rs:1321 (also `suprnova::rules::RequiredUnless`, `suprnova::validation::rule::rules::RequiredUnless`)
  - Public fields: `other`, `value`
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::RequiredWith` · framework/src/validation/rule.rs:1263 (also `suprnova::rules::RequiredWith`, `suprnova::validation::rule::rules::RequiredWith`)
  - Public fields: `others`
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::RequiredWithAll` · framework/src/validation/rule.rs:1291 (also `suprnova::rules::RequiredWithAll`, `suprnova::validation::rule::rules::RequiredWithAll`)
  - Public fields: `others`
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::Same` · framework/src/validation/rule.rs:1349 (also `suprnova::rules::Same`, `suprnova::validation::rule::rules::Same`)
  - Public fields: `other`
  - Implements: `suprnova::ContextualRule`
- [ ] struct `suprnova::Url` · framework/src/validation/rule.rs:575 (also `suprnova::rules::Url`, `suprnova::validation::rule::rules::Url`)
  - Implements: `suprnova::Rule`
  - [ ] fn `suprnova::Url::protocols` · framework/src/validation/rule.rs:600
- [ ] struct `suprnova::UrlProtocols` · framework/src/validation/rule.rs:643 (also `suprnova::rules::UrlProtocols`, `suprnova::validation::rule::rules::UrlProtocols`)
  - Public tuple fields: 1
  - Implements: `suprnova::Rule`
- [ ] struct `suprnova::Uuid` · framework/src/validation/rule.rs:694 (also `suprnova::rules::Uuid`, `suprnova::validation::rule::rules::Uuid`)
  - Implements: `suprnova::Rule`
- [ ] enum `suprnova::CompareWith` · framework/src/validation/rule.rs:1692 (also `suprnova::rules::CompareWith`, `suprnova::validation::rule::rules::CompareWith`)
  - Variants: `Number`, `NumericField`, `LengthField`
- [ ] trait `suprnova::UncompromisedVerifier` · framework/src/validation/rule.rs:1054 (also `suprnova::rules::UncompromisedVerifier`, `suprnova::validation::rule::rules::UncompromisedVerifier`)
  - Implemented here by: `HibpVerifier`
  - [ ] fn `suprnova::UncompromisedVerifier::verify` · framework/src/validation/rule.rs:1078 (required)

## vector

### `suprnova::vector`

- [ ] struct `suprnova::Vector` · framework/src/vector/mod.rs:61 (also `suprnova::vector::Vector`)
  - [ ] fn `suprnova::Vector::register` · framework/src/vector/mod.rs:68
  - [ ] fn `suprnova::Vector::store` · framework/src/vector/mod.rs:75
  - [ ] fn `suprnova::Vector::registered_names` · framework/src/vector/mod.rs:80

### `suprnova::vector::driver`

- [ ] struct `suprnova::VectorItem` · framework/src/vector/driver.rs:9 (also `suprnova::vector::VectorItem`, `suprnova::vector::driver::VectorItem`)
  - Public fields: `id`, `embedding`, `metadata`
  - [ ] fn `suprnova::VectorItem::new` · framework/src/vector/driver.rs:22
- [ ] struct `suprnova::VectorMatch` · framework/src/vector/driver.rs:33 (also `suprnova::vector::VectorMatch`, `suprnova::vector::driver::VectorMatch`)
  - Public fields: `id`, `score`, `metadata`
- [ ] trait `suprnova::VectorDriver` · framework/src/vector/driver.rs:64 (also `suprnova::vector::VectorDriver`, `suprnova::vector::driver::VectorDriver`)
  - Implemented here by: `MariaDbVectorDriver`, `MemoryVectorDriver`, `PineconeVectorDriver`, `QdrantVectorDriver`
  - [ ] fn `suprnova::VectorDriver::upsert` · framework/src/vector/driver.rs:67 (required)
  - [ ] fn `suprnova::VectorDriver::similar` · framework/src/vector/driver.rs:71 (required)
  - [ ] fn `suprnova::VectorDriver::delete` · framework/src/vector/driver.rs:80 (required)
  - [ ] fn `suprnova::VectorDriver::count` · framework/src/vector/driver.rs:85 (required)

### `suprnova::vector::mariadb` (feature: `vector-mariadb`)

- [ ] struct `suprnova::MariaDbVectorDriver` · framework/src/vector/mariadb.rs:159 (also `suprnova::vector::MariaDbVectorDriver`, `suprnova::vector::mariadb::MariaDbVectorDriver`)
  - Implements: `suprnova::VectorDriver`
  - [ ] fn `suprnova::MariaDbVectorDriver::from_pool` · framework/src/vector/mariadb.rs:188
  - [ ] fn `suprnova::MariaDbVectorDriver::from_url` · framework/src/vector/mariadb.rs:204
  - [ ] fn `suprnova::MariaDbVectorDriver::with_distance` · framework/src/vector/mariadb.rs:212
  - [ ] fn `suprnova::MariaDbVectorDriver::pool` · framework/src/vector/mariadb.rs:219
  - [ ] fn `suprnova::MariaDbVectorDriver::distance` · framework/src/vector/mariadb.rs:224
  - [ ] fn `suprnova::MariaDbVectorDriver::validate_store_name` · framework/src/vector/mariadb.rs:234
  - [ ] fn `suprnova::MariaDbVectorDriver::ensure_table_sql_for` · framework/src/vector/mariadb.rs:272
  - [ ] fn `suprnova::MariaDbVectorDriver::ensure_table_sql` · framework/src/vector/mariadb.rs:291
  - [ ] fn `suprnova::MariaDbVectorDriver::embedding_to_vec_text` · framework/src/vector/mariadb.rs:321
  - [ ] fn `suprnova::MariaDbVectorDriver::score_from_distance` · framework/src/vector/mariadb.rs:351
- [ ] enum `suprnova::MariaDbDistance` · framework/src/vector/mariadb.rs:129 (also `suprnova::vector::MariaDbDistance`, `suprnova::vector::mariadb::MariaDbDistance`)
  - Variants: `Cosine`, `Euclidean`
  - [ ] fn `suprnova::MariaDbDistance::index_clause` · framework/src/vector/mariadb.rs:140
  - [ ] fn `suprnova::MariaDbDistance::fn_name` · framework/src/vector/mariadb.rs:150

### `suprnova::vector::memory`

- [ ] struct `suprnova::MemoryVectorDriver` · framework/src/vector/memory.rs:19 (also `suprnova::vector::MemoryVectorDriver`, `suprnova::vector::memory::MemoryVectorDriver`)
  - Implements: `suprnova::VectorDriver`
  - [ ] fn `suprnova::MemoryVectorDriver::new` · framework/src/vector/memory.rs:27

### `suprnova::vector::pinecone` (feature: `vector-pinecone`, off by default)

- [ ] struct `suprnova::vector::PineconeMatch` · framework/src/vector/pinecone.rs:160 (also `suprnova::vector::pinecone::PineconeMatch`)
  - Public fields: `id`, `score`, `metadata`
- [ ] struct `suprnova::vector::PineconeVector` · framework/src/vector/pinecone.rs:148 (also `suprnova::vector::pinecone::PineconeVector`)
  - Public fields: `id`, `values`, `metadata`
- [ ] struct `suprnova::PineconeVectorDriver` · framework/src/vector/pinecone.rs:174 (also `suprnova::vector::PineconeVectorDriver`, `suprnova::vector::pinecone::PineconeVectorDriver`)
  - Implements: `suprnova::VectorDriver`
  - [ ] fn `suprnova::PineconeVectorDriver::from_api_key` · framework/src/vector/pinecone.rs:204
  - [ ] fn `suprnova::PineconeVectorDriver::from_env` · framework/src/vector/pinecone.rs:229
  - [ ] fn `suprnova::PineconeVectorDriver::with_namespace` · framework/src/vector/pinecone.rs:248
  - [ ] fn `suprnova::PineconeVectorDriver::with_control_plane` · framework/src/vector/pinecone.rs:257
  - [ ] fn `suprnova::PineconeVectorDriver::with_api_version` · framework/src/vector/pinecone.rs:263
  - [ ] fn `suprnova::PineconeVectorDriver::with_index_host` · framework/src/vector/pinecone.rs:276
  - [ ] fn `suprnova::PineconeVectorDriver::namespace` · framework/src/vector/pinecone.rs:287
  - [ ] fn `suprnova::PineconeVectorDriver::control_plane` · framework/src/vector/pinecone.rs:292
  - [ ] fn `suprnova::PineconeVectorDriver::api_version` · framework/src/vector/pinecone.rs:297
  - [ ] fn `suprnova::PineconeVectorDriver::metadata_from_json` · framework/src/vector/pinecone.rs:309
  - [ ] fn `suprnova::PineconeVectorDriver::metadata_to_json` · framework/src/vector/pinecone.rs:324
  - [ ] fn `suprnova::PineconeVectorDriver::build_vector` · framework/src/vector/pinecone.rs:332
  - [ ] fn `suprnova::PineconeVectorDriver::decode_match` · framework/src/vector/pinecone.rs:341
  - [ ] fn `suprnova::PineconeVectorDriver::index_host` · framework/src/vector/pinecone.rs:363
  - [ ] fn `suprnova::PineconeVectorDriver::control_plane_get` · framework/src/vector/pinecone.rs:392
  - [ ] fn `suprnova::PineconeVectorDriver::control_plane_post` · framework/src/vector/pinecone.rs:404
  - [ ] fn `suprnova::PineconeVectorDriver::data_plane_post` · framework/src/vector/pinecone.rs:420
- [ ] const `suprnova::vector::DEFAULT_API_VERSION` · framework/src/vector/pinecone.rs:125 (also `suprnova::vector::pinecone::DEFAULT_API_VERSION`)
- [ ] const `suprnova::vector::DEFAULT_CONTROL_PLANE` · framework/src/vector/pinecone.rs:114 (also `suprnova::vector::pinecone::DEFAULT_CONTROL_PLANE`)

### `suprnova::vector::qdrant`

- [ ] struct `suprnova::QdrantVectorDriver` · framework/src/vector/qdrant.rs:104 (also `suprnova::vector::QdrantVectorDriver`, `suprnova::vector::qdrant::QdrantVectorDriver`)
  - Implements: `suprnova::VectorDriver`
  - [ ] fn `suprnova::QdrantVectorDriver::from_client` · framework/src/vector/qdrant.rs:115
  - [ ] fn `suprnova::QdrantVectorDriver::from_url` · framework/src/vector/qdrant.rs:126
  - [ ] fn `suprnova::QdrantVectorDriver::from_url_with_api_key` · framework/src/vector/qdrant.rs:135
  - [ ] fn `suprnova::QdrantVectorDriver::with_auto_create` · framework/src/vector/qdrant.rs:148
  - [ ] fn `suprnova::QdrantVectorDriver::with_distance` · framework/src/vector/qdrant.rs:155
  - [ ] fn `suprnova::QdrantVectorDriver::client` · framework/src/vector/qdrant.rs:163
  - [ ] fn `suprnova::QdrantVectorDriver::resolve_point_id` · framework/src/vector/qdrant.rs:170
  - [ ] fn `suprnova::QdrantVectorDriver::build_point` · framework/src/vector/qdrant.rs:189
  - [ ] fn `suprnova::QdrantVectorDriver::decode_match` · framework/src/vector/qdrant.rs:216
- [ ] enum `suprnova::QdrantDistance` · framework/src/vector/qdrant.rs:79 (also `suprnova::vector::QdrantDistance`, `suprnova::vector::qdrant::QdrantDistance`)
  - Variants: `Cosine`, `Euclidean`, `Dot`, `Manhattan`
- [ ] const `suprnova::SUPRNOVA_ID_PAYLOAD_KEY` · framework/src/vector/qdrant.rs:68 (also `suprnova::vector::SUPRNOVA_ID_PAYLOAD_KEY`, `suprnova::vector::qdrant::SUPRNOVA_ID_PAYLOAD_KEY`)

### `suprnova::vector::registry`

- [ ] struct `suprnova::VectorRegistry` · framework/src/vector/registry.rs:19 (also `suprnova::vector::VectorRegistry`, `suprnova::vector::registry::VectorRegistry`)
  - [ ] fn `suprnova::VectorRegistry::install` · framework/src/vector/registry.rs:30
  - [ ] fn `suprnova::VectorRegistry::lookup` · framework/src/vector/registry.rs:45
  - [ ] fn `suprnova::VectorRegistry::names` · framework/src/vector/registry.rs:64
- [ ] struct `suprnova::VectorStore` · framework/src/vector/registry.rs:84 (also `suprnova::vector::VectorStore`, `suprnova::vector::registry::VectorStore`)
  - [ ] fn `suprnova::VectorStore::name` · framework/src/vector/registry.rs:91
  - [ ] fn `suprnova::VectorStore::upsert` · framework/src/vector/registry.rs:96
  - [ ] fn `suprnova::VectorStore::similar` · framework/src/vector/registry.rs:101
  - [ ] fn `suprnova::VectorStore::delete` · framework/src/vector/registry.rs:110
  - [ ] fn `suprnova::VectorStore::count` · framework/src/vector/registry.rs:119

## view

### `suprnova::view`

- [ ] struct `suprnova::view::ViewRenderer` · framework/src/view/mod.rs:87
  - [ ] fn `suprnova::view::ViewRenderer::new` · framework/src/view/mod.rs:93
  - [ ] fn `suprnova::view::ViewRenderer::render_document` · framework/src/view/mod.rs:98
  - [ ] fn `suprnova::view::ViewRenderer::render_island` · framework/src/view/mod.rs:116
  - [ ] fn `suprnova::view::ViewRenderer::validate_island_fragment` · framework/src/view/mod.rs:132
  - [ ] fn `suprnova::view::ViewRenderer::validate_island_output` · framework/src/view/mod.rs:141
- [ ] enum `suprnova::view::TemplateFailure` · framework/src/view/mod.rs:32
  - Variants: `MissingData`, `InvalidData`, `Failed`
- [ ] trait `suprnova::view::ViewTemplate` · framework/src/view/mod.rs:61
  - [ ] fn `suprnova::view::ViewTemplate::render_view` · framework/src/view/mod.rs:63 (required)
- [ ] type `suprnova::view::FilterResult` · framework/src/view/mod.rs:24

### `suprnova::view::response` (private module; items are public through re-exports)

- [ ] fn `suprnova::view::document_response` · framework/src/view/response.rs:54
- [ ] struct `suprnova::view::DocumentResponseError` · framework/src/view/response.rs:19
  - [ ] fn `suprnova::view::DocumentResponseError::kind` · framework/src/view/response.rs:27
- [ ] enum `suprnova::view::DocumentResponseErrorKind` · framework/src/view/response.rs:12
  - Variants: `NonTextHeaderValue`

## workflow

### `suprnova::workflow`

- [ ] fn `suprnova::start_named` · framework/src/workflow/mod.rs:192 (also `suprnova::workflow::start_named`)
- [ ] struct `suprnova::WorkflowWorker` · framework/src/workflow/mod.rs:205 (also `suprnova::workflow::WorkflowWorker`)
  - [ ] fn `suprnova::WorkflowWorker::new` · framework/src/workflow/mod.rs:227
  - [ ] fn `suprnova::WorkflowWorker::with_config` · framework/src/workflow/mod.rs:248
  - [ ] fn `suprnova::WorkflowWorker::worker_id` · framework/src/workflow/mod.rs:258
  - [ ] fn `suprnova::WorkflowWorker::work_loop` · framework/src/workflow/mod.rs:267
  - [ ] fn `suprnova::WorkflowWorker::run_with_cancel` · framework/src/workflow/mod.rs:278

### `suprnova::workflow::config`

- [ ] struct `suprnova::WorkflowConfig` · framework/src/workflow/config.rs:40 (also `suprnova::workflow::WorkflowConfig`, `suprnova::workflow::config::WorkflowConfig`)
  - Public fields: `poll_interval_ms`, `concurrency`, `lock_timeout_secs`, `max_attempts`, `retry_backoff_secs`
  - [ ] fn `suprnova::WorkflowConfig::from_env` · framework/src/workflow/config.rs:61
  - [ ] fn `suprnova::WorkflowConfig::validate` · framework/src/workflow/config.rs:127
- [ ] const `suprnova::workflow::config::MIN_CONCURRENCY` · framework/src/workflow/config.rs:9
- [ ] const `suprnova::workflow::config::MIN_LOCK_TIMEOUT_SECS` · framework/src/workflow/config.rs:16
- [ ] const `suprnova::workflow::config::MIN_MAX_ATTEMPTS` · framework/src/workflow/config.rs:21

### `suprnova::workflow::context`

- [ ] struct `suprnova::WorkflowContext` · framework/src/workflow/context.rs:18 (also `suprnova::workflow::WorkflowContext`, `suprnova::workflow::context::WorkflowContext`)
  - [ ] fn `suprnova::WorkflowContext::enter` · framework/src/workflow/context.rs:57
  - [ ] fn `suprnova::WorkflowContext::current` · framework/src/workflow/context.rs:65
  - [ ] fn `suprnova::WorkflowContext::is_active` · framework/src/workflow/context.rs:70
  - [ ] fn `suprnova::WorkflowContext::run_step_with_input` · framework/src/workflow/context.rs:90
  - [ ] fn `suprnova::WorkflowContext::run_step` · framework/src/workflow/context.rs:228

### `suprnova::workflow::entities::workflow_steps`

- [ ] struct `suprnova::workflow::entities::workflow_steps::ActiveModel` · framework/src/workflow/entities.rs:66
  - Public fields: `id`, `workflow_id`, `step_index`, `step_name`, `status`, `input`, `output`, `error`, `attempts`, `created_at`, `updated_at`, `started_at`, `completed_at`
- [ ] struct `suprnova::workflow::entities::workflow_steps::ColumnIter` · framework/src/workflow/entities.rs:66
- [ ] struct `suprnova::workflow::entities::workflow_steps::Entity` · framework/src/workflow/entities.rs:66
- [ ] struct `suprnova::workflow::entities::workflow_steps::Model` · framework/src/workflow/entities.rs:68
  - Public fields: `id`, `workflow_id`, `step_index`, `step_name`, `status`, `input`, `output`, `error`, `attempts`, `created_at`, `updated_at`, `started_at`, `completed_at`
  - [ ] fn `suprnova::workflow::entities::workflow_steps::Model::into_ex` · framework/src/workflow/entities.rs:66
- [ ] struct `suprnova::workflow::entities::workflow_steps::PrimaryKeyIter` · framework/src/workflow/entities.rs:66
- [ ] struct `suprnova::workflow::entities::workflow_steps::RelationIter` · framework/src/workflow/entities.rs:102
- [ ] enum `suprnova::workflow::entities::workflow_steps::Column` · framework/src/workflow/entities.rs:66
  - Variants: `Id`, `WorkflowId`, `StepIndex`, `StepName`, `Status`, `Input`, `Output`, `Error`, `Attempts`, `CreatedAt`, `UpdatedAt`, `StartedAt`, `CompletedAt`
- [ ] enum `suprnova::workflow::entities::workflow_steps::PrimaryKey` · framework/src/workflow/entities.rs:66
  - Variants: `Id`
- [ ] enum `suprnova::workflow::entities::workflow_steps::Relation` · framework/src/workflow/entities.rs:103

### `suprnova::workflow::entities::workflows`

- [ ] struct `suprnova::workflow::entities::workflows::ActiveModel` · framework/src/workflow/entities.rs:12
  - Public fields: `id`, `name`, `status`, `input`, `output`, `error`, `attempts`, `max_attempts`, `next_run_at`, `locked_until`, `worker_id`, `created_at`, `updated_at`, `started_at`, `completed_at`
- [ ] struct `suprnova::workflow::entities::workflows::ColumnIter` · framework/src/workflow/entities.rs:12
- [ ] struct `suprnova::workflow::entities::workflows::Entity` · framework/src/workflow/entities.rs:12
- [ ] struct `suprnova::workflow::entities::workflows::Model` · framework/src/workflow/entities.rs:14
  - Public fields: `id`, `name`, `status`, `input`, `output`, `error`, `attempts`, `max_attempts`, `next_run_at`, `locked_until`, `worker_id`, `created_at`, `updated_at`, `started_at`, `completed_at`
  - [ ] fn `suprnova::workflow::entities::workflows::Model::into_ex` · framework/src/workflow/entities.rs:12
- [ ] struct `suprnova::workflow::entities::workflows::PrimaryKeyIter` · framework/src/workflow/entities.rs:12
- [ ] struct `suprnova::workflow::entities::workflows::RelationIter` · framework/src/workflow/entities.rs:52
- [ ] enum `suprnova::workflow::entities::workflows::Column` · framework/src/workflow/entities.rs:12
  - Variants: `Id`, `Name`, `Status`, `Input`, `Output`, `Error`, `Attempts`, `MaxAttempts`, `NextRunAt`, `LockedUntil`, `WorkerId`, `CreatedAt`, `UpdatedAt`, `StartedAt`, `CompletedAt`
- [ ] enum `suprnova::workflow::entities::workflows::PrimaryKey` · framework/src/workflow/entities.rs:12
  - Variants: `Id`
- [ ] enum `suprnova::workflow::entities::workflows::Relation` · framework/src/workflow/entities.rs:53

### `suprnova::workflow::migrations::m_create_workflow_steps_table`

- [ ] struct `suprnova::workflow::migrations::CreateWorkflowStepsTable` · framework/src/workflow/migrations/m_create_workflow_steps_table.rs:11 (also `suprnova::workflow::migrations::m_create_workflow_steps_table::Migration`)

### `suprnova::workflow::migrations::m_create_workflows_table`

- [ ] struct `suprnova::workflow::migrations::CreateWorkflowsTable` · framework/src/workflow/migrations/m_create_workflows_table.rs:11 (also `suprnova::workflow::migrations::m_create_workflows_table::Migration`)

### `suprnova::workflow::migrations::m_normalize_mysql_datetime_columns`

- [ ] struct `suprnova::workflow::migrations::NormalizeWorkflowDateTimesForMysql` · framework/src/workflow/migrations/m_normalize_mysql_datetime_columns.rs:33 (also `suprnova::workflow::migrations::m_normalize_mysql_datetime_columns::Migration`)

### `suprnova::workflow::store`

- [ ] fn `suprnova::workflow::store::claim_next_workflow` · framework/src/workflow/store.rs:132
- [ ] fn `suprnova::workflow::store::get_workflow_output` · framework/src/workflow/store.rs:69
- [ ] fn `suprnova::workflow::store::get_workflow_record` · framework/src/workflow/store.rs:80
- [ ] fn `suprnova::workflow::store::get_workflow_status` · framework/src/workflow/store.rs:56
- [ ] fn `suprnova::workflow::store::insert_step_running` · framework/src/workflow/store.rs:646
- [ ] fn `suprnova::workflow::store::insert_workflow` · framework/src/workflow/store.rs:21
- [ ] fn `suprnova::workflow::store::load_step` · framework/src/workflow/store.rs:613
- [ ] fn `suprnova::workflow::store::load_step_by_index` · framework/src/workflow/store.rs:629
- [ ] fn `suprnova::workflow::store::mark_failed` · framework/src/workflow/store.rs:564
- [ ] fn `suprnova::workflow::store::mark_running` · framework/src/workflow/store.rs:90
- [ ] fn `suprnova::workflow::store::mark_step_failed` · framework/src/workflow/store.rs:819
- [ ] fn `suprnova::workflow::store::mark_step_succeeded` · framework/src/workflow/store.rs:761
- [ ] fn `suprnova::workflow::store::mark_succeeded` · framework/src/workflow/store.rs:453
- [ ] fn `suprnova::workflow::store::refresh_lock` · framework/src/workflow/store.rs:286
- [ ] fn `suprnova::workflow::store::requeue` · framework/src/workflow/store.rs:510
- [ ] fn `suprnova::workflow::store::update_step_running` · framework/src/workflow/store.rs:713

### `suprnova::workflow::types`

- [ ] struct `suprnova::workflow::types::ClaimedWorkflow` · framework/src/workflow/types.rs:194
  - Public fields: `id`, `name`, `input`, `attempts`, `max_attempts`, `worker_id`
- [ ] struct `suprnova::WorkflowHandle` · framework/src/workflow/types.rs:83 (also `suprnova::workflow::WorkflowHandle`, `suprnova::workflow::types::WorkflowHandle`)
  - [ ] fn `suprnova::WorkflowHandle::id` · framework/src/workflow/types.rs:93
  - [ ] fn `suprnova::WorkflowHandle::status` · framework/src/workflow/types.rs:98
  - [ ] fn `suprnova::WorkflowHandle::wait` · framework/src/workflow/types.rs:105
  - [ ] fn `suprnova::WorkflowHandle::wait_with_timeout` · framework/src/workflow/types.rs:118
  - [ ] fn `suprnova::WorkflowHandle::wait_with_options` · framework/src/workflow/types.rs:130
  - [ ] fn `suprnova::WorkflowHandle::output_raw` · framework/src/workflow/types.rs:174
  - [ ] fn `suprnova::WorkflowHandle::output` · framework/src/workflow/types.rs:179
- [ ] enum `suprnova::StepStatus` · framework/src/workflow/types.rs:49 (also `suprnova::workflow::StepStatus`, `suprnova::workflow::types::StepStatus`)
  - Variants: `Running`, `Succeeded`, `Failed`
  - [ ] fn `suprnova::StepStatus::as_str` · framework/src/workflow/types.rs:60
  - [ ] fn `suprnova::StepStatus::from_str` · framework/src/workflow/types.rs:71
- [ ] enum `suprnova::WorkflowStatus` · framework/src/workflow/types.rs:11 (also `suprnova::workflow::WorkflowStatus`, `suprnova::workflow::types::WorkflowStatus`)
  - Variants: `Pending`, `Running`, `Succeeded`, `Failed`
  - [ ] fn `suprnova::WorkflowStatus::as_str` · framework/src/workflow/types.rs:24
  - [ ] fn `suprnova::WorkflowStatus::from_str` · framework/src/workflow/types.rs:36

## ws

### `suprnova::ws`

- [ ] struct `suprnova::WsConfig` · framework/src/ws/mod.rs:88 (also `suprnova::ws::WsConfig`)
  - Public fields: `ping_interval`, `max_message_size`, `max_frame_size`, `max_missed_pings`, `origin_policy`, `accepted_protocols`
  - [ ] fn `suprnova::WsConfig::generous` · framework/src/ws/mod.rs:219
- [ ] enum `suprnova::ws::OriginPolicy` · framework/src/ws/mod.rs:67
  - Variants: `SameOrigin`, `AllowAny`, `AllowList`
- [ ] trait `suprnova::WebSocketHandler` · framework/src/ws/mod.rs:44 (also `suprnova::ws::WebSocketHandler`)
  - Implemented here by: `BroadcastingWsHandler`
  - [ ] fn `suprnova::WebSocketHandler::handle` · framework/src/ws/mod.rs:48 (required)
- [ ] type `suprnova::ws::BoxedWebSocketHandler` · framework/src/ws/mod.rs:268

### `suprnova::ws::heartbeat`

- [ ] fn `suprnova::ws::heartbeat::run` · framework/src/ws/heartbeat.rs:37

### `suprnova::ws::socket` (private module; items are public through re-exports)

- [ ] struct `suprnova::WsSocket` · framework/src/ws/socket.rs:34 (also `suprnova::ws::WsSocket`)
  - [ ] fn `suprnova::WsSocket::from_stream` · framework/src/ws/socket.rs:78
  - [ ] fn `suprnova::WsSocket::from_stream_with_heartbeat` · framework/src/ws/socket.rs:91
  - [ ] fn `suprnova::WsSocket::send_text` · framework/src/ws/socket.rs:203
  - [ ] fn `suprnova::WsSocket::send_binary` · framework/src/ws/socket.rs:211
  - [ ] fn `suprnova::WsSocket::recv_text` · framework/src/ws/socket.rs:241
  - [ ] fn `suprnova::WsSocket::recv` · framework/src/ws/socket.rs:260
  - [ ] fn `suprnova::WsSocket::close` · framework/src/ws/socket.rs:295

## Public but unnameable

Reachable from the public API (as a return type, field or supertrait) but not importable by any public path.

- [ ] trait `suprnova::live::registry::sealed::Sealed` · framework/src/live/registry.rs:13 (sealed trait: a supertrait that stops implementations outside the crate)
- [ ] trait `suprnova::view::sealed::Sealed` · framework/src/view/mod.rs:42 (sealed trait: a supertrait that stops implementations outside the crate)
