//! Default SeaORM implementation of the atomic first-email-proof boundary.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, DbErr, EntityTrait,
    QueryFilter, TransactionTrait,
};
use secrecy::ExposeSecret;

use crate::crypto::Encryptor;
use crate::default_schema::{
    DefaultAuthSchema, accounts, methods, provider_tokens, remembers, sessions, two_factor, users,
};
use crate::first_email_proof::{
    FirstEmailProofCommit, FirstEmailProofKind, FirstEmailProofMutation, FirstEmailProofStore,
    NewVerifiedProviderAccount, VerifiedProviderAccountCommit,
};
use crate::storage::{AuthTransaction, PASSWORD_RESET_PURPOSE, SeaOrmStorage, TokenStore};
use crate::{Error, Result};

/// Atomic first-email-proof store for Magnetar's default application schema.
#[derive(Clone)]
pub struct SqlFirstEmailProofStore {
    database: DatabaseConnection,
    _encryptor: Arc<dyn Encryptor>,
}

impl SqlFirstEmailProofStore {
    /// Bind the store to one default-schema database and purpose-bound crypto.
    #[must_use]
    pub fn new(database: DatabaseConnection, encryptor: Arc<dyn Encryptor>) -> Self {
        Self {
            database,
            _encryptor: encryptor,
        }
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
            let first_proof = user.email_verified_at.is_none();
            let current_epoch = user.auth_epoch;
            let next_epoch = current_epoch
                .checked_add(1)
                .ok_or_else(|| Error::Conflict {
                    resource: "user".to_owned(),
                    message: "authentication epoch is exhausted".to_owned(),
                })?;
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
                auth_epoch: Set(next_epoch),
                ..Default::default()
            };
            if first_proof {
                update.email_verified_at = Set(Some(now));
            }
            let mut query = users::Entity::update_many()
                .set(update)
                .filter(users::Column::Id.eq(numeric_user_id))
                .filter(users::Column::AuthEpoch.eq(current_epoch));
            if first_proof {
                query = query.filter(users::Column::EmailVerifiedAt.is_null());
            }
            let updated = query.exec(&transaction).await.map_err(database_error)?;
            if updated.rows_affected != 1 {
                return Err(Error::Conflict {
                    resource: "first-email-proof".to_owned(),
                    message: "account proof state changed concurrently".to_owned(),
                });
            }

            Ok(FirstEmailProofCommit {
                user_id,
                kind: FirstEmailProofKind::PasswordReset,
                first_proof,
                auth_epoch: next_epoch as u64,
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
    async fn apply(&self, mutation: FirstEmailProofMutation) -> Result<FirstEmailProofCommit> {
        match mutation {
            FirstEmailProofMutation::PasswordReset {
                token,
                expected_user_id,
                new_password_hash,
            } => {
                self.apply_password_reset(token, expected_user_id, new_password_hash)
                    .await
            }
            FirstEmailProofMutation::MagicLink { .. } => Err(Error::InvalidInput {
                field: "first-email-proof mutation".to_owned(),
                message: "magic-link proof is not composed yet".to_owned(),
            }),
            FirstEmailProofMutation::OAuthEmailCompletion { .. } => Err(Error::InvalidInput {
                field: "first-email-proof mutation".to_owned(),
                message: "OAuth email-completion proof is not composed yet".to_owned(),
            }),
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

fn database_error(error: DbErr) -> Error {
    Error::Internal {
        message: format!("first-email-proof database error: {error}"),
    }
}
