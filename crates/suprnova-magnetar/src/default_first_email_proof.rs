//! Default SeaORM implementation of the atomic first-email-proof boundary.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    DatabaseTransaction, DbBackend, DbErr, EntityTrait, QueryFilter, QuerySelect, TransactionTrait,
};
use secrecy::ExposeSecret;

use crate::crypto::Encryptor;
use crate::default_schema::{
    DefaultAuthSchema, accounts, ceremonies, methods, provider_tokens, remembers, sessions,
    two_factor, users,
};
use crate::first_email_proof::{
    FirstEmailProofCommit, FirstEmailProofKind, FirstEmailProofMutation, FirstEmailProofOutcome,
    FirstEmailProofStore, NewVerifiedProviderAccount, VerifiedProviderAccountCommit,
};
#[cfg(feature = "oauth")]
use crate::oauth::{
    authorization::decrypt,
    email_completion::{
        BindingPayload, OAUTH_EMAIL_COMPLETION_BINDING_KIND, OAUTH_EMAIL_COMPLETION_PURPOSE,
    },
    identity::{OAUTH_PENDING_IDENTITY_KIND, PendingIdentityPayload},
};
#[cfg(feature = "magic-link")]
use crate::plugins::magic_link::MAGIC_LINK_PURPOSE;
use crate::storage::{AuthTransaction, PASSWORD_RESET_PURPOSE, SeaOrmStorage, TokenStore};
use crate::{Error, Result};

enum FirstProofEpoch {
    Acquired(i64),
    AlreadyVerified(i64),
}

/// Atomic first-email-proof store for Magnetar's default application schema.
#[derive(Clone)]
pub struct SqlFirstEmailProofStore {
    database: DatabaseConnection,
    encryptor: Arc<dyn Encryptor>,
}

impl SqlFirstEmailProofStore {
    /// Bind the store to one default-schema database and purpose-bound crypto.
    #[must_use]
    pub fn new(database: DatabaseConnection, encryptor: Arc<dyn Encryptor>) -> Self {
        Self {
            database,
            encryptor,
        }
    }
    async fn try_increment_auth_epoch(
        transaction: &DatabaseTransaction,
        user_id: i64,
        current_epoch: i64,
        email_must_be_unverified: bool,
    ) -> Result<Option<i64>> {
        let next_epoch = current_epoch
            .checked_add(1)
            .ok_or_else(|| Error::Conflict {
                resource: "user".to_owned(),
                message: "authentication epoch is exhausted".to_owned(),
            })?;
        let mut query = users::Entity::update_many()
            .col_expr(
                users::Column::AuthEpoch,
                Expr::col(users::Column::AuthEpoch).add(1),
            )
            .filter(users::Column::Id.eq(user_id))
            .filter(users::Column::AuthEpoch.eq(current_epoch));
        if email_must_be_unverified {
            query = query.filter(users::Column::EmailVerifiedAt.is_null());
        }
        let updated = query.exec(transaction).await.map_err(database_error)?;
        match updated.rows_affected {
            0 => Ok(None),
            1 => Ok(Some(next_epoch)),
            _ => Err(proof_state_conflict()),
        }
    }

    async fn acquire_first_proof_epoch(
        transaction: &DatabaseTransaction,
        user_id: i64,
        current_epoch: i64,
    ) -> Result<FirstProofEpoch> {
        if let Some(next_epoch) =
            Self::try_increment_auth_epoch(transaction, user_id, current_epoch, true).await?
        {
            return Ok(FirstProofEpoch::Acquired(next_epoch));
        }

        let latest = if transaction.get_database_backend() == DbBackend::Sqlite {
            users::Entity::find_by_id(user_id).one(transaction).await
        } else {
            users::Entity::find_by_id(user_id)
                .lock_exclusive()
                .one(transaction)
                .await
        }
        .map_err(database_error)?
        .ok_or_else(proof_state_conflict)?;
        if latest.email_verified_at.is_some() {
            return Ok(FirstProofEpoch::AlreadyVerified(latest.auth_epoch));
        }

        Self::try_increment_auth_epoch(transaction, user_id, latest.auth_epoch, true)
            .await?
            .map(FirstProofEpoch::Acquired)
            .ok_or_else(proof_state_conflict)
    }

