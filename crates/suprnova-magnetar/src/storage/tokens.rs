//! Single-use token issuance, transactional consumption, and password reset.

use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, ExprTrait, IntoActiveModel, QueryFilter, QuerySelect};
use secrecy::SecretString;
use sha2::{Digest, Sha256};

use super::{
    AuthTransaction, SeaOrmStorage, db_error, expose_secret, in_retried_transaction,
    in_transaction, random_id, random_token,
};
use crate::schema::{
    AuthSchema, EntityBinding, SessionEpoch, SessionFields, TokenFields, UserFields,
    UserOptionalFields,
};
use crate::{Error, Result};

/// Purpose namespace used by the password-reset composite.
pub const PASSWORD_RESET_PURPOSE: &str = "password-reset";

/// Input used to issue a one-time token.
#[derive(Clone, Debug)]
pub struct IssueToken {
    /// Owning user identifier.
    pub user_id: String,
    /// Purpose namespace; purpose is part of the CAS predicate.
    pub purpose: String,
    /// Token lifetime.
    pub ttl: std::time::Duration,
}

/// Plaintext token presented by a caller.
#[derive(Debug)]
pub struct PresentedToken(pub SecretString);

impl PresentedToken {
    /// Wrap a plaintext token at the storage boundary.
    pub fn new(value: impl Into<String>) -> Self {
        Self(SecretString::new(value.into().into()))
    }
}

/// Token returned exactly once by issuance.
#[derive(Debug)]
pub struct IssuedToken {
    /// Plaintext bearer token. Callers must not persist this value.
    pub plaintext: SecretString,
    /// Application-owned token row identifier.
    pub token_id: String,
    /// Expiry timestamp.
    pub expires_at: DateTime<Utc>,
}

/// Token metadata returned after a successful consume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsumedToken {
    /// Application-owned token row identifier.
    pub token_id: String,
    /// Owning user identifier, if the binding stores one.
    pub user_id: Option<String>,
    /// Purpose namespace.
    pub purpose: String,
    /// Expiry timestamp.
    pub expires_at: DateTime<Utc>,
}

/// What a token of one purpose is, read without consuming it: whose it is,
/// the address it was issued for, and whether it was used or has expired.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenState {
    /// Owning user identifier, if the binding stores one.
    pub user_id: Option<String>,
    /// The [`address_digest`] of the address the token was issued for
    /// ([`TokenStore::issue_bound`]); `None` for a token issued with none,
    /// or by a binding that keeps none.
    pub address_digest: Option<String>,
    /// Whether the token was consumed, or invalidated by a sibling's consume.
    pub used: bool,
    /// Whether the token's lifetime has ended.
    pub expired: bool,
}

/// The digest a token keeps of the address it was issued for: the SHA-256,
/// in lowercase hex, of the address in lowercase.
///
/// A digest rather than the address, because the address is personal data
/// and a token only has to tell whether an address is the one it was issued
/// for. In lowercase, because the case of an address is not part of the
/// mailbox it names.
pub fn address_digest(address: &str) -> String {
    digest(&address.to_lowercase())
}

/// Password-reset composite input.
///
/// The reset user is derived from the consumed token row, mirroring the
/// consume-then-rotate ordering of the deployed flow. A caller that already
/// knows which user it is resetting may pin that expectation with
/// [`PasswordResetInput::expecting_user`]; a mismatch rolls the whole
/// composite back.
#[derive(Debug)]
pub struct PasswordResetInput {
    /// Reset token presented by the caller.
    pub token: PresentedToken,
    /// Already-hashed replacement password.
    pub new_password_hash: String,
    /// Optional caller-asserted owner; a mismatch aborts the reset.
    pub expected_user_id: Option<String>,
}

impl PasswordResetInput {
    /// Construct a reset request with an already-hashed credential.
    pub fn new(token: PresentedToken, new_password_hash: impl Into<String>) -> Self {
        Self {
            token,
            new_password_hash: new_password_hash.into(),
            expected_user_id: None,
        }
    }

