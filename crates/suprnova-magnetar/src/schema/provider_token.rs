//! Typed accessors for the token broker's persisted provider-token records
//! (`docs/specs/suprnova-magnetar/11-token-broker.md`'s "Token records" and
//! "Refresh under rotation" sections).
//!
//! One row per broker-owned record: either a linked account's third-party
//! access/refresh token pair, or a cached machine-to-machine
//! (client-credentials) token. Both shapes share one table because both go
//! through the identical pre-call lease/CAS protocol
//! ([`crate::broker::lease`]) -- the row's `id` is the broker's own
//! `record_id`, an opaque application-owned key the caller derives (a
//! linked-account id for the former, [`crate::broker::M2MCacheKey::record_id`]
//! for the latter).

use chrono::{DateTime, Utc};

use super::EntityBinding;

/// Fields required by the token broker's lease/CAS protocol and encrypted
/// token storage.
pub trait ProviderTokenFields: EntityBinding {
    /// Read the record identifier (the broker's `record_id`).
    fn read_id(model: &Self::Model) -> String;
    /// Return the generated identifier column.
    fn id_column() -> Self::Column;
    /// Set the record identifier when constructing a new row.
    fn write_id(model: &mut Self::ActiveModel, value: &str);

    /// Read the owning provider's registry name.
    fn read_provider(model: &Self::Model) -> String;
    /// Return the generated provider column.
    fn provider_column() -> Self::Column;
    /// Set the provider name when constructing a new row.
    fn write_provider(model: &mut Self::ActiveModel, value: &str);

    /// Read the encrypted access token
    /// ([`crate::crypto::CryptoPurpose::ProviderToken`]).
    fn read_access_ciphertext(model: &Self::Model) -> Vec<u8>;
    /// Return the generated access-ciphertext column.
    fn access_ciphertext_column() -> Self::Column;
    /// Store the encrypted access token.
    fn write_access_ciphertext(model: &mut Self::ActiveModel, value: &[u8]);

    /// Read the encrypted refresh token
    /// ([`crate::crypto::CryptoPurpose::RefreshToken`]), when this record
    /// carries one.
    fn read_refresh_ciphertext(model: &Self::Model) -> Option<Vec<u8>>;
    /// Return the generated refresh-ciphertext column.
    fn refresh_ciphertext_column() -> Self::Column;
    /// Store (or clear) the encrypted refresh token.
    fn write_refresh_ciphertext(model: &mut Self::ActiveModel, value: Option<&[u8]>);

    /// Read the encrypted raw provider grant payload (the exact
    /// token-endpoint response body, for byte-faithful round-tripping of
    /// provider-specific fields).
    fn read_raw_payload_ciphertext(model: &Self::Model) -> Vec<u8>;
    /// Return the generated raw-payload-ciphertext column.
    fn raw_payload_ciphertext_column() -> Self::Column;
    /// Store the encrypted raw provider grant payload.
    fn write_raw_payload_ciphertext(model: &mut Self::ActiveModel, value: &[u8]);

    /// Read the token type (ordinarily `Bearer`).
    fn read_token_type(model: &Self::Model) -> String;
    /// Return the generated token-type column.
    fn token_type_column() -> Self::Column;
    /// Store the token type.
    fn write_token_type(model: &mut Self::ActiveModel, value: &str);

    /// Read the space-joined granted scopes.
    fn read_scopes(model: &Self::Model) -> String;
    /// Return the generated scopes column.
    fn scopes_column() -> Self::Column;
    /// Store the space-joined granted scopes.
    fn write_scopes(model: &mut Self::ActiveModel, value: &str);

    /// Read the access-token expiry, when the provider stated one.
    fn read_access_expires_at(model: &Self::Model) -> Option<DateTime<Utc>>;
    /// Return the generated access-expiry column.
    fn access_expires_at_column() -> Self::Column;
    /// Store the access-token expiry.
    fn write_access_expires_at(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>);

    /// Read the refresh-token generation (Task 5's CAS rotation counter).
    fn read_generation(model: &Self::Model) -> i64;
    /// Return the generated generation column.
    fn generation_column() -> Self::Column;
    /// Store the generation.
    fn write_generation(model: &mut Self::ActiveModel, value: i64);

    /// Read the current lease claim identifier, when one is live.
    fn read_claim_id(model: &Self::Model) -> Option<String>;
    /// Return the generated claim-id column.
    fn claim_id_column() -> Self::Column;
    /// Store (or clear) the claim identifier.
    fn write_claim_id(model: &mut Self::ActiveModel, value: Option<&str>);

    /// Read the current lease's deadline, when one is live.
    fn read_claim_deadline(model: &Self::Model) -> Option<DateTime<Utc>>;
    /// Return the generated claim-deadline column.
    fn claim_deadline_column() -> Self::Column;
    /// Store (or clear) the claim deadline.
    fn write_claim_deadline(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>);

    /// Read the revocation timestamp, when this record's family has been
    /// revoked.
    fn read_revoked_at(model: &Self::Model) -> Option<DateTime<Utc>>;
    /// Return the generated revocation-timestamp column.
    fn revoked_at_column() -> Self::Column;
    /// Store the revocation timestamp.
    fn write_revoked_at(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>);

    /// Read whether a revoked record was revoked as detected reuse (`true`)
    /// or dossier-driven ordinary revocation (`false`); `None` when the
    /// record has never been revoked.
    fn read_revoked_reused(model: &Self::Model) -> Option<bool>;
    /// Return the generated revoked-reused column.
    fn revoked_reused_column() -> Self::Column;
    /// Store the revoked-reused flag.
    fn write_revoked_reused(model: &mut Self::ActiveModel, value: Option<bool>);

    /// Read the row's creation timestamp.
    fn read_created_at(model: &Self::Model) -> DateTime<Utc>;
    /// Store the row's creation timestamp when constructing a new row.
    fn write_created_at(model: &mut Self::ActiveModel, value: DateTime<Utc>);
}