    async fn apply_password_reset(
        &self,
        token: crate::storage::PresentedToken,
        expected_user_id: Option<String>,
        new_password_hash: secrecy::SecretString,
    ) -> Result<FirstEmailProofCommit> {
        let mut transaction = self.database.begin().await.map_err(database_error)?;
        let result = async {
            let consumed = {
                let storage = SeaOrmStorage::<DefaultAuthSchema>::new(self.database.clone());
                let mut auth_transaction = AuthTransaction::new(&mut transaction);
                storage
                    .consume_in(&mut auth_transaction, token, PASSWORD_RESET_PURPOSE)
                    .await?
            };
            let user_id = consumed.user_id.ok_or_else(|| Error::Conflict {
                resource: "first-email-proof".to_owned(),
                message: "proof token carries no owner".to_owned(),
            })?;
            if let Some(expected) = expected_user_id
                && expected != user_id
            {
                return Err(Error::Conflict {
                    resource: "first-email-proof".to_owned(),
                    message: "proof token belongs to another user".to_owned(),
                });
            }
            let numeric_user_id = parse_user_id(&user_id)?;
            let user = users::Entity::find_by_id(numeric_user_id)
                .one(&transaction)
                .await
                .map_err(database_error)?
                .ok_or_else(|| Error::NotFound {
                    resource: "user".to_owned(),
                    identifier: user_id.clone(),
                })?;
            let mut first_proof = user.email_verified_at.is_none();
            let current_epoch = user.auth_epoch;
            let next_epoch = if first_proof {
                match Self::acquire_first_proof_epoch(&transaction, numeric_user_id, current_epoch)
                    .await?
                {
                    FirstProofEpoch::Acquired(next_epoch) => next_epoch,
                    FirstProofEpoch::AlreadyVerified(latest_epoch) => {
                        first_proof = false;
                        Self::try_increment_auth_epoch(
                            &transaction,
                            numeric_user_id,
                            latest_epoch,
                            false,
                        )
                        .await?
                        .ok_or_else(proof_state_conflict)?
                    }
                }
            } else {
                Self::try_increment_auth_epoch(&transaction, numeric_user_id, current_epoch, false)
                    .await?
                    .ok_or_else(proof_state_conflict)?
            };
            let now = Utc::now();

            if first_proof {
                let linked_accounts = accounts::Entity::find()
                    .filter(accounts::Column::UserId.eq(numeric_user_id))
                    .all(&transaction)
                    .await
                    .map_err(database_error)?;
                let provider_record_ids = linked_accounts
                    .iter()
                    .map(|account| account.id.to_string())
                    .collect::<Vec<_>>();
                if !provider_record_ids.is_empty() {
                    provider_tokens::Entity::delete_many()
                        .filter(provider_tokens::Column::Id.is_in(provider_record_ids))
                        .exec(&transaction)
                        .await
                        .map_err(database_error)?;
                }
                accounts::Entity::delete_many()
                    .filter(accounts::Column::UserId.eq(numeric_user_id))
                    .exec(&transaction)
                    .await
                    .map_err(database_error)?;
                methods::Entity::delete_many()
                    .filter(methods::Column::UserId.eq(numeric_user_id))
                    .exec(&transaction)
                    .await
                    .map_err(database_error)?;
                two_factor::Entity::delete_by_id(&user_id)
                    .exec(&transaction)
                    .await
                    .map_err(database_error)?;
            }

            let revoked_sessions = sessions::Entity::update_many()
                .set(sessions::ActiveModel {
                    revoked_at: Set(Some(now)),
                    ..Default::default()
                })
                .filter(sessions::Column::UserId.eq(numeric_user_id))
                .filter(sessions::Column::RevokedAt.is_null())
                .exec(&transaction)
                .await
                .map_err(database_error)?
                .rows_affected;
            let revoked_remember_rows = remembers::Entity::delete_many()
                .filter(remembers::Column::UserId.eq(&user_id))
                .exec(&transaction)
                .await
                .map_err(database_error)?
                .rows_affected;

            let mut update = users::ActiveModel {
                password_hash: Set(Some(new_password_hash.expose_secret().to_owned())),
                remember_token: Set(None),
                locked_at: Set(None),
                ..Default::default()
            };
            if first_proof {
                update.email_verified_at = Set(Some(now));
            }
            users::Entity::update_many()
                .set(update)
                .filter(users::Column::Id.eq(numeric_user_id))
                .exec(&transaction)
                .await
                .map_err(database_error)?;

            Ok(FirstEmailProofCommit {
                user_id,
                kind: FirstEmailProofKind::PasswordReset,
                first_proof,
                auth_epoch: next_epoch as u64,
                provider_account_id: None,
                revoked_sessions,
                revoked_remember_rows,
            })
        }
        .await;

        match result {
            Ok(commit) => transaction
                .commit()
                .await
                .map_err(database_error)
                .map(|()| commit),
            Err(error) => {
                let _ = transaction.rollback().await;
                Err(error)
            }
        }
    }