    /// Require the consumed token to belong to a specific user.
    #[must_use]
    pub fn expecting_user(mut self, user_id: impl Into<String>) -> Self {
        self.expected_user_id = Some(user_id.into());
        self
    }
}

/// Result of an atomic password-reset composite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasswordResetCommit {
    /// Reset user identifier.
    pub user_id: String,
    /// Epoch after the successful reset.
    pub auth_epoch: u64,
    /// Number of opaque sessions revoked.
    pub revoked_sessions: u64,
}

/// Storage API for one-time tokens.
#[async_trait]
pub trait TokenStore: Send + Sync {
    /// Insert a digest and return the plaintext only to the caller.
    async fn issue(&self, input: IssueToken) -> Result<IssuedToken>;
    /// Report whether a live, unused token exists without consuming it.
    async fn check(&self, token: PresentedToken, purpose: &str) -> Result<bool>;
    /// Consume and invalidate all unused siblings in one owned transaction.
    async fn consume(&self, token: PresentedToken, purpose: &str) -> Result<ConsumedToken>;
    /// Consume inside a caller-owned transaction without beginning or committing it.
    async fn consume_in(
        &self,
        tx: &mut AuthTransaction<'_>,
        token: PresentedToken,
        purpose: &str,
    ) -> Result<ConsumedToken>;
    /// Insert a digest, as [`Self::issue`] does, and keep with it the
    /// [`address_digest`] of `address`, the address the token goes to. A
    /// flow that must prove the token's owner still holds that address
    /// reads it back through [`Self::inspect`].
    ///
    /// The default refuses, as does a store whose binding keeps no address
    /// ([`TokenFields::write_address_digest`]): a token that must prove an
    /// address is never stored without it.
    async fn issue_bound(&self, input: IssueToken, address: &str) -> Result<IssuedToken> {
        let _ = (input, address);
        Err(address_binding_unavailable())
    }
    /// The state of the token of `purpose` that `token` presents, read
    /// without consuming it; `None` when no token of that purpose has this
    /// value. A flow reads it to tell an expired token from an unknown one,
    /// and to refuse another user's token without spending it.
    ///
    /// The default reports the state unavailable, which keeps existing
    /// stores source-compatible; a flow that needs the state then fails
    /// closed.
    async fn inspect(&self, token: PresentedToken, purpose: &str) -> Result<Option<TokenState>> {
        let _ = (token, purpose);
        Err(Error::DependencyUnavailable {
            dependency: "token store".to_owned(),
            message: "this token store does not report a token's state".to_owned(),
        })
    }
}

/// Storage API for the all-mutations-in-one-transaction password reset flow.
#[async_trait]
pub trait PasswordResetStore: Send + Sync {
    /// Consume the reset token, replace the credential, advance epoch, and
    /// revoke opaque sessions atomically.
    async fn apply_password_reset(&self, input: PasswordResetInput) -> Result<PasswordResetCommit>;
}

fn digest(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    crate::storage::hex_lower(&hasher.finalize())
}

fn address_binding_unavailable() -> Error {
    Error::DependencyUnavailable {
        dependency: "token store".to_owned(),
        message: "this token store does not keep the address a token was issued for".to_owned(),
    }
}

fn invalid_id(field: &str) -> Error {
    Error::InvalidInput {
        field: field.to_owned(),
        message: "empty identifier".to_owned(),
    }
}

