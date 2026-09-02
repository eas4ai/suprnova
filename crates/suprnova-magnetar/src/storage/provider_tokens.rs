//! Provider-token-record storage: the token broker's lease/CAS primitives.
//!
//! One row serves both a linked account's third-party access/refresh token
//! pair and a cached machine-to-machine (client-credentials) token; both
//! shapes go through the identical pre-call lease protocol described in
//! `docs/specs/suprnova-magnetar/11-token-broker.md`. The row's `id` is the
//! broker's own opaque `record_id` -- this module never interprets it.
//!
//! Every mutation follows the transactional
//! select-then-conditional-`update_many`-checking-`rows_affected` idiom used
//! by [`super::ceremonies`] and [`super::device`]: a claim, a commit, and a
//! revocation are each a single atomically-conditioned SQL statement, so two
//! broker instances racing the same row over the same database converge
//! correctly with no in-process coordination required.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Expr, ExprTrait, IntoColumnRef};
use sea_orm::{ColumnTrait, ConnectionTrait, DbBackend, EntityTrait, QueryFilter};
use sha2::{Digest, Sha256};

use super::{SeaOrmStorage, db_error, in_transaction};
use crate::schema::{BrokerSchema, EntityBinding, ProviderTokenFields};
use crate::{Error, Result};

const EXCHANGE_CLAIM_PREFIX: &str = "98510432671946852037";
const EXCHANGE_CLAIM_DIGEST_LEN: usize = 64;