    #[cfg(feature = "magic-link")]
    async fn apply_magic_link(
        &self,
        token: crate::storage::PresentedToken,
    ) -> Result<FirstEmailProofCommit> {
        let mut transaction = self.database.begin().await.map_err(database_error)?;
        let result = async {
            let consumed = {
                let storage = SeaOrmStorage::<DefaultAuthSchema>::new(self.database.clone());
                let mut auth_transaction = AuthTransaction::new(&mut transaction);
                storage
                    .consume_in(&mut auth_transaction, token, MAGIC_LINK_PURPOSE)
                    .await?
            };
            let user_id = consumed.user_id.ok_or_else(|| Error::Conflict {
                resource: "first-email-proof".to_owned(),
                message: "proof token carries no owner".to_owned(),
            })?;
            let numeric_user_id = parse_user_id(&user_id)?;
            let user = users::Entity::find_by_id(numeric_user_id)
                .one(&transaction)
                .await
                .map_err(database_error)?
                .ok_or_else(|| Error::NotFound {
                    resource: "user".to_owned(),
                    identifier: user_id.clone(),
                })?;
            let proof_epoch = if user.email_verified_at.is_some() {
                FirstProofEpoch::AlreadyVerified(user.auth_epoch)
            } else {
                Self::acquire_first_proof_epoch(&transaction, numeric_user_id, user.auth_epoch)
                    .await?
            };
            let next_epoch = match proof_epoch {
                FirstProofEpoch::Acquired(next_epoch) => next_epoch,
                FirstProofEpoch::AlreadyVerified(auth_epoch) => {
                    return Ok(FirstEmailProofCommit {
                        user_id,
                        kind: FirstEmailProofKind::MagicLink,
                        first_proof: false,
                        auth_epoch: auth_epoch as u64,
                        provider_account_id: None,
                        revoked_sessions: 0,
                        revoked_remember_rows: 0,
                    });
                }
            };
            let now = Utc::now();
            let linked_accounts = accounts::Entity::find()
                .filter(accounts::Column::UserId.eq(numeric_user_id))
                .all(&transaction)
                .await
                .map_err(database_error)?;
            let provider_record_ids = linked_accounts
                .iter()
                .map(|account| account.id.to_string())
                .collect::<Vec<_>>();
            if !provider_record_ids.is_empty() {
                provider_tokens::Entity::delete_many()
                    .filter(provider_tokens::Column::Id.is_in(provider_record_ids))
                    .exec(&transaction)
                    .await
                    .map_err(database_error)?;
            }
            accounts::Entity::delete_many()
                .filter(accounts::Column::UserId.eq(numeric_user_id))
                .exec(&transaction)
                .await
                .map_err(database_error)?;
            methods::Entity::delete_many()
                .filter(methods::Column::UserId.eq(numeric_user_id))
                .exec(&transaction)
                .await
                .map_err(database_error)?;
            two_factor::Entity::delete_by_id(&user_id)
                .exec(&transaction)
                .await
                .map_err(database_error)?;
            let revoked_sessions = sessions::Entity::update_many()
                .set(sessions::ActiveModel {
                    revoked_at: Set(Some(now)),
                    ..Default::default()
                })
                .filter(sessions::Column::UserId.eq(numeric_user_id))
                .filter(sessions::Column::RevokedAt.is_null())
                .exec(&transaction)
                .await
                .map_err(database_error)?
                .rows_affected;
            let revoked_remember_rows = remembers::Entity::delete_many()
                .filter(remembers::Column::UserId.eq(&user_id))
                .exec(&transaction)
                .await
                .map_err(database_error)?
                .rows_affected;
            users::Entity::update_many()
                .set(users::ActiveModel {
                    password_hash: Set(None),
                    remember_token: Set(None),
                    email_verified_at: Set(Some(now)),
                    locked_at: Set(None),
                    ..Default::default()
                })
                .filter(users::Column::Id.eq(numeric_user_id))
                .exec(&transaction)
                .await
                .map_err(database_error)?;
            Ok(FirstEmailProofCommit {
                user_id,
                kind: FirstEmailProofKind::MagicLink,
                first_proof: true,
                auth_epoch: next_epoch as u64,
                provider_account_id: None,
                revoked_sessions,
                revoked_remember_rows,
            })
        }
        .await;
        match result {
            Ok(commit) => transaction
                .commit()
                .await
                .map_err(database_error)
                .map(|()| commit),
            Err(error) => {
                let _ = transaction.rollback().await;
                Err(error)
            }
        }
    }