impl<S> SeaOrmStorage<S>
where
    S: AuthSchema,
    S::Token: TokenFields,
    <S::Token as EntityBinding>::Entity: EntityTrait<
            Model = <S::Token as EntityBinding>::Model,
            ActiveModel = <S::Token as EntityBinding>::ActiveModel,
        >,
    <S::Token as EntityBinding>::Column: ColumnTrait,
{
    /// Insert one token row, bound to `address` when one is given.
    async fn insert_token(&self, input: IssueToken, address: Option<&str>) -> Result<IssuedToken> {
        if input.user_id.is_empty() {
            return Err(invalid_id("user_id"));
        }
        if input.purpose.is_empty() {
            return Err(invalid_id("purpose"));
        }
        if input.ttl.is_zero() {
            return Err(Error::InvalidInput {
                field: "ttl".to_owned(),
                message: "token lifetime must be positive".to_owned(),
            });
        }
        let plaintext = random_token();
        let token_id = random_id();
        let expires_at = Utc::now()
            + ChronoDuration::from_std(input.ttl).map_err(|_| Error::InvalidInput {
                field: "ttl".to_owned(),
                message: "token lifetime is out of range".to_owned(),
            })?;
        let mut model = <S::Token as EntityBinding>::ActiveModel::default();
        S::Token::write_token_id(&mut model, &token_id);
        S::Token::write_user_id(&mut model, Some(&input.user_id));
        S::Token::write_purpose(&mut model, &input.purpose);
        S::Token::write_digest(&mut model, &digest(&plaintext));
        S::Token::write_expires_at(&mut model, expires_at);
        if let Some(address) = address
            && !S::Token::write_address_digest(&mut model, &address_digest(address))
        {
            return Err(address_binding_unavailable());
        }
        <S::Token as EntityBinding>::Entity::insert(model)
            .exec(self.database())
            .await
            .map_err(db_error)?;
        Ok(IssuedToken {
            plaintext: SecretString::new(plaintext.into()),
            token_id,
            expires_at,
        })
    }
}