/// Derive the canonical opaque owner stored for an exchange-started claim.
///
/// Custom [`ProviderTokenStore`] implementations that opt into exchange
/// fencing must persist this value as the row's `claim_id` while preserving
/// the ordinary claim deadline.
pub fn exchange_claim_id(claim_id: &str) -> String {
    let digest = Sha256::digest(claim_id.as_bytes());
    let mut encoded = String::with_capacity(EXCHANGE_CLAIM_PREFIX.len() + digest.len() * 2);
    encoded.push_str(EXCHANGE_CLAIM_PREFIX);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

/// Return whether a claim identifier is a canonical exchange-fence owner.
///
/// Store implementations must never reclaim an identifier for which this
/// returns `true`, even after its preserved deadline expires.
pub fn is_exchange_claim_id(claim_id: &str) -> bool {
    let Some(digest) = claim_id.strip_prefix(EXCHANGE_CLAIM_PREFIX) else {
        return false;
    };
    digest.len() == EXCHANGE_CLAIM_DIGEST_LEN
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn exact_text_eq<C>(backend: DbBackend, column: C, value: &str) -> Expr
where
    C: IntoColumnRef,
{
    match backend {
        DbBackend::MySql => Expr::cust_with_exprs(
            "BINARY ? = BINARY ?",
            [Expr::col(column), Expr::value(value.to_owned())],
        ),
        DbBackend::Sqlite => Expr::cust_with_exprs(
            "CAST(? AS BLOB) = CAST(? AS BLOB)",
            [Expr::col(column), Expr::value(value.to_owned())],
        ),
        DbBackend::Postgres => Expr::col(column).eq(value.to_owned()),
        _ => Expr::value(false),
    }
}

/// Input for provisioning a fresh, unclaimed record at generation zero.
#[derive(Clone, Debug)]
pub struct NewProviderToken {
    /// The broker's opaque record identifier.
    pub id: String,
    /// The owning provider's registry name.
    pub provider: String,
}

/// Input committed under an owned claim once a provider exchange succeeds.
#[derive(Clone, Debug)]
pub struct CommitProviderToken {
    /// Encrypted access token
    /// ([`crate::crypto::CryptoPurpose::ProviderToken`]).
    pub access_ciphertext: Vec<u8>,
    /// A newly issued encrypted refresh token
    /// ([`crate::crypto::CryptoPurpose::RefreshToken`]), when the provider
    /// rotated it on this exchange. `None` leaves the stored refresh token
    /// (or its absence) untouched.
    pub refresh_ciphertext: Option<Vec<u8>>,
    /// Encrypted raw provider grant payload (the exact response body).
    pub raw_payload_ciphertext: Vec<u8>,
    /// Token type (ordinarily `Bearer`).
    pub token_type: String,
    /// Space-joined granted scopes.
    pub scopes: String,
    /// Access-token expiry, when the provider stated one.
    pub access_expires_at: Option<DateTime<Utc>>,
    /// The generation to commit: unchanged from the presented generation
    /// for a non-rotating outcome, or incremented by the caller when the
    /// provider rotated the refresh token.
    pub new_generation: i64,
}

/// A generic view of one provider-token row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderTokenRow {
    /// The broker's opaque record identifier.
    pub id: String,
    /// The owning provider's registry name.
    pub provider: String,
    /// Encrypted access token.
    pub access_ciphertext: Vec<u8>,
    /// Encrypted refresh token, when this record carries one.
    pub refresh_ciphertext: Option<Vec<u8>>,
    /// Encrypted raw provider grant payload.
    pub raw_payload_ciphertext: Vec<u8>,
    /// Token type.
    pub token_type: String,
    /// Space-joined granted scopes.
    pub scopes: String,
    /// Access-token expiry, when the provider stated one.
    pub access_expires_at: Option<DateTime<Utc>>,
    /// The refresh-token generation (the CAS rotation counter).
    pub generation: i64,
    /// The current lease claim identifier, when one is live.
    pub claim_id: Option<String>,
    /// The current lease's deadline, when one is live.
    pub claim_deadline: Option<DateTime<Utc>>,
    /// Revocation timestamp, when this record's family has been revoked.
    pub revoked_at: Option<DateTime<Utc>>,
    /// Whether a revoked record was revoked as detected reuse (`true`) or
    /// dossier-driven ordinary revocation (`false`); meaningless when
    /// `revoked_at` is `None`.
    pub revoked_reused: Option<bool>,
    /// The row's creation timestamp.
    pub created_at: DateTime<Utc>,
}

impl ProviderTokenRow {
    /// Whether a lease claim is currently live (held and not past its
    /// deadline) as of `now`.
    pub fn has_live_claim(&self, now: DateTime<Utc>) -> bool {
        match (&self.claim_id, self.claim_deadline) {
            (Some(_), Some(deadline)) => deadline > now,
            _ => false,
        }
    }
}

/// Storage API for the token broker's lease/CAS protocol.
///
/// Every conditional method returns `Ok(true)` only when this call's
/// predicate uniquely matched and the write committed; `Ok(false)` means the
/// row's state had already moved on (lost race, expired presenter, already
/// revoked) -- never an error, per spec 11's "a failed claim is never an
/// attack signal."
#[async_trait]
pub trait ProviderTokenStore: Send + Sync {
    /// Idempotently provision a fresh record at generation zero. Returns
    /// the existing row when one is already present.
    async fn create_if_missing(&self, input: NewProviderToken) -> Result<ProviderTokenRow>;

    /// Read the current row without mutating it.
    async fn read(&self, record_id: &str) -> Result<Option<ProviderTokenRow>>;

    /// Conditionally claim `presented_generation` for `record_id`. Succeeds
    /// only when the stored generation matches, the record is not revoked,
    /// and no live claim currently owns it. Implementations must reject the
    /// reserved exchange-owner namespace and must never reclaim an owner for
    /// which [`is_exchange_claim_id`] returns `true`.
    async fn claim(
        &self,
        record_id: &str,
        presented_generation: i64,
        claim_id: &str,
        claim_deadline: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool>;

    /// Extend an owned claim's deadline without releasing it.
    async fn heartbeat(
        &self,
        record_id: &str,
        claim_id: &str,
        new_deadline: DateTime<Utc>,
    ) -> Result<bool>;

    /// Atomically fence an owned ordinary claim immediately before its
    /// linked-account provider exchange begins. The existing deadline is
    /// preserved so followers can distinguish a live exchange from an
    /// abandoned one. A successful implementation must replace the exact
    /// ordinary `claim_id` with [`exchange_claim_id(claim_id)`]. The default
    /// fails closed for custom stores.
    async fn mark_exchange_started(
        &self,
        _record_id: &str,
        _claim_id: &str,
        _presented_generation: i64,
        _now: DateTime<Utc>,
    ) -> Result<bool> {
        Ok(false)
    }

    /// Commit a successful provider exchange under an owned claim and
    /// release it. Succeeds only when `claim_id` and `presented_generation`
    /// still match the stored row (a stale, reclaimed leader's late result
    /// is discarded, never overwriting a newer lease).
    async fn commit(
        &self,
        record_id: &str,
        claim_id: &str,
        presented_generation: i64,
        input: CommitProviderToken,
    ) -> Result<bool>;

    /// Commit a linked-account exchange only under its exact fenced claim
    /// and presented generation. The default fails closed for custom
    /// stores; legacy [`Self::commit`] semantics remain available to direct
    /// callers and the machine-to-machine cache.
    async fn commit_exchange(
        &self,
        _record_id: &str,
        _exchange_claim_id: &str,
        _presented_generation: i64,
        _input: CommitProviderToken,
    ) -> Result<bool> {
        Ok(false)
    }

    /// Release an owned claim as a terminal, dossier-driven revocation
    /// (`invalid_grant`), marking the record revoked. Succeeds only when
    /// `claim_id` still matches.
    async fn mark_revoked_by_claim(
        &self,
        record_id: &str,
        claim_id: &str,
        reused: bool,
    ) -> Result<bool>;

    /// Revoke a linked-account family only under its exact fenced claim
    /// and presented generation. The default fails closed for custom
    /// stores.
    async fn mark_revoked_by_exchange(
        &self,
        _record_id: &str,
        _exchange_claim_id: &str,
        _presented_generation: i64,
        _reused: bool,
    ) -> Result<bool> {
        Ok(false)
    }

    /// Revoke a stale-presenter's family as detected reuse, independent of
    /// any claim, conditioned on the record not already being revoked and
    /// not currently mid-refresh. Returns whether this call performed the
    /// revocation, so callers fire a reuse hook exactly once.
    async fn revoke_family_if_unrevoked(&self, record_id: &str, now: DateTime<Utc>)
    -> Result<bool>;

    /// Delete a record outright (account-unlink / cache-eviction cascade).
    /// Returns whether a row was deleted.
    async fn delete(&self, record_id: &str) -> Result<bool>;
}

fn record<S>(model: &<S::ProviderToken as EntityBinding>::Model) -> ProviderTokenRow
where
    S: BrokerSchema,
    S::ProviderToken: ProviderTokenFields,
{
    ProviderTokenRow {
        id: S::ProviderToken::read_id(model),
        provider: S::ProviderToken::read_provider(model),
        access_ciphertext: S::ProviderToken::read_access_ciphertext(model),
        refresh_ciphertext: S::ProviderToken::read_refresh_ciphertext(model),
        raw_payload_ciphertext: S::ProviderToken::read_raw_payload_ciphertext(model),
        token_type: S::ProviderToken::read_token_type(model),
        scopes: S::ProviderToken::read_scopes(model),
        access_expires_at: S::ProviderToken::read_access_expires_at(model),
        generation: S::ProviderToken::read_generation(model),
        claim_id: S::ProviderToken::read_claim_id(model),
        claim_deadline: S::ProviderToken::read_claim_deadline(model),
        revoked_at: S::ProviderToken::read_revoked_at(model),
        revoked_reused: S::ProviderToken::read_revoked_reused(model),
        created_at: S::ProviderToken::read_created_at(model),
    }
}

fn empty(field: &str) -> Error {
    Error::InvalidInput {
        field: field.to_owned(),
        message: format!("{field} must be non-empty"),
    }
}

#[async_trait]
impl<S> ProviderTokenStore for SeaOrmStorage<S>
where
    S: BrokerSchema,
    S::ProviderToken: ProviderTokenFields,
    <S::ProviderToken as EntityBinding>::Entity: EntityTrait<
            Model = <S::ProviderToken as EntityBinding>::Model,
            ActiveModel = <S::ProviderToken as EntityBinding>::ActiveModel,
        >,
    <S::ProviderToken as EntityBinding>::Column: ColumnTrait,
{
    async fn create_if_missing(&self, input: NewProviderToken) -> Result<ProviderTokenRow> {
        if input.id.is_empty() {
            return Err(empty("id"));
        }
        if input.provider.is_empty() {
            return Err(empty("provider"));
        }
        let now = Utc::now();
        let mut model = <S::ProviderToken as EntityBinding>::ActiveModel::default();
        S::ProviderToken::write_id(&mut model, &input.id);
        S::ProviderToken::write_provider(&mut model, &input.provider);
        S::ProviderToken::write_access_ciphertext(&mut model, &[]);
        S::ProviderToken::write_refresh_ciphertext(&mut model, None);
        S::ProviderToken::write_raw_payload_ciphertext(&mut model, &[]);
        S::ProviderToken::write_token_type(&mut model, "");
        S::ProviderToken::write_scopes(&mut model, "");
        S::ProviderToken::write_access_expires_at(&mut model, None);
        S::ProviderToken::write_generation(&mut model, 0);
        S::ProviderToken::write_claim_id(&mut model, None);
        S::ProviderToken::write_claim_deadline(&mut model, None);
        S::ProviderToken::write_revoked_at(&mut model, None);
        S::ProviderToken::write_revoked_reused(&mut model, None);
        S::ProviderToken::write_created_at(&mut model, now);
        let insert = <S::ProviderToken as EntityBinding>::Entity::insert(model)
            .exec(self.database())
            .await;
        if insert.is_err() {
            if let Some(existing) = self.read(&input.id).await? {
                return Ok(existing);
            }
            insert.map_err(db_error)?;
        }
        self.read(&input.id)
            .await?
            .ok_or_else(|| db_error(sea_orm::DbErr::Custom("row vanished after insert".into())))
    }

    async fn read(&self, record_id: &str) -> Result<Option<ProviderTokenRow>> {
        if record_id.is_empty() {
            return Err(empty("record_id"));
        }
        let rows = <S::ProviderToken as EntityBinding>::Entity::find()
            .filter(S::ProviderToken::id_column().eq(record_id.to_owned()))
            .all(self.database())
            .await
            .map_err(db_error)?;
        Ok(rows.first().map(record::<S>))
    }

    async fn claim(
        &self,
        record_id: &str,
        presented_generation: i64,
        claim_id: &str,
        claim_deadline: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        if record_id.is_empty() || claim_id.is_empty() {
            return Err(empty("record_id/claim_id"));
        }
        if claim_id.starts_with(EXCHANGE_CLAIM_PREFIX) {
            return Err(Error::InvalidInput {
                field: "claim_id".to_owned(),
                message: "claim_id uses a reserved prefix".to_owned(),
            });
        }
        let result = <S::ProviderToken as EntityBinding>::Entity::update_many()
            .col_expr(
                S::ProviderToken::claim_id_column(),
                Expr::value(claim_id.to_owned()),
            )
            .col_expr(
                S::ProviderToken::claim_deadline_column(),
                Expr::value(claim_deadline),
            )
            .filter(S::ProviderToken::id_column().eq(record_id.to_owned()))
            .filter(S::ProviderToken::generation_column().eq(presented_generation))
            .filter(S::ProviderToken::revoked_at_column().is_null())
            .filter(
                sea_orm::Condition::any()
                    .add(S::ProviderToken::claim_id_column().is_null())
                    .add(
                        sea_orm::Condition::all()
                            .add(S::ProviderToken::claim_deadline_column().lte(now))
                            .add(
                                S::ProviderToken::claim_id_column()
                                    .not_like(format!("{EXCHANGE_CLAIM_PREFIX}%")),
                            ),
                    ),
            )
            .exec(self.database())
            .await
            .map_err(db_error)?;
        Ok(result.rows_affected == 1)
    }

    async fn mark_exchange_started(
        &self,
        record_id: &str,
        claim_id: &str,
        presented_generation: i64,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        if record_id.is_empty() || claim_id.is_empty() {
            return Err(empty("record_id/claim_id"));
        }
        if claim_id.starts_with(EXCHANGE_CLAIM_PREFIX) {
            return Ok(false);
        }
        let result = <S::ProviderToken as EntityBinding>::Entity::update_many()
            .col_expr(
                S::ProviderToken::claim_id_column(),
                Expr::value(exchange_claim_id(claim_id)),
            )
            .filter(S::ProviderToken::id_column().eq(record_id.to_owned()))
            .filter(exact_text_eq(
                self.database().get_database_backend(),
                S::ProviderToken::claim_id_column(),
                claim_id,
            ))
            .filter(S::ProviderToken::claim_deadline_column().gt(now))
            .filter(S::ProviderToken::generation_column().eq(presented_generation))
            .filter(S::ProviderToken::revoked_at_column().is_null())
            .exec(self.database())
            .await
            .map_err(db_error)?;
        Ok(result.rows_affected == 1)
    }

    async fn heartbeat(
        &self,
        record_id: &str,
        claim_id: &str,
        new_deadline: DateTime<Utc>,
    ) -> Result<bool> {
        if record_id.is_empty() || claim_id.is_empty() {
            return Err(empty("record_id/claim_id"));
        }
        if is_exchange_claim_id(claim_id) {
            return Ok(false);
        }
        let result = <S::ProviderToken as EntityBinding>::Entity::update_many()
            .col_expr(
                S::ProviderToken::claim_deadline_column(),
                Expr::value(new_deadline),
            )
            .filter(S::ProviderToken::id_column().eq(record_id.to_owned()))
            .filter(exact_text_eq(
                self.database().get_database_backend(),
                S::ProviderToken::claim_id_column(),
                claim_id,
            ))
            .filter(S::ProviderToken::revoked_at_column().is_null())
            .exec(self.database())
            .await
            .map_err(db_error)?;
        Ok(result.rows_affected == 1)
    }

    async fn commit(
        &self,
        record_id: &str,
        claim_id: &str,
        presented_generation: i64,
        input: CommitProviderToken,
    ) -> Result<bool> {
        if record_id.is_empty() || claim_id.is_empty() {
            return Err(empty("record_id/claim_id"));
        }
        let record_id = record_id.to_owned();
        let claim_id = claim_id.to_owned();
        in_transaction(self.database(), move |tx| {
            Box::pin(async move {
                let mut query = <S::ProviderToken as EntityBinding>::Entity::update_many()
                    .col_expr(
                        S::ProviderToken::access_ciphertext_column(),
                        Expr::value(input.access_ciphertext.clone()),
                    )
                    .col_expr(
                        S::ProviderToken::raw_payload_ciphertext_column(),
                        Expr::value(input.raw_payload_ciphertext.clone()),
                    )
                    .col_expr(
                        S::ProviderToken::token_type_column(),
                        Expr::value(input.token_type.clone()),
                    )
                    .col_expr(
                        S::ProviderToken::scopes_column(),
                        Expr::value(input.scopes.clone()),
                    )
                    .col_expr(
                        S::ProviderToken::access_expires_at_column(),
                        Expr::value(input.access_expires_at),
                    )
                    .col_expr(
                        S::ProviderToken::generation_column(),
                        Expr::value(input.new_generation),
                    )
                    .col_expr(
                        S::ProviderToken::claim_id_column(),
                        Expr::value(Option::<String>::None),
                    )
                    .col_expr(
                        S::ProviderToken::claim_deadline_column(),
                        Expr::value(Option::<DateTime<Utc>>::None),
                    );
                if let Some(refresh_ciphertext) = input.refresh_ciphertext.clone() {
                    query = query.col_expr(
                        S::ProviderToken::refresh_ciphertext_column(),
                        Expr::value(refresh_ciphertext),
                    );
                }
                let result = query
                    .filter(S::ProviderToken::id_column().eq(record_id.clone()))
                    .filter(exact_text_eq(
                        tx.connection().get_database_backend(),
                        S::ProviderToken::claim_id_column(),
                        &claim_id,
                    ))
                    .filter(S::ProviderToken::generation_column().eq(presented_generation))
                    .filter(S::ProviderToken::revoked_at_column().is_null())
                    .exec(tx.connection())
                    .await
                    .map_err(db_error)?;
                Ok(result.rows_affected == 1)
            })
        })
        .await
    }

    async fn commit_exchange(
        &self,
        record_id: &str,
        exchange_claim_id: &str,
        presented_generation: i64,
        input: CommitProviderToken,
    ) -> Result<bool> {
        if !is_exchange_claim_id(exchange_claim_id) {
            return Ok(false);
        }
        self.commit(record_id, exchange_claim_id, presented_generation, input)
            .await
    }

    async fn mark_revoked_by_claim(
        &self,
        record_id: &str,
        claim_id: &str,
        reused: bool,
    ) -> Result<bool> {
        if record_id.is_empty() || claim_id.is_empty() {
            return Err(empty("record_id/claim_id"));
        }
        if is_exchange_claim_id(claim_id) {
            return Ok(false);
        }
        let now = Utc::now();
        let result = <S::ProviderToken as EntityBinding>::Entity::update_many()
            .col_expr(S::ProviderToken::revoked_at_column(), Expr::value(now))
            .col_expr(
                S::ProviderToken::revoked_reused_column(),
                Expr::value(reused),
            )
            .col_expr(
                S::ProviderToken::claim_id_column(),
                Expr::value(Option::<String>::None),
            )
            .col_expr(
                S::ProviderToken::claim_deadline_column(),
                Expr::value(Option::<DateTime<Utc>>::None),
            )
            .filter(S::ProviderToken::id_column().eq(record_id.to_owned()))
            .filter(exact_text_eq(
                self.database().get_database_backend(),
                S::ProviderToken::claim_id_column(),
                claim_id,
            ))
            .filter(S::ProviderToken::revoked_at_column().is_null())
            .exec(self.database())
            .await
            .map_err(db_error)?;
        Ok(result.rows_affected == 1)
    }

    async fn mark_revoked_by_exchange(
        &self,
        record_id: &str,
        exchange_claim_id: &str,
        presented_generation: i64,
        reused: bool,
    ) -> Result<bool> {
        if record_id.is_empty() || exchange_claim_id.is_empty() {
            return Err(empty("record_id/exchange_claim_id"));
        }
        if !is_exchange_claim_id(exchange_claim_id) {
            return Ok(false);
        }
        let now = Utc::now();
        let result = <S::ProviderToken as EntityBinding>::Entity::update_many()
            .col_expr(S::ProviderToken::revoked_at_column(), Expr::value(now))
            .col_expr(
                S::ProviderToken::revoked_reused_column(),
                Expr::value(reused),
            )
            .col_expr(
                S::ProviderToken::claim_id_column(),
                Expr::value(Option::<String>::None),
            )
            .col_expr(
                S::ProviderToken::claim_deadline_column(),
                Expr::value(Option::<DateTime<Utc>>::None),
            )
            .filter(S::ProviderToken::id_column().eq(record_id.to_owned()))
            .filter(exact_text_eq(
                self.database().get_database_backend(),
                S::ProviderToken::claim_id_column(),
                exchange_claim_id,
            ))
            .filter(S::ProviderToken::generation_column().eq(presented_generation))
            .filter(S::ProviderToken::revoked_at_column().is_null())
            .exec(self.database())
            .await
            .map_err(db_error)?;
        Ok(result.rows_affected == 1)
    }

    async fn revoke_family_if_unrevoked(
        &self,
        record_id: &str,
        now: DateTime<Utc>,
    ) -> Result<bool> {
        if record_id.is_empty() {
            return Err(empty("record_id"));
        }
        let result = <S::ProviderToken as EntityBinding>::Entity::update_many()
            .col_expr(S::ProviderToken::revoked_at_column(), Expr::value(now))
            .col_expr(S::ProviderToken::revoked_reused_column(), Expr::value(true))
            .col_expr(
                S::ProviderToken::claim_id_column(),
                Expr::value(Option::<String>::None),
            )
            .col_expr(
                S::ProviderToken::claim_deadline_column(),
                Expr::value(Option::<DateTime<Utc>>::None),
            )
            .filter(S::ProviderToken::id_column().eq(record_id.to_owned()))
            .filter(S::ProviderToken::revoked_at_column().is_null())
            .filter(
                sea_orm::Condition::any()
                    .add(S::ProviderToken::claim_id_column().is_null())
                    .add(
                        sea_orm::Condition::all()
                            .add(S::ProviderToken::claim_deadline_column().lte(now))
                            .add(
                                S::ProviderToken::claim_id_column()
                                    .not_like(format!("{EXCHANGE_CLAIM_PREFIX}%")),
                            ),
                    ),
            )
            .exec(self.database())
            .await
            .map_err(db_error)?;
        Ok(result.rows_affected == 1)
    }

    async fn delete(&self, record_id: &str) -> Result<bool> {
        if record_id.is_empty() {
            return Err(empty("record_id"));
        }
        let result = <S::ProviderToken as EntityBinding>::Entity::delete_many()
            .filter(S::ProviderToken::id_column().eq(record_id.to_owned()))
            .exec(self.database())
            .await
            .map_err(db_error)?;
        Ok(result.rows_affected > 0)
    }
}

#[cfg(test)]
mod exact_text_tests {
    use sea_orm::sea_query::{
        Alias, MysqlQueryBuilder, PostgresQueryBuilder, Query, SqliteQueryBuilder,
    };

    use super::*;

    #[test]
    fn render_exact_text_predicates() {
        let sqlite = Query::select()
            .expr(exact_text_eq(
                DbBackend::Sqlite,
                Alias::new("claim_id"),
                "AbC",
            ))
            .to_string(SqliteQueryBuilder);
        let mysql = Query::select()
            .expr(exact_text_eq(
                DbBackend::MySql,
                Alias::new("claim_id"),
                "AbC",
            ))
            .to_string(MysqlQueryBuilder);
        let postgres = Query::select()
            .expr(exact_text_eq(
                DbBackend::Postgres,
                Alias::new("claim_id"),
                "AbC",
            ))
            .to_string(PostgresQueryBuilder);

        assert_eq!(
            sqlite,
            r#"SELECT CAST("claim_id" AS BLOB) = CAST('AbC' AS BLOB)"#
        );
        assert_eq!(mysql, "SELECT BINARY `claim_id` = BINARY 'AbC'");
        assert_eq!(postgres, r#"SELECT "claim_id" = 'AbC'"#);
    }

    #[test]
    fn exchange_owner_is_canonical_and_opaque() {
        let owner = exchange_claim_id("Case-Sensitive-Ordinary-Owner");

        assert!(is_exchange_claim_id(&owner));
        assert_eq!(
            owner.len(),
            EXCHANGE_CLAIM_PREFIX.len() + EXCHANGE_CLAIM_DIGEST_LEN
        );
        assert!(!owner.contains("Case-Sensitive-Ordinary-Owner"));
        assert!(!is_exchange_claim_id(&owner.to_uppercase()));
        assert!(!is_exchange_claim_id(EXCHANGE_CLAIM_PREFIX));
    }
}