    #[cfg(feature = "oauth")]
    async fn consume_ceremony(
        transaction: &DatabaseTransaction,
        selector: &str,
        kind: &str,
    ) -> Result<Vec<u8>> {
        let row = ceremonies::Entity::find()
            .filter(ceremonies::Column::Selector.eq(selector))
            .filter(ceremonies::Column::Kind.eq(kind))
            .filter(ceremonies::Column::UsedAt.is_null())
            .filter(ceremonies::Column::ExpiresAt.gt(Utc::now()))
            .one(transaction)
            .await
            .map_err(database_error)?
            .ok_or_else(|| Error::NotFound {
                resource: "first-email-proof ceremony".to_owned(),
                identifier: selector.to_owned(),
            })?;
        let deleted = ceremonies::Entity::delete_by_id(row.id)
            .exec(transaction)
            .await
            .map_err(database_error)?;
        if deleted.rows_affected != 1 {
            return Err(Error::Conflict {
                resource: "first-email-proof ceremony".to_owned(),
                message: "ceremony was consumed concurrently".to_owned(),
            });
        }
        Ok(row.payload)
    }

    #[cfg(feature = "oauth")]
    async fn apply_oauth_email_completion(
        &self,
        token: crate::storage::PresentedToken,
    ) -> Result<FirstEmailProofOutcome> {
        let mut transaction = self.database.begin().await.map_err(database_error)?;
        let result = async {
            let consumed = {
                let storage = SeaOrmStorage::<DefaultAuthSchema>::new(self.database.clone());
                let mut auth_transaction = AuthTransaction::new(&mut transaction);
                storage
                    .consume_in(&mut auth_transaction, token, OAUTH_EMAIL_COMPLETION_PURPOSE)
                    .await?
            };
            let binding_payload = Self::consume_ceremony(
                &transaction,
                &consumed.token_id,
                OAUTH_EMAIL_COMPLETION_BINDING_KIND,
            )
            .await?;
            let binding: BindingPayload = decrypt(self.encryptor.as_ref(), &binding_payload)?;
            let pending_payload = Self::consume_ceremony(
                &transaction,
                &binding.pending_id,
                OAUTH_PENDING_IDENTITY_KIND,
            )
            .await?;
            let pending: PendingIdentityPayload =
                decrypt(self.encryptor.as_ref(), &pending_payload)?;
            if consumed.user_id.as_deref() != Some(pending.sibling_key.as_str()) {
                return Err(Error::Conflict {
                    resource: "first-email-proof".to_owned(),
                    message: "completion token does not match pending identity".to_owned(),
                });
            }

            let existing = users::Entity::find()
                .filter(users::Column::Email.eq(&binding.normalized_email))
                .one(&transaction)
                .await
                .map_err(database_error)?;
            let Some(user) = existing else {
                let user = users::ActiveModel {
                    email: Set(binding.normalized_email),
                    password_hash: Set(None),
                    email_verified_at: Set(Some(Utc::now())),
                    auth_epoch: Set(0),
                    ..Default::default()
                }
                .insert(&transaction)
                .await
                .map_err(database_error)?;
                accounts::ActiveModel {
                    user_id: Set(user.id),
                    provider: Set(pending.provider),
                    provider_account_id: Set(pending.subject.clone()),
                    ..Default::default()
                }
                .insert(&transaction)
                .await
                .map_err(database_error)?;
                return Ok(FirstEmailProofOutcome::Committed(FirstEmailProofCommit {
                    user_id: user.id.to_string(),
                    kind: FirstEmailProofKind::OAuthEmailCompletion,
                    first_proof: false,
                    auth_epoch: user.auth_epoch as u64,
                    provider_account_id: Some(pending.subject),
                    revoked_sessions: 0,
                    revoked_remember_rows: 0,
                }));
            };
            let proof_epoch = if user.email_verified_at.is_some() {
                FirstProofEpoch::AlreadyVerified(user.auth_epoch)
            } else {
                Self::acquire_first_proof_epoch(&transaction, user.id, user.auth_epoch).await?
            };
            let next_epoch = match proof_epoch {
                FirstProofEpoch::Acquired(next_epoch) => next_epoch,
                FirstProofEpoch::AlreadyVerified(_) => {
                    return Ok(FirstEmailProofOutcome::ExplicitLinkRequired {
                        normalized_email: user.email,
                    });
                }
            };

            let user_id = user.id.to_string();
            let now = Utc::now();
            let linked_accounts = accounts::Entity::find()
                .filter(accounts::Column::UserId.eq(user.id))
                .all(&transaction)
                .await
                .map_err(database_error)?;
            let provider_record_ids = linked_accounts
                .iter()
                .map(|account| account.id.to_string())
                .collect::<Vec<_>>();
            if !provider_record_ids.is_empty() {
                provider_tokens::Entity::delete_many()
                    .filter(provider_tokens::Column::Id.is_in(provider_record_ids))
                    .exec(&transaction)
                    .await
                    .map_err(database_error)?;
            }
            accounts::Entity::delete_many()
                .filter(accounts::Column::UserId.eq(user.id))
                .exec(&transaction)
                .await
                .map_err(database_error)?;
            methods::Entity::delete_many()
                .filter(methods::Column::UserId.eq(user.id))
                .exec(&transaction)
                .await
                .map_err(database_error)?;
            two_factor::Entity::delete_by_id(&user_id)
                .exec(&transaction)
                .await
                .map_err(database_error)?;
            let revoked_sessions = sessions::Entity::update_many()
                .set(sessions::ActiveModel {
                    revoked_at: Set(Some(now)),
                    ..Default::default()
                })
                .filter(sessions::Column::UserId.eq(user.id))
                .filter(sessions::Column::RevokedAt.is_null())
                .exec(&transaction)
                .await
                .map_err(database_error)?
                .rows_affected;
            let revoked_remember_rows = remembers::Entity::delete_many()
                .filter(remembers::Column::UserId.eq(&user_id))
                .exec(&transaction)
                .await
                .map_err(database_error)?
                .rows_affected;
            users::Entity::update_many()
                .set(users::ActiveModel {
                    password_hash: Set(None),
                    remember_token: Set(None),
                    email_verified_at: Set(Some(now)),
                    locked_at: Set(None),
                    ..Default::default()
                })
                .filter(users::Column::Id.eq(user.id))
                .exec(&transaction)
                .await
                .map_err(database_error)?;
            accounts::ActiveModel {
                user_id: Set(user.id),
                provider: Set(pending.provider),
                provider_account_id: Set(pending.subject.clone()),
                ..Default::default()
            }
            .insert(&transaction)
            .await
            .map_err(database_error)?;
            Ok(FirstEmailProofOutcome::Committed(FirstEmailProofCommit {
                user_id,
                kind: FirstEmailProofKind::OAuthEmailCompletion,
                first_proof: true,
                auth_epoch: next_epoch as u64,
                provider_account_id: Some(pending.subject),
                revoked_sessions,
                revoked_remember_rows,
            }))
        }
        .await;
        match result {
            Ok(outcome) => transaction
                .commit()
                .await
                .map_err(database_error)
                .map(|()| outcome),
            Err(error) => {
                let _ = transaction.rollback().await;
                Err(error)
            }
        }
    }