#[async_trait]
impl<S> TokenStore for SeaOrmStorage<S>
where
    S: AuthSchema,
    S::Token: TokenFields,
    <S::Token as EntityBinding>::Entity: EntityTrait<
            Model = <S::Token as EntityBinding>::Model,
            ActiveModel = <S::Token as EntityBinding>::ActiveModel,
        >,
    <S::Token as EntityBinding>::Column: ColumnTrait,
{
    async fn issue(&self, input: IssueToken) -> Result<IssuedToken> {
        self.insert_token(input, None).await
    }

    async fn issue_bound(&self, input: IssueToken, address: &str) -> Result<IssuedToken> {
        if address.is_empty() {
            return Err(invalid_id("address"));
        }
        self.insert_token(input, Some(address)).await
    }

    async fn check(&self, token: PresentedToken, purpose: &str) -> Result<bool> {
        if purpose.is_empty() {
            return Err(invalid_id("purpose"));
        }
        let now = Utc::now();
        let presented_digest = digest(expose_secret(&token.0));
        let rows = <S::Token as EntityBinding>::Entity::find()
            .filter(S::Token::digest_column().eq(presented_digest))
            .filter(S::Token::purpose_column().eq(purpose.to_owned()))
            .filter(S::Token::used_at_column().is_null())
            .filter(S::Token::expires_at_column().gt(now))
            .all(self.database())
            .await
            .map_err(db_error)?;
        Ok(!rows.is_empty())
    }

    async fn inspect(&self, token: PresentedToken, purpose: &str) -> Result<Option<TokenState>> {
        if purpose.is_empty() {
            return Err(invalid_id("purpose"));
        }
        let presented_digest = digest(expose_secret(&token.0));
        let row = <S::Token as EntityBinding>::Entity::find()
            .filter(S::Token::digest_column().eq(presented_digest))
            .filter(S::Token::purpose_column().eq(purpose.to_owned()))
            .one(self.database())
            .await
            .map_err(db_error)?;
        Ok(row.map(|row| TokenState {
            user_id: S::Token::read_user_id(&row),
            address_digest: S::Token::read_address_digest(&row),
            used: S::Token::read_used_at(&row).is_some(),
            expired: S::Token::read_expires_at(&row) <= Utc::now(),
        }))
    }

    async fn consume(&self, token: PresentedToken, purpose: &str) -> Result<ConsumedToken> {
        // Each run of the transaction works through a handle of its own over
        // the same connection, as the body must not borrow `self`, and
        // presents a copy of the token: a run the database ends in a
        // conflict with a concurrent consume runs again
        // (`in_retried_transaction`), and finds the token spent if that
        // consume won.
        let database = self.database().clone();
        let secret = token.0;
        let purpose = purpose.to_owned();
        in_retried_transaction(self.database(), move |tx| {
            let this = SeaOrmStorage::<S>::new(database.clone());
            let token = PresentedToken(secret.clone());
            let purpose = purpose.clone();
            Box::pin(async move { this.consume_in(tx, token, &purpose).await })
        })
        .await
    }

    async fn consume_in(
        &self,
        tx: &mut AuthTransaction<'_>,
        token: PresentedToken,
        purpose: &str,
    ) -> Result<ConsumedToken> {
        if purpose.is_empty() {
            return Err(invalid_id("purpose"));
        }
        let now = Utc::now();
        let presented_digest = digest(expose_secret(&token.0));
        let digest_column = S::Token::digest_column();
        let purpose_column = S::Token::purpose_column();
        let used_column = S::Token::used_at_column();
        let expires_column = S::Token::expires_at_column();
        // The compare-and-set comes first, before any plain read in the
        // transaction. A plain read fixes the transaction's snapshot, and
        // under MariaDB's `innodb_snapshot_isolation` a later write to a row
        // that a concurrent consume changed since then fails with 1020.
        // Written first, the compare-and-set sees the committed row: a token
        // a concurrent consume spent is no longer unused, and this consume
        // finds no live token.
        let update = <S::Token as EntityBinding>::Entity::update_many()
            .col_expr(used_column, Expr::value(now))
            .filter(digest_column.eq(presented_digest.clone()))
            .filter(purpose_column.eq(purpose.to_owned()))
            .filter(used_column.is_null())
            .filter(expires_column.gt(now))
            .exec(tx.connection())
            .await
            .map_err(|error| tx.database_error(error))?;
        match update.rows_affected {
            0 => {
                return Err(Error::NotFound {
                    resource: "token".to_owned(),
                    identifier: purpose.to_owned(),
                });
            }
            1 => {}
            _ => {
                return Err(Error::Conflict {
                    resource: "token".to_owned(),
                    message: "more than one live token has this digest".to_owned(),
                });
            }
        }
        // The row is this transaction's now. Reading it with a lock, rather
        // than a plain read, keeps the transaction without a snapshot for
        // the sibling update below too.
        let row = <S::Token as EntityBinding>::Entity::find()
            .filter(digest_column.eq(presented_digest))
            .filter(purpose_column.eq(purpose.to_owned()))
            .lock_exclusive()
            .one(tx.connection())
            .await
            .map_err(|error| tx.database_error(error))?
            .ok_or_else(|| Error::NotFound {
                resource: "token".to_owned(),
                identifier: purpose.to_owned(),
            })?;
        let id = S::Token::read_token_id(&row);
        if let Some(user_id) = S::Token::read_user_id(&row) {
            // A deadlock with a concurrent consume of a sibling token is
            // still possible here; `consume` runs the transaction again.
            let sibling = <S::Token as EntityBinding>::Entity::update_many()
                .col_expr(used_column, Expr::value(now))
                .filter(S::Token::user_id_column().eq(S::Token::user_id_value(&user_id)))
                .filter(purpose_column.eq(purpose.to_owned()))
                .filter(used_column.is_null())
                .exec(tx.connection())
                .await
                .map_err(|error| tx.database_error(error))?;
            let _ = sibling.rows_affected;
        }
        Ok(ConsumedToken {
            token_id: id,
            user_id: S::Token::read_user_id(&row),
            purpose: S::Token::read_purpose(&row),
            expires_at: S::Token::read_expires_at(&row),
        })
    }
}

#[async_trait]
impl<S> PasswordResetStore for SeaOrmStorage<S>
where
    S: AuthSchema,
    S::Token: TokenFields,
    S::User: UserFields + UserOptionalFields + SessionEpoch,
    S::Session: SessionFields,
    <S::Token as EntityBinding>::Entity: EntityTrait<
            Model = <S::Token as EntityBinding>::Model,
            ActiveModel = <S::Token as EntityBinding>::ActiveModel,
        >,
    <S::Token as EntityBinding>::Column: ColumnTrait,
    <S::User as EntityBinding>::Entity: EntityTrait<
            Model = <S::User as EntityBinding>::Model,
            ActiveModel = <S::User as EntityBinding>::ActiveModel,
        >,
    <S::User as EntityBinding>::Column: ColumnTrait,
    <S::Session as EntityBinding>::Entity: EntityTrait<
            Model = <S::Session as EntityBinding>::Model,
            ActiveModel = <S::Session as EntityBinding>::ActiveModel,
        >,
    <S::Session as EntityBinding>::Column: ColumnTrait,
{
    async fn apply_password_reset(&self, input: PasswordResetInput) -> Result<PasswordResetCommit> {
        // The transaction body must not borrow `self`, so it works through a
        // handle of its own over the same connection.
        let this = SeaOrmStorage::<S>::new(self.database().clone());
        in_transaction(self.database(), move |tx| {
            Box::pin(async move {
                let consumed = <SeaOrmStorage<S> as TokenStore>::consume_in(
                    &this,
                    &mut *tx,
                    input.token,
                    PASSWORD_RESET_PURPOSE,
                )
                .await?;
                let user_id = consumed.user_id.ok_or_else(|| Error::Conflict {
                    resource: "password-reset".to_owned(),
                    message: "reset token carries no owner".to_owned(),
                })?;
                if let Some(expected) = &input.expected_user_id
                    && expected != &user_id
                {
                    return Err(Error::Conflict {
                        resource: "password-reset".to_owned(),
                        message: "reset token belongs to another user".to_owned(),
                    });
                }
                let user = <S::User as EntityBinding>::Entity::find()
                    .filter(S::User::user_id_column().eq(S::User::user_id_value(&user_id)))
                    .all(tx.connection())
                    .await
                    .map_err(db_error)?
                    .into_iter()
                    .next()
                    .ok_or_else(|| Error::NotFound {
                        resource: "user".to_owned(),
                        identifier: user_id.clone(),
                    })?;
                let current_epoch = S::User::auth_epoch(&user);
                let mut active: <S::User as EntityBinding>::ActiveModel = Default::default();
                S::User::write_password_hash(&mut active, Some(&input.new_password_hash));
                S::User::write_remember_token(&mut active, None);
                S::User::write_locked_at(&mut active, None);
                let update = <S::User as EntityBinding>::Entity::update_many()
                    .set(active)
                    .col_expr(
                        S::User::auth_epoch_column(),
                        Expr::col(S::User::auth_epoch_column()).add(1),
                    )
                    .filter(S::User::user_id_column().eq(S::User::user_id_value(&user_id)))
                    .filter(S::User::auth_epoch_column().eq(current_epoch as i64))
                    .exec(tx.connection())
                    .await
                    .map_err(db_error)?;
                if update.rows_affected != 1 {
                    return Err(Error::Conflict {
                        resource: "user".to_owned(),
                        message: "authentication epoch changed concurrently".to_owned(),
                    });
                }
                let now = Utc::now();
                let sessions = <S::Session as EntityBinding>::Entity::find()
                    .filter(S::Session::user_id_column().eq(S::Session::user_id_value(&user_id)))
                    .filter(S::Session::revoked_at_column().is_null())
                    .all(tx.connection())
                    .await
                    .map_err(db_error)?;
                let mut revoked_sessions = 0;
                for session in sessions {
                    let mut active = session.into_active_model();
                    S::Session::write_revoked_at(&mut active, Some(now));
                    <S::Session as EntityBinding>::Entity::update(active)
                        .exec(tx.connection())
                        .await
                        .map_err(db_error)?;
                    revoked_sessions += 1;
                }
                Ok(PasswordResetCommit {
                    user_id,
                    auth_epoch: current_epoch.saturating_add(1),
                    revoked_sessions,
                })
            })
        })
        .await
    }
}