    async fn initialize_provider_account(
        &self,
        input: NewVerifiedProviderAccount,
    ) -> Result<VerifiedProviderAccountCommit> {
        if input.provider.is_empty()
            || input.provider_account_id.is_empty()
            || input.email.is_empty()
        {
            return Err(Error::InvalidInput {
                field: "verified provider account".to_owned(),
                message: "provider, provider account id, and email must be non-empty".to_owned(),
            });
        }
        if let Some(winner) = self
            .provider_owner(&input.provider, &input.provider_account_id)
            .await?
        {
            return Ok(winner);
        }

        let provider = input.provider;
        let provider_account_id = input.provider_account_id;
        let transaction = self.database.begin().await.map_err(database_error)?;
        let user = match (users::ActiveModel {
            email: Set(input.email),
            email_verified_at: Set(Some(Utc::now())),
            auth_epoch: Set(0),
            ..Default::default()
        })
        .insert(&transaction)
        .await
        {
            Ok(user) => user,
            Err(error) => {
                let _ = transaction.rollback().await;
                return Err(database_error(error));
            }
        };
        let linked = accounts::ActiveModel {
            user_id: Set(user.id),
            provider: Set(provider.clone()),
            provider_account_id: Set(provider_account_id.clone()),
            ..Default::default()
        }
        .insert(&transaction)
        .await;
        match linked {
            Ok(_) => transaction
                .commit()
                .await
                .map_err(database_error)
                .map(|()| VerifiedProviderAccountCommit {
                    user_id: user.id.to_string(),
                    auth_epoch: user.auth_epoch as u64,
                }),
            Err(error)
                if matches!(
                    error.sql_err(),
                    Some(sea_orm::SqlErr::UniqueConstraintViolation(_))
                ) =>
            {
                let _ = transaction.rollback().await;
                self.provider_owner(&provider, &provider_account_id)
                    .await?
                    .ok_or_else(|| Error::Conflict {
                        resource: "verified provider account".to_owned(),
                        message: "provider identity changed concurrently".to_owned(),
                    })
            }
            Err(error) => {
                let _ = transaction.rollback().await;
                Err(database_error(error))
            }
        }
    }

    async fn provider_owner(
        &self,
        provider: &str,
        provider_account_id: &str,
    ) -> Result<Option<VerifiedProviderAccountCommit>> {
        let account = accounts::Entity::find()
            .filter(accounts::Column::Provider.eq(provider))
            .filter(accounts::Column::ProviderAccountId.eq(provider_account_id))
            .one(&self.database)
            .await
            .map_err(database_error)?;
        let Some(account) = account else {
            return Ok(None);
        };
        let user = users::Entity::find_by_id(account.user_id)
            .one(&self.database)
            .await
            .map_err(database_error)?
            .ok_or_else(|| Error::Conflict {
                resource: "verified provider account".to_owned(),
                message: "linked provider has no owning user".to_owned(),
            })?;
        Ok(Some(VerifiedProviderAccountCommit {
            user_id: user.id.to_string(),
            auth_epoch: user.auth_epoch as u64,
        }))
    }
}

#[async_trait]
impl FirstEmailProofStore for SqlFirstEmailProofStore {
    async fn apply(&self, mutation: FirstEmailProofMutation) -> Result<FirstEmailProofOutcome> {
        match mutation {
            FirstEmailProofMutation::PasswordReset {
                token,
                expected_user_id,
                new_password_hash,
            } => self
                .apply_password_reset(token, expected_user_id, new_password_hash)
                .await
                .map(FirstEmailProofOutcome::Committed),
            FirstEmailProofMutation::MagicLink { token } => {
                #[cfg(feature = "magic-link")]
                {
                    self.apply_magic_link(token)
                        .await
                        .map(FirstEmailProofOutcome::Committed)
                }
                #[cfg(not(feature = "magic-link"))]
                {
                    let _ = token;
                    Err(Error::InvalidInput {
                        field: "first-email-proof mutation".to_owned(),
                        message: "magic-link feature is disabled".to_owned(),
                    })
                }
            }
            FirstEmailProofMutation::OAuthEmailCompletion { token } => {
                #[cfg(feature = "oauth")]
                {
                    self.apply_oauth_email_completion(token).await
                }
                #[cfg(not(feature = "oauth"))]
                {
                    let _ = token;
                    Err(Error::InvalidInput {
                        field: "first-email-proof mutation".to_owned(),
                        message: "OAuth feature is disabled".to_owned(),
                    })
                }
            }
        }
    }

    async fn create_verified_provider_account(
        &self,
        input: NewVerifiedProviderAccount,
    ) -> Result<VerifiedProviderAccountCommit> {
        self.initialize_provider_account(input).await
    }
}

fn parse_user_id(value: &str) -> Result<i64> {
    value.parse::<i64>().map_err(|_| Error::InvalidInput {
        field: "user_id".to_owned(),
        message: "default schema user ids must be signed 64-bit integers".to_owned(),
    })
}

fn proof_state_conflict() -> Error {
    Error::Conflict {
        resource: "first-email-proof".to_owned(),
        message: "account proof state changed concurrently".to_owned(),
    }
}

fn database_error(error: DbErr) -> Error {
    Error::Internal {
        message: format!("first-email-proof database error: {error}"),
    }
}
